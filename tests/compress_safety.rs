//! Safety tests for the compression pipeline:
//! - preprocess failures must fall back to the RAW group (byte-identical round trip)
//! - output is written atomically (pre-existing archives survive failed runs)
//! - the output archive is never included in itself
//! - symlinks are skipped without breaking the run

use std::fs;
use std::io::{Seek, SeekFrom};
use std::path::Path;

use sbk::filter::{CompressOptions, FilterMode};
use sbk::format::header::Algorithm;
use sbk::format::index::IndexEntry;
use tempfile::tempdir;

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

fn read_archive_index(archive: &Path) -> Vec<IndexEntry> {
    let mut f = fs::File::open(archive).unwrap();
    let header = sbk::format::header::read_header(&mut f).unwrap();
    let codec = sbk::codec::from_algorithm(header.algorithm);
    f.seek(SeekFrom::Start(header.index_offset)).unwrap();
    sbk::format::index::read_index(
        &mut f,
        &*codec,
        header.index_compressed_size,
        header.index_checksum,
    )
    .unwrap()
}

/// Compress `world`, decompress into a fresh dir, return the restored bytes of
/// `rel` and the group_id recorded for it in the archive index.
fn round_trip(world: &Path, rel: &str) -> (Vec<u8>, u8) {
    let out_dir = tempdir().unwrap();
    let archive = out_dir.path().join("test.sbk");
    sbk::compress::compress(world, &opts(&archive)).unwrap();

    let entries = read_archive_index(&archive);
    let entry = entries
        .iter()
        .find(|e| e.path == rel)
        .unwrap_or_else(|| panic!("{} missing from archive index", rel));
    let group_id = entry.group_id;

    let restore = tempdir().unwrap();
    sbk::decompress::decompress(&archive, restore.path(), 2).unwrap();
    let restored = fs::read(restore.path().join(rel)).unwrap();
    (restored, group_id)
}

// ─── C1: preprocess failures fall back to RAW ───────────────────────────────

#[test]
fn non_gzip_dat_stored_raw_and_round_trips() {
    let world = tempdir().unwrap();
    let original = b"plain nbt not gzipped".to_vec();
    fs::write(world.path().join("level.dat"), &original).unwrap();

    let (restored, group_id) = round_trip(world.path(), "level.dat");
    assert_eq!(restored, original, "non-gzip .dat must round-trip verbatim");
    assert_eq!(group_id, 3, "failed NBT preprocess must reassign to RAW group");
}

#[test]
fn zero_byte_dat_round_trips() {
    let world = tempdir().unwrap();
    fs::write(world.path().join("empty.dat"), b"").unwrap();
    // Companion file so the archive has at least one data frame. (Extraction of
    // archives with zero frames never reaches the write phase — a pre-existing
    // extract.rs limitation independent of this test's subject.)
    fs::write(world.path().join("other.bin"), b"padding").unwrap();

    let (restored, _group_id) = round_trip(world.path(), "empty.dat");
    assert_eq!(restored, b"", "zero-byte .dat must round-trip byte-identical");
}

#[test]
fn invalid_json_stored_raw_and_round_trips() {
    let world = tempdir().unwrap();
    let original = b"not json {{{".to_vec();
    fs::write(world.path().join("bad.json"), &original).unwrap();

    let (restored, group_id) = round_trip(world.path(), "bad.json");
    assert_eq!(restored, original, "invalid .json must round-trip verbatim");
    assert_eq!(group_id, 3, "failed JSON preprocess must reassign to RAW group");
}

#[test]
fn corrupt_mca_stored_raw_and_round_trips() {
    let world = tempdir().unwrap();
    fs::create_dir_all(world.path().join("region")).unwrap();
    // 9000 bytes of 0xAB: the location table points at sectors far outside the
    // file, so preprocessing must fail and the file must be stored raw.
    let original = vec![0xABu8; 9000];
    fs::write(world.path().join("region/r.0.0.mca"), &original).unwrap();

    let (restored, group_id) = round_trip(world.path(), "region/r.0.0.mca");
    assert_eq!(restored, original, "corrupt .mca must round-trip verbatim");
    assert_eq!(group_id, 3, "failed MCA preprocess must reassign to RAW group");
}

// Depends on preprocess_mca_from_bytes returning Err for inputs < 8192 bytes
// (owned by another change); with that in place the file is deferred to RAW.
#[test]
fn tiny_mca_round_trips() {
    let world = tempdir().unwrap();
    fs::create_dir_all(world.path().join("region")).unwrap();
    let original: Vec<u8> = (0..100u32).map(|i| i as u8).collect();
    fs::write(world.path().join("region/r.9.9.mca"), &original).unwrap();

    let (restored, _group_id) = round_trip(world.path(), "region/r.9.9.mca");
    assert_eq!(restored, original, "100-byte .mca must round-trip byte-identical");
}

