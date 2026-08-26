//! Aider scanner: checks `.aider.chat.history` metadata only, never reads
//! conversation content.
//!
//! Availability (the `aider` executable
//! on PATH) is separate from historical-data discoverability (`.aider.chat.history`
//! markers under `~/work`, `~/projects`, `~/dev`, `~/src`). Aider has no central
//! session database, so a project is identified by the presence and modification
//! time of its history marker; the file body is never read and no session ID
//! exists.

use std::collections::HashSet;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use crate::scanner::custom::executable_on_path;
use crate::scanner::{
    bounded_recursive_files, complete_session_files, home_directory, missing_source,
    no_follow_metadata, probe_directory, source_read_failure, ScanBudget, ScanDiagnostic,
    ScanDiagnosticSeverity, ScanOutcome, ScannedProject, Scanner, SourceProbe, SESSION_READ_FAILED,
};

/// Aider history-marker file name; a project is its parent directory.
const HISTORY_MARKER: &str = ".aider.chat.history";

/// Default search-root names under the SessionAtlas home, in probe order.
const SEARCH_ROOTS: [&str; 4] = ["work", "projects", "dev", "src"];

/// Aider scanner.
pub struct AiderScanner {
    is_command_available: Box<dyn Fn() -> bool>,
    budget: ScanBudget,
}

impl AiderScanner {
    /// Scanner probing the real `aider` executable for availability.
    pub fn new() -> Self {
        Self::with_availability(|| executable_on_path("aider"))
    }

    /// Scanner with an injected availability predicate (used by tests and
    /// embedders that probe PATH differently). Historical-data discovery never
    /// depends on this predicate.
    pub fn with_availability(is_available: impl Fn() -> bool + 'static) -> Self {
        Self {
            is_command_available: Box::new(is_available),
            budget: ScanBudget::default(),
        }
    }
    pub fn with_budget(mut self, budget: ScanBudget) -> Self {
        self.budget = budget;
        self
    }
}

impl Default for AiderScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Scanner for AiderScanner {
    fn tool_key(&self) -> &str {
        "aider"
    }

    fn tool_name(&self) -> &str {
        "Aider"
    }

    fn is_available(&self) -> bool {
        (self.is_command_available)()
    }

    fn scan(&self) -> ScanOutcome {
        let home = match home_directory() {
            Some(home) => home,
            None => return missing_source(self.tool_key(), self.is_available()),
        };

        let mut roots: Vec<PathBuf> = Vec::new();
        for name in SEARCH_ROOTS {
            let candidate = home.join(name);
            match probe_directory(&candidate) {
                SourceProbe::Failed => {
                    return source_read_failure(self.tool_key(), "an Aider search root");
                }
                SourceProbe::Exists => roots.push(candidate),
                SourceProbe::Missing => {}
            }
        }

        if roots.is_empty() {
            return missing_source(self.tool_key(), self.is_available());
        }
        roots.sort();

        let mut projects: Vec<ScannedProject> = Vec::new();
        let mut diagnostics: Vec<ScanDiagnostic> = Vec::new();
        let context = self.budget.context();
        let mut seen_paths: HashSet<String> = HashSet::new();
        let mut marker_count: usize = 0;

        for root in &roots {
            let entries = match bounded_recursive_files(root, &context) {
                Ok(entries) => entries,
                Err(error) => {
                    return ScanOutcome::failed([context.diagnostic(self.tool_key(), error)])
                }
            };
            for path in entries {
                if no_follow_metadata(&path).map_or(true, |meta| !meta.is_file())
                    || path.file_name().is_none_or(|name| name != HISTORY_MARKER)
                {
                    continue;
                }
                marker_count += 1;
                if let Err(error) = context.source_file(0) {
                    return ScanOutcome::failed([context.diagnostic(self.tool_key(), error)]);
                }
                if let Err(error) = context.record() {
                    return ScanOutcome::failed([context.diagnostic(self.tool_key(), error)]);
                }
                let Some(parent) = path.parent() else {
                    continue;
                };
                let Some(normalized) = crate::path::normalize_native(&parent.to_string_lossy())
                else {
                    continue;
                };
                if !seen_paths.insert(case_fold(&normalized)) {
                    continue;
                }
                let last_accessed = match no_follow_metadata(&path).and_then(|meta| meta.modified())
                {
                    Ok(modified) => DateTime::<Utc>::from(modified),
                    Err(_) => {
                        diagnostics.push(ScanDiagnostic::new(
                            self.tool_key(),
                            ScanDiagnosticSeverity::Warning,
                            SESSION_READ_FAILED,
                            "An Aider history marker could not be inspected and was skipped.",
                        ));
                        continue;
                    }
                };
                projects.push(ScannedProject {
                    path: normalized,
                    last_accessed_at: last_accessed,
                    session_id: None,
                    git_branch: None,
                });
            }
        }

        complete_session_files(self.tool_key(), marker_count, projects, diagnostics)
    }
}

/// Case rule for in-scan path deduplication: Windows paths fold
/// case-insensitively and all other paths remain byte-exact.
fn case_fold(path: &str) -> String {
    if cfg!(windows) {
        path.to_lowercase()
    } else {
        path.to_string()
    }
}
