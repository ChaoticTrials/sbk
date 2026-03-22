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

pub fn reconstruct_nbt(raw: &[u8], out_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut enc = GzEncoder::new(File::create(out_path)?, Compression::new(6));
    enc.write_all(raw)?;
    enc.finish()?;
    Ok(())
}
