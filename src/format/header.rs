use std::io::{Read, Seek, SeekFrom, Write};

use crate::checksum::hash;
use crate::error::SbkError;

/// File magic: "SBK!V1\r\n"
pub const MAGIC: [u8; 8] = [0x53, 0x42, 0x4B, 0x21, 0x56, 0x31, 0x0D, 0x0A];

/// On-disk header layout (all fields after magic, relative offsets within section starting at byte 8):
///
///   File  Rel   Size  Field
///   byte  off
///   8     0     1     format_version
///   9     1     1     flags
///   10    2     2     reserved (u16)
///   12    4     8     file_count (u64)
///   20    12    8     frame_size_bytes (u64)
///   28    20    8     frame_dir_offset (u64)
///   36    28    8     frame_dir_size (u64)
///   44    36    8     index_offset (u64)
///   52    44    8     index_compressed_size (u64)
///   60    52    8     index_raw_size (u64)
///   68    60    4     index_checksum (u32)
///   72    64    4     header_checksum (u32) = xxHash32(bytes 0..68 with this field = 0)
///
/// Total on disk = 76 bytes (8 magic + 68 header section).
/// The spec table says "relative offset 64" for header_checksum within the 60-byte section,
/// but 1+1+2+8+8+8+8+8+8+4+4 = 60 doesn't fit in 60 — the section is actually 68 bytes.
/// We implement the layout exactly as the table describes.
pub const HEADER_DISK_SIZE: usize = 76;

#[derive(Debug, Clone)]
pub struct Header {
    pub format_version: u8,
    pub flags: u8,
    pub reserved: u16,
    pub file_count: u64,
    pub frame_size_bytes: u64,
    pub frame_dir_offset: u64,
    pub frame_dir_size: u64,
    pub index_offset: u64,
    pub index_compressed_size: u64,
    pub index_raw_size: u64,
    pub index_checksum: u32,
}

impl Header {
    pub fn new_placeholder(frame_size_bytes: u64) -> Self {
        Header {
            format_version: 1,
            flags: 0,
            reserved: 0,
            file_count: 0,
            frame_size_bytes,
            frame_dir_offset: 0,
            frame_dir_size: 0,
            index_offset: 0,
            index_compressed_size: 0,
            index_raw_size: 0,
            index_checksum: 0,
        }
    }
}

fn serialize(h: &Header) -> [u8; HEADER_DISK_SIZE] {
    let mut buf = [0u8; HEADER_DISK_SIZE];
    buf[0..8].copy_from_slice(&MAGIC);
    buf[8] = h.format_version;
    buf[9] = h.flags;
    buf[10..12].copy_from_slice(&h.reserved.to_le_bytes());
    // bytes 12–19: file_count (rel 4)
    buf[12..20].copy_from_slice(&h.file_count.to_le_bytes());
    // bytes 20–27: frame_size_bytes (rel 12)
    buf[20..28].copy_from_slice(&h.frame_size_bytes.to_le_bytes());
    // bytes 28–35: frame_dir_offset (rel 20)
    buf[28..36].copy_from_slice(&h.frame_dir_offset.to_le_bytes());
    // bytes 36–43: frame_dir_size (rel 28)
    buf[36..44].copy_from_slice(&h.frame_dir_size.to_le_bytes());
    // bytes 44–51: index_offset (rel 36)
    buf[44..52].copy_from_slice(&h.index_offset.to_le_bytes());
    // bytes 52–59: index_compressed_size (rel 44)
    buf[52..60].copy_from_slice(&h.index_compressed_size.to_le_bytes());
    // bytes 60–67: index_raw_size (rel 52)
    buf[60..68].copy_from_slice(&h.index_raw_size.to_le_bytes());
    // bytes 68–71: index_checksum (rel 60)
    buf[68..72].copy_from_slice(&h.index_checksum.to_le_bytes());
    // bytes 72–75: header_checksum (rel 64) — zero during computation
    buf
}

/// Write header to `w`. Computes and writes header_checksum.
pub fn write_header(w: &mut impl Write, h: &Header) -> anyhow::Result<()> {
    let mut buf = serialize(h);
    // header_checksum = xxHash32 of bytes 0..68 (68 bytes), with header_checksum field zeroed
    let checksum = hash(&buf[0..68]);
    buf[72..76].copy_from_slice(&checksum.to_le_bytes());
    w.write_all(&buf)?;
    Ok(())
}

/// Write 76 zero-bytes as a placeholder header at current position.
pub fn write_placeholder(w: &mut impl Write) -> anyhow::Result<()> {
    w.write_all(&[0u8; HEADER_DISK_SIZE])?;
    Ok(())
}

/// Read and validate the 76-byte header from position 0 of a seekable reader.
pub fn read_header(r: &mut (impl Read + Seek)) -> anyhow::Result<Header> {
    r.seek(SeekFrom::Start(0))?;
    let mut buf = [0u8; HEADER_DISK_SIZE];
    r.read_exact(&mut buf)?;

    if buf[0..8] != MAGIC {
        return Err(SbkError::BadMagic.into());
    }

    let format_version = buf[8];
    if format_version != 1 {
        return Err(SbkError::UnsupportedVersion(format_version).into());
    }

    // Verify header_checksum
    let stored = u32::from_le_bytes(buf[72..76].try_into().unwrap());
    let mut tmp = buf;
    tmp[72..76].fill(0);
    let computed = hash(&tmp[0..68]);
    if computed != stored {
        return Err(SbkError::HeaderChecksumMismatch.into());
    }

    Ok(Header {
        format_version,
        flags: buf[9],
        reserved: u16::from_le_bytes(buf[10..12].try_into().unwrap()),
        file_count: u64::from_le_bytes(buf[12..20].try_into().unwrap()),
        frame_size_bytes: u64::from_le_bytes(buf[20..28].try_into().unwrap()),
        frame_dir_offset: u64::from_le_bytes(buf[28..36].try_into().unwrap()),
        frame_dir_size: u64::from_le_bytes(buf[36..44].try_into().unwrap()),
        index_offset: u64::from_le_bytes(buf[44..52].try_into().unwrap()),
        index_compressed_size: u64::from_le_bytes(buf[52..60].try_into().unwrap()),
        index_raw_size: u64::from_le_bytes(buf[60..68].try_into().unwrap()),
        index_checksum: u32::from_le_bytes(buf[68..72].try_into().unwrap()),
    })
}
