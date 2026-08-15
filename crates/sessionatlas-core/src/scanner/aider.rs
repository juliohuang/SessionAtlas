//! Aider scanner: checks `.aider.chat.history` metadata only, never reads
//! conversation content.
//!
//! Mirrors `Core/Scanner/AiderScanner.cs`. Availability (the `aider` executable
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
    complete_session_files, home_directory, missing_source, probe_directory,
    recursive_file_enumeration, source_read_failure, ScanDiagnostic, ScanDiagnosticSeverity,
    ScanOutcome, ScannedProject, Scanner, SourceProbe, SESSION_READ_FAILED,
};

/// Aider history-marker file name; a project is its parent directory.
const HISTORY_MARKER: &str = ".aider.chat.history";

/// Default search-root names under the sessionatlas home, in C# order.
const SEARCH_ROOTS: [&str; 4] = ["work", "projects", "dev", "src"];

/// Aider scanner.
pub struct AiderScanner {
    is_command_available: Box<dyn Fn() -> bool>,
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
        }
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
        let mut seen_paths: HashSet<String> = HashSet::new();
        let mut marker_count: usize = 0;

        for root in &roots {
            for entry in recursive_file_enumeration(root) {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => {
                        return source_read_failure(
                            self.tool_key(),
                            "the configured Aider search roots",
                        );
                    }
                };
                if !entry.file_type().is_file() || entry.file_name() != HISTORY_MARKER {
                    continue;
                }
                marker_count += 1;
                let Some(parent) = entry.path().parent() else {
                    continue;
                };
                let Some(normalized) = crate::path::normalize_native(&parent.to_string_lossy())
                else {
                    continue;
                };
                if !seen_paths.insert(case_fold(&normalized)) {
                    continue;
                }
                let last_accessed =
                    match std::fs::metadata(entry.path()).and_then(|meta| meta.modified()) {
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

/// Case rule for the in-scan path deduplication: Windows paths fold
/// case-insensitively, everything else byte-exactly, matching the C#.
fn case_fold(path: &str) -> String {
    if cfg!(windows) {
        path.to_lowercase()
    } else {
        path.to_string()
    }
}
