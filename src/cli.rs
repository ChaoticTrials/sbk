use std::path::PathBuf;

use clap::{Parser, Subcommand};

fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get() / 2)
        .unwrap_or(4)
}

#[derive(Parser)]
#[command(name = "sbk", version, about = "Minecraft world compressor")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Compress a Minecraft world directory into an SBK archive
    Compress {
        /// Path to the world directory
        world_dir: PathBuf,
        /// Output archive path (default: <world_dir_name>.sbk)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Number of threads (default: logical CPU count)
        #[arg(short, long, default_value_t = default_threads())]
        threads: usize,
        /// LZMA preset level 1–9 (default: 9)
        #[arg(short, long, default_value_t = 9,
              value_parser = clap::value_parser!(u32).range(1..=9))]
        level: u32,
        /// Skip files not modified within the last N milliseconds (N >= 1)
        #[arg(long, value_name = "MS")]
        max_age: Option<u64>,
        /// Skip files with mtime below this millisecond Unix timestamp
        #[arg(long, value_name = "TIMESTAMP")]
        since: Option<i64>,
        /// Exclude files matching these glob patterns (mutually exclusive with --include)
        #[arg(long, value_name = "PATTERN", num_args = 1..)]
        exclude: Vec<String>,
        /// Include ONLY files matching these glob patterns (mutually exclusive with --exclude)
        #[arg(long, value_name = "PATTERN", num_args = 1..)]
        include: Vec<String>,
        /// Include session.lock in the archive (excluded by default)
        #[arg(long)]
        include_session_lock: bool,
        /// Suppress all output (progress bars and summary)
        #[arg(short, long)]
        quiet: bool,
        /// Compression algorithm: lzma2 (default) or zstd.
        /// For zstd, --level 1–9 maps to zstd levels 3–19 internally.
        #[arg(short, long, default_value = "lzma2", value_name = "ALGO")]
        algorithm: String,
    },
    /// Decompress an SBK archive into a directory
    Decompress {
        /// Path to the SBK archive
        archive: PathBuf,
        /// Output directory (default: <archive_stem>/ in current dir)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Number of threads (default: logical CPU count)
        #[arg(short, long, default_value_t = default_threads())]
        threads: usize,
        /// Suppress all output
        #[arg(short, long)]
        quiet: bool,
    },
    /// Extract specific files from an SBK archive
    Extract {
        /// Path to the SBK archive
        archive: PathBuf,
        /// One or more exact paths or glob patterns
        #[arg(required = true)]
        patterns: Vec<String>,
        /// Output directory (default: current dir)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Number of threads (default: logical CPU count)
        #[arg(short, long, default_value_t = default_threads())]
        threads: usize,
        /// Suppress all output
        #[arg(short, long)]
        quiet: bool,
    },
    /// Display information about an SBK archive
    Info {
        /// Path to the SBK archive
        archive: PathBuf,
        /// Print full file manifest
        #[arg(long)]
        list: bool,
    },
    /// Verify checksums of all frames in an SBK archive
    Verify {
        /// Path to the SBK archive
        archive: PathBuf,
        /// Number of threads (default: logical CPU count)
        #[arg(short, long, default_value_t = default_threads())]
        threads: usize,
    },
    /// Convert an SBK archive to a standard archive format (zip, tar.gz, tar.xz)
    Convert {
        /// Path to the SBK archive
        archive: PathBuf,
        /// Target format: zip, tar.gz, tar.xz
        #[arg(long, value_name = "FORMAT")]
        to: String,
        /// Output file path (default: <archive_stem>.<ext> in current dir)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Number of threads for SBK decompression (default: logical CPU count)
        #[arg(short, long, default_value_t = default_threads())]
        threads: usize,
        /// Compression level 1–9 for the target format (default: 6)
        #[arg(short, long, default_value_t = 6,
              value_parser = clap::value_parser!(u32).range(1..=9))]
        level: u32,
    },
}
