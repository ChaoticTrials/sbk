# SBK Benchmark Results

## System

| Component        | Value    |
|------------------|----------|
| **CPU**          | AMD Ryzen 7 7800X3D 8-Core Processor     |
| **RAM**          | 61,9 GB     |
| **Threads used** | 16 |

## Commands Used

```bash
# SBK (sbk 0.1.0) — lzma2 (default)
sbk compress <world> -o <world>.sbk -l 1 -t 16
sbk compress <world> -o <world>.sbk -l 5 -t 16
sbk compress <world> -o <world>.sbk -l 9 -t 16
sbk decompress <world>.sbk -o <out_dir>/ -t 16

# SBK (sbk 0.1.0) — zstd
sbk compress <world> -o <world>.sbk --algorithm zstd -l 1 -t 16
sbk compress <world> -o <world>.sbk --algorithm zstd -l 5 -t 16
sbk compress <world> -o <world>.sbk --algorithm zstd -l 9 -t 16
sbk decompress <world>.sbk -o <out_dir>/ -t 16

# zip
zip -1 -r <world>.zip <world>/
zip -5 -r <world>.zip <world>/
zip -9 -r <world>.zip <world>/
unzip -q <world>.zip -d <out_dir>/

# tar.xz
tar -cJf <world>.tar.xz <world>/
tar -xJf <world>.tar.xz -C <out_dir>/

# 7z
7z a -mx=1 <world>.7z <world>/
7z a -mx=5 <world>.7z <world>/
7z a -mx=9 <world>.7z <world>/
7z x <world>.7z -o<out_dir>/
```

## Results

### 10  _(Minecraft 1.21.1)_

| Method | Size | Ratio | Compress | Decompress |
|--------|-----:|------:|---------:|-----------:|
| Uncompressed | 12,7 MB | 100% | — | — |
| **sbk lzma2 -l 1** | 3,1 MB | 24,2% | 0,28s | 0,13s |
| **sbk lzma2 -l 5** | 2,7 MB | 21,2% | 1,49s | 0,12s |
| **sbk lzma2 -l 9** | 2,6 MB  | 20,1%  | 2,24s  | 0,13s  |
| **sbk zstd -l 1**  | 4,0 MB | 31,4% | 0,07s | 0,07s |
| **sbk zstd -l 5**  | 3,2 MB | 24,9% | 0,27s | 0,08s |
| **sbk zstd -l 9**  | 2,7 MB | 21,4% | 2,54s | 0,08s |
| zip -1 | 6,5 MB | 51,6% | 0,13s | 0,05s |
| zip -5 | 6,5 MB | 51,4% | 0,14s | 0,06s |
| zip -9 | 6,5 MB | 51,4% | 0,17s | 0,06s |
| tar.xz | 5,6 MB | 44,4% | 0,99s | 0,22s |
| 7z -mx=1 | 6,2 MB | 49,2% | 0,09s | 0,04s |
| 7z -mx=5 | 5,6 MB | 44,2% | 0,33s | 0,21s |
| 7z -mx=9 | 5,6 MB | 44,2% | 0,34s | 0,22s |

### 50  _(Minecraft 1.21.1)_

| Method | Size | Ratio | Compress | Decompress |
|--------|-----:|------:|---------:|-----------:|
| Uncompressed | 129,3 MB | 100% | — | — |
| **sbk lzma2 -l 1** | 59,2 MB | 45,8% | 1,82s | 1,73s |
| **sbk lzma2 -l 5** | 53,3 MB | 41,2% | 7,39s | 1,11s |
| **sbk lzma2 -l 9** | 50,6 MB  | 39,1%  | 11,39s  | 1,32s  |
| **sbk zstd -l 1**  | 79,3 MB | 61,3% | 0,72s | 1,00s |
| **sbk zstd -l 5**  | 63,9 MB | 49,4% | 1,90s | 1,00s |
| **sbk zstd -l 9**  | 54,4 MB | 42,0% | 14,96s | 1,01s |
| zip -1 | 94,4 MB | 73,0% | 1,79s | 0,55s |
| zip -5 | 94,3 MB | 72,9% | 1,90s | 0,59s |
| zip -9 | 94,2 MB | 72,9% | 2,11s | 0,58s |
| tar.xz | 90,1 MB | 69,7% | 19,38s | 3,52s |
| 7z -mx=1 | 93,6 MB | 72,4% | 0,60s | 0,38s |
| 7z -mx=5 | 89,6 MB | 69,3% | 3,25s | 1,75s |
| 7z -mx=9 | 88,6 MB | 68,5% | 9,34s | 3,25s |

