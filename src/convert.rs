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
use crate::format::index::{IndexEntry, read_index};
use crate::preprocess::{mca::reconstruct_mca_bytes, nbt::reconstruct_nbt_bytes};
use crate::solid::extractor::slice_from_frames;

/// The target archive format for `sbk convert`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertFormat {
    Zip,
    TarGz,
    TarXz,
}

impl ConvertFormat {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "zip" => Some(Self::Zip),
            "tar.gz" => Some(Self::TarGz),
            "tar.xz" => Some(Self::TarXz),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => ".zip",
            Self::TarGz => ".tar.gz",
            Self::TarXz => ".tar.xz",
        }
    }
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
pub fn convert(
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

    f.seek(SeekFrom::Start(header.frame_dir_offset))?;
    let frame_dir = read_frame_dir(&mut f)?;

    f.seek(SeekFrom::Start(header.index_offset))?;
    let all_entries = read_index(
        &mut f,
        &*codec,
        header.index_compressed_size,
        header.index_checksum,
    )?;

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

    if all_entries.is_empty() {
        // Create an empty archive
        let out = File::create(output)
            .with_context(|| format!("failed to create output '{}'", output.display()))?;
        match format {
            ConvertFormat::Zip => {
                let zip = zip::ZipWriter::new(out);
                zip.finish()?;
            }
            ConvertFormat::TarGz => {
                let encoder = flate2::write::GzEncoder::new(out, flate2::Compression::new(level));
                let mut tar = tar::Builder::new(encoder);
                tar.finish()?;
            }
            ConvertFormat::TarXz => {
                let encoder = xz2::write::XzEncoder::new(out, level);
                let mut tar = tar::Builder::new(encoder);
                tar.finish()?;
            }
        }
        return Ok(0);
    }

    // Stage 2: collect unique frames needed, read + verify + decompress them
    let frame_size = header.frame_size_bytes;
    let mut unique_frames: HashSet<(u8, u32)> = HashSet::new();
    for entry in &all_entries {
        if entry.stream_raw_size == 0 {
            continue;
        }
        let start = (entry.stream_offset / frame_size) as u32;
        let end = ((entry.stream_offset + entry.stream_raw_size - 1) / frame_size) as u32;
        for fi in start..=end {
            unique_frames.insert((entry.group_id, fi));
        }
    }

    // Read compressed frames sequentially
    let mut frame_data_map: HashMap<(u8, u32), Vec<u8>> = HashMap::new();
    for &(group_id, frame_idx) in &unique_frames {
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

        // Verify checksum
        if hash(&buf) != entry.frame_checksum {
            return Err(SbkError::FrameChecksumMismatch(frame_idx).into());
        }

        frame_data_map.insert((group_id, frame_idx), buf);
    }

    // Decompress frames (using rayon for parallelism)
    use rayon::prelude::*;
    let frame_raw_sz_map: HashMap<(u8, u32), u32> = unique_frames
        .iter()
        .map(|&(group_id, frame_idx)| {
            let entry = &frame_dir.groups[group_id as usize][frame_idx as usize];
            ((group_id, frame_idx), entry.frame_raw_sz)
        })
        .collect();

    let decompressed_frames: HashMap<(u8, u32), Vec<u8>> = frame_data_map
        .par_iter()
        .map(|(&key, compressed)| {
            let expected_raw_sz = frame_raw_sz_map[&key];
            let raw = codec.decompress(compressed, expected_raw_sz).map_err(|e| {
                anyhow::anyhow!("frame ({},{}) decompression failed: {}", key.0, key.1, e)
            })?;
            Ok((key, raw))
        })
        .collect::<anyhow::Result<_>>()?;

    // Stage 3: write to target format
    let out = File::create(output)
        .with_context(|| format!("failed to create output '{}'", output.display()))?;

    let n = all_entries.len() as u64;

    match format {
        ConvertFormat::Zip => {
            write_zip(out, &all_entries, &decompressed_frames, frame_size, level)?
        }
        ConvertFormat::TarGz => {
            let encoder = flate2::write::GzEncoder::new(out, flate2::Compression::new(level));
            write_tar(encoder, &all_entries, &decompressed_frames, frame_size)?;
        }
        ConvertFormat::TarXz => {
            let encoder = xz2::write::XzEncoder::new(out, level);
            write_tar(encoder, &all_entries, &decompressed_frames, frame_size)?;
        }
    }

    Ok(n)
}

fn write_zip<W: Write + Seek>(
    out: W,
    entries: &[IndexEntry],
    frames: &HashMap<(u8, u32), Vec<u8>>,
    frame_size: u64,
    level: u32,
) -> anyhow::Result<()> {
    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;

    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(level as i64));

    let mut zip = zip::ZipWriter::new(out);
    for e in entries {
        let preprocessed = slice_from_frames(
            frames,
            e.group_id,
            e.stream_offset,
            e.stream_raw_size,
            frame_size,
        )?;
        let reconstructed = reconstruct_bytes(e.group_id, &preprocessed)
            .with_context(|| format!("failed to reconstruct '{}'", e.path))?;
        let opts = options.last_modified_time(mtime_to_zip_datetime(e.mtime_ms));
        zip.start_file(&e.path, opts)?;
        zip.write_all(&reconstructed)?;
    }
    zip.finish()?;
    Ok(())
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

fn write_tar<W: Write>(
    writer: W,
    entries: &[IndexEntry],
    frames: &HashMap<(u8, u32), Vec<u8>>,
    frame_size: u64,
) -> anyhow::Result<()> {
    let mut tar = tar::Builder::new(writer);
    for e in entries {
        let preprocessed = slice_from_frames(
            frames,
            e.group_id,
            e.stream_offset,
            e.stream_raw_size,
            frame_size,
        )?;
        let reconstructed = reconstruct_bytes(e.group_id, &preprocessed)
            .with_context(|| format!("failed to reconstruct '{}'", e.path))?;
        let mut header = tar::Header::new_gnu();
        header.set_size(reconstructed.len() as u64);
        // mtime in seconds; clamp negatives to 0
        let mtime_secs = if e.mtime_ms >= 0 {
            (e.mtime_ms / 1000) as u64
        } else {
            0
        };
        header.set_mtime(mtime_secs);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, &e.path, reconstructed.as_slice())?;
    }
    tar.finish()?;
    Ok(())
}
