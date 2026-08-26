//! Integration tests for the R05A Codex and Claude scanners.
//!
//! These tests use only synthetic fixtures written under a temporary
//! `SESSIONATLAS_HOME` (via the `tempfile` crate) and never touch the real
//! `~/.sessionatlas`, the real user home, or the user's `Temp` directly. No
//! real AI CLI is launched; availability is injected explicitly.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use sessionatlas_core::indexer::{build_index, IndexedToolScan};
use sessionatlas_core::path;
use sessionatlas_core::scanner::claude::ClaudeScanner;
use sessionatlas_core::scanner::codex::CodexScanner;
use sessionatlas_core::scanner::{
    ScanDiagnosticSeverity, ScanOutcome, ScanStatus, ScannedProject, Scanner,
    MALFORMED_SESSION_RECORD, MISSING_PROJECT_PATH, MISSING_SESSION_ID, NO_VALID_SESSIONS,
    SESSION_READ_FAILED, SOURCE_READ_FAILED, SOURCE_UNAVAILABLE, TIMESTAMP_FALLBACK,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());
const SECRET: &str = "SUPERSECRET-PROMPT-CONTENT";

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

fn codex_sessions(home: &Path) -> PathBuf {
    let sessions = home.join(".codex").join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    sessions
}

fn claude_projects(home: &Path) -> PathBuf {
    let projects = home.join(".claude").join("projects");
    std::fs::create_dir_all(&projects).unwrap();
    projects
}

fn write_jsonl(path: &Path, lines: &[Value]) {
    let mut content = String::new();
    for line in lines {
        content.push_str(&serde_json::to_string(line).unwrap());
        content.push('\n');
    }
    std::fs::write(path, content).unwrap();
}

fn codex_meta(cwd: &str, id: &str, timestamp: &str) -> Value {
    serde_json::json!({
        "timestamp": timestamp,
        "type": "session_meta",
        "payload": { "id": id, "cwd": cwd, "timestamp": timestamp },
    })
}

fn claude_line(
    cwd: Option<&str>,
    session_id: Option<&str>,
    git_branch: Option<&str>,
    timestamp: Option<&str>,
) -> Value {
    let mut object = serde_json::Map::new();
    if let Some(cwd) = cwd {
        object.insert("cwd".to_string(), Value::String(cwd.to_string()));
    }
    if let Some(session_id) = session_id {
        object.insert(
            "sessionId".to_string(),
            Value::String(session_id.to_string()),
        );
    }
    if let Some(git_branch) = git_branch {
        object.insert(
            "gitBranch".to_string(),
            Value::String(git_branch.to_string()),
        );
    }
    if let Some(timestamp) = timestamp {
        object.insert(
            "timestamp".to_string(),
            Value::String(timestamp.to_string()),
        );
    }
    Value::Object(object)
}

fn rfc3339(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

fn has_code(outcome: &ScanOutcome, code: &'static str) -> bool {
    outcome
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.code == code)
}

fn codes(outcome: &ScanOutcome) -> Vec<&'static str> {
    outcome
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
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

