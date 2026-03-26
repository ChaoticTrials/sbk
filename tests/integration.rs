use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use filetime::FileTime;
use flate2::Compression;
use flate2::write::{GzEncoder, ZlibEncoder};
use sbk::error::SbkError;
use sbk::filter::{CompressOptions, FilterMode, capture_now_ms};
use sbk::format::header::Algorithm;
use tempfile::TempDir;

// ─── helper: build a minimal valid MCA file ─────────────────────────────────

fn make_mca_bytes(chunks: &[(u8, u8, &[u8])]) -> Vec<u8> {
    let compressed: Vec<(u8, u8, Vec<u8>)> = chunks
        .iter()
        .map(|(x, z, nbt)| {
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::new(6));
            enc.write_all(nbt).unwrap();
            (*x, *z, enc.finish().unwrap())
        })
        .collect();

    let mut sector_data: Vec<(usize, Vec<u8>)> = Vec::new();
    let mut location_entries = vec![(0u32, 0u8); 1024];
    let mut current_sector: usize = 2;

    for (x, z, cdata) in &compressed {
        let slot = (*z as usize) * 32 + (*x as usize);
        let total_len = 5 + cdata.len();
        let required_sectors = (total_len + 4095) / 4096;
        location_entries[slot] = (current_sector as u32, required_sectors as u8);
        sector_data.push((current_sector, cdata.clone()));
        current_sector += required_sectors;
    }

    let total_size = current_sector * 4096;
    let mut file = vec![0u8; total_size];

    for (slot, (sector_offset, sector_count)) in location_entries.iter().enumerate() {
        if *sector_count > 0 {
            let base = slot * 4;
            let entry = ((*sector_offset) << 8) | (*sector_count as u32);
            file[base..base + 4].copy_from_slice(&entry.to_be_bytes());
        }
    }

    for (sector, cdata) in &sector_data {
        let pos = sector * 4096;
        let length = (cdata.len() + 1) as u32;
        file[pos..pos + 4].copy_from_slice(&length.to_be_bytes());
        file[pos + 4] = 2; // zlib
        file[pos + 5..pos + 5 + cdata.len()].copy_from_slice(cdata);
    }

    file
}

fn make_gzip_nbt(data: &[u8]) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::new(6));
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

/// Create a synthetic Minecraft world directory.
fn make_test_world(dir: &Path) {
    // region/r.0.0.mca — 5 chunks with minimal synthetic NBT
    fs::create_dir_all(dir.join("region")).unwrap();
    let mca_chunks: Vec<(u8, u8, Vec<u8>)> = (0u8..5)
        .map(|i| (i % 5, i / 5, format!("nbt_data_{}", i).into_bytes()))
        .collect();
    let mca_refs: Vec<(u8, u8, &[u8])> = mca_chunks
        .iter()
        .map(|(x, z, d)| (*x, *z, d.as_slice()))
        .collect();
    let mca_bytes = make_mca_bytes(&mca_refs);
    fs::write(dir.join("region/r.0.0.mca"), &mca_bytes).unwrap();

    // DIM-1/region/r.0.0.mca — same structure
    fs::create_dir_all(dir.join("DIM-1/region")).unwrap();
    fs::write(dir.join("DIM-1/region/r.0.0.mca"), &mca_bytes).unwrap();

    // level.dat — gzip-wrapped NBT
    let nbt_raw = b"\x0a\x00\x00\x0a\x00\x04Data\x00"; // minimal compound NBT
    let level_dat = make_gzip_nbt(nbt_raw);
    fs::write(dir.join("level.dat"), &level_dat).unwrap();

    // advancements/player.json — JSON with extra whitespace
    fs::create_dir_all(dir.join("advancements")).unwrap();
    fs::write(
        dir.join("advancements/player.json"),
        b"{  \"DataVersion\":  3700  ,  \"minecraft:story/root\":  true  }",
    )
    .unwrap();

    // icon.png — fixed random bytes (RAW group)
    fs::write(dir.join("icon.png"), b"\x89PNG\r\n\x1a\nfakeicon").unwrap();

    // session.lock — always excluded
    fs::write(dir.join("session.lock"), b"lock").unwrap();
}

fn default_opts(_world_dir: &Path, output: &Path) -> CompressOptions {
    CompressOptions {
        output: output.to_path_buf(),
        threads: 2,
        level: 1, // fast compression for tests
        algorithm: Algorithm::Lzma2,
        max_age: None,
        since: None,
        patterns: FilterMode::None,
        include_session_lock: false,
        quiet: true,
    }
}

fn set_mtime(path: &Path, ms: i64) {
    let ft = FileTime::from_unix_time(ms / 1000, ((ms % 1000) * 1_000_000) as u32);
    filetime::set_file_mtime(path, ft).unwrap();
}

fn get_mtime_ms(path: &Path) -> i64 {
    let ft = FileTime::from_last_modification_time(&fs::metadata(path).unwrap());
    ft.unix_seconds() * 1000 + ft.nanoseconds() as i64 / 1_000_000
}

// ─── Test 1: Full round-trip ─────────────────────────────────────────────────

#[test]
fn test01_full_round_trip() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");
    let out_dir = tmp.path().join("restored");

    make_test_world(&world_dir);

    let opts = default_opts(&world_dir, &archive);
    sbk::compress::compress(&world_dir, &opts).unwrap();
    assert!(archive.exists());

    sbk::decompress::decompress(&archive, &out_dir, 2).unwrap();

    // session.lock must not be present
    assert!(!out_dir.join("session.lock").exists());

    // Check each expected file
    assert!(out_dir.join("region/r.0.0.mca").exists());
    assert!(out_dir.join("DIM-1/region/r.0.0.mca").exists());
    assert!(out_dir.join("level.dat").exists());
    assert!(out_dir.join("advancements/player.json").exists());
    assert!(out_dir.join("icon.png").exists());

    // icon.png RAW — byte-identical
    let orig_icon = fs::read(world_dir.join("icon.png")).unwrap();
    let rest_icon = fs::read(out_dir.join("icon.png")).unwrap();
    assert_eq!(orig_icon, rest_icon);

    // JSON should be compact (no extra spaces)
    let json_content = fs::read_to_string(out_dir.join("advancements/player.json")).unwrap();
    assert!(!json_content.contains("  "));
    assert!(json_content.contains("DataVersion"));
}

