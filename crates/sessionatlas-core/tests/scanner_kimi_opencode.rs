//! Integration tests for the R05B Kimi and OpenCode scanners.
//!
//! These tests use only synthetic fixtures written under a temporary
//! `SESSIONATLAS_HOME` (via the `tempfile` crate) and synthetic SQLite
//! databases created in that same temporary root; they never touch the real
//! `~/.sessionatlas`, the real user home, or the user's `Temp` directly. No
//! real AI CLI is launched; availability is injected explicitly.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use sessionatlas_core::path;
use sessionatlas_core::scanner::kimi::KimiScanner;
use sessionatlas_core::scanner::opencode::OpenCodeScanner;
use sessionatlas_core::scanner::{
    ScanDiagnosticSeverity, ScanOutcome, ScanStatus, ScannedProject, Scanner,
    MALFORMED_SESSION_RECORD, MISSING_PROJECT_PATH, NO_VALID_SESSIONS, SESSION_READ_FAILED,
    SOURCE_READ_FAILED, SOURCE_UNAVAILABLE, TIMESTAMP_FALLBACK,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());
const SECRET: &str = "SUPERSECRET-PROMPT-CONTENT";

/// Runs `body` with the given environment overrides, restoring every affected
/// variable even on panic.
fn with_env<R>(set: &[(&str, &str)], clear: &[&str], body: impl FnOnce() -> R) -> R {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let mut previous: Vec<(String, Option<std::ffi::OsString>)> = Vec::new();
    for (name, value) in set {
        previous.push((name.to_string(), std::env::var_os(name)));
        std::env::set_var(name, value);
    }
    for name in clear {
        previous.push((name.to_string(), std::env::var_os(name)));
        std::env::remove_var(name);
    }
    struct Restore(Vec<(String, Option<std::ffi::OsString>)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            for (name, value) in &self.0 {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
    let _restore = Restore(previous);
    body()
}

/// Sets a temporary `SESSIONATLAS_HOME`, runs `body`, then restores the
/// previous value even on panic.
fn with_home<R>(path: &Path, body: impl FnOnce() -> R) -> R {
    with_env(&[("SESSIONATLAS_HOME", &path.to_string_lossy())], &[], body)
}

fn rfc3339(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

fn unix_millis(value: &str) -> i64 {
    rfc3339(value).timestamp_millis()
}

/// Sets a file's modification time. Windows needs a write-enabled handle for
/// `set_modified`, so a read-only `File::open` handle is not sufficient.
fn set_modified(path: &Path, modified: DateTime<Utc>) {
    let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(modified.into()).unwrap();
}

fn has_code(outcome: &ScanOutcome, code: &'static str) -> bool {
    outcome
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.code == code)
}

fn project_by_path<'a>(projects: &'a [ScannedProject], normalized: &str) -> &'a ScannedProject {
    projects
        .iter()
        .find(|project| project.path == normalized)
        .expect("project present for the expected path")
}

fn home_join(home: &Path, relative: &str) -> String {
    home.join(relative).to_string_lossy().into_owned()
}

// ---------------------------------------------------------------------------
// Kimi
// ---------------------------------------------------------------------------

fn kimi_sessions(home: &Path) -> PathBuf {
    let sessions = home.join(".kimi-code").join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    sessions
}

fn write_kimi_state(
    dir: &Path,
    worktree: &str,
    session: &str,
    value: serde_json::Value,
) -> PathBuf {
    let session_dir = dir.join(worktree).join(session);
    std::fs::create_dir_all(&session_dir).unwrap();
    let state_path = session_dir.join("state.json");
    std::fs::write(&state_path, serde_json::to_string(&value).unwrap()).unwrap();
    state_path
}

fn kimi_state(workdir: Option<&str>, updated_at: Option<&str>) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    if let Some(workdir) = workdir {
        object.insert(
            "workDir".to_string(),
            serde_json::Value::String(workdir.to_string()),
        );
    }
    if let Some(updated_at) = updated_at {
        object.insert(
            "updatedAt".to_string(),
            serde_json::Value::String(updated_at.to_string()),
        );
    }
    serde_json::Value::Object(object)
}

