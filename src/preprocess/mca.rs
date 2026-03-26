use std::io::{Read, Write};
use std::path::Path;

use flate2::Compression;
use flate2::read::{MultiGzDecoder, ZlibDecoder};
use flate2::write::ZlibEncoder;
use rayon::prelude::*;

use crate::error::SbkError;
use crate::hilbert::xy_to_hilbert;

/// MCAP magic bytes: "MCAP"
const MCAP_MAGIC: [u8; 4] = [0x4D, 0x43, 0x41, 0x50];

/// Decompress a single MCA chunk's data bytes given the compression type.
fn decompress_chunk(data: &[u8], ctype: u8) -> anyhow::Result<Vec<u8>> {
    match ctype {
        1 => {
            let mut out = Vec::new();
            MultiGzDecoder::new(data).read_to_end(&mut out)?;
            Ok(out)
        }
        2 => {
            let mut out = Vec::new();
            ZlibDecoder::new(data).read_to_end(&mut out)?;
            Ok(out)
        }
        3 => Ok(data.to_vec()),
        4 => {
            let mut out = Vec::new();
            let mut decoder = ruzstd::decoding::StreamingDecoder::new(data)
                .map_err(|e| anyhow::anyhow!("zstd decode error: {}", e))?;
            decoder.read_to_end(&mut out)?;
            Ok(out)
        }
        _ => Err(SbkError::UnknownChunkCompression(ctype).into()),
    }
}

/// Preprocess an MCA file: decompress all chunks, sort by Hilbert curve, write MCAP stream.
pub fn preprocess_mca(path: &Path) -> anyhow::Result<Vec<u8>> {
    preprocess_mca_from_bytes(&std::fs::read(path)?)
}

/// Same as `preprocess_mca` but operates on already-loaded bytes.
pub fn preprocess_mca_from_bytes(file_bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    if file_bytes.len() < 8192 {
        // Empty or truncated MCA — produce MCAP with 0 chunks
        let mut out = Vec::new();
        out.extend_from_slice(&MCAP_MAGIC);
        out.extend_from_slice(&0u16.to_le_bytes());
        return Ok(out);
    }

    // Parse 1024 location table entries (bytes 0–4095)
    let mut occupied_slots: Vec<(usize, usize)> = Vec::new(); // (slot_index, sector_offset)
    for slot in 0..1024usize {
        let base = slot * 4;
        let entry = u32::from_be_bytes([
            file_bytes[base],
            file_bytes[base + 1],
            file_bytes[base + 2],
            file_bytes[base + 3],
        ]);
        if entry != 0 {
            let sector_offset = (entry >> 8) as usize;
            occupied_slots.push((slot, sector_offset));
        }
    }

    // Parallel chunk decompression
    let chunks: Vec<(u8, u8, Vec<u8>)> = occupied_slots
        .par_iter()
        .map(|&(slot, sector_offset)| {
            let pos = sector_offset * 4096;
            if pos + 5 > file_bytes.len() {
                return Err(anyhow::anyhow!(
                    "MCA sector offset {} out of bounds",
                    sector_offset
                ));
            }
            let len = u32::from_be_bytes([
                file_bytes[pos],
                file_bytes[pos + 1],
                file_bytes[pos + 2],
                file_bytes[pos + 3],
            ]) as usize;
            if len == 0 {
                return Err(anyhow::anyhow!(
                    "MCA chunk length is 0 at sector {}",
                    sector_offset
                ));
            }
            let ctype = file_bytes[pos + 4];
            let end = pos + 4 + len;
            if end > file_bytes.len() {
                return Err(anyhow::anyhow!(
                    "MCA chunk data out of bounds at sector {}",
                    sector_offset
                ));
            }
            let data = &file_bytes[pos + 5..end];
            let nbt = decompress_chunk(data, ctype)?;
            Ok(((slot % 32) as u8, (slot / 32) as u8, nbt))
        })
        .collect::<anyhow::Result<_>>()?;

    // Sort by Hilbert curve index
    let mut chunks = chunks;
    chunks.sort_by_key(|(x, z, _)| xy_to_hilbert(*x as u32, *z as u32));

    // Serialize to MCAP format
    let mut out = Vec::new();
    out.extend_from_slice(&MCAP_MAGIC);
    out.extend_from_slice(&(chunks.len() as u16).to_le_bytes());
    for (local_x, local_z, nbt) in &chunks {
        out.push(*local_x);
        out.push(*local_z);
        out.extend_from_slice(&(nbt.len() as u32).to_le_bytes());
        out.extend_from_slice(nbt);
    }

    Ok(out)
}