// ─── Test 2: mtime preservation ──────────────────────────────────────────────

#[test]
fn test02_mtime_preservation() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");
    let out_dir = tmp.path().join("restored");

    make_test_world(&world_dir);

    // Stamp known mtime
    let known_ms = 1_700_000_000_000i64;
    set_mtime(&world_dir.join("icon.png"), known_ms);
    set_mtime(&world_dir.join("advancements/player.json"), known_ms + 1000);

    let opts = default_opts(&world_dir, &archive);
    sbk::compress::compress(&world_dir, &opts).unwrap();
    sbk::decompress::decompress(&archive, &out_dir, 2).unwrap();

    let got_icon = get_mtime_ms(&out_dir.join("icon.png"));
    assert!(
        (got_icon - known_ms).abs() <= 1,
        "icon mtime off by {} ms",
        (got_icon - known_ms).abs()
    );

    let got_json = get_mtime_ms(&out_dir.join("advancements/player.json"));
    assert!(
        (got_json - (known_ms + 1000)).abs() <= 1,
        "json mtime off by {} ms",
        (got_json - (known_ms + 1000)).abs()
    );
}

// ─── Test 3: --max-age excludes old file ─────────────────────────────────────

#[test]
fn test03_max_age_excludes_old() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");
    let out_dir = tmp.path().join("restored");

    make_test_world(&world_dir);

    let now_ms = capture_now_ms();
    let ten_days_ms: i64 = 864_000_000;
    let five_days_ms: u64 = 432_000_000;

    // Backdate region/r.0.0.mca by 10 days
    set_mtime(&world_dir.join("region/r.0.0.mca"), now_ms - ten_days_ms);

    let opts = CompressOptions {
        max_age: Some(five_days_ms),
        ..default_opts(&world_dir, &archive)
    };
    sbk::compress::compress(&world_dir, &opts).unwrap();
    sbk::decompress::decompress(&archive, &out_dir, 2).unwrap();

    // Old MCA must be absent
    assert!(!out_dir.join("region/r.0.0.mca").exists());
    // Other files present
    assert!(out_dir.join("icon.png").exists());
}

// ─── Test 4: --max-age boundary precision ────────────────────────────────────

#[test]
fn test04_max_age_boundary() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");
    let out_dir = tmp.path().join("restored");

    make_test_world(&world_dir);

    let now_ms = capture_now_ms();
    let max_age_ms: u64 = 1_000_000;

    // file A: exactly at cutoff (should be included)
    set_mtime(&world_dir.join("icon.png"), now_ms - max_age_ms as i64);
    // file B: just before cutoff (should be excluded)
    set_mtime(
        &world_dir.join("advancements/player.json"),
        now_ms - max_age_ms as i64 - 1,
    );

    let opts = CompressOptions {
        max_age: Some(max_age_ms),
        ..default_opts(&world_dir, &archive)
    };
    sbk::compress::compress(&world_dir, &opts).unwrap();
    sbk::decompress::decompress(&archive, &out_dir, 2).unwrap();

    assert!(
        out_dir.join("icon.png").exists(),
        "icon.png at cutoff should be included"
    );
    assert!(
        !out_dir.join("advancements/player.json").exists(),
        "player.json just before cutoff should be excluded"
    );
}

// ─── Test 5: --since basic ───────────────────────────────────────────────────

#[test]
fn test05_since_basic() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");
    let out_dir = tmp.path().join("restored");

    make_test_world(&world_dir);

    let since: i64 = 1_700_000_000_000;
    set_mtime(&world_dir.join("icon.png"), since - 1); // excluded
    set_mtime(&world_dir.join("advancements/player.json"), since); // included

    let opts = CompressOptions {
        since: Some(since),
        ..default_opts(&world_dir, &archive)
    };
    sbk::compress::compress(&world_dir, &opts).unwrap();
    sbk::decompress::decompress(&archive, &out_dir, 2).unwrap();

    assert!(
        !out_dir.join("icon.png").exists(),
        "icon.png before --since should be excluded"
    );
    assert!(
        out_dir.join("advancements/player.json").exists(),
        "player.json at --since should be included"
    );
}

// ─── Test 6: --since epoch zero and far future ───────────────────────────────

#[test]
fn test06_since_epoch_zero() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive_all = tmp.path().join("all.sbk");
    let archive_none = tmp.path().join("none.sbk");
    let out_dir = tmp.path().join("restored");

    make_test_world(&world_dir);

    // --since 0: all files included (all have mtime > 0)
    let opts_all = CompressOptions {
        since: Some(0),
        ..default_opts(&world_dir, &archive_all)
    };
    sbk::compress::compress(&world_dir, &opts_all).unwrap();
    sbk::decompress::decompress(&archive_all, &out_dir, 2).unwrap();
    assert!(out_dir.join("icon.png").exists());

    // --since year 2100: archive should be (nearly) empty — no error
    let year2100_ms: i64 = 4_102_444_800_000;
    let opts_none = CompressOptions {
        since: Some(year2100_ms),
        ..default_opts(&world_dir, &archive_none)
    };
    sbk::compress::compress(&world_dir, &opts_none).unwrap();
    // Just check the archive exists and doesn't error; decompress is trivial
    assert!(archive_none.exists());
}

// ─── Test 7: --max-age and --since combined ───────────────────────────────────

#[test]
fn test07_max_age_and_since_combined() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");

    make_test_world(&world_dir);

    // Use controlled times to avoid clock dependency
    // We'll set all files to known times and use since/max_age relative to a fake "now"
    // But the real filter uses capture_now_ms() internally in compress.
    // Instead, test the filter function directly.
    use sbk::filter::accept;

    let now = 2_000_000i64;
    let max_age = Some(500_000u64); // relative cutoff = 1_500_000
    let since = Some(1_600_000i64);

    // A: mtime 1_400_000 — fails both
    assert!(!accept(
        "a.dat",
        1_400_000,
        now,
        max_age,
        since,
        &FilterMode::None,
        false
    ));
    // B: mtime 1_600_000 — passes both
    assert!(accept(
        "b.dat",
        1_600_000,
        now,
        max_age,
        since,
        &FilterMode::None,
        false
    ));
    // C: mtime 1_800_000 — passes both
    assert!(accept(
        "c.dat",
        1_800_000,
        now,
        max_age,
        since,
        &FilterMode::None,
        false
    ));
}

