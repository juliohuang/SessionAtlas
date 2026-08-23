//! Pi Coding Agent scanner: recursive JSONL traversal with header-only project
//! identity extraction and activity-time aggregation.
//!
//! Pi stores sessions under `~/.pi/agent/sessions` by default. The session
//! directory can be overridden by `PI_CODING_AGENT_SESSION_DIR` or the global
//! `settings.json` `sessionDir` value; the environment variable wins. Only the
//! session header's `cwd`, `id`, and timestamps are retained. Message and tool
//! content is never copied into scanner output or diagnostics.

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
use super::custom::executable_on_path;
use super::parsing::{
    expand_tilde, home_directory, recursive_file_enumeration, try_normalize_project_path,
    try_read_utc_timestamp,
};

const AGENT_DIR_ENV: &str = "PI_CODING_AGENT_DIR";
const SESSION_DIR_ENV: &str = "PI_CODING_AGENT_SESSION_DIR";

/// Pi Coding Agent scanner.
pub struct PiScanner {
    is_command_available: Box<dyn Fn() -> bool>,
}

impl PiScanner {
    /// Scanner probing whether the `pi` executable is reachable on PATH.
    pub fn new() -> Self {
        Self::with_availability(|| executable_on_path("pi"))
    }

    /// Scanner with an injected availability predicate for isolated tests.
    pub fn with_availability(is_available: impl Fn() -> bool + 'static) -> Self {
        Self {
            is_command_available: Box::new(is_available),
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
        (self.is_command_available)()
    }

    fn scan(&self) -> ScanOutcome {
        self.scan_source()
    }
}

impl PiScanner {
    fn scan_source(&self) -> ScanOutcome {
        let Some(home) = home_directory() else {
            return missing_source(self.tool_key(), self.is_available());
        };
        let Some(agent_dir) = resolve_agent_directory(&home) else {
            return source_read_failure(self.tool_key(), "the configured Pi agent directory");
        };
        let sessions_dir = match resolve_sessions_directory(&agent_dir) {
            Ok(directory) => directory,
            Err(()) => {
                return source_read_failure(self.tool_key(), "the Pi settings file");
            }
        };

        match probe_directory(&sessions_dir) {
            SourceProbe::Missing => return missing_source(self.tool_key(), self.is_available()),
            SourceProbe::Failed => {
                return source_read_failure(self.tool_key(), "the Pi sessions directory");
            }
            SourceProbe::Exists => {}
        }

        let session_files = match enumerate_jsonl_files(&sessions_dir) {
            Ok(files) => files,
            Err(()) => return source_read_failure(self.tool_key(), "the Pi sessions directory"),
        };
        let mut projects = Vec::new();
        let mut diagnostics = Vec::new();
        let agent_root = agent_dir.to_string_lossy().into_owned();
        for session_file in &session_files {
            parse_session_file(session_file, &agent_root, &mut projects, &mut diagnostics);
        }

        complete_session_files(self.tool_key(), session_files.len(), projects, diagnostics)
    }
}

fn resolve_agent_directory(home: &Path) -> Option<PathBuf> {
    match nonblank_env_path(AGENT_DIR_ENV) {
        Some(path) => resolve_path(path, None),
        None => Some(home.join(".pi").join("agent")),
    }
}

fn resolve_sessions_directory(agent_dir: &Path) -> Result<PathBuf, ()> {
    if let Some(path) = nonblank_env_path(SESSION_DIR_ENV) {
        return resolve_path(path, None).ok_or(());
    }

    let settings_path = agent_dir.join("settings.json");
    match probe_file(&settings_path) {
        SourceProbe::Missing => {}
        SourceProbe::Failed => return Err(()),
        SourceProbe::Exists => {
            let bytes = std::fs::read(&settings_path).map_err(|_| ())?;
            let settings: Value = serde_json::from_slice(strip_utf8_bom(&bytes)).map_err(|_| ())?;
            let settings = settings.as_object().ok_or(())?;
            if let Some(configured) = settings.get("sessionDir") {
                let Some(configured) = configured.as_str() else {
                    return Err(());
                };
                if !configured.trim().is_empty() {
                    return resolve_path(PathBuf::from(configured.trim()), Some(agent_dir))
                        .ok_or(());
                }
            }
        }
    }

    Ok(agent_dir.join("sessions"))
}

fn nonblank_env_path(name: &str) -> Option<PathBuf> {
    let value = std::env::var_os(name)?;
    if value.to_string_lossy().trim().is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

fn resolve_path(path: PathBuf, relative_base: Option<&Path>) -> Option<PathBuf> {
    let text = path.to_string_lossy();
    let expanded = expand_tilde(&text).map(PathBuf::from).unwrap_or(path);
    if expanded.is_absolute() {
        return Some(expanded);
    }
    match relative_base {
        Some(base) => Some(base.join(expanded)),
        None => std::path::absolute(expanded).ok(),
    }
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
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
    agent_root: &str,
    projects: &mut Vec<ScannedProject>,
    diagnostics: &mut Vec<ScanDiagnostic>,
) {
    let file = match std::fs::File::open(session_file) {
        Ok(file) => file,
        Err(_) => {
            diagnostics.push(read_failure());
            return;
        }
    };

    let mut project_path: Option<String> = None;
    let mut session_id: Option<String> = None;
    let mut latest_activity: Option<DateTime<Utc>> = None;
    let mut malformed_lines = 0usize;
    let mut first_line = true;

    for line in BufReader::new(file).lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => {
                diagnostics.push(read_failure());
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
        if record.get("type").and_then(Value::as_str) != Some("session") {
            continue;
        }
        if let Some(cwd) = record.get("cwd").and_then(Value::as_str) {
            project_path = Some(cwd.to_owned());
        }
        if let Some(id) = record.get("id").and_then(Value::as_str) {
            session_id = Some(id.to_owned());
        }
    }

    if malformed_lines > 0 {
        diagnostics.push(ScanDiagnostic::new(
            "pi",
            ScanDiagnosticSeverity::Warning,
            MALFORMED_SESSION_RECORD,
            format!(
                "A Pi session contained {malformed_lines} malformed record(s); valid records were retained."
            ),
        ));
    }

    let Some(normalized_path) = project_path
        .as_deref()
        .and_then(|candidate| try_normalize_project_path(candidate, agent_root))
    else {
        diagnostics.push(ScanDiagnostic::new(
            "pi",
            ScanDiagnosticSeverity::Warning,
            MISSING_PROJECT_PATH,
            "A Pi session did not contain a safe absolute project path and was skipped.",
        ));
        return;
    };
    let Some(session_id) = session_id.filter(|id| !id.trim().is_empty()) else {
        diagnostics.push(ScanDiagnostic::new(
            "pi",
            ScanDiagnosticSeverity::Warning,
            MISSING_SESSION_ID,
            "A Pi session did not contain a native session ID and was skipped.",
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
                "A Pi session had no valid activity timestamp; file modification time was used.",
            ));
            let Some(timestamp) = file_last_write_utc(session_file) else {
                return;
            };
            timestamp
        }
    };

    projects.push(ScannedProject {
        path: normalized_path,
        last_accessed_at,
        session_id: Some(session_id),
        git_branch: None,
    });
}

fn read_failure() -> ScanDiagnostic {
    ScanDiagnostic::new(
        "pi",
        ScanDiagnosticSeverity::Warning,
        SESSION_READ_FAILED,
        "A Pi session file could not be read and was skipped.",
    )
}

fn file_last_write_utc(path: &Path) -> Option<DateTime<Utc>> {
    Some(std::fs::metadata(path).ok()?.modified().ok()?.into())
}
