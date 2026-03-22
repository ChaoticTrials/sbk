# SBK File Format & Compression Specification

## 1. Project Overview

`sbk` is a command-line tool written in Rust that compresses Minecraft Java Edition save directories into a single `.sbk` archive. The archive is significantly smaller than the original save, is not readable by Minecraft, and can be fully restored by the companion decompressor. Individual files or subtrees can be extracted without decompressing the entire archive.

### Goals

- Achieve better compression than `tar.xz` on a Minecraft world by exploiting Minecraft-specific data structure
- Preserve every file's **exact relative path** and **last-modified timestamp** (millisecond precision)
- Support **relative age filtering** during compression (`--max-age <ms>`): skip files whose mtime is more than N milliseconds before now
- Support **absolute timestamp filtering** during compression (`--since <timestamp>`): skip files whose mtime is below a given millisecond Unix timestamp
- Support **file pattern filtering** during compression: restrict which files are included via glob patterns
- Support **selective extraction** of individual files or subtrees by path or glob pattern
- Saturate available CPU cores during both compression and decompression
- Produce a single self-contained `.sbk` file
- Provide `sbk compress`, `sbk decompress`, `sbk extract`, `sbk info`, and `sbk verify` subcommands in one binary

### Non-goals

- Encryption
- Incremental / differential updates
- Streaming partial reads

---

## 2. Minecraft World Structure (Reference)

A Minecraft Java Edition save directory looks like this:

```
<world_name>/
  level.dat                        # gzip-compressed NBT, global world data
  level.dat_old                    # backup of level.dat
  session.lock                     # lock file (tiny)
  icon.png                         # optional world icon
  region/
    r.X.Z.mca                      # region files — the bulk of world data
  entities/
    r.X.Z.mca
  poi/
    r.X.Z.mca
  DIM-1/region/r.X.Z.mca           # Nether
  DIM1/region/r.X.Z.mca            # End
  playerdata/<uuid>.dat            # per-player gzip-compressed NBT
  advancements/<uuid>.json
  stats/<uuid>.json
  data/*.dat                       # maps, raids, villages, etc.
  datapacks/                       # optional zip files
```

### Key insight: MCA files are internally compressed with a tiny window

Each `.mca` file stores up to 1024 chunks. Each chunk is individually compressed with **zlib deflate** (or zstd in 1.20+). This per-chunk compression resets the LZ dictionary every few kilobytes, which is disastrous for a global compressor. The SBK pipeline strips all inner compression, sorts the raw chunk data by spatial locality, then recompresses the result as a large solid stream — giving LZMA a much larger and more repetitive window to work with.

---

## 3. File Classification

Every file that passes the compression filters is assigned to exactly one of four groups before any processing begins.

| Group ID | Name   | Matched patterns                  | Preprocessing applied                               |
|----------|--------|-----------------------------------|-----------------------------------------------------|
| 0        | `MCA`  | `**/*.mca`                        | MCA chunk decompression + Hilbert sort (see §5)     |
| 1        | `NBT`  | `**/*.dat`, `**/*.dat_old`        | Strip outer gzip layer, store raw NBT               |
| 2        | `JSON` | `**/*.json`                       | Minify (remove all insignificant whitespace)        |
| 3        | `RAW`  | everything else                   | No preprocessing, store verbatim                    |

Classification is based purely on file extension. `session.lock` falls into `RAW` and is excluded by default; pass `--include-session-lock` to override.

---

## 4. Compression Filters

Compression filters are evaluated during file enumeration, before classification and before any preprocessing. A file that does not pass all active filters is silently skipped — it will not appear in the archive at all and cannot be extracted later.

Filters are applied in this order:

1. **Hardcoded skip**: `session.lock` is excluded by default. Pass `--include-session-lock` to override.
2. **Relative age filter** (optional, `--max-age <ms>`): skip files whose mtime is more than N milliseconds before now.
3. **Absolute timestamp filter** (optional, `--since <timestamp>`): skip files whose mtime is below the given millisecond Unix timestamp.
4. **Pattern filter** (optional, `--exclude` or `--include`, mutually exclusive): skip files that do not match the user's pattern rules.

