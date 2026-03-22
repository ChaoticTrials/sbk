/// Maps (local_x, local_z) in [0,31]^2 to a 1D Hilbert curve index in [0,1023].
/// Order-5 Hilbert curve for a 32x32 grid.
pub fn xy_to_hilbert(mut x: u32, mut z: u32) -> u32 {
    let mut d = 0u32;
    let mut s = 16u32;
    while s > 0 {
        let rx = if (x & s) > 0 { 1 } else { 0 };
        let rz = if (z & s) > 0 { 1 } else { 0 };
        d += s * s * ((3 * rx) ^ rz);
        if rz == 0 {
            if rx == 1 {
                x = s.wrapping_sub(1).wrapping_sub(x);
                z = s.wrapping_sub(1).wrapping_sub(z);
            }
            std::mem::swap(&mut x, &mut z);
        }
        s >>= 1;
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hilbert_corners() {
        assert_eq!(xy_to_hilbert(0, 0), 0);
        // The max value 1023 is at (31, 0) for this algorithm
        assert_eq!(xy_to_hilbert(31, 0), 1023);
        // Verify the function produces all 1024 unique values in [0,1023]
        let mut values: Vec<u32> = (0..32u32)
            .flat_map(|x| (0..32u32).map(move |z| xy_to_hilbert(x, z)))
            .collect();
        values.sort_unstable();
        values.dedup();
        assert_eq!(values.len(), 1024);
        assert_eq!(values[0], 0);
        assert_eq!(values[1023], 1023);
    }

    #[test]
    fn hilbert_no_duplicates() {
        let mut values: Vec<u32> = (0..32)
            .flat_map(|x| (0..32).map(move |z| xy_to_hilbert(x, z)))
            .collect();
        values.sort_unstable();
        values.dedup();
        assert_eq!(values.len(), 1024);
    }

    #[test]
    fn hilbert_range() {
        for x in 0..32 {
            for z in 0..32 {
                let h = xy_to_hilbert(x, z);
                assert!(h < 1024, "hilbert({},{}) = {} out of range", x, z, h);
            }
        }
    }
}