#[test]
fn codex_and_claude_keep_missing_configured_projects_for_index_marking() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let codex_path = home_join(dir.path(), "gone/codex-project");
        let claude_path = home_join(dir.path(), "gone/claude-project");
        assert!(!Path::new(&codex_path).exists());
        assert!(!Path::new(&claude_path).exists());

        let codex_source = codex_sessions(dir.path()).join("2026/08/16");
        std::fs::create_dir_all(&codex_source).unwrap();
        write_jsonl(
            &codex_source.join("missing.jsonl"),
            &[codex_meta(
                &codex_path,
                "codex-missing",
                "2026-08-16T01:00:00Z",
            )],
        );

        let claude_source = claude_projects(dir.path()).join("encoded");
        std::fs::create_dir_all(&claude_source).unwrap();
        write_jsonl(
            &claude_source.join("missing.jsonl"),
            &[claude_line(
                Some(&claude_path),
                Some("claude-missing"),
                Some("old-branch"),
                Some("2026-08-16T02:00:00Z"),
            )],
        );

        let codex = CodexScanner::with_availability(|| false).scan();
        let claude = ClaudeScanner::with_availability(|| false).scan();
        assert_eq!(codex.status(), ScanStatus::Succeeded);
        assert_eq!(claude.status(), ScanStatus::Succeeded);

        let indexed = build_index(&[
            IndexedToolScan {
                tool_key: "codex".to_string(),
                tool_name: "Codex CLI".to_string(),
                projects: codex.into_projects(),
            },
            IndexedToolScan {
                tool_key: "claude".to_string(),
                tool_name: "Claude Code".to_string(),
                projects: claude.into_projects(),
            },
        ]);
        assert_eq!(indexed.len(), 2);
        assert!(indexed.iter().all(|project| project.path_missing));
        assert!(indexed.iter().any(|project| {
            project.path == path::normalize_native(&codex_path).unwrap()
                && project.tool_usages[0].tool_key == "codex"
        }));
        assert!(indexed.iter().any(|project| {
            project.path == path::normalize_native(&claude_path).unwrap()
                && project.tool_usages[0].tool_key == "claude"
        }));
    });
}

// ---------------------------------------------------------------------------
// Codex
// ---------------------------------------------------------------------------

#[test]
fn codex_recursive_jsonl_extracts_minimal_fields() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = codex_sessions(dir.path());
        std::fs::create_dir_all(sessions.join("2025/01/02")).unwrap();
        std::fs::create_dir_all(sessions.join("2025/03/04")).unwrap();

        let cwd_a = home_join(dir.path(), "work/repo-a");
        let cwd_b = home_join(dir.path(), "work/repo-b");
        write_jsonl(
            &sessions.join("2025/01/02/aaa.jsonl"),
            &[
                codex_meta(&cwd_a, "codex-session-a", "2026-07-30T09:00:00Z"),
                serde_json::json!({
                    "timestamp": "2026-07-31T10:00:00Z",
                    "type": "user",
                    "payload": { "content": "ignored" },
                }),
            ],
        );
        write_jsonl(
            &sessions.join("2025/03/04/bbb.jsonl"),
            &[codex_meta(
                &cwd_b,
                "codex-session-b",
                "2026-08-01T08:30:00Z",
            )],
        );

        let outcome = CodexScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(outcome.diagnostics().is_empty());

        let projects = outcome.projects();
        assert_eq!(projects.len(), 2);

        let a = project_by_path(projects, &path::normalize_native(&cwd_a).unwrap());
        assert_eq!(a.session_id.as_deref(), Some("codex-session-a"));
        assert_eq!(
            a.last_accessed_at,
            rfc3339("2026-07-31T10:00:00Z"),
            "the greatest timestamp wins across session_meta and non-meta records"
        );

        let b = project_by_path(projects, &path::normalize_native(&cwd_b).unwrap());
        assert_eq!(b.session_id.as_deref(), Some("codex-session-b"));
        assert_eq!(b.last_accessed_at, rfc3339("2026-08-01T08:30:00Z"));
    });
}

#[test]
fn codex_bad_lines_form_diagnostic_and_retain_valid_records() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = codex_sessions(dir.path());
        std::fs::create_dir_all(sessions.join("2025/01/02")).unwrap();
        let cwd = home_join(dir.path(), "work/repo");

        write_jsonl(
            &sessions.join("2025/01/02/aaa.jsonl"),
            &[codex_meta(&cwd, "ok-session", "2026-07-30T09:00:00Z")],
        );

        let bad = sessions.join("2025/01/02/bbb.jsonl");
        write_jsonl(
            &bad,
            &[codex_meta(&cwd, "kept-session", "2026-07-30T09:00:00Z")],
        );
        let existing = std::fs::read_to_string(&bad).unwrap();
        std::fs::write(
            &bad,
            format!("{existing}this line is not valid json\n[1,2,3]\n"),
        )
        .unwrap();

        let outcome = CodexScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 2);
        let diagnostic = outcome
            .diagnostics()
            .iter()
            .find(|d| d.code == MALFORMED_SESSION_RECORD)
            .expect("malformed-session diagnostic");
        assert_eq!(diagnostic.severity, ScanDiagnosticSeverity::Warning);
        assert!(diagnostic.message.contains("1 malformed record"));
    });
}