Filters 2 and 3 are independent and can be combined. A file must pass every active filter to be included.

### 4.1 Relative Age Filter (`--max-age <ms>`)

When `--max-age N` is provided, the compressor records the current wall-clock time once at the very start of the run (before `walkdir` traversal begins). Any file whose mtime is more than N milliseconds before that reference point is excluded.

```
cutoff_ms = now_unix_ms - N
include file if: file_mtime_ms >= cutoff_ms
```

`N` is a positive integer in **milliseconds**. It must be ≥ 1. A value of `0` is rejected at startup before any files are read.

Examples:
- `--max-age 3600000` — only files modified in the last hour (3 600 000 ms)
- `--max-age 86400000` — only files modified in the last 24 hours
- `--max-age 604800000` — only files modified in the last 7 days

**Why milliseconds?** The mtime values stored in the archive index are milliseconds since the Unix epoch. Using the same unit means `--max-age` and `--since` operate on the same number line with no unit conversion, no rounding surprises, and no ambiguity about what "1 day" means across DST boundaries.

**Implementation note:** capture `now_unix_ms` once before traversal. Pass it as a plain `i64` through the pipeline so `accept()` never calls the clock itself.

### 4.2 Absolute Timestamp Filter (`--since <timestamp>`)

When `--since T` is provided, any file whose mtime is strictly less than T is excluded.

```
include file if: file_mtime_ms >= T
```

`T` is a **millisecond Unix timestamp**. There is no arithmetic against the current clock; the comparison is a direct integer comparison between the file's stored mtime and the provided value.

Examples:
- `--since 1700000000000` — include only files modified on or after 2023-11-14 22:13:20 UTC
- `--since $(date -d "2024-01-01" +%s%3N)` — shell expansion to a concrete timestamp

**Distinction from `--max-age`:**

| Flag                  | Reference point           | Argument means                                |
|-----------------------|---------------------------|-----------------------------------------------|
| `--max-age <ms>`      | Current time at run start | Duration in milliseconds to look back         |
| `--since <timestamp>` | Fixed point in time       | Millisecond Unix timestamp to compare against |

Both can be active simultaneously. A file must satisfy both:
```
file_mtime_ms >= (now_unix_ms - max_age_ms)   [if --max-age is set]
file_mtime_ms >= since_ms                      [if --since is set]
```

When both are set, the effective cutoff is `max(now_unix_ms - max_age_ms, since_ms)`, though the implementation simply checks both conditions independently and short-circuits on the first failure.

### 4.3 Pattern Filter (`--exclude` / `--include`)

These two flags are mutually exclusive. The user may specify one or more glob patterns with each flag. Patterns are matched against the file's **relative path within the world directory** using forward slashes (e.g., `region/r.0.0.mca`, `playerdata/abc123.dat`). Matching uses `glob::Pattern::matches`. Patterns follow standard Unix glob syntax: `*` matches within one path segment, `**` matches across path separators.

**`--exclude <pattern> [<pattern>...]`**

Include all files by default; exclude any file whose relative path matches at least one of the provided patterns.

```
--exclude "region/*.mca"              skip all overworld region files
--exclude "DIM-1/**" "DIM1/**"        skip Nether and End entirely
--exclude "*.dat_old"                 skip all backup dat files
```

**`--include <pattern> [<pattern>...]`**

Exclude all files by default; include only files whose relative path matches at least one of the provided patterns.

```
--include "region/*.mca"              only overworld region files
--include "playerdata/**" "stats/**"  only player data
--include "level.dat"                 only the root level file
```

**Mutual exclusivity:** if both `--exclude` and `--include` are specified, the CLI must reject the command immediately with a clear error message before opening any files:
```
error: --exclude and --include are mutually exclusive; use one or the other
```

### 4.4 `CompressOptions` Struct

