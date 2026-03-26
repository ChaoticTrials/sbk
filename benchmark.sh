#!/usr/bin/env bash
# Benchmark sbk against zip and tar.xz (and 7z if available).
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
THREADS=16

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
HAS_XZ=false; command -v tar &>/dev/null && command -v xz &>/dev/null && HAS_XZ=true
HAS_7Z=false; command -v 7z  &>/dev/null && HAS_7Z=true

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
# Uses an if-condition so set -e does not trigger on failure.
run_timed() {
    local _t; _t=$(date +%s%3N)
    if "$@"; then _ms=$(( $(date +%s%3N) - _t )); return 0
    else           _ms=0;                          return 1
    fi
}

# zip needs cd first; this helper runs in a subshell so the cd stays local.
_do_zip() { (cd "$1" && zip "-$2" -rq "$3" "$4"); }

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
    # Sort integers numerically, strings lexicographically.
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
    echo "# SBK ($SBK_VER) — lzma2 (default)"
    echo "sbk compress <world> -o <world>.sbk -l 1 -t $THREADS"
    echo "sbk compress <world> -o <world>.sbk -l 5 -t $THREADS"
    echo "sbk compress <world> -o <world>.sbk -l 9 -t $THREADS"
    echo "sbk decompress <world>.sbk -o <out_dir>/ -t $THREADS"
    echo ""
    echo "# SBK ($SBK_VER) — zstd"
    echo "sbk compress <world> -o <world>.sbk --algorithm zstd -l 1 -t $THREADS"
    echo "sbk compress <world> -o <world>.sbk --algorithm zstd -l 5 -t $THREADS"
    echo "sbk compress <world> -o <world>.sbk --algorithm zstd -l 9 -t $THREADS"
    echo "sbk decompress <world>.sbk -o <out_dir>/ -t $THREADS"
    echo ""
    echo "# zip"
    echo "zip -1 -r <world>.zip <world>/"
    echo "zip -5 -r <world>.zip <world>/"
    echo "zip -9 -r <world>.zip <world>/"
    echo "unzip -q <world>.zip -d <out_dir>/"
    if $HAS_XZ; then
        echo ""
        echo "# tar.xz"
        echo "tar -cJf <world>.tar.xz <world>/"
        echo "tar -xJf <world>.tar.xz -C <out_dir>/"
    fi
    if $HAS_7Z; then
        echo ""
        echo "# 7z"
        echo "7z a -mx=1 <world>.7z <world>/"
        echo "7z a -mx=5 <world>.7z <world>/"
        echo "7z a -mx=9 <world>.7z <world>/"
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
        sbkz1_ok=false; sz_sbkz1=0; ms_sbkz1_c=0; ms_sbkz1_d=0
        sbkz5_ok=false; sz_sbkz5=0; ms_sbkz5_c=0; ms_sbkz5_d=0
        sbkz9_ok=false; sz_sbkz9=0; ms_sbkz9_c=0; ms_sbkz9_d=0

        # ── sbk -l 1 ──────────────────────────────────────────────────────────
        printf "   sbk -l 1  … "
        sbk1_ok=false; sz_sbk1=0; ms_sbk1_c=0; ms_sbk1_d=0
        if run_timed "$SBK" compress "$world_dir" -o "$out" -l 1 -t "$THREADS" >/dev/null 2>&1; then
            ms_sbk1_c=$_ms; sz_sbk1=$(du -sb "$out" | awk '{print $1}')
            rm -rf "$DEC_DIR/sbk1"; mkdir -p "$DEC_DIR/sbk1"
            run_timed "$SBK" decompress "$out" -o "$DEC_DIR/sbk1/" -t "$THREADS" >/dev/null 2>&1 || true
            ms_sbk1_d=$_ms; rm -f "$out"; sbk1_ok=true
            echo "$(fmt_size $sz_sbk1)  $(fmt_ratio $raw_bytes $sz_sbk1)  c:$(fmt_time $ms_sbk1_c)  d:$(fmt_time $ms_sbk1_d)"
        else echo "FAILED"; fi

        # ── sbk -l 5 ──────────────────────────────────────────────────────────
        printf "   sbk -l 5  … "
        sbk5_ok=false; sz_sbk5=0; ms_sbk5_c=0; ms_sbk5_d=0
        if run_timed "$SBK" compress "$world_dir" -o "$out" -l 5 -t "$THREADS" >/dev/null 2>&1; then
            ms_sbk5_c=$_ms; sz_sbk5=$(du -sb "$out" | awk '{print $1}')
            rm -rf "$DEC_DIR/sbk5"; mkdir -p "$DEC_DIR/sbk5"
            run_timed "$SBK" decompress "$out" -o "$DEC_DIR/sbk5/" -t "$THREADS" >/dev/null 2>&1 || true
            ms_sbk5_d=$_ms; rm -f "$out"; sbk5_ok=true
            echo "$(fmt_size $sz_sbk5)  $(fmt_ratio $raw_bytes $sz_sbk5)  c:$(fmt_time $ms_sbk5_c)  d:$(fmt_time $ms_sbk5_d)"
        else echo "FAILED"; fi

        # ── sbk -l 9 ──────────────────────────────────────────────────────────
        printf "   sbk -l 9  … "
        sbk9_ok=false; sz_sbk9=0; ms_sbk9_c=0; ms_sbk9_d=0
        if run_timed "$SBK" compress "$world_dir" -o "$out" -l 9 -t "$THREADS" >/dev/null 2>&1; then
            ms_sbk9_c=$_ms; sz_sbk9=$(du -sb "$out" | awk '{print $1}')
            rm -rf "$DEC_DIR/sbk9"; mkdir -p "$DEC_DIR/sbk9"
            run_timed "$SBK" decompress "$out" -o "$DEC_DIR/sbk9/" -t "$THREADS" >/dev/null 2>&1 || true
            ms_sbk9_d=$_ms; rm -f "$out"; sbk9_ok=true
            echo "$(fmt_size $sz_sbk9)  $(fmt_ratio $raw_bytes $sz_sbk9)  c:$(fmt_time $ms_sbk9_c)  d:$(fmt_time $ms_sbk9_d)"
        else echo "FAILED"; fi

        # ── sbk zstd -l 1 ─────────────────────────────────────────────────────
        printf "   sbk zstd -l 1 … "
        if run_timed "$SBK" compress "$world_dir" -o "$out" --algorithm zstd -l 1 -t "$THREADS" >/dev/null 2>&1; then
            ms_sbkz1_c=$_ms; sz_sbkz1=$(du -sb "$out" | awk '{print $1}')
            rm -rf "$DEC_DIR/sbkz1"; mkdir -p "$DEC_DIR/sbkz1"
            run_timed "$SBK" decompress "$out" -o "$DEC_DIR/sbkz1/" -t "$THREADS" >/dev/null 2>&1 || true
            ms_sbkz1_d=$_ms; rm -f "$out"; sbkz1_ok=true
            echo "$(fmt_size $sz_sbkz1)  $(fmt_ratio $raw_bytes $sz_sbkz1)  c:$(fmt_time $ms_sbkz1_c)  d:$(fmt_time $ms_sbkz1_d)"
        else echo "FAILED"; fi

        # ── sbk zstd -l 5 ─────────────────────────────────────────────────────
        printf "   sbk zstd -l 5 … "
        if run_timed "$SBK" compress "$world_dir" -o "$out" --algorithm zstd -l 5 -t "$THREADS" >/dev/null 2>&1; then
            ms_sbkz5_c=$_ms; sz_sbkz5=$(du -sb "$out" | awk '{print $1}')
            rm -rf "$DEC_DIR/sbkz5"; mkdir -p "$DEC_DIR/sbkz5"
            run_timed "$SBK" decompress "$out" -o "$DEC_DIR/sbkz5/" -t "$THREADS" >/dev/null 2>&1 || true
            ms_sbkz5_d=$_ms; rm -f "$out"; sbkz5_ok=true
            echo "$(fmt_size $sz_sbkz5)  $(fmt_ratio $raw_bytes $sz_sbkz5)  c:$(fmt_time $ms_sbkz5_c)  d:$(fmt_time $ms_sbkz5_d)"
        else echo "FAILED"; fi

        # ── sbk zstd -l 9 ─────────────────────────────────────────────────────
        printf "   sbk zstd -l 9 … "
        if run_timed "$SBK" compress "$world_dir" -o "$out" --algorithm zstd -l 9 -t "$THREADS" >/dev/null 2>&1; then
            ms_sbkz9_c=$_ms; sz_sbkz9=$(du -sb "$out" | awk '{print $1}')
            rm -rf "$DEC_DIR/sbkz9"; mkdir -p "$DEC_DIR/sbkz9"
            run_timed "$SBK" decompress "$out" -o "$DEC_DIR/sbkz9/" -t "$THREADS" >/dev/null 2>&1 || true
            ms_sbkz9_d=$_ms; rm -f "$out"; sbkz9_ok=true
            echo "$(fmt_size $sz_sbkz9)  $(fmt_ratio $raw_bytes $sz_sbkz9)  c:$(fmt_time $ms_sbkz9_c)  d:$(fmt_time $ms_sbkz9_d)"
        else echo "FAILED"; fi

        # ── zip ───────────────────────────────────────────────────────────────
        sz_zip1=0; ms_zip1_c=0; ms_zip1_d=0; zip1_ok=false
        sz_zip5=0; ms_zip5_c=0; ms_zip5_d=0; zip5_ok=false
        sz_zip9=0; ms_zip9_c=0; ms_zip9_d=0; zip9_ok=false
        out_zip="$OUT_DIR/out.zip"

        printf "   zip -1    … "
        if run_timed _do_zip "$world_parent" 1 "$out_zip" "$world" >/dev/null 2>&1; then
            ms_zip1_c=$_ms; sz_zip1=$(du -sb "$out_zip" | awk '{print $1}')
            rm -rf "$DEC_DIR/zip1"; mkdir -p "$DEC_DIR/zip1"
            run_timed unzip -q "$out_zip" -d "$DEC_DIR/zip1/" >/dev/null 2>&1 || true
            ms_zip1_d=$_ms; rm -f "$out_zip"; zip1_ok=true
            echo "$(fmt_size $sz_zip1)  $(fmt_ratio $raw_bytes $sz_zip1)  c:$(fmt_time $ms_zip1_c)  d:$(fmt_time $ms_zip1_d)"
        else echo "FAILED"; fi

        printf "   zip -5    … "
        if run_timed _do_zip "$world_parent" 5 "$out_zip" "$world" >/dev/null 2>&1; then
            ms_zip5_c=$_ms; sz_zip5=$(du -sb "$out_zip" | awk '{print $1}')
            rm -rf "$DEC_DIR/zip5"; mkdir -p "$DEC_DIR/zip5"
            run_timed unzip -q "$out_zip" -d "$DEC_DIR/zip5/" >/dev/null 2>&1 || true
            ms_zip5_d=$_ms; rm -f "$out_zip"; zip5_ok=true
            echo "$(fmt_size $sz_zip5)  $(fmt_ratio $raw_bytes $sz_zip5)  c:$(fmt_time $ms_zip5_c)  d:$(fmt_time $ms_zip5_d)"
        else echo "FAILED"; fi

        printf "   zip -9    … "
        if run_timed _do_zip "$world_parent" 9 "$out_zip" "$world" >/dev/null 2>&1; then
            ms_zip9_c=$_ms; sz_zip9=$(du -sb "$out_zip" | awk '{print $1}')
            rm -rf "$DEC_DIR/zip9"; mkdir -p "$DEC_DIR/zip9"
            run_timed unzip -q "$out_zip" -d "$DEC_DIR/zip9/" >/dev/null 2>&1 || true
            ms_zip9_d=$_ms; rm -f "$out_zip"; zip9_ok=true
            echo "$(fmt_size $sz_zip9)  $(fmt_ratio $raw_bytes $sz_zip9)  c:$(fmt_time $ms_zip9_c)  d:$(fmt_time $ms_zip9_d)"
        else echo "FAILED"; fi

        # ── tar.xz (optional) ─────────────────────────────────────────────────
        sz_xz=0; ms_xz_c=0; ms_xz_d=0; xz_ok=false
        if $HAS_XZ; then
            out_xz="$OUT_DIR/out.tar.xz"
            printf "   tar.xz    … "
            if run_timed tar -cJf "$out_xz" -C "$world_parent" "$world" >/dev/null 2>&1; then
                ms_xz_c=$_ms; sz_xz=$(du -sb "$out_xz" | awk '{print $1}')
                rm -rf "$DEC_DIR/xz"; mkdir -p "$DEC_DIR/xz"
                run_timed tar -xJf "$out_xz" -C "$DEC_DIR/xz/" >/dev/null 2>&1 || true
                ms_xz_d=$_ms; rm -f "$out_xz"; xz_ok=true
                echo "$(fmt_size $sz_xz)  $(fmt_ratio $raw_bytes $sz_xz)  c:$(fmt_time $ms_xz_c)  d:$(fmt_time $ms_xz_d)"
            else echo "FAILED"; fi
        fi

        # ── 7z (optional) ─────────────────────────────────────────────────────
        ms_7z1_c=0; sz_7z1=0; ms_7z1_d=0; z7_1_ok=false
        ms_7z5_c=0; sz_7z5=0; ms_7z5_d=0; z7_5_ok=false
        ms_7z9_c=0; sz_7z9=0; ms_7z9_d=0; z7_9_ok=false
        if $HAS_7Z; then
            out_7z="$OUT_DIR/out.7z"

            printf "   7z -mx=1  … "
            if run_timed 7z a -mx=1 "$out_7z" "$world_dir" >/dev/null 2>&1; then
                ms_7z1_c=$_ms; sz_7z1=$(du -sb "$out_7z" | awk '{print $1}')
                rm -rf "$DEC_DIR/7z1"; mkdir -p "$DEC_DIR/7z1"
                run_timed 7z x "$out_7z" -o"$DEC_DIR/7z1/" >/dev/null 2>&1 || true
                ms_7z1_d=$_ms; rm -f "$out_7z"; z7_1_ok=true
                echo "$(fmt_size $sz_7z1)  $(fmt_ratio $raw_bytes $sz_7z1)  c:$(fmt_time $ms_7z1_c)  d:$(fmt_time $ms_7z1_d)"
            else echo "FAILED"; fi

            printf "   7z -mx=5  … "
            if run_timed 7z a -mx=5 "$out_7z" "$world_dir" >/dev/null 2>&1; then
                ms_7z5_c=$_ms; sz_7z5=$(du -sb "$out_7z" | awk '{print $1}')
                rm -rf "$DEC_DIR/7z5"; mkdir -p "$DEC_DIR/7z5"
                run_timed 7z x "$out_7z" -o"$DEC_DIR/7z5/" >/dev/null 2>&1 || true
                ms_7z5_d=$_ms; rm -f "$out_7z"; z7_5_ok=true
                echo "$(fmt_size $sz_7z5)  $(fmt_ratio $raw_bytes $sz_7z5)  c:$(fmt_time $ms_7z5_c)  d:$(fmt_time $ms_7z5_d)"
            else echo "FAILED"; fi

            printf "   7z -mx=9  … "
            if run_timed 7z a -mx=9 "$out_7z" "$world_dir" >/dev/null 2>&1; then
                ms_7z9_c=$_ms; sz_7z9=$(du -sb "$out_7z" | awk '{print $1}')
                rm -rf "$DEC_DIR/7z9"; mkdir -p "$DEC_DIR/7z9"
                run_timed 7z x "$out_7z" -o"$DEC_DIR/7z9/" >/dev/null 2>&1 || true
                ms_7z9_d=$_ms; rm -f "$out_7z"; z7_9_ok=true
                echo "$(fmt_size $sz_7z9)  $(fmt_ratio $raw_bytes $sz_7z9)  c:$(fmt_time $ms_7z9_c)  d:$(fmt_time $ms_7z9_d)"
            else echo "FAILED"; fi
        fi

        # ── Append table to BENCHMARK.md ──────────────────────────────────────
        {
            echo "### $world  _(Minecraft $version)_"
            echo ""
            echo "| Method | Size | Ratio | Compress | Decompress |"
            echo "|--------|-----:|------:|---------:|-----------:|"
            echo "| Uncompressed | $(fmt_size $raw_bytes) | 100% | — | — |"
            $sbk1_ok && echo "| **sbk lzma2 -l 1** | $(fmt_size $sz_sbk1) | $(fmt_ratio $raw_bytes $sz_sbk1) | $(fmt_time $ms_sbk1_c) | $(fmt_time $ms_sbk1_d) |" || echo "| **sbk lzma2 -l 1** | — | — | FAILED | — |"
            $sbk5_ok && echo "| **sbk lzma2 -l 5** | $(fmt_size $sz_sbk5) | $(fmt_ratio $raw_bytes $sz_sbk5) | $(fmt_time $ms_sbk5_c) | $(fmt_time $ms_sbk5_d) |" || echo "| **sbk lzma2 -l 5** | — | — | FAILED | — |"
            $sbk9_ok  && echo "| **sbk lzma2 -l 9** | $(fmt_size $sz_sbk9)  | $(fmt_ratio $raw_bytes $sz_sbk9)  | $(fmt_time $ms_sbk9_c)  | $(fmt_time $ms_sbk9_d)  |" || echo "| **sbk lzma2 -l 9** | — | — | FAILED | — |"
            $sbkz1_ok && echo "| **sbk zstd -l 1**  | $(fmt_size $sz_sbkz1) | $(fmt_ratio $raw_bytes $sz_sbkz1) | $(fmt_time $ms_sbkz1_c) | $(fmt_time $ms_sbkz1_d) |" || echo "| **sbk zstd -l 1**  | — | — | FAILED | — |"
            $sbkz5_ok && echo "| **sbk zstd -l 5**  | $(fmt_size $sz_sbkz5) | $(fmt_ratio $raw_bytes $sz_sbkz5) | $(fmt_time $ms_sbkz5_c) | $(fmt_time $ms_sbkz5_d) |" || echo "| **sbk zstd -l 5**  | — | — | FAILED | — |"
            $sbkz9_ok && echo "| **sbk zstd -l 9**  | $(fmt_size $sz_sbkz9) | $(fmt_ratio $raw_bytes $sz_sbkz9) | $(fmt_time $ms_sbkz9_c) | $(fmt_time $ms_sbkz9_d) |" || echo "| **sbk zstd -l 9**  | — | — | FAILED | — |"
            $zip1_ok && echo "| zip -1 | $(fmt_size $sz_zip1) | $(fmt_ratio $raw_bytes $sz_zip1) | $(fmt_time $ms_zip1_c) | $(fmt_time $ms_zip1_d) |" || echo "| zip -1 | — | — | FAILED | — |"
            $zip5_ok && echo "| zip -5 | $(fmt_size $sz_zip5) | $(fmt_ratio $raw_bytes $sz_zip5) | $(fmt_time $ms_zip5_c) | $(fmt_time $ms_zip5_d) |" || echo "| zip -5 | — | — | FAILED | — |"
            $zip9_ok && echo "| zip -9 | $(fmt_size $sz_zip9) | $(fmt_ratio $raw_bytes $sz_zip9) | $(fmt_time $ms_zip9_c) | $(fmt_time $ms_zip9_d) |" || echo "| zip -9 | — | — | FAILED | — |"
            if $HAS_XZ; then
                $xz_ok && echo "| tar.xz | $(fmt_size $sz_xz) | $(fmt_ratio $raw_bytes $sz_xz) | $(fmt_time $ms_xz_c) | $(fmt_time $ms_xz_d) |" || echo "| tar.xz | — | — | FAILED | — |"
            fi
            if $HAS_7Z; then
                $z7_1_ok && echo "| 7z -mx=1 | $(fmt_size $sz_7z1) | $(fmt_ratio $raw_bytes $sz_7z1) | $(fmt_time $ms_7z1_c) | $(fmt_time $ms_7z1_d) |" || echo "| 7z -mx=1 | — | — | FAILED | — |"
                $z7_5_ok && echo "| 7z -mx=5 | $(fmt_size $sz_7z5) | $(fmt_ratio $raw_bytes $sz_7z5) | $(fmt_time $ms_7z5_c) | $(fmt_time $ms_7z5_d) |" || echo "| 7z -mx=5 | — | — | FAILED | — |"
                $z7_9_ok && echo "| 7z -mx=9 | $(fmt_size $sz_7z9) | $(fmt_ratio $raw_bytes $sz_7z9) | $(fmt_time $ms_7z9_c) | $(fmt_time $ms_7z9_d) |" || echo "| 7z -mx=9 | — | — | FAILED | — |"
            fi
            echo ""
        } >> "$MD"
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
Requires: `zip`, `unzip`. Optional: `tar`/`xz` (`sudo apt install xz-utils`), `7z` (`sudo apt install p7zip-full`).
HOWTO

echo ""
echo "✓  Done — results written to BENCHMARK.md."