#[test]
fn codex_missing_session_id_skips_that_session_but_keeps_valid_ones() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = codex_sessions(dir.path());
        std::fs::create_dir_all(sessions.join("2025/01/02")).unwrap();
        let cwd_a = home_join(dir.path(), "work/repo-a");
        let cwd_b = home_join(dir.path(), "work/repo-b");
        let cwd_c = home_join(dir.path(), "work/repo-c");

        write_jsonl(
            &sessions.join("2025/01/02/aaa.jsonl"),
            &[serde_json::json!({
                "timestamp": "2026-07-30T09:00:00Z",
                "type": "session_meta",
                "payload": { "cwd": cwd_a },
            })],
        );
        write_jsonl(
            &sessions.join("2025/01/02/bbb.jsonl"),
            &[codex_meta(&cwd_b, "valid-session", "2026-07-30T09:00:00Z")],
        );
        write_jsonl(
            &sessions.join("2025/01/02/ccc.jsonl"),
            &[serde_json::json!({
                "timestamp": "2026-07-30T09:00:00Z",
                "type": "session_meta",
                "payload": { "id": "", "cwd": cwd_c },
            })],
        );

        let outcome = CodexScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(
            outcome.projects()[0].session_id.as_deref(),
            Some("valid-session")
        );
        let missing = outcome
            .diagnostics()
            .iter()
            .filter(|d| d.code == MISSING_SESSION_ID)
            .count();
        assert_eq!(missing, 2, "absent and blank session IDs are both skipped");
    });
}

#[test]
fn codex_missing_or_malformed_timestamp_falls_back_to_mtime() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = codex_sessions(dir.path());
        std::fs::create_dir_all(sessions.join("2025/01/02")).unwrap();
        let cwd_a = home_join(dir.path(), "work/a");
        let cwd_b = home_join(dir.path(), "work/b");

        write_jsonl(
            &sessions.join("2025/01/02/no-ts.jsonl"),
            &[serde_json::json!({
                "type": "session_meta",
                "payload": { "id": "no-ts", "cwd": cwd_a },
            })],
        );
        write_jsonl(
            &sessions.join("2025/01/02/bad-ts.jsonl"),
            &[serde_json::json!({
                "type": "session_meta",
                "payload": { "id": "bad-ts", "cwd": cwd_b, "timestamp": "not-a-date" },
            })],
        );

        let outcome = CodexScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 2);
        assert_eq!(
            outcome
                .diagnostics()
                .iter()
                .filter(|d| d.code == TIMESTAMP_FALLBACK)
                .count(),
            2,
            "each timestamp-less session falls back exactly once"
        );
        let now = Utc::now();
        for project in outcome.projects() {
            let delta = now.signed_duration_since(project.last_accessed_at).abs();
            assert!(
                delta < Duration::seconds(60),
                "filesystem modification time fallback is recent: {delta}"
            );
        }
    });
}

#[test]
fn codex_missing_source_separates_availability_from_discovery() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let unavailable = CodexScanner::with_availability(|| false).scan();
        assert_eq!(unavailable.status(), ScanStatus::Unavailable);
        assert!(unavailable.projects().is_empty());
        assert_eq!(codes(&unavailable), vec![SOURCE_UNAVAILABLE]);
        assert_eq!(
            unavailable.diagnostics()[0].severity,
            ScanDiagnosticSeverity::Info
        );

        let empty_success = CodexScanner::with_availability(|| true).scan();
        assert_eq!(empty_success.status(), ScanStatus::Succeeded);
        assert!(empty_success.is_successful());
        assert!(empty_success.projects().is_empty());
        assert!(empty_success.diagnostics().is_empty());
    });
}