#[test]
fn kimi_recursive_state_json_extracts_minimal_fields() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = kimi_sessions(dir.path());
        let cwd_a = home_join(dir.path(), "work/repo-a");
        let cwd_b = home_join(dir.path(), "work/repo-b");
        write_kimi_state(
            &sessions,
            "worktree-a",
            "session-a",
            kimi_state(Some(&cwd_a), Some("2026-07-30T09:00:00Z")),
        );
        write_kimi_state(
            &sessions,
            "worktree-b",
            "session-b",
            serde_json::json!({
                "workDir": cwd_b,
                "title": "Fixture",
                "timestamp": "2026-08-01T08:30:00Z",
            }),
        );

        let outcome = KimiScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(outcome.diagnostics().is_empty());

        let projects = outcome.projects();
        assert_eq!(projects.len(), 2);
        let a = project_by_path(projects, &path::normalize_native(&cwd_a).unwrap());
        assert_eq!(a.session_id.as_deref(), Some("session-a"));
        assert_eq!(a.last_accessed_at, rfc3339("2026-07-30T09:00:00Z"));

        let b = project_by_path(projects, &path::normalize_native(&cwd_b).unwrap());
        assert_eq!(b.session_id.as_deref(), Some("session-b"));
        assert_eq!(b.last_accessed_at, rfc3339("2026-08-01T08:30:00Z"));
    });
}

#[test]
fn kimi_utf8_bom_state_json_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = kimi_sessions(dir.path());
        let cwd = home_join(dir.path(), "work/bom-kimi");
        let state = write_kimi_state(
            &sessions,
            "worktree",
            "bom-kimi",
            kimi_state(Some(&cwd), Some("2026-07-30T09:00:00Z")),
        );
        let bytes = std::fs::read(&state).unwrap();
        let mut with_bom = b"\xef\xbb\xbf".to_vec();
        with_bom.extend(bytes);
        std::fs::write(state, with_bom).unwrap();

        let outcome = KimiScanner::with_availability(|| false).scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(
            outcome.projects()[0].session_id.as_deref(),
            Some("bom-kimi")
        );
    });
}

#[test]
fn kimi_timestamp_candidates_are_read_in_documented_order() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = kimi_sessions(dir.path());
        let cwd = home_join(dir.path(), "work/repo");
        write_kimi_state(
            &sessions,
            "wt",
            "all-three",
            serde_json::json!({
                "workDir": cwd,
                "updatedAt": "2026-07-30T09:00:00Z",
                "lastUpdatedAt": "2026-07-30T10:00:00Z",
                "timestamp": "2026-07-30T11:00:00Z",
            }),
        );
        write_kimi_state(
            &sessions,
            "wt",
            "no-updated-at",
            serde_json::json!({
                "workDir": cwd,
                "lastUpdatedAt": "2026-07-31T08:00:00Z",
                "timestamp": "2026-07-31T09:00:00Z",
            }),
        );

        let outcome = KimiScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(outcome.diagnostics().is_empty());

        let mut projects = outcome.projects().to_vec();
        projects.sort_by_key(|p| p.session_id.clone());
        let all_three = &projects[0];
        let fallback = &projects[1];
        assert_eq!(all_three.session_id.as_deref(), Some("all-three"));
        assert_eq!(
            all_three.last_accessed_at,
            rfc3339("2026-07-30T09:00:00Z"),
            "updatedAt wins over lastUpdatedAt and timestamp"
        );
        assert_eq!(fallback.session_id.as_deref(), Some("no-updated-at"));
        assert_eq!(
            fallback.last_accessed_at,
            rfc3339("2026-07-31T08:00:00Z"),
            "lastUpdatedAt wins over timestamp"
        );
    });
}