// ─── H5: atomic output ───────────────────────────────────────────────────────

#[test]
fn successful_compress_replaces_existing_output_without_leftovers() {
    let world = tempdir().unwrap();
    fs::write(world.path().join("data.bin"), b"hello world").unwrap();

    let out_dir = tempdir().unwrap();
    let archive = out_dir.path().join("backup.sbk");
    fs::write(&archive, b"OLD JUNK ARCHIVE").unwrap();

    sbk::compress::compress(world.path(), &opts(&archive)).unwrap();

    // The output must now be a valid archive containing the file.
    let entries = read_archive_index(&archive);
    assert!(entries.iter().any(|e| e.path == "data.bin"));

    // No temp files may remain next to the output.
    let leftovers: Vec<String> = fs::read_dir(out_dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != "backup.sbk")
        .collect();
    assert!(leftovers.is_empty(), "leftover temp files: {:?}", leftovers);
}

#[cfg(unix)]
#[test]
fn failed_compress_leaves_existing_output_untouched() {
    use std::os::unix::fs::PermissionsExt;

    let world = tempdir().unwrap();
    fs::write(world.path().join("data.bin"), b"hello world").unwrap();

    let out_dir = tempdir().unwrap();
    let archive = out_dir.path().join("backup.sbk");
    let junk = b"PRECIOUS PRE-EXISTING ARCHIVE".to_vec();
    fs::write(&archive, &junk).unwrap();

    // Make the output's parent directory read-only: the temp file cannot be
    // created, so compression must fail without touching the existing output.
    fs::set_permissions(out_dir.path(), fs::Permissions::from_mode(0o555)).unwrap();

    let result = sbk::compress::compress(world.path(), &opts(&archive));

    // Restore permissions before asserting so the tempdir can be cleaned up.
    fs::set_permissions(out_dir.path(), fs::Permissions::from_mode(0o755)).unwrap();

    assert!(result.is_err(), "compress must fail when no temp file can be created");
    assert_eq!(
        fs::read(&archive).unwrap(),
        junk,
        "pre-existing output must be untouched by a failed compress"
    );
}

#[cfg(unix)]
#[test]
fn output_permissions_follow_umask() {
    use std::os::unix::fs::PermissionsExt;

    let world = tempdir().unwrap();
    fs::write(world.path().join("data.bin"), b"hello world").unwrap();

    let out_dir = tempdir().unwrap();
    let archive = out_dir.path().join("backup.sbk");
    sbk::compress::compress(world.path(), &opts(&archive)).unwrap();

    // A plainly-created file in the same directory shows what the umask allows.
    // The archive must not be more restrictive than that (NamedTempFile creates
    // 0600 files; persisting one verbatim would make backups unreadable to
    // group/other even when the umask permits it).
    let control = out_dir.path().join("control");
    fs::write(&control, b"x").unwrap();
    let control_mode = fs::metadata(&control).unwrap().permissions().mode() & 0o777;
    let archive_mode = fs::metadata(&archive).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        archive_mode, control_mode,
        "archive permissions {:o} must match umask-derived {:o}",
        archive_mode, control_mode
    );
}

// ─── M1: never include the output archive in itself ─────────────────────────

#[test]
fn output_inside_world_dir_is_not_archived() {
    let world = tempdir().unwrap();
    fs::write(world.path().join("data.bin"), b"hello world").unwrap();
    let archive = world.path().join("backup.sbk");
    fs::write(&archive, b"OLD ARCHIVE FROM PREVIOUS RUN").unwrap();

    sbk::compress::compress(world.path(), &opts(&archive)).unwrap();

    let entries = read_archive_index(&archive);
    assert!(
        !entries.iter().any(|e| e.path == "backup.sbk"),
        "output archive must not be included in itself"
    );
    assert!(entries.iter().any(|e| e.path == "data.bin"));
}

// ─── M5: symlinks are skipped without aborting ───────────────────────────────

#[cfg(unix)]
#[test]
fn symlink_is_skipped() {
    let world = tempdir().unwrap();
    fs::write(world.path().join("target.dat"), b"").unwrap();
    fs::write(world.path().join("real.bin"), b"real data").unwrap();
    std::os::unix::fs::symlink(world.path().join("real.bin"), world.path().join("link.dat"))
        .unwrap();

    let out_dir = tempdir().unwrap();
    let archive = out_dir.path().join("test.sbk");
    sbk::compress::compress(world.path(), &opts(&archive)).unwrap();

    let entries = read_archive_index(&archive);
    assert!(
        !entries.iter().any(|e| e.path == "link.dat"),
        "symlinks must not be archived"
    );
    assert!(entries.iter().any(|e| e.path == "real.bin"));
}
