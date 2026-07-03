use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::checksum::hash;
use crate::codec;
use crate::format::frame_dir::read_frame_dir;
use crate::format::header::read_header;
use crate::format::index::read_index;

/// Maximum allowed compressed frame size for verification (256 MiB).
/// Guards against OOM when allocating a frame buffer from an untrusted archive.
const MAX_FRAME_COMPRESSED_SIZE: u32 = 256 * 1024 * 1024;

/// Verify all frame checksums and the index checksum in the archive.
/// Returns `Ok(true)` if everything passes, `Ok(false)` if any checksum fails.
///
/// Frames are read sequentially in batches of `threads` and hashed in
/// parallel, so peak memory usage is bounded by one batch of compressed
/// frames rather than the whole archive.
pub fn verify(archive: &Path, threads: usize) -> anyhow::Result<bool> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global()
        .ok();

    let mut f = File::open(archive)?;
    let header = read_header(&mut f)?;
    let codec = codec::from_algorithm(header.algorithm);

    f.seek(SeekFrom::Start(header.frame_dir_offset))?;
    let frame_dir = read_frame_dir(&mut f, header.frame_size_bytes)?;

    // Verify index checksum (read_index returns Err on mismatch).
    f.seek(SeekFrom::Start(header.index_offset))?;
    let _entries = read_index(
        &mut f,
        &*codec,
        header.index_compressed_size,
        header.index_checksum,
    )?;

    // Verify frame checksums: read each batch sequentially, hash in parallel.
    // Reading all frames at once would consume O(total compressed size) of RAM;
    // batching keeps peak usage at O(threads × largest single frame).
    use rayon::prelude::*;

    let batch_size = threads.max(1);
    let mut failed = 0usize;

    for (g, group_frames) in frame_dir.groups.iter().enumerate() {
        for (chunk_idx, chunk) in group_frames.chunks(batch_size).enumerate() {
            // Phase A: sequential reads
            let mut bufs: Vec<(u32, Vec<u8>)> = Vec::with_capacity(chunk.len());
            for (j, entry) in chunk.iter().enumerate() {
                let fi = chunk_idx * batch_size + j;
                if entry.frame_compressed_sz > MAX_FRAME_COMPRESSED_SIZE {
                    return Err(anyhow::anyhow!(
                        "Frame {} in group {} has compressed size {} B which exceeds the \
                         {}-B safety limit; the archive may be corrupt",
                        fi,
                        g,
                        entry.frame_compressed_sz,
                        MAX_FRAME_COMPRESSED_SIZE,
                    ));
                }
                let mut buf = vec![0u8; entry.frame_compressed_sz as usize];
                f.seek(SeekFrom::Start(entry.frame_offset))?;
                f.read_exact(&mut buf)?;
                bufs.push((entry.frame_checksum, buf));
            }

            // Phase B: hash the batch in parallel
            failed += bufs
                .par_iter()
                .filter(|(expected, buf)| hash(buf) != *expected)
                .count();
        }
    }

    Ok(failed == 0)
}
