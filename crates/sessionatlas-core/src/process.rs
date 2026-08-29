//! Process runner abstraction: an injectable runner boundary, the
//! `program + argv + working-directory` spec, and PATH executable resolution.
//!
//! Production code (`SystemProcessRunner`, `PathProgramResolver`) is the ONLY
//! place in the crate allowed to call `std::process`; everything else talks to
//! the [`ProcessRunner`] and [`ProgramResolver`] traits so tests can substitute
//! recording fakes and never start a real terminal, AI CLI, `where`, or
//! `which`. The production runner never runs a shell: it spawns `program`
//! directly with the argument array in the given working directory.

use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A local process as `program + argument array + working directory`. Project
/// paths live in `working_directory` (or a dedicated argument), never inside a
/// shell command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    /// Executable to launch (bare name resolved through PATH, or a path).
    pub program: String,
    /// Argument array passed to the executable.
    pub arguments: Vec<String>,
    /// Working directory the process starts in.
    pub working_directory: String,
}

/// Captured output of a blocking [`ProcessRunner::run`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Why a process could not be started or completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessError {
    /// The program that failed to start.
    pub program: String,
    /// Human-readable detail (sanitized by callers before terminal output).
    pub detail: String,
}

impl fmt::Display for ProcessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "无法启动进程 '{}': {}", self.program, self.detail)
    }
}

impl std::error::Error for ProcessError {}

/// Boundary around operating-system process execution. Mirrors
/// The production implementation uses
/// [`SystemProcessRunner`]; tests supply a recording fake so they never start
/// terminals, tools, `where`, or `which`.
pub trait ProcessRunner {
    /// Detached start (fire-and-forget), used for interactive terminal
    /// launches. The child keeps running after this returns.
    fn start(&self, spec: &ProcessSpec) -> Result<(), ProcessError>;
    /// Blocking run that waits for completion and captures output.
    fn run(&self, spec: &ProcessSpec) -> Result<ProcessOutput, ProcessError>;
}

/// Production runner. This is the only type in the crate that calls
/// `std::process`. `start` never waits; `run` waits and captures output.
pub struct SystemProcessRunner;

impl ProcessRunner for SystemProcessRunner {
    fn start(&self, spec: &ProcessSpec) -> Result<(), ProcessError> {
        let mut command = std::process::Command::new(&spec.program);
        command.args(&spec.arguments);
        command.current_dir(&spec.working_directory);
        let _child = command.spawn().map_err(|error| ProcessError {
            program: spec.program.clone(),
            detail: error.to_string(),
        })?;
        Ok(())
    }

