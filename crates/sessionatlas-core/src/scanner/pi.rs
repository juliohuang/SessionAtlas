//! Pi Coding Agent scanner for JSONL sessions under
//! `~/.pi/agent/sessions/<encoded-working-directory>/`.
//!
//! Pi writes a `type: "session"` header containing the native session ID,
//! creation timestamp, and absolute `cwd`. Session entries are JSONL records
//! with their own timestamps. SessionAtlas extracts only those metadata
//! fields; prompts, responses, tool calls, and other conversation content are
//! never retained.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::base::{
    complete_session_files, missing_source, probe_directory, probe_file, source_read_failure,
    ScanDiagnostic, ScanDiagnosticSeverity, ScanOutcome, ScannedProject, Scanner, SourceProbe,
    MALFORMED_SESSION_RECORD, MISSING_PROJECT_PATH, MISSING_SESSION_ID, SESSION_READ_FAILED,
    TIMESTAMP_FALLBACK,
};
use super::parsing::{
    expand_tilde, home_directory, recursive_file_enumeration, try_normalize_project_path,
    try_read_unix_timestamp, try_read_utc_timestamp,
};

const AGENT_DIR_ENV: &str = "PI_CODING_AGENT_DIR";
const SESSION_DIR_ENV: &str = "PI_CODING_AGENT_SESSION_DIR";

/// Scanner for Pi Coding Agent's persisted JSONL sessions.
pub struct PiScanner {
    is_available: Box<dyn Fn() -> bool>,
}

impl PiScanner {
    /// Availability defaults to whether the `pi` executable is on `PATH`;
    /// historical sessions remain discoverable if the executable is removed.
    pub fn new() -> Self {
        Self::with_availability(|| command_available("pi"))
    }

    /// Availability override so tests can pin outcomes deterministically.
    pub fn with_availability(availability: impl Fn() -> bool + 'static) -> Self {
        Self {
            is_available: Box::new(availability),
        }
    }
}

impl Default for PiScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Scanner for PiScanner {
    fn tool_key(&self) -> &str {
        "pi"
    }

    fn tool_name(&self) -> &str {
        "Pi Coding Agent"
    }

    fn is_available(&self) -> bool {
        (self.is_available)()
    }

    fn scan(&self) -> ScanOutcome {
        self.scan_source()
    }
}

impl PiScanner {
    fn scan_source(&self) -> ScanOutcome {
        let Some(agent_dir) = resolve_agent_dir() else {
            return missing_source("pi", self.is_available());
        };
        let sessions_dir = match resolve_sessions_dir(&agent_dir) {
            Ok(path) => path,
            Err(()) => return source_read_failure("pi", "the Pi Coding Agent settings file"),
        };
        match probe_directory(&sessions_dir) {
            SourceProbe::Missing => return missing_source("pi", self.is_available()),
            SourceProbe::Failed => {
                return source_read_failure("pi", "the Pi Coding Agent sessions directory");
            }
            SourceProbe::Exists => {}
        }

        let session_files = match enumerate_jsonl_files(&sessions_dir) {
            Ok(files) => files,
            Err(()) => {
                return source_read_failure("pi", "the Pi Coding Agent sessions directory");
            }
        };

        let mut projects = Vec::new();
        let mut diagnostics = Vec::new();
        let source_root = agent_dir.to_string_lossy().into_owned();
        for session_file in &session_files {
            parse_session_file(session_file, &source_root, &mut projects, &mut diagnostics);
        }

        complete_session_files("pi", session_files.len(), projects, diagnostics)
    }
}

fn resolve_agent_dir() -> Option<PathBuf> {
    let home = home_directory()?;
    let Some(value) = non_blank_env(AGENT_DIR_ENV) else {
        return Some(home.join(".pi").join("agent"));
    };
    if let Some(expanded) = expand_tilde(&value) {
        return Some(PathBuf::from(expanded));
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Some(path)
    } else {
        std::path::absolute(path).ok()
    }
}

/// Resolve the official override first, then the global `settings.json`
/// `sessionDir`, then Pi's default `~/.pi/agent/sessions` location.
fn resolve_sessions_dir(agent_dir: &Path) -> Result<PathBuf, ()> {
    if let Some(value) = non_blank_env(SESSION_DIR_ENV) {
        return Ok(resolve_configured_path(&value, agent_dir));
    }

    let settings_path = agent_dir.join("settings.json");
    match probe_file(&settings_path) {
        SourceProbe::Missing => {}
        SourceProbe::Failed => return Err(()),
        SourceProbe::Exists => {
            let content = std::fs::read_to_string(settings_path).map_err(|_| ())?;
            let settings =
                serde_json::from_str::<Value>(content.strip_prefix('\u{feff}').unwrap_or(&content))
                    .map_err(|_| ())?;
            let settings = settings.as_object().ok_or(())?;
            if let Some(value) = settings.get("sessionDir") {
                let value = value.as_str().ok_or(())?.trim();
                if !value.is_empty() {
                    return Ok(resolve_configured_path(value, agent_dir));
                }
            }
        }
    }

    Ok(agent_dir.join("sessions"))
}

