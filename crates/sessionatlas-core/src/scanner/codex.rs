//! Codex scanner: recursive JSONL traversal, minimal field extraction,
//! bad-line tolerance, time fallback, missing-session-id handling.
//!
//! Only the project path, native
//! session ID and activity timestamp are extracted from each record; prompt,
//! message and other session content is never read into the output.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::base::{
    bounded_recursive_files, complete_session_files, missing_source, no_follow_metadata,
    probe_directory, source_read_failure, BoundedLines, BudgetError, ScanBudget, ScanContext,
    ScanDiagnostic, ScanDiagnosticSeverity, ScanOutcome, ScannedProject, Scanner, SourceProbe,
    AUXILIARY_SESSION_FILTERED, MALFORMED_SESSION_RECORD, MISSING_PROJECT_PATH, MISSING_SESSION_ID,
    SESSION_READ_FAILED, TIMESTAMP_FALLBACK,
};
use super::cache::FileCache;
use super::parsing::{home_directory, try_normalize_project_path, try_read_utc_timestamp};

/// Codex CLI scanner for date-nested rollout JSONL files under
/// `~/.codex/sessions/YYYY/MM/DD/`.
pub struct CodexScanner {
    is_available: Box<dyn Fn() -> bool>,
    budget: ScanBudget,
}

const PARSER_VERSION: u32 = 3;
/// Small rollouts retain exact per-record timestamps and malformed-line
/// diagnostics. Larger append-only transcripts only need their metadata head
/// plus modification time, so scan cost does not grow with conversation text.
const FULL_SCAN_MAX_BYTES: u64 = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodexSessionKind {
    Primary,
    Child,
    Guardian,
    Auxiliary,
}

impl CodexSessionKind {
    /// Auxiliary markers are sticky for a rollout. A malformed or duplicated
    /// metadata tail must not turn a child/guardian file back into a resume
    /// target after an earlier metadata record identified it as auxiliary.
    fn merge(self, observed: Self) -> Self {
        match (self, observed) {
            (Self::Guardian, _) | (_, Self::Guardian) => Self::Guardian,
            (Self::Child, _) | (_, Self::Child) => Self::Child,
            (Self::Auxiliary, _) | (_, Self::Auxiliary) => Self::Auxiliary,
            _ => Self::Primary,
        }
    }
}

#[derive(Default)]
struct AuxiliaryCounts {
    child: usize,
    guardian: usize,
    other: usize,
}

impl AuxiliaryCounts {
    fn observe(&mut self, kind: CodexSessionKind) {
        match kind {
            CodexSessionKind::Primary => {}
            CodexSessionKind::Child => self.child += 1,
            CodexSessionKind::Guardian => self.guardian += 1,
            CodexSessionKind::Auxiliary => self.other += 1,
        }
    }

    fn total(&self) -> usize {
        self.child + self.guardian + self.other
    }
}

impl CodexScanner {
    /// Availability defaults to whether `codex` is on `PATH`; historical data
    /// stays discoverable regardless of the executable.
    pub fn new() -> Self {
        Self::with_availability(|| command_available("codex"))
    }

    /// Availability override so tests can pin the outcome deterministically.
    pub fn with_availability(availability: impl Fn() -> bool + 'static) -> Self {
        Self {
            is_available: Box::new(availability),
            budget: ScanBudget::default(),
        }
    }
    pub fn with_budget(mut self, budget: ScanBudget) -> Self {
        self.budget = budget;
        self
    }
}

impl Default for CodexScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Scanner for CodexScanner {
    fn tool_key(&self) -> &str {
        "codex"
    }

    fn tool_name(&self) -> &str {
        "Codex CLI"
    }

    fn is_available(&self) -> bool {
        (self.is_available)()
    }

    fn scan(&self) -> ScanOutcome {
        self.scan_source()
    }
}

