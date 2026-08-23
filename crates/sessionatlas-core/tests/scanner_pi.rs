//! Integration tests for the Pi Coding Agent scanner.
//!
//! Every fixture is isolated under a temporary `SESSIONATLAS_HOME`. The tests
//! never read a real Pi session, launch the Pi executable, or retain message
//! content from synthetic JSONL records.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sessionatlas_core::indexer::{build_index, IndexedToolScan};
use sessionatlas_core::path;
use sessionatlas_core::scanner::pi::PiScanner;
use sessionatlas_core::scanner::{
    ScanStatus, Scanner, MALFORMED_SESSION_RECORD, NO_VALID_SESSIONS, SOURCE_READ_FAILED,
    SOURCE_UNAVAILABLE,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());
const SECRET: &str = "PI-PRIVATE-PROMPT-SENTINEL";

fn with_pi_environment<R>(home: &Path, body: impl FnOnce() -> R) -> R {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let names = [
        "SESSIONATLAS_HOME",
        "PI_CODING_AGENT_DIR",
        "PI_CODING_AGENT_SESSION_DIR",
    ];
    let previous: Vec<_> = names
        .iter()
        .map(|name| (*name, std::env::var_os(name)))
        .collect();
    std::env::set_var("SESSIONATLAS_HOME", home);
    std::env::remove_var("PI_CODING_AGENT_DIR");
    std::env::remove_var("PI_CODING_AGENT_SESSION_DIR");

    struct Restore(Vec<(&'static str, Option<std::ffi::OsString>)>);
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

fn sessions(home: &Path) -> PathBuf {
    let path = home.join(".pi").join("agent").join("sessions");
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn write_jsonl(path: &Path, lines: &[Value]) {
    let content = lines
        .iter()
        .map(|line| serde_json::to_string(line).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, format!("{content}\n")).unwrap();
}

fn header(cwd: &Path, id: &str, timestamp: &str) -> Value {
    serde_json::json!({
        "type": "session",
        "version": 3,
        "id": id,
        "timestamp": timestamp,
        "cwd": cwd.to_string_lossy(),
    })
}

fn rfc3339(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn pi_scans_present_and_missing_projects_without_retaining_message_content() {
    let root = tempfile::tempdir().unwrap();
    with_pi_environment(root.path(), || {
        let source = sessions(root.path()).join("--workspace--repo");
        std::fs::create_dir_all(&source).unwrap();
        let present = root.path().join("workspace").join("present");
        let missing = root.path().join("workspace").join("deleted");
        std::fs::create_dir_all(&present).unwrap();
        assert!(!missing.exists());

        write_jsonl(
            &source.join("present.jsonl"),
            &[
                header(&present, "pi-present", "2026-08-16T01:00:00Z"),
                serde_json::json!({
                    "type": "message",
                    "timestamp": "2026-08-16T03:00:00Z",
                    "message": { "role": "user", "content": SECRET }
                }),
            ],
        );
        write_jsonl(
            &source.join("missing.jsonl"),
            &[header(&missing, "pi-missing", "2026-08-16T02:00:00Z")],
        );

        let outcome = PiScanner::with_availability(|| false).scan();
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert_eq!(outcome.projects().len(), 2);
        assert!(outcome.diagnostics().is_empty());
        assert!(!format!("{outcome:?}").contains(SECRET));

        let present_project = outcome
            .projects()
            .iter()
            .find(|project| project.session_id.as_deref() == Some("pi-present"))
            .unwrap();
        assert_eq!(
            present_project.last_accessed_at,
            rfc3339("2026-08-16T03:00:00Z")
        );

        let indexed = build_index(&[IndexedToolScan {
            tool_key: "pi".to_string(),
            tool_name: "Pi Coding Agent".to_string(),
            projects: outcome.into_projects(),
        }]);
        let present_path = path::normalize_native(&present.to_string_lossy()).unwrap();
        let missing_path = path::normalize_native(&missing.to_string_lossy()).unwrap();
        assert!(indexed.iter().any(|project| {
            project.path == present_path
                && !project.path_missing
                && project.tool_usages[0].tool_key == "pi"
        }));
        assert!(indexed.iter().any(|project| {
            project.path == missing_path
                && project.path_missing
                && project.tool_usages[0].last_session_id.as_deref() == Some("pi-missing")
        }));
    });
}

#[test]
fn pi_honors_settings_session_directory_and_environment_precedence() {
    let root = tempfile::tempdir().unwrap();
    with_pi_environment(root.path(), || {
        let agent = root.path().join(".pi").join("agent");
        let configured = agent.join("configured-sessions");
        let environment = root.path().join("environment-sessions");
        std::fs::create_dir_all(&configured).unwrap();
        std::fs::create_dir_all(&environment).unwrap();
        std::fs::write(
            agent.join("settings.json"),
            r#"{"sessionDir":"configured-sessions"}"#,
        )
        .unwrap();
        let configured_project = root.path().join("configured-project");
        let environment_project = root.path().join("environment-project");
        write_jsonl(
            &configured.join("configured.jsonl"),
            &[header(
                &configured_project,
                "configured",
                "2026-08-16T01:00:00Z",
            )],
        );
        write_jsonl(
            &environment.join("environment.jsonl"),
            &[header(
                &environment_project,
                "environment",
                "2026-08-16T02:00:00Z",
            )],
        );

        let configured_outcome = PiScanner::with_availability(|| false).scan();
        assert_eq!(configured_outcome.status(), ScanStatus::Succeeded);
        assert_eq!(configured_outcome.projects().len(), 1);
        assert_eq!(
            configured_outcome.projects()[0].session_id.as_deref(),
            Some("configured")
        );

        std::env::set_var("PI_CODING_AGENT_SESSION_DIR", &environment);
        let environment_outcome = PiScanner::with_availability(|| false).scan();
        assert_eq!(environment_outcome.status(), ScanStatus::Succeeded);
        assert_eq!(environment_outcome.projects().len(), 1);
        assert_eq!(
            environment_outcome.projects()[0].session_id.as_deref(),
            Some("environment")
        );

        std::env::remove_var("PI_CODING_AGENT_SESSION_DIR");
        let alternate_agent = root.path().join("alternate-agent");
        let alternate_sessions = alternate_agent.join("sessions");
        std::fs::create_dir_all(&alternate_sessions).unwrap();
        write_jsonl(
            &alternate_sessions.join("alternate.jsonl"),
            &[header(
                &root.path().join("alternate-project"),
                "alternate-agent",
                "2026-08-16T03:00:00Z",
            )],
        );
        std::env::set_var("PI_CODING_AGENT_DIR", &alternate_agent);
        let alternate_outcome = PiScanner::with_availability(|| false).scan();
        assert_eq!(alternate_outcome.status(), ScanStatus::Succeeded);
        assert_eq!(alternate_outcome.projects().len(), 1);
        assert_eq!(
            alternate_outcome.projects()[0].session_id.as_deref(),
            Some("alternate-agent")
        );
    });
}

#[test]
fn pi_distinguishes_missing_empty_malformed_and_broken_sources() {
    let root = tempfile::tempdir().unwrap();
    with_pi_environment(root.path(), || {
        let unavailable = PiScanner::with_availability(|| false).scan();
        assert_eq!(unavailable.status(), ScanStatus::Unavailable);
        assert_eq!(unavailable.diagnostics()[0].code, SOURCE_UNAVAILABLE);

        let empty = PiScanner::with_availability(|| true).scan();
        assert_eq!(empty.status(), ScanStatus::Succeeded);
        assert!(empty.projects().is_empty());

        let source = sessions(root.path());
        std::fs::write(
            source.join("malformed.jsonl"),
            format!("not-json\n{{\"type\":\"message\",\"content\":\"{SECRET}\"}}\n"),
        )
        .unwrap();
        let malformed = PiScanner::with_availability(|| true).scan();
        assert_eq!(malformed.status(), ScanStatus::Failed);
        assert!(malformed
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == MALFORMED_SESSION_RECORD));
        assert!(malformed
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == NO_VALID_SESSIONS));
        assert!(!format!("{malformed:?}").contains(SECRET));

        std::fs::remove_file(source.join("malformed.jsonl")).unwrap();
        std::fs::write(
            root.path().join(".pi").join("agent").join("settings.json"),
            "{",
        )
        .unwrap();
        let broken = PiScanner::with_availability(|| true).scan();
        assert_eq!(broken.status(), ScanStatus::Failed);
        assert_eq!(broken.diagnostics()[0].code, SOURCE_READ_FAILED);
    });
}
