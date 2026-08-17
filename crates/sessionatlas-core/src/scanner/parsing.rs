//! Shared parsing: RFC 3339 and Unix timestamp parsing, `~` expansion, safe
//! native absolute-path normalization, trailing-separator trimming, and the
//! recursive source-enumeration policy.
//!
//! Everything here is pure and
//! filesystem-safe: it never touches `~/.sessionatlas` and never requires a
//! tool-specific on-disk format.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

/// Process-wide lock serializing environment overrides across the scanner test
/// modules. Rust runs tests in parallel inside one binary; each scanner module
/// previously used its own private lock, so a parsing/opencode test could
/// overwrite `SESSIONATLAS_HOME`/`KIMI_CODE_HOME` while a kimi test was
/// mid-resolution, producing cross-module flaky failures.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Resolves the sessionatlas home directory. `SESSIONATLAS_HOME`, when set to
/// a non-blank value, takes precedence (used by isolated tests and portable
/// installs); otherwise the platform user home is returned. Blank overrides
/// are ignored.
pub fn home_directory() -> Option<PathBuf> {
    if let Some(override_home) = std::env::var_os("SESSIONATLAS_HOME") {
        if !override_home.to_string_lossy().trim().is_empty() {
            let value = PathBuf::from(override_home);
            return Some(std::path::absolute(&value).unwrap_or(value));
        }
    }
    dirs::home_dir()
}

/// Whether the trimmed input is a `~`, `~/...` or (on Windows) `~\...` form.
fn is_tilde_form(trimmed: &str) -> bool {
    if trimmed == "~" {
        return true;
    }
    if cfg!(windows) {
        trimmed.starts_with("~/") || trimmed.starts_with("~\\")
    } else {
        trimmed.starts_with("~/")
    }
}

/// Expands a leading `~`, `~/` or (on Windows) `~\` to the sessionatlas home.
/// Returns `None` when the input has no tilde form or the home is unavailable.
pub fn expand_tilde(candidate: &str) -> Option<String> {
    let trimmed = candidate.trim();
    if !is_tilde_form(trimmed) {
        return None;
    }
    let home = home_directory()?;
    let rest = trimmed[1..].trim_start_matches(['/', '\\']);
    Some(Path::new(&home).join(rest).to_string_lossy().into_owned())
}

/// Parses a UTC timestamp from a JSON value: RFC 3339 strings (offset-aware
/// and converted to UTC) or Unix seconds/milliseconds numbers.
pub fn try_read_utc_timestamp(value: &serde_json::Value) -> Option<DateTime<Utc>> {
    match value {
        serde_json::Value::String(text) => DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|parsed| parsed.with_timezone(&Utc)),
        serde_json::Value::Number(number) => try_read_unix_timestamp(number.as_i64()?),
        _ => None,
    }
}

/// Reads a Unix timestamp. Values with ten or more digits are milliseconds;
/// smaller values are seconds. Returns `None` for out-of-range inputs.
pub fn try_read_unix_timestamp(value: i64) -> Option<DateTime<Utc>> {
    if value.unsigned_abs() >= 100_000_000_000 {
        DateTime::from_timestamp_millis(value)
    } else {
        DateTime::from_timestamp(value, 0)
    }
}

/// Validates and normalizes a candidate project path against a source root.
///
/// Accepts `~` forms, requires an absolute native path, and rejects paths that
/// are the source root itself or inside it — a tool's own data directory is
/// never a project. Returns `None` for blank, relative, or malformed input.
pub fn try_normalize_project_path(candidate: &str, source_root: &str) -> Option<String> {
    if candidate.trim().is_empty() {
        return None;
    }
    let trimmed = candidate.trim();
    let expanded = match expand_tilde(trimmed) {
        Some(value) => value,
        None => trimmed.to_string(),
    };
    let normalized = crate::path::normalize_native(&expanded)?;
    let normalized_source = crate::path::normalize_native(source_root)?;
    if crate::path::is_same_or_child_native(&normalized, &normalized_source) {
        return None;
    }
    Some(normalized)
}

/// Trims trailing separators while preserving roots: `C:\repo\` → `C:\repo`,
/// `C:\` and `/` stay unchanged. Falls back to a lexical trim when the input
/// is not a valid native absolute path.
pub fn trim_trailing_separators(path: &str) -> String {
    match crate::path::normalize_native(path) {
        Some(normalized) => normalized,
        None => path.trim_end_matches(['/', '\\']).to_string(),
    }
}