```rust
pub struct CompressOptions {
    pub output:               PathBuf,
    pub threads:              usize,
    pub level:                u32,           // LZMA preset 1–9
    pub max_age:              Option<u64>,   // milliseconds; None = no relative age filter
    pub since:                Option<i64>,   // millisecond Unix timestamp; None = no absolute filter
    pub patterns:             FilterMode,
    pub include_session_lock: bool,          // if true, session.lock is not excluded
    pub quiet:                bool,          // suppress all output (progress bars + summary)
}

pub enum FilterMode {
    None,
    Exclude(Vec<glob::Pattern>),   // skip matching files
    Include(Vec<glob::Pattern>),   // skip non-matching files
}
```

### 4.5 File Acceptance Logic

```rust
pub fn accept(
    rel_path:             &str,
    mtime_ms:             i64,
    now_ms:               i64,
    max_age_ms:           Option<u64>,
    since_ms:             Option<i64>,
    filter:               &FilterMode,
    include_session_lock: bool,
) -> bool {
    // 1. Hardcoded skip (overridable)
    if !include_session_lock && rel_path == "session.lock" { return false; }

    // 2. Relative age filter
    if let Some(age) = max_age_ms {
        if mtime_ms < now_ms - age as i64 { return false; }
    }

    // 3. Absolute timestamp filter
    if let Some(since) = since_ms {
        if mtime_ms < since { return false; }
    }

    // 4. Pattern filter
    match filter {
        FilterMode::None               => true,
        FilterMode::Exclude(patterns)  => !patterns.iter().any(|p| p.matches(rel_path)),
        FilterMode::Include(patterns)  =>  patterns.iter().any(|p| p.matches(rel_path)),
    }
}
```

This function is pure and stateless — safe to call from multiple threads simultaneously.

### 4.6 Startup Time Capture

```rust
pub fn capture_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
```

Call once before `walkdir` traversal. Pass `now_ms` into every `accept()` call. Never call inside the loop — all files must be compared against the same reference point.

---

## 5. Compression Algorithm

### 5.1 MCA Preprocessing — MCAP format

**Minecraft's MCA on-disk layout:**
```
Bytes 0x0000–0x0FFF   Location table:  1024 × 4-byte entries (big-endian)
                        Top 3 bytes = sector offset from file start
                        Low 1 byte  = sector count occupied by this chunk
Bytes 0x1000–0x1FFF   Timestamp table: 1024 × 4-byte Unix timestamps (big-endian, seconds)
Bytes 0x2000–EOF      Chunk data, sector-aligned (each sector = 4096 bytes):
                        [length: u32 BE]          = compressed_data_length + 1
                        [compression_type: u8]    1=gzip  2=zlib  3=none  4=zstd
                        [compressed_data: ...]
```

Slot index within the 1024-entry table: `slot = chunk_z_local * 32 + chunk_x_local`, both coordinates in [0, 31].

**Preprocessing steps for one MCA file:**

1. Parse all 1024 location table entries. Identify occupied slots (entry ≠ 0).
2. For each occupied slot: seek to `sector_offset * 4096`, read `length` (u32 BE) and `compression_type` (u8), then decompress `length - 1` bytes:
   - Type 1 (gzip): `flate2::read::MultiGzDecoder`
   - Type 2 (zlib): `flate2::read::ZlibDecoder`
   - Type 3 (none): use bytes directly
   - Type 4 (zstd): pure-Rust zstd decoder. The original compression type is discarded; all chunks are always re-compressed as zlib on reconstruction.
   - Unknown type: return `Err(SbkError::UnknownChunkCompression(n))`
3. Compute local coordinates: `local_x = slot % 32`, `local_z = slot / 32`.
4. Compute a **Hilbert curve index** for each chunk (see §5.2). Sort ascending. This groups spatially adjacent chunks together in the output byte stream.
5. Write the **MCAP stream** (§5.3) to an in-memory `Vec<u8>`.

Step 2 is parallelised within a single MCA file using Rayon.

### 5.2 Hilbert Curve (order 5, 32 × 32 grid)

Maps `(local_x, local_z)` ∈ [0,31]² to a 1D index in [0, 1023].

