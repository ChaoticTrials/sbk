/// Compute xxHash32 of `data` with seed 0.
pub fn hash(data: &[u8]) -> u32 {
    xxhash_rust::xxh32::xxh32(data, 0)
}