/// Recursive enumeration policy for a tool source directory: recursion is on,
/// inaccessible entries surface as errors (never silently skipped), and
/// reparse points (symlinks/junctions) are skipped without being followed.
pub fn recursive_file_enumeration(
    path: &Path,
) -> impl Iterator<Item = Result<walkdir::DirEntry, walkdir::Error>> + '_ {
    walkdir::WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !entry.file_type().is_symlink())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn scanner_parsing_rfc3339_offsets_convert_to_utc() {
        let parsed = try_read_utc_timestamp(&json!("2026-07-30T10:00:01Z")).unwrap();
        assert_eq!(parsed.to_rfc3339(), "2026-07-30T10:00:01+00:00");

        let offset = try_read_utc_timestamp(&json!("2026-07-30T18:00:00+08:00")).unwrap();
        assert_eq!(offset.to_rfc3339(), "2026-07-30T10:00:00+00:00");
    }

    #[test]
    fn scanner_parsing_rejects_malformed_or_non_timestamp_values() {
        assert!(try_read_utc_timestamp(&json!("not-a-date")).is_none());
        assert!(try_read_utc_timestamp(&json!({"x": 1})).is_none());
        assert!(try_read_utc_timestamp(&serde_json::Value::Null).is_none());
        assert!(try_read_utc_timestamp(&json!(false)).is_none());
    }

    #[test]
    fn scanner_parsing_unix_seconds_and_milliseconds() {
        let seconds = try_read_unix_timestamp(1_000_000_000).unwrap();
        assert_eq!(seconds.to_rfc3339(), "2001-09-09T01:46:40+00:00");

        let millis = try_read_unix_timestamp(1_000_000_000_000).unwrap();
        assert_eq!(millis, seconds);

        let negative = try_read_unix_timestamp(-1_000_000_000).unwrap();
        assert_eq!(negative.to_rfc3339(), "1938-04-24T22:13:20+00:00");

        assert!(try_read_unix_timestamp(i64::MAX).is_none());
        assert!(try_read_unix_timestamp(i64::MIN).is_none());
    }

    #[test]
    fn scanner_parsing_number_timestamps_route_through_json_value() {
        let seconds = try_read_utc_timestamp(&json!(1_000_000_000)).unwrap();
        assert_eq!(seconds.to_rfc3339(), "2001-09-09T01:46:40+00:00");
        let millis = try_read_utc_timestamp(&json!(1_000_000_000_000i64)).unwrap();
        assert_eq!(millis, seconds);
    }

    #[test]
    fn scanner_parsing_normalize_project_path_rejects_blank_relative_and_source_root() {
        let source_root = if cfg!(windows) {
            r"C:\Users\me\.codex"
        } else {
            "/home/me/.codex"
        };
        let project = if cfg!(windows) {
            r"C:\Users\me\work\repo"
        } else {
            "/home/me/work/repo"
        };

        assert!(try_normalize_project_path("", source_root).is_none());
        assert!(try_normalize_project_path("   ", source_root).is_none());
        assert!(try_normalize_project_path("relative/path", source_root).is_none());

        let source_child = format!("{source_root}/sessions");
        assert!(
            try_normalize_project_path(&source_child, source_root).is_none(),
            "a tool's own data directory is never a project"
        );

        let normalized = try_normalize_project_path(project, source_root).expect("accepted");
        assert_eq!(
            normalized,
            crate::path::normalize_native(project).expect("project is absolute")
        );
    }

    #[test]
    fn scanner_parsing_normalize_project_path_accepts_and_normalizes_sibling_paths() {
        let source_root = if cfg!(windows) {
            r"C:\Users\me\.codex"
        } else {
            "/home/me/.codex"
        };
        let project = if cfg!(windows) {
            r"C:\Users\me\work\repo"
        } else {
            "/home/me/work/repo"
        };

        let with_trailing = format!("{project}/");
        let normalized = try_normalize_project_path(&with_trailing, source_root).unwrap();
        assert_eq!(
            normalized,
            crate::path::normalize_native(project).unwrap(),
            "trailing separators are trimmed and dotted segments resolved"
        );
    }

    #[test]
    fn scanner_parsing_trailing_separators_trimmed_except_root() {
        match crate::path::PathFlavor::native() {
            crate::path::PathFlavor::Windows => {
                assert_eq!(trim_trailing_separators(r"C:\"), r"C:\");
                assert_eq!(trim_trailing_separators(r"C:\repo\"), r"C:\repo");
                assert_eq!(trim_trailing_separators(r"C:\repo\\\"), r"C:\repo");
            }
            crate::path::PathFlavor::Unix => {
                assert_eq!(trim_trailing_separators("/"), "/");
                assert_eq!(trim_trailing_separators("/repo/"), "/repo");
                assert_eq!(trim_trailing_separators("/repo///"), "/repo");
            }
        }
        assert_eq!(trim_trailing_separators("relative/"), "relative");
    }

    #[test]
    fn scanner_parsing_expand_tilde_uses_sessionatlas_home() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("SESSIONATLAS_HOME");
        std::env::set_var("SESSIONATLAS_HOME", dir.path());
        let expanded = expand_tilde("~/repo").expect("tilde expansion");
        let bare = expand_tilde("~").expect("bare tilde expansion");
        let untouched = expand_tilde("/absolute/path");
        match previous {
            Some(value) => std::env::set_var("SESSIONATLAS_HOME", value),
            None => std::env::remove_var("SESSIONATLAS_HOME"),
        }

        assert_eq!(Path::new(&expanded), dir.path().join("repo"));
        assert_eq!(Path::new(&bare), dir.path());
        assert_eq!(untouched, None);
    }

    #[test]
    fn scanner_parsing_ignores_whitespace_only_sessionatlas_home() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let previous = std::env::var_os("SESSIONATLAS_HOME");
        std::env::set_var("SESSIONATLAS_HOME", " \t\n");

        let resolved = home_directory();

        match previous {
            Some(value) => std::env::set_var("SESSIONATLAS_HOME", value),
            None => std::env::remove_var("SESSIONATLAS_HOME"),
        }

        assert_eq!(resolved, dirs::home_dir());
    }

    #[test]
    fn scanner_parsing_normalize_project_path_expands_tilde() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("SESSIONATLAS_HOME");
        std::env::set_var("SESSIONATLAS_HOME", dir.path());
        let source_root = if cfg!(windows) {
            r"C:\Users\me\.other-tool"
        } else {
            "/home/me/.other-tool"
        };
        let result = try_normalize_project_path("~/work/repo", source_root);
        match previous {
            Some(value) => std::env::set_var("SESSIONATLAS_HOME", value),
            None => std::env::remove_var("SESSIONATLAS_HOME"),
        }

        let expected_path = dir.path().join("work").join("repo");
        let expected = crate::path::normalize_native(expected_path.to_str().unwrap()).unwrap();
        assert_eq!(result, Some(expected));
    }

    #[test]
    fn scanner_parsing_recursive_enumeration_descends_and_skips_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("source");
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/top.jsonl"), "{}").unwrap();
        std::fs::write(root.join("a/b/deep.jsonl"), "{}").unwrap();

        let target = dir.path().join("real");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("inside.jsonl"), "{}").unwrap();
        let link = root.join("link");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();

        #[cfg(windows)]
        let link_created = std::os::windows::fs::symlink_dir(&target, &link).is_ok();

        let files: Vec<_> = recursive_file_enumeration(&root)
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| entry.path().to_path_buf())
            .collect();

        #[cfg(unix)]
        {
            assert_eq!(
                files.len(),
                2,
                "symlinked directory must not be descended into"
            );
        }
        #[cfg(windows)]
        if link_created {
            assert_eq!(
                files.len(),
                2,
                "symlinked directory must not be descended into"
            );
        } else {
            assert_eq!(files.len(), 2, "no link was created; plain recursion holds");
        }

        assert!(files.iter().any(|path| path.ends_with("a/top.jsonl")));
        assert!(files.iter().any(|path| path.ends_with("a/b/deep.jsonl")));
    }

    #[test]
    fn scanner_parsing_recursive_enumeration_surfaces_errors() {
        let dir = tempfile::tempdir().unwrap();
        let errors: Vec<_> = recursive_file_enumeration(&dir.path().join("missing"))
            .filter_map(Result::err)
            .collect();
        assert!(
            !errors.is_empty(),
            "an unreadable root is surfaced as an error, never a silent empty list"
        );
    }
}