fn non_blank_env(name: &str) -> Option<String> {
    std::env::var_os(name)
        .map(|value| value.to_string_lossy().trim().to_string())
        .filter(|value| !value.is_empty())
}

fn resolve_configured_path(value: &str, agent_dir: &Path) -> PathBuf {
    if let Some(expanded) = expand_tilde(value) {
        return PathBuf::from(expanded);
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        agent_dir.join(path)
    }
}

fn enumerate_jsonl_files(sessions_dir: &Path) -> Result<Vec<PathBuf>, ()> {
    let mut files = Vec::new();
    for entry in recursive_file_enumeration(sessions_dir) {
        let entry = entry.map_err(|_| ())?;
        if entry.file_type().is_file() && is_jsonl(entry.path()) {
            files.push(entry.into_path());
        }
    }
    files.sort();
    Ok(files)
}

fn is_jsonl(path: &Path) -> bool {
    let Some(extension) = path.extension() else {
        return false;
    };
    if cfg!(windows) {
        extension.eq_ignore_ascii_case("jsonl")
    } else {
        extension == "jsonl"
    }
}

fn parse_session_file(
    session_file: &Path,
    source_root: &str,
    projects: &mut Vec<ScannedProject>,
    diagnostics: &mut Vec<ScanDiagnostic>,
) {
    let file = match std::fs::File::open(session_file) {
        Ok(file) => file,
        Err(_) => {
            diagnostics.push(ScanDiagnostic::new(
                "pi",
                ScanDiagnosticSeverity::Warning,
                SESSION_READ_FAILED,
                "A Pi Coding Agent session file could not be read and was skipped.",
            ));
            return;
        }
    };

    let mut project_path: Option<String> = None;
    let mut session_id: Option<String> = None;
    let mut latest_activity: Option<DateTime<Utc>> = None;
    let mut malformed_lines = 0;
    let mut first_line = true;

    for line in BufReader::new(file).lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => {
                diagnostics.push(ScanDiagnostic::new(
                    "pi",
                    ScanDiagnosticSeverity::Warning,
                    SESSION_READ_FAILED,
                    "A Pi Coding Agent session file could not be read and was skipped.",
                ));
                return;
            }
        };
        let line = if first_line {
            first_line = false;
            line.strip_prefix('\u{feff}').unwrap_or(&line)
        } else {
            &line
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            malformed_lines += 1;
            continue;
        };

        if let Some(timestamp) = record.get("timestamp").and_then(try_read_utc_timestamp) {
            latest_activity = latest_activity.max(Some(timestamp));
        }
        if let Some(timestamp) = record
            .get("message")
            .and_then(|message| message.get("timestamp"))
            .and_then(Value::as_i64)
            .and_then(try_read_unix_timestamp)
        {
            latest_activity = latest_activity.max(Some(timestamp));
        }

        if record.get("type").and_then(Value::as_str) == Some("session") {
            if session_id.is_none() {
                session_id = record.get("id").and_then(Value::as_str).map(str::to_owned);
            }
            if project_path.is_none() {
                project_path = record.get("cwd").and_then(Value::as_str).map(str::to_owned);
            }
        }
    }

    if malformed_lines > 0 {
        diagnostics.push(ScanDiagnostic::new(
            "pi",
            ScanDiagnosticSeverity::Warning,
            MALFORMED_SESSION_RECORD,
            format!(
                "A Pi Coding Agent session contained {malformed_lines} malformed record(s); valid records were retained."
            ),
        ));
    }

    let Some(normalized_path) = project_path
        .as_deref()
        .and_then(|candidate| try_normalize_project_path(candidate, source_root))
    else {
        diagnostics.push(ScanDiagnostic::new(
            "pi",
            ScanDiagnosticSeverity::Warning,
            MISSING_PROJECT_PATH,
            "A Pi Coding Agent session did not contain a safe absolute cwd and was skipped.",
        ));
        return;
    };

    let Some(session_id) = session_id.filter(|id| !id.trim().is_empty()) else {
        diagnostics.push(ScanDiagnostic::new(
            "pi",
            ScanDiagnosticSeverity::Warning,
            MISSING_SESSION_ID,
            "A Pi Coding Agent session did not contain a native session ID and was skipped.",
        ));
        return;
    };

    let last_accessed_at = match latest_activity {
        Some(timestamp) => timestamp,
        None => {
            diagnostics.push(ScanDiagnostic::new(
                "pi",
                ScanDiagnosticSeverity::Warning,
                TIMESTAMP_FALLBACK,
                "A Pi Coding Agent session had no valid activity timestamp; file modification time was used.",
            ));
            let Some(fallback) = file_last_write_utc(session_file) else {
                return;
            };
            fallback
        }
    };

    projects.push(ScannedProject {
        path: normalized_path,
        last_accessed_at,
        session_id: Some(session_id),
        git_branch: None,
    });
}

