//! Hardening tests: corrupt/malicious archives must produce errors, never
//! panics; empty files must survive extraction; zip conversion must be safe.

use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use tempfile::TempDir;

use sbk::checksum::hash;
use sbk::filter::{CompressOptions, FilterMode};
use sbk::format::header::Algorithm;

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Create a world directory from a list of (relative path, contents) pairs.
fn make_world(dir: &Path, files: &[(&str, &[u8])]) {
    for (rel, data) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, data).unwrap();
    }
}

fn opts(output: &Path) -> CompressOptions {
    CompressOptions {
        output: output.to_path_buf(),
        threads: 2,
        level: 1,
        algorithm: Algorithm::Zstd,
        max_age: None,
        since: None,
        patterns: FilterMode::None,
        include_session_lock: false,
        quiet: true,
    }
}

/// Compress a small default world and return (tempdir, archive path).
fn small_archive() -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let world = tmp.path().join("world");
    make_world(
        &world,
        &[
            ("icon.png", b"\x89PNG\r\n\x1a\nfakeicon" as &[u8]),
            ("data/notes.txt", b"hello world, this is a test file"),
        ],
    );
    let archive = tmp.path().join("world.sbk");
    sbk::compress::compress(&world, &opts(&archive)).unwrap();
    (tmp, archive)
}

/// Overwrite `len(bytes)` bytes at `offset` in the file.
fn patch(path: &Path, offset: u64, bytes: &[u8]) {
    let mut f = fs::OpenOptions::new().write(true).open(path).unwrap();
    f.seek(SeekFrom::Start(offset)).unwrap();
    f.write_all(bytes).unwrap();
}

/// Recompute the header checksum (xxHash32 over bytes 0..75, stored at 75..79)
/// after patching header bytes.
fn refix_header_checksum(path: &Path) {
    let mut f = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let mut buf = [0u8; 79];
    f.seek(SeekFrom::Start(0)).unwrap();
    f.read_exact(&mut buf).unwrap();
    buf[75..79].fill(0);
    let checksum = hash(&buf[0..75]);
    f.seek(SeekFrom::Start(75)).unwrap();
    f.write_all(&checksum.to_le_bytes()).unwrap();
}

/// Read header fields we need for byte-patching without going through
/// `read_header` (which may reject the archive after patching).
fn read_u64_at(path: &Path, offset: u64) -> u64 {
    let mut f = File::open(path).unwrap();
    f.seek(SeekFrom::Start(offset)).unwrap();
    let mut buf = [0u8; 8];
    f.read_exact(&mut buf).unwrap();
    u64::from_le_bytes(buf)
}

// ─── M3: header frame_size validation ────────────────────────────────────────

#[test]
fn header_frame_size_zero_rejected() {
    let (tmp, archive) = small_archive();
    // bytes 23..31 = frame_size_bytes (u64 LE)
    patch(&archive, 23, &0u64.to_le_bytes());
    refix_header_checksum(&archive);

    let out = tmp.path().join("out");
    let result = sbk::extract::extract(&archive, &["**".to_string()], &out, 1);
    assert!(result.is_err(), "frame_size_bytes = 0 must be rejected");

    let mut f = File::open(&archive).unwrap();
    assert!(sbk::format::header::read_header(&mut f).is_err());
}

#[test]
fn header_frame_size_huge_rejected() {
    let (_tmp, archive) = small_archive();
    // 2 GiB frame size — over the 1 GiB sanity limit
    patch(&archive, 23, &(2u64 * 1024 * 1024 * 1024).to_le_bytes());
    refix_header_checksum(&archive);

    let mut f = File::open(&archive).unwrap();
    assert!(
        sbk::format::header::read_header(&mut f).is_err(),
        "frame_size_bytes > 1 GiB must be rejected"
    );
}

// ─── M3: frame directory raw-size bound ──────────────────────────────────────