#[test]
fn kimi_missing_timestamp_falls_back_to_state_file_mtime() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = kimi_sessions(dir.path());
        let cwd = home_join(dir.path(), "work/repo");
        let state_path = write_kimi_state(
            &sessions,
            "wt",
            "no-ts",
            serde_json::json!({ "workDir": cwd, "title": "Fixture" }),
        );
        let modified = rfc3339("2026-07-30T11:00:00Z");
        set_modified(&state_path, modified);

        let outcome = KimiScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert!(has_code(&outcome, TIMESTAMP_FALLBACK));
        let delta = outcome.projects()[0]
            .last_accessed_at
            .signed_duration_since(modified)
            .abs();
        assert!(
            delta < Duration::seconds(5),
            "state-file modification time fallback matches the set mtime: {delta}"
        );
    });
}

#[test]
fn kimi_malformed_state_file_keeps_valid_sessions() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = kimi_sessions(dir.path());
        let cwd = home_join(dir.path(), "work/repo");
        write_kimi_state(
            &sessions,
            "wt",
            "valid",
            kimi_state(Some(&cwd), Some("2026-07-30T09:00:00Z")),
        );
        write_kimi_state(
            &sessions,
            "wt",
            "broken",
            serde_json::json!({ "workDir": cwd, "title": 123 }),
        );
        let broken_dir = sessions.join("wt").join("broken");
        std::fs::write(broken_dir.join("state.json"), b"this is not valid json\n").unwrap();

        let outcome = KimiScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(outcome.projects()[0].session_id.as_deref(), Some("valid"));
        let diagnostic = outcome
            .diagnostics()
            .iter()
            .find(|d| d.code == MALFORMED_SESSION_RECORD)
            .expect("malformed-session diagnostic");
        assert_eq!(diagnostic.severity, ScanDiagnosticSeverity::Warning);
    });
}

#[test]
fn kimi_missing_workdir_skips_that_session() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = kimi_sessions(dir.path());
        write_kimi_state(
            &sessions,
            "wt",
            "no-workdir",
            serde_json::json!({ "title": "Fixture", "updatedAt": "2026-07-30T09:00:00Z" }),
        );

        let outcome = KimiScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Failed);
        assert!(outcome.projects().is_empty());
        assert!(has_code(&outcome, MISSING_PROJECT_PATH));
        assert!(has_code(&outcome, NO_VALID_SESSIONS));
    });
}

#[test]
fn kimi_workdir_inside_kimi_home_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = kimi_sessions(dir.path());
        let inside = sessions
            .join("wt")
            .join("inside")
            .to_string_lossy()
            .into_owned();
        write_kimi_state(
            &sessions,
            "wt",
            "inside",
            serde_json::json!({ "workDir": inside, "updatedAt": "2026-07-30T09:00:00Z" }),
        );

        let outcome = KimiScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Failed);
        assert!(outcome.projects().is_empty());
        assert!(has_code(&outcome, MISSING_PROJECT_PATH));
    });
}

#[test]
fn kimi_missing_source_separates_availability_from_discovery() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let unavailable = KimiScanner::with_availability(|| false).scan();
        assert_eq!(unavailable.status(), ScanStatus::Unavailable);
        assert!(unavailable.projects().is_empty());
        assert_eq!(unavailable.diagnostics()[0].code, SOURCE_UNAVAILABLE);
        assert_eq!(
            unavailable.diagnostics()[0].severity,
            ScanDiagnosticSeverity::Info
        );

        let empty_success = KimiScanner::with_availability(|| true).scan();
        assert_eq!(empty_success.status(), ScanStatus::Succeeded);
        assert!(empty_success.is_successful());
        assert!(empty_success.projects().is_empty());
        assert!(empty_success.diagnostics().is_empty());
    });
}

#[test]
fn kimi_existing_empty_source_is_trustworthy_empty_success() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        kimi_sessions(dir.path());
        let outcome = KimiScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(outcome.projects().is_empty());
        assert!(outcome.diagnostics().is_empty());
    });
}

