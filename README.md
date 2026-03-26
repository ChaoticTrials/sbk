# sbk

A format-aware archiver for Minecraft worlds. `sbk` understands Minecraft's internal file formats and preprocesses them before compression, achieving significantly better ratios than generic archivers like `zip` or `7z`.

## Why sbk?

Generic archivers treat world files as opaque blobs. `sbk` doesn't:

- **MCA region files** — strips per-chunk zlib/zstd compression, reorders chunks along a Hilbert curve for better spatial locality, then feeds the raw NBT stream into the compressor
- **NBT files** (`.dat`, `.dat_old`) — strips the outer gzip wrapper, stores raw NBT
- **JSON files** — minifies before compression
- All files are grouped by type and compressed together as solid archives (LZMA2 or Zstd), maximizing cross-file redundancy

The result: roughly **40–55% of original size** vs. **70–77%** for zip/7z on typical Minecraft worlds.

## Benchmarks

Tested on Minecraft 1.21.1 worlds, AMD Ryzen 7 7800X3D, 16 threads:

| World size | sbk -l 9 | 7z -mx=9 | zip -9 |
|-----------:|---------:|---------:|-------:|
| 12.7 MB    | 20.1%    | 44.2%    | 51.4%  |
| 129.3 MB   | 39.1%    | 68.5%    | 72.9%  |
| 493.3 MB   | 43.6%    | 73.0%    | 76.2%  |
| 1.02 GB    | 44.7%    | 74.1%    | 77.0%  |
| 1.78 GB    | 41.9%    | 72.5%    | 77.4%  |

See [BENCHMARK.md](BENCHMARK.md) for full results, including compression and decompression times.

## Installation

```bash
cargo install sbk
```

Or build from source:

```bash
cargo build --release
# binary at: target/release/sbk
```

## Usage

### Compress a world

```bash
sbk compress <world_dir> [OPTIONS]
```

| Option | Default | Description |
|--------|---------|-------------|
| `-o, --output <FILE>` | `<world_dir>.sbk` | Output archive path |
| `-t, --threads <N>` | logical CPU count | Worker threads |
| `-l, --level <1–9>` | `9` | Compression level |
| `--algorithm <ALG>` | `lzma2` | Compression algorithm: `lzma2` or `zstd` |
| `--max-age <MS>` | — | Skip files not modified in the last N ms |
| `--since <TIMESTAMP>` | — | Skip files with mtime below Unix timestamp (ms) |
| `--exclude <PATTERN>…` | — | Exclude files matching glob patterns |
| `--include <PATTERN>…` | — | Include ONLY files matching glob patterns |
| `--include-session-lock` | off | Include `session.lock` (excluded by default) |
| `-q, --quiet` | off | Suppress progress bars and summary |

`--exclude` and `--include` are mutually exclusive.

### Decompress

```bash
sbk decompress <file.sbk> [-o <dir>] [-t <threads>]
```

### Extract specific files

```bash
sbk extract <file.sbk> <pattern>... [-o <dir>]
```

Patterns are glob expressions (e.g. `region/*.mca`, `level.dat`).

### Show archive info

```bash
sbk info <file.sbk> [--list]
```

`--list` prints the full file manifest as a tree, with file type and size for each entry.

### Convert to a standard archive format

```bash
sbk convert <file.sbk> --to <format> [-o <file>] [-t <threads>] [-l <level>]
```

| Option | Default | Description |
|--------|---------|-------------|
| `--to <format>` | — | Target format: `zip`, `tar.gz`, `tar.xz` |
| `-o, --output <FILE>` | `<archive_stem>.<ext>` | Output file path |
| `-t, --threads <N>` | logical CPU count | Worker threads for SBK decompression |
| `-l, --level <1–9>` | `6` | Compression level for the target format |

Files are reconstructed in memory and streamed directly into the output archive — no temporary directory is needed.

### Verify integrity

```bash
sbk verify <file.sbk>
```

Checks xxHash32 checksums of all frames.

## SBK format

An `.sbk` file consists of:

1. **Header** (79 bytes) — magic `SBK!V1\r\n`, file counts, offsets, xxHash32 header checksum
2. **Data blocks** — four solid streams (one per file group: MCA, NBT, JSON, RAW), compressed with LZMA2 or Zstd as selected at compress time
3. **Frame directory** — per-frame offsets, sizes, and xxHash32 checksums
4. **Index block** — sorted file manifest, compressed with the same algorithm as the data blocks

All integers are little-endian. Frame size is 16 MiB uncompressed. See [SPEC.md](SPEC.md) for the full format specification.
