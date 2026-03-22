use std::path::Path;

pub fn preprocess_json(path: &Path) -> anyhow::Result<Vec<u8>> {
    preprocess_json_from_bytes(&std::fs::read(path)?)
}

/// Same as `preprocess_json` but operates on already-loaded bytes.
pub fn preprocess_json_from_bytes(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let v: serde_json::Value = serde_json::from_slice(bytes)?;
    Ok(serde_json::to_vec(&v)?)
}

pub fn reconstruct_json(raw: &[u8], out_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out_path, raw)?;
    Ok(())
}