#[test]
fn kimi_sessions_path_that_is_a_file_is_read_failure() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = dir.path().join(".kimi-code").join("sessions");
        std::fs::create_dir_all(sessions.parent().unwrap()).unwrap();
        std::fs::write(&sessions, b"not a directory").unwrap();

        let outcome = KimiScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Failed);
        assert!(outcome.projects().is_empty());
        assert_eq!(outcome.diagnostics()[0].code, SOURCE_READ_FAILED);
        assert_eq!(
            outcome.diagnostics()[0].severity,
            ScanDiagnosticSeverity::Error
        );
    });
}

#[test]
fn kimi_unreadable_state_file_keeps_valid_files() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = kimi_sessions(dir.path());
        let cwd = home_join(dir.path(), "work/repo");
        write_kimi_state(
            &sessions,
            "wt",
            "ok",
            kimi_state(Some(&cwd), Some("2026-07-30T09:00:00Z")),
        );
        let unreadable = sessions.join("wt").join("unreadable");
        std::fs::create_dir_all(&unreadable).unwrap();
        std::fs::write(unreadable.join("state.json"), b"invalid utf-8 \xff\xfe\n").unwrap();

        let outcome = KimiScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(outcome.projects()[0].session_id.as_deref(), Some("ok"));
        assert!(has_code(&outcome, SESSION_READ_FAILED));
    });
}

#[test]
fn kimi_normalizes_tilde_and_trailing_separators() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = kimi_sessions(dir.path());
        write_kimi_state(
            &sessions,
            "wt",
            "tilde",
            serde_json::json!({
                "workDir": "~/work/tilde-repo/",
                "updatedAt": "2026-07-30T09:00:00Z",
            }),
        );

        let outcome = KimiScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        let expected = path::normalize_native(&home_join(dir.path(), "work/tilde-repo")).unwrap();
        assert_eq!(outcome.projects()[0].path, expected);
    });
}

#[test]
fn kimi_uses_kimi_code_home_override() {
    let kimi_home = tempfile::tempdir().unwrap();
    let project_dir = tempfile::tempdir().unwrap();
    with_env(
        &[("KIMI_CODE_HOME", &kimi_home.path().to_string_lossy())],
        &["SESSIONATLAS_HOME"],
        || {
            let sessions = kimi_home.path().join("sessions");
            std::fs::create_dir_all(sessions.join("wt")).unwrap();
            let project_path = home_join(project_dir.path(), "work/repo");
            write_kimi_state(
                &sessions,
                "wt",
                "override",
                kimi_state(Some(&project_path), Some("2026-07-30T09:00:00Z")),
            );

            let outcome = KimiScanner::with_availability(|| false).scan();
            assert_eq!(outcome.status(), ScanStatus::Succeeded);
            assert_eq!(outcome.projects().len(), 1);
            assert_eq!(
                outcome.projects()[0].session_id.as_deref(),
                Some("override")
            );
        },
    );
}

#[test]
fn kimi_state_never_leaks_conversation_content() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = kimi_sessions(dir.path());
        let cwd = home_join(dir.path(), "work/repo");
        write_kimi_state(
            &sessions,
            "wt",
            "privacy",
            serde_json::json!({
                "workDir": cwd,
                "updatedAt": "2026-07-30T09:00:00Z",
                "messages": [{ "content": SECRET }],
            }),
        );

        let outcome = KimiScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert!(
            outcome
                .diagnostics()
                .iter()
                .all(|d| !d.message.contains(SECRET)),
            "diagnostics must never embed session content"
        );
        for project in outcome.projects() {
            assert!(
                !format!("{project:?}").contains(SECRET),
                "projects must carry only minimal fields"
            );
        }
    });
}

// ---------------------------------------------------------------------------
// OpenCode
// ---------------------------------------------------------------------------