#[test]
fn frame_dir_raw_sz_exceeding_frame_size_rejected() {
    let (tmp, archive) = small_archive();

    // frame_dir_offset lives at header bytes 31..39
    let frame_dir_offset = read_u64_at(&archive, 31);

    // Walk the 4 groups to find the first one with frames; patch the first
    // frame's raw_sz (offset +4 count, +8 frame_offset, +4 compressed_sz).
    let mut f = File::open(&archive).unwrap();
    f.seek(SeekFrom::Start(frame_dir_offset)).unwrap();
    let mut pos = frame_dir_offset;
    let mut patched = false;
    for _ in 0..4 {
        let mut cnt = [0u8; 4];
        f.read_exact(&mut cnt).unwrap();
        let count = u32::from_le_bytes(cnt) as u64;
        pos += 4;
        if count > 0 {
            drop(f);
            patch(&archive, pos + 8 + 4, &u32::MAX.to_le_bytes());
            patched = true;
            break;
        }
        pos += count * 20;
    }
    assert!(patched, "archive should contain at least one frame");

    let out = tmp.path().join("out");
    let result = sbk::extract::extract(&archive, &["**".to_string()], &out, 1);
    assert!(
        result.is_err(),
        "frame_raw_sz > frame_size must be rejected, got {:?}",
        result
    );
}

// ─── M3: index entry validation (unit tests) ─────────────────────────────────

#[test]
fn validate_entries_rejects_bad_entries() {
    use sbk::format::frame_dir::{FrameDir, FrameEntry};
    use sbk::format::index::{IndexEntry, validate_entries};

    let frame_size = 16u64 * 1024 * 1024;
    let mut fd = FrameDir::new();
    // one frame in group 3 (RAW)
    fd.groups[3].push(FrameEntry {
        frame_offset: 79,
        frame_compressed_sz: 100,
        frame_raw_sz: 1000,
        frame_checksum: 0,
    });

    let entry = |group_id: u8, off: u64, size: u64| IndexEntry {
        path: "a.txt".to_string(),
        mtime_ms: 0,
        group_id,
        stream_offset: off,
        stream_raw_size: size,
        original_size: size,
        file_checksum: 0,
    };

    // Valid entry passes
    assert!(validate_entries(&[entry(3, 0, 1000)], &fd, frame_size).is_ok());

    // group_id out of range
    assert!(validate_entries(&[entry(7, 0, 10)], &fd, frame_size).is_err());

    // stream_offset + stream_raw_size overflows u64
    assert!(validate_entries(&[entry(3, u64::MAX, 2)], &fd, frame_size).is_err());

    // stream extends beyond the group's frames (1 frame × frame_size)
    assert!(validate_entries(&[entry(3, frame_size - 1, 2)], &fd, frame_size).is_err());

    // group with zero frames but non-zero stream
    assert!(validate_entries(&[entry(0, 0, 1)], &fd, frame_size).is_err());

    // zero-size entry in an empty group is fine
    assert!(validate_entries(&[entry(0, 0, 0)], &fd, frame_size).is_ok());
}

// ─── M3: slice_from_frames bounds check ──────────────────────────────────────

#[test]
fn slice_from_frames_short_frame_errors_instead_of_panicking() {
    use sbk::solid::extractor::slice_from_frames;

    let frame_size = 16u64;

    // Single frame shorter than the declared stream
    let mut frames: HashMap<(u8, u32), Vec<u8>> = HashMap::new();
    frames.insert((0, 0), vec![1, 2, 3]);
    let result = slice_from_frames(&frames, 0, 0, 10, frame_size);
    assert!(result.is_err(), "short frame must be an error, not a panic");

    // Stream spanning two frames where the middle/first frame is short
    let mut frames: HashMap<(u8, u32), Vec<u8>> = HashMap::new();
    frames.insert((0, 0), vec![0u8; 4]); // should be 16 bytes
    frames.insert((0, 1), vec![0u8; 8]);
    let result = slice_from_frames(&frames, 0, 8, 12, frame_size);
    assert!(result.is_err(), "short first frame must be an error");

    // Sanity: correct frames still work
    let mut frames: HashMap<(u8, u32), Vec<u8>> = HashMap::new();
    frames.insert((0, 0), (0u8..16).collect());
    frames.insert((0, 1), (16u8..24).collect());
    let out = slice_from_frames(&frames, 0, 12, 8, frame_size).unwrap();
    assert_eq!(out, vec![12, 13, 14, 15, 16, 17, 18, 19]);
}

// ─── M2: empty files are extracted ───────────────────────────────────────────

#[test]
fn empty_file_only_world_extracts_the_empty_file() {
    let tmp = TempDir::new().unwrap();
    let world = tmp.path().join("world");
    make_world(&world, &[("empty.txt", b"" as &[u8])]);
    let archive = tmp.path().join("world.sbk");
    sbk::compress::compress(&world, &opts(&archive)).unwrap();

    let out = tmp.path().join("out");
    let n = sbk::extract::extract(&archive, &["**".to_string()], &out, 2).unwrap();
    assert_eq!(n, 1);
    let restored = out.join("empty.txt");
    assert!(
        restored.exists(),
        "zero-byte file must be written even when no frames are needed"
    );
    assert_eq!(fs::metadata(&restored).unwrap().len(), 0);
}

