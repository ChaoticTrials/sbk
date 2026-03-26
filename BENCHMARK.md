# SBK Benchmark Results

## System

| Component        | Value    |
|------------------|----------|
| **CPU**          | AMD Ryzen 7 7800X3D 8-Core Processor     |
| **RAM**          | 61,9 GB     |
| **Threads used** | 16 |

## Commands Used

```bash
# SBK (sbk 0.1.0) — lzma2 levels 1–9
sbk compress <world> -o <world>.sbk --algorithm lzma2 -l <1-9> -t 16
sbk decompress <world>.sbk -o <out_dir>/ -t 16

# SBK (sbk 0.1.0) — zstd levels 1–9
sbk compress <world> -o <world>.sbk --algorithm zstd -l <1-9> -t 16
sbk decompress <world>.sbk -o <out_dir>/ -t 16

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

> ★ marks the sbk variant with the best compression×speed score (smallest ratio × compress time).

## Results

### 120 MB  _(Minecraft 26.1)_

| Method | Size | Ratio | Compress | Decompress |
|--------|-----:|------:|---------:|-----------:|
| Uncompressed | 115,3 MB | 100% | — | — |
| **sbk lzma2 -l 1** | 54,3 MB | 47,1% | 1,63s | 0,76s |
| **sbk lzma2 -l 2** | 53,9 MB | 46,7% | 1,97s | 0,74s |
| **sbk lzma2 -l 3** | 53,7 MB | 46,6% | 2,65s | 0,75s |
| **sbk lzma2 -l 4** | 52,8 MB | 45,8% | 4,71s | 0,76s |
| **sbk lzma2 -l 5** | 49,2 MB | 42,7% | 6,96s | 0,74s |
| **sbk lzma2 -l 6** | 46,7 MB | 40,5% | 9,72s | 0,73s |
| **sbk lzma2 -l 7** | 46,6 MB | 40,4% | 10,34s | 0,72s |
| **sbk lzma2 -l 8** | 46,6 MB | 40,4% | 10,57s | 0,76s |
| **sbk lzma2 -l 9** | 46,6 MB | 40,4% | 10,56s | 0,77s |
| **sbk zstd -l 1** | 72,1 MB | 62,5% | 0,69s | 0,60s |
| **sbk zstd -l 2** | 67,4 MB | 58,4% | 0,78s | 0,60s |
| **sbk zstd -l 3** | 61,1 MB | 53,0% | 0,98s | 0,58s |
| **sbk zstd -l 4** | 60,0 MB | 52,0% | 1,16s | 0,59s |
| **sbk zstd -l 5** | 58,8 MB | 51,0% | 1,66s | 0,57s |
| **sbk zstd -l 6** | 58,7 MB | 50,9% | 2,28s | 0,60s |
| **sbk zstd -l 7** | 57,3 MB | 49,7% | 3,93s | 0,58s |
| **sbk zstd -l 8** | 52,9 MB | 45,9% | 5,70s | 0,58s |
| **sbk zstd -l 9** | 49,7 MB | 43,1% | 13,54s | 0,58s |
| zip      | 79,2 MB | 68,7% | 1,56s | 0,51s |
| tar.gz   | 79,2 MB | 68,7% | 1,62s | 0,47s |
| 7z       | 78,1 MB  | 67,8%  | 2,38s  | 1,62s  |

### 1 GB  _(Minecraft 26.1)_

| Method | Size | Ratio | Compress | Decompress |
|--------|-----:|------:|---------:|-----------:|
| Uncompressed | 955,0 MB | 100% | — | — |
| **sbk lzma2 -l 1** | 505,3 MB | 52,9% | 10,85s | 5,45s |
| **sbk lzma2 -l 2** | 500,9 MB | 52,5% | 13,84s | 5,43s |
| **sbk lzma2 -l 3** | 499,8 MB | 52,3% | 18,77s | 5,49s |
| **sbk lzma2 -l 4** | 491,3 MB | 51,4% | 34,75s | 5,62s |
| **sbk lzma2 -l 5** | 459,3 MB | 48,1% | 52,79s | 5,51s |
| **sbk lzma2 -l 6** | 435,6 MB | 45,6% | 1m 14s | 5,42s |
| **sbk lzma2 -l 7** | 435,1 MB | 45,6% | 1m 17s | 5,49s |
| **sbk lzma2 -l 8** | 435,1 MB | 45,6% | 1m 20s | 5,69s |
| **sbk lzma2 -l 9** | 435,1 MB | 45,6% | 1m 20s | 5,60s |
| **sbk zstd -l 1** | 676,4 MB | 70,8% | 3,71s | 4,63s |
| **sbk zstd -l 2** | 633,3 MB | 66,3% | 4,81s | 4,55s |
| **sbk zstd -l 3** | 571,9 MB | 59,9% | 6,24s | 4,23s |
| **sbk zstd -l 4** | 561,7 MB | 58,8% | 7,68s | 4,34s |
| **sbk zstd -l 5** | 551,1 MB | 57,7% | 12,64s | 4,36s |
| **sbk zstd -l 6** | 549,7 MB | 57,6% | 17,02s | 4,46s |
| **sbk zstd -l 7** | 537,6 MB | 56,3% | 31,95s | 4,49s |
| **sbk zstd -l 8** | 496,9 MB | 52,0% | 48,07s | 4,73s |
| **sbk zstd -l 9** | 466,3 MB | 48,8% | 1m 55s | 5,02s |
| zip      | 708,1 MB | 74,1% | 13,90s | 4,30s |
| tar.gz   | 708,0 MB | 74,1% | 14,25s | 3,90s |
| 7z       | 703,4 MB  | 73,7%  | 10,97s  | 3,21s  |

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