impl CodexScanner {
    fn scan_source(&self) -> ScanOutcome {
        let Some(home) = home_directory() else {
            return missing_source("codex", self.is_available());
        };
        let codex_home = home.join(".codex");
        let sessions_dir = codex_home.join("sessions");
        match probe_directory(&sessions_dir) {
            SourceProbe::Missing => return missing_source("codex", self.is_available()),
            SourceProbe::Failed => {
                return source_read_failure("codex", "the Codex sessions directory");
            }
            SourceProbe::Exists => {}
        }

        let context = self.budget.context();
        let session_files = match enumerate_jsonl_files(&sessions_dir, &context) {
            Ok(files) => files,
            Err(error) => return ScanOutcome::failed([context.diagnostic("codex", error)]),
        };

        let mut projects = Vec::new();
        let mut diagnostics = Vec::new();
        let mut auxiliary_counts = AuxiliaryCounts::default();
        let codex_home = codex_home.to_string_lossy().into_owned();
        let mut cache =
            FileCache::load_with_limit(&home, PARSER_VERSION, context.budget.max_cache_bytes);
        cache.retain_paths("codex", &session_files);
        for session_file in &session_files {
            let metadata = match no_follow_metadata(session_file) {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => {
                    return ScanOutcome::failed([
                        context.diagnostic("codex", BudgetError::Exceeded)
                    ]);
                }
            };
            // Codex JSONL files are append-only transcripts and can grow far
            // beyond the generic whole-file limit. Charge the source entry
            // here; BoundedLines separately charges only bytes actually read.
            if context.source_file(0).is_err() {
                return ScanOutcome::failed([context.diagnostic("codex", BudgetError::Exceeded)]);
            }
            if let Some(cached) = cache.get("codex", session_file) {
                for _ in &cached {
                    if let Err(error) = context.record() {
                        return ScanOutcome::failed([context.diagnostic("codex", error)]);
                    }
                }
                projects.extend(cached);
                continue;
            }
            let project_start = projects.len();
            let diagnostic_start = diagnostics.len();
            let scan_full_file =
                metadata.len() <= FULL_SCAN_MAX_BYTES.min(context.budget.max_file_bytes);
            let kind = match parse_session_file(
                session_file,
                &codex_home,
                &context,
                scan_full_file,
                &mut projects,
                &mut diagnostics,
            ) {
                Ok(kind) => kind,
                Err(error) => {
                    return ScanOutcome::failed([context.diagnostic("codex", error)]);
                }
            };
            if let Some(kind) = kind {
                auxiliary_counts.observe(kind);
            }
            let added_projects = &projects[project_start..];
            if diagnostics.len() == diagnostic_start
                && !added_projects.is_empty()
                && added_projects
                    .iter()
                    .all(|project| project.session_id.is_some())
            {
                cache.record("codex", session_file, &projects[project_start..]);
            }
        }
        if auxiliary_counts.total() > 0 {
            diagnostics.push(ScanDiagnostic::new(
                "codex",
                ScanDiagnosticSeverity::Info,
                AUXILIARY_SESSION_FILTERED,
                format!(
                    "Kept {} child session(s), {} guardian review session(s), and {} other auxiliary session(s) as activity, but excluded them from resume targets.",
                    auxiliary_counts.child, auxiliary_counts.guardian, auxiliary_counts.other
                ),
            ));
        }
        let outcome = complete_session_files("codex", session_files.len(), projects, diagnostics);
        if outcome.is_successful() {
            cache.save();
        }
        outcome
    }
}