const OPENCODE_SCHEMA: &str = "
    CREATE TABLE project (
        id TEXT PRIMARY KEY,
        worktree TEXT NOT NULL,
        name TEXT,
        time_created INTEGER NOT NULL,
        time_updated INTEGER NOT NULL
    );
    CREATE TABLE session (
        id TEXT PRIMARY KEY,
        project_id TEXT NOT NULL,
        directory TEXT NOT NULL,
        title TEXT NOT NULL,
        version TEXT NOT NULL,
        time_created INTEGER NOT NULL,
        time_updated INTEGER NOT NULL,
        time_archived INTEGER,
        FOREIGN KEY (project_id) REFERENCES project(id) ON DELETE CASCADE
    );
";

fn opencode_home_db(home: &Path) -> PathBuf {
    home.join(".local")
        .join("share")
        .join("opencode")
        .join("opencode.db")
}

fn create_opencode_db(database_path: &Path) -> Connection {
    if let Some(parent) = database_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let connection = Connection::open(database_path).unwrap();
    connection.execute_batch(OPENCODE_SCHEMA).unwrap();
    connection
}

fn insert_opencode_row(
    connection: &Connection,
    session_id: &str,
    project_id: &str,
    worktree: &str,
    directory: &str,
    session_updated: i64,
    project_updated: i64,
) {
    connection
        .execute(
            "INSERT INTO project (id, worktree, name, time_created, time_updated)
             VALUES (?1, ?2, 'Demo', ?3, ?3)",
            rusqlite::params![project_id, worktree, project_updated],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO session
                (id, project_id, directory, title, version, time_created, time_updated)
             VALUES (?1, ?2, ?3, 'Fixture', '0.0.0', ?4, ?4)",
            rusqlite::params![session_id, project_id, directory, session_updated],
        )
        .unwrap();
}

#[test]
fn opencode_scanner_reads_project_and_session_tables_read_only() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let database_path = opencode_home_db(dir.path());
        let project_path = home_join(dir.path(), "work/opencode-project");
        let updated = unix_millis("2026-07-30T12:00:00Z");

        let connection = create_opencode_db(&database_path);
        insert_opencode_row(
            &connection,
            "session-opencode-demo",
            "project-demo",
            &project_path,
            &project_path,
            updated,
            updated,
        );
        drop(connection);
        let bytes_before = std::fs::read(&database_path).unwrap();

        let outcome = OpenCodeScanner::with_availability(|| false).scan();

        let bytes_after = std::fs::read(&database_path).unwrap();
        assert_eq!(bytes_before, bytes_after, "the database is never written");
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(outcome.diagnostics().is_empty());

        let projects = outcome.projects();
        assert_eq!(projects.len(), 1);
        let project = project_by_path(projects, &path::normalize_native(&project_path).unwrap());
        assert_eq!(project.session_id.as_deref(), Some("session-opencode-demo"));
        assert_eq!(project.last_accessed_at, rfc3339("2026-07-30T12:00:00Z"));
    });
}

#[test]
fn opencode_scanner_uses_worktree_when_session_directory_is_blank() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let database_path = opencode_home_db(dir.path());
        let worktree_a = home_join(dir.path(), "work/repo-a");
        let worktree_b = home_join(dir.path(), "work/repo-b");
        let updated = unix_millis("2026-07-30T12:00:00Z");

        let connection = create_opencode_db(&database_path);
        insert_opencode_row(&connection, "s-a", "p-a", &worktree_a, "", updated, updated);
        insert_opencode_row(
            &connection,
            "s-b",
            "p-b",
            &worktree_b,
            "   ",
            updated,
            updated,
        );
        drop(connection);

        let outcome = OpenCodeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 2);
        let a = project_by_path(
            outcome.projects(),
            &path::normalize_native(&worktree_a).unwrap(),
        );
        assert_eq!(a.session_id.as_deref(), Some("s-a"));
        let b = project_by_path(
            outcome.projects(),
            &path::normalize_native(&worktree_b).unwrap(),
        );
        assert_eq!(b.session_id.as_deref(), Some("s-b"));
    });
}

