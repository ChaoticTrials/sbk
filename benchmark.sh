#!/usr/bin/env bash
# Benchmark sbk against zip, tar.gz, and 7z (if available).
# Reads test-worlds/benchmark-worlds.zip, runs all tools on every world,
# and writes results to BENCHMARK.md.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ZIP_FILE="$SCRIPT_DIR/test-worlds/benchmark-worlds.zip"
WORK_DIR="$(mktemp -d)"
OUT_DIR="$(mktemp -d)"
DEC_DIR="$(mktemp -d)"
MD="$SCRIPT_DIR/BENCHMARK.md"
SBK="$SCRIPT_DIR/target/release/sbk"
THREADS=8

cleanup() { rm -rf "$WORK_DIR" "$OUT_DIR" "$DEC_DIR"; }
trap cleanup EXIT

# ── Build ──────────────────────────────────────────────────────────────────────
echo "Building sbk (release)…"
cargo build --release --quiet --manifest-path "$SCRIPT_DIR/Cargo.toml"

# ── System info ────────────────────────────────────────────────────────────────
CPU=$(grep -m1 "model name" /proc/cpuinfo | cut -d: -f2- | xargs 2>/dev/null || echo "unknown")
RAM=$(awk '/MemTotal/ { printf "%.1f GB", $2 / 1048576 }' /proc/meminfo 2>/dev/null || echo "unknown")
SBK_VER=$("$SBK" --version 2>/dev/null || echo "unknown")

command -v zip   &>/dev/null || { echo "error: zip is required but not found";   exit 1; }
command -v unzip &>/dev/null || { echo "error: unzip is required but not found"; exit 1; }
HAS_TARGZ=false; command -v tar &>/dev/null && HAS_TARGZ=true
HAS_7Z=false;    command -v 7z  &>/dev/null && HAS_7Z=true

# ── Helpers ────────────────────────────────────────────────────────────────────
fmt_size() {
    local b=$1
    if (( b >= 1073741824 )); then
        awk "BEGIN { printf \"%.2f GB\", $b / 1073741824 }"
    else
        awk "BEGIN { printf \"%.1f MB\", $b / 1048576 }"
    fi
}

fmt_ratio() {
    local raw=$1 comp=$2
    awk "BEGIN { printf \"%.1f%%\", $comp * 100 / $raw }"
}

fmt_time() {
    local ms=$1
    if (( ms < 60000 )); then
        awk "BEGIN { printf \"%.2fs\", $ms / 1000 }"
    else
        awk "BEGIN { printf \"%.0fm %ds\", int($ms/60000), int(($ms%60000)/1000) }"
    fi
}

# Run a command, store elapsed ms in global _ms. Returns the command's exit code.
run_timed() {
    local _t; _t=$(date +%s%3N)
    if "$@"; then _ms=$(( $(date +%s%3N) - _t )); return 0
    else           _ms=0;                          return 1
    fi
}

# zip needs cd first; run in a subshell so the cd stays local.
_do_zip() { (cd "$1" && zip -rq "$2" "$3"); }

