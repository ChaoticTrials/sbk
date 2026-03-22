use std::io::Write;

pub const FRAME_SIZE: u64 = 16 * 1024 * 1024;

/// Compress a single frame's bytes with LZMA2 at the given preset.
pub fn compress_frame_data(data: &[u8], preset: u32) -> anyhow::Result<Vec<u8>> {
    let mut enc = xz2::write::XzEncoder::new(Vec::new(), preset);
    enc.write_all(data)?;
    Ok(enc.finish()?)
}
