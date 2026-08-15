//! Public-API tests for the R05C Aider and custom-tool scanners.
//!
//! These tests use only synthetic temporary data: every home-dependent case
//! sets a temporary `SESSIONATLAS_HOME` and restores it afterward. They never
//! touch the real `~/.sessionatlas`, the real user home, or the system temp,
//! and never launch a real AI CLI (availability predicates are injected).

use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use sessionatlas_core::model::ToolSource;
use sessionatlas_core::path;
use sessionatlas_core::scanner::aider::AiderScanner;
use sessionatlas_core::scanner::custom::CustomToolScanner;
use sessionatlas_core::scanner::{
    ScanDiagnosticSeverity, ScanStatus, Scanner, MALFORMED_SESSION_RECORD, SOURCE_READ_FAILED,
    SOURCE_UNAVAILABLE,
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

fn utc(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .unwrap()
        .with_timezone(&Utc)
}

fn set_mtime(path: &Path, time: DateTime<Utc>) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(time.into()).unwrap();
}

fn dir_mtime(path: &Path) -> DateTime<Utc> {
    DateTime::<Utc>::from(std::fs::metadata(path).unwrap().modified().unwrap())
}

/// Normalized form of an absolute path, matching what the scanners emit.
fn normalized(path: &Path) -> String {
    path::normalize_native(&path.to_string_lossy()).expect("absolute fixture path")
}

fn fixture_tool(data_directory: &str) -> ToolSource {
    ToolSource {
        key: "fixture".to_string(),
        name: "Fixture".to_string(),
        cli_command: "fixture-cli".to_string(),
        data_directory: data_directory.to_string(),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Aider
// ---------------------------------------------------------------------------

#[test]
fn aider_discovers_history_marker_from_metadata_only() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let project_dir = home.path().join("projects").join("aider-project");
        std::fs::create_dir_all(&project_dir).unwrap();
        let marker = project_dir.join(".aider.chat.history");
        std::fs::write(&marker, "user: arbitrary conversation content").unwrap();
        let modified = utc("2026-07-30T13:00:00Z");
        set_mtime(&marker, modified);

        let outcome = AiderScanner::with_availability(|| false).scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(outcome.projects()[0].path, normalized(&project_dir));
        assert_eq!(outcome.projects()[0].last_accessed_at, modified);
        assert_eq!(outcome.projects()[0].session_id, None);
        assert!(outcome.diagnostics().is_empty());
    });
}

#[test]
fn aider_never_reads_marker_content() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let project_dir = home.path().join("dev").join("no-content-repo");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(
            project_dir.join(".aider.chat.history"),
            "user: secret-sentinel {not json}\nassistant: secret-sentinel again\n",
        )
        .unwrap();

        let outcome = AiderScanner::with_availability(|| false).scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert!(outcome.diagnostics().is_empty());
    });
}

#[test]
fn aider_recurses_and_matches_only_exact_marker_name() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let deep = home
            .path()
            .join("work")
            .join("repo")
            .join("nested")
            .join("deeper");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join(".aider.chat.history"), "marker").unwrap();

        let other = home.path().join("work").join("other-repo");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("prefix.aider.chat.history"), "not a marker").unwrap();
        std::fs::write(other.join("notes.txt"), "not a marker either").unwrap();

        let outcome = AiderScanner::with_availability(|| false).scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(outcome.projects()[0].path, normalized(&deep));
        assert_eq!(outcome.projects()[0].session_id, None);
    });
}

#[test]
fn aider_discovers_across_multiple_search_roots() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        for relative in ["work/alpha", "projects/beta", "src/gamma"] {
            let directory = home.path().join(relative);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(".aider.chat.history"), "marker").unwrap();
        }

        let outcome = AiderScanner::with_availability(|| false).scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 3);
        for relative in ["work/alpha", "projects/beta", "src/gamma"] {
            let expected = normalized(&home.path().join(relative));
            assert!(
                outcome
                    .projects()
                    .iter()
                    .any(|project| project.path == expected),
                "expected {expected} among discovered projects"
            );
        }
    });
}

#[test]
fn aider_no_search_roots_not_available_is_unavailable() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let outcome = AiderScanner::with_availability(|| false).scan();

        assert_eq!(outcome.status(), ScanStatus::Unavailable);
        assert!(outcome.projects().is_empty());
        assert_eq!(outcome.diagnostics()[0].code, SOURCE_UNAVAILABLE);
    });
}

#[test]
fn aider_no_search_roots_installed_tool_is_empty_success() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let outcome = AiderScanner::with_availability(|| true).scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(outcome.is_successful());
        assert!(outcome.projects().is_empty());
    });
}

#[test]
fn aider_file_where_search_root_is_expected_is_failure() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        std::fs::write(home.path().join("work"), "not a directory").unwrap();

        let outcome = AiderScanner::with_availability(|| true).scan();

        assert_eq!(outcome.status(), ScanStatus::Failed);
        assert!(outcome.projects().is_empty());
        assert_eq!(outcome.diagnostics()[0].code, SOURCE_READ_FAILED);
    });
}