#[test]
fn codex_existing_empty_source_is_trustworthy_empty_success() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        codex_sessions(dir.path());
        let outcome = CodexScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(outcome.projects().is_empty());
        assert!(outcome.diagnostics().is_empty());
    });
}

#[test]
fn codex_source_path_that_is_a_file_is_read_failure() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = dir.path().join(".codex").join("sessions");
        std::fs::create_dir_all(sessions.parent().unwrap()).unwrap();
        std::fs::write(&sessions, b"not a directory").unwrap();

        let outcome = CodexScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Failed);
        assert!(outcome.projects().is_empty());
        assert_eq!(codes(&outcome), vec![SOURCE_READ_FAILED]);
        assert_eq!(
            outcome.diagnostics()[0].severity,
            ScanDiagnosticSeverity::Error
        );
    });
}

#[test]
fn codex_unreadable_session_file_keeps_valid_files() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = codex_sessions(dir.path());
        std::fs::create_dir_all(sessions.join("2025/01/02")).unwrap();
        let cwd = home_join(dir.path(), "work/repo");

        write_jsonl(
            &sessions.join("2025/01/02/ok.jsonl"),
            &[codex_meta(&cwd, "ok", "2026-07-30T09:00:00Z")],
        );
        std::fs::write(
            sessions.join("2025/01/02/bad.jsonl"),
            b"invalid utf-8 \xff\xfe\n",
        )
        .unwrap();

        let outcome = CodexScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(outcome.projects()[0].session_id.as_deref(), Some("ok"));
        assert!(has_code(&outcome, SESSION_READ_FAILED));
    });
}

#[test]
fn codex_files_with_no_valid_project_fail_with_no_valid_sessions() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = codex_sessions(dir.path());
        std::fs::create_dir_all(sessions.join("2025/01/02")).unwrap();
        write_jsonl(
            &sessions.join("2025/01/02/empty.jsonl"),
            &[serde_json::json!({ "type": "user", "payload": { "content": "x" } })],
        );

        let outcome = CodexScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Failed);
        assert!(outcome.projects().is_empty());
        assert!(has_code(&outcome, MISSING_PROJECT_PATH));
        assert!(has_code(&outcome, NO_VALID_SESSIONS));
    });
}

#[test]
fn codex_session_never_leaks_prompt_or_message_content() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = codex_sessions(dir.path());
        std::fs::create_dir_all(sessions.join("2025/01/02")).unwrap();
        let cwd = home_join(dir.path(), "work/repo");
        let file = sessions.join("2025/01/02/aaa.jsonl");
        let content = format!(
            "{}\n{{\"type\":\"user\",\"payload\":{{\"content\":\"{SECRET}\"}}}}\ngarbage line mentioning {SECRET}\n",
            serde_json::to_string(&codex_meta(&cwd, "privacy", "2026-07-30T09:00:00Z")).unwrap()
        );
        std::fs::write(&file, content).unwrap();

        let outcome = CodexScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert!(has_code(&outcome, MALFORMED_SESSION_RECORD));
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

#[test]
fn codex_cwd_inside_tool_home_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = codex_sessions(dir.path());
        std::fs::create_dir_all(sessions.join("2025/01/02")).unwrap();
        let inside = sessions.join("2025/01/02").to_string_lossy().into_owned();
        write_jsonl(
            &sessions.join("2025/01/02/aaa.jsonl"),
            &[serde_json::json!({
                "type": "session_meta",
                "payload": { "id": "inside", "cwd": inside, "timestamp": "2026-07-30T09:00:00Z" },
            })],
        );

        let outcome = CodexScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Failed);
        assert!(outcome.projects().is_empty());
        assert!(has_code(&outcome, MISSING_PROJECT_PATH));
    });
}