#[test]
fn opencode_scanner_uses_project_timestamp_when_session_timestamp_invalid() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let database_path = opencode_home_db(dir.path());
        let project_path = home_join(dir.path(), "work/repo");
        let valid = unix_millis("2026-07-30T12:00:00Z");

        let connection = create_opencode_db(&database_path);
        insert_opencode_row(
            &connection,
            "s-invalid",
            "p-invalid",
            &project_path,
            &project_path,
            i64::MAX,
            valid,
        );
        drop(connection);

        let outcome = OpenCodeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(
            !has_code(&outcome, TIMESTAMP_FALLBACK),
            "a usable project timestamp avoids the fallback"
        );
        let project = &outcome.projects()[0];
        assert_eq!(project.last_accessed_at, rfc3339("2026-07-30T12:00:00Z"));
    });
}

#[test]
fn opencode_scanner_falls_back_to_db_mtime_when_both_timestamps_invalid() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let database_path = opencode_home_db(dir.path());
        let project_path = home_join(dir.path(), "work/repo");

        let connection = create_opencode_db(&database_path);
        insert_opencode_row(
            &connection,
            "s-invalid",
            "p-invalid",
            &project_path,
            &project_path,
            i64::MAX,
            i64::MAX,
        );
        drop(connection);

        let modified = rfc3339("2026-07-30T11:00:00Z");
        set_modified(&database_path, modified);

        let outcome = OpenCodeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(has_code(&outcome, TIMESTAMP_FALLBACK));
        let delta = outcome.projects()[0]
            .last_accessed_at
            .signed_duration_since(modified)
            .abs();
        assert!(
            delta < Duration::seconds(5),
            "database modification time fallback matches the set mtime: {delta}"
        );
    });
}

#[test]
fn opencode_scanner_skips_sessions_with_unsafe_paths() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let database_path = opencode_home_db(dir.path());
        let source_root = database_path
            .parent()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let valid = home_join(dir.path(), "work/repo-valid");
        let updated = unix_millis("2026-07-30T12:00:00Z");

        let connection = create_opencode_db(&database_path);
        insert_opencode_row(
            &connection,
            "s-unsafe",
            "p-unsafe",
            &source_root,
            &source_root,
            updated,
            updated,
        );
        insert_opencode_row(
            &connection,
            "s-relative",
            "p-relative",
            "relative/dir",
            "relative/dir",
            updated,
            updated,
        );
        insert_opencode_row(
            &connection,
            "s-valid",
            "p-valid",
            &valid,
            &valid,
            updated,
            updated,
        );
        drop(connection);

        let outcome = OpenCodeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(outcome.projects()[0].session_id.as_deref(), Some("s-valid"));
        assert_eq!(
            outcome
                .diagnostics()
                .iter()
                .filter(|d| d.code == MISSING_PROJECT_PATH)
                .count(),
            2,
            "source-root and relative session paths are both skipped"
        );
    });
}

#[test]
fn opencode_scanner_schema_failure_is_failed_not_empty_success() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let database_path = opencode_home_db(dir.path());
        if let Some(parent) = database_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let connection = Connection::open(&database_path).unwrap();
        connection
            .execute_batch("CREATE TABLE unrelated (id TEXT)")
            .unwrap();
        drop(connection);

        let outcome = OpenCodeScanner::with_availability(|| true).scan();
        assert_eq!(outcome.status(), ScanStatus::Failed);
        assert!(outcome.projects().is_empty());
        let diagnostic = outcome
            .diagnostics()
            .iter()
            .find(|d| d.code == SOURCE_READ_FAILED)
            .expect("source-read-failed diagnostic");
        assert_eq!(diagnostic.severity, ScanDiagnosticSeverity::Error);
    });
}