    fn run(&self, spec: &ProcessSpec) -> Result<ProcessOutput, ProcessError> {
        let mut command = std::process::Command::new(&spec.program);
        command.args(&spec.arguments);
        command.current_dir(&spec.working_directory);
        let output = command.output().map_err(|error| ProcessError {
            program: spec.program.clone(),
            detail: error.to_string(),
        })?;
        Ok(ProcessOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Test double that records every [`ProcessRunner::start`] spec instead of
/// starting anything. Its start outcome is configurable so tests can exercise
/// launch failures without a real process; queued run outcomes script
/// [`ProcessRunner::run`]. No thread-safety issue arises because the shared
/// state is a `Mutex` and a test owns the instance.
pub struct RecordingProcessRunner {
    starts: Mutex<Vec<ProcessSpec>>,
    start_outcome: Mutex<Result<(), ProcessError>>,
    run_outcomes: Mutex<VecDeque<Result<ProcessOutput, ProcessError>>>,
}

impl RecordingProcessRunner {
    /// A runner that accepts every start and returns an empty run result.
    pub fn new() -> Self {
        Self {
            starts: Mutex::new(Vec::new()),
            start_outcome: Mutex::new(Ok(())),
            run_outcomes: Mutex::new(VecDeque::new()),
        }
    }

    /// Makes every subsequent `start` fail with the given message. Used to
    /// assert that a failed launch never records a session.
    pub fn fail_starts(&self, message: impl Into<String>) {
        *self.start_outcome.lock().unwrap() = Err(ProcessError {
            program: String::new(),
            detail: message.into(),
        });
    }

    /// Queues a run result returned by the next [`ProcessRunner::run`] call.
    pub fn queue_run(&self, outcome: Result<ProcessOutput, ProcessError>) {
        self.run_outcomes.lock().unwrap().push_back(outcome);
    }

    /// Every [`ProcessRunner::start`] spec captured so far, in order.
    pub fn started(&self) -> Vec<ProcessSpec> {
        self.starts.lock().unwrap().clone()
    }

    /// Number of captured starts.
    pub fn start_count(&self) -> usize {
        self.starts.lock().unwrap().len()
    }
}

impl Default for RecordingProcessRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessRunner for RecordingProcessRunner {
    fn start(&self, spec: &ProcessSpec) -> Result<(), ProcessError> {
        self.starts.lock().unwrap().push(spec.clone());
        self.start_outcome.lock().unwrap().clone()
    }

    fn run(&self, spec: &ProcessSpec) -> Result<ProcessOutput, ProcessError> {
        let mut outcomes = self.run_outcomes.lock().unwrap();
        if let Some(outcome) = outcomes.pop_front() {
            return outcome;
        }
        drop(outcomes);
        // Unscripted runs still record the spec and succeed with empty output.
        self.starts.lock().unwrap().push(spec.clone());
        Ok(ProcessOutput {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

/// Executable resolution used for availability probing and terminal detection.
/// Implementations never launch anything: they inspect PATH lexically and via
/// filesystem metadata without invoking `where`/`which`.
pub trait ProgramResolver {
    /// Whether `program` resolves to an executable through PATH (or exists as
    /// a direct path).
    fn is_on_path(&self, program: &str) -> bool;
    /// Resolves `program` to an absolute/qualified path when present.
    fn resolve(&self, program: &str) -> Option<PathBuf>;
}

/// Production resolver that reads the process `PATH` (and `PATHEXT` on
/// Windows) and checks filesystem metadata. Never spawns a process.
pub struct PathProgramResolver;

impl ProgramResolver for PathProgramResolver {
    fn is_on_path(&self, program: &str) -> bool {
        resolve_program(program).is_some()
    }

    fn resolve(&self, program: &str) -> Option<PathBuf> {
        resolve_program(program)
    }
}

/// Resolves `program` through the process environment: `PATH` plus `PATHEXT`
/// on Windows, or as a direct path when it contains a separator. Never
/// launches anything.
pub fn resolve_program(program: &str) -> Option<PathBuf> {
    if program.trim().is_empty() {
        return None;
    }
    let path_var = std::env::var_os("PATH")
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let path_ext = std::env::var_os("PATHEXT")
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    search_path(program, &path_var, &path_ext)
}

/// Returns whether `program` is a single executable name resolved through
/// PATH, rather than an absolute or relative path. This is intentionally
/// stricter than [`search_path`], which also supports explicit paths for
/// trusted local process configuration.
pub fn is_bare_program_name(program: &str) -> bool {
    !program.is_empty()
        && program == program.trim()
        && !program.starts_with('-')
        && !matches!(program, "." | "..")
        && !program.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || matches!(character, '/' | '\\' | ':')
        })
}

/// Resolve an executable only when it is a bare PATH name. Explicit paths
/// remain available through [`resolve_program`] for trusted configuration,
/// but adapter manifests must use this narrower boundary.
pub fn resolve_bare_program(program: &str) -> Option<PathBuf> {
    is_bare_program_name(program)
        .then(|| resolve_program(program))
        .flatten()
}

/// Pure PATH search over an explicit `path_var` (and `path_ext` on Windows),
/// so tests can point it at synthetic temporary directories without touching
/// the process environment. On POSIX an executable must also carry an execute
/// permission bit.
pub fn search_path(program: &str, path_var: &str, path_ext: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.components().count() > 1 {
        return executable_file(candidate).map(|path| path.to_path_buf());
    }
    let separator = if cfg!(windows) { ';' } else { ':' };
    let extensions: Vec<String> = if cfg!(windows) {
        path_ext
            .split(';')
            .filter(|entry| !entry.trim().is_empty())
            .map(|entry| entry.trim().to_string())
            .collect()
    } else {
        Vec::new()
    };
    for directory in path_var.split(separator) {
        if directory.trim().is_empty() {
            continue;
        }
        let base = Path::new(directory).join(program);
        if cfg!(windows) {
            if Path::new(program).extension().is_none() {
                // Windows package managers such as npm place both a POSIX
                // extensionless shim and a native `.cmd` launcher on PATH.
                // `CreateProcessW` cannot execute the POSIX shim, so respect
                // PATHEXT before considering an exact extensionless file.
                for extension in &extensions {
                    let with_extension = base.with_extension(extension.trim_start_matches('.'));
                    if let Some(path) = executable_file(&with_extension) {
                        return Some(path.to_path_buf());
                    }
                }
            }
            if let Some(path) = executable_file(&base) {
                return Some(path.to_path_buf());
            }
        } else if let Some(path) = executable_file(&base) {
            return Some(path.to_path_buf());
        }
    }
    None
}

/// Filesystem check that a path is a regular file (and on POSIX carries an
/// execute permission bit). Returns the path unchanged on success.
fn executable_file(path: &Path) -> Option<&Path> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::{is_bare_program_name, resolve_bare_program, search_path};

    #[test]
    fn bare_program_names_reject_path_forms_on_all_platforms() {
        for value in [
            r"C:\tools\codex.exe",
            r"C:/tools/codex.exe",
            r"\\server\share\codex.exe",
            "/opt/tools/codex",
            "./tools/codex",
            "../tools/codex",
            "tools/codex",
            "tools\\codex",
            "C:codex",
        ] {
            assert!(!is_bare_program_name(value), "path accepted: {value}");
        }
    }

    #[test]
    fn bare_program_names_keep_valid_path_lookup_and_reject_direct_resolution() {
        assert!(is_bare_program_name("codex"));
        assert!(is_bare_program_name("my-tool.v2"));
        assert!(!is_bare_program_name(" codex "));
        assert!(!is_bare_program_name("-codex"));
        assert!(resolve_bare_program("definitely-not-installed-sessionatlas").is_none());
        assert!(search_path("tools/codex", "", "").is_none());
    }
}
