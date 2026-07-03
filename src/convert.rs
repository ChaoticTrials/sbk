use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::Context;

use crate::checksum::hash;
use crate::classify::Group;
use crate::codec;
use crate::error::SbkError;
use crate::format::frame_dir::read_frame_dir;
use crate::format::header::read_header;
use crate::format::index::{IndexEntry, read_index, validate_entries};
use crate::preprocess::{mca::reconstruct_mca_bytes, nbt::reconstruct_nbt_bytes};
use crate::solid::extractor::slice_from_frames;

/// The target archive format for `sbk convert`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertFormat {
    Sbk,
    Zip,
    TarGz,
    TarXz,
}

impl ConvertFormat {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "sbk" => Some(Self::Sbk),
            "zip" => Some(Self::Zip),
            "tar.gz" => Some(Self::TarGz),
            "tar.xz" => Some(Self::TarXz),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Sbk => ".sbk",
            Self::Zip => ".zip",
            Self::TarGz => ".tar.gz",
            Self::TarXz => ".tar.xz",
        }
    }
}

/// Detect the archive format from a file path's extension.
/// Returns `None` if the format is unrecognised.
pub fn detect_format(path: &Path) -> Option<ConvertFormat> {
    let name = path.file_name()?.to_str()?;
    if name.ends_with(".sbk") {
        Some(ConvertFormat::Sbk)
    } else if name.ends_with(".tar.gz") {
        Some(ConvertFormat::TarGz)
    } else if name.ends_with(".tar.xz") {
        Some(ConvertFormat::TarXz)
    } else if name.ends_with(".zip") {
        Some(ConvertFormat::Zip)
    } else {
        None
    }
}

/// Convert a zip/tar.gz/tar.xz archive to an SBK archive.
///
/// The input archive is extracted into a temporary directory, then compressed
/// with `sbk compress`. Returns the number of files written (always 0 — cosmetic).
pub fn convert_to_sbk(
    archive: &Path,
    output: &Path,
    from_format: ConvertFormat,
    threads: usize,
    level: u32,
    algorithm: crate::format::header::Algorithm,
) -> anyhow::Result<u64> {
    let temp_dir = tempfile::TempDir::new()
        .with_context(|| "failed to create temporary directory for extraction")?;

    match from_format {
        ConvertFormat::Zip => {
            let file = File::open(archive)
                .with_context(|| format!("failed to open '{}'", archive.display()))?;
            let mut zip_archive = zip::ZipArchive::new(file)
                .with_context(|| format!("failed to read ZIP '{}'", archive.display()))?;
            for i in 0..zip_archive.len() {
                let mut entry = zip_archive.by_index(i)?;
                if entry.is_dir() {
                    continue;
                }
                // Zip-slip guard: `enclosed_name()` rejects absolute paths and
                // `..` components that would escape the extraction directory.
                let rel_path = entry.enclosed_name().ok_or_else(|| {
                    anyhow::anyhow!("zip entry has unsafe path: '{}'", entry.name())
                })?;
                let out_path = temp_dir.path().join(rel_path);
                if let Some(parent) = out_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut out_file = File::create(&out_path)
                    .with_context(|| format!("failed to create '{}'", out_path.display()))?;
                std::io::copy(&mut entry, &mut out_file)?;
                drop(out_file);
                // Restore the entry's mtime (assumed UTC) so it survives into
                // the SBK index. Entries without a valid datetime keep the
                // extraction time.
                if let Some(dt) = entry.last_modified() {
                    if let Ok(pdt) = time::PrimitiveDateTime::try_from(dt) {
                        let secs = pdt.assume_utc().unix_timestamp();
                        let ft = filetime::FileTime::from_unix_time(secs, 0);
                        filetime::set_file_mtime(&out_path, ft).with_context(|| {
                            format!("failed to set mtime on '{}'", out_path.display())
                        })?;
                    }
                }
            }
        }
        ConvertFormat::TarGz => {
            let file = File::open(archive)
                .with_context(|| format!("failed to open '{}'", archive.display()))?;
            let decoder = flate2::read::GzDecoder::new(file);
            let mut tar = tar::Archive::new(decoder);
            tar.unpack(temp_dir.path())
                .with_context(|| format!("failed to unpack tar.gz '{}'", archive.display()))?;
        }
        ConvertFormat::TarXz => {
            let file = File::open(archive)
                .with_context(|| format!("failed to open '{}'", archive.display()))?;
            let decoder = xz2::read::XzDecoder::new(file);
            let mut tar = tar::Archive::new(decoder);
            tar.unpack(temp_dir.path())
                .with_context(|| format!("failed to unpack tar.xz '{}'", archive.display()))?;
        }
        ConvertFormat::Sbk => {
            return Err(anyhow::anyhow!("cannot convert SBK to SBK"));
        }
    }

    let opts = crate::filter::CompressOptions {
        output: output.to_path_buf(),
        threads,
        level,
        algorithm,
        max_age: None,
        since: None,
        patterns: crate::filter::FilterMode::None,
        include_session_lock: false,
        quiet: false,
    };
    crate::compress::compress(temp_dir.path(), &opts)?;

    Ok(0)
}