# Sort world names: pure integers numerically first, then strings alphabetically.
sorted_worlds() {
    local dir=$1
    local ints=() strs=()
    for d in "$dir"*/; do
        [[ -d "$d" ]] || continue
        local name; name=$(basename "$d")
        if [[ "$name" =~ ^[0-9]+$ ]]; then
            ints+=("$name")
        else
            strs+=("$name")
        fi
    done
    (
        [ ${#ints[@]} -gt 0 ] && printf '%s\n' "${ints[@]}" | sort -n
        [ ${#strs[@]} -gt 0 ] && printf '%s\n' "${strs[@]}" | sort
    )
}

# ── Unzip worlds ───────────────────────────────────────────────────────────────
echo "Unpacking benchmark-worlds.zip…"
unzip -q "$ZIP_FILE" -d "$WORK_DIR"

# ── Initialize BENCHMARK.md ────────────────────────────────────────────────────
{
    echo "# SBK Benchmark Results"
    echo ""
    echo "## System"
    echo ""
    echo "| Component        | Value    |"
    echo "|------------------|----------|"
    echo "| **CPU**          | $CPU     |"
    echo "| **RAM**          | $RAM     |"
    echo "| **Threads used** | $THREADS |"
    echo ""
    echo "## Commands Used"
    echo ""
    echo '```bash'
    echo "# SBK ($SBK_VER) — lzma2 levels 1–9"
    echo "sbk compress <world> -o <world>.sbk --algorithm lzma2 -l <1-9> -t $THREADS"
    echo "sbk decompress <world>.sbk -o <out_dir>/ -t $THREADS"
    echo ""
    echo "# SBK ($SBK_VER) — zstd levels 1–9"
    echo "sbk compress <world> -o <world>.sbk --algorithm zstd -l <1-9> -t $THREADS"
    echo "sbk decompress <world>.sbk -o <out_dir>/ -t $THREADS"
    echo ""
    echo "# zip (default level)"
    echo "zip -r <world>.zip <world>/"
    echo "unzip -q <world>.zip -d <out_dir>/"
    if $HAS_TARGZ; then
        echo ""
        echo "# tar.gz (default level)"
        echo "tar -czf <world>.tar.gz <world>/"
        echo "tar -xzf <world>.tar.gz -C <out_dir>/"
    fi
    if $HAS_7Z; then
        echo ""
        echo "# 7z (default level)"
        echo "7z a <world>.7z <world>/"
        echo "7z x <world>.7z -o<out_dir>/"
    fi
    echo '```'
    echo ""
    echo "## Results"
    echo ""
} > "$MD"

# ── Main benchmark loop ────────────────────────────────────────────────────────
for version_dir in "$WORK_DIR"/*/; do
    [[ -d "$version_dir" ]] || continue
    version=$(basename "$version_dir")

    while IFS= read -r world; do
        world_dir="$version_dir$world/"
        [[ -d "$world_dir" ]] || continue
        world_parent=$(dirname "$world_dir")

        echo ""
        echo "▶  Minecraft $version / $world"

        raw_bytes=$(du -sb "$world_dir" | awk '{print $1}')
        echo "   Uncompressed: $(fmt_size $raw_bytes)"

        out="$OUT_DIR/out.sbk"

        # Arrays: index = level (1–9)
        declare -a lzma_ok lzma_sz lzma_ms_c lzma_ms_d
        declare -a zstd_ok zstd_sz zstd_ms_c zstd_ms_d
        for l in $(seq 1 9); do
            lzma_ok[$l]=false; lzma_sz[$l]=0; lzma_ms_c[$l]=0; lzma_ms_d[$l]=0
            zstd_ok[$l]=false; zstd_sz[$l]=0; zstd_ms_c[$l]=0; zstd_ms_d[$l]=0
        done

        # ── sbk lzma2 levels 1–9 ──────────────────────────────────────────────
        for l in $(seq 1 9); do
            printf "   sbk lzma2 -l %d … " "$l"
            if run_timed "$SBK" compress "$world_dir" -o "$out" --algorithm lzma2 -l "$l" -t "$THREADS" >/dev/null 2>&1; then
                lzma_ms_c[$l]=$_ms; lzma_sz[$l]=$(du -sb "$out" | awk '{print $1}')
                rm -rf "$DEC_DIR/lzma$l"; mkdir -p "$DEC_DIR/lzma$l"
                run_timed "$SBK" decompress "$out" -o "$DEC_DIR/lzma$l/" -t "$THREADS" >/dev/null 2>&1 || true
                lzma_ms_d[$l]=$_ms; rm -f "$out"; lzma_ok[$l]=true
                echo "$(fmt_size ${lzma_sz[$l]})  $(fmt_ratio $raw_bytes ${lzma_sz[$l]})  c:$(fmt_time ${lzma_ms_c[$l]})  d:$(fmt_time ${lzma_ms_d[$l]})"
            else echo "FAILED"; fi
        done

        # ── sbk zstd levels 1–9 ───────────────────────────────────────────────
        for l in $(seq 1 9); do
            printf "   sbk zstd  -l %d … " "$l"
            if run_timed "$SBK" compress "$world_dir" -o "$out" --algorithm zstd -l "$l" -t "$THREADS" >/dev/null 2>&1; then
                zstd_ms_c[$l]=$_ms; zstd_sz[$l]=$(du -sb "$out" | awk '{print $1}')
                rm -rf "$DEC_DIR/zstd$l"; mkdir -p "$DEC_DIR/zstd$l"
                run_timed "$SBK" decompress "$out" -o "$DEC_DIR/zstd$l/" -t "$THREADS" >/dev/null 2>&1 || true
                zstd_ms_d[$l]=$_ms; rm -f "$out"; zstd_ok[$l]=true
                echo "$(fmt_size ${zstd_sz[$l]})  $(fmt_ratio $raw_bytes ${zstd_sz[$l]})  c:$(fmt_time ${zstd_ms_c[$l]})  d:$(fmt_time ${zstd_ms_d[$l]})"
            else echo "FAILED"; fi
        done

        # ── zip (default) ─────────────────────────────────────────────────────
        sz_zip=0; ms_zip_c=0; ms_zip_d=0; zip_ok=false
        out_zip="$OUT_DIR/out.zip"
        printf "   zip (default) … "
        if run_timed _do_zip "$world_parent" "$out_zip" "$world" >/dev/null 2>&1; then
            ms_zip_c=$_ms; sz_zip=$(du -sb "$out_zip" | awk '{print $1}')
            rm -rf "$DEC_DIR/zip"; mkdir -p "$DEC_DIR/zip"
            run_timed unzip -q "$out_zip" -d "$DEC_DIR/zip/" >/dev/null 2>&1 || true
            ms_zip_d=$_ms; rm -f "$out_zip"; zip_ok=true
            echo "$(fmt_size $sz_zip)  $(fmt_ratio $raw_bytes $sz_zip)  c:$(fmt_time $ms_zip_c)  d:$(fmt_time $ms_zip_d)"
        else echo "FAILED"; fi

        # ── tar.gz (default, optional) ────────────────────────────────────────
        sz_tgz=0; ms_tgz_c=0; ms_tgz_d=0; tgz_ok=false
        if $HAS_TARGZ; then
            out_tgz="$OUT_DIR/out.tar.gz"
            printf "   tar.gz (default) … "
            if run_timed tar -czf "$out_tgz" -C "$world_parent" "$world" >/dev/null 2>&1; then
                ms_tgz_c=$_ms; sz_tgz=$(du -sb "$out_tgz" | awk '{print $1}')
                rm -rf "$DEC_DIR/tgz"; mkdir -p "$DEC_DIR/tgz"
                run_timed tar -xzf "$out_tgz" -C "$DEC_DIR/tgz/" >/dev/null 2>&1 || true
                ms_tgz_d=$_ms; rm -f "$out_tgz"; tgz_ok=true
                echo "$(fmt_size $sz_tgz)  $(fmt_ratio $raw_bytes $sz_tgz)  c:$(fmt_time $ms_tgz_c)  d:$(fmt_time $ms_tgz_d)"
            else echo "FAILED"; fi
        fi

        # ── 7z (default, optional) ────────────────────────────────────────────
        sz_7z=0; ms_7z_c=0; ms_7z_d=0; z7_ok=false
        if $HAS_7Z; then
            out_7z="$OUT_DIR/out.7z"
            printf "   7z (default) … "
            if run_timed 7z a "$out_7z" "$world_dir" >/dev/null 2>&1; then
                ms_7z_c=$_ms; sz_7z=$(du -sb "$out_7z" | awk '{print $1}')
                rm -rf "$DEC_DIR/7z"; mkdir -p "$DEC_DIR/7z"
                run_timed 7z x "$out_7z" -o"$DEC_DIR/7z/" >/dev/null 2>&1 || true
                ms_7z_d=$_ms; rm -f "$out_7z"; z7_ok=true
                echo "$(fmt_size $sz_7z)  $(fmt_ratio $raw_bytes $sz_7z)  c:$(fmt_time $ms_7z_c)  d:$(fmt_time $ms_7z_d)"
            else echo "FAILED"; fi
        fi

        # Helper: emit a table row
        sbk_row() {
            local label=$1 method=$2 ok=$3 sz=$4 ms_c=$5 ms_d=$6
            if $ok; then
                echo "| **${method}** | $(fmt_size $sz) | $(fmt_ratio $raw_bytes $sz) | $(fmt_time $ms_c) | $(fmt_time $ms_d) |"
            else
                echo "| **${method}** | — | — | FAILED | — |"
            fi
        }

        # ── Append table to BENCHMARK.md ──────────────────────────────────────
        {
            echo "### $world  _(Minecraft $version)_"
            echo ""
            echo "| Method | Size | Ratio | Compress | Decompress |"
            echo "|--------|-----:|------:|---------:|-----------:|"
            echo "| Uncompressed | $(fmt_size $raw_bytes) | 100% | — | — |"
            for l in $(seq 1 9); do
                sbk_row "lzma2-$l" "sbk lzma2 -l $l" "${lzma_ok[$l]}" "${lzma_sz[$l]}" "${lzma_ms_c[$l]}" "${lzma_ms_d[$l]}"
            done
            for l in $(seq 1 9); do
                sbk_row "zstd-$l" "sbk zstd -l $l" "${zstd_ok[$l]}" "${zstd_sz[$l]}" "${zstd_ms_c[$l]}" "${zstd_ms_d[$l]}"
            done
            $zip_ok  && echo "| zip      | $(fmt_size $sz_zip) | $(fmt_ratio $raw_bytes $sz_zip) | $(fmt_time $ms_zip_c) | $(fmt_time $ms_zip_d) |" \
                     || echo "| zip      | — | — | FAILED | — |"
            if $HAS_TARGZ; then
                $tgz_ok && echo "| tar.gz   | $(fmt_size $sz_tgz) | $(fmt_ratio $raw_bytes $sz_tgz) | $(fmt_time $ms_tgz_c) | $(fmt_time $ms_tgz_d) |" \
                        || echo "| tar.gz   | — | — | FAILED | — |"
            fi
            if $HAS_7Z; then
                $z7_ok  && echo "| 7z       | $(fmt_size $sz_7z)  | $(fmt_ratio $raw_bytes $sz_7z)  | $(fmt_time $ms_7z_c)  | $(fmt_time $ms_7z_d)  |" \
                        || echo "| 7z       | — | — | FAILED | — |"
            fi
            echo ""
        } >> "$MD"

        # Clean up arrays for next world
        unset lzma_ok lzma_sz lzma_ms_c lzma_ms_d
        unset zstd_ok zstd_sz zstd_ms_c zstd_ms_d
    done < <(sorted_worlds "$version_dir")
done

# ── How-to section ─────────────────────────────────────────────────────────────
cat >> "$MD" << 'HOWTO'
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
HOWTO

echo ""
echo "✓  Done — results written to BENCHMARK.md."
