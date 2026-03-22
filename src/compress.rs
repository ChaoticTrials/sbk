use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use rayon::prelude::*;
use walkdir::WalkDir;

use crate::checksum::hash;
use crate::classify::{classify, Group};
use crate::filter::{accept, capture_now_ms, CompressOptions};
use crate::format::frame_dir::{write_frame_dir, FrameDir, FrameEntry};
use crate::format::header::{write_header, write_placeholder, Header, HEADER_DISK_SIZE};
use crate::format::index::{write_index, IndexEntry};
use crate::preprocess::json::preprocess_json_from_bytes;
use crate::preprocess::mca::preprocess_mca_from_bytes;
use crate::preprocess::nbt::preprocess_nbt_from_bytes;
use crate::solid::compressor::{compress_frame_data, FRAME_SIZE};

struct FileInfo {
    abs_path: std::path::PathBuf,
    rel_path: String,
    mtime_ms: i64,
}

pub fn compress(world_dir: &Path, opts: &CompressOptions) -> anyhow::Result<()> {
    let now_ms = capture_now_ms();

    rayon::ThreadPoolBuilder::new()
        .num_threads(opts.threads)
        .build_global()
        .ok();

    let mp = if opts.quiet {
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

    for entry in WalkDir::new(world_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        total_scanned += 1;
        let abs_path = entry.path().to_path_buf();
        let rel = abs_path
            .strip_prefix(world_dir)
            .unwrap_or(&abs_path)
            .to_string_lossy()
            .replace('\\', "/");

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Warning: metadata failed for {}: {}", rel, e);
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
    let mut out = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&opts.output)?;

    write_placeholder(&mut out)?;
    let mut current_offset: u64 = HEADER_DISK_SIZE as u64;

    let mut fd = FrameDir::new();
    let mut all_index_entries: Vec<IndexEntry> = Vec::new();

    let batch_size = opts.threads.max(1);

    for g in 0..4usize {
        let files = &files_by_group[g];
        if files.is_empty() {
            continue;
        }

        let group_enum = Group::from_u8(g as u8).unwrap();
        let mut frame_buf: Vec<u8> = Vec::with_capacity(FRAME_SIZE as usize);
        let mut frame_batch: Vec<(Vec<u8>, u32)> = Vec::new(); // (raw_bytes, raw_size)
        let mut stream_offset: u64 = 0;

        // Process files in chunks of batch_size: preprocess each chunk in parallel,
        // then drain results in sorted order into the frame buffer.
        for chunk in files.chunks(batch_size) {
            // Parallel preprocessing: read + hash + preprocess each file concurrently.
            let results: Vec<Option<(u64, u32, Vec<u8>)>> = chunk
                .par_iter()
                .map(|fi| {
                    let file_bytes = match std::fs::read(&fi.abs_path) {
                        Ok(b) => b,
                        Err(e) => {
                            eprintln!("Warning: could not read {}: {}", fi.rel_path, e);
                            return None;
                        }
                    };
                    let original_size = file_bytes.len() as u64;
                    let file_checksum = hash(&file_bytes);
                    let preprocessed = match group_enum {
                        Group::Mca => preprocess_mca_from_bytes(&file_bytes).unwrap_or_else(|e| {
                            eprintln!("Warning: MCA preprocess failed for {}: {}", fi.rel_path, e);
                            file_bytes.clone()
                        }),
                        Group::Nbt => preprocess_nbt_from_bytes(&file_bytes).unwrap_or_else(|e| {
                            eprintln!("Warning: NBT preprocess failed for {}: {}", fi.rel_path, e);
                            file_bytes.clone()
                        }),
                        Group::Json => {
                            preprocess_json_from_bytes(&file_bytes).unwrap_or_else(|e| {
                                eprintln!(
                                    "Warning: JSON preprocess failed for {}: {}",
                                    fi.rel_path, e
                                );
                                file_bytes.clone()
                            })
                        }
                        Group::Raw => file_bytes.clone(),
                    };
                    Some((original_size, file_checksum, preprocessed))
                })
                .collect();

            // Drain results in order into the frame buffer.
            for (fi, opt) in chunk.iter().zip(results) {
                let Some((original_size, file_checksum, preprocessed)) = opt else {
                    bar_pre.inc(1);
                    continue;
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
        write_index(&all_index_entries, opts.level, &mut out)?;

    // === Seek back and write the final header ===
    out.seek(SeekFrom::Start(0))?;
    let header = Header {
        format_version: 1,
        flags: 0,
        reserved: 0,
        file_count: total_included,
        frame_size_bytes: FRAME_SIZE,
        frame_dir_offset,
        frame_dir_size,
        index_offset,
        index_compressed_size,
        index_raw_size,
        index_checksum,
    };
    write_header(&mut out, &header)?;

    // === Print summary ===
    if !opts.quiet {
        println!(
            "Scanned {} files → included {}  ({} skipped by filters)",
            total_scanned, total_included, skipped
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
    preset: u32,
    mut current_offset: u64,
    bar: &ProgressBar,
) -> anyhow::Result<u64> {
    // Compress all frames in the batch concurrently.
    let compressed: Vec<Vec<u8>> = batch
        .par_iter()
        .map(|(raw, _)| compress_frame_data(raw, preset))
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
