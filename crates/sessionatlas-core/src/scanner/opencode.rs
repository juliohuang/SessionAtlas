//! OpenCode scanner: read-only SQLite open, alternate candidate paths, and a
//! schema or query mismatch that must never masquerade as an empty success.
//!
//! Only the session id, the
//! directory / project worktree and the two unix timestamps are read from the
//! `session` and `project` tables; session bodies are never read.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use super::base::{
    missing_source, probe_file, source_read_failure, ScanDiagnostic, ScanDiagnosticSeverity,
    ScanOutcome, ScannedProject, Scanner, SourceProbe, MISSING_PROJECT_PATH, SOURCE_READ_FAILED,
    TIMESTAMP_FALLBACK,
};
use super::parsing::{home_directory, try_normalize_project_path, try_read_unix_timestamp};
use crate::path::{self, PathFlavor};

/// OpenCode scanner for its SQLite project/session store.
pub struct OpenCodeScanner {
    is_available: Box<dyn Fn() -> bool>,
}

impl OpenCodeScanner {
    /// Availability defaults to whether `opencode` is on `PATH`; historical
    /// data stays discoverable regardless of the executable.
    pub fn new() -> Self {
        Self::with_availability(|| command_available("opencode"))
    }

    /// Availability override so tests can pin the outcome deterministically.
    pub fn with_availability(availability: impl Fn() -> bool + 'static) -> Self {
        Self {
            is_available: Box::new(availability),
        }
    }
}

impl Default for OpenCodeScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Scanner for OpenCodeScanner {
    fn tool_key(&self) -> &str {
        "opencode"
    }

    fn tool_name(&self) -> &str {
        "OpenCode"
    }

    fn is_available(&self) -> bool {
        (self.is_available)()
    }

    fn scan(&self) -> ScanOutcome {
        self.scan_source()
    }
}

impl OpenCodeScanner {
    fn scan_source(&self) -> ScanOutcome {
        let mut candidates = candidate_database_paths();
        candidates.sort();
        candidates.dedup_by(|a, b| {
            path::paths_equal(
                PathFlavor::native(),
                &a.to_string_lossy(),
                &b.to_string_lossy(),
            )
        });

        let mut database_paths = Vec::new();
        for candidate in &candidates {
            match probe_file(candidate) {
                SourceProbe::Failed => {
                    return source_read_failure("opencode", "an OpenCode database path");
                }
                SourceProbe::Exists => database_paths.push(candidate.clone()),
                SourceProbe::Missing => {}
            }
        }
        if database_paths.is_empty() {
            return missing_source("opencode", self.is_available());
        }

        let mut projects = Vec::new();
        let mut diagnostics = Vec::new();
        for database_path in &database_paths {
            if !try_read_database(database_path, &mut projects, &mut diagnostics) {
                diagnostics.push(ScanDiagnostic::new(
                    "opencode",
                    ScanDiagnosticSeverity::Error,
                    SOURCE_READ_FAILED,
                    "Could not safely inspect the OpenCode database; the previous index is preserved.",
                ));
                return ScanOutcome::failed(diagnostics);
            }
        }
        ScanOutcome::succeeded(projects, diagnostics)
    }
}

/// Candidate database paths in probe order: `~/.local/share/opencode`,
/// `~/.opencode`, and (only when `SESSIONATLAS_HOME` is blank) the XDG data
/// home.
fn candidate_database_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = home_directory() {
        paths.push(
            home.join(".local")
                .join("share")
                .join("opencode")
                .join("opencode.db"),
        );
        paths.push(home.join(".opencode").join("opencode.db"));
        if !env_var_non_blank("SESSIONATLAS_HOME") {
            if let Some(xdg_data_home) = std::env::var_os("XDG_DATA_HOME") {
                let trimmed = xdg_data_home.to_string_lossy().trim().to_string();
                if !trimmed.is_empty() {
                    paths.push(PathBuf::from(trimmed).join("opencode").join("opencode.db"));
                }
            }
        }
    }
    paths
}

/// Whether an environment variable holds a non-blank value.
fn env_var_non_blank(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.to_string_lossy().trim().is_empty())
}

