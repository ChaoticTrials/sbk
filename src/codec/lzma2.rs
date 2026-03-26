use super::Codec;

pub struct Lzma2Codec;

impl Codec for Lzma2Codec {
    fn compress(&self, data: &[u8], level: u32) -> anyhow::Result<Vec<u8>> {
        use std::io::Write;
        let mut enc = xz2::write::XzEncoder::new(Vec::new(), level);
        enc.write_all(data)?;
        Ok(enc.finish()?)
    }

    fn decompress(&self, compressed: &[u8], expected_raw_size: u32) -> anyhow::Result<Vec<u8>> {
        use std::io::Read;
        let cap = expected_raw_size as u64 + 1;
        let mut dec = xz2::read::XzDecoder::new(compressed);
        let mut out = Vec::with_capacity(expected_raw_size as usize);
        dec.by_ref().take(cap).read_to_end(&mut out)?;
        if out.len() > expected_raw_size as usize {
            anyhow::bail!(
                "lzma2 decompressed to more than declared {} bytes",
                expected_raw_size
            );
        }
        Ok(out)
    }
}
