/// Compute xxHash32 of `data` with seed 0.
pub fn hash(data: &[u8]) -> u32 {
    xxhash_rust::xxh32::xxh32(data, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let data = b"hello world";
        assert_eq!(hash(data), hash(data));
    }

    #[test]
    fn different_inputs_differ() {
        assert_ne!(hash(b"foo"), hash(b"bar"));
        assert_ne!(hash(b"x"), hash(&[]));
    }

    #[test]
    fn known_empty_value() {
        // xxHash32 of empty slice with seed 0, per xxHash spec
        assert_eq!(hash(&[]), 0x02CC5D05);
    }

    #[test]
    fn byte_order_sensitive() {
        // Swapping bytes must produce a different hash
        assert_ne!(hash(&[0x01, 0x02]), hash(&[0x02, 0x01]));
    }
}
