use std::io::{Read, Write};

use crate::checksum::hash;
use crate::error::SbkError;

/// One entry in the Index Block.
#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub path: String,
    pub mtime_ms: i64,
    pub group_id: u8,
    pub stream_offset: u64,
    pub stream_raw_size: u64,
    pub original_size: u64,
    pub file_checksum: u32,
}

/// Serialize all index entries and write as a compressed xz stream.
/// Entries are written in the order provided (caller must sort by path first).
pub fn write_index(
    entries: &[IndexEntry],
    preset: u32,
    w: &mut impl Write,
) -> anyhow::Result<(u64, u64, u32)> {
    // Serialize raw index
    let raw = serialize_raw(entries);
    let raw_size = raw.len() as u64;

    // Compress with xz
    use std::io::Write as IoWrite;
    let mut enc = xz2::write::XzEncoder::new(Vec::new(), preset);
    enc.write_all(&raw)?;
    let compressed = enc.finish()?;
    let compressed_size = compressed.len() as u64;
    let checksum = hash(&compressed);

    w.write_all(&compressed)?;

    Ok((compressed_size, raw_size, checksum))
}

fn serialize_raw(entries: &[IndexEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    // entry_count: u64
    out.extend_from_slice(&(entries.len() as u64).to_le_bytes());
    for e in entries {
        let path_bytes = e.path.as_bytes();
        // path_len: u16
        out.extend_from_slice(&(path_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(path_bytes);
        out.extend_from_slice(&e.mtime_ms.to_le_bytes());
        out.push(e.group_id);
        out.extend_from_slice(&e.stream_offset.to_le_bytes());
        out.extend_from_slice(&e.stream_raw_size.to_le_bytes());
        out.extend_from_slice(&e.original_size.to_le_bytes());
        out.extend_from_slice(&e.file_checksum.to_le_bytes());
    }
    out
}

/// Maximum allowed compressed index size (256 MiB). Prevents OOM on corrupt headers.
const MAX_INDEX_COMPRESSED_SIZE: u64 = 256 * 1024 * 1024;

/// Read and decompress the index block.
/// `compressed_size` bytes will be read from `r` (which should be positioned at the index start).
pub fn read_index(
    r: &mut impl Read,
    compressed_size: u64,
    expected_checksum: u32,
) -> anyhow::Result<Vec<IndexEntry>> {
    if compressed_size > MAX_INDEX_COMPRESSED_SIZE {
        return Err(anyhow::anyhow!(
            "index compressed size {} exceeds sanity limit {}",
            compressed_size,
            MAX_INDEX_COMPRESSED_SIZE
        ));
    }
    let mut compressed = vec![0u8; compressed_size as usize];
    r.read_exact(&mut compressed)?;

    // Verify index checksum
    let checksum = hash(&compressed);
    if checksum != expected_checksum {
        return Err(SbkError::IndexChecksumMismatch.into());
    }

    // Decompress
    use std::io::Read as IoRead;
    let mut dec = xz2::read::XzDecoder::new(&compressed[..]);
    let mut raw = Vec::new();
    dec.read_to_end(&mut raw)?;

    parse_raw(&raw)
}

/// Maximum number of index entries we'll accept from an archive.
/// A world with 100 million files would be pathological; this prevents OOM on corrupt data.
const MAX_INDEX_ENTRIES: usize = 10_000_000;

fn parse_raw(data: &[u8]) -> anyhow::Result<Vec<IndexEntry>> {
    if data.len() < 8 {
        return Err(anyhow::anyhow!("index data too short"));
    }
    let entry_count = u64::from_le_bytes(data[0..8].try_into().unwrap()) as usize;
    if entry_count > MAX_INDEX_ENTRIES {
        return Err(anyhow::anyhow!(
            "index entry count {} exceeds sanity limit {}",
            entry_count,
            MAX_INDEX_ENTRIES
        ));
    }
    let mut entries = Vec::with_capacity(entry_count);
    let mut pos = 8;

    for _ in 0..entry_count {
        if pos + 2 > data.len() {
            return Err(anyhow::anyhow!("truncated index entry"));
        }
        let path_len = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
        pos += 2;

        if pos + path_len > data.len() {
            return Err(anyhow::anyhow!("truncated path in index"));
        }
        let path = String::from_utf8(data[pos..pos + path_len].to_vec())
            .map_err(|e| anyhow::anyhow!("invalid UTF-8 in path: {}", e))?;
        pos += path_len;

        if pos + 37 > data.len() {
            return Err(anyhow::anyhow!("truncated index entry fields"));
        }
        let mtime_ms = i64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
        pos += 8;
        let group_id = data[pos];
        pos += 1;
        let stream_offset = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
        pos += 8;
        let stream_raw_size = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
        pos += 8;
        let original_size = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
        pos += 8;
        let file_checksum = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
        pos += 4;

        entries.push(IndexEntry {
            path,
            mtime_ms,
            group_id,
            stream_offset,
            stream_raw_size,
            original_size,
            file_checksum,
        });
    }

    Ok(entries)
}

/// Find an entry by exact path match.
pub fn find_exact<'a>(entries: &'a [IndexEntry], path: &str) -> Option<&'a IndexEntry> {
    entries.iter().find(|e| e.path == path)
}

/// Find all entries whose path matches the given glob pattern.
pub fn find_glob<'a>(
    entries: &'a [IndexEntry],
    pattern: &str,
) -> anyhow::Result<Vec<&'a IndexEntry>> {
    let pat = glob::Pattern::new(pattern).map_err(|e| SbkError::InvalidPattern {
        pattern: pattern.to_string(),
        source: e,
    })?;
    Ok(entries.iter().filter(|e| pat.matches(&e.path)).collect())
}

/// Find all entries matching any of the given glob patterns (used for extract).
pub fn find_patterns<'a>(
    entries: &'a [IndexEntry],
    patterns: &[String],
) -> anyhow::Result<Vec<&'a IndexEntry>> {
    let pats: Vec<glob::Pattern> = patterns
        .iter()
        .map(|s| {
            glob::Pattern::new(s).map_err(|e| {
                anyhow::anyhow!(
                    "{}",
                    SbkError::InvalidPattern {
                        pattern: s.clone(),
                        source: e,
                    }
                )
            })
        })
        .collect::<anyhow::Result<_>>()?;

    Ok(entries
        .iter()
        .filter(|e| pats.iter().any(|p| p.matches(&e.path)))
        .collect())
}
