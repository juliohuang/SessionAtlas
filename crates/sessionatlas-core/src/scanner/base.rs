//! Scanner framework primitives: `Scanner` trait, `ScanOutcome`, `ScanStatus`,
//! `ScanDiagnostic`, `ScannedProject`, source probing and base outcome rules.
//!
//! Mirrors `Core/Scanner/IProjectScanner.cs` and
//! `Core/Scanner/ProjectScannerBase.cs`. Availability (`is_available`) is
//! intentionally separate from data discoverability: historical data stays
//! scannable even when the CLI executable is gone, and an installed executable
//! alone never erases the distinction between a successful inspection and an
//! unreadable source.

use std::path::Path;

use chrono::{DateTime, Utc};

/// The CLI executable and its local session source were both absent.
pub const SOURCE_UNAVAILABLE: &str = "source_unavailable";
/// A source existed but could not be inspected safely.
pub const SOURCE_READ_FAILED: &str = "source_read_failed";
/// Session files existed but reading one or more of them failed.
pub const SESSION_READ_FAILED: &str = "session_read_failed";
/// A session record could not be parsed into the expected shape.
pub const MALFORMED_SESSION_RECORD: &str = "malformed_session_record";
/// A session record carried no usable project path.
pub const MISSING_PROJECT_PATH: &str = "missing_project_path";
/// A session record carried no usable session ID.
pub const MISSING_SESSION_ID: &str = "missing_session_id";
/// A timestamp was malformed or missing and a fallback was used.
pub const TIMESTAMP_FALLBACK: &str = "timestamp_fallback";
/// Session files were present but none produced a safe project record.
pub const NO_VALID_SESSIONS: &str = "no_valid_sessions";
/// The custom-tool configuration could not be read.
pub const CONFIG_READ_FAILED: &str = "config_read_failed";
/// An unexpected scanner failure escaped the tool-specific logic.
pub const UNEXPECTED_SCANNER_FAILURE: &str = "unexpected_scanner_failure";

/// Outcome state of one tool scan.
///
/// `Succeeded` is the only state allowed to replace a tool snapshot; it may
/// contain zero projects when the source was inspected successfully.
/// `Unavailable` and `Failed` never carry projects and must preserve the prior
/// snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStatus {
    Succeeded,
    Unavailable,
    Failed,
}

/// Severity of a structured [`ScanDiagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

/// A structured, stable diagnostic. The message must never contain prompt
/// text, message bodies, credentials, or other session content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanDiagnostic {
    pub tool_key: String,
    pub severity: ScanDiagnosticSeverity,
    pub code: &'static str,
    pub message: String,
}

impl ScanDiagnostic {
    pub fn new(
        tool_key: impl Into<String>,
        severity: ScanDiagnosticSeverity,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            tool_key: tool_key.into(),
            severity,
            code,
            message: message.into(),
        }
    }
}

/// Raw result of scanning a single tool source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedProject {
    /// Normalized absolute project path.
    pub path: String,
    /// Greatest valid activity timestamp among the discovered sessions.
    pub last_accessed_at: DateTime<Utc>,
    /// Tool-native session ID, when the source exposes one.
    pub session_id: Option<String>,
    /// Git branch of the working tree at scan time.
    pub git_branch: Option<String>,
}

/// The result of one [`Scanner::scan`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanOutcome {
    status: ScanStatus,
    projects: Vec<ScannedProject>,
    diagnostics: Vec<ScanDiagnostic>,
}

impl ScanOutcome {
    /// A trustworthy snapshot. The project list may be empty when the source
    /// was inspected successfully.
    pub fn succeeded(
        projects: impl IntoIterator<Item = ScannedProject>,
        diagnostics: impl IntoIterator<Item = ScanDiagnostic>,
    ) -> Self {
        Self {
            status: ScanStatus::Succeeded,
            projects: projects.into_iter().collect(),
            diagnostics: diagnostics.into_iter().collect(),
        }
    }

    /// Neither a local source nor a launchable executable was found. The prior
    /// snapshot is preserved.
    pub fn unavailable(diagnostics: impl IntoIterator<Item = ScanDiagnostic>) -> Self {
        Self {
            status: ScanStatus::Unavailable,
            projects: Vec::new(),
            diagnostics: diagnostics.into_iter().collect(),
        }
    }

    /// A source exists but could not be inspected safely. The prior snapshot
    /// is preserved.
    pub fn failed(diagnostics: impl IntoIterator<Item = ScanDiagnostic>) -> Self {
        Self {
            status: ScanStatus::Failed,
            projects: Vec::new(),
            diagnostics: diagnostics.into_iter().collect(),
        }
    }