// ─── Test 8: --exclude single pattern ────────────────────────────────────────

#[test]
fn test08_exclude_single_pattern() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");
    let out_dir = tmp.path().join("restored");

    make_test_world(&world_dir);

    let pat = glob::Pattern::new("region/*.mca").unwrap();
    let opts = CompressOptions {
        patterns: FilterMode::Exclude(vec![pat]),
        ..default_opts(&world_dir, &archive)
    };
    sbk::compress::compress(&world_dir, &opts).unwrap();
    sbk::decompress::decompress(&archive, &out_dir, 2).unwrap();

    // Overworld MCA excluded
    assert!(!out_dir.join("region/r.0.0.mca").exists());
    // DIM-1 MCA present
    assert!(out_dir.join("DIM-1/region/r.0.0.mca").exists());
}

// ─── Test 9: --exclude multiple patterns ─────────────────────────────────────

#[test]
fn test09_exclude_multiple_patterns() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");
    let out_dir = tmp.path().join("restored");

    make_test_world(&world_dir);

    let p1 = glob::Pattern::new("DIM-1/**").unwrap();
    let opts = CompressOptions {
        patterns: FilterMode::Exclude(vec![p1]),
        ..default_opts(&world_dir, &archive)
    };
    sbk::compress::compress(&world_dir, &opts).unwrap();
    sbk::decompress::decompress(&archive, &out_dir, 2).unwrap();

    // DIM-1 absent
    assert!(!out_dir.join("DIM-1/region/r.0.0.mca").exists());
    // Overworld present
    assert!(out_dir.join("region/r.0.0.mca").exists());
}

// ─── Test 10: --include single pattern ───────────────────────────────────────

#[test]
fn test10_include_single_pattern() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");
    let out_dir = tmp.path().join("restored");

    make_test_world(&world_dir);

    let pat = glob::Pattern::new("**/*.json").unwrap();
    let opts = CompressOptions {
        patterns: FilterMode::Include(vec![pat]),
        ..default_opts(&world_dir, &archive)
    };
    sbk::compress::compress(&world_dir, &opts).unwrap();
    sbk::decompress::decompress(&archive, &out_dir, 2).unwrap();

    // Only JSON files
    assert!(out_dir.join("advancements/player.json").exists());
    // No MCA, no RAW, no NBT
    assert!(!out_dir.join("region/r.0.0.mca").exists());
    assert!(!out_dir.join("icon.png").exists());
    assert!(!out_dir.join("level.dat").exists());
    // session.lock still absent
    assert!(!out_dir.join("session.lock").exists());
}

// ─── Test 11: --include multiple patterns ────────────────────────────────────

#[test]
fn test11_include_multiple_patterns() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");
    let out_dir = tmp.path().join("restored");

    make_test_world(&world_dir);

    let p1 = glob::Pattern::new("level.dat").unwrap();
    // advancements/** to match any json under advancements
    let p2 = glob::Pattern::new("advancements/**").unwrap();
    let opts = CompressOptions {
        patterns: FilterMode::Include(vec![p1, p2]),
        ..default_opts(&world_dir, &archive)
    };
    sbk::compress::compress(&world_dir, &opts).unwrap();
    sbk::decompress::decompress(&archive, &out_dir, 2).unwrap();

    assert!(out_dir.join("level.dat").exists());
    assert!(out_dir.join("advancements/player.json").exists());
    assert!(!out_dir.join("region/r.0.0.mca").exists());
    assert!(!out_dir.join("icon.png").exists());
}

// ─── Test 12: --max-age + --include combined ─────────────────────────────────

#[test]
fn test12_max_age_and_include_combined() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");

    make_test_world(&world_dir);

    let now_ms = capture_now_ms();
    let ten_days_ms = 864_000_000i64;
    let five_days_ms: u64 = 432_000_000;

    // Backdate the only included file (region MCA) by 10 days
    set_mtime(&world_dir.join("region/r.0.0.mca"), now_ms - ten_days_ms);

    let pat = glob::Pattern::new("region/*.mca").unwrap();
    let opts = CompressOptions {
        max_age: Some(five_days_ms),
        patterns: FilterMode::Include(vec![pat]),
        ..default_opts(&world_dir, &archive)
    };
    sbk::compress::compress(&world_dir, &opts).unwrap();
    // Archive exists, no error
    assert!(archive.exists());
}

// ─── Test 13: Conflicting filters error ──────────────────────────────────────

#[test]
fn test13_conflicting_filters_error() {
    // This is validated at the CLI level in main.rs before compress() is called.
    // We test the error type directly.
    let err = SbkError::ConflictingFilters;
    let msg = err.to_string();
    assert!(msg.contains("mutually exclusive"));
}

// ─── Test 14: Invalid --max-age ──────────────────────────────────────────────

#[test]
fn test14_invalid_max_age() {
    let err = SbkError::InvalidMaxAge;
    assert!(err.to_string().contains("1 millisecond"));
}

// ─── Test 15: Invalid --since ────────────────────────────────────────────────

#[test]
fn test15_invalid_since() {
    let err = SbkError::InvalidSinceTimestamp;
    assert!(err.to_string().contains("non-negative"));
}

// ─── Test 16: Selective extract, single file ────────────────────────────────

#[test]
fn test16_selective_extract_single_file() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");
    let out_dir = tmp.path().join("extracted");

    make_test_world(&world_dir);
    let opts = default_opts(&world_dir, &archive);
    sbk::compress::compress(&world_dir, &opts).unwrap();

    let n =
        sbk::extract::extract(&archive, &["region/r.0.0.mca".to_string()], &out_dir, 2).unwrap();

    assert_eq!(n, 1);
    assert!(out_dir.join("region/r.0.0.mca").exists());
    assert!(!out_dir.join("icon.png").exists());
}

// ─── Test 17: Selective extract, glob ────────────────────────────────────────

