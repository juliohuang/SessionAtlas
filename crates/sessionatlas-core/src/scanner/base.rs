//! Scanner framework primitives: `Scanner` trait, `ScanOutcome`, `ScanStatus`,
//! `ScanDiagnostic`, `ScannedProject`, source probing and base outcome rules.
//!
//! Availability (`is_available`) is
//! intentionally separate from data discoverability: historical data stays
//! scannable even when the CLI executable is gone, and an installed executable
//! alone never erases the distinction between a successful inspection and an
//! unreadable source.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

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
/// Child-agent or likely one-shot delegated sessions were retained as tool
/// activity but intentionally excluded from resume targets.
pub const AUXILIARY_SESSION_FILTERED: &str = "auxiliary_session_filtered";
/// The custom-tool configuration could not be read.
pub const CONFIG_READ_FAILED: &str = "config_read_failed";
/// An installed adapter manifest was invalid or unavailable.
pub const ADAPTER_LOAD_FAILED: &str = "adapter_load_failed";
/// An unexpected scanner failure escaped the tool-specific logic.
pub const UNEXPECTED_SCANNER_FAILURE: &str = "unexpected_scanner_failure";
/// A scan exceeded one of its bounded resource budgets.
pub const SCAN_RESOURCE_LIMIT_EXCEEDED: &str = "scan_resource_limit_exceeded";
/// A scan was cancelled or its deadline elapsed.
pub const SCAN_CANCELLED: &str = "scan_cancelled";

/// Shared limits for one tool scan. Counters are deliberately shared by every
/// file and parser invocation; callers must not create one per file.
#[derive(Clone, Debug)]
pub struct ScanBudget {
    pub max_depth: usize,
    pub max_entries: u64,
    pub max_source_files: u64,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_line_bytes: usize,
    pub max_records: u64,
    pub max_duration: Duration,
    pub max_cache_bytes: u64,
    pub max_database_bytes: u64,
    cancel: Option<Arc<AtomicBool>>,
}

impl Default for ScanBudget {
    fn default() -> Self {
        Self {
            max_depth: 32,
            max_entries: 200_000,
            max_source_files: 100_000,
            max_file_bytes: 64 * 1024 * 1024,
            max_total_bytes: 512 * 1024 * 1024,
            max_line_bytes: 8 * 1024 * 1024,
            max_records: 1_000_000,
            max_duration: Duration::from_secs(60),
            max_cache_bytes: 16 * 1024 * 1024,
            max_database_bytes: 2 * 1024 * 1024 * 1024,
            cancel: None,
        }
    }
}

impl ScanBudget {
    /// Compact constructor useful for deterministic tests.
    #[allow(clippy::too_many_arguments)]
    pub fn with_limits(
        max_depth: usize,
        max_entries: u64,
        max_source_files: u64,
        max_file_bytes: u64,
        max_total_bytes: u64,
        max_line_bytes: usize,
        max_records: u64,
        max_duration: Duration,
    ) -> Self {
        Self {
            max_depth,
            max_entries,
            max_source_files,
            max_file_bytes,
            max_total_bytes,
            max_line_bytes,
            max_records,
            max_duration,
            ..Self::default()
        }
    }
    pub fn context(&self) -> ScanContext {
        let context = ScanContext::new(self.clone());
        self.cancel.as_ref().map_or(context.clone(), |cancel| {
            context.with_cancel(cancel.clone())
        })
    }
    pub fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }
}

struct BudgetState {
    entries: AtomicU64,
    files: AtomicU64,
    bytes: AtomicU64,
    records: AtomicU64,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
    clock: Arc<dyn Fn() -> Instant + Send + Sync>,
}

/// The per-scan shared accounting context. It is cheap to clone and safe to
/// pass to helpers, while all counters remain global to the tool scan.
#[derive(Clone)]
pub struct ScanContext {
    pub budget: ScanBudget,
    state: Arc<BudgetState>,
}