#[test]
fn codex_normalizes_tilde_and_trailing_separators() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = codex_sessions(dir.path());
        std::fs::create_dir_all(sessions.join("2025/01/02")).unwrap();
        write_jsonl(
            &sessions.join("2025/01/02/aaa.jsonl"),
            &[serde_json::json!({
                "type": "session_meta",
                "payload": { "id": "tilde", "cwd": "~/work/tilde-repo/", "timestamp": "2026-07-30T09:00:00Z" },
            })],
        );

        let outcome = CodexScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        let expected = path::normalize_native(&home_join(dir.path(), "work/tilde-repo")).unwrap();
        assert_eq!(outcome.projects()[0].path, expected);
    });
}

#[test]
fn codex_scanner_identity_and_availability_injection() {
    let available = CodexScanner::with_availability(|| true);
    assert_eq!(available.tool_key(), "codex");
    assert_eq!(available.tool_name(), "Codex CLI");
    assert!(available.is_available());

    let unavailable = CodexScanner::with_availability(|| false);
    assert_eq!(unavailable.tool_key(), "codex");
    assert!(!unavailable.is_available());
}

// ---------------------------------------------------------------------------
// Claude
// ---------------------------------------------------------------------------

#[test]
fn claude_recursive_jsonl_extracts_minimal_fields_and_git_branch() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let projects_dir = claude_projects(dir.path());
        std::fs::create_dir_all(projects_dir.join("enc/one")).unwrap();
        std::fs::create_dir_all(projects_dir.join("enc/two")).unwrap();
        let cwd_a = home_join(dir.path(), "work/repo-a");
        let cwd_b = home_join(dir.path(), "work/repo-b");

        write_jsonl(
            &projects_dir.join("enc/one/aaa.jsonl"),
            &[claude_line(
                Some(&cwd_a),
                Some("claude-session-a"),
                Some("main"),
                Some("2026-07-30T09:00:00Z"),
            )],
        );
        write_jsonl(
            &projects_dir.join("enc/two/bbb.jsonl"),
            &[claude_line(
                Some(&cwd_b),
                Some("claude-session-b"),
                Some("feature/x"),
                Some("2026-08-01T08:30:00Z"),
            )],
        );

        let outcome = ClaudeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(outcome.diagnostics().is_empty());

        let found = outcome.projects();
        assert_eq!(found.len(), 2);
        let a = found
            .iter()
            .find(|p| p.session_id.as_deref() == Some("claude-session-a"))
            .unwrap();
        assert_eq!(a.path, path::normalize_native(&cwd_a).unwrap());
        assert_eq!(a.git_branch.as_deref(), Some("main"));
        assert_eq!(a.last_accessed_at, rfc3339("2026-07-30T09:00:00Z"));

        let b = found
            .iter()
            .find(|p| p.session_id.as_deref() == Some("claude-session-b"))
            .unwrap();
        assert_eq!(b.path, path::normalize_native(&cwd_b).unwrap());
        assert_eq!(b.git_branch.as_deref(), Some("feature/x"));
        assert_eq!(b.last_accessed_at, rfc3339("2026-08-01T08:30:00Z"));
    });
}

#[test]
fn claude_bad_lines_form_diagnostic_and_retain_valid_records() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let projects_dir = claude_projects(dir.path());
        std::fs::create_dir_all(projects_dir.join("enc")).unwrap();
        let cwd = home_join(dir.path(), "work/repo");

        let bad = projects_dir.join("enc/bbb.jsonl");
        write_jsonl(
            &bad,
            &[claude_line(
                Some(&cwd),
                Some("kept"),
                Some("main"),
                Some("2026-07-30T09:00:00Z"),
            )],
        );
        let existing = std::fs::read_to_string(&bad).unwrap();
        std::fs::write(&bad, format!("{existing}garbage not json\n")).unwrap();

        let outcome = ClaudeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(outcome.projects()[0].session_id.as_deref(), Some("kept"));
        let diagnostic = outcome
            .diagnostics()
            .iter()
            .find(|d| d.code == MALFORMED_SESSION_RECORD)
            .expect("malformed-session diagnostic");
        assert_eq!(diagnostic.severity, ScanDiagnosticSeverity::Warning);
    });
}

