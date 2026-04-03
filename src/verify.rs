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
/// Each frame is read and hashed individually, so peak memory usage is bounded
/// by the size of the largest single frame rather than the whole archive.
pub fn verify(archive: &Path, _threads: usize) -> anyhow::Result<bool> {
    let mut f = File::open(archive)?;
    let header = read_header(&mut f)?;
    let codec = codec::from_algorithm(header.algorithm);

    f.seek(SeekFrom::Start(header.frame_dir_offset))?;
    let frame_dir = read_frame_dir(&mut f)?;

    // Verify index checksum (read_index returns Err on mismatch).
    f.seek(SeekFrom::Start(header.index_offset))?;
    let _entries = read_index(
        &mut f,
        &*codec,
        header.index_compressed_size,
        header.index_checksum,
    )?;

    // Verify each frame's checksum sequentially.
    // Reading all frames at once would consume O(total compressed size) of RAM;
    // sequential verification keeps peak usage at O(largest single frame).
    let mut failed = 0u32;

    for (g, group_frames) in frame_dir.groups.iter().enumerate() {
        for (fi, entry) in group_frames.iter().enumerate() {
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
            if hash(&buf) != entry.frame_checksum {
                failed += 1;
            }
        }
    }

    Ok(failed == 0)
}
