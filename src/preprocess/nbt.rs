use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;

pub fn preprocess_nbt(path: &Path) -> anyhow::Result<Vec<u8>> {
    preprocess_nbt_from_bytes(&std::fs::read(path)?)
}

/// Same as `preprocess_nbt` but operates on already-loaded bytes.
pub fn preprocess_nbt_from_bytes(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    MultiGzDecoder::new(bytes).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Reconstruct an NBT file from raw (decompressed) bytes, returning gzip-wrapped bytes.
pub fn reconstruct_nbt_bytes(raw: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::new(6));
    enc.write_all(raw)?;
    Ok(enc.finish()?)
}

pub fn reconstruct_nbt(raw: &[u8], out_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut enc = GzEncoder::new(File::create(out_path)?, Compression::new(6));
    enc.write_all(raw)?;
    enc.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write as IoWrite;
    use tempfile::tempdir;

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut enc = GzEncoder::new(Vec::new(), Compression::new(6));
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn strips_gzip_wrapper() {
        let raw = b"minimal nbt payload";
        let result = preprocess_nbt_from_bytes(&gzip(raw)).unwrap();
        assert_eq!(result, raw);
    }

    #[test]
    fn invalid_input_errors() {
        assert!(preprocess_nbt_from_bytes(b"not gzip at all").is_err());
    }

    #[test]
    fn round_trip() {
        use flate2::read::GzDecoder;
        use std::io::Read;

        let raw = b"some nbt compound data bytes";
        let preprocessed = preprocess_nbt_from_bytes(&gzip(raw)).unwrap();
        assert_eq!(preprocessed, raw);

        let dir = tempdir().unwrap();
        let out_path = dir.path().join("level.dat");
        reconstruct_nbt(&preprocessed, &out_path).unwrap();

        let mut dec = GzDecoder::new(std::fs::File::open(&out_path).unwrap());
        let mut decoded = Vec::new();
        dec.read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, raw);
    }

    #[test]
    fn empty_payload_round_trip() {
        use flate2::read::GzDecoder;
        use std::io::Read;

        let raw = b"";
        let preprocessed = preprocess_nbt_from_bytes(&gzip(raw)).unwrap();
        assert_eq!(preprocessed, raw);

        let dir = tempdir().unwrap();
        let out_path = dir.path().join("empty.dat");
        reconstruct_nbt(&preprocessed, &out_path).unwrap();

        let mut dec = GzDecoder::new(std::fs::File::open(&out_path).unwrap());
        let mut decoded = Vec::new();
        dec.read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, raw);
    }
}