### 10  _(Minecraft 26.1)_

| Method | Size | Ratio | Compress | Decompress |
|--------|-----:|------:|---------:|-----------:|
| Uncompressed | 37,7 MB | 100% | — | — |
| **sbk lzma2 -l 1** | 15,2 MB | 40,3% | 0,52s | 0,34s |
| **sbk lzma2 -l 5** | 13,8 MB | 36,5% | 2,37s | 0,33s |
| **sbk lzma2 -l 9** | 13,0 MB  | 34,6%  | 3,54s  | 0,34s  |
| **sbk zstd -l 1**  | 20,2 MB | 53,5% | 0,22s | 0,27s |
| **sbk zstd -l 5**  | 16,4 MB | 43,6% | 0,56s | 0,29s |
| **sbk zstd -l 9**  | 13,9 MB | 36,8% | 4,65s | 0,28s |
| zip -1 | 22,3 MB | 59,1% | 0,42s | 0,16s |
| zip -5 | 22,2 MB | 58,9% | 0,45s | 0,17s |
| zip -9 | 22,2 MB | 58,9% | 0,54s | 0,17s |
| tar.xz | 21,7 MB | 57,5% | 4,00s | 0,87s |
| 7z -mx=1 | 22,0 MB | 58,2% | 0,17s | 0,09s |
| 7z -mx=5 | 21,7 MB | 57,4% | 1,08s | 0,82s |
| 7z -mx=9 | 21,6 MB | 57,4% | 1,12s | 0,82s |

### 50  _(Minecraft 26.1)_

| Method | Size | Ratio | Compress | Decompress |
|--------|-----:|------:|---------:|-----------:|
| Uncompressed | 115,3 MB | 100% | — | — |
| **sbk lzma2 -l 1** | 54,3 MB | 47,1% | 1,64s | 1,11s |
| **sbk lzma2 -l 5** | 49,2 MB | 42,7% | 7,51s | 1,08s |
| **sbk lzma2 -l 9** | 46,6 MB  | 40,4%  | 11,11s  | 1,09s  |
| **sbk zstd -l 1**  | 72,1 MB | 62,5% | 0,64s | 0,93s |
| **sbk zstd -l 5**  | 58,8 MB | 51,0% | 1,75s | 0,92s |
| **sbk zstd -l 9**  | 49,7 MB | 43,1% | 14,60s | 0,97s |
| zip -1 | 79,4 MB | 68,9% | 1,47s | 0,48s |
| zip -5 | 79,2 MB | 68,7% | 1,55s | 0,52s |
| zip -9 | 79,2 MB | 68,7% | 1,80s | 0,52s |
| tar.xz | 78,1 MB | 67,8% | 15,54s | 3,09s |
| 7z -mx=1 | 78,9 MB | 68,5% | 0,49s | 0,29s |
| 7z -mx=5 | 78,1 MB | 67,8% | 2,57s | 1,66s |
| 7z -mx=9 | 78,0 MB | 67,6% | 6,97s | 2,94s |

---

## How to Run on Your Own Hardware

1. **Build** sbk in release mode:
   ```bash
   cargo build --release
   ```

2. **Prepare worlds** — create `test-worlds/benchmark-worlds.zip` with this structure:
   ```
   benchmark-worlds.zip
   └── <mc_version>/
       └── <world_name>/
           ├── level.dat
           ├── region/
           └── …
   ```

3. **Run** the benchmark:
   ```bash
   ./benchmark.sh
   ```

Results are written to `BENCHMARK.md`.
Requires: `zip`, `unzip`. Optional: `tar`/`xz` (`sudo apt install xz-utils`), `7z` (`sudo apt install p7zip-full`).
