//! Kimi scanner: recursive `state.json` discovery, `workDir` extraction,
//! timestamp candidates and file-modification-time fallback.
//!
//! Mirrors `Core/Scanner/KimiScanner.cs`. Sessions live at
//! `~/.kimi-code/sessions/<worktree-key>/<session-id>/state.json`. Only the
//! project path, the session ID (the state file's parent directory name) and
//! an activity timestamp are extracted; no conversation content is read.

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

/// Kimi Code state file name under each session directory.
const STATE_FILE_NAME: &str = "state.json";

/// Kimi CLI scanner for `~/.kimi-code/sessions/<worktree-key>/<session-id>/state.json`.
pub struct KimiScanner {
    is_available: Box<dyn Fn() -> bool>,
}

impl KimiScanner {
    /// Availability defaults to whether `kimi` is on `PATH`; historical data
    /// stays discoverable regardless of the executable.
    pub fn new() -> Self {
        Self::with_availability(|| command_available("kimi"))
    }

    /// Availability override, mirroring the C# `KimiScanner(Func<bool>?)`
    /// constructor so tests can pin the outcome deterministically.
    pub fn with_availability(availability: impl Fn() -> bool + 'static) -> Self {
        Self {
            is_available: Box::new(availability),
        }
    }
}

impl Default for KimiScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Scanner for KimiScanner {
    fn tool_key(&self) -> &str {
        "kimi"
    }

    fn tool_name(&self) -> &str {
        "Kimi CLI"
    }

    fn is_available(&self) -> bool {
        (self.is_available)()
    }

    fn scan(&self) -> ScanOutcome {
        self.scan_source()
    }
}

impl KimiScanner {
    fn scan_source(&self) -> ScanOutcome {
        let Some(kimi_home) = resolve_kimi_home() else {
            return missing_source("kimi", self.is_available());
        };
        let sessions_dir = kimi_home.join("sessions");
        match probe_directory(&sessions_dir) {
            SourceProbe::Missing => return missing_source("kimi", self.is_available()),
            SourceProbe::Failed => {
                return source_read_failure("kimi", "the Kimi Code sessions directory");
            }
            SourceProbe::Exists => {}
        }

        let state_files = match enumerate_state_files(&sessions_dir) {
            Ok(files) => files,
            Err(()) => {
                return source_read_failure("kimi", "the Kimi Code sessions directory");
            }
        };

        let mut projects = Vec::new();
        let mut diagnostics = Vec::new();
        let kimi_home = kimi_home.to_string_lossy().into_owned();
        for state_file in &state_files {
            parse_state_file(state_file, &kimi_home, &mut projects, &mut diagnostics);
        }

        complete_session_files("kimi", state_files.len(), projects, diagnostics)
    }
}

/// Resolves the Kimi Code home, mirroring `KimiScanner.ResolveKimiHome`: a
/// non-blank `SESSIONATLAS_HOME` pins `~/.kimi-code` under it; otherwise a
/// non-blank `KIMI_CODE_HOME` wins; otherwise `~/.kimi-code`.
fn resolve_kimi_home() -> Option<PathBuf> {
    let application_home = home_directory()?;
    if env_var_non_blank("SESSIONATLAS_HOME") {
        return Some(application_home.join(".kimi-code"));
    }
    if let Some(configured) = std::env::var_os("KIMI_CODE_HOME") {
        let trimmed = configured.to_string_lossy().trim().to_string();
        if !trimmed.is_empty() {
            return Some(std::path::absolute(&trimmed).unwrap_or_else(|_| PathBuf::from(&trimmed)));
        }
    }
    Some(application_home.join(".kimi-code"))
}

/// Whether an environment variable holds a non-blank value, mirroring the C#
/// `string.IsNullOrWhiteSpace` guard.
fn env_var_non_blank(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.to_string_lossy().trim().is_empty())
}

/// Recursively enumerates `state.json` files in ordinal path order. Any
/// inaccessible entry surfaces as `Err(())`, matching the C# recursive
/// enumeration which throws on the first unreadable path.
fn enumerate_state_files(sessions_dir: &Path) -> Result<Vec<PathBuf>, ()> {
    let mut files = Vec::new();
    for entry in recursive_file_enumeration(sessions_dir) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => return Err(()),
        };
        if entry.file_type().is_file() && is_state_file(entry.path()) {
            files.push(entry.into_path());
        }
    }
    files.sort();
    Ok(files)
}

/// The C# `Directory.GetFiles(..., "state.json", ...)` search pattern matches
/// case-insensitively on Windows and byte-exactly elsewhere.
fn is_state_file(path: &Path) -> bool {
    let Some(file_name) = path.file_name() else {
        return false;
    };
    let file_name = file_name.to_string_lossy();
    if cfg!(windows) {
        file_name.eq_ignore_ascii_case(STATE_FILE_NAME)
    } else {
        file_name == STATE_FILE_NAME
    }
}

