//! Public-API tests for the R04 scanner framework.
//!
//! These tests use only synthetic temporary data and never touch the real
//! `~/.sessionatlas` or the real user home: every home-dependent case sets a
//! temporary `SESSIONATLAS_HOME` and restores it afterward.

use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use sessionatlas_core::path::{self, PathFlavor};
use sessionatlas_core::scanner::{
    complete_session_files, expand_tilde, home_directory, missing_source, probe_directory,
    probe_file, recursive_file_enumeration, source_read_failure, trim_trailing_separators,
    try_normalize_project_path, try_read_unix_timestamp, try_read_utc_timestamp, ScanDiagnostic,
    ScanDiagnosticSeverity, ScanOutcome, ScanStatus, ScannedProject, Scanner, SourceProbe,
    NO_VALID_SESSIONS, SOURCE_READ_FAILED, SOURCE_UNAVAILABLE,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Sets a temporary `SESSIONATLAS_HOME`, runs `body`, then restores the
/// previous value even on panic.
fn with_home<R>(path: &Path, body: impl FnOnce() -> R) -> R {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let previous = std::env::var_os("SESSIONATLAS_HOME");
    std::env::set_var("SESSIONATLAS_HOME", path);
    struct Restore(Option<std::ffi::OsString>);
    impl Drop for Restore {
        fn drop(&mut self) {
            match &self.0 {
                Some(value) => std::env::set_var("SESSIONATLAS_HOME", value),
                None => std::env::remove_var("SESSIONATLAS_HOME"),
            }
        }
    }
    let _restore = Restore(previous);
    body()
}

struct MinimalScanner {
    key: &'static str,
    name: &'static str,
    available: bool,
    outcome: ScanOutcome,
}

impl Scanner for MinimalScanner {
    fn tool_key(&self) -> &str {
        self.key
    }
    fn tool_name(&self) -> &str {
        self.name
    }
    fn is_available(&self) -> bool {
        self.available
    }
    fn scan(&self) -> ScanOutcome {
        self.outcome.clone()
    }
}

#[test]
fn scanner_framework_missing_source_availability_is_separate_from_discovery() {
    let unavailable = MinimalScanner {
        key: "codex",
        name: "Codex",
        available: false,
        outcome: missing_source("codex", false),
    };
    assert!(!unavailable.is_available());
    let outcome = unavailable.scan();
    assert_eq!(outcome.status(), ScanStatus::Unavailable);
    assert!(outcome.projects().is_empty());
    assert_eq!(outcome.diagnostics()[0].code, SOURCE_UNAVAILABLE);

    let empty_success = MinimalScanner {
        key: "codex",
        name: "Codex",
        available: true,
        outcome: missing_source("codex", true),
    };
    assert!(empty_success.is_available());
    let outcome = empty_success.scan();
    assert_eq!(outcome.status(), ScanStatus::Succeeded);
    assert!(outcome.is_successful());
    assert!(outcome.projects().is_empty());
}

#[test]
fn scanner_framework_only_successful_inspection_yields_empty_replacement_snapshot() {
    let source_read_failed = source_read_failure("kimi", "~/.kimi-code/sessions");
    assert_eq!(source_read_failed.status(), ScanStatus::Failed);
    assert!(!source_read_failed.is_successful());
    assert_eq!(source_read_failed.diagnostics()[0].code, SOURCE_READ_FAILED);

    let no_valid = complete_session_files("codex", 4, vec![], vec![]);
    assert_eq!(no_valid.status(), ScanStatus::Failed);
    assert_eq!(no_valid.diagnostics()[0].code, NO_VALID_SESSIONS);

    let inspected_empty = complete_session_files("codex", 0, vec![], vec![]);
    assert_eq!(inspected_empty.status(), ScanStatus::Succeeded);
    assert!(inspected_empty.is_successful());
    assert!(inspected_empty.projects().is_empty());
}

#[test]
fn scanner_framework_full_scan_flow_yields_success_with_projects() {
    let projects = vec![ScannedProject {
        path: path::normalize_native("/repo")
            .expect("absolute")
            .to_string(),
        last_accessed_at: Utc::now(),
        session_id: Some("session-demo".to_string()),
        git_branch: Some("main".to_string()),
    }];
    let outcome = complete_session_files("codex", 1, projects, vec![]);
    assert_eq!(outcome.status(), ScanStatus::Succeeded);
    assert_eq!(outcome.projects().len(), 1);
    assert_eq!(
        outcome.projects()[0].session_id.as_deref(),
        Some("session-demo")
    );
    assert_eq!(outcome.projects()[0].git_branch.as_deref(), Some("main"));
}

#[test]
fn scanner_framework_diagnostic_carries_tool_severity_code_and_actionable_message() {
    let diagnostic = ScanDiagnostic::new(
        "claude",
        ScanDiagnosticSeverity::Warning,
        "timestamp_fallback",
        "A timestamp was malformed and the filesystem modification time was used.",
    );
    assert_eq!(diagnostic.tool_key, "claude");
    assert_eq!(diagnostic.severity, ScanDiagnosticSeverity::Warning);
    assert_eq!(diagnostic.code, "timestamp_fallback");
    assert!(!diagnostic.message.is_empty());
}

#[test]
fn scanner_framework_probe_matches_csharp_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let file = dir.path().join("marker");
    std::fs::write(&file, b"x").unwrap();

    assert_eq!(probe_directory(&source), SourceProbe::Exists);
    assert_eq!(
        probe_directory(&dir.path().join("nope")),
        SourceProbe::Missing
    );
    assert_eq!(probe_directory(&file), SourceProbe::Failed);

    assert_eq!(probe_file(&file), SourceProbe::Exists);
    assert_eq!(probe_file(&dir.path().join("nope")), SourceProbe::Missing);
    assert_eq!(probe_file(&source), SourceProbe::Failed);
}

