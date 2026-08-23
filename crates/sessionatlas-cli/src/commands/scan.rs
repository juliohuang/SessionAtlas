//! `sessionatlas scan` — run the configured scanners and atomically replace
//! the tool snapshots of every successfully scanned tool.
//!
//! Task R09 implements the scanning pipeline that R08 left as a stub. The
//! scanner set is injected so tests drive the contract with fake scanners and
//! never touch real tool data directories; production builds the canonical set
//! (six built-ins plus enabled, non-colliding custom tools) from the config
//! file. Only `ScanStatus::Succeeded` tools feed `replace_tool_snapshots` —
//! `Failed`/`Unavailable` outcomes and panics preserve the previous snapshot.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use sessionatlas_core::config::load as load_config;
use sessionatlas_core::indexer::{build_index, IndexedToolScan};
use sessionatlas_core::scanner::aider::AiderScanner;
use sessionatlas_core::scanner::claude::ClaudeScanner;
use sessionatlas_core::scanner::codex::CodexScanner;
use sessionatlas_core::scanner::custom::CustomToolScanner;
use sessionatlas_core::scanner::kimi::KimiScanner;
use sessionatlas_core::scanner::opencode::OpenCodeScanner;
use sessionatlas_core::scanner::pi::PiScanner;
use sessionatlas_core::scanner::{
    ScanDiagnostic, ScanDiagnosticSeverity, ScanStatus, ScannedProject, Scanner,
    CONFIG_READ_FAILED, UNEXPECTED_SCANNER_FAILURE,
};
use sessionatlas_core::store::SqliteStore;

use crate::cli::ScanArgs;
use crate::render::sanitize;
use crate::Io;

/// Builds the canonical scanner set for the config file at `config_path`: the
/// six built-in scanners, then each enabled custom
/// tool whose key does not collide with a built-in (case-insensitive). When
/// the config cannot be read or parsed, built-ins remain available and a
/// `config_read_failed` diagnostic is returned, mirroring `ScannerRegistry`.
pub fn build_default_scanners(config_path: &Path) -> (Vec<Box<dyn Scanner>>, Vec<ScanDiagnostic>) {
    let mut scanners: Vec<Box<dyn Scanner>> = vec![
        Box::new(ClaudeScanner::new()),
        Box::new(KimiScanner::new()),
        Box::new(CodexScanner::new()),
        Box::new(OpenCodeScanner::new()),
        Box::new(AiderScanner::new()),
        Box::new(PiScanner::new()),
    ];
    let mut diagnostics = Vec::new();
    match load_config(config_path) {
        Ok(config) => {
            for tool in config.custom_tools.iter().filter(|tool| tool.is_enabled) {
                if scanners
                    .iter()
                    .any(|scanner| scanner.tool_key().eq_ignore_ascii_case(&tool.key))
                {
                    continue;
                }
                scanners.push(Box::new(CustomToolScanner::new(tool.clone())));
            }
        }
        Err(_) => diagnostics.push(ScanDiagnostic::new(
            "config",
            ScanDiagnosticSeverity::Warning,
            CONFIG_READ_FAILED,
            "The custom-tool configuration could not be read; built-in scanners remain available.",
        )),
    }
    (scanners, diagnostics)
}

