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

use sessionatlas_core::indexer::{build_index, IndexedToolScan};
use sessionatlas_core::scan_coordinator::IndexWriteGuard;
use sessionatlas_core::scanner::{
    build_adapter_scanners, ScanDiagnostic, ScanDiagnosticSeverity, ScanStatus, ScannedProject,
    Scanner, UNEXPECTED_SCANNER_FAILURE,
};
use sessionatlas_core::store::SqliteStore;

use crate::cli::ScanArgs;
use crate::render::sanitize;
use crate::Io;

/// Builds the canonical scanner set for the config file at `config_path`: the
/// six built-in scanners in canonical registration order, then each enabled custom
/// tool whose key does not collide with a built-in (case-insensitive). When
/// the config cannot be read or parsed, built-ins remain available and a
/// a `config_read_failed` diagnostic is returned.
pub fn build_default_scanners(config_path: &Path) -> (Vec<Box<dyn Scanner>>, Vec<ScanDiagnostic>) {
    build_adapter_scanners(config_path)
}

/// Runs `scan` against an injected scanner list. Only successful outcomes are
/// passed to `replace_tool_snapshots`; a missing or failing scan leaves the
/// previous index untouched. Exit codes:
/// * `0` — at least one tool produced a trustworthy snapshot and the index was
///   atomically updated;
/// * `1` — an unknown `--tool`, zero successful tools (no database writes), a
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

    let mut expected_generation = match SqliteStore::read_scan_generation_at(db_path) {
        Ok(generation) => generation,
        Err(error) => {
            io.err(&format!(
                "无法读取索引扫描代际: {}\n",
                sanitize(&error.to_string())
            ));
            return 1;
        }
    };
    let mut collection = collect_scans(&selected, initial_diagnostics);
    emit_diagnostics(io, &collection.diagnostics);
    if collection.successful.is_empty() {
        io.err("没有工具产生可信快照，索引未发生变化。\n");
        return 1;
    }

    let total_raw: usize = collection
        .successful
        .iter()
        .map(|(_, _, projects)| projects.len())
        .sum();
    io.out(&format!("\n原始扫描结果: {total_raw} 条\n"));

    let guard = match IndexWriteGuard::acquire(db_path) {
        Ok(guard) => guard,
        Err(error) => {
            io.err(&format!(
                "索引写锁不可用: {}\n",
                sanitize(&error.to_string())
            ));
            return 1;
        }
    };
    // A scan can finish after another writer commits. Keep the lock while
    // refreshing stale results so this writer cannot be overtaken again.
    let projects;
    let mut retried_under_lock = false;
    loop {
        let current_generation = match SqliteStore::read_scan_generation_at(db_path) {
            Ok(generation) => generation,
            Err(error) => {
                io.err(&format!(
                    "无法复核索引扫描代际: {}\n",
                    sanitize(&error.to_string())
                ));
                return 1;
            }
        };
        if current_generation == expected_generation {
            let tool_scans: Vec<IndexedToolScan> = collection
                .successful
                .iter()
                .map(|(key, name, projects)| IndexedToolScan {
                    tool_key: key.clone(),
                    tool_name: name.clone(),
                    projects: projects.clone(),
                })
                .collect();
            projects = build_index(&tool_scans);
            break;
        }
        if retried_under_lock {
            io.err("持有索引写锁期间扫描代际仍发生变化；已拒绝覆盖并保留现有索引。\n");
            return 1;
        }
        retried_under_lock = true;
        io.out("索引已有更新，正在锁内刷新扫描结果...\n");
        expected_generation = current_generation;
        collection = collect_scans(&selected, initial_diagnostics);
        emit_diagnostics(io, &collection.diagnostics);
        if collection.successful.is_empty() {
            io.err("重试扫描未产生可信快照，索引未发生变化。\n");
            return 1;
        }
    }
    io.out(&format!("去重合并后: {} 个项目\n", projects.len()));
    let mut store = match SqliteStore::new_with_guard(db_path, guard) {
        Ok(store) => store,
        Err(error) => {
            io.err(&format!("创建索引数据库失败: {error}\n"));
            return 1;
        }
    };
    let scanned_keys: Vec<&str> = collection
        .successful
        .iter()
        .map(|(key, _, _)| key.as_str())
        .collect();
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
    if !collection.skipped.is_empty() {
        io.err(&format!(
            "{} 个工具保留了上一份索引。\n",
            collection.skipped.len()
        ));
    }
    0
}

struct ScanCollection {
    diagnostics: Vec<ScanDiagnostic>,
    successful: Vec<(String, String, Vec<ScannedProject>)>,
    skipped: Vec<(String, ScanStatus)>,
}

fn collect_scans(
    scanners: &[&dyn Scanner],
    initial_diagnostics: &[ScanDiagnostic],
) -> ScanCollection {
    let mut diagnostics = initial_diagnostics.to_vec();
    let mut successful = Vec::new();
    let mut skipped = Vec::new();
    for scanner in scanners {
        let outcome = match catch_unwind(AssertUnwindSafe(|| scanner.scan())) {
            Ok(outcome) => outcome,
            Err(_) => {
                diagnostics.push(ScanDiagnostic::new(
                    scanner.tool_key(),
                    ScanDiagnosticSeverity::Error,
                    UNEXPECTED_SCANNER_FAILURE,
                    "The scanner stopped unexpectedly; its previous index is preserved.",
                ));
                skipped.push((scanner.tool_key().to_string(), ScanStatus::Failed));
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
            skipped.push((scanner.tool_key().to_string(), outcome.status()));
        }
    }
    ScanCollection {
        diagnostics,
        successful,
        skipped,
    }
}

fn emit_diagnostics(io: &mut Io<'_>, diagnostics: &[ScanDiagnostic]) {
    for diagnostic in diagnostics {
        io.err(&format!(
            "{} · {}: {}\n",
            sanitize(&diagnostic.tool_key),
            sanitize(diagnostic.code),
            sanitize(&diagnostic.message)
        ));
    }
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
