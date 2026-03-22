# SBK Benchmark Results

## System

| Component        | Value    |
|------------------|----------|
| **CPU**          | AMD Ryzen 7 7800X3D 8-Core Processor     |
| **RAM**          | 61,9 GB     |
| **Threads used** | 16 |

## Commands Used

```bash
# SBK (sbk 0.1.0)
sbk compress <world> -o <world>.sbk -l 1 -t 16
sbk compress <world> -o <world>.sbk -l 5 -t 16
sbk compress <world> -o <world>.sbk -l 9 -t 16
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
| **sbk -l 1** | 3,1 MB | 24,2% | 0,29s | 0,13s |
| **sbk -l 5** | 2,7 MB | 21,2% | 1,52s | 0,14s |
| **sbk -l 9** | 2,6 MB | 20,1% | 2,27s | 0,13s |
| zip -1 | 6,5 MB | 51,6% | 0,13s | 0,05s |
| zip -5 | 6,5 MB | 51,4% | 0,14s | 0,06s |
| zip -9 | 6,5 MB | 51,4% | 0,17s | 0,06s |
| tar.xz | 5,6 MB | 44,4% | 1,09s | 0,22s |
| 7z -mx=1 | 6,2 MB | 49,2% | 0,07s | 0,04s |
| 7z -mx=5 | 5,6 MB | 44,2% | 0,32s | 0,21s |
| 7z -mx=9 | 5,6 MB | 44,2% | 0,34s | 0,21s |

### 50  _(Minecraft 1.21.1)_

| Method | Size | Ratio | Compress | Decompress |
|--------|-----:|------:|---------:|-----------:|
| Uncompressed | 129,3 MB | 100% | — | — |
| **sbk -l 1** | 59,2 MB | 45,8% | 1,84s | 1,21s |
| **sbk -l 5** | 53,3 MB | 41,2% | 7,68s | 1,19s |
| **sbk -l 9** | 50,6 MB | 39,1% | 11,74s | 1,23s |
| zip -1 | 94,4 MB | 73,0% | 1,74s | 0,55s |
| zip -5 | 94,3 MB | 72,9% | 1,85s | 0,58s |
| zip -9 | 94,2 MB | 72,9% | 2,07s | 0,58s |
| tar.xz | 90,1 MB | 69,7% | 20,03s | 3,50s |
| 7z -mx=1 | 93,6 MB | 72,4% | 0,61s | 0,35s |
| 7z -mx=5 | 89,6 MB | 69,3% | 3,17s | 1,74s |
| 7z -mx=9 | 88,6 MB | 68,5% | 9,31s | 3,26s |

### 10  _(Minecraft 26.1)_

| Method | Size | Ratio | Compress | Decompress |
|--------|-----:|------:|---------:|-----------:|
| Uncompressed | 37,7 MB | 100% | — | — |
| **sbk -l 1** | 15,2 MB | 40,3% | 0,52s | 0,35s |
| **sbk -l 5** | 13,8 MB | 36,5% | 2,38s | 0,35s |
| **sbk -l 9** | 13,0 MB | 34,6% | 3,60s | 0,35s |
| zip -1 | 22,3 MB | 59,1% | 0,42s | 0,15s |
| zip -5 | 22,2 MB | 58,9% | 0,46s | 0,17s |
| zip -9 | 22,2 MB | 58,9% | 0,53s | 0,17s |
| tar.xz | 21,7 MB | 57,5% | 4,12s | 0,87s |
| 7z -mx=1 | 22,0 MB | 58,2% | 0,16s | 0,10s |
| 7z -mx=5 | 21,7 MB | 57,4% | 1,00s | 0,82s |
| 7z -mx=9 | 21,6 MB | 57,4% | 1,11s | 0,82s |

### 50  _(Minecraft 26.1)_

| Method | Size | Ratio | Compress | Decompress |
|--------|-----:|------:|---------:|-----------:|
| Uncompressed | 115,3 MB | 100% | — | — |
| **sbk -l 1** | 54,3 MB | 47,1% | 1,76s | 1,13s |
| **sbk -l 5** | 49,2 MB | 42,7% | 7,82s | 1,12s |
| **sbk -l 9** | 46,6 MB | 40,4% | 11,48s | 1,14s |
| zip -1 | 79,4 MB | 68,9% | 1,47s | 0,48s |
| zip -5 | 79,2 MB | 68,7% | 1,54s | 0,52s |
| zip -9 | 79,2 MB | 68,7% | 1,77s | 0,52s |
| tar.xz | 78,1 MB | 67,8% | 16,34s | 3,09s |
| 7z -mx=1 | 78,9 MB | 68,5% | 0,51s | 0,30s |
| 7z -mx=5 | 78,1 MB | 67,8% | 2,50s | 1,64s |
| 7z -mx=9 | 78,0 MB | 67,6% | 6,80s | 2,89s |

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