    pub fn status(&self) -> ScanStatus {
        self.status
    }

    /// Whether the source was inspected successfully (possibly with zero
    /// projects). Only this state may replace a stored tool snapshot.
    pub fn is_successful(&self) -> bool {
        self.status == ScanStatus::Succeeded
    }

    pub fn projects(&self) -> &[ScannedProject] {
        &self.projects
    }

    pub fn diagnostics(&self) -> &[ScanDiagnostic] {
        &self.diagnostics
    }

    pub fn into_projects(self) -> Vec<ScannedProject> {
        self.projects
    }

    pub fn into_diagnostics(self) -> Vec<ScanDiagnostic> {
        self.diagnostics
    }
}

/// One AI CLI tool scanner. Implementations are owned by the R05 tasks.
pub trait Scanner {
    /// Lowercase tool key, e.g. `codex`, `claude`.
    fn tool_key(&self) -> &str;
    /// Human-readable tool name.
    fn tool_name(&self) -> &str;
    /// Whether the CLI executable can currently be launched. This is separate
    /// from historical-source discoverability.
    fn is_available(&self) -> bool;
    /// Inspects the tool's local source and returns a [`ScanOutcome`].
    fn scan(&self) -> ScanOutcome;
}

/// Result of probing whether a source path exists with the expected shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceProbe {
    Exists,
    Missing,
    Failed,
}

/// Probes a directory source path. A path that exists but is not a directory,
/// and any non-NotFound access error, both classify as [`SourceProbe::Failed`].
pub fn probe_directory(path: &Path) -> SourceProbe {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => SourceProbe::Exists,
        Ok(_) => SourceProbe::Failed,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => SourceProbe::Missing,
        Err(_) => SourceProbe::Failed,
    }
}

/// Probes a file source path. A path that exists but is a directory, and any
/// non-NotFound access error, both classify as [`SourceProbe::Failed`].
pub fn probe_file(path: &Path) -> SourceProbe {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => SourceProbe::Failed,
        Ok(_) => SourceProbe::Exists,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => SourceProbe::Missing,
        Err(_) => SourceProbe::Failed,
    }
}

/// Missing-source handling. An installed executable makes this a trustworthy
/// empty success; otherwise the tool is unavailable and the prior snapshot is
/// preserved.
pub fn missing_source(tool_key: &str, is_available: bool) -> ScanOutcome {
    if is_available {
        return ScanOutcome::succeeded([], []);
    }
    ScanOutcome::unavailable([ScanDiagnostic::new(
        tool_key,
        ScanDiagnosticSeverity::Info,
        SOURCE_UNAVAILABLE,
        "The CLI executable and its local session source were not found; the previous index is preserved.",
    )])
}

/// Source-exists-but-cannot-be-inspected handling.
pub fn source_read_failure(tool_key: &str, source_description: &str) -> ScanOutcome {
    ScanOutcome::failed([ScanDiagnostic::new(
        tool_key,
        ScanDiagnosticSeverity::Error,
        SOURCE_READ_FAILED,
        format!("Could not safely inspect {source_description}; the previous index is preserved."),
    )])
}