#[test]
fn aider_availability_is_separate_from_data_discoverability() {
    let available = AiderScanner::with_availability(|| true);
    let unavailable = AiderScanner::with_availability(|| false);
    assert!(available.is_available());
    assert!(!unavailable.is_available());
}

// ---------------------------------------------------------------------------
// Custom tools
// ---------------------------------------------------------------------------

#[test]
fn custom_reads_metadata_project_path_timestamp_and_id() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let data = home.path().join("custom-data");
        let child = data.join("project-a");
        let real_project = home.path().join("real-project");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(
            child.join("metadata.json"),
            format!(
                r#"{{"project_path": "{}", "last_accessed": "2026-07-30T14:00:00Z", "id": "session-custom-demo"}}"#,
                real_project.to_string_lossy().replace('\\', "\\\\")
            ),
        )
        .unwrap();

        let scanner =
            CustomToolScanner::with_availability(fixture_tool(&data.to_string_lossy()), || false);
        let outcome = scanner.scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(outcome.projects()[0].path, normalized(&real_project));
        assert_eq!(
            outcome.projects()[0].last_accessed_at,
            utc("2026-07-30T14:00:00Z")
        );
        assert_eq!(
            outcome.projects()[0].session_id.as_deref(),
            Some("session-custom-demo")
        );
    });
}

#[test]
fn custom_utf8_bom_metadata_json_is_accepted() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let data = home.path().join("custom-data");
        let child = data.join("project-bom");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(
            child.join("metadata.json"),
            b"\xef\xbb\xbf{\"id\":\"custom-bom\"}",
        )
        .unwrap();

        let scanner =
            CustomToolScanner::with_availability(fixture_tool(&data.to_string_lossy()), || false);
        let outcome = scanner.scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(
            outcome.projects()[0].session_id.as_deref(),
            Some("custom-bom")
        );
    });
}

#[test]
fn custom_falls_back_to_cwd_and_directory_metadata() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let data = home.path().join("custom-data");
        let with_cwd = data.join("project-b");
        let plain = data.join("project-c");
        let cwd_target = home.path().join("via-cwd");
        std::fs::create_dir_all(&with_cwd).unwrap();
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(
            with_cwd.join("metadata.json"),
            format!(
                r#"{{"cwd": "{}"}}"#,
                cwd_target.to_string_lossy().replace('\\', "\\\\")
            ),
        )
        .unwrap();

        let scanner =
            CustomToolScanner::with_availability(fixture_tool(&data.to_string_lossy()), || false);
        let outcome = scanner.scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 2);

        let cwd_project = outcome
            .projects()
            .iter()
            .find(|project| project.path == normalized(&cwd_target))
            .expect("cwd-derived project");
        assert_eq!(cwd_project.session_id, None);

        let plain_project = outcome
            .projects()
            .iter()
            .find(|project| project.path == normalized(&plain))
            .expect("directory-metadata project");
        assert_eq!(plain_project.last_accessed_at, dir_mtime(&plain));
        assert_eq!(plain_project.session_id, None);
    });
}

#[test]
fn custom_expands_tilde_in_data_directory() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let data = home.path().join("custom-data");
        let child = data.join("project-a");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(child.join("metadata.json"), r#"{"id": "tilde-session"}"#).unwrap();

        let scanner = CustomToolScanner::with_availability(fixture_tool("~/custom-data"), || false);
        let outcome = scanner.scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(outcome.projects()[0].path, normalized(&child));
        assert_eq!(
            outcome.projects()[0].session_id.as_deref(),
            Some("tilde-session")
        );
    });
}

#[test]
fn custom_expands_tilde_in_project_path() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let data = home.path().join("custom-data");
        let child = data.join("project-a");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(
            child.join("metadata.json"),
            r#"{"project_path": "~/tilde-project"}"#,
        )
        .unwrap();

        let scanner =
            CustomToolScanner::with_availability(fixture_tool(&data.to_string_lossy()), || false);
        let outcome = scanner.scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(
            outcome.projects()[0].path,
            normalized(&home.path().join("tilde-project"))
        );
    });
}

#[test]
fn custom_accepts_session_id_key_as_id_fallback() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let data = home.path().join("custom-data");
        let child = data.join("project-a");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(
            child.join("metadata.json"),
            r#"{"session_id": "session-via-alias"}"#,
        )
        .unwrap();

        let scanner =
            CustomToolScanner::with_availability(fixture_tool(&data.to_string_lossy()), || false);
        let outcome = scanner.scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(
            outcome.projects()[0].session_id.as_deref(),
            Some("session-via-alias")
        );
    });
}

#[test]
fn custom_malformed_metadata_json_degrades_with_diagnostic() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let data = home.path().join("custom-data");
        let child = data.join("project-broken");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(child.join("metadata.json"), "{not-valid-json").unwrap();

        let scanner =
            CustomToolScanner::with_availability(fixture_tool(&data.to_string_lossy()), || false);
        let outcome = scanner.scan();

        assert_eq!(
            outcome.status(),
            ScanStatus::Succeeded,
            "a bad record degrades; it does not fail the scan"
        );
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(outcome.projects()[0].path, normalized(&child));
        assert_eq!(outcome.projects()[0].session_id, None);

        let diagnostic = outcome
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code == MALFORMED_SESSION_RECORD)
            .expect("malformed-record diagnostic");
        assert_eq!(diagnostic.severity, ScanDiagnosticSeverity::Warning);
        assert!(
            !diagnostic.message.contains("not-valid-json"),
            "diagnostics never echo metadata content"
        );
    });
}