#[test]
fn test17_selective_extract_glob() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");
    let out_dir = tmp.path().join("extracted");

    make_test_world(&world_dir);
    let opts = default_opts(&world_dir, &archive);
    sbk::compress::compress(&world_dir, &opts).unwrap();

    sbk::extract::extract(&archive, &["**/*.json".to_string()], &out_dir, 2).unwrap();

    assert!(out_dir.join("advancements/player.json").exists());
    assert!(!out_dir.join("region/r.0.0.mca").exists());
    assert!(!out_dir.join("icon.png").exists());
}

// ─── Test 18: No match (extract) ─────────────────────────────────────────────

#[test]
fn test18_no_match_extract() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");
    let out_dir = tmp.path().join("extracted");

    make_test_world(&world_dir);
    let opts = default_opts(&world_dir, &archive);
    sbk::compress::compress(&world_dir, &opts).unwrap();

    let result =
        sbk::extract::extract(&archive, &["nonexistent/file.dat".to_string()], &out_dir, 2);

    assert!(result.is_err());
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("No files matched") || err_str.contains("nonexistent"),
        "unexpected error: {}",
        err_str
    );
}

// ─── Test 19: Thread determinism ─────────────────────────────────────────────

#[test]
fn test19_thread_determinism() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");

    make_test_world(&world_dir);

    // Set consistent mtimes so output is deterministic
    let base_ms = 1_700_000_000_000i64;
    set_mtime(&world_dir.join("region/r.0.0.mca"), base_ms);
    set_mtime(&world_dir.join("DIM-1/region/r.0.0.mca"), base_ms + 1000);
    set_mtime(&world_dir.join("level.dat"), base_ms + 2000);
    set_mtime(&world_dir.join("advancements/player.json"), base_ms + 3000);
    set_mtime(&world_dir.join("icon.png"), base_ms + 4000);
    set_mtime(&world_dir.join("session.lock"), base_ms + 5000);

    let archives: Vec<_> = [1usize, 2, 4]
        .iter()
        .map(|&t| {
            let archive = tmp.path().join(format!("world_{}.sbk", t));
            let opts = CompressOptions {
                threads: t,
                level: 1,
                ..default_opts(&world_dir, &archive)
            };
            sbk::compress::compress(&world_dir, &opts).unwrap();
            archive
        })
        .collect();

    let bytes: Vec<Vec<u8>> = archives.iter().map(|a| fs::read(a).unwrap()).collect();
    assert_eq!(bytes[0], bytes[1], "1-thread vs 2-thread differ");
    assert_eq!(bytes[0], bytes[2], "1-thread vs 4-thread differ");
}

// ─── Test 20: --include-session-lock ─────────────────────────────────────────

#[test]
fn test20_include_session_lock() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");
    let out_dir = tmp.path().join("restored");

    make_test_world(&world_dir);

    // Without flag: session.lock absent
    let opts = default_opts(&world_dir, &archive);
    sbk::compress::compress(&world_dir, &opts).unwrap();
    sbk::decompress::decompress(&archive, &out_dir, 2).unwrap();
    assert!(!out_dir.join("session.lock").exists());

    // With flag: session.lock present
    let archive2 = tmp.path().join("world2.sbk");
    let out_dir2 = tmp.path().join("restored2");
    let opts2 = CompressOptions {
        include_session_lock: true,
        output: archive2.clone(),
        ..default_opts(&world_dir, &archive2)
    };
    sbk::compress::compress(&world_dir, &opts2).unwrap();
    sbk::decompress::decompress(&archive2, &out_dir2, 2).unwrap();
    assert!(out_dir2.join("session.lock").exists());
    assert_eq!(fs::read(out_dir2.join("session.lock")).unwrap(), b"lock");
}

// ─── Test 21: verify — clean archive passes ───────────────────────────────────

#[test]
fn test21_verify_clean() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");

    make_test_world(&world_dir);
    sbk::compress::compress(&world_dir, &default_opts(&world_dir, &archive)).unwrap();

    let ok = sbk::verify::verify(&archive, 2).unwrap();
    assert!(ok, "verify should return true for a clean archive");
}

// ─── Test 22: verify — corrupted frame is detected ────────────────────────────

#[test]
fn test22_verify_corrupted() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");

    make_test_world(&world_dir);
    sbk::compress::compress(&world_dir, &default_opts(&world_dir, &archive)).unwrap();

    // Flip some bytes in the middle of the file
    let mut bytes = fs::read(&archive).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xFF;
    bytes[mid + 1] ^= 0xFF;
    fs::write(&archive, &bytes).unwrap();

    // verify should either return Ok(false) or error — either means corruption detected
    match sbk::verify::verify(&archive, 2) {
        Ok(ok) => assert!(!ok, "corrupted archive should fail verification"),
        Err(_) => {} // also acceptable
    }
}

// ─── Test 23: info — smoke test ───────────────────────────────────────────────

#[test]
fn test23_info_smoke() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");

    make_test_world(&world_dir);
    sbk::compress::compress(&world_dir, &default_opts(&world_dir, &archive)).unwrap();

    // Both modes must not error
    sbk::info::info(&archive, false).unwrap();
    sbk::info::info(&archive, true).unwrap();
}

// ─── Test 24: MCA and NBT content round-trip ─────────────────────────────────

#[test]
fn test24_content_round_trip() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");
    let out_dir = tmp.path().join("restored");

    make_test_world(&world_dir);
    sbk::compress::compress(&world_dir, &default_opts(&world_dir, &archive)).unwrap();
    sbk::decompress::decompress(&archive, &out_dir, 2).unwrap();

    // MCA: the decompressed file must be a valid MCA (parseable) and have the same chunks
    let orig_mca = fs::read(world_dir.join("region/r.0.0.mca")).unwrap();
    let rest_mca = fs::read(out_dir.join("region/r.0.0.mca")).unwrap();
    // Both must be multiples of 4096 (sector size)
    assert_eq!(orig_mca.len() % 4096, 0);
    assert_eq!(rest_mca.len() % 4096, 0);
    // Location table must agree on which slots are populated
    for slot in 0..1024 {
        let base = slot * 4;
        let orig_entry = u32::from_be_bytes(orig_mca[base..base + 4].try_into().unwrap());
        let rest_entry = u32::from_be_bytes(rest_mca[base..base + 4].try_into().unwrap());
        let orig_present = (orig_entry & 0xFF) != 0;
        let rest_present = (rest_entry & 0xFF) != 0;
        assert_eq!(orig_present, rest_present, "slot {} presence differs", slot);
    }

    // NBT: level.dat must be gzip-decodable and contain our tag bytes
    let rest_nbt = fs::read(out_dir.join("level.dat")).unwrap();
    use flate2::read::GzDecoder;
    use std::io::Read;
    let mut dec = GzDecoder::new(rest_nbt.as_slice());
    let mut raw = Vec::new();
    dec.read_to_end(&mut raw)
        .expect("level.dat should be valid gzip after round-trip");
    assert!(raw.len() >= 4, "decompressed NBT too short");
}

