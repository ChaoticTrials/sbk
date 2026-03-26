use std::path::PathBuf;
use std::process;
use std::time::Instant;

use clap::Parser;

use sbk::cli::{Cli, Commands};
use sbk::error::SbkError;
use sbk::filter::{CompressOptions, FilterMode};
use sbk::format::header::Algorithm;

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {}", e);
        process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Compress {
            world_dir,
            output,
            threads,
            level,
            max_age,
            since,
            exclude,
            include,
            include_session_lock,
            quiet,
            algorithm: algorithm_str,
        } => {
            // Validation
            if let Some(age) = max_age {
                if age == 0 {
                    return Err(SbkError::InvalidMaxAge.into());
                }
            }
            if let Some(ts) = since {
                if ts < 0 {
                    return Err(SbkError::InvalidSinceTimestamp.into());
                }
            }
            if !exclude.is_empty() && !include.is_empty() {
                return Err(SbkError::ConflictingFilters.into());
            }

            // Validate world_dir
            if !world_dir.exists() || !world_dir.is_dir() {
                return Err(anyhow::anyhow!(
                    "world_dir '{}' does not exist or is not a directory",
                    world_dir.display()
                ));
            }

            // Parse algorithm
            let algorithm = match algorithm_str.to_lowercase().as_str() {
                "lzma2" => Algorithm::Lzma2,
                "zstd" => Algorithm::Zstd,
                other => {
                    eprintln!("error: {}", SbkError::InvalidAlgorithm(other.to_string()));
                    eprintln!("Valid values: lzma2, zstd");
                    std::process::exit(1);
                }
            };

            // Build FilterMode
            let patterns = if !exclude.is_empty() {
                let pats = exclude
                    .iter()
                    .map(|s| {
                        glob::Pattern::new(s).map_err(|e| SbkError::InvalidPattern {
                            pattern: s.clone(),
                            source: e,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                FilterMode::Exclude(pats)
            } else if !include.is_empty() {
                let pats = include
                    .iter()
                    .map(|s| {
                        glob::Pattern::new(s).map_err(|e| SbkError::InvalidPattern {
                            pattern: s.clone(),
                            source: e,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                FilterMode::Include(pats)
            } else {
                FilterMode::None
            };

            // Determine output path
            let output = output.unwrap_or_else(|| {
                let name = world_dir
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                PathBuf::from(format!("{}.sbk", name))
            });

            let opts = CompressOptions {
                output,
                threads,
                level,
                algorithm,
                max_age,
                since,
                patterns,
                include_session_lock,
                quiet,
            };

            let t = Instant::now();
            sbk::compress::compress(&world_dir, &opts)?;
            if !quiet {
                println!("Done in {:.2}s.", t.elapsed().as_secs_f64());
            }
        }

        Commands::Decompress {
            archive,
            output,
            threads,
            quiet,
        } => {
            let output = output.unwrap_or_else(|| {
                let stem = archive
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                PathBuf::from(stem)
            });
            let t = Instant::now();
            sbk::decompress::decompress(&archive, &output, threads)?;
            if !quiet {
                println!("Done in {:.2}s.", t.elapsed().as_secs_f64());
            }
        }

        Commands::Extract {
            archive,
            patterns,
            output,
            threads,
            quiet,
        } => {
            let output = output.unwrap_or_else(|| PathBuf::from("."));
            let t = Instant::now();
            let n = sbk::extract::extract(&archive, &patterns, &output, threads)?;
            if !quiet {
                println!(
                    "Extracted {} file(s) in {:.2}s.",
                    n,
                    t.elapsed().as_secs_f64()
                );
            }
        }

        Commands::Info { archive, list } => {
            sbk::info::info(&archive, list)?;
        }

        Commands::Verify { archive, threads } => {
            let ok = sbk::verify::verify(&archive, threads)?;
            if !ok {
                process::exit(1);
            }
        }
    }

    Ok(())
}