/// Finalizes a scan over enumerated session files.
///
/// When files were present but none produced a safe project record this is a
/// failure, not an empty success: only successful inspection may replace a
/// tool snapshot.
pub fn complete_session_files(
    tool_key: &str,
    source_file_count: usize,
    projects: Vec<ScannedProject>,
    diagnostics: Vec<ScanDiagnostic>,
) -> ScanOutcome {
    if source_file_count > 0 && projects.is_empty() {
        let mut all = diagnostics;
        all.push(ScanDiagnostic::new(
            tool_key,
            ScanDiagnosticSeverity::Error,
            NO_VALID_SESSIONS,
            "Session files were present but none produced a safe project record; the previous index is preserved.",
        ));
        return ScanOutcome::failed(all);
    }
    ScanOutcome::succeeded(projects, diagnostics)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn project(path: &str) -> ScannedProject {
        ScannedProject {
            path: path.to_string(),
            last_accessed_at: Utc::now(),
            session_id: None,
            git_branch: None,
        }
    }

    #[test]
    fn scanner_base_available_missing_source_is_empty_success() {
        let outcome = missing_source("codex", true);
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(outcome.is_successful());
        assert!(outcome.projects().is_empty());
        assert!(outcome.diagnostics().is_empty());
    }

    #[test]
    fn scanner_base_unavailable_missing_source_preserves_old_data() {
        let outcome = missing_source("codex", false);
        assert_eq!(outcome.status(), ScanStatus::Unavailable);
        assert!(!outcome.is_successful());
        assert!(outcome.projects().is_empty());
        let diagnostics = outcome.diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].tool_key, "codex");
        assert_eq!(diagnostics[0].severity, ScanDiagnosticSeverity::Info);
        assert_eq!(diagnostics[0].code, SOURCE_UNAVAILABLE);
    }

    #[test]
    fn scanner_base_source_read_failure_is_error_and_never_empty_success() {
        let outcome = source_read_failure("kimi", "~/.kimi-code/sessions");
        assert_eq!(outcome.status(), ScanStatus::Failed);
        assert!(outcome.projects().is_empty());
        let diagnostic = &outcome.diagnostics()[0];
        assert_eq!(diagnostic.code, SOURCE_READ_FAILED);
        assert_eq!(diagnostic.severity, ScanDiagnosticSeverity::Error);
        assert!(diagnostic.message.contains("previous index is preserved"));
    }

    #[test]
    fn scanner_base_present_files_with_no_valid_projects_is_failed() {
        let outcome = complete_session_files("codex", 3, vec![], vec![]);
        assert_eq!(outcome.status(), ScanStatus::Failed);
        assert!(outcome.projects().is_empty());
        let diagnostic = &outcome.diagnostics()[0];
        assert_eq!(diagnostic.code, NO_VALID_SESSIONS);
        assert_eq!(diagnostic.severity, ScanDiagnosticSeverity::Error);
    }

    #[test]
    fn scanner_base_no_source_files_is_trustworthy_empty_success() {
        let outcome = complete_session_files("codex", 0, vec![], vec![]);
        assert_eq!(outcome.status(), ScanStatus::Succeeded);
        assert!(outcome.is_successful());
        assert!(outcome.projects().is_empty());
    }

    #[test]
    fn scanner_base_unavailable_and_failed_never_carry_projects() {
        let unavailable = ScanOutcome::unavailable([ScanDiagnostic::new(
            "codex",
            ScanDiagnosticSeverity::Info,
            SOURCE_UNAVAILABLE,
            "gone",
        )]);
        let failed = ScanOutcome::failed([ScanDiagnostic::new(
            "codex",
            ScanDiagnosticSeverity::Error,
            SOURCE_READ_FAILED,
            "unreadable",
        )]);
        assert!(unavailable.projects().is_empty());
        assert!(failed.projects().is_empty());
        assert!(!unavailable.is_successful());
        assert!(!failed.is_successful());

        let empty = ScanOutcome::succeeded([], []);
        let with_projects = ScanOutcome::succeeded([project("native:/repo")], []);
        assert!(empty.is_successful());
        assert_eq!(empty.projects().len(), 0);
        assert!(with_projects.is_successful());
        assert_eq!(with_projects.projects().len(), 1);
        assert_eq!(with_projects.projects()[0].path, "native:/repo");
    }

    #[test]
    fn scanner_base_probe_directory_distinguishes_exists_missing_failed() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("source");
        std::fs::create_dir(&existing).unwrap();
        let file = dir.path().join("file.txt");
        std::fs::write(&file, b"x").unwrap();

        assert_eq!(probe_directory(&existing), SourceProbe::Exists);
        assert_eq!(
            probe_directory(&dir.path().join("nope")),
            SourceProbe::Missing
        );
        assert_eq!(probe_directory(&file), SourceProbe::Failed);
    }

    #[test]
    fn scanner_base_probe_file_distinguishes_exists_missing_failed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file.txt");
        std::fs::write(&file, b"x").unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();

        assert_eq!(probe_file(&file), SourceProbe::Exists);
        assert_eq!(
            probe_file(&dir.path().join("nope.txt")),
            SourceProbe::Missing
        );
        assert_eq!(probe_file(&sub), SourceProbe::Failed);
    }

    struct FakeScanner;

    impl Scanner for FakeScanner {
        fn tool_key(&self) -> &str {
            "fake"
        }

        fn tool_name(&self) -> &str {
            "Fake"
        }

        fn is_available(&self) -> bool {
            false
        }

        fn scan(&self) -> ScanOutcome {
            missing_source("fake", false)
        }
    }

    #[test]
    fn scanner_base_trait_separates_availability_from_discoverability() {
        let scanner = FakeScanner;
        assert_eq!(scanner.tool_key(), "fake");
        assert!(!scanner.is_available());
        let outcome = scanner.scan();
        assert_eq!(outcome.status(), ScanStatus::Unavailable);
        assert_eq!(outcome.diagnostics()[0].code, SOURCE_UNAVAILABLE);
    }
}
