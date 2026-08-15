//! Custom-tool scanner: `metadata.json` `project_path`/`cwd`/session `id`,
//! `~` expansion, bad-JSON degradation.
//!
//! Mirrors `Core/Scanner/CustomToolScanner.cs`. A configured data directory's
//! direct children are projects or carry a `metadata.json` pointing at the real
//! project. Availability probes the configured CLI executable and is separate
//! from historical-data discovery, which reads only path, timestamp and
//! session-ID metadata.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::model::ToolSource;
use crate::scanner::{
    expand_tilde, missing_source, probe_directory, source_read_failure, try_normalize_project_path,
    try_read_utc_timestamp, ScanDiagnostic, ScanDiagnosticSeverity, ScanOutcome, ScannedProject,
    Scanner, SourceProbe, MALFORMED_SESSION_RECORD, SESSION_READ_FAILED, TIMESTAMP_FALLBACK,
};

/// Whether `executable` resolves on PATH (`where` on Windows, `which`
/// elsewhere), matching the C# `ScannerRegistry.CommandExists`.
pub(crate) fn executable_on_path(executable: &str) -> bool {
    if executable.trim().is_empty() {
        return false;
    }
    let probe = if cfg!(windows) { "where" } else { "which" };
    std::process::Command::new(probe)
        .arg(executable)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// First whitespace-separated token of a CLI command, i.e. the executable.
fn first_command_token(command: &str) -> &str {
    command.split_whitespace().next().unwrap_or("")
}

/// Generic scanner for a user-configured directory whose direct children are
/// projects or contain `metadata.json` pointing at the real project.
pub struct CustomToolScanner {
    tool: ToolSource,
    is_command_available: Box<dyn Fn() -> bool>,
}

impl CustomToolScanner {
    /// Scanner probing the configured `CliCommand` executable for availability.
    pub fn new(tool: ToolSource) -> Self {
        let executable = first_command_token(&tool.cli_command).to_string();
        Self::with_availability(tool, move || executable_on_path(&executable))
    }

    /// Scanner with an injected availability predicate (used by tests and
    /// embedders that probe PATH differently).
    pub fn with_availability(tool: ToolSource, is_available: impl Fn() -> bool + 'static) -> Self {
        Self {
            tool,
            is_command_available: Box::new(is_available),
        }
    }
}

impl Scanner for CustomToolScanner {
    fn tool_key(&self) -> &str {
        &self.tool.key
    }

    fn tool_name(&self) -> &str {
        &self.tool.name
    }

    fn is_available(&self) -> bool {
        (self.is_command_available)()
    }

    fn scan(&self) -> ScanOutcome {
        let data_directory = match self.resolve_data_directory() {
            Some(directory) => directory,
            None => return missing_source(self.tool_key(), self.is_available()),
        };
        match probe_directory(&data_directory) {
            SourceProbe::Missing => {
                return missing_source(self.tool_key(), self.is_available());
            }
            SourceProbe::Failed => {
                return source_read_failure(
                    self.tool_key(),
                    "the configured custom-tool data directory",
                );
            }
            SourceProbe::Exists => {}
        }
        let directories = match read_direct_child_directories(&data_directory) {
            Ok(directories) => directories,
            Err(_) => {
                return source_read_failure(
                    self.tool_key(),
                    "the configured custom-tool data directory",
                );
            }
        };

        let mut projects: Vec<ScannedProject> = Vec::new();
        let mut diagnostics: Vec<ScanDiagnostic> = Vec::new();
        for directory in directories {
            self.parse_project(&data_directory, &directory, &mut projects, &mut diagnostics);
        }
        ScanOutcome::succeeded(projects, diagnostics)
    }
}

impl CustomToolScanner {
    /// Resolves the configured data directory with `~` expansion and
    /// `GetFullPath` semantics; `None` for a blank or unusable value.
    fn resolve_data_directory(&self) -> Option<PathBuf> {
        let value = self.tool.data_directory.trim();
        if value.is_empty() {
            return None;
        }
        let expanded = match expand_tilde(value) {
            Some(expanded) => expanded,
            None => value.to_string(),
        };
        std::path::absolute(expanded).ok()
    }

    /// Parses one direct child directory into a project. `metadata.json`, when
    /// present, supplies `project_path` (then `cwd`), `last_accessed`, and the
    /// session `id` (with `session_id` as a fallback key). Malformed or
    /// unreadable metadata degrades to the directory's own metadata with a
    /// warning diagnostic; the project is still recorded.
    fn parse_project(
        &self,
        data_directory: &Path,
        directory: &Path,
        projects: &mut Vec<ScannedProject>,
        diagnostics: &mut Vec<ScanDiagnostic>,
    ) {
        let mut project_path = normalize_path_or_raw(directory);
        let mut last_accessed = directory_last_modified(directory).unwrap_or_else(|| {
            diagnostics.push(ScanDiagnostic::new(
                self.tool_key(),
                ScanDiagnosticSeverity::Warning,
                TIMESTAMP_FALLBACK,
                "A custom-tool project directory modification time could not be read; a minimum timestamp was used.",
            ));
            DateTime::<Utc>::MIN_UTC
        });
        let mut session_id: Option<String> = None;

        let metadata_path = directory.join("metadata.json");
        if metadata_path.is_file() {
            match std::fs::read(&metadata_path) {
                Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(strip_utf8_bom(&bytes)) {
                    Ok(root) => {
                        if let Some(configured) = first_usable_path(&root) {
                            if let Some(normalized) = try_normalize_project_path(
                                &configured,
                                &data_directory.to_string_lossy(),
                            ) {
                                project_path = normalized;
                            }
                        }
                        if let Some(timestamp) = root
                            .get("last_accessed")
                            .and_then(try_read_utc_timestamp)
                        {
                            last_accessed = timestamp;
                        }
                        session_id = root
                            .get("id")
                            .and_then(serde_json::Value::as_str)
                            .or_else(|| {
                                root.get("session_id").and_then(serde_json::Value::as_str)
                            })
                            .map(str::to_string);
                    }
                    Err(_) => diagnostics.push(ScanDiagnostic::new(
                        self.tool_key(),
                        ScanDiagnosticSeverity::Warning,
                        MALFORMED_SESSION_RECORD,
                        "A custom-tool metadata file contained malformed JSON; directory metadata was used.",
                    )),
                },
                Err(_) => diagnostics.push(ScanDiagnostic::new(
                    self.tool_key(),
                    ScanDiagnosticSeverity::Warning,
                    SESSION_READ_FAILED,
                    "A custom-tool metadata file could not be read; directory metadata was used.",
                )),
            }
        }

        projects.push(ScannedProject {
            path: project_path,
            last_accessed_at: last_accessed,
            session_id,
            git_branch: None,
        });
    }
}

/// Direct child directories of `root`, sorted by path (C# `OrderBy(Ordinal)`).
fn read_direct_child_directories(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut directories = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.metadata()?.is_dir() {
            directories.push(entry.path());
        }
    }
    directories.sort();
    Ok(directories)
}

/// The `project_path` value when present and usable, falling back to `cwd`,
/// in the C# field order.
fn first_usable_path(root: &serde_json::Value) -> Option<String> {
    ["project_path", "cwd"]
        .into_iter()
        .find_map(|key| root.get(key).and_then(serde_json::Value::as_str))
        .map(str::to_string)
}

/// Normalizes a directory to a native absolute path, falling back to the raw
/// string for the degenerate case where the entry cannot be normalized.
fn normalize_path_or_raw(path: &Path) -> String {
    crate::path::normalize_native(&path.to_string_lossy())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Directory modification time as UTC, or `None` when it cannot be read.
fn directory_last_modified(directory: &Path) -> Option<DateTime<Utc>> {
    std::fs::metadata(directory)
        .and_then(|metadata| metadata.modified())
        .ok()
        .map(DateTime::<Utc>::from)
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes)
}