/// Recursively enumerates `*.jsonl` session files in ordinal path order.
/// Any inaccessible entry surfaces as `Err(())`; enumeration stops on the
/// first unreadable path.
fn enumerate_jsonl_files(
    sessions_dir: &Path,
    context: &ScanContext,
) -> Result<Vec<PathBuf>, BudgetError> {
    let mut files = Vec::new();
    for path in bounded_recursive_files(sessions_dir, context)? {
        if is_jsonl(&path) {
            files.push(path);
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

/// Parses one session file. Small files retain the last `session_meta` record
/// and greatest record timestamp. Large append-only rollouts stop after their
/// metadata head and use modification time for current activity. This keeps
/// scanning proportional to session count rather than transcript volume.
fn parse_session_file(
    session_file: &Path,
    codex_home: &str,
    context: &ScanContext,
    scan_full_file: bool,
    projects: &mut Vec<ScannedProject>,
    diagnostics: &mut Vec<ScanDiagnostic>,
) -> Result<Option<CodexSessionKind>, BudgetError> {
    let mut lines = match BoundedLines::open(session_file, context) {
        Ok(lines) => lines,
        Err(crate::scanner::BoundedLineError::Budget(error)) => return Err(error),
        Err(crate::scanner::BoundedLineError::Io) => {
            diagnostics.push(ScanDiagnostic::new(
                "codex",
                ScanDiagnosticSeverity::Warning,
                SESSION_READ_FAILED,
                "A Codex session file could not be read and was skipped.",
            ));
            return Ok(None);
        }
    };

    let mut project_path: Option<String> = None;
    let mut session_id: Option<String> = None;
    let mut latest_activity: Option<DateTime<Utc>> = None;
    let mut session_kind = CodexSessionKind::Primary;
    let mut malformed_lines = 0;
    let mut first_line = true;
    let mut head_bytes = 0u64;

    loop {
        let raw_line = match lines.next_line() {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(crate::scanner::BoundedLineError::Budget(error)) => return Err(error),
            Err(crate::scanner::BoundedLineError::Io) => {
                diagnostics.push(ScanDiagnostic::new(
                    "codex",
                    ScanDiagnosticSeverity::Warning,
                    SESSION_READ_FAILED,
                    "A Codex session file could not be read and was skipped.",
                ));
                return Ok(None);
            }
        };
        head_bytes = head_bytes.saturating_add(raw_line.len() as u64);
        let line = match String::from_utf8(raw_line) {
            Ok(line) => line,
            Err(_) => {
                diagnostics.push(ScanDiagnostic::new(
                    "codex",
                    ScanDiagnosticSeverity::Warning,
                    SESSION_READ_FAILED,
                    "A Codex session file could not be read and was skipped.",
                ));
                return Ok(None);
            }
        };
        let line = if first_line {
            first_line = false;
            line.strip_prefix('\u{feff}').unwrap_or(&line)
        } else {
            &line
        };
        if line.trim().is_empty() {
            if !scan_full_file && head_bytes >= FULL_SCAN_MAX_BYTES {
                break;
            }
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            malformed_lines += 1;
            if !scan_full_file && head_bytes >= FULL_SCAN_MAX_BYTES {
                break;
            }
            continue;
        };

        if let Some(timestamp) = record.get("timestamp").and_then(try_read_utc_timestamp) {
            latest_activity = latest_activity.max(Some(timestamp));
        }

        // A large rollout is allowed to exceed the normal whole-file budget,
        // but a missing/corrupt metadata head must not make us consume the
        // entire transcript before we can report it. The first metadata record
        // normally appears at the head; this cap is the bounded fallback for
        // leading blank, malformed, or non-metadata records.
        let Some(payload) = record
            .get("type")
            .and_then(Value::as_str)
            .filter(|kind| *kind == "session_meta")
            .and_then(|_| record.get("payload"))
        else {
            if !scan_full_file && head_bytes >= FULL_SCAN_MAX_BYTES {
                break;
            }
            continue;
        };

        if let Some(id) = payload.get("id").and_then(Value::as_str) {
            session_id = Some(id.to_owned());
        }
        if let Some(cwd) = payload.get("cwd").and_then(Value::as_str) {
            project_path = Some(cwd.to_owned());
        }
        if let Some(timestamp) = payload.get("timestamp").and_then(try_read_utc_timestamp) {
            latest_activity = latest_activity.max(Some(timestamp));
        }
        session_kind = session_kind.merge(classify_session(payload));
        if !scan_full_file {
            break;
        }
    }

    if malformed_lines > 0 {
        diagnostics.push(ScanDiagnostic::new(
            "codex",
            ScanDiagnosticSeverity::Warning,
            MALFORMED_SESSION_RECORD,
            format!(
                "A Codex session contained {malformed_lines} malformed record(s); valid records were retained."
            ),
        ));
    }

    let Some(normalized_path) = project_path
        .as_deref()
        .and_then(|candidate| try_normalize_project_path(candidate, codex_home))
    else {
        diagnostics.push(ScanDiagnostic::new(
            "codex",
            ScanDiagnosticSeverity::Warning,
            MISSING_PROJECT_PATH,
            "A Codex session did not contain a safe absolute project path and was skipped.",
        ));
        return Ok(None);
    };

    let session_id = if session_kind == CodexSessionKind::Primary {
        let Some(session_id) = session_id.filter(|id| !id.trim().is_empty()) else {
            diagnostics.push(ScanDiagnostic::new(
                "codex",
                ScanDiagnosticSeverity::Warning,
                MISSING_SESSION_ID,
                "A Codex session did not contain a native session ID and was skipped.",
            ));
            return Ok(None);
        };
        Some(session_id)
    } else {
        None
    };

    let last_accessed_at = match (latest_activity, scan_full_file) {
        (Some(timestamp), false) => {
            file_last_write_utc(session_file).map_or(timestamp, |modified| modified.max(timestamp))
        }
        (Some(timestamp), true) => timestamp,
        (None, _) => {
            diagnostics.push(ScanDiagnostic::new(
                "codex",
                ScanDiagnosticSeverity::Warning,
                TIMESTAMP_FALLBACK,
                "A Codex session had no valid activity timestamp; file modification time was used.",
            ));
            let Some(fallback) = file_last_write_utc(session_file) else {
                return Ok(None);
            };
            fallback
        }
    };

    projects.push(ScannedProject {
        path: normalized_path,
        last_accessed_at,
        session_id,
        git_branch: None,
    });
    Ok(Some(session_kind))
}

fn classify_session(payload: &Value) -> CodexSessionKind {
    let thread_source = payload.get("thread_source").and_then(Value::as_str);
    let subagent = payload
        .get("source")
        .and_then(|source| source.get("subagent"));
    let is_guardian = matches!(thread_source, Some("guardian") | Some("guardian_review"))
        || subagent
            .and_then(|source| source.get("other"))
            .and_then(Value::as_str)
            == Some("guardian");
    if is_guardian {
        return CodexSessionKind::Guardian;
    }
    if matches!(thread_source, Some("child") | Some("subagent"))
        || subagent.is_some_and(|source| source.get("thread_spawn").is_some())
    {
        return CodexSessionKind::Child;
    }
    if subagent.is_some() {
        CodexSessionKind::Auxiliary
    } else {
        CodexSessionKind::Primary
    }
}

/// Reads the file modification time as a UTC timestamp. Returns `None` only when the metadata
/// is unavailable despite a successful read moments earlier.
fn file_last_write_utc(path: &Path) -> Option<DateTime<Utc>> {
    let metadata = no_follow_metadata(path).ok()?;
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
