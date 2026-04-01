# SBK Benchmark Results

## System

| Component        | Value    |
|------------------|----------|
| **CPU**          | AMD Ryzen 7 7800X3D 8-Core Processor     |
| **RAM**          | 61,9 GB     |
| **Threads used** | 8 |

## Commands Used

```bash
# SBK (sbk 0.1.0) — lzma2 levels 1–9
sbk compress <world> -o <world>.sbk --algorithm lzma2 -l <1-9> -t 8
sbk decompress <world>.sbk -o <out_dir>/ -t 8

# SBK (sbk 0.1.0) — zstd levels 1–9
sbk compress <world> -o <world>.sbk --algorithm zstd -l <1-9> -t 8
sbk decompress <world>.sbk -o <out_dir>/ -t 8

# zip (default level)
zip -r <world>.zip <world>/
unzip -q <world>.zip -d <out_dir>/

# tar.gz (default level)
tar -czf <world>.tar.gz <world>/
tar -xzf <world>.tar.gz -C <out_dir>/

# 7z (default level)
7z a <world>.7z <world>/
7z x <world>.7z -o<out_dir>/
```

## Results

### 120 MB  _(Minecraft 26.1)_

| Method | Size | Ratio | Compress | Decompress |
|--------|-----:|------:|---------:|-----------:|
| Uncompressed | 115,3 MB | 100% | — | — |
| **sbk lzma2 -l 1** | 54,3 MB | 47,1% | 1,93s | 1,02s |
| **sbk lzma2 -l 2** | 53,9 MB | 46,7% | 2,38s | 0,97s |
| **sbk lzma2 -l 3** | 53,7 MB | 46,6% | 3,25s | 0,98s |
| **sbk lzma2 -l 4** | 52,8 MB | 45,8% | 6,10s | 0,98s |
| **sbk lzma2 -l 5** | 49,2 MB | 42,7% | 9,71s | 1,01s |
| **sbk lzma2 -l 6** | 46,7 MB | 40,5% | 13,30s | 0,97s |
| **sbk lzma2 -l 7** | 46,6 MB | 40,4% | 13,96s | 0,97s |
| **sbk lzma2 -l 8** | 46,6 MB | 40,4% | 14,41s | 1,07s |
| **sbk lzma2 -l 9** | 46,6 MB | 40,4% | 14,54s | 1,05s |
| **sbk zstd -l 1** | 72,1 MB | 62,5% | 0,70s | 0,74s |
| **sbk zstd -l 2** | 67,4 MB | 58,4% | 0,83s | 0,72s |
| **sbk zstd -l 3** | 61,1 MB | 53,0% | 1,06s | 0,71s |
| **sbk zstd -l 4** | 60,0 MB | 52,0% | 1,24s | 0,73s |
| **sbk zstd -l 5** | 58,8 MB | 51,0% | 1,98s | 0,78s |
| **sbk zstd -l 6** | 58,7 MB | 50,9% | 2,99s | 0,72s |
| **sbk zstd -l 7** | 57,3 MB | 49,7% | 5,50s | 0,72s |
| **sbk zstd -l 8** | 52,9 MB | 45,9% | 7,92s | 0,76s |
| **sbk zstd -l 9** | 49,7 MB | 43,1% | 17,55s | 0,74s |
| zip      | 79,2 MB | 68,7% | 1,59s | 0,52s |
| tar.gz   | 79,2 MB | 68,7% | 1,64s | 0,48s |
| 7z       | 78,1 MB  | 67,8%  | 2,51s  | 1,64s  |

### 1 GB  _(Minecraft 26.1)_

| Method | Size | Ratio | Compress | Decompress |
|--------|-----:|------:|---------:|-----------:|
| Uncompressed | 955,0 MB | 100% | — | — |
| **sbk lzma2 -l 1** | 505,3 MB | 52,9% | 14,16s | 8,55s |
| **sbk lzma2 -l 2** | 500,9 MB | 52,5% | 18,30s | 8,55s |
| **sbk lzma2 -l 3** | 499,8 MB | 52,3% | 25,63s | 8,38s |
| **sbk lzma2 -l 4** | 491,3 MB | 51,4% | 48,91s | 8,30s |
| **sbk lzma2 -l 5** | 459,3 MB | 48,1% | 1m 16s | 8,57s |
| **sbk lzma2 -l 6** | 435,6 MB | 45,6% | 1m 51s | 8,17s |
| **sbk lzma2 -l 7** | 435,1 MB | 45,6% | 1m 51s | 8,04s |
| **sbk lzma2 -l 8** | 435,1 MB | 45,6% | 1m 55s | 8,56s |
| **sbk lzma2 -l 9** | 435,1 MB | 45,6% | 1m 56s | 8,75s |
| **sbk zstd -l 1** | 676,4 MB | 70,8% | 4,19s | 6,17s |
| **sbk zstd -l 2** | 633,3 MB | 66,3% | 5,48s | 6,09s |
| **sbk zstd -l 3** | 571,9 MB | 59,9% | 7,27s | 6,27s |
| **sbk zstd -l 4** | 561,7 MB | 58,8% | 8,80s | 6,00s |
| **sbk zstd -l 5** | 551,1 MB | 57,7% | 14,58s | 6,07s |
| **sbk zstd -l 6** | 549,7 MB | 57,6% | 20,84s | 6,29s |
| **sbk zstd -l 7** | 537,6 MB | 56,3% | 38,65s | 6,25s |
| **sbk zstd -l 8** | 496,9 MB | 52,0% | 1m 1s | 6,02s |
| **sbk zstd -l 9** | 466,3 MB | 48,8% | 2m 18s | 6,15s |
| zip      | 708,1 MB | 74,1% | 13,85s | 4,29s |
| tar.gz   | 708,0 MB | 74,1% | 14,23s | 3,93s |
| 7z       | 703,4 MB  | 73,7%  | 11,10s  | 2,98s  |

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
Requires: `zip`, `unzip`. Optional: `tar` (for tar.gz), `7z` (`sudo apt install p7zip-full`).