#[test]
fn opencode_scanner_invalid_database_file_is_failed_not_empty_success() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let database_path = opencode_home_db(dir.path());
        if let Some(parent) = database_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&database_path, b"this is not a sqlite database").unwrap();

        let outcome = OpenCodeScanner::with_availability(|| true).scan();
        assert_eq!(outcome.status(), ScanStatus::Failed);
        assert!(outcome.projects().is_empty());
        assert!(has_code(&outcome, SOURCE_READ_FAILED));
    });
}

#[test]
fn opencode_scanner_valid_empty_database_is_successful_empty() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let database_path = opencode_home_db(dir.path());
        drop(create_opencode_db(&database_path));

        let outcome = OpenCodeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(outcome.is_successful());
        assert!(outcome.projects().is_empty());
        assert!(outcome.diagnostics().is_empty());
    });
}

#[test]
fn opencode_scanner_missing_source_separates_availability_from_discovery() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let unavailable = OpenCodeScanner::with_availability(|| false).scan();
        assert_eq!(unavailable.status(), ScanStatus::Unavailable);
        assert!(unavailable.projects().is_empty());
        assert_eq!(unavailable.diagnostics()[0].code, SOURCE_UNAVAILABLE);
        assert_eq!(
            unavailable.diagnostics()[0].severity,
            ScanDiagnosticSeverity::Info
        );

        let empty_success = OpenCodeScanner::with_availability(|| true).scan();
        assert_eq!(empty_success.status(), ScanStatus::Succeeded);
        assert!(empty_success.is_successful());
        assert!(empty_success.projects().is_empty());
        assert!(empty_success.diagnostics().is_empty());
    });
}

#[test]
fn opencode_scanner_alternate_database_path_under_home_opencode() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let database_path = dir.path().join(".opencode").join("opencode.db");
        let project_path = home_join(dir.path(), "work/repo");
        let updated = unix_millis("2026-07-30T12:00:00Z");

        let connection = create_opencode_db(&database_path);
        insert_opencode_row(
            &connection,
            "s-alternate",
            "p-alternate",
            &project_path,
            &project_path,
            updated,
            updated,
        );
        drop(connection);

        let outcome = OpenCodeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(
            outcome.projects()[0].session_id.as_deref(),
            Some("s-alternate")
        );
    });
}

#[test]
fn opencode_scanner_candidate_that_is_a_directory_is_read_failure() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let database_path = dir.path().join(".opencode").join("opencode.db");
        std::fs::create_dir_all(&database_path).unwrap();

        let outcome = OpenCodeScanner::with_availability(|| true).scan();
        assert_eq!(outcome.status(), ScanStatus::Failed);
        assert!(outcome.projects().is_empty());
        assert_eq!(outcome.diagnostics()[0].code, SOURCE_READ_FAILED);
    });
}

#[test]
fn opencode_scanner_db_never_leaks_session_content() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let database_path = opencode_home_db(dir.path());
        let project_path = home_join(dir.path(), "work/repo");
        let updated = unix_millis("2026-07-30T12:00:00Z");

        let connection = create_opencode_db(&database_path);
        connection
            .execute(
                "INSERT INTO project (id, worktree, name, time_created, time_updated)
                 VALUES ('p-privacy', ?1, 'Demo', ?2, ?2)",
                rusqlite::params![project_path, updated],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO session
                    (id, project_id, directory, title, version, time_created, time_updated)
                 VALUES ('s-privacy', 'p-privacy', ?1, ?2, '0.0.0', ?3, ?3)",
                rusqlite::params![project_path, SECRET, updated],
            )
            .unwrap();
        drop(connection);

        let outcome = OpenCodeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert!(
            outcome
                .diagnostics()
                .iter()
                .all(|d| !d.message.contains(SECRET)),
            "diagnostics must never embed session content"
        );
        for project in outcome.projects() {
            assert!(
                !format!("{project:?}").contains(SECRET),
                "projects must carry only minimal fields"
            );
        }
    });
}
