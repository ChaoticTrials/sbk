use crate::format::header::Algorithm;

pub mod lzma2;
pub mod zstd;

/// Compression and decompression for a single SBK frame or the index block.
/// Implementations must be `Send + Sync` for use in Rayon parallel iterators.
pub trait Codec: Send + Sync {
    /// Compress `data` at the given level (1–9). Returns compressed bytes.
    fn compress(&self, data: &[u8], level: u32) -> anyhow::Result<Vec<u8>>;

    /// Decompress `compressed`. Capped at `expected_raw_size + 1` bytes to guard
    /// against decompression bombs.
    fn decompress(&self, compressed: &[u8], expected_raw_size: u32) -> anyhow::Result<Vec<u8>>;
}

/// Returns a boxed `Codec` for the given algorithm.
pub fn from_algorithm(algorithm: Algorithm) -> Box<dyn Codec> {
    match algorithm {
        Algorithm::Lzma2 => Box::new(lzma2::Lzma2Codec),
        Algorithm::Zstd => Box::new(zstd::ZstdCodec),
    }
}