fn file_last_write_utc(path: &Path) -> Option<DateTime<Utc>> {
    Some(std::fs::metadata(path).ok()?.modified().ok()?.into())
}

fn command_available(command: &str) -> bool {
    let Some(path_value) = std::env::var_os("PATH") else {
        return false;
    };
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var_os("PATHEXT")
            .map(|value| {
                std::env::split_paths(&value)
                    .map(|extension| extension.to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_else(|| {
                vec![
                    String::new(),
                    ".exe".to_string(),
                    ".cmd".to_string(),
                    ".bat".to_string(),
                ]
            })
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&path_value).any(|directory| {
        extensions
            .iter()
            .any(|extension| directory.join(format!("{command}{extension}")).is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{ScanStatus, MALFORMED_SESSION_RECORD};

    fn with_home<R>(path: &Path, body: impl FnOnce() -> R) -> R {
        let _guard = crate::scanner::parsing::ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let previous_home = std::env::var_os("SESSIONATLAS_HOME");
        let previous_sessions = std::env::var_os(SESSION_DIR_ENV);
        std::env::set_var("SESSIONATLAS_HOME", path);
        std::env::remove_var(SESSION_DIR_ENV);
        struct Restore(Option<std::ffi::OsString>, Option<std::ffi::OsString>);
        impl Drop for Restore {
            fn drop(&mut self) {
                match &self.0 {
                    Some(value) => std::env::set_var("SESSIONATLAS_HOME", value),
                    None => std::env::remove_var("SESSIONATLAS_HOME"),
                }
                match &self.1 {
                    Some(value) => std::env::set_var(SESSION_DIR_ENV, value),
                    None => std::env::remove_var(SESSION_DIR_ENV),
                }
            }
        }
        let _restore = Restore(previous_home, previous_sessions);
        body()
    }

    #[test]
    fn pi_scanner_reads_header_and_latest_activity_without_conversation_content() {
        let home = tempfile::tempdir().unwrap();
        let project = home.path().join("work").join("repo");
        std::fs::create_dir_all(&project).unwrap();
        let session_dir = home.path().join(".pi/agent/sessions/--work-repo--");
        std::fs::create_dir_all(&session_dir).unwrap();
        let session_file = session_dir.join("2026-08-17T00-00-00-000Z_session-1.jsonl");
        let content = format!(
            "{{\"type\":\"session\",\"version\":3,\"id\":\"session-1\",\"timestamp\":\"2026-08-17T00:00:00Z\",\"cwd\":{}}}\n\
             {{\"type\":\"message\",\"id\":\"m1\",\"timestamp\":\"2026-08-17T01:00:00Z\",\"message\":{{\"role\":\"user\",\"content\":\"private prompt\"}}}}\n",
            serde_json::to_string(&project.to_string_lossy()).unwrap()
        );
        std::fs::write(&session_file, content).unwrap();

        with_home(home.path(), || {
            let outcome = PiScanner::with_availability(|| true).scan();
            assert_eq!(outcome.status(), ScanStatus::Succeeded);
            assert_eq!(outcome.projects().len(), 1);
            assert_eq!(
                outcome.projects()[0].session_id.as_deref(),
                Some("session-1")
            );
            assert_eq!(
                outcome.projects()[0].last_accessed_at.to_rfc3339(),
                "2026-08-17T01:00:00+00:00"
            );
        });
    }

    #[test]
    fn pi_scanner_retains_valid_metadata_when_another_line_is_malformed() {
        let home = tempfile::tempdir().unwrap();
        let project = home.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let session_dir = home.path().join(".pi/agent/sessions/project");
        std::fs::create_dir_all(&session_dir).unwrap();
        let session_file = session_dir.join("session.jsonl");
        let content = format!(
            "{{\"type\":\"session\",\"id\":\"safe-id\",\"timestamp\":\"2026-08-17T00:00:00Z\",\"cwd\":{}}}\nnot-json\n",
            serde_json::to_string(&project.to_string_lossy()).unwrap()
        );
        std::fs::write(session_file, content).unwrap();

        with_home(home.path(), || {
            let outcome = PiScanner::with_availability(|| true).scan();
            assert_eq!(outcome.status(), ScanStatus::Succeeded);
            assert_eq!(outcome.projects().len(), 1);
            assert!(outcome
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == MALFORMED_SESSION_RECORD));
        });
    }

    #[test]
    fn pi_session_directory_override_has_priority() {
        let home = tempfile::tempdir().unwrap();
        let custom = home.path().join("custom-sessions");
        with_home(home.path(), || {
            std::env::set_var(SESSION_DIR_ENV, &custom);
            assert_eq!(
                resolve_sessions_dir(&resolve_agent_dir().unwrap()).unwrap(),
                custom
            );
        });
    }
}