#[test]
fn scanner_framework_rfc3339_and_unix_timestamps_parse_to_utc() {
    let rfc3339: DateTime<Utc> =
        try_read_utc_timestamp(&serde_json::json!("2026-07-30T18:00:00+08:00")).unwrap();
    assert_eq!(rfc3339.to_rfc3339(), "2026-07-30T10:00:00+00:00");

    let seconds = try_read_unix_timestamp(1_000_000_000).unwrap();
    let millis = try_read_unix_timestamp(1_000_000_000_000).unwrap();
    assert_eq!(seconds, millis);
    assert_eq!(seconds.to_rfc3339(), "2001-09-09T01:46:40+00:00");

    assert!(try_read_utc_timestamp(&serde_json::json!("garbage")).is_none());
    assert!(try_read_unix_timestamp(i64::MAX).is_none());
}

#[test]
fn scanner_framework_project_path_normalization_is_safe_and_native() {
    let flavor = PathFlavor::native();
    let (source_root, project) = match flavor {
        PathFlavor::Windows => (r"C:\Users\me\.codex", r"C:\Users\me\work\repo"),
        PathFlavor::Unix => ("/home/me/.codex", "/home/me/work/repo"),
    };

    assert!(try_normalize_project_path("", source_root).is_none());
    assert!(try_normalize_project_path("relative", source_root).is_none());

    let source_child = format!("{source_root}{}/sessions", flavor.separator());
    assert!(try_normalize_project_path(&source_child, source_root).is_none());

    let trailing = format!("{project}{}", flavor.separator());
    let normalized = try_normalize_project_path(&trailing, source_root).unwrap();
    assert_eq!(normalized, path::normalize_native(project).unwrap());
}

#[test]
fn scanner_framework_trailing_separators_never_destroy_roots() {
    match PathFlavor::native() {
        PathFlavor::Windows => {
            assert_eq!(trim_trailing_separators(r"C:\"), r"C:\");
            assert_eq!(trim_trailing_separators(r"C:\repo\"), r"C:\repo");
        }
        PathFlavor::Unix => {
            assert_eq!(trim_trailing_separators("/"), "/");
            assert_eq!(trim_trailing_separators("/repo/"), "/repo");
        }
    }
}

#[test]
fn scanner_framework_tilde_expansion_uses_sessionatlas_home_override() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        assert_eq!(
            Path::new(&expand_tilde("~/repo").unwrap()),
            dir.path().join("repo")
        );
        assert_eq!(home_directory().as_deref(), Some(dir.path()));
        assert_eq!(expand_tilde("/not/tilde"), None);
    });
}

#[test]
fn scanner_framework_whitespace_only_home_override_is_ignored() {
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
fn scanner_framework_normalize_project_path_expands_tilde_under_temp_home() {
    let dir = tempfile::tempdir().unwrap();
    let flavor = PathFlavor::native();
    let source_root = match flavor {
        PathFlavor::Windows => r"C:\Users\me\.other-tool",
        PathFlavor::Unix => "/home/me/.other-tool",
    };
    let expected = dir.path().join("work").join("repo");
    with_home(dir.path(), || {
        let result = try_normalize_project_path("~/work/repo", source_root);
        assert_eq!(
            result,
            Some(path::normalize_native(expected.to_str().unwrap()).unwrap())
        );
    });
}

#[test]
fn scanner_framework_recursive_enumeration_uses_synthetic_data_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("source");
    std::fs::create_dir_all(root.join("nested/deep")).unwrap();
    std::fs::write(root.join("one.jsonl"), "{}").unwrap();
    std::fs::write(root.join("nested/deep/two.jsonl"), "{}").unwrap();

    let found: Vec<_> = recursive_file_enumeration(&root)
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
        .collect();
    assert_eq!(found.len(), 2);
}