impl ScanContext {
    pub fn new(budget: ScanBudget) -> Self {
        let now = Instant::now();
        let deadline = now + budget.max_duration;
        Self {
            budget,
            state: Arc::new(BudgetState {
                entries: AtomicU64::new(0),
                files: AtomicU64::new(0),
                bytes: AtomicU64::new(0),
                records: AtomicU64::new(0),
                cancelled: Arc::new(AtomicBool::new(false)),
                deadline,
                clock: Arc::new(Instant::now),
            }),
        }
    }
    pub fn with_cancel(&self, cancel: Arc<AtomicBool>) -> Self {
        let old = &self.state;
        Self {
            budget: self.budget.clone(),
            state: Arc::new(BudgetState {
                entries: AtomicU64::new(old.entries.load(Ordering::Relaxed)),
                files: AtomicU64::new(old.files.load(Ordering::Relaxed)),
                bytes: AtomicU64::new(old.bytes.load(Ordering::Relaxed)),
                records: AtomicU64::new(old.records.load(Ordering::Relaxed)),
                cancelled: cancel,
                deadline: old.deadline,
                clock: old.clock.clone(),
            }),
        }
    }
    /// Builds a context with an injected monotonic clock for deterministic
    /// deadline tests.
    pub fn with_clock(budget: ScanBudget, clock: Arc<dyn Fn() -> Instant + Send + Sync>) -> Self {
        let now = clock();
        let deadline = now + budget.max_duration;
        Self {
            budget,
            state: Arc::new(BudgetState {
                entries: AtomicU64::new(0),
                files: AtomicU64::new(0),
                bytes: AtomicU64::new(0),
                records: AtomicU64::new(0),
                cancelled: Arc::new(AtomicBool::new(false)),
                deadline,
                clock,
            }),
        }
    }
    pub fn cancel(&self) {
        self.state.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn checkpoint(&self) -> Result<(), BudgetError> {
        if self.state.cancelled.load(Ordering::Relaxed)
            || (self.state.clock)() >= self.state.deadline
        {
            return Err(BudgetError::Cancelled);
        }
        Ok(())
    }
    pub fn entry(&self, depth: usize) -> Result<(), BudgetError> {
        self.checkpoint()?;
        if depth > self.budget.max_depth {
            return Err(BudgetError::Exceeded);
        }
        let n = self
            .state
            .entries
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        if n > self.budget.max_entries {
            return Err(BudgetError::Exceeded);
        }
        Ok(())
    }
    pub fn source_file(&self, size: u64) -> Result<(), BudgetError> {
        self.checkpoint()?;
        if size > self.budget.max_file_bytes {
            return Err(BudgetError::Exceeded);
        }
        let n = self
            .state
            .files
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        if n > self.budget.max_source_files {
            return Err(BudgetError::Exceeded);
        }
        Ok(())
    }
    pub fn source_file_size(&self, path: &Path) -> Result<u64, BudgetError> {
        let metadata = no_follow_metadata(path).map_err(|_| BudgetError::Exceeded)?;
        if !metadata.is_file() || metadata.len() > self.budget.max_file_bytes {
            return Err(BudgetError::Exceeded);
        }
        Ok(metadata.len())
    }
    pub fn source_file_path(&self, path: &Path) -> Result<(), BudgetError> {
        let size = self.source_file_size(path)?;
        self.source_file(size)
    }
    pub fn database_file(&self, path: &Path) -> Result<u64, BudgetError> {
        let metadata = no_follow_metadata(path).map_err(|_| BudgetError::Exceeded)?;
        if !metadata.is_file() || metadata.len() > self.budget.max_database_bytes {
            return Err(BudgetError::Exceeded);
        }
        self.checkpoint()?;
        let n = self
            .state
            .files
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        if n > self.budget.max_source_files {
            return Err(BudgetError::Exceeded);
        }
        Ok(metadata.len())
    }
    /// Validates the main SQLite file and any readable WAL/journal sidecars.
    /// Database footprint has its own ceiling and is intentionally not charged
    /// to total source bytes: SQLite page reads are controlled by its progress
    /// handler rather than guessed from file size.
    pub fn database_footprint(&self, path: &Path) -> Result<u64, BudgetError> {
        let mut total = 0u64;
        for suffix in ["", "-wal", "-journal"] {
            let sidecar = if suffix.is_empty() {
                path.to_path_buf()
            } else {
                PathBuf::from(format!("{}{}", path.to_string_lossy(), suffix))
            };
            match no_follow_metadata(&sidecar) {
                Ok(metadata) if metadata.is_file() => {
                    total = total.saturating_add(metadata.len());
                    if total > self.budget.max_database_bytes {
                        return Err(BudgetError::Exceeded);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound && !suffix.is_empty() => {}
                Ok(_) | Err(_) => return Err(BudgetError::Exceeded),
            }
        }
        self.source_file(0)?;
        self.checkpoint()?;
        Ok(total)
    }
    pub fn bytes(&self, amount: u64) -> Result<(), BudgetError> {
        self.checkpoint()?;
        let prior = self.state.bytes.fetch_add(amount, Ordering::Relaxed);
        if prior.saturating_add(amount) > self.budget.max_total_bytes {
            return Err(BudgetError::Exceeded);
        }
        Ok(())
    }
    pub fn release_bytes(&self, amount: u64) {
        let _ = self
            .state
            .bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_sub(amount))
            });
    }
    pub fn record(&self) -> Result<(), BudgetError> {
        self.checkpoint()?;
        let n = self
            .state
            .records
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        if n > self.budget.max_records {
            return Err(BudgetError::Exceeded);
        }
        Ok(())
    }
    pub fn diagnostic(&self, tool: &str, error: BudgetError) -> ScanDiagnostic {
        let (code, message) = match error {
            BudgetError::Exceeded => (
                SCAN_RESOURCE_LIMIT_EXCEEDED,
                "The scan exceeded its bounded resource budget; the previous index is preserved.",
            ),
            BudgetError::Cancelled => (
                SCAN_CANCELLED,
                "The scan was cancelled or exceeded its deadline; the previous index is preserved.",
            ),
        };
        ScanDiagnostic::new(tool, ScanDiagnosticSeverity::Error, code, message)
    }
    pub fn budget_error(&self) -> Option<BudgetError> {
        if self.state.cancelled.load(Ordering::Relaxed)
            || (self.state.clock)() >= self.state.deadline
        {
            return Some(BudgetError::Cancelled);
        }
        if self.state.entries.load(Ordering::Relaxed) > self.budget.max_entries
            || self.state.files.load(Ordering::Relaxed) > self.budget.max_source_files
            || self.state.bytes.load(Ordering::Relaxed) > self.budget.max_total_bytes
            || self.state.records.load(Ordering::Relaxed) > self.budget.max_records
        {
            return Some(BudgetError::Exceeded);
        }
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetError {
    Exceeded,
    Cancelled,
}

/// Metadata which never follows links (and rejects Windows reparse points).
pub fn no_follow_metadata(path: &Path) -> io::Result<std::fs::Metadata> {
    let metadata = std::fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return Err(io::Error::other("reparse point"));
        }
    }
    if metadata.file_type().is_symlink() {
        return Err(io::Error::other("symlink"));
    }
    Ok(metadata)
}

