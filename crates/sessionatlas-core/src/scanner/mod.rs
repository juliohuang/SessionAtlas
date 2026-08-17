//! Scanner framework: `Scanner` trait, `ScanOutcome` / `ScanStatus`,
//! `ScanDiagnostic`, availability vs. data discoverability separation.
//!
//! `base` owns the framework primitives (`Scanner`, outcomes, diagnostics,
//! source probing, base outcome rules); `parsing` owns the shared pure
//! parsing/normalization helpers (timestamps, paths, enumeration policy).
//! Both are re-exported at this module's root so concrete scanners and the
//! rest of the crate consume one stable surface.
//!
//! Module layout (each scanner is owned by exactly one migration task and is
//! predeclared here so later tasks never need to edit this file):
//!
//! | module   | owned by |
//! |----------|----------|
//! | base     | R04      |
//! | parsing  | R04      |
//! | codex    | R05A     |
//! | claude   | R05A     |
//! | kimi     | R05B     |
//! | opencode | R05B     |
//! | aider    | R05C     |
//! | custom   | R05C     |
//! | pi       | native   |

pub mod aider;
pub mod base;
pub mod claude;
pub mod codex;
pub mod custom;
pub mod kimi;
pub mod opencode;
pub mod parsing;
pub mod pi;

pub use base::{
    complete_session_files, missing_source, probe_directory, probe_file, source_read_failure,
    ScanDiagnostic, ScanDiagnosticSeverity, ScanOutcome, ScanStatus, ScannedProject, Scanner,
    SourceProbe,
};
pub use base::{
    CONFIG_READ_FAILED, MALFORMED_SESSION_RECORD, MISSING_PROJECT_PATH, MISSING_SESSION_ID,
    NO_VALID_SESSIONS, SESSION_READ_FAILED, SOURCE_READ_FAILED, SOURCE_UNAVAILABLE,
    TIMESTAMP_FALLBACK, UNEXPECTED_SCANNER_FAILURE,
};
pub use parsing::{
    expand_tilde, home_directory, recursive_file_enumeration, trim_trailing_separators,
    try_normalize_project_path, try_read_unix_timestamp, try_read_utc_timestamp,
};
