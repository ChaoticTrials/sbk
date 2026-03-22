use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::checksum::hash;
use crate::classify::Group;
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
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global()
        .ok();

    // Stage 1: read header, frame dir, index
    let mut f = File::open(archive)?;
    let header = read_header(&mut f)?;

    f.seek(SeekFrom::Start(header.frame_dir_offset))?;
    let frame_dir = read_frame_dir(&mut f)?;

    f.seek(SeekFrom::Start(header.index_offset))?;
    let all_entries = read_index(&mut f, header.index_compressed_size, header.index_checksum)?;

    // Match patterns
    let matched = find_patterns(&all_entries, patterns)?;
    if matched.is_empty() {
        if patterns.len() == 1 && patterns[0] != "**" {
            return Err(SbkError::NoMatch(patterns[0].clone()).into());
        }
        // Empty match for "**" is valid (empty archive)
        return Ok(0);
    }

    // Collect unique frames needed
    let frame_size = header.frame_size_bytes;
    let mut unique_frames: HashSet<(u8, u32)> = HashSet::new();
    for entry in &matched {
        if entry.stream_raw_size == 0 {
            continue;
        }
        let start = (entry.stream_offset / frame_size) as u32;
        let end = ((entry.stream_offset + entry.stream_raw_size - 1) / frame_size) as u32;
        for fi in start..=end {
            unique_frames.insert((entry.group_id, fi));
        }
    }

    let mp = MultiProgress::new();
    let bar_style = ProgressStyle::with_template("{prefix} [{bar:40}] {pos}/{len} {wide_msg}")
        .unwrap()
        .progress_chars("=> ");

    let bar_decomp = mp.add(ProgressBar::new(unique_frames.len() as u64));
    bar_decomp.set_style(bar_style.clone());
    bar_decomp.set_prefix("Decompressing");

    // Stage 2: decompress unique frames in parallel
    let unique_frames_vec: Vec<(u8, u32)> = unique_frames.into_iter().collect();

    // Read archive bytes for parallel processing (we need to seek independently)
    // We'll read the needed compressed frame data first, then decompress in parallel
    let mut frame_data_map: HashMap<(u8, u32), Vec<u8>> = HashMap::new();
    for &(group_id, frame_idx) in &unique_frames_vec {
        let group_frames = &frame_dir.groups[group_id as usize];
        if frame_idx as usize >= group_frames.len() {
            return Err(anyhow::anyhow!(
                "frame index {} out of bounds for group {}",
                frame_idx,
                group_id
            ));
        }
        let entry = &group_frames[frame_idx as usize];
        let mut buf = vec![0u8; entry.frame_compressed_sz as usize];
        f.seek(SeekFrom::Start(entry.frame_offset))?;
        f.read_exact(&mut buf)?;

        // Verify checksum
        if hash(&buf) != entry.frame_checksum {
            return Err(SbkError::FrameChecksumMismatch(frame_idx).into());
        }

        frame_data_map.insert((group_id, frame_idx), buf);
    }

    bar_decomp.inc(unique_frames_vec.len() as u64);
    bar_decomp.finish();

    // Decompress frames in parallel
    let bar_decomp2 = mp.add(ProgressBar::new(frame_data_map.len() as u64));
    bar_decomp2.set_style(bar_style.clone());
    bar_decomp2.set_prefix("XZ Decode    ");

    let decompressed_frames: HashMap<(u8, u32), Vec<u8>> = frame_data_map
        .par_iter()
        .map(|(&key, compressed)| {
            use std::io::Read as IoRead;
            let mut dec = xz2::read::XzDecoder::new(&compressed[..]);
            let mut raw = Vec::new();
            dec.read_to_end(&mut raw)?;
            bar_decomp2.inc(1);
            Ok((key, raw))
        })
        .collect::<anyhow::Result<_>>()?;

    bar_decomp2.finish();

    // Stage 3: reconstruct files in parallel
    let bar_write = mp.add(ProgressBar::new(matched.len() as u64));
    bar_write.set_style(bar_style);
    bar_write.set_prefix("Writing      ");

    let created_dirs: Mutex<HashSet<PathBuf>> = Mutex::new(HashSet::new());

    let written: Vec<anyhow::Result<()>> = matched
        .par_iter()
        .map(|entry| {
            let preprocessed = slice_from_frames(
                &decompressed_frames,
                entry.group_id,
                entry.stream_offset,
                entry.stream_raw_size,
                frame_size,
            )?;

            let out_path = output_dir.join(&entry.path);

            // Create parent directory (with deduplication)
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

            // Reconstruct based on group
            let group = Group::from_u8(entry.group_id)
                .ok_or_else(|| anyhow::anyhow!("unknown group_id {}", entry.group_id))?;

            match group {
                Group::Mca => {
                    reconstruct_mca(&preprocessed, &out_path)?;
                }
                Group::Nbt => {
                    reconstruct_nbt(&preprocessed, &out_path)?;
                }
                Group::Json => {
                    reconstruct_json(&preprocessed, &out_path)?;
                }
                Group::Raw => {
                    std::fs::write(&out_path, &preprocessed)?;
                }
            }

            // Restore mtime
            let ft = filetime::FileTime::from_unix_time(
                entry.mtime_ms / 1000,
                ((entry.mtime_ms % 1000) * 1_000_000) as u32,
            );
            filetime::set_file_mtime(&out_path, ft)?;

            bar_write.inc(1);
            Ok(())
        })
        .collect();

    bar_write.finish();

    // Propagate first error
    for r in written {
        r?;
    }

    Ok(matched.len() as u64)
}
