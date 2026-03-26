use super::Codec;

pub struct ZstdCodec;

impl Codec for ZstdCodec {
    fn compress(&self, data: &[u8], level: u32) -> anyhow::Result<Vec<u8>> {
        // Map [1,9] → [3,19]: level 1 → 3 (fast), level 9 → 19 (high)
        let zstd_level = (level * 2 + 1).min(22) as i32;
        Ok(zstd::stream::encode_all(
            std::io::Cursor::new(data),
            zstd_level,
        )?)
    }

    fn decompress(&self, compressed: &[u8], expected_raw_size: u32) -> anyhow::Result<Vec<u8>> {
        use std::io::Read;
        let cap = expected_raw_size as u64 + 1;
        let mut dec = zstd::stream::read::Decoder::new(std::io::Cursor::new(compressed))?;
        let mut out = Vec::with_capacity(expected_raw_size as usize);
        dec.by_ref().take(cap).read_to_end(&mut out)?;
        if out.len() > expected_raw_size as usize {
            anyhow::bail!(
                "zstd decompressed to more than declared {} bytes",
                expected_raw_size
            );
        }
        Ok(out)
    }
}