// ─── Helper: build a minimal valid archive from scratch ──────────────────────

fn make_crafted_archive(archive_path: &Path, entry_path: &str) {
    use sbk::codec;
    use sbk::format::frame_dir::{FrameDir, write_frame_dir};
    use sbk::format::header::{
        Algorithm, HEADER_DISK_SIZE, Header, write_header, write_placeholder,
    };
    use sbk::format::index::{IndexEntry, write_index};
    use sbk::solid::FRAME_SIZE;

    let codec = codec::from_algorithm(Algorithm::Lzma2);
    let mut f = fs::File::create(archive_path).unwrap();
    write_placeholder(&mut f).unwrap();

    let frame_dir_offset = HEADER_DISK_SIZE as u64;
    let fd = FrameDir::new();
    write_frame_dir(&mut f, &fd).unwrap();
    let frame_dir_size = fd.disk_size();
    let index_offset = frame_dir_offset + frame_dir_size;

    let entries = vec![IndexEntry {
        path: entry_path.to_string(),
        mtime_ms: 0,
        group_id: 3,
        stream_offset: 0,
        stream_raw_size: 0,
        original_size: 0,
        file_checksum: 0,
    }];
    let (index_compressed_size, index_raw_size, index_checksum) =
        write_index(&entries, &*codec, 1, &mut f).unwrap();

    f.seek(SeekFrom::Start(0)).unwrap();
    write_header(
        &mut f,
        &Header {
            format_version: 1,
            flags: 0,
            algorithm: Algorithm::Lzma2,
            file_count: 1,
            frame_size_bytes: FRAME_SIZE,
            frame_dir_offset,
            frame_dir_size,
            index_offset,
            index_compressed_size,
            index_raw_size,
            index_checksum,
        },
    )
    .unwrap();
}

// ─── Test 25: path traversal with `..` is rejected ───────────────────────────

#[test]
fn test25_path_traversal_dotdot_rejected() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("crafted.sbk");
    let out_dir = tmp.path().join("out");

    make_crafted_archive(&archive, "../escape.txt");

    let result = sbk::extract::extract(&archive, &["**".to_string()], &out_dir, 1);
    assert!(result.is_err(), "path traversal should be rejected");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("..") || msg.contains("traversal") || msg.contains("path"),
        "unexpected error: {}",
        msg
    );
}

// ─── Test 26: absolute path in archive is rejected ───────────────────────────

#[test]
fn test26_absolute_path_in_archive_rejected() {
    let tmp = TempDir::new().unwrap();
    let archive = tmp.path().join("crafted.sbk");
    let out_dir = tmp.path().join("out");

    make_crafted_archive(&archive, "/etc/passwd");

    let result = sbk::extract::extract(&archive, &["**".to_string()], &out_dir, 1);
    assert!(result.is_err(), "absolute path should be rejected");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("absolute") || msg.contains("path"),
        "unexpected error: {}",
        msg
    );
}

// ─── Test 27: malicious frame count is rejected ───────────────────────────────

#[test]
fn test27_malicious_frame_count_rejected() {
    use std::io::Cursor;

    // Frame count > MAX_FRAMES_PER_GROUP (1_000_000) for group 0
    let mut data = Vec::new();
    data.extend_from_slice(&1_000_001u32.to_le_bytes());

    let result = sbk::format::frame_dir::read_frame_dir(&mut Cursor::new(data));
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("sanity limit") || msg.contains("frame count"),
        "unexpected error: {}",
        msg
    );
}

// ─── Test 28: malicious index entry count is rejected ────────────────────────

#[test]
fn test28_malicious_index_entry_count_rejected() {
    use sbk::codec;
    use sbk::format::header::Algorithm;
    use std::io::Cursor;

    // Craft a raw index claiming 20_000_000 entries (> MAX_INDEX_ENTRIES = 10_000_000)
    let mut raw = Vec::new();
    raw.extend_from_slice(&20_000_000u64.to_le_bytes());

    let codec = codec::from_algorithm(Algorithm::Lzma2);
    let compressed = codec.compress(&raw, 1).unwrap();
    let compressed_size = compressed.len() as u64;
    let checksum = sbk::checksum::hash(&compressed);

    let result = sbk::format::index::read_index(
        &mut Cursor::new(compressed),
        &*codec,
        compressed_size,
        checksum,
    );
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("sanity limit") || msg.contains("entry count"),
        "unexpected error: {}",
        msg
    );
}

// ─── Test 29: malicious index compressed size is rejected ────────────────────

#[test]
fn test29_malicious_index_compressed_size_rejected() {
    use sbk::codec;
    use sbk::format::header::Algorithm;
    use std::io::Cursor;

    let codec = codec::from_algorithm(Algorithm::Lzma2);
    // 300 MiB > MAX_INDEX_COMPRESSED_SIZE (256 MiB)
    let huge: u64 = 300 * 1024 * 1024;
    let result =
        sbk::format::index::read_index(&mut Cursor::new(Vec::<u8>::new()), &*codec, huge, 0);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("sanity limit") || msg.contains("index compressed"),
        "unexpected error: {}",
        msg
    );
}

// ─── Test 30: empty world compresses and decompresses without error ───────────