#[test]
fn mixed_empty_and_normal_files_both_extract() {
    let tmp = TempDir::new().unwrap();
    let world = tmp.path().join("world");
    make_world(
        &world,
        &[
            ("empty.txt", b"" as &[u8]),
            ("sub/data.txt", b"some real content here"),
        ],
    );
    let archive = tmp.path().join("world.sbk");
    sbk::compress::compress(&world, &opts(&archive)).unwrap();

    let out = tmp.path().join("out");
    let n = sbk::extract::extract(&archive, &["**".to_string()], &out, 2).unwrap();
    assert_eq!(n, 2);
    assert!(out.join("empty.txt").exists());
    assert_eq!(fs::metadata(out.join("empty.txt")).unwrap().len(), 0);
    assert_eq!(
        fs::read(out.join("sub/data.txt")).unwrap(),
        b"some real content here"
    );
}

// ─── H1: zip-slip in convert_to_sbk ──────────────────────────────────────────

#[test]
fn convert_to_sbk_rejects_zip_slip_entry() {
    use zip::write::SimpleFileOptions;

    let tmp = TempDir::new().unwrap();
    let zip_path = tmp.path().join("evil.zip");

    let evil_name = "../sbk_hardening_evil_sentinel.txt";
    // Sentinel location: TempDir::new() creates dirs inside env::temp_dir(),
    // so `<tempdir>/../<name>` lands in the system temp directory.
    let sentinel = std::env::temp_dir().join("sbk_hardening_evil_sentinel.txt");
    let _ = fs::remove_file(&sentinel);

    {
        let f = File::create(&zip_path).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        zw.start_file(evil_name, SimpleFileOptions::default())
            .unwrap();
        zw.write_all(b"pwned").unwrap();
        zw.finish().unwrap();
    }

    let out = tmp.path().join("out.sbk");
    let result = sbk::convert::convert_to_sbk(
        &zip_path,
        &out,
        sbk::convert::ConvertFormat::Zip,
        1,
        1,
        Algorithm::Zstd,
    );

    let escaped = sentinel.exists();
    let _ = fs::remove_file(&sentinel); // clean up litter regardless of outcome

    assert!(result.is_err(), "zip entry with '..' must be rejected");
    assert!(
        !escaped,
        "zip-slip entry must not be written outside the temp dir"
    );
}

// ─── M6: zip mtimes survive conversion to SBK ────────────────────────────────

#[test]
fn convert_to_sbk_preserves_zip_mtimes() {
    use zip::write::SimpleFileOptions;

    let tmp = TempDir::new().unwrap();
    let zip_path = tmp.path().join("world.zip");

    // 2020-05-15 10:30:20 — both sides of the conversion assume UTC, and DOS
    // times have 2-second resolution, so we assert equality within 2 s.
    let dt = zip::DateTime::from_date_and_time(2020, 5, 15, 10, 30, 20).unwrap();
    let expected_unix_secs: i64 = 1_589_538_620;

    {
        let f = File::create(&zip_path).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        let opts_zip = SimpleFileOptions::default().last_modified_time(dt);
        zw.start_file("stamped.txt", opts_zip).unwrap();
        zw.write_all(b"timestamped content").unwrap();
        zw.finish().unwrap();
    }

    let out = tmp.path().join("world.sbk");
    sbk::convert::convert_to_sbk(
        &zip_path,
        &out,
        sbk::convert::ConvertFormat::Zip,
        1,
        1,
        Algorithm::Zstd,
    )
    .unwrap();

    // Read the SBK index and check the stored mtime.
    let mut f = File::open(&out).unwrap();
    let header = sbk::format::header::read_header(&mut f).unwrap();
    let codec = sbk::codec::from_algorithm(header.algorithm);
    f.seek(SeekFrom::Start(header.index_offset)).unwrap();
    let entries = sbk::format::index::read_index(
        &mut f,
        &*codec,
        header.index_compressed_size,
        header.index_checksum,
    )
    .unwrap();

    let entry = entries
        .iter()
        .find(|e| e.path == "stamped.txt")
        .expect("stamped.txt missing from index");
    let stored_secs = entry.mtime_ms / 1000;
    assert!(
        (stored_secs - expected_unix_secs).abs() <= 2,
        "mtime not preserved: stored {} s, expected {} s (±2 s DOS resolution)",
        stored_secs,
        expected_unix_secs
    );
}