#[test]
fn claude_missing_session_id_falls_back_to_file_stem() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let projects_dir = claude_projects(dir.path());
        std::fs::create_dir_all(projects_dir.join("enc")).unwrap();
        let cwd = home_join(dir.path(), "work/repo");
        let cwd_2 = home_join(dir.path(), "work/repo-2");

        write_jsonl(
            &projects_dir.join("enc/fallback.jsonl"),
            &[claude_line(
                Some(&cwd),
                None,
                Some("main"),
                Some("2026-07-30T09:00:00Z"),
            )],
        );
        write_jsonl(
            &projects_dir.join("enc/null-id.jsonl"),
            &[
                serde_json::json!({ "cwd": cwd_2, "sessionId": Value::Null, "timestamp": "2026-07-30T09:00:00Z" }),
            ],
        );

        let outcome = ClaudeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(outcome.diagnostics().is_empty());
        let ids: Vec<&str> = outcome
            .projects()
            .iter()
            .map(|project| project.session_id.as_deref().unwrap())
            .collect();
        assert_eq!(ids, vec!["fallback", "null-id"]);
    });
}

#[test]
fn claude_first_cwd_and_session_id_win_but_latest_git_branch_and_timestamp_win() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let projects_dir = claude_projects(dir.path());
        std::fs::create_dir_all(projects_dir.join("enc")).unwrap();
        let cwd_first = home_join(dir.path(), "work/first");
        let cwd_second = home_join(dir.path(), "work/second");

        write_jsonl(
            &projects_dir.join("enc/multi.jsonl"),
            &[
                claude_line(
                    Some(&cwd_first),
                    Some("id-first"),
                    Some("main"),
                    Some("2026-07-30T09:00:00Z"),
                ),
                claude_line(
                    Some(&cwd_second),
                    Some("id-second"),
                    Some("feature"),
                    Some("2026-07-30T10:00:00Z"),
                ),
            ],
        );

        let outcome = ClaudeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        let project = &outcome.projects()[0];
        assert_eq!(project.path, path::normalize_native(&cwd_first).unwrap());
        assert_eq!(project.session_id.as_deref(), Some("id-first"));
        assert_eq!(project.git_branch.as_deref(), Some("feature"));
        assert_eq!(project.last_accessed_at, rfc3339("2026-07-30T10:00:00Z"));
    });
}

#[test]
fn claude_null_git_branch_keeps_previous_branch() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let projects_dir = claude_projects(dir.path());
        std::fs::create_dir_all(projects_dir.join("enc")).unwrap();
        let cwd_first = home_join(dir.path(), "work/first");
        let cwd_second = home_join(dir.path(), "work/second");

        write_jsonl(
            &projects_dir.join("enc/branch.jsonl"),
            &[
                claude_line(
                    Some(&cwd_first),
                    Some("s"),
                    Some("main"),
                    Some("2026-07-30T09:00:00Z"),
                ),
                serde_json::json!({
                    "cwd": cwd_second,
                    "sessionId": "later",
                    "gitBranch": Value::Null,
                    "timestamp": "2026-07-30T10:00:00Z",
                }),
            ],
        );

        let outcome = ClaudeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        let project = &outcome.projects()[0];
        assert_eq!(project.path, path::normalize_native(&cwd_first).unwrap());
        assert_eq!(project.session_id.as_deref(), Some("s"));
        assert_eq!(project.git_branch.as_deref(), Some("main"));
        assert_eq!(project.last_accessed_at, rfc3339("2026-07-30T10:00:00Z"));
    });
}

