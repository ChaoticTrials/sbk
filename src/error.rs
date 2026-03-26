use thiserror::Error;

#[derive(Debug, Error)]
pub enum SbkError {
    #[error("Not an SBK archive (bad magic bytes)")]
    BadMagic,
    #[error("Unsupported SBK format version: {0}")]
    UnsupportedVersion(u8),
    #[error("Header checksum mismatch")]
    HeaderChecksumMismatch,
    #[error("Index checksum mismatch")]
    IndexChecksumMismatch,
    #[error("Frame {0} checksum mismatch")]
    FrameChecksumMismatch(u32),
    #[error("Unknown MCA chunk compression type: {0}")]
    UnknownChunkCompression(u8),
    #[error("Invalid MCAP stream: {0}")]
    InvalidMcap(&'static str),
    #[error("No files matched pattern '{0}'")]
    NoMatch(String),
    #[error("Invalid glob pattern '{pattern}': {source}")]
    InvalidPattern {
        pattern: String,
        source: glob::PatternError,
    },
    #[error("--exclude and --include are mutually exclusive; use one or the other")]
    ConflictingFilters,
    #[error("--max-age must be at least 1 millisecond")]
    InvalidMaxAge,
    #[error("--since must be a non-negative millisecond Unix timestamp")]
    InvalidSinceTimestamp,
    #[error("Unsupported compression algorithm: {0}")]
    UnsupportedAlgorithm(u8),
    #[error("Invalid --algorithm value '{0}': expected 'lzma2' or 'zstd'")]
    InvalidAlgorithm(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
