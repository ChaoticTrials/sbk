use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use rayon::prelude::*;
use walkdir::WalkDir;

use crate::checksum::hash;
use crate::classify::{Group, classify};
use crate::codec::{self, Codec};
use crate::filter::{CompressOptions, accept, capture_now_ms};
use crate::format::frame_dir::{FrameDir, FrameEntry, write_frame_dir};
use crate::format::header::{HEADER_DISK_SIZE, Header, write_header, write_placeholder};
use crate::format::index::{IndexEntry, write_index};
use crate::preprocess::json::preprocess_json_from_bytes;
use crate::preprocess::mca::preprocess_mca_from_bytes;
use crate::preprocess::nbt::preprocess_nbt_from_bytes;
use crate::solid::FRAME_SIZE;

#[derive(Clone)]
struct FileInfo {
    abs_path: std::path::PathBuf,
    rel_path: String,
    mtime_ms: i64,
}

/// Outcome of reading + preprocessing a single file in the parallel stage.
enum PreprocessOutcome {
    /// File was read and preprocessed; ready to be appended to the group stream.
    Ready {
        original_size: u64,
        file_checksum: u32,
        preprocessed: Vec<u8>,
    },
    /// Preprocessing failed; the file must be stored verbatim in the RAW group.
    Deferred,
    /// File could not be read; it is dropped from the archive.
    Skipped,
}

/// Deletes the in-progress `.part` file on drop unless it was renamed into
/// place (signalled by setting `path` to `None`).
struct PartFileGuard {
    path: Option<PathBuf>,
}

impl Drop for PartFileGuard {
    fn drop(&mut self) {
        if let Some(p) = &self.path {
            let _ = std::fs::remove_file(p);
        }
    }
}

pub fn compress(world_dir: &Path, opts: &CompressOptions) -> anyhow::Result<()> {
    compress_impl(world_dir, opts, |_, _, _| {}, false)
}

/// Compression with a progress callback for GUI use.
/// `on_progress(completed, total, current_path)` is called after each file is processed.
pub fn compress_with_progress<F>(
    world_dir: &Path,
    opts: &CompressOptions,
    on_progress: F,
) -> anyhow::Result<()>
where
    F: Fn(usize, usize, &str),
{
    compress_impl(world_dir, opts, on_progress, true)
}

