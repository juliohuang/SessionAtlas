//! Claude Code scanner: recursive JSONL traversal, filename session-id
//! fallback, git-branch capture, timestamp parsing and fallback.
//!
//! Only the project path, session
//! ID, git branch and activity timestamp are extracted; prompt and message
//! content is never read into the output.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::base::{
    complete_session_files, missing_source, probe_directory, source_read_failure, ScanDiagnostic,
    ScanDiagnosticSeverity, ScanOutcome, ScannedProject, Scanner, SourceProbe,
    MALFORMED_SESSION_RECORD, MISSING_PROJECT_PATH, SESSION_READ_FAILED, TIMESTAMP_FALLBACK,
};
use super::parsing::{
    home_directory, recursive_file_enumeration, try_normalize_project_path, try_read_utc_timestamp,
};

/// Claude Code scanner for project-bucket JSONL sessions under
/// `~/.claude/projects/`.
pub struct ClaudeScanner {
    is_available: Box<dyn Fn() -> bool>,
}

impl ClaudeScanner {
    /// Availability defaults to whether `claude` is on `PATH`; historical data
    /// stays discoverable regardless of the executable.
    pub fn new() -> Self {
        Self::with_availability(|| command_available("claude"))
    }

    /// Availability override so tests can pin the outcome deterministically.
    pub fn with_availability(availability: impl Fn() -> bool + 'static) -> Self {
        Self {
            is_available: Box::new(availability),
        }
    }
}

impl Default for ClaudeScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Scanner for ClaudeScanner {
    fn tool_key(&self) -> &str {
        "claude"
    }

    fn tool_name(&self) -> &str {
        "Claude Code"
    }

    fn is_available(&self) -> bool {
        (self.is_available)()
    }

    fn scan(&self) -> ScanOutcome {
        self.scan_source()
    }
}

impl ClaudeScanner {
    fn scan_source(&self) -> ScanOutcome {
        let Some(home) = home_directory() else {
            return missing_source("claude", self.is_available());
        };
        let claude_home = home.join(".claude");
        let projects_dir = claude_home.join("projects");
        match probe_directory(&projects_dir) {
            SourceProbe::Missing => return missing_source("claude", self.is_available()),
            SourceProbe::Failed => {
                return source_read_failure("claude", "the Claude Code projects directory");
            }
            SourceProbe::Exists => {}
        }

        let session_files = match enumerate_jsonl_files(&projects_dir) {
            Ok(files) => files,
            Err(()) => {
                return source_read_failure("claude", "the Claude Code projects directory");
            }
        };

        let mut projects = Vec::new();
        let mut diagnostics = Vec::new();
        let claude_home = claude_home.to_string_lossy().into_owned();
        for session_file in &session_files {
            parse_session_file(session_file, &claude_home, &mut projects, &mut diagnostics);
        }

        complete_session_files("claude", session_files.len(), projects, diagnostics)
    }
}

/// Recursively enumerates `*.jsonl` session files in ordinal path order.
/// Any inaccessible entry surfaces as `Err(())`; enumeration stops on the
/// first unreadable path.
fn enumerate_jsonl_files(projects_dir: &Path) -> Result<Vec<PathBuf>, ()> {
    let mut files = Vec::new();
    for entry in recursive_file_enumeration(projects_dir) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => return Err(()),
        };
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

/// Parses one session file: the first `cwd` and the first `sessionId` win, the
/// last valid `gitBranch` and the greatest activity timestamp win. Malformed
/// lines are counted into a diagnostic and never abort valid records.
fn parse_session_file(
    session_file: &Path,
    claude_home: &str,
    projects: &mut Vec<ScannedProject>,
    diagnostics: &mut Vec<ScanDiagnostic>,
) {
    let file = match std::fs::File::open(session_file) {
        Ok(file) => file,
        Err(_) => {
            diagnostics.push(ScanDiagnostic::new(
                "claude",
                ScanDiagnosticSeverity::Warning,
                SESSION_READ_FAILED,
                "A Claude Code session file could not be read and was skipped.",
            ));
            return;
        }
    };

    let mut project_path: Option<String> = None;
    let mut session_id: Option<String> = None;
    let mut git_branch: Option<String> = None;
    let mut latest_activity: Option<DateTime<Utc>> = None;
    let mut malformed_lines = 0;
    let mut first_line = true;

    for line in BufReader::new(file).lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => {
                diagnostics.push(ScanDiagnostic::new(
                    "claude",
                    ScanDiagnosticSeverity::Warning,
                    SESSION_READ_FAILED,
                    "A Claude Code session file could not be read and was skipped.",
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

        if project_path.is_none() {
            if let Some(cwd) = record.get("cwd").and_then(Value::as_str) {
                project_path = Some(cwd.to_owned());
            }
        }
        if session_id.is_none() {
            if let Some(id) = record.get("sessionId").and_then(Value::as_str) {
                session_id = Some(id.to_owned());
            }
        }
        if let Some(branch) = record.get("gitBranch").and_then(Value::as_str) {
            git_branch = Some(branch.to_owned());
        }
        if let Some(timestamp) = record.get("timestamp").and_then(try_read_utc_timestamp) {
            latest_activity = latest_activity.max(Some(timestamp));
        }
    }

    if malformed_lines > 0 {
        diagnostics.push(ScanDiagnostic::new(
            "claude",
            ScanDiagnosticSeverity::Warning,
            MALFORMED_SESSION_RECORD,
            format!(
                "A Claude Code session contained {malformed_lines} malformed record(s); valid records were retained."
            ),
        ));
    }

    let Some(normalized_path) = project_path
        .as_deref()
        .and_then(|candidate| try_normalize_project_path(candidate, claude_home))
    else {
        diagnostics.push(ScanDiagnostic::new(
            "claude",
            ScanDiagnosticSeverity::Warning,
            MISSING_PROJECT_PATH,
            "A Claude Code session did not contain a safe absolute project path and was skipped.",
        ));
        return;
    };

    let session_id = session_id.or_else(|| {
        session_file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
    });

    let last_accessed_at = match latest_activity {
        Some(timestamp) => timestamp,
        None => {
            diagnostics.push(ScanDiagnostic::new(
                "claude",
                ScanDiagnosticSeverity::Warning,
                TIMESTAMP_FALLBACK,
                "A Claude Code session had no valid activity timestamp; file modification time was used.",
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
        session_id,
        git_branch,
    });
}

/// Reads the file modification time as a UTC timestamp. Returns `None` only when the metadata
/// is unavailable despite a successful read moments earlier.
fn file_last_write_utc(path: &Path) -> Option<DateTime<Utc>> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    Some(modified.into())
}

/// Whether a command executable is reachable on `PATH` without launching it.
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