```rust
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
```

### 5.3 MCAP Stream Format

```
[magic:       4 bytes]       0x4D434150  ("MCAP")
[chunk_count: u16 LE]        number of occupied chunks (0–1024)
For each chunk in ascending Hilbert order:
  [local_x:   u8]            0–31
  [local_z:   u8]            0–31
  [raw_len:   u32 LE]        byte length of decompressed NBT
  [nbt_data:  raw_len bytes] raw (uncompressed) NBT bytes
```

### 5.4 NBT Preprocessing

Strip the outer gzip layer using `flate2::read::MultiGzDecoder`. Store raw NBT bytes. On reconstruction: re-wrap with `flate2::write::GzEncoder` at level 6.

### 5.5 JSON Preprocessing

Parse with `serde_json` into `serde_json::Value`, serialize compactly with `serde_json::to_vec`. On reconstruction: write compact bytes directly (Minecraft accepts compact JSON).

### 5.6 Frame-Based Solid Blocks

Files within the same group are concatenated into one **solid stream** and compressed together, allowing LZMA to exploit repetition across all files in that group. The solid stream is split into fixed-size **frames** before compression. Each frame is LZMA2-compressed **independently**, enabling selective extraction without decompressing the full archive.

**Frame size: 16 MiB uncompressed** (`FRAME_SIZE: u64 = 16 * 1024 * 1024`). Stored in the fixed header for forward compatibility.

**Compression: LZMA2** via `xz2::write::XzEncoder` at the user-specified preset (1–9, default 9). Preset 9 uses a 64 MiB sliding dictionary, which vastly outperforms Minecraft's per-chunk 32 KB window. All frames across all groups are compressed in parallel.

### 5.7 MCAP Reconstruction (Decompression)

1. Validate 4-byte magic. Read `chunk_count: u16 LE`.
2. Parse all `(local_x, local_z, raw_nbt)` entries.
3. **Parallel chunk re-compression** with Rayon: compress each chunk's NBT using `flate2::write::ZlibEncoder` at level 6. Set `compression_type = 2`.
4. Lay chunks out starting at sector 2. Compute `required_sectors = ceil((5 + compressed_len) / 4096)`. Zero-pad to sector boundary.
5. Build location table: for each occupied `slot = local_z * 32 + local_x`, write `[sector_offset: u24 BE | sector_count: u8]`.
6. Write zeroed timestamp table (1024 × 4 bytes). Minecraft regenerates on load.
7. Write all sector data. Caller restores mtime after file is closed.

---

## 6. SBK File Format Specification v1

### 6.1 Overall Layout

```
[File Magic]        8 bytes
[Fixed Header]      68 bytes
[Data Blocks]       variable   (frame data for all 4 groups, consecutive)
[Frame Directory]   variable   (written after data; offset stored in Fixed Header)
[Index Block]       variable   (written last; offset stored in Fixed Header)
```

Readers always locate the Frame Directory by seeking to `frame_dir_offset` from the Fixed Header, so its position after the data blocks is fully compatible.

All integers are **little-endian** unless explicitly noted otherwise.

### 6.2 File Magic

```
Bytes 0–7:  53 42 4B 21 56 31 0D 0A   →   "SBK!V1\r\n"
```

The `\r\n` suffix detects accidental line-ending conversion.

### 6.3 Fixed Header (bytes 8–75, 68 bytes)

```
Relative  Size  Type    Field
offset
--------  ----  ------  -----
0         1     u8      format_version          currently = 1
1         1     u8      flags                   reserved, must be 0
2         2     u16     reserved                must be 0
4         8     u64     file_count              total files stored in this archive
12        8     u64     frame_size_bytes        uncompressed frame size (= 16777216)
20        8     u64     frame_dir_offset        absolute byte offset of Frame Directory
28        8     u64     frame_dir_size          byte size of Frame Directory on disk
36        8     u64     index_offset            absolute byte offset of Index Block
44        8     u64     index_compressed_size   compressed byte size of Index Block
52        8     u64     index_raw_size          uncompressed byte size of Index Block
60        4     u32     index_checksum          xxHash32 of the compressed index bytes
64        4     u32     header_checksum         xxHash32 of bytes 0–71 (this field = 0 during compute)
```

