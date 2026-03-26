use crate::checksum::hash;
use crate::error::SbkError;
use std::io::{Read, Seek, SeekFrom, Write};

pub const MAGIC: [u8; 8] = [0x53, 0x42, 0x4B, 0x21, 0x56, 0x31, 0x0D, 0x0A]; // "SBK!V1\r\n"
pub const HEADER_DISK_SIZE: usize = 79;

/// Compression algorithm used for all data frames and the index block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Algorithm {
    /// LZMA2 in XZ container format. Default.
    Lzma2 = 0,
    /// Zstandard.
    Zstd = 1,
}

impl Algorithm {
    pub fn from_u8(v: u8) -> anyhow::Result<Self> {
        match v {
            0 => Ok(Self::Lzma2),
            1 => Ok(Self::Zstd),
            n => Err(SbkError::UnsupportedAlgorithm(n).into()),
        }
    }
}

impl std::fmt::Display for Algorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lzma2 => write!(f, "lzma2"),
            Self::Zstd => write!(f, "zstd"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Header {
    pub format_version: u8,
    pub flags: u8,
    pub algorithm: Algorithm,
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
    pub fn new_placeholder(frame_size_bytes: u64, algorithm: Algorithm) -> Self {
        Header {
            format_version: 1,
            flags: 0,
            algorithm,
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
    buf[10] = h.algorithm as u8;
    // bytes 11–14: reserved = 0x00000000 (already zero)
    buf[15..23].copy_from_slice(&h.file_count.to_le_bytes());
    buf[23..31].copy_from_slice(&h.frame_size_bytes.to_le_bytes());
    buf[31..39].copy_from_slice(&h.frame_dir_offset.to_le_bytes());
    buf[39..47].copy_from_slice(&h.frame_dir_size.to_le_bytes());
    buf[47..55].copy_from_slice(&h.index_offset.to_le_bytes());
    buf[55..63].copy_from_slice(&h.index_compressed_size.to_le_bytes());
    buf[63..71].copy_from_slice(&h.index_raw_size.to_le_bytes());
    buf[71..75].copy_from_slice(&h.index_checksum.to_le_bytes());
    // bytes 75–78: header_checksum — filled in by write_header
    buf
}

pub fn write_header(w: &mut impl Write, h: &Header) -> anyhow::Result<()> {
    let mut buf = serialize(h);
    let checksum = hash(&buf[0..75]);
    buf[75..79].copy_from_slice(&checksum.to_le_bytes());
    w.write_all(&buf)?;
    Ok(())
}

pub fn write_placeholder(w: &mut impl Write) -> anyhow::Result<()> {
    w.write_all(&[0u8; HEADER_DISK_SIZE])?;
    Ok(())
}

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

    let stored = u32::from_le_bytes(buf[75..79].try_into().unwrap());
    let mut tmp = buf;
    tmp[75..79].fill(0);
    let computed = hash(&tmp[0..75]);
    if computed != stored {
        return Err(SbkError::HeaderChecksumMismatch.into());
    }

    let algorithm = Algorithm::from_u8(buf[10])?;

    if buf[11..15] != [0u8; 4] {
        return Err(anyhow::anyhow!(
            "non-zero reserved bytes in header (bytes 11–14): {:?}",
            &buf[11..15]
        ));
    }

    Ok(Header {
        format_version,
        flags: buf[9],
        algorithm,
        file_count: u64::from_le_bytes(buf[15..23].try_into().unwrap()),
        frame_size_bytes: u64::from_le_bytes(buf[23..31].try_into().unwrap()),
        frame_dir_offset: u64::from_le_bytes(buf[31..39].try_into().unwrap()),
        frame_dir_size: u64::from_le_bytes(buf[39..47].try_into().unwrap()),
        index_offset: u64::from_le_bytes(buf[47..55].try_into().unwrap()),
        index_compressed_size: u64::from_le_bytes(buf[55..63].try_into().unwrap()),
        index_raw_size: u64::from_le_bytes(buf[63..71].try_into().unwrap()),
        index_checksum: u32::from_le_bytes(buf[71..75].try_into().unwrap()),
    })
}
