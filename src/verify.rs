use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use rayon::prelude::*;

use crate::checksum::hash;
use crate::codec;
use crate::format::frame_dir::read_frame_dir;
use crate::format::header::read_header;
use crate::format::index::read_index;

/// Verify all frame checksums in the archive.
/// Returns Ok(true) if all pass, Ok(false) if any fail.
pub fn verify(archive: &Path, threads: usize) -> anyhow::Result<bool> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global()
        .ok();

    let mut f = File::open(archive)?;
    let header = read_header(&mut f)?;
    let codec = codec::from_algorithm(header.algorithm);

    f.seek(SeekFrom::Start(header.frame_dir_offset))?;
    let frame_dir = read_frame_dir(&mut f)?;

    // Also verify the index checksum
    f.seek(SeekFrom::Start(header.index_offset))?;
    let _entries = read_index(
        &mut f,
        &*codec,
        header.index_compressed_size,
        header.index_checksum,
    )?;

    // Collect all frames to verify
    let mut frames_to_check: Vec<(u8, u32, u64, u32, u32)> = Vec::new(); // (group, frame_idx, offset, compressed_sz, expected_checksum)
    for (g, group_frames) in frame_dir.groups.iter().enumerate() {
        for (fi, entry) in group_frames.iter().enumerate() {
            frames_to_check.push((
                g as u8,
                fi as u32,
                entry.frame_offset,
                entry.frame_compressed_sz,
                entry.frame_checksum,
            ));
        }
    }

    // Read all frame data (sequential — file seeking)
    let mut frame_buffers: Vec<(u8, u32, Vec<u8>, u32)> = Vec::new(); // (group, idx, data, expected)
    for (group, fi, offset, sz, expected) in &frames_to_check {
        let mut buf = vec![0u8; *sz as usize];
        f.seek(SeekFrom::Start(*offset))?;
        f.read_exact(&mut buf)?;
        frame_buffers.push((*group, *fi, buf, *expected));
    }

    // Verify checksums in parallel
    let failures: Vec<(u8, u32)> = frame_buffers
        .par_iter()
        .filter_map(|(group, fi, data, expected)| {
            let computed = hash(data);
            if computed != *expected {
                Some((*group, *fi))
            } else {
                None
            }
        })
        .collect();

    if failures.is_empty() {
        println!("All {} frame(s) OK.", frames_to_check.len());
        Ok(true)
    } else {
        for (group, fi) in &failures {
            eprintln!("FAIL: frame {} in group {} checksum mismatch", fi, group);
        }
        println!(
            "{} frame(s) failed verification out of {}.",
            failures.len(),
            frames_to_check.len()
        );
        Ok(false)
    }
}