Total on disk: 8 bytes magic + 68 bytes header = **76 bytes** before the Frame Directory.

**Computing `header_checksum`:** zero the 4 bytes at relative offset 64 (absolute bytes 72–75), compute xxHash32 over bytes 0–71 (72 bytes — covering all fields including `index_checksum`), write the result back at offset 64.

### 6.4 Frame Directory (at `frame_dir_offset`)

Flat list of per-frame metadata for all four groups, in group order 0–3. Enables O(1) seeking to any frame.

```
For each group g in [0, 1, 2, 3]:
  [frame_count: u32]              number of frames in this group (0 if group is empty)
  For each frame f in [0 .. frame_count):
    [frame_offset:        u64]    absolute byte offset of this frame's compressed data
    [frame_compressed_sz: u32]    byte count of compressed frame data on disk
    [frame_raw_sz:        u32]    byte count after decompression (≤ frame_size_bytes)
    [frame_checksum:      u32]    xxHash32 of the compressed frame bytes
```

Per-frame entry size: 20 bytes. An empty group contributes only the 4-byte `frame_count = 0`.

### 6.5 Data Blocks

Compressed frame payloads for all groups in group order (0, 1, 2, 3), frames within each group in ascending frame-index order. `frame_offset` fields are absolute byte offsets into the file. Each frame is an independent xz/LZMA2 stream; decompress with `xz2::read::XzDecoder` on exactly `frame_compressed_sz` bytes.

### 6.6 Index Block (at `index_offset`)

Written last so all frame offsets are known. Compressed as a single xz stream. Raw (pre-compression) content:

```
[entry_count: u64]
For each IndexEntry (sorted by relative path, ascending):
  [path_len:         u16]    byte length of the UTF-8 relative path string
  [path:             ...]    relative path, UTF-8, forward slashes, no leading slash
  [mtime_ms:         i64]    last-modified time in milliseconds since Unix epoch
  [group_id:         u8]     0=MCA  1=NBT  2=JSON  3=RAW
  [stream_offset:    u64]    byte offset of this file's preprocessed data within its
                             group's uncompressed solid stream
  [stream_raw_size:  u64]    byte length of this file's preprocessed data
  [original_size:    u64]    original file size before any preprocessing
  [file_checksum:    u32]    xxHash32 of original file bytes (before preprocessing)
```

The index contains only files that passed all compression filters. Excluded files have no entry and cannot be extracted.

---

## 7. Threading Model

`sbk` is designed to saturate all available cores at every stage. The user controls thread count with `--threads N` (default: logical CPU count). One global Rayon thread pool of size N is created once at startup via `build_global()` and reused everywhere, including nested parallel calls.

### 7.1 Compression Thread Pipeline

```
Stage 1 [parallel, per file]   — enumeration, filtering, preprocessing
  walkdir produces (absolute_path, relative_path, metadata) for every file.
  Rayon par_iter() over this list. For each file, each thread independently:
    a. Reads mtime_ms from metadata.
    b. Calls accept(rel_path, mtime_ms, now_ms, max_age_ms, since_ms, &filter).
       → false: skip immediately. Increment atomic skip counter. Done.
    c. Reads file bytes from disk.
    d. Computes file_checksum = xxHash32(original_bytes).
    e. Runs the appropriate preprocessor.
    f. Returns (Group, IndexEntryStub, preprocessed_bytes).

Stage 1a [parallel, per chunk within one MCA file]
  Inside preprocess_mca(), occupied chunk slots are decompressed in parallel.
  Results collected, then sorted by Hilbert index (cheap; ≤ 1024 elements).

Stage 2 [streaming, per group]   — frame assembly + compression + writing
  For each group 0 → 3, files are processed in path-sorted order.
  Files within each group are preprocessed in parallel batches of N (= thread count).
  Preprocessed bytes are drained in sorted order into a 16 MiB frame buffer.
  When the frame buffer reaches FRAME_SIZE, the frame is added to a compression batch.
  When the compression batch reaches N frames, all N are LZMA2-compressed in parallel
  and written to disk in order. This bounds peak memory to O(N × FRAME_SIZE).

Stage 3 [single-threaded, I/O bound]   — finalize
  1. Write 76 placeholder zero bytes for Fixed Header.
  2. Stream frame data for groups 0 → 3 (interleaved with Stage 2 above).
  3. Write Frame Directory (now fully known) immediately after the last frame.
  4. Compress Index Block as a single xz stream; write it; record offset and sizes.
  5. Seek to byte 0. Write complete Fixed Header with all fields filled.
  6. Compute and write header_checksum.
```