/// Reconstruct the original file bytes from the preprocessed stream bytes.
/// This is the inverse of the preprocessing done at compress time.
pub fn reconstruct_bytes(group_id: u8, preprocessed: &[u8]) -> anyhow::Result<Vec<u8>> {
    match Group::from_u8(group_id) {
        Some(Group::Mca) => reconstruct_mca_bytes(preprocessed),
        Some(Group::Nbt) => reconstruct_nbt_bytes(preprocessed),
        Some(Group::Json) | Some(Group::Raw) | None => Ok(preprocessed.to_vec()),
    }
}

/// Convert an SBK archive to a standard archive format.
///
/// Returns the number of files written into the output archive.
pub fn convert_from_sbk(
    archive: &Path,
    output: &Path,
    format: ConvertFormat,
    threads: usize,
    level: u32,
) -> anyhow::Result<u64> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global()
        .ok();

    // Stage 1: read header, frame dir, index
    let mut f = File::open(archive)
        .with_context(|| format!("failed to open archive '{}'", archive.display()))?;
    let header = read_header(&mut f)?;
    let codec = codec::from_algorithm(header.algorithm);

    let frame_size = header.frame_size_bytes;

    f.seek(SeekFrom::Start(header.frame_dir_offset))?;
    let frame_dir = read_frame_dir(&mut f, frame_size)?;

    f.seek(SeekFrom::Start(header.index_offset))?;
    let all_entries = read_index(
        &mut f,
        &*codec,
        header.index_compressed_size,
        header.index_checksum,
    )?;
    validate_entries(&all_entries, &frame_dir, frame_size)?;

    // Path traversal guard: reject absolute paths and `..` components
    for entry in &all_entries {
        let p = std::path::Path::new(&entry.path);
        if p.is_absolute() {
            return Err(anyhow::anyhow!(
                "archive entry has absolute path: {}",
                entry.path
            ));
        }
        for component in p.components() {
            if component == std::path::Component::ParentDir {
                return Err(anyhow::anyhow!(
                    "archive entry path contains '..': {}",
                    entry.path
                ));
            }
        }
    }

    // Stage 2: stream entries into the target archive with bounded memory.
    //
    // Entries are processed in (group_id, stream_offset) order — the order
    // their bytes appear in the frame streams — so frames can be loaded once,
    // in batches of `threads`, and evicted as soon as no later entry needs
    // them. Peak memory is O(threads × frame_size) plus the largest single
    // file, not O(archive). Note this writes archive members in stream order,
    // which may differ from the index (path) order.
    let mut sorted_entries: Vec<IndexEntry> = all_entries.clone();
    sorted_entries.sort_by(|a, b| {
        (a.group_id, a.stream_offset, &a.path).cmp(&(b.group_id, b.stream_offset, &b.path))
    });

    // Ordered load schedule of all unique frames needed.
    let mut needed_set: HashSet<(u8, u32)> = HashSet::new();
    for entry in &sorted_entries {
        if entry.stream_raw_size == 0 {
            continue;
        }
        let start = (entry.stream_offset / frame_size) as u32;
        let end = ((entry.stream_offset + entry.stream_raw_size - 1) / frame_size) as u32;
        for fi in start..=end {
            let group_frames = &frame_dir.groups[entry.group_id as usize];
            if fi as usize >= group_frames.len() {
                return Err(anyhow::anyhow!(
                    "frame index {} out of bounds for group {}",
                    fi,
                    entry.group_id
                ));
            }
            needed_set.insert((entry.group_id, fi));
        }
    }
    let mut needed_frames: Vec<(u8, u32)> = needed_set.into_iter().collect();
    needed_frames.sort_unstable();

    let out = File::create(output)
        .with_context(|| format!("failed to create output '{}'", output.display()))?;
    let mut sink: Box<dyn EntrySink> = match format {
        ConvertFormat::Zip => Box::new(ZipSink {
            writer: zip::ZipWriter::new(out),
            level,
        }),
        ConvertFormat::TarGz => Box::new(TarSink {
            builder: tar::Builder::new(flate2::write::GzEncoder::new(
                out,
                flate2::Compression::new(level),
            )),
        }),
        ConvertFormat::TarXz => Box::new(TarSink {
            builder: tar::Builder::new(xz2::write::XzEncoder::new(out, level)),
        }),
        ConvertFormat::Sbk => {
            return Err(anyhow::anyhow!(
                "cannot convert SBK to SBK: use a different --to format"
            ));
        }
    };

    let batch_size = threads.max(1);
    let mut cache: HashMap<(u8, u32), Vec<u8>> = HashMap::new();
    let mut cursor = 0usize; // next unloaded frame in `needed_frames`

    for (idx, e) in sorted_entries.iter().enumerate() {
        // Load frame batches until this entry's last frame is cached. Frames
        // are loaded in sorted order, matching the sorted entry order, so the
        // entry's earlier frames are already cached (or still retained).
        if e.stream_raw_size > 0 {
            let end_frame = ((e.stream_offset + e.stream_raw_size - 1) / frame_size) as u32;
            let last_key = (e.group_id, end_frame);
            while !cache.contains_key(&last_key) {
                if cursor >= needed_frames.len() {
                    return Err(anyhow::anyhow!(
                        "frame ({},{}) missing from load schedule (corrupt index?)",
                        last_key.0,
                        last_key.1
                    ));
                }
                let end = (cursor + batch_size).min(needed_frames.len());
                load_frame_batch(
                    &mut f,
                    &frame_dir,
                    &*codec,
                    &needed_frames[cursor..end],
                    &mut cache,
                )?;
                cursor = end;
            }
        }

        let preprocessed = slice_from_frames(
            &cache,
            e.group_id,
            e.stream_offset,
            e.stream_raw_size,
            frame_size,
        )?;
        let reconstructed = reconstruct_bytes(e.group_id, &preprocessed)
            .with_context(|| format!("failed to reconstruct '{}'", e.path))?;
        sink.add(e, &reconstructed)?;

        // Evict frames no later entry needs: with entries sorted by
        // (group_id, stream_offset), every frame of an earlier group and
        // every frame before the next entry's start frame is done.
        if let Some(next) = sorted_entries.get(idx + 1) {
            let next_start = (next.stream_offset / frame_size) as u32;
            cache.retain(|&(g, fi), _| {
                g > next.group_id || (g == next.group_id && fi >= next_start)
            });
        } else {
            cache.clear();
        }
    }

    sink.finish()?;

    Ok(sorted_entries.len() as u64)
}

