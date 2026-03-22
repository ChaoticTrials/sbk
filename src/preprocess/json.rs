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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn strips_whitespace() {
        let result = preprocess_json_from_bytes(b"{  \"key\":  \"value\"  }").unwrap();
        assert_eq!(result, b"{\"key\":\"value\"}");
    }

    #[test]
    fn preserves_structure() {
        let input = b"{\"a\": 1, \"b\": [1, 2, 3]}";
        let result = preprocess_json_from_bytes(input).unwrap();
        let orig: serde_json::Value = serde_json::from_slice(input).unwrap();
        let processed: serde_json::Value = serde_json::from_slice(&result).unwrap();
        assert_eq!(orig, processed);
        // No spurious spaces in output
        assert!(!result.contains(&b' '));
    }

    #[test]
    fn invalid_json_errors() {
        assert!(preprocess_json_from_bytes(b"not json {{{").is_err());
        assert!(preprocess_json_from_bytes(b"").is_err());
    }

    #[test]
    fn reconstruct_writes_bytes_verbatim() {
        let dir = tempdir().unwrap();
        let out = dir.path().join("data.json");
        let data = b"{\"key\":\"value\"}";
        reconstruct_json(data, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), data);
    }
}