### 7.2 Decompression / Extraction Thread Pipeline

```
Stage 1 [single-threaded]   — index and pattern matching
  • Read and validate Fixed Header (magic + header_checksum).
  • Read Frame Directory into memory.
  • Read and decompress Index Block; parse all IndexEntry records.
  • Match requested patterns against all entry paths using glob::Pattern::matches.

Stage 2 [parallel, per unique frame needed]   — frame decompression
  Collect all (group_id, frame_index) pairs from matched entries.
  Deduplicate into a HashSet. Decompress each unique frame exactly once, in parallel.
  Store results in HashMap<(u8, u32), Vec<u8>>.
  This map is fully built and read-only before Stage 3 begins — no locking needed.

Stage 3 [parallel, per matched file]   — reconstruction and writing
  matched_entries.par_iter(). For each entry:
    • Slice preprocessed bytes from the frame map via slice_from_frames() (lock-free).
    • Call the appropriate reconstruct_*().
    • Create parent directories via a Mutex<HashSet<PathBuf>> cache.
    • Write output file.
    • Restore mtime with filetime::set_file_mtime.

Stage 3a [parallel, per chunk within one MCA reconstruction]
  Inside reconstruct_mca(), zlib re-compression of all chunks is parallel.
```

### 7.3 Thread Safety Considerations

- **File handles**: each thread writes to its own output file. No sharing.
- **Directory creation**: `Mutex<HashSet<PathBuf>>` guards `create_dir_all`. Without this, concurrent threads may race creating the same parent directory.
- **Frame cache**: `HashMap<(u8, u32), Vec<u8>>` is fully populated before Stage 3 and is read-only thereafter. No locking needed.
- **Skip counter**: `std::sync::atomic::AtomicU64` counts filtered-out files from multiple threads without a mutex.
- **Progress bars**: `indicatif::ProgressBar` is `Send + Sync`. Clone the handle and call `.inc(1)` from any thread.
- **Rayon nesting**: one global pool handles both outer (per-file) and inner (per-chunk) parallelism via work-stealing. No thread over-subscription occurs.

---

## 8. Selective Extraction Design

### 8.1 Frame Arithmetic

Given an `IndexEntry` with `stream_offset` and `stream_raw_size`:

```
start_frame = stream_offset / frame_size_bytes
start_intra = stream_offset % frame_size_bytes
end_frame   = (stream_offset + stream_raw_size - 1) / frame_size_bytes
end_intra   = (stream_offset + stream_raw_size - 1) % frame_size_bytes + 1
```

Byte ranges to slice from decompressed frames:
- Single frame (`start_frame == end_frame`): `frame[start_intra .. end_intra]`
- Start frame: `frame[start_intra .. frame_size_bytes]`
- Middle frames: `frame[0 .. frame_size_bytes]`
- End frame: `frame[0 .. end_intra]`

A file almost always fits within one 16 MiB frame. Two frames are needed only if the file straddles a frame boundary.

### 8.2 Multi-File Frame Deduplication

Stage 2 of extraction collects all `(group_id, frame_index)` pairs from all matched entries, deduplicates them into a `HashSet`, then decompresses each unique frame exactly once in parallel. Extracting 800 MCA files that happen to land in 60 frames requires the same 60 decompressions as a full decompress.

