use std::path::Path;

use crate::extract::extract;

pub fn decompress(archive: &Path, output_dir: &Path, threads: usize) -> anyhow::Result<()> {
    extract(archive, &["**".to_string()], output_dir, threads).map(|_| ())
}