#[test]
fn claude_missing_or_malformed_timestamp_falls_back_to_mtime() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let projects_dir = claude_projects(dir.path());
        std::fs::create_dir_all(projects_dir.join("enc")).unwrap();
        let cwd = home_join(dir.path(), "work/repo");

        write_jsonl(
            &projects_dir.join("enc/no-ts.jsonl"),
            &[claude_line(Some(&cwd), Some("no-ts"), Some("main"), None)],
        );
        write_jsonl(
            &projects_dir.join("enc/bad-ts.jsonl"),
            &[serde_json::json!({
                "cwd": cwd,
                "sessionId": "bad-ts",
                "gitBranch": "main",
                "timestamp": "yesterday-ish",
            })],
        );

        let outcome = ClaudeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 2);
        assert_eq!(
            outcome
                .diagnostics()
                .iter()
                .filter(|d| d.code == TIMESTAMP_FALLBACK)
                .count(),
            2
        );
        let now = Utc::now();
        for project in outcome.projects() {
            let delta = now.signed_duration_since(project.last_accessed_at).abs();
            assert!(
                delta < Duration::seconds(60),
                "mtime fallback is recent: {delta}"
            );
        }
    });
}

#[test]
fn claude_missing_source_separates_availability_from_discovery() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let unavailable = ClaudeScanner::with_availability(|| false).scan();
        assert_eq!(unavailable.status(), ScanStatus::Unavailable);
        assert!(unavailable.projects().is_empty());
        assert_eq!(codes(&unavailable), vec![SOURCE_UNAVAILABLE]);

        let empty_success = ClaudeScanner::with_availability(|| true).scan();
        assert_eq!(empty_success.status(), ScanStatus::Succeeded);
        assert!(empty_success.is_successful());
        assert!(empty_success.projects().is_empty());
    });
}

#[test]
fn claude_existing_empty_source_is_trustworthy_empty_success() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        claude_projects(dir.path());
        let outcome = ClaudeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(outcome.projects().is_empty());
        assert!(outcome.diagnostics().is_empty());
    });
}

#[test]
fn claude_source_path_that_is_a_file_is_read_failure() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let projects = dir.path().join(".claude").join("projects");
        std::fs::create_dir_all(projects.parent().unwrap()).unwrap();
        std::fs::write(&projects, b"not a directory").unwrap();

        let outcome = ClaudeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Failed);
        assert!(outcome.projects().is_empty());
        assert_eq!(codes(&outcome), vec![SOURCE_READ_FAILED]);
    });
}

#[test]
fn claude_unreadable_session_file_keeps_valid_files() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let projects_dir = claude_projects(dir.path());
        std::fs::create_dir_all(projects_dir.join("enc")).unwrap();
        let cwd = home_join(dir.path(), "work/repo");

        write_jsonl(
            &projects_dir.join("enc/ok.jsonl"),
            &[claude_line(
                Some(&cwd),
                Some("ok"),
                Some("main"),
                Some("2026-07-30T09:00:00Z"),
            )],
        );
        std::fs::write(
            projects_dir.join("enc/bad.jsonl"),
            b"invalid utf-8 \xff\xfe\n",
        )
        .unwrap();

        let outcome = ClaudeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(outcome.projects()[0].session_id.as_deref(), Some("ok"));
        assert!(has_code(&outcome, SESSION_READ_FAILED));
    });
}

#[test]
fn claude_files_with_no_valid_project_fail_with_no_valid_sessions() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let projects_dir = claude_projects(dir.path());
        std::fs::create_dir_all(projects_dir.join("enc")).unwrap();
        write_jsonl(
            &projects_dir.join("enc/empty.jsonl"),
            &[claude_line(
                None,
                Some("s"),
                None,
                Some("2026-07-30T09:00:00Z"),
            )],
        );

        let outcome = ClaudeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Failed);
        assert!(outcome.projects().is_empty());
        assert!(has_code(&outcome, MISSING_PROJECT_PATH));
        assert!(has_code(&outcome, NO_VALID_SESSIONS));
    });
}

