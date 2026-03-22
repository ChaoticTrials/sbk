use std::io::{Read, Write};

use crate::classify::Group;

/// Per-frame entry in the Frame Directory. 20 bytes on disk.
#[derive(Debug, Clone)]
pub struct FrameEntry {
    pub frame_offset: u64,        // absolute byte offset in the archive file
    pub frame_compressed_sz: u32, // compressed size in bytes
    pub frame_raw_sz: u32,        // decompressed size in bytes
    pub frame_checksum: u32,      // xxHash32 of compressed bytes
}

/// Frame Directory: per-group frame metadata.
#[derive(Debug, Clone)]
pub struct FrameDir {
    pub groups: [Vec<FrameEntry>; 4],
}

impl FrameDir {
    pub fn new() -> Self {
        FrameDir {
            groups: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
        }
    }

    pub fn frames_for_group(&self, g: Group) -> &[FrameEntry] {
        &self.groups[g as u8 as usize]
    }

    /// Compute on-disk byte size of this frame directory.
    pub fn disk_size(&self) -> u64 {
        let mut size = 0u64;
        for g in &self.groups {
            size += 4; // frame_count: u32
            size += g.len() as u64 * 20; // 20 bytes per FrameEntry
        }
        size
    }
}

impl Default for FrameDir {
    fn default() -> Self {
        Self::new()
    }
}

/// Write the Frame Directory to `w`.
pub fn write_frame_dir(w: &mut impl Write, fd: &FrameDir) -> anyhow::Result<()> {
    for group_frames in &fd.groups {
        let count = group_frames.len() as u32;
        w.write_all(&count.to_le_bytes())?;
        for entry in group_frames {
            w.write_all(&entry.frame_offset.to_le_bytes())?;
            w.write_all(&entry.frame_compressed_sz.to_le_bytes())?;
            w.write_all(&entry.frame_raw_sz.to_le_bytes())?;
            w.write_all(&entry.frame_checksum.to_le_bytes())?;
        }
    }
    Ok(())
}

/// Maximum number of frames per group we'll accept from an archive.
/// 1 MiB frames × 1 M frames = 1 PiB of raw data, far beyond any real archive.
const MAX_FRAMES_PER_GROUP: usize = 1_000_000;

/// Read the Frame Directory from `r`.
pub fn read_frame_dir(r: &mut impl Read) -> anyhow::Result<FrameDir> {
    let mut fd = FrameDir::new();
    for g in 0..4usize {
        let mut count_buf = [0u8; 4];
        r.read_exact(&mut count_buf)?;
        let count = u32::from_le_bytes(count_buf) as usize;
        if count > MAX_FRAMES_PER_GROUP {
            return Err(anyhow::anyhow!(
                "frame count {} for group {} exceeds sanity limit {}",
                count,
                g,
                MAX_FRAMES_PER_GROUP
            ));
        }
        fd.groups[g] = Vec::with_capacity(count);
        for _ in 0..count {
            let mut buf = [0u8; 20];
            r.read_exact(&mut buf)?;
            let frame_offset = u64::from_le_bytes(buf[0..8].try_into().unwrap());
            let frame_compressed_sz = u32::from_le_bytes(buf[8..12].try_into().unwrap());
            let frame_raw_sz = u32::from_le_bytes(buf[12..16].try_into().unwrap());
            let frame_checksum = u32::from_le_bytes(buf[16..20].try_into().unwrap());
            fd.groups[g].push(FrameEntry {
                frame_offset,
                frame_compressed_sz,
                frame_raw_sz,
                frame_checksum,
            });
        }
    }
    Ok(fd)
}