#[test]
fn test30_empty_world_round_trip() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("empty_world");
    let archive = tmp.path().join("empty.sbk");
    let out_dir = tmp.path().join("restored");

    fs::create_dir_all(&world_dir).unwrap();

    let opts = default_opts(&world_dir, &archive);
    sbk::compress::compress(&world_dir, &opts).unwrap();
    assert!(archive.exists());

    sbk::decompress::decompress(&archive, &out_dir, 2).unwrap();

    // extract with ** on an empty archive returns 0 files
    let n = sbk::extract::extract(&archive, &["**".to_string()], &out_dir, 2).unwrap();
    assert_eq!(n, 0);
}

// ─── Test 31: two actual exclude patterns ────────────────────────────────────
// (test09 uses only one pattern despite its name; this test exercises two)

#[test]
fn test31_exclude_two_actual_patterns() {
    let tmp = TempDir::new().unwrap();
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");
    let out_dir = tmp.path().join("restored");

    make_test_world(&world_dir);

    let p1 = glob::Pattern::new("region/*.mca").unwrap();
    let p2 = glob::Pattern::new("DIM-1/**").unwrap();
    let opts = CompressOptions {
        patterns: FilterMode::Exclude(vec![p1, p2]),
        ..default_opts(&world_dir, &archive)
    };
    sbk::compress::compress(&world_dir, &opts).unwrap();
    sbk::decompress::decompress(&archive, &out_dir, 2).unwrap();

    // Both MCA files excluded by their respective patterns
    assert!(!out_dir.join("region/r.0.0.mca").exists());
    assert!(!out_dir.join("DIM-1/region/r.0.0.mca").exists());
    // Non-excluded files still present
    assert!(out_dir.join("level.dat").exists());
    assert!(out_dir.join("icon.png").exists());
    assert!(out_dir.join("advancements/player.json").exists());
}

// ── Header round-trip: lzma2 ─────────────────────────────────────────────
#[test]
fn header_roundtrip_lzma2() {
    use sbk::format::header::{Algorithm, HEADER_DISK_SIZE, Header, read_header, write_header};
    let h = Header {
        format_version: 1,
        flags: 0,
        algorithm: Algorithm::Lzma2,
        file_count: 42,
        frame_size_bytes: 16 * 1024 * 1024,
        frame_dir_offset: 79,
        frame_dir_size: 24,
        index_offset: 1024,
        index_compressed_size: 512,
        index_raw_size: 1024,
        index_checksum: 0xDEADBEEF,
    };
    let mut buf = Vec::new();
    write_header(&mut buf, &h).unwrap();
    assert_eq!(buf.len(), HEADER_DISK_SIZE, "header must be 79 bytes");
    let h2 = read_header(&mut std::io::Cursor::new(&buf)).unwrap();
    assert_eq!(h2.algorithm, Algorithm::Lzma2);
    assert_eq!(h2.file_count, 42);
    assert_eq!(h2.frame_dir_offset, 79);
    assert_eq!(h2.index_checksum, 0xDEADBEEF);
}

// ── Header round-trip: zstd ──────────────────────────────────────────────
#[test]
fn header_roundtrip_zstd() {
    use sbk::format::header::{Algorithm, HEADER_DISK_SIZE, Header, read_header, write_header};
    let h = Header {
        format_version: 1,
        flags: 0,
        algorithm: Algorithm::Zstd,
        file_count: 7,
        frame_size_bytes: 16 * 1024 * 1024,
        frame_dir_offset: 79,
        frame_dir_size: 0,
        index_offset: 200,
        index_compressed_size: 100,
        index_raw_size: 200,
        index_checksum: 0xCAFEBABE,
    };
    let mut buf = Vec::new();
    write_header(&mut buf, &h).unwrap();
    assert_eq!(buf.len(), HEADER_DISK_SIZE);
    assert_eq!(buf[10], 1u8, "byte 10 must be 1 for zstd");
    let h2 = read_header(&mut std::io::Cursor::new(&buf)).unwrap();
    assert_eq!(h2.algorithm, Algorithm::Zstd);
}

// ── Unknown algorithm byte rejected ─────────────────────────────────────
#[test]
fn unknown_algorithm_rejected() {
    use sbk::checksum::hash;
    use sbk::error::SbkError;
    use sbk::format::header::{HEADER_DISK_SIZE, MAGIC, read_header};
    let mut buf = [0u8; HEADER_DISK_SIZE];
    buf[0..8].copy_from_slice(&MAGIC);
    buf[8] = 1; // format_version
    buf[10] = 99; // unknown algorithm
    // recompute header_checksum: hash of bytes 0..75 with 75..79 zeroed
    let cs = hash(&buf[0..75]);
    buf[75..79].copy_from_slice(&cs.to_le_bytes());
    let result = read_header(&mut std::io::Cursor::new(&buf));
    assert!(matches!(
        result.unwrap_err().downcast::<SbkError>().unwrap(),
        SbkError::UnsupportedAlgorithm(99)
    ));
}

// ── Non-zero reserved bytes rejected ────────────────────────────────────
#[test]
fn nonzero_reserved_rejected() {
    use sbk::checksum::hash;
    use sbk::format::header::{HEADER_DISK_SIZE, MAGIC, read_header};
    let mut buf = [0u8; HEADER_DISK_SIZE];
    buf[0..8].copy_from_slice(&MAGIC);
    buf[8] = 1; // format_version
    buf[14] = 1; // non-zero reserved byte
    let cs = hash(&buf[0..75]);
    buf[75..79].copy_from_slice(&cs.to_le_bytes());
    let result = read_header(&mut std::io::Cursor::new(&buf));
    assert!(result.is_err(), "non-zero reserved bytes must be rejected");
}