#[test]
fn custom_relative_project_path_degrades_to_directory() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let data = home.path().join("custom-data");
        let child = data.join("project-a");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(
            child.join("metadata.json"),
            r#"{"project_path": "relative/path"}"#,
        )
        .unwrap();

        let scanner =
            CustomToolScanner::with_availability(fixture_tool(&data.to_string_lossy()), || false);
        let outcome = scanner.scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects()[0].path, normalized(&child));
    });
}

#[test]
fn custom_project_path_inside_data_directory_is_not_a_project() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let data = home.path().join("custom-data");
        let child = data.join("project-a");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(
            child.join("metadata.json"),
            format!(
                r#"{{"project_path": "{}"}}"#,
                data.to_string_lossy().replace('\\', "\\\\")
            ),
        )
        .unwrap();

        let scanner =
            CustomToolScanner::with_availability(fixture_tool(&data.to_string_lossy()), || false);
        let outcome = scanner.scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(
            outcome.projects()[0].path,
            normalized(&child),
            "a path inside the tool's own data directory is never a project"
        );
    });
}

#[test]
fn custom_invalid_timestamp_falls_back_to_directory_time() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let data = home.path().join("custom-data");
        let child = data.join("project-a");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(
            child.join("metadata.json"),
            r#"{"last_accessed": "not-a-timestamp"}"#,
        )
        .unwrap();

        let scanner =
            CustomToolScanner::with_availability(fixture_tool(&data.to_string_lossy()), || false);
        let outcome = scanner.scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects()[0].last_accessed_at, dir_mtime(&child));
    });
}

#[test]
fn custom_metadata_path_that_is_a_directory_is_treated_as_absent() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let data = home.path().join("custom-data");
        let child = data.join("project-a");
        std::fs::create_dir_all(child.join("metadata.json")).unwrap();

        let scanner =
            CustomToolScanner::with_availability(fixture_tool(&data.to_string_lossy()), || false);
        let outcome = scanner.scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(outcome.projects()[0].path, normalized(&child));
        assert_eq!(outcome.projects()[0].session_id, None);
        assert!(outcome.diagnostics().is_empty());
    });
}

#[test]
fn custom_blank_data_directory_is_unavailable() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let scanner = CustomToolScanner::with_availability(fixture_tool("   "), || false);
        let outcome = scanner.scan();

        assert_eq!(outcome.status(), ScanStatus::Unavailable);
        assert!(outcome.projects().is_empty());
        assert_eq!(outcome.diagnostics()[0].code, SOURCE_UNAVAILABLE);
    });
}

#[test]
fn custom_missing_data_directory_is_unavailable() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let scanner =
            CustomToolScanner::with_availability(fixture_tool("~/does-not-exist"), || false);
        let outcome = scanner.scan();

        assert_eq!(outcome.status(), ScanStatus::Unavailable);
        assert!(outcome.projects().is_empty());
        assert_eq!(outcome.diagnostics()[0].code, SOURCE_UNAVAILABLE);
    });
}

#[test]
fn custom_installed_tool_with_missing_source_is_empty_success() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        let scanner =
            CustomToolScanner::with_availability(fixture_tool("~/does-not-exist"), || true);
        let outcome = scanner.scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(outcome.is_successful());
        assert!(outcome.projects().is_empty());
    });
}

#[test]
fn custom_file_where_data_directory_is_expected_is_failure() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        std::fs::write(home.path().join("custom-data"), "not a directory").unwrap();
        let scanner = CustomToolScanner::with_availability(fixture_tool("~/custom-data"), || true);
        let outcome = scanner.scan();

        assert_eq!(outcome.status(), ScanStatus::Failed);
        assert!(outcome.projects().is_empty());
        assert_eq!(outcome.diagnostics()[0].code, SOURCE_READ_FAILED);
    });
}

#[test]
fn custom_empty_data_directory_is_successful_empty() {
    let home = tempfile::tempdir().unwrap();
    with_home(home.path(), || {
        std::fs::create_dir_all(home.path().join("custom-data")).unwrap();
        let scanner = CustomToolScanner::with_availability(fixture_tool("~/custom-data"), || false);
        let outcome = scanner.scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(outcome.is_successful());
        assert!(outcome.projects().is_empty());
    });
}

#[test]
fn custom_availability_is_separate_from_data_discoverability() {
    let available = CustomToolScanner::with_availability(fixture_tool("~/nope"), || true);
    let unavailable = CustomToolScanner::with_availability(fixture_tool("~/nope"), || false);

    assert!(available.is_available());
    assert!(!unavailable.is_available());
    assert_eq!(available.scan().status(), ScanStatus::Succeeded);
    assert_eq!(unavailable.scan().status(), ScanStatus::Unavailable);
}