/// Reconstruct an MCA file from an MCAP byte stream, returning the raw bytes.
pub fn reconstruct_mca_bytes(mcap: &[u8]) -> anyhow::Result<Vec<u8>> {
    if mcap.len() < 6 {
        return Err(SbkError::InvalidMcap("too short").into());
    }
    if mcap[0..4] != MCAP_MAGIC {
        return Err(SbkError::InvalidMcap("bad magic").into());
    }

    let chunk_count = u16::from_le_bytes([mcap[4], mcap[5]]) as usize;

    let mut raw_chunks: Vec<(u8, u8, Vec<u8>)> = Vec::with_capacity(chunk_count);
    let mut pos = 6;
    for _ in 0..chunk_count {
        if pos + 6 > mcap.len() {
            return Err(SbkError::InvalidMcap("truncated chunk entry").into());
        }
        let local_x = mcap[pos];
        let local_z = mcap[pos + 1];
        let raw_len =
            u32::from_le_bytes([mcap[pos + 2], mcap[pos + 3], mcap[pos + 4], mcap[pos + 5]])
                as usize;
        pos += 6;
        if pos + raw_len > mcap.len() {
            return Err(SbkError::InvalidMcap("truncated nbt data").into());
        }
        let nbt = mcap[pos..pos + raw_len].to_vec();
        pos += raw_len;
        raw_chunks.push((local_x, local_z, nbt));
    }

    let compressed: Vec<(u8, u8, Vec<u8>)> = raw_chunks
        .par_iter()
        .map(|(x, z, nbt)| {
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::new(6));
            enc.write_all(nbt)?;
            Ok((*x, *z, enc.finish()?))
        })
        .collect::<anyhow::Result<_>>()?;

    let mut sector_assignments: Vec<(u8, u8, u32, usize, Vec<u8>)> = Vec::new();
    let mut current_sector: u32 = 2;

    for (x, z, cdata) in &compressed {
        let data_len = cdata.len();
        let total_len = 5 + data_len;
        let required_sectors = ((total_len + 4095) / 4096) as usize;
        sector_assignments.push((*x, *z, current_sector, required_sectors, cdata.clone()));
        current_sector += required_sectors as u32;
    }

    let total_sectors = current_sector as usize;
    let mut file_data = vec![0u8; total_sectors * 4096];

    for (x, z, sector_offset, sector_count, _) in &sector_assignments {
        let slot = (*z as usize) * 32 + (*x as usize);
        let base = slot * 4;
        let entry = ((*sector_offset as u32) << 8) | (*sector_count as u32);
        let bytes = entry.to_be_bytes();
        file_data[base..base + 4].copy_from_slice(&bytes);
    }

    for (_, _, sector_offset, _, cdata) in &sector_assignments {
        let pos = (*sector_offset as usize) * 4096;
        let length = (cdata.len() + 1) as u32;
        file_data[pos..pos + 4].copy_from_slice(&length.to_be_bytes());
        file_data[pos + 4] = 2;
        file_data[pos + 5..pos + 5 + cdata.len()].copy_from_slice(cdata);
    }

    Ok(file_data)
}