// ── Full round-trip: lzma2 ───────────────────────────────────────────────
#[test]
fn full_roundtrip_lzma2() {
    let world_dir = tempfile::tempdir().unwrap();
    let out_dir = tempfile::tempdir().unwrap();
    let archive = out_dir.path().join("test.sbk");
    make_test_world(world_dir.path());
    let opts = default_opts(world_dir.path(), &archive);
    sbk::compress::compress(world_dir.path(), &opts).unwrap();
    let extract_dir = tempfile::tempdir().unwrap();
    sbk::extract::extract(&archive, &["**".to_string()], extract_dir.path(), 2).unwrap();
    // Verify all files exist in extracted output.
    // Note: JSON and MCA/NBT files are preprocessed (minified/re-encoded) during compression,
    // so we only do byte-exact comparison for RAW group files (e.g. .png).
    for entry in walkdir::WalkDir::new(world_dir.path())
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            let rel = entry.path().strip_prefix(world_dir.path()).unwrap();
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if rel_str == "session.lock" {
                continue;
            }
            let extracted = extract_dir.path().join(rel);
            assert!(extracted.exists(), "missing: {}", rel_str);
            // Only byte-compare RAW files (non-preprocessed)
            let ext = entry
                .path()
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if !["mca", "dat", "dat_old", "json"].contains(&ext) {
                assert_eq!(
                    std::fs::read(entry.path()).unwrap(),
                    std::fs::read(&extracted).unwrap(),
                    "content mismatch: {}",
                    rel_str
                );
            }
        }
    }
}

// ── Full round-trip: zstd ────────────────────────────────────────────────
#[test]
fn full_roundtrip_zstd() {
    use sbk::format::header::{Algorithm, read_header};
    let world_dir = tempfile::tempdir().unwrap();
    let out_dir = tempfile::tempdir().unwrap();
    let archive = out_dir.path().join("test_zstd.sbk");
    make_test_world(world_dir.path());
    let mut opts = default_opts(world_dir.path(), &archive);
    opts.algorithm = Algorithm::Zstd;
    sbk::compress::compress(world_dir.path(), &opts).unwrap();
    // verify algorithm byte in archive
    let mut f = std::fs::File::open(&archive).unwrap();
    let h = read_header(&mut f).unwrap();
    assert_eq!(h.algorithm, Algorithm::Zstd);
    // round-trip
    let extract_dir = tempfile::tempdir().unwrap();
    sbk::extract::extract(&archive, &["**".to_string()], extract_dir.path(), 2).unwrap();
    for entry in walkdir::WalkDir::new(world_dir.path())
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            let rel = entry.path().strip_prefix(world_dir.path()).unwrap();
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if rel_str == "session.lock" {
                continue;
            }
            let extracted = extract_dir.path().join(rel);
            assert!(extracted.exists(), "missing: {}", rel_str);
        }
    }
}

// ── zstd archive: verify + info ──────────────────────────────────────────
#[test]
fn zstd_verify_and_info() {
    use sbk::format::header::Algorithm;
    let world_dir = tempfile::tempdir().unwrap();
    let out_dir = tempfile::tempdir().unwrap();
    let archive = out_dir.path().join("verify_zstd.sbk");
    make_test_world(world_dir.path());
    let mut opts = default_opts(world_dir.path(), &archive);
    opts.algorithm = Algorithm::Zstd;
    sbk::compress::compress(world_dir.path(), &opts).unwrap();
    let ok = sbk::verify::verify(&archive, 2).unwrap();
    assert!(ok, "verify must pass for valid zstd archive");
}

// ── Invalid --algorithm CLI value ────────────────────────────────────────
#[test]
fn invalid_algorithm_cli() {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_sbk"));
    let world_dir = tempfile::tempdir().unwrap();
    let out_dir = tempfile::tempdir().unwrap();
    let archive = out_dir.path().join("out.sbk");
    let output = cmd
        .args([
            "compress",
            world_dir.path().to_str().unwrap(),
            "-o",
            archive.to_str().unwrap(),
            "--algorithm",
            "bogus",
        ])
        .output()
        .unwrap();
    assert_ne!(output.status.code().unwrap_or(0), 0, "must exit non-zero");
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Must mention valid options
    assert!(
        stderr.contains("lzma2") || stderr.contains("zstd"),
        "error message must mention valid options, got: {stderr}"
    );
}

// ── Algorithm byte covered by header checksum ────────────────────────────
#[test]
fn algorithm_byte_covered_by_checksum() {
    use sbk::format::header::{Algorithm, read_header};
    let world_dir = tempfile::tempdir().unwrap();
    let out_dir = tempfile::tempdir().unwrap();
    let archive = out_dir.path().join("flip_algo.sbk");
    make_test_world(world_dir.path());
    let mut opts = default_opts(world_dir.path(), &archive);
    opts.algorithm = Algorithm::Zstd;
    sbk::compress::compress(world_dir.path(), &opts).unwrap();
    // Flip byte 10 from 1 (zstd) to 0 (lzma2) WITHOUT recomputing checksum
    let mut data = std::fs::read(&archive).unwrap();
    assert_eq!(data[10], 1u8);
    data[10] = 0;
    std::fs::write(&archive, &data).unwrap();
    // Must fail with HeaderChecksumMismatch
    use sbk::error::SbkError;
    let mut f = std::fs::File::open(&archive).unwrap();
    let err = read_header(&mut f).unwrap_err();
    assert!(matches!(
        err.downcast::<SbkError>().unwrap(),
        SbkError::HeaderChecksumMismatch
    ));
}

// ── Frame decompression bomb guard ──────────────────────────────────────
#[test]
fn frame_decompression_bomb_guard() {
    use sbk::codec::from_algorithm;
    use sbk::format::header::Algorithm;
    // Build a payload that compresses small but decompresses large
    // 200 bytes of actual data
    let big_data = vec![0xAAu8; 200];
    for algo in [Algorithm::Lzma2, Algorithm::Zstd] {
        let codec = from_algorithm(algo);
        let compressed = codec.compress(&big_data, 1).unwrap();
        // Claim only 50 bytes expected — must fail
        let result = codec.decompress(&compressed, 50);
        assert!(result.is_err(), "bomb guard must reject for {:?}", algo);
    }
}

// ─── Convert tests ────────────────────────────────────────────────────────────

fn compress_test_world(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let world_dir = tmp.path().join("world");
    let archive = tmp.path().join("world.sbk");
    make_test_world(&world_dir);
    let opts = default_opts(&world_dir, &archive);
    sbk::compress::compress(&world_dir, &opts).unwrap();
    (world_dir, archive)
}

