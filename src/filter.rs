use std::path::PathBuf;

use crate::format::header::Algorithm;

pub struct CompressOptions {
    pub output: PathBuf,
    pub threads: usize,
    pub level: u32,           // compression level 1–9
    pub algorithm: Algorithm, // compression algorithm
    pub max_age: Option<u64>, // milliseconds; None = no relative age filter
    pub since: Option<i64>,   // millisecond Unix timestamp; None = no absolute filter
    pub patterns: FilterMode,
    pub include_session_lock: bool, // if true, session.lock is not excluded
    pub quiet: bool,                // if true, suppress all stdout output
}

pub enum FilterMode {
    None,
    Exclude(Vec<glob::Pattern>), // skip matching files
    Include(Vec<glob::Pattern>), // skip non-matching files
}

/// Call once before walkdir traversal. Returns current time in ms since Unix epoch.
pub fn capture_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// Returns true if this file should be included in the archive.
/// rel_path:    forward-slash-separated relative path within the world directory.
/// mtime_ms:    file's last-modified time in milliseconds since Unix epoch.
/// now_ms:      current time captured once at startup (used for --max-age).
/// max_age_ms:  --max-age value in milliseconds; None = no relative filter.
/// since_ms:    --since value as a millisecond Unix timestamp; None = no absolute filter.
pub fn accept(
    rel_path: &str,
    mtime_ms: i64,
    now_ms: i64,
    max_age_ms: Option<u64>,
    since_ms: Option<i64>,
    filter: &FilterMode,
    include_session_lock: bool,
) -> bool {
    // 1. Hardcoded skip (overridable)
    if !include_session_lock && rel_path == "session.lock" {
        return false;
    }

    // 2. Relative age filter: file must be newer than (now - max_age_ms)
    if let Some(age) = max_age_ms {
        if mtime_ms < now_ms - age as i64 {
            return false;
        }
    }

    // 3. Absolute timestamp filter: file mtime must be >= since_ms
    if let Some(since) = since_ms {
        if mtime_ms < since {
            return false;
        }
    }

    // 4. Pattern filter
    match filter {
        FilterMode::None => true,
        FilterMode::Exclude(patterns) => !patterns.iter().any(|p| p.matches(rel_path)),
        FilterMode::Include(patterns) => patterns.iter().any(|p| p.matches(rel_path)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_lock_excluded_by_default() {
        assert!(!accept(
            "session.lock",
            i64::MAX,
            0,
            None,
            None,
            &FilterMode::None,
            false
        ));
    }

    #[test]
    fn session_lock_included_when_flag_set() {
        assert!(accept(
            "session.lock",
            i64::MAX,
            0,
            None,
            None,
            &FilterMode::None,
            true
        ));
    }

    #[test]
    fn max_age_boundary() {
        // now_ms = 1_000_000, max_age = 100 ms → cutoff = 999_900
        assert!(!accept(
            "level.dat",
            999_899,
            1_000_000,
            Some(100),
            None,
            &FilterMode::None,
            false
        ));
        assert!(accept(
            "level.dat",
            999_900,
            1_000_000,
            Some(100),
            None,
            &FilterMode::None,
            false
        ));
        assert!(accept(
            "level.dat",
            1_000_000,
            1_000_000,
            Some(100),
            None,
            &FilterMode::None,
            false
        ));
    }

    #[test]
    fn since_boundary() {
        assert!(!accept(
            "level.dat",
            999_999,
            0,
            None,
            Some(1_000_000),
            &FilterMode::None,
            false
        ));
        assert!(accept(
            "level.dat",
            1_000_000,
            0,
            None,
            Some(1_000_000),
            &FilterMode::None,
            false
        ));
    }

    #[test]
    fn max_age_and_since_both_active() {
        // now=2_000_000, max_age=500_000 → relative cutoff=1_500_000
        // since=1_600_000 → effective cutoff is max(1_500_000, 1_600_000) = 1_600_000
        let now = 2_000_000i64;
        let age = Some(500_000u64);
        let since = Some(1_600_000i64);
        assert!(!accept(
            "f",
            1_599_999,
            now,
            age,
            since,
            &FilterMode::None,
            false
        ));
        assert!(accept(
            "f",
            1_600_000,
            now,
            age,
            since,
            &FilterMode::None,
            false
        ));
    }

    #[test]
    fn include_filter() {
        let p = glob::Pattern::new("region/*.mca").unwrap();
        let f = FilterMode::Include(vec![p]);
        assert!(accept("region/r.0.0.mca", 0, 0, None, None, &f, false));
        assert!(!accept("level.dat", 0, 0, None, None, &f, false));
    }

    #[test]
    fn exclude_filter() {
        let p = glob::Pattern::new("DIM-1/**").unwrap();
        let f = FilterMode::Exclude(vec![p]);
        assert!(!accept(
            "DIM-1/region/r.0.0.mca",
            0,
            0,
            None,
            None,
            &f,
            false
        ));
        assert!(accept("region/r.0.0.mca", 0, 0, None, None, &f, false));
    }
}