/// Opens a regular file without following a link introduced after the path
/// check. The returned handle, rather than the path, is the source of truth.
pub(crate) fn open_regular_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return Err(io::Error::other("reparse point"));
        }
    }
    if !metadata.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    Ok(file)
}

/// Bounded physical-line reader. Every returned record has consumed bytes from
/// the shared total budget, including malformed records and a final partial line.
pub struct BoundedLines {
    reader: BufReader<std::fs::File>,
    context: ScanContext,
    file_bytes: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoundedLineError {
    Budget(BudgetError),
    Io,
}
impl BoundedLines {
    pub(crate) fn open(path: &Path, context: &ScanContext) -> Result<Self, BoundedLineError> {
        let metadata = no_follow_metadata(path)
            .map_err(|_| BoundedLineError::Budget(BudgetError::Exceeded))?;
        if !metadata.is_file() {
            return Err(BoundedLineError::Budget(BudgetError::Exceeded));
        }
        let file = open_regular_file(path).map_err(|_| BoundedLineError::Io)?;
        Ok(Self {
            reader: BufReader::new(file),
            context: context.clone(),
            file_bytes: 0,
        })
    }
    pub(crate) fn next_line(&mut self) -> Result<Option<Vec<u8>>, BoundedLineError> {
        self.context
            .checkpoint()
            .map_err(BoundedLineError::Budget)?;
        let mut bytes = Vec::new();
        loop {
            let buffer = self.reader.fill_buf().map_err(|_| BoundedLineError::Io)?;
            if buffer.is_empty() {
                break;
            }
            let newline = buffer.iter().position(|byte| *byte == b'\n');
            let take = newline.map_or(buffer.len(), |position| position + 1);
            if bytes.len().saturating_add(take) > self.context.budget.max_line_bytes {
                return Err(BoundedLineError::Budget(BudgetError::Exceeded));
            }
            if self.file_bytes.saturating_add(take as u64) > self.context.budget.max_file_bytes {
                return Err(BoundedLineError::Budget(BudgetError::Exceeded));
            }
            self.context
                .bytes(take as u64)
                .map_err(BoundedLineError::Budget)?;
            bytes.extend_from_slice(&buffer[..take]);
            self.reader.consume(take);
            self.file_bytes = self.file_bytes.saturating_add(take as u64);
            if newline.is_some() {
                break;
            }
        }
        if bytes.is_empty() {
            return Ok(None);
        }
        self.context.record().map_err(BoundedLineError::Budget)?;
        Ok(Some(bytes))
    }
}

/// Bounded whole-file read for JSON/settings/metadata files.
pub fn read_bounded_file(path: &Path, context: &ScanContext) -> Result<Vec<u8>, BudgetError> {
    read_bounded_file_detailed(path, context).map_err(|error| match error {
        BoundedFileError::Budget(error) => error,
        BoundedFileError::Io => BudgetError::Exceeded,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoundedFileError {
    Budget(BudgetError),
    Io,
}

pub(crate) fn read_bounded_file_detailed(
    path: &Path,
    context: &ScanContext,
) -> Result<Vec<u8>, BoundedFileError> {
    let metadata =
        no_follow_metadata(path).map_err(|_| BoundedFileError::Budget(BudgetError::Exceeded))?;
    if !metadata.is_file() || metadata.len() > context.budget.max_file_bytes {
        return Err(BoundedFileError::Budget(BudgetError::Exceeded));
    }
    context
        .source_file(metadata.len())
        .map_err(BoundedFileError::Budget)?;
    let mut file = open_regular_file(path).map_err(|_| BoundedFileError::Io)?;
    context
        .bytes(metadata.len())
        .map_err(BoundedFileError::Budget)?;
    let mut output = Vec::with_capacity(metadata.len().min(1024 * 1024) as usize);
    let mut limited = (&mut file).take(context.budget.max_file_bytes.saturating_add(1));
    limited.read_to_end(&mut output).map_err(|_| {
        context.release_bytes(metadata.len().saturating_sub(output.len() as u64));
        BoundedFileError::Io
    })?;
    if output.len() as u64 > metadata.len() {
        context
            .bytes(output.len() as u64 - metadata.len())
            .map_err(BoundedFileError::Budget)?;
    } else {
        context.release_bytes(metadata.len() - output.len() as u64);
    }
    if output.len() as u64 > context.budget.max_file_bytes || output.len() as u64 > metadata.len() {
        return Err(BoundedFileError::Budget(BudgetError::Exceeded));
    }
    if output.len() as u64 != metadata.len() {
        return Err(BoundedFileError::Budget(BudgetError::Exceeded));
    }
    Ok(output)
}

/// Bounded, no-follow recursive enumeration. A link/reparse point is an
/// unsafe source rather than an omitted entry, and depth overflow is fatal.
pub fn bounded_recursive_files(
    path: &Path,
    context: &ScanContext,
) -> Result<Vec<PathBuf>, BudgetError> {
    let mut files = Vec::new();
    for item in walkdir::WalkDir::new(path).follow_links(false).into_iter() {
        let entry = item.map_err(|_| BudgetError::Exceeded)?;
        context.entry(entry.depth())?;
        if entry.file_type().is_symlink() {
            continue;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if entry
                .metadata()
                .map_err(|_| BudgetError::Exceeded)?
                .file_attributes()
                & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                != 0
            {
                continue;
            }
        }
        if entry.file_type().is_file() {
            files.push(entry.into_path());
        }
    }
    files.sort();
    Ok(files)
}

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
    match no_follow_metadata(path) {
        Ok(metadata) if metadata.is_dir() => SourceProbe::Exists,
        Ok(_) => SourceProbe::Failed,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => SourceProbe::Missing,
        Err(_) => SourceProbe::Failed,
    }
}

/// Probes a file source path. A path that exists but is a directory, and any
/// non-NotFound access error, both classify as [`SourceProbe::Failed`].
pub fn probe_file(path: &Path) -> SourceProbe {
    match no_follow_metadata(path) {
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