#[test]
fn test_convert_zip() {
    use std::io::Read as _;

    let tmp = TempDir::new().unwrap();
    let (_world_dir, archive) = compress_test_world(&tmp);
    let zip_path = tmp.path().join("world.zip");

    let n =
        sbk::convert::convert(&archive, &zip_path, sbk::convert::ConvertFormat::Zip, 2, 6).unwrap();
    assert!(n > 0, "expected at least one file in converted ZIP");
    assert!(zip_path.exists());

    // Inspect ZIP contents
    let zip_file = fs::File::open(&zip_path).unwrap();
    let mut archive = zip::ZipArchive::new(zip_file).unwrap();
    let names: Vec<String> = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .collect();

    assert!(
        names.iter().any(|n| n.ends_with(".mca")),
        "ZIP should contain an MCA file; got: {:?}",
        names
    );
    assert!(
        names.iter().any(|n| n == "level.dat"),
        "ZIP should contain level.dat; got: {:?}",
        names
    );
    assert!(
        names.iter().any(|n| n.ends_with(".json")),
        "ZIP should contain a JSON file; got: {:?}",
        names
    );
    assert!(
        names.iter().any(|n| n == "icon.png"),
        "ZIP should contain icon.png; got: {:?}",
        names
    );

    // Verify icon.png is byte-identical to original
    let mut entry = archive
        .by_name("icon.png")
        .expect("icon.png must be in ZIP");
    let mut data = Vec::new();
    entry.read_to_end(&mut data).unwrap();
    assert_eq!(data, b"\x89PNG\r\n\x1a\nfakeicon");

    // session.lock must not be present
    assert!(
        !names.iter().any(|n| n == "session.lock"),
        "session.lock must not appear in converted archive"
    );
}

#[test]
fn test_convert_tar_gz() {
    let tmp = TempDir::new().unwrap();
    let (_world_dir, archive) = compress_test_world(&tmp);
    let tar_gz_path = tmp.path().join("world.tar.gz");

    let n = sbk::convert::convert(
        &archive,
        &tar_gz_path,
        sbk::convert::ConvertFormat::TarGz,
        2,
        6,
    )
    .unwrap();
    assert!(n > 0);
    assert!(tar_gz_path.exists());

    // Inspect tar.gz contents
    let file = fs::File::open(&tar_gz_path).unwrap();
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    let names: Vec<String> = tar
        .entries()
        .unwrap()
        .map(|e| e.unwrap().path().unwrap().to_string_lossy().to_string())
        .collect();

    assert!(
        names.iter().any(|n| n.ends_with(".mca")),
        "tar.gz should contain an MCA file; got: {:?}",
        names
    );
    assert!(
        names.iter().any(|n| n == "level.dat"),
        "tar.gz should contain level.dat; got: {:?}",
        names
    );
    assert!(
        !names.iter().any(|n| n == "session.lock"),
        "session.lock must not appear in converted archive"
    );
}

#[test]
fn test_convert_tar_xz() {
    let tmp = TempDir::new().unwrap();
    let (_world_dir, archive) = compress_test_world(&tmp);
    let tar_xz_path = tmp.path().join("world.tar.xz");

    let n = sbk::convert::convert(
        &archive,
        &tar_xz_path,
        sbk::convert::ConvertFormat::TarXz,
        2,
        1, // fast for test
    )
    .unwrap();
    assert!(n > 0);
    assert!(tar_xz_path.exists());

    // Inspect tar.xz contents
    let file = fs::File::open(&tar_xz_path).unwrap();
    let decoder = xz2::read::XzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    let names: Vec<String> = tar
        .entries()
        .unwrap()
        .map(|e| e.unwrap().path().unwrap().to_string_lossy().to_string())
        .collect();

    assert!(
        names.iter().any(|n| n.ends_with(".mca")),
        "tar.xz should contain an MCA file; got: {:?}",
        names
    );
    assert!(
        names.iter().any(|n| n == "level.dat"),
        "tar.xz should contain level.dat; got: {:?}",
        names
    );
    assert!(
        !names.iter().any(|n| n == "session.lock"),
        "session.lock must not appear in converted archive"
    );
}

#[test]
fn test_convert_invalid_format() {
    let tmp = TempDir::new().unwrap();
    let (_world_dir, _archive) = compress_test_world(&tmp);

    // ConvertFormat::from_str should return None for unknown format
    let fmt = sbk::convert::ConvertFormat::from_str("rar");
    assert!(fmt.is_none(), "unknown format must return None");
}

#[test]
fn test_convert_default_output_path() {
    // The default output path logic is in main.rs; test it by verifying the
    // extension() method returns the right string for each format.
    use sbk::convert::ConvertFormat;

    let stem = "myworld";
    let zip_path = format!("{}{}", stem, ConvertFormat::Zip.extension());
    let tar_gz_path = format!("{}{}", stem, ConvertFormat::TarGz.extension());
    let tar_xz_path = format!("{}{}", stem, ConvertFormat::TarXz.extension());

    assert_eq!(zip_path, "myworld.zip");
    assert_eq!(tar_gz_path, "myworld.tar.gz");
    assert_eq!(tar_xz_path, "myworld.tar.xz");
}

#[test]
fn test_convert_zip_round_trip_mca() {
    // Compress a world with an MCA file, convert to ZIP, and verify the MCA
    // can be re-preprocessed to yield the same chunk data.
    use std::io::Read as _;

    let tmp = TempDir::new().unwrap();
    let (_world_dir, archive) = compress_test_world(&tmp);
    let zip_path = tmp.path().join("world_rt.zip");

    sbk::convert::convert(&archive, &zip_path, sbk::convert::ConvertFormat::Zip, 2, 1).unwrap();

    let zip_file = fs::File::open(&zip_path).unwrap();
    let mut za = zip::ZipArchive::new(zip_file).unwrap();

    // Read back region/r.0.0.mca from ZIP and verify it parses as valid MCA
    let mca_data = {
        let mut entry = za.by_name("region/r.0.0.mca").expect("MCA must be in ZIP");
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        buf
    };

    // Re-preprocess it — should succeed and yield 5 chunks
    let mcap = sbk::preprocess::mca::preprocess_mca_from_bytes(&mca_data).unwrap();
    let chunk_count = u16::from_le_bytes([mcap[4], mcap[5]]);
    assert_eq!(chunk_count, 5, "reconstructed MCA must have 5 chunks");
}