### 8.3 Glob Matching (Extraction)

Patterns for `sbk extract` follow the same glob syntax as the compression pattern filter.

```
"region/r.0.0.mca"    exact file
"region/*.mca"        all overworld region files
"playerdata/**"       all player files including subdirectories
"**/*.dat"            all NBT files anywhere
"**"                  everything (equivalent to sbk decompress)
```

---

## 9. CLI Design

One binary: `sbk`

```
USAGE:
  sbk compress   <world_dir>  [OPTIONS]
  sbk decompress <file.sbk>   [OPTIONS]
  sbk extract    <file.sbk>   <pattern>...  [OPTIONS]
  sbk info       <file.sbk>   [--list]
  sbk verify     <file.sbk>

COMPRESS OPTIONS:
  -o, --output  <file.sbk>           Default: <world_dir_name>.sbk in current dir
  -t, --threads <n>                  Default: logical CPU count
  -l, --level   <1-9>                LZMA preset; default 9
      --max-age  <ms>                Skip files not modified within the last N milliseconds
      --since    <timestamp>         Skip files with mtime below this millisecond Unix timestamp
      --exclude  <pattern>...        Skip files matching any of these glob patterns
      --include  <pattern>...        Include ONLY files matching any of these glob patterns
                                     (--exclude and --include are mutually exclusive)
      --include-session-lock         Include session.lock (excluded by default)
  -q, --quiet                        Suppress progress bars and summary output

DECOMPRESS OPTIONS:
  -o, --output  <dir>                Default: <world_name>/ in current dir
  -t, --threads <n>                  Default: logical CPU count
  -q, --quiet                        Suppress output

EXTRACT OPTIONS:
  <pattern>...                       One or more exact paths or glob patterns (required)
  -o, --output  <dir>                Default: current dir; preserves subdirectory structure
  -t, --threads <n>                  Default: logical CPU count
  -q, --quiet                        Suppress output

INFO OPTIONS:
  --list                             Print full file manifest table

NOTES:
  sbk decompress is equivalent to: sbk extract <file.sbk> "**"
  sbk verify exits with code 0 on success, 1 on any checksum failure.
  Files excluded during compression are absent from the archive and cannot be extracted.
  session.lock is excluded by default; use --include-session-lock to override.
```

### 9.1 Startup Validation (compress)

Before opening any files, validate:
1. `--max-age` value is ≥ 1 if provided. Error if 0.
2. `--since` value is a non-negative integer if provided.
3. `--exclude` and `--include` are not both present.
4. All glob patterns in `--exclude` / `--include` are syntactically valid. Error and list all invalid patterns if any fail.
5. `--level` is in range 1–9.
6. `world_dir` exists and is a directory.

These checks all run before the Rayon pool is created and before `walkdir` traversal begins. `capture_now_ms()` is called only after all validation passes.

### 9.2 Post-Compression Filter Summary

After the compression run completes, print:
```
Scanned 1,402 files → included 1,247  (155 skipped by filters)
```

If `--max-age` was used, also print the computed cutoff as an absolute timestamp:
```
--max-age 3600000 ms  →  cutoff timestamp: 1735000000000  (2024-12-24 10:26:40 UTC)
```

If `--since` was used:
```
--since 1700000000000  (2023-11-14 22:13:20 UTC)
```

---

## 10. Expected Compression Performance

Benchmarks on a typical survival world (~1 GB):

| Method                   | Output size     | Notes                                     |
|--------------------------|-----------------|-------------------------------------------|
| Raw `.tar`               | ~1000 MB        | no compression                            |
| `tar.gz` (gzip -9)       | ~750 MB         | per-file, small window                    |
| `tar.xz` (LZMA preset 9) | ~640 MB         | solid, best generic baseline              |
| **SBK**                  | **~570–610 MB** | MCA stripping + Hilbert sort + solid LZMA |

Using `--max-age` or `--exclude` on rarely-visited dimensions (e.g., `--exclude "DIM-1/**"`) reduces output size proportionally.