fn compress_impl<F>(
    world_dir: &Path,
    opts: &CompressOptions,
    on_file_preprocessed: F,
    hide_bars: bool,
) -> anyhow::Result<()>
where
    F: Fn(usize, usize, &str),
{
    let now_ms = capture_now_ms();
    let codec: Box<dyn Codec> = codec::from_algorithm(opts.algorithm);

    rayon::ThreadPoolBuilder::new()
        .num_threads(opts.threads)
        .build_global()
        .ok();

    let mp = if opts.quiet || hide_bars {
        MultiProgress::with_draw_target(ProgressDrawTarget::hidden())
    } else {
        MultiProgress::new()
    };
    let bar_style = ProgressStyle::with_template("{prefix} [{bar:40}] {pos}/{len} {wide_msg}")
        .unwrap()
        .progress_chars("=> ");

    // === Pass 1: enumerate + filter (metadata only, no file reads) ===
    let mut files_by_group: [Vec<FileInfo>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    let mut total_scanned = 0u64;
    let mut skipped = 0u64;

    // Resolve the output's final absolute path so a pre-existing archive inside
    // world_dir is never included in itself. The file may not exist yet, so
    // canonicalize its parent and re-join the file name.
    let output_abs: Option<PathBuf> = opts.output.file_name().and_then(|name| {
        let parent = opts
            .output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        parent.canonicalize().ok().map(|p| p.join(name))
    });

    for entry in WalkDir::new(world_dir) {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                if !opts.quiet {
                    let path = e
                        .path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "<unknown>".to_string());
                    eprintln!("Warning: cannot access {}: {}", path, e);
                }
                skipped += 1;
                continue;
            }
        };
        if entry.file_type().is_symlink() {
            if !opts.quiet {
                eprintln!(
                    "Warning: skipping symlink {} (symlinks are not archived)",
                    entry.path().display()
                );
            }
            skipped += 1;
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }

        total_scanned += 1;
        let abs_path = entry.path().to_path_buf();
        let rel = abs_path
            .strip_prefix(world_dir)
            .unwrap_or(&abs_path)
            .to_string_lossy()
            .replace('\\', "/");

        // Never include the output archive in itself (e.g. re-running a backup
        // whose output path lies inside the world directory).
        if let Some(out_abs) = &output_abs {
            if abs_path.file_name() == out_abs.file_name()
                && abs_path.canonicalize().is_ok_and(|p| &p == out_abs)
            {
                if !opts.quiet {
                    eprintln!(
                        "Warning: skipping {} (it is the output archive)",
                        rel
                    );
                }
                skipped += 1;
                continue;
            }
        }

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                if !opts.quiet {
                    eprintln!("Warning: metadata failed for {}: {}", rel, e);
                }
                skipped += 1;
                continue;
            }
        };
        let ft = filetime::FileTime::from_last_modification_time(&meta);
        let mtime_ms = ft.unix_seconds() * 1000 + ft.nanoseconds() as i64 / 1_000_000;

        if !accept(
            &rel,
            mtime_ms,
            now_ms,
            opts.max_age,
            opts.since,
            &opts.patterns,
            opts.include_session_lock,
        ) {
            skipped += 1;
            continue;
        }

        let group = classify(&abs_path);
        files_by_group[group as usize].push(FileInfo {
            abs_path,
            rel_path: rel,
            mtime_ms,
        });
    }

    // Sort each group by rel_path for deterministic output regardless of thread count.
    for g in 0..4 {
        files_by_group[g].sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    }

    let total_included: u64 = files_by_group.iter().map(|v| v.len() as u64).sum();

    let bar_pre = mp.add(ProgressBar::new(total_included));
    bar_pre.set_style(bar_style);
    bar_pre.set_prefix("Preprocessing");

    let bar_comp = mp.add(ProgressBar::new(0));
    bar_comp.set_style(ProgressStyle::with_template("{prefix} {pos} frames").unwrap());
    bar_comp.set_prefix("Compressing  ");

    // === Pass 2: open output, write placeholder header, then stream frames ===
    // Frame directory is written AFTER all frame data so we don't need to know
    // frame counts upfront. The reader seeks to frame_dir_offset from the header.
    //
    // The archive is written to a `.part` file next to the output and only
    // renamed into place once the final header is written, so a mid-run failure
    // never destroys a pre-existing archive at the output path. A plain file
    // (not NamedTempFile) is used so permissions follow the umask instead of
    // the 0600 that temp files get. The guard deletes the partial file on error.
    let part_path = {
        let mut name = opts
            .output
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("output path has no file name"))?
            .to_os_string();
        name.push(".part");
        opts.output.with_file_name(name)
    };
    let mut part_guard = PartFileGuard {
        path: Some(part_path.clone()),
    };
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&part_path)?;

    write_placeholder(&mut out)?;
    let mut current_offset: u64 = HEADER_DISK_SIZE as u64;

    let mut fd = FrameDir::new();
    let mut all_index_entries: Vec<IndexEntry> = Vec::new();

    let batch_size = opts.threads.max(1);
    let mut files_done: usize = 0;

    // Files whose group-specific preprocessing fails are collected here and
    // stored verbatim in the RAW group instead, so extraction never has to
    // run a group-specific reconstruction on bytes that were not preprocessed.
    let mut deferred_raw: Vec<FileInfo> = Vec::new();

    for g in 0..4usize {
        let files = if g == Group::Raw as usize {
            let mut v = std::mem::take(&mut files_by_group[g]);
            v.append(&mut deferred_raw);
            // Re-sort so deferred files keep output byte-identical and the
            // index deterministic regardless of thread count.
            v.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
            v
        } else {
            std::mem::take(&mut files_by_group[g])
        };
        if files.is_empty() {
            continue;
        }

        let group_enum = Group::from_u8(g as u8).unwrap();
        let mut frame_buf: Vec<u8> = Vec::with_capacity(FRAME_SIZE as usize);
        let mut frame_batch: Vec<(Vec<u8>, u32)> = Vec::new(); // (raw_bytes, raw_size)
        let mut stream_offset: u64 = 0;

        // Process files in chunks of batch_size: preprocess each chunk in parallel,
        // then drain results in sorted order into the frame buffer.
        let quiet = opts.quiet;
        for chunk in files.chunks(batch_size) {
            // Parallel preprocessing: read + hash + preprocess each file concurrently.
            let results: Vec<PreprocessOutcome> = chunk
                .par_iter()
                .map(|fi| {
                    let file_bytes = match std::fs::read(&fi.abs_path) {
                        Ok(b) => b,
                        Err(e) => {
                            if !quiet {
                                eprintln!("Warning: could not read {}: {}", fi.rel_path, e);
                            }
                            return PreprocessOutcome::Skipped;
                        }
                    };
                    let original_size = file_bytes.len() as u64;
                    let file_checksum = hash(&file_bytes);
                    let preprocessed = if group_enum == Group::Raw {
                        file_bytes
                    } else {
                        // Empty files cannot survive group-specific reconstruction
                        // (e.g. NBT re-gzips unconditionally); store them raw.
                        if file_bytes.is_empty() {
                            return PreprocessOutcome::Deferred;
                        }
                        let result = match group_enum {
                            Group::Mca => preprocess_mca_from_bytes(&file_bytes),
                            Group::Nbt => preprocess_nbt_from_bytes(&file_bytes),
                            Group::Json => preprocess_json_from_bytes(&file_bytes),
                            Group::Raw => unreachable!(),
                        };
                        match result {
                            Ok(p) => p,
                            Err(e) => {
                                if !quiet {
                                    eprintln!(
                                        "Warning: {:?} preprocess failed for {}: {} — storing file raw",
                                        group_enum, fi.rel_path, e
                                    );
                                }
                                return PreprocessOutcome::Deferred;
                            }
                        }
                    };
                    PreprocessOutcome::Ready {
                        original_size,
                        file_checksum,
                        preprocessed,
                    }
                })
                .collect();

            // Drain results in order into the frame buffer.
            for (fi, outcome) in chunk.iter().zip(results) {
                let (original_size, file_checksum, preprocessed) = match outcome {
                    PreprocessOutcome::Ready {
                        original_size,
                        file_checksum,
                        preprocessed,
                    } => (original_size, file_checksum, preprocessed),
                    PreprocessOutcome::Deferred => {
                        // Re-queued into the RAW group; progress is counted
                        // when the file is actually drained there.
                        deferred_raw.push(fi.clone());
                        continue;
                    }
                    PreprocessOutcome::Skipped => {
                        bar_pre.inc(1);
                        continue;
                    }
                };

                let file_stream_offset = stream_offset;
                let file_stream_raw_size = preprocessed.len() as u64;
                stream_offset += file_stream_raw_size;

                let mut rem = preprocessed.as_slice();
                while !rem.is_empty() {
                    let space = FRAME_SIZE as usize - frame_buf.len();
                    let n = space.min(rem.len());
                    frame_buf.extend_from_slice(&rem[..n]);
                    rem = &rem[n..];

                    if frame_buf.len() == FRAME_SIZE as usize {
                        let raw = std::mem::take(&mut frame_buf);
                        frame_buf = Vec::with_capacity(FRAME_SIZE as usize);
                        frame_batch.push((raw, FRAME_SIZE as u32));

                        if frame_batch.len() >= batch_size {
                            current_offset = flush_frame_batch(
                                &mut out,
                                &mut fd.groups[g],
                                &mut frame_batch,
                                &*codec,
                                opts.level,
                                current_offset,
                                &bar_comp,
                            )?;
                        }
                    }
                }
                drop(preprocessed);

                all_index_entries.push(IndexEntry {
                    path: fi.rel_path.clone(),
                    mtime_ms: fi.mtime_ms,
                    group_id: g as u8,
                    stream_offset: file_stream_offset,
                    stream_raw_size: file_stream_raw_size,
                    original_size,
                    file_checksum,
                });
                bar_pre.inc(1);
                files_done += 1;
                on_file_preprocessed(files_done, total_included as usize, &fi.rel_path);
            }
        }

        // Flush the remaining partial frame and any pending batch.
        if !frame_buf.is_empty() {
            let raw_size = frame_buf.len() as u32;
            frame_batch.push((std::mem::take(&mut frame_buf), raw_size));
        }
        if !frame_batch.is_empty() {
            current_offset = flush_frame_batch(
                &mut out,
                &mut fd.groups[g],
                &mut frame_batch,
                &*codec,
                opts.level,
                current_offset,
                &bar_comp,
            )?;
        }
    }

    bar_pre.finish();
    bar_comp.finish();

    // === Write frame directory (after all frame data) ===
    let frame_dir_offset = current_offset;
    let frame_dir_size = fd.disk_size();
    write_frame_dir(&mut out, &fd)?;

    // === Write index ===
    all_index_entries.sort_by(|a, b| a.path.cmp(&b.path));
    let index_offset = frame_dir_offset + frame_dir_size;
    let (index_compressed_size, index_raw_size, index_checksum) =
        write_index(&all_index_entries, &*codec, opts.level, &mut out)?;

    // === Seek back and write the final header ===
    out.seek(SeekFrom::Start(0))?;
    // Use the actual number of index entries written, not the pre-filter count.
    // Files that fail to read or preprocess are silently skipped and not added
    // to all_index_entries, so total_included may be higher than the truth.
    let actual_file_count = all_index_entries.len() as u64;

    let header = Header {
        format_version: 1,
        flags: 0,
        algorithm: opts.algorithm,
        file_count: actual_file_count,
        frame_size_bytes: FRAME_SIZE,
        frame_dir_offset,
        frame_dir_size,
        index_offset,
        index_compressed_size,
        index_raw_size,
        index_checksum,
    };
    write_header(&mut out, &header)?;

    // Atomically move the finished archive into place. Until this point a
    // pre-existing file at the output path is untouched.
    out.flush()?;
    drop(out);
    std::fs::rename(&part_path, &opts.output)?;
    part_guard.path = None; // renamed away; nothing to clean up

    // === Print summary ===
    if !opts.quiet && !hide_bars {
        println!(
            "Scanned {} files → included {}  ({} skipped by filters)",
            total_scanned, actual_file_count, skipped
        );
        if let Some(age) = opts.max_age {
            let cutoff = now_ms - age as i64;
            println!("--max-age {} ms  →  cutoff timestamp: {}", age, cutoff);
        }
        if let Some(since) = opts.since {
            println!("--since {}", since);
        }
    }

    Ok(())
}

/// Compress all frames in `batch` in parallel, write them in order to `out`,
/// append a `FrameEntry` for each, clear the batch, and return the updated offset.
fn flush_frame_batch(
    out: &mut impl Write,
    entries: &mut Vec<FrameEntry>,
    batch: &mut Vec<(Vec<u8>, u32)>,
    codec: &dyn Codec,
    level: u32,
    mut current_offset: u64,
    bar: &ProgressBar,
) -> anyhow::Result<u64> {
    // Compress all frames in the batch concurrently.
    let compressed: Vec<Vec<u8>> = batch
        .par_iter()
        .map(|(raw, _)| codec.compress(raw, level))
        .collect::<anyhow::Result<_>>()?;

    // Write in order so frame offsets are correct.
    for ((_, raw_size), compressed_data) in batch.iter().zip(&compressed) {
        let checksum = hash(compressed_data);
        entries.push(FrameEntry {
            frame_offset: current_offset,
            frame_compressed_sz: compressed_data.len() as u32,
            frame_raw_sz: *raw_size,
            frame_checksum: checksum,
        });
        out.write_all(compressed_data)?;
        current_offset += compressed_data.len() as u64;
        bar.inc(1);
    }

    batch.clear();
    Ok(current_offset)
}