#[test]
fn claude_session_never_leaks_prompt_or_message_content() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let projects_dir = claude_projects(dir.path());
        std::fs::create_dir_all(projects_dir.join("enc")).unwrap();
        let cwd = home_join(dir.path(), "work/repo");
        let file = projects_dir.join("enc/aaa.jsonl");
        let content = format!(
            "{}\n{{\"type\":\"user\",\"message\":{{\"content\":\"{SECRET}\"}}}}\ngarbage mentioning {SECRET}\n",
            serde_json::to_string(&claude_line(Some(&cwd), Some("privacy"), Some("main"), Some("2026-07-30T09:00:00Z"))).unwrap()
        );
        std::fs::write(&file, content).unwrap();

        let outcome = ClaudeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert!(has_code(&outcome, MALFORMED_SESSION_RECORD));
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

#[test]
fn claude_cwd_inside_tool_home_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let projects_dir = claude_projects(dir.path());
        std::fs::create_dir_all(projects_dir.join("enc")).unwrap();
        let inside = projects_dir.join("enc").to_string_lossy().into_owned();
        write_jsonl(
            &projects_dir.join("enc/aaa.jsonl"),
            &[claude_line(
                Some(&inside),
                Some("inside"),
                Some("main"),
                Some("2026-07-30T09:00:00Z"),
            )],
        );

        let outcome = ClaudeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Failed);
        assert!(outcome.projects().is_empty());
        assert!(has_code(&outcome, MISSING_PROJECT_PATH));
    });
}

#[test]
fn claude_normalizes_tilde_and_trailing_separators() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let projects_dir = claude_projects(dir.path());
        std::fs::create_dir_all(projects_dir.join("enc")).unwrap();
        write_jsonl(
            &projects_dir.join("enc/aaa.jsonl"),
            &[claude_line(
                Some("~/work/tilde-repo/"),
                Some("tilde"),
                Some("main"),
                Some("2026-07-30T09:00:00Z"),
            )],
        );

        let outcome = ClaudeScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        let expected = path::normalize_native(&home_join(dir.path(), "work/tilde-repo")).unwrap();
        assert_eq!(outcome.projects()[0].path, expected);
    });
}

#[test]
fn claude_scanner_identity_and_availability_injection() {
    let available = ClaudeScanner::with_availability(|| true);
    assert_eq!(available.tool_key(), "claude");
    assert_eq!(available.tool_name(), "Claude Code");
    assert!(available.is_available());

    let unavailable = ClaudeScanner::with_availability(|| false);
    assert_eq!(unavailable.tool_key(), "claude");
    assert!(!unavailable.is_available());
}

#[test]
fn codex_utf8_bom_and_streaming_jsonl_are_supported() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let sessions = codex_sessions(dir.path());
        let file = sessions.join("bom.jsonl");
        let cwd = home_join(dir.path(), "work/bom-codex");
        let record =
            serde_json::to_string(&codex_meta(&cwd, "codex-bom", "2026-07-30T09:00:00Z")).unwrap();
        std::fs::write(file, format!("\u{feff}{record}\n")).unwrap();

        let outcome = CodexScanner::with_availability(|| false).scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(
            outcome.projects()[0].session_id.as_deref(),
            Some("codex-bom")
        );
    });
}

#[test]
fn claude_utf8_bom_and_streaming_jsonl_are_supported() {
    let dir = tempfile::tempdir().unwrap();
    with_home(dir.path(), || {
        let projects = claude_projects(dir.path());
        let file = projects.join("claude-bom.jsonl");
        let cwd = home_join(dir.path(), "work/bom-claude");
        let record = serde_json::to_string(&claude_line(
            Some(&cwd),
            Some("claude-bom"),
            Some("main"),
            Some("2026-07-30T09:00:00Z"),
        ))
        .unwrap();
        std::fs::write(file, format!("\u{feff}{record}\n")).unwrap();

        let outcome = ClaudeScanner::with_availability(|| false).scan();

        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 1);
        assert_eq!(
            outcome.projects()[0].session_id.as_deref(),
            Some("claude-bom")
        );
    });
}
