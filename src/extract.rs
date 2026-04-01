use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use rayon::prelude::*;

use crate::checksum::hash;
use crate::classify::Group;
use crate::codec;
use crate::error::SbkError;
use crate::format::frame_dir::read_frame_dir;
use crate::format::header::read_header;
use crate::format::index::{find_patterns, read_index};
use crate::preprocess::{json::reconstruct_json, mca::reconstruct_mca, nbt::reconstruct_nbt};
use crate::solid::extractor::slice_from_frames;

pub fn extract(
    archive: &Path,
    patterns: &[String],
    output_dir: &Path,
    threads: usize,
) -> anyhow::Result<u64> {
    extract_impl(archive, patterns, output_dir, threads, |_, _, _| true, true)
}

/// Extraction with a progress callback for GUI use.
/// `on_progress(phase, completed, total)` is called after each frame/file is processed.
/// Returns `false` from the callback to cancel extraction.
pub fn extract_with_progress<F>(
    archive: &Path,
    patterns: &[String],
    output_dir: &Path,
    threads: usize,
    on_progress: F,
) -> anyhow::Result<u64>
where
    F: Fn(&str, usize, usize) -> bool + Send + Sync,
{
    extract_impl(archive, patterns, output_dir, threads, on_progress, false)
}

fn extract_impl<F>(
    archive: &Path,
    patterns: &[String],
    output_dir: &Path,
    threads: usize,
    on_progress: F,
    show_bars: bool,
) -> anyhow::Result<u64>
where
    F: Fn(&str, usize, usize) -> bool + Send + Sync,
{
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global()
        .ok();

    // Stage 1: read header, frame dir, index
    let mut f = File::open(archive)?;
    let header = read_header(&mut f)?;
    let codec = codec::from_algorithm(header.algorithm);
    let decode_label = match header.algorithm {
        crate::format::header::Algorithm::Lzma2 => "XZ Decode    ",
        crate::format::header::Algorithm::Zstd => "ZSTD Decode  ",
    };

    f.seek(SeekFrom::Start(header.frame_dir_offset))?;
    let frame_dir = read_frame_dir(&mut f)?;

    f.seek(SeekFrom::Start(header.index_offset))?;
    let all_entries = read_index(
        &mut f,
        &*codec,
        header.index_compressed_size,
        header.index_checksum,
    )?;

    // Match patterns
    let matched = find_patterns(&all_entries, patterns)?;

    // Validate that no archive entry path escapes the output directory (path traversal guard).
    for entry in &matched {
        // Reject absolute paths and paths containing `..` components.
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
    if matched.is_empty() {
        if patterns.len() == 1 && patterns[0] != "**" {
            return Err(SbkError::NoMatch(patterns[0].clone()).into());
        }
        // Empty match for "**" is valid (empty archive)
        return Ok(0);
    }

    // Collect unique frames needed, with per-entry tracking and reference counts.
    let frame_size = header.frame_size_bytes;
    let mut frames_needed: Vec<HashSet<(u8, u32)>> = Vec::with_capacity(matched.len());
    let mut frame_refcount: HashMap<(u8, u32), usize> = HashMap::new();

    for entry in &matched {
        let mut needed: HashSet<(u8, u32)> = HashSet::new();
        if entry.stream_raw_size > 0 {
            let start = (entry.stream_offset / frame_size) as u32;
            let end = ((entry.stream_offset + entry.stream_raw_size - 1) / frame_size) as u32;
            for fi in start..=end {
                let key = (entry.group_id, fi);
                needed.insert(key);
                *frame_refcount.entry(key).or_insert(0) += 1;
            }
        }
        frames_needed.push(needed);
    }

    let batch_size = threads.max(1);
    let mut unique_frames_sorted: Vec<(u8, u32)> = frame_refcount.keys().copied().collect();
    unique_frames_sorted.sort_unstable();
    let total_frames = unique_frames_sorted.len();

    let mp = if show_bars {
        MultiProgress::new()
    } else {
        MultiProgress::with_draw_target(ProgressDrawTarget::hidden())
    };
    let bar_style = ProgressStyle::with_template("{prefix} [{bar:40}] {pos}/{len} {wide_msg}")
        .unwrap()
        .progress_chars("=> ");

    let bar_decomp = mp.add(ProgressBar::new(total_frames as u64));
    bar_decomp.set_style(bar_style.clone());
    bar_decomp.set_prefix("Decompressing");

    let bar_decode = mp.add(ProgressBar::new(total_frames as u64));
    bar_decode.set_style(bar_style.clone());
    bar_decode.set_prefix(decode_label);

    let bar_write = mp.add(ProgressBar::new(matched.len() as u64));
    bar_write.set_style(bar_style.clone());
    bar_write.set_prefix("Writing      ");

    let mut decompressed_frames: HashMap<(u8, u32), Vec<u8>> = HashMap::new();
    let mut entry_written: Vec<bool> = vec![false; matched.len()];
    let total_files = matched.len();
    let completed = Arc::new(AtomicUsize::new(0));
    let created_dirs: Mutex<HashSet<PathBuf>> = Mutex::new(HashSet::new());
    let mut frames_read: usize = 0;
    let decode_count = AtomicUsize::new(0);

    for chunk in unique_frames_sorted.chunks(batch_size) {
        // --- Phase A: read compressed bytes sequentially ---
        let mut batch_compressed: Vec<((u8, u32), Vec<u8>)> = Vec::with_capacity(chunk.len());
        for &(group_id, frame_idx) in chunk {
            let group_frames = &frame_dir.groups[group_id as usize];
            if frame_idx as usize >= group_frames.len() {
                return Err(anyhow::anyhow!(
                    "frame index {} out of bounds for group {}",
                    frame_idx,
                    group_id
                ));
            }
            let entry = &group_frames[frame_idx as usize];
            // Sanity-check compressed size before allocating (guards against corrupt/malicious archives).
            // 256 MiB is a generous upper bound; real compressed frames are typically far smaller.
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
            batch_compressed.push(((group_id, frame_idx), buf));
            bar_decomp.inc(1);
            frames_read += 1;
            if !on_progress("decompress", frames_read, total_frames) {
                return Err(anyhow::anyhow!("cancelled"));
            }
        }

        // --- Phase B: decompress batch in parallel ---
        let batch_raw_sz: HashMap<(u8, u32), u32> = batch_compressed
            .iter()
            .map(|((gid, fi), _)| {
                let sz = frame_dir.groups[*gid as usize][*fi as usize].frame_raw_sz;
                ((*gid, *fi), sz)
            })
            .collect();

        let batch_decompressed: Vec<((u8, u32), Vec<u8>)> = batch_compressed
            .par_iter()
            .map(|(key, compressed)| {
                let expected_raw_sz = batch_raw_sz[key];
                let raw = codec.decompress(compressed, expected_raw_sz).map_err(|e| {
                    anyhow::anyhow!("frame ({},{}) decompression failed: {}", key.0, key.1, e)
                })?;
                bar_decode.inc(1);
                let n = decode_count.fetch_add(1, Ordering::Relaxed) + 1;
                if !on_progress("decode", n, total_frames) {
                    return Err(anyhow::anyhow!("cancelled"));
                }
                Ok((*key, raw))
            })
            .collect::<anyhow::Result<_>>()?;

        for (key, raw) in batch_decompressed {
            decompressed_frames.insert(key, raw);
        }
        // batch_compressed drops here, freeing compressed bytes for this batch.

        // --- Phase C: write every entry whose frames are now fully available ---
        let ready_indices: Vec<usize> = entry_written
            .iter()
            .enumerate()
            .filter(|&(i, &written)| {
                !written
                    && frames_needed[i]
                        .iter()
                        .all(|key| decompressed_frames.contains_key(key))
            })
            .map(|(i, _)| i)
            .collect();

        let write_results: Vec<anyhow::Result<()>> = ready_indices
            .par_iter()
            .map(|&i| {
                let entry = &matched[i];
                let preprocessed = slice_from_frames(
                    &decompressed_frames,
                    entry.group_id,
                    entry.stream_offset,
                    entry.stream_raw_size,
                    frame_size,
                )?;
                let out_path = output_dir.join(&entry.path);
                if let Some(parent) = out_path.parent() {
                    let needs_create = {
                        let dirs = created_dirs.lock().unwrap();
                        !dirs.contains(parent)
                    };
                    if needs_create {
                        std::fs::create_dir_all(parent)?;
                        let mut dirs = created_dirs.lock().unwrap();
                        dirs.insert(parent.to_path_buf());
                    }
                }
                let group = Group::from_u8(entry.group_id)
                    .ok_or_else(|| anyhow::anyhow!("unknown group_id {}", entry.group_id))?;
                match group {
                    Group::Mca => reconstruct_mca(&preprocessed, &out_path)?,
                    Group::Nbt => reconstruct_nbt(&preprocessed, &out_path)?,
                    Group::Json => reconstruct_json(&preprocessed, &out_path)?,
                    Group::Raw => std::fs::write(&out_path, &preprocessed)?,
                }
                let ft = filetime::FileTime::from_unix_time(
                    entry.mtime_ms / 1000,
                    ((entry.mtime_ms % 1000) * 1_000_000) as u32,
                );
                filetime::set_file_mtime(&out_path, ft)?;
                bar_write.inc(1);
                let n = completed.fetch_add(1, Ordering::Relaxed) + 1;
                if !on_progress("write", n, total_files) {
                    return Err(anyhow::anyhow!("cancelled"));
                }
                Ok(())
            })
            .collect();

        for r in write_results {
            r?;
        }

        for &i in &ready_indices {
            entry_written[i] = true;
        }

        // --- Phase D: evict frames no longer needed by any unwritten entry ---
        for &i in &ready_indices {
            for key in &frames_needed[i] {
                if let Some(count) = frame_refcount.get_mut(key) {
                    *count -= 1;
                    if *count == 0 {
                        decompressed_frames.remove(key);
                    }
                }
            }
        }
    }

    bar_decomp.finish();
    bar_decode.finish();
    bar_write.finish();

    Ok(matched.len() as u64)
}