/// Parses one Kimi state file. The `workDir` is normalized against the Kimi
/// home; the timestamp is the first of `updatedAt` / `lastUpdatedAt` /
/// `timestamp`, falling back to the file modification time; the session ID is
/// the state file's parent directory name. Malformed JSON or unreadable files
/// degrade to a diagnostic and are skipped.
fn parse_state_file(
    state_file: &Path,
    kimi_home: &str,
    projects: &mut Vec<ScannedProject>,
    diagnostics: &mut Vec<ScanDiagnostic>,
) {
    let content = match std::fs::read_to_string(state_file) {
        Ok(content) => content,
        Err(_) => {
            diagnostics.push(ScanDiagnostic::new(
                "kimi",
                ScanDiagnosticSeverity::Warning,
                SESSION_READ_FAILED,
                "A Kimi Code state file could not be read and was skipped.",
            ));
            return;
        }
    };

    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let root: Value = match serde_json::from_str(content) {
        Ok(root) => root,
        Err(_) => {
            diagnostics.push(ScanDiagnostic::new(
                "kimi",
                ScanDiagnosticSeverity::Warning,
                MALFORMED_SESSION_RECORD,
                "A Kimi Code state file contained malformed JSON and was skipped.",
            ));
            return;
        }
    };

    let Some(normalized_path) = root
        .get("workDir")
        .and_then(Value::as_str)
        .and_then(|candidate| try_normalize_project_path(candidate, kimi_home))
    else {
        diagnostics.push(ScanDiagnostic::new(
            "kimi",
            ScanDiagnosticSeverity::Warning,
            MISSING_PROJECT_PATH,
            "A Kimi Code session did not contain a safe absolute workDir and was skipped.",
        ));
        return;
    };

    let last_accessed_at = match read_timestamp(&root) {
        Some(timestamp) => timestamp,
        None => {
            diagnostics.push(ScanDiagnostic::new(
                "kimi",
                ScanDiagnosticSeverity::Warning,
                TIMESTAMP_FALLBACK,
                "A Kimi Code session had no valid activity timestamp; state-file modification time was used.",
            ));
            let Some(fallback) = file_last_write_utc(state_file) else {
                return;
            };
            fallback
        }
    };

    projects.push(ScannedProject {
        path: normalized_path,
        last_accessed_at,
        session_id: state_file
            .parent()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned()),
        git_branch: None,
    });
}

/// First usable timestamp among the C# candidate property names, in order.
fn read_timestamp(root: &Value) -> Option<DateTime<Utc>> {
    ["updatedAt", "lastUpdatedAt", "timestamp"]
        .into_iter()
        .find_map(|property| root.get(property).and_then(try_read_utc_timestamp))
}

/// Reads the file modification time as a UTC timestamp, mirroring the C#
/// `File.GetLastWriteTimeUtc` fallback. Returns `None` only when the metadata
/// is unavailable despite a successful read moments earlier.
fn file_last_write_utc(path: &Path) -> Option<DateTime<Utc>> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    Some(modified.into())
}

/// Whether a command executable is reachable on `PATH`. Mirrors the C#
/// `ScannerRegistry.CommandExists` without launching anything.
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

    /// Runs `body` with the given environment overrides, restoring every
    /// affected variable even on panic. Uses the shared parsing-module lock so
    /// parallel tests in different scanner modules cannot corrupt each other's
    /// environment overrides.
    fn with_env<R>(set: &[(&str, &str)], clear: &[&str], body: impl FnOnce() -> R) -> R {
        let _guard = crate::scanner::parsing::ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut previous: Vec<(String, Option<std::ffi::OsString>)> = Vec::new();
        for (name, value) in set {
            previous.push((name.to_string(), std::env::var_os(name)));
            std::env::set_var(name, value);
        }
        for name in clear {
            previous.push((name.to_string(), std::env::var_os(name)));
            std::env::remove_var(name);
        }
        struct Restore(Vec<(String, Option<std::ffi::OsString>)>);
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

    #[test]
    fn kimi_scanner_home_resolution_follows_sessionatlas_then_kimi_code_home() {
        let dir = tempfile::tempdir().unwrap();
        let kimi_home = tempfile::tempdir().unwrap();

        with_env(
            &[("SESSIONATLAS_HOME", &dir.path().to_string_lossy())],
            &["KIMI_CODE_HOME"],
            || {
                let resolved = resolve_kimi_home().unwrap();
                assert_eq!(resolved, dir.path().join(".kimi-code"));
            },
        );

        with_env(
            &[("KIMI_CODE_HOME", &kimi_home.path().to_string_lossy())],
            &["SESSIONATLAS_HOME"],
            || {
                let resolved = resolve_kimi_home().unwrap();
                assert_eq!(resolved, kimi_home.path());
            },
        );

        with_env(&[], &["SESSIONATLAS_HOME", "KIMI_CODE_HOME"], || {
            let resolved = resolve_kimi_home().unwrap();
            assert_eq!(resolved, home_directory().unwrap().join(".kimi-code"));
        });
    }
}