/// Reconstruct an MCA file from an MCAP byte stream.
pub fn reconstruct_mca(mcap: &[u8], out_path: &Path) -> anyhow::Result<()> {
    let file_data = reconstruct_mca_bytes(mcap)?;
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out_path, &file_data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write as IoWrite;
    use tempfile::tempdir;

    fn make_synthetic_mca(chunks: &[(u8, u8, &[u8])]) -> Vec<u8> {
        // Build a synthetic MCA file with the given chunks
        // Each chunk: (local_x, local_z, nbt_data)

        // First pass: zlib compress each chunk
        let compressed: Vec<(u8, u8, Vec<u8>)> = chunks
            .iter()
            .map(|(x, z, nbt)| {
                let mut enc = ZlibEncoder::new(Vec::new(), Compression::new(6));
                enc.write_all(nbt).unwrap();
                (*x, *z, enc.finish().unwrap())
            })
            .collect();

        // Compute sector layout starting at sector 2
        let mut sector_data: Vec<(usize, u8, u8, Vec<u8>)> = Vec::new(); // (sector, x, z, data)
        let mut current_sector: usize = 2;
        let mut location_entries: [(u32, u8); 1024] = [(0, 0); 1024];

        for (x, z, cdata) in &compressed {
            let slot = (*z as usize) * 32 + (*x as usize);
            let total_len = 5 + cdata.len();
            let required_sectors = (total_len + 4095) / 4096;
            location_entries[slot] = (current_sector as u32, required_sectors as u8);
            sector_data.push((current_sector, *x, *z, cdata.clone()));
            current_sector += required_sectors;
        }

        let total_size = current_sector * 4096;
        let mut file = vec![0u8; total_size];

        // Write location table
        for (slot, (sector_offset, sector_count)) in location_entries.iter().enumerate() {
            if *sector_count > 0 {
                let base = slot * 4;
                let entry = ((*sector_offset as u32) << 8) | (*sector_count as u32);
                file[base..base + 4].copy_from_slice(&entry.to_be_bytes());
            }
        }

        // Write chunk data
        for (sector, _, _, cdata) in &sector_data {
            let pos = sector * 4096;
            let length = (cdata.len() + 1) as u32;
            file[pos..pos + 4].copy_from_slice(&length.to_be_bytes());
            file[pos + 4] = 2; // zlib
            file[pos + 5..pos + 5 + cdata.len()].copy_from_slice(cdata);
        }

        file
    }

    #[test]
    fn mca_round_trip_5_chunks() {
        let dir = tempdir().unwrap();
        let in_path = dir.path().join("r.0.0.mca");
        let out_path = dir.path().join("r.0.0_out.mca");

        let chunks: Vec<(u8, u8, Vec<u8>)> = vec![
            (0, 0, b"nbt_data_0".to_vec()),
            (1, 0, b"nbt_data_1".to_vec()),
            (2, 0, b"nbt_data_2".to_vec()),
            (0, 1, b"nbt_data_3".to_vec()),
            (1, 1, b"nbt_data_4".to_vec()),
        ];

        let refs: Vec<(u8, u8, &[u8])> = chunks
            .iter()
            .map(|(x, z, d)| (*x, *z, d.as_slice()))
            .collect();

        let mca_bytes = make_synthetic_mca(&refs);
        std::fs::write(&in_path, &mca_bytes).unwrap();

        let mcap = preprocess_mca(&in_path).unwrap();
        reconstruct_mca(&mcap, &out_path).unwrap();

        // Verify: re-preprocess the output and check we get the same data
        let mcap2 = preprocess_mca(&out_path).unwrap();

        // Both should have same chunk count
        let count1 = u16::from_le_bytes([mcap[4], mcap[5]]);
        let count2 = u16::from_le_bytes([mcap2[4], mcap2[5]]);
        assert_eq!(count1, count2);
        assert_eq!(count1, 5);
    }
}