/// One projected row from the OpenCode `session`/`project` join.
struct OpenCodeRow {
    session_id: String,
    project_path: String,
    session_time_updated: Option<i64>,
    project_time_updated: Option<i64>,
}

/// Reads the OpenCode `session`/`project` tables read-only. Any open, schema,
/// or row error is a failure so a malformed database can never masquerade as
/// an empty success.
fn try_read_database(
    database_path: &Path,
    projects: &mut Vec<ScannedProject>,
    diagnostics: &mut Vec<ScanDiagnostic>,
) -> bool {
    let connection = match rusqlite::Connection::open_with_flags(
        database_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(connection) => connection,
        Err(_) => return false,
    };

    const QUERY: &str = "\
        SELECT \
            session.id, \
            CASE \
                WHEN TRIM(session.directory) <> '' THEN session.directory \
                ELSE project.worktree \
            END, \
            session.time_updated, \
            project.time_updated \
        FROM session \
        JOIN project ON project.id = session.project_id \
        ORDER BY session.id \
    ";

    let mut statement = match connection.prepare(QUERY) {
        Ok(statement) => statement,
        Err(_) => return false,
    };
    let rows = match statement.query_map([], |row| {
        Ok(OpenCodeRow {
            session_id: row.get(0)?,
            project_path: row.get(1)?,
            session_time_updated: row.get(2)?,
            project_time_updated: row.get(3)?,
        })
    }) {
        Ok(rows) => rows,
        Err(_) => return false,
    };

    let source_root = database_path
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_default();

    for row in rows {
        let row = match row {
            Ok(row) => row,
            Err(_) => return false,
        };

        let Some(normalized_path) = try_normalize_project_path(&row.project_path, &source_root)
        else {
            diagnostics.push(ScanDiagnostic::new(
                "opencode",
                ScanDiagnosticSeverity::Warning,
                MISSING_PROJECT_PATH,
                "An OpenCode session did not contain a safe absolute directory and was skipped.",
            ));
            continue;
        };

        let last_accessed_at = match session_timestamp(&row) {
            Some(timestamp) => timestamp,
            None => {
                diagnostics.push(ScanDiagnostic::new(
                    "opencode",
                    ScanDiagnosticSeverity::Warning,
                    TIMESTAMP_FALLBACK,
                    "An OpenCode session had no valid activity timestamp; database modification time was used.",
                ));
                let Some(fallback) = file_last_write_utc(database_path) else {
                    continue;
                };
                fallback
            }
        };

        projects.push(ScannedProject {
            path: normalized_path,
            last_accessed_at,
            session_id: Some(row.session_id),
            git_branch: None,
        });
    }
    true
}

/// Session `time_updated`, then project `time_updated`. Returns `None` only
/// when neither stored timestamp is usable.
fn session_timestamp(row: &OpenCodeRow) -> Option<DateTime<Utc>> {
    row.session_time_updated
        .and_then(try_read_unix_timestamp)
        .or_else(|| row.project_time_updated.and_then(try_read_unix_timestamp))
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
    fn opencode_scanner_home_candidates_use_sessionatlas_home_override() {
        let dir = tempfile::tempdir().unwrap();
        with_env(
            &[("SESSIONATLAS_HOME", &dir.path().to_string_lossy())],
            &["XDG_DATA_HOME"],
            || {
                let candidates = candidate_database_paths();
                assert!(candidates
                    .iter()
                    .any(|path| *path == dir.path().join(".local/share/opencode/opencode.db")));
                assert!(candidates
                    .iter()
                    .any(|path| *path == dir.path().join(".opencode/opencode.db")));
                assert_eq!(candidates.len(), 2, "the XDG candidate is excluded");
            },
        );
    }

    #[test]
    fn opencode_scanner_xdg_candidate_appears_only_without_sessionatlas_home() {
        let dir = tempfile::tempdir().unwrap();
        with_env(
            &[("XDG_DATA_HOME", &dir.path().to_string_lossy())],
            &["SESSIONATLAS_HOME"],
            || {
                let candidates = candidate_database_paths();
                assert!(candidates
                    .iter()
                    .any(|path| *path == dir.path().join("opencode/opencode.db")));
            },
        );
    }
}