/// Read (sequentially), checksum-verify, and decompress (in parallel) one
/// batch of frames into `cache`.
fn load_frame_batch(
    f: &mut File,
    frame_dir: &crate::format::frame_dir::FrameDir,
    codec: &dyn crate::codec::Codec,
    batch: &[(u8, u32)],
    cache: &mut HashMap<(u8, u32), Vec<u8>>,
) -> anyhow::Result<()> {
    use rayon::prelude::*;

    // Phase A: sequential read + checksum verification
    let mut compressed: Vec<((u8, u32), u32, Vec<u8>)> = Vec::with_capacity(batch.len());
    for &(group_id, frame_idx) in batch {
        let group_frames = &frame_dir.groups[group_id as usize];
        if frame_idx as usize >= group_frames.len() {
            return Err(anyhow::anyhow!(
                "frame index {} out of bounds for group {}",
                frame_idx,
                group_id
            ));
        }
        let entry = &group_frames[frame_idx as usize];
        const MAX_COMPRESSED_FRAME: u32 = 256 * 1024 * 1024;
        if entry.frame_compressed_sz > MAX_COMPRESSED_FRAME {
            return Err(anyhow::anyhow!(
                "frame ({},{}) compressed size {} exceeds sanity limit",
                group_id,
                frame_idx,
                entry.frame_compressed_sz
            ));
        }
        let mut buf = vec![0u8; entry.frame_compressed_sz as usize];
        f.seek(SeekFrom::Start(entry.frame_offset))?;
        f.read_exact(&mut buf)?;
        if hash(&buf) != entry.frame_checksum {
            return Err(SbkError::FrameChecksumMismatch(frame_idx).into());
        }
        compressed.push(((group_id, frame_idx), entry.frame_raw_sz, buf));
    }

    // Phase B: decompress the batch in parallel
    let decompressed: Vec<((u8, u32), Vec<u8>)> = compressed
        .par_iter()
        .map(|(key, raw_sz, buf)| {
            let raw = codec.decompress(buf, *raw_sz).map_err(|e| {
                anyhow::anyhow!("frame ({},{}) decompression failed: {}", key.0, key.1, e)
            })?;
            Ok((*key, raw))
        })
        .collect::<anyhow::Result<_>>()?;

    for (key, raw) in decompressed {
        cache.insert(key, raw);
    }
    Ok(())
}

