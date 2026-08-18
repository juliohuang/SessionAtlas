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

use std::path::Path;

use crate::adapter::{adapter_root_for_config, AdapterRegistry, RegisteredAdapter};
use crate::config;
use crate::model::{ToolSource, DEFAULT_OPEN_COMMAND_TEMPLATE};

pub mod aider;
pub mod base;
mod cache;
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
    ADAPTER_LOAD_FAILED, AUXILIARY_SESSION_FILTERED, CONFIG_READ_FAILED, MALFORMED_SESSION_RECORD,
    MISSING_PROJECT_PATH, MISSING_SESSION_ID, NO_VALID_SESSIONS, SESSION_READ_FAILED,
    SOURCE_READ_FAILED, SOURCE_UNAVAILABLE, TIMESTAMP_FALLBACK, UNEXPECTED_SCANNER_FAILURE,
};
pub use parsing::{
    expand_tilde, home_directory, recursive_file_enumeration, trim_trailing_separators,
    try_normalize_project_path, try_read_unix_timestamp, try_read_utc_timestamp,
};

/// Builds the scanner set from the active adapter registry, then appends
/// legacy `CustomTools` entries that do not collide with an adapter identity.
/// Missing config keeps the bundled adapters enabled; malformed config emits a
/// warning and uses the same safe fallback.
pub fn build_adapter_scanners(config_path: &Path) -> (Vec<Box<dyn Scanner>>, Vec<ScanDiagnostic>) {
    let mut diagnostics = Vec::new();
    let config = match config::load(config_path) {
        Ok(config) => config,
        Err(_) => {
            diagnostics.push(ScanDiagnostic::new(
                "config",
                ScanDiagnosticSeverity::Warning,
                CONFIG_READ_FAILED,
                "The adapter configuration could not be read; bundled adapters remain available.",
            ));
            config::AppConfig::default()
        }
    };
    let registry = match AdapterRegistry::load(&adapter_root_for_config(config_path), &config) {
        Ok(registry) => registry,
        Err(error) => {
            diagnostics.push(ScanDiagnostic::new(
                "adapters",
                ScanDiagnosticSeverity::Error,
                ADAPTER_LOAD_FAILED,
                format!("The adapter registry could not be loaded: {error}"),
            ));
            return (Vec::new(), diagnostics);
        }
    };
    diagnostics.extend(registry.diagnostics().iter().map(|message| {
        ScanDiagnostic::new(
            "adapters",
            ScanDiagnosticSeverity::Warning,
            ADAPTER_LOAD_FAILED,
            message.clone(),
        )
    }));

    let mut scanners = Vec::<Box<dyn Scanner>>::new();
    for adapter in registry
        .enabled(&config)
        .filter(|adapter| adapter.supports_platform(std::env::consts::OS))
    {
        match scanner_from_adapter(adapter) {
            Ok(scanner) => scanners.push(scanner),
            Err(message) => diagnostics.push(ScanDiagnostic::new(
                adapter.id.clone(),
                ScanDiagnosticSeverity::Warning,
                ADAPTER_LOAD_FAILED,
                message,
            )),
        }
    }
    for tool in config.custom_tools.iter().filter(|tool| tool.is_enabled) {
        if scanners
            .iter()
            .any(|scanner| scanner.tool_key().eq_ignore_ascii_case(&tool.key))
        {
            continue;
        }
        scanners.push(Box::new(custom::CustomToolScanner::new(tool.clone())));
    }
    (scanners, diagnostics)
}

fn scanner_from_adapter(adapter: &RegisteredAdapter) -> Result<Box<dyn Scanner>, String> {
    let scanner: Box<dyn Scanner> = match adapter.scanner.handler.as_str() {
        "builtin.claude" => Box::new(claude::ClaudeScanner::new()),
        "builtin.codex" => Box::new(codex::CodexScanner::new()),
        "builtin.kimi" => Box::new(kimi::KimiScanner::new()),
        "builtin.opencode" => Box::new(opencode::OpenCodeScanner::new()),
        "builtin.aider" => Box::new(aider::AiderScanner::new()),
        "builtin.pi" => Box::new(pi::PiScanner::new()),
        "metadata-v1" => Box::new(custom::CustomToolScanner::new(ToolSource {
            key: adapter.id.clone(),
            name: adapter.name.clone(),
            cli_command: adapter.command.clone(),
            data_directory: adapter
                .scanner
                .data_directory
                .clone()
                .ok_or_else(|| "metadata-v1 adapter is missing dataDirectory".to_string())?,
            scanner_type: "metadata-v1".to_string(),
            is_installed: false,
            is_enabled: true,
            open_command_template: DEFAULT_OPEN_COMMAND_TEMPLATE.to_string(),
        })),
        handler => return Err(format!("unsupported adapter scanner handler: {handler}")),
    };
    Ok(scanner)
}

#[cfg(test)]
mod adapter_factory_tests {
    use super::*;

    #[test]
    fn default_factory_uses_the_six_bundled_adapter_scanners() {
        let temporary = tempfile::tempdir().unwrap();
        let config_path = temporary.path().join("config.json");
        let (scanners, diagnostics) = build_adapter_scanners(&config_path);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(
            scanners
                .iter()
                .map(|scanner| scanner.tool_key())
                .collect::<Vec<_>>(),
            vec!["claude", "kimi", "codex", "opencode", "aider", "pi"]
        );
    }
}