// ─── M4: convert_from_sbk round trips after the streaming rewrite ────────────

fn round_trip_world(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let world = tmp.path().join("world");
    make_world(
        &world,
        &[
            ("icon.png", b"\x89PNG\r\n\x1a\nfakeicon" as &[u8]),
            ("data/a.txt", b"alpha content"),
            ("data/b.bin", &[0xAAu8; 4096]),
            ("empty.dat_ignore", b""), // zero-byte RAW file
        ],
    );
    let archive = tmp.path().join("world.sbk");
    sbk::compress::compress(&world, &opts(&archive)).unwrap();
    (world, archive)
}

#[test]
fn convert_from_sbk_to_targz_round_trip() {
    let tmp = TempDir::new().unwrap();
    let (world, archive) = round_trip_world(&tmp);

    let targz = tmp.path().join("world.tar.gz");
    let n =
        sbk::convert::convert_from_sbk(&archive, &targz, sbk::convert::ConvertFormat::TarGz, 2, 1)
            .unwrap();
    assert_eq!(n, 4);

    // Unpack and compare bytes with the original world.
    let unpacked = tmp.path().join("unpacked");
    fs::create_dir_all(&unpacked).unwrap();
    let f = File::open(&targz).unwrap();
    let dec = flate2::read::GzDecoder::new(f);
    let mut tar = tar::Archive::new(dec);
    tar.unpack(&unpacked).unwrap();

    for rel in ["icon.png", "data/a.txt", "data/b.bin", "empty.dat_ignore"] {
        let orig = fs::read(world.join(rel)).unwrap();
        let got = fs::read(unpacked.join(rel))
            .unwrap_or_else(|e| panic!("missing {} in tar.gz output: {}", rel, e));
        assert_eq!(orig, got, "content mismatch for {}", rel);
    }
}

#[test]
fn convert_from_sbk_to_zip_round_trip() {
    let tmp = TempDir::new().unwrap();
    let (world, archive) = round_trip_world(&tmp);

    let zip_path = tmp.path().join("world.zip");
    let n =
        sbk::convert::convert_from_sbk(&archive, &zip_path, sbk::convert::ConvertFormat::Zip, 2, 1)
            .unwrap();
    assert_eq!(n, 4);

    let f = File::open(&zip_path).unwrap();
    let mut za = zip::ZipArchive::new(f).unwrap();
    assert_eq!(za.len(), 4);
    for rel in ["icon.png", "data/a.txt", "data/b.bin", "empty.dat_ignore"] {
        let orig = fs::read(world.join(rel)).unwrap();
        let mut entry = za
            .by_name(rel)
            .unwrap_or_else(|e| panic!("missing {} in zip output: {}", rel, e));
        let mut got = Vec::new();
        entry.read_to_end(&mut got).unwrap();
        assert_eq!(orig, got, "content mismatch for {}", rel);
    }
}

// ─── verify: parallel verification correctness ───────────────────────────────

#[test]
fn verify_ok_then_detects_flipped_frame_byte() {
    let (_tmp, archive) = small_archive();

    assert!(
        sbk::verify::verify(&archive, 2).unwrap(),
        "pristine archive must verify"
    );

    // Locate the first frame's data and flip one byte in it.
    let frame_dir_offset = read_u64_at(&archive, 31);
    let mut f = File::open(&archive).unwrap();
    let header = sbk::format::header::read_header(&mut f).unwrap();
    f.seek(SeekFrom::Start(frame_dir_offset)).unwrap();
    let frame_dir =
        sbk::format::frame_dir::read_frame_dir(&mut f, header.frame_size_bytes).unwrap();
    let entry = frame_dir
        .groups
        .iter()
        .flat_map(|g| g.iter())
        .next()
        .expect("archive should contain at least one frame");
    let target = entry.frame_offset;
    let mut byte = [0u8; 1];
    f.seek(SeekFrom::Start(target)).unwrap();
    f.read_exact(&mut byte).unwrap();
    drop(f);
    patch(&archive, target, &[byte[0] ^ 0xFF]);

    assert!(
        !sbk::verify::verify(&archive, 2).unwrap(),
        "corrupted frame must fail verification"
    );
}