/// Runs `scan` against an injected scanner list. Only successful outcomes are
/// passed to `replace_tool_snapshots`; a missing or failing scan leaves the
/// previous index untouched. Exit codes:
/// * `0` — at least one tool produced a trustworthy snapshot and the index was
///   atomically updated;
/// * `1` — an unknown `--tool`, zero successful tools (no database access), a
///   store failure, or a diagnostic-worthy configuration problem.
pub fn run_scan(
    io: &mut Io<'_>,
    db_path: &Path,
    args: &ScanArgs,
    scanners: &[Box<dyn Scanner>],
    initial_diagnostics: &[ScanDiagnostic],
) -> i32 {
    let selected = match select_scanners(scanners, args.tool.as_deref()) {
        Ok(selected) => selected,
        Err(message) => {
            io.err(&format!("{message}\n"));
            return 1;
        }
    };

    io.out(&format!("将扫描 {} 个工具...\n\n", selected.len()));

    let mut diagnostics: Vec<ScanDiagnostic> = initial_diagnostics.to_vec();
    let mut successful: Vec<(String, String, Vec<ScannedProject>)> = Vec::new();
    let mut skipped: Vec<(&str, ScanStatus)> = Vec::new();

    for scanner in selected {
        let outcome = match catch_unwind(AssertUnwindSafe(|| scanner.scan())) {
            Ok(outcome) => outcome,
            Err(_) => {
                diagnostics.push(ScanDiagnostic::new(
                    scanner.tool_key(),
                    ScanDiagnosticSeverity::Error,
                    UNEXPECTED_SCANNER_FAILURE,
                    "The scanner stopped unexpectedly; its previous index is preserved.",
                ));
                skipped.push((scanner.tool_key(), ScanStatus::Failed));
                continue;
            }
        };
        diagnostics.extend(outcome.diagnostics().iter().cloned());
        if outcome.is_successful() {
            successful.push((
                scanner.tool_key().to_string(),
                scanner.tool_name().to_string(),
                outcome.into_projects(),
            ));
        } else {
            skipped.push((scanner.tool_key(), outcome.status()));
        }
    }

    for diagnostic in &diagnostics {
        io.err(&format!(
            "{} · {}: {}\n",
            sanitize(&diagnostic.tool_key),
            sanitize(diagnostic.code),
            sanitize(&diagnostic.message)
        ));
    }

    if successful.is_empty() {
        io.err("没有工具产生可信快照，索引未发生变化。\n");
        return 1;
    }

    let total_raw: usize = successful
        .iter()
        .map(|(_, _, projects)| projects.len())
        .sum();
    io.out(&format!("\n原始扫描结果: {total_raw} 条\n"));

    let tool_scans: Vec<IndexedToolScan> = successful
        .iter()
        .map(|(key, name, projects)| IndexedToolScan {
            tool_key: key.clone(),
            tool_name: name.clone(),
            projects: projects.clone(),
        })
        .collect();
    let projects = build_index(&tool_scans);
    io.out(&format!("去重合并后: {} 个项目\n", projects.len()));

    let mut store = match SqliteStore::new(db_path) {
        Ok(store) => store,
        Err(error) => {
            io.err(&format!("创建索引数据库失败: {error}\n"));
            return 1;
        }
    };
    let scanned_keys: Vec<&str> = successful.iter().map(|(key, _, _)| key.as_str()).collect();
    if let Err(error) = store.replace_tool_snapshots(&projects, &scanned_keys) {
        io.err(&format!("更新索引失败: {error}\n"));
        return 1;
    }

    io.out("索引已原子更新到本地数据库。\n");
    match store.refresh_project_content_index() {
        Ok(stats) => io.out(&format!(
            "内容索引: {} 个项目，更新 {} 个文件，复用 {} 个文件，索引 {} 字节。\n",
            stats.projects_scanned, stats.files_indexed, stats.files_reused, stats.indexed_bytes
        )),
        Err(error) => io.err(&format!(
            "内容索引更新失败，项目名和路径搜索仍可用: {error}\n"
        )),
    }
    if !skipped.is_empty() {
        io.err(&format!("{} 个工具保留了上一份索引。\n", skipped.len()));
    }
    0
}

/// Selects scanners by the optional `--tool` filter (case-insensitive).
/// A non-blank filter that matches no scanner is an error so an unknown tool
/// exits non-zero before any database access.
fn select_scanners<'a>(
    scanners: &'a [Box<dyn Scanner>],
    filter: Option<&str>,
) -> Result<Vec<&'a dyn Scanner>, String> {
    let Some(filter) = filter.map(str::trim).filter(|filter| !filter.is_empty()) else {
        return Ok(scanners.iter().map(|scanner| scanner.as_ref()).collect());
    };
    let selected: Vec<&dyn Scanner> = scanners
        .iter()
        .map(|scanner| scanner.as_ref())
        .filter(|scanner| scanner.tool_key().eq_ignore_ascii_case(filter))
        .collect();
    if selected.is_empty() {
        return Err(format!("未检测到可扫描的工具：{}", sanitize(filter)));
    }
    Ok(selected)
}