/// Streaming sink over the supported target archive writers.
trait EntrySink {
    fn add(&mut self, entry: &IndexEntry, data: &[u8]) -> anyhow::Result<()>;
    fn finish(self: Box<Self>) -> anyhow::Result<()>;
}

struct ZipSink {
    writer: zip::ZipWriter<File>,
    level: u32,
}

impl EntrySink for ZipSink {
    fn add(&mut self, entry: &IndexEntry, data: &[u8]) -> anyhow::Result<()> {
        use zip::CompressionMethod;
        use zip::write::SimpleFileOptions;

        let opts = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(self.level as i64))
            .last_modified_time(mtime_to_zip_datetime(entry.mtime_ms));
        self.writer.start_file(&entry.path, opts)?;
        self.writer.write_all(data)?;
        Ok(())
    }

    fn finish(self: Box<Self>) -> anyhow::Result<()> {
        self.writer.finish()?;
        Ok(())
    }
}

struct TarSink<W: Write> {
    builder: tar::Builder<W>,
}

impl<W: Write> EntrySink for TarSink<W> {
    fn add(&mut self, entry: &IndexEntry, data: &[u8]) -> anyhow::Result<()> {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        // mtime in seconds; clamp negatives to 0
        let mtime_secs = if entry.mtime_ms >= 0 {
            (entry.mtime_ms / 1000) as u64
        } else {
            0
        };
        header.set_mtime(mtime_secs);
        header.set_mode(0o644);
        header.set_cksum();
        self.builder.append_data(&mut header, &entry.path, data)?;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> anyhow::Result<()> {
        self.builder.finish()?;
        Ok(())
    }
}

fn mtime_to_zip_datetime(mtime_ms: i64) -> zip::DateTime {
    if mtime_ms < 0 {
        return zip::DateTime::default();
    }
    let secs = mtime_ms / 1000;
    let nanos = ((mtime_ms % 1000) * 1_000_000) as i32;
    let odt = time::OffsetDateTime::from_unix_timestamp(secs)
        .ok()
        .and_then(|t| t.checked_add(time::Duration::nanoseconds(nanos as i64)));
    match odt {
        Some(t) => {
            let pdt = time::PrimitiveDateTime::new(t.date(), t.time());
            zip::DateTime::try_from(pdt).unwrap_or_default()
        }
        None => zip::DateTime::default(),
    }
}
