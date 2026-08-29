use std::sync::{atomic::AtomicBool, Arc};
use std::time::Duration;

#[cfg(unix)]
use sessionatlas_core::model::ToolSource;
use sessionatlas_core::scanner::codex::CodexScanner;
#[cfg(unix)]
use sessionatlas_core::scanner::custom::CustomToolScanner;
use sessionatlas_core::scanner::kimi::KimiScanner;
use sessionatlas_core::scanner::{
    bounded_recursive_files, BudgetError, ScanBudget, ScanContext, ScanStatus, Scanner,
    SCAN_CANCELLED, SCAN_RESOURCE_LIMIT_EXCEEDED,
};
#[cfg(unix)]
use sessionatlas_core::scanner::{probe_directory, SourceProbe};
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_home<R>(home: &std::path::Path, body: impl FnOnce() -> R) -> R {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let previous = std::env::var_os("SESSIONATLAS_HOME");
    std::env::set_var("SESSIONATLAS_HOME", home);
    struct Restore(Option<std::ffi::OsString>);
    impl Drop for Restore {
        fn drop(&mut self) {
            match &self.0 {
                Some(value) => std::env::set_var("SESSIONATLAS_HOME", value),
                None => std::env::remove_var("SESSIONATLAS_HOME"),
            }
        }
    }
    let _restore = Restore(previous);
    body()
}

fn codex_file(home: &std::path::Path, body: &str) {
    let path = home.join(".codex/sessions/2026/08/26/session.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

#[test]
fn scanner_resource_limit_fails_without_partial_projects() {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    codex_file(
        home.path(),
        &format!(
            r#"{{"type":"session_meta","payload":{{"id":"s","cwd":{}}}}}
"#,
            serde_json::to_string(&project.to_string_lossy()).unwrap()
        ),
    );
    let budget = ScanBudget::with_limits(
        32,
        200_000,
        100_000,
        64 * 1024 * 1024,
        1,
        8 * 1024 * 1024,
        1_000_000,
        Duration::from_secs(60),
    );
    let outcome = with_home(home.path(), || {
        CodexScanner::with_availability(|| true)
            .with_budget(budget)
            .scan()
    });
    assert_eq!(outcome.status(), ScanStatus::Failed);
    assert!(outcome.projects().is_empty());
    assert_eq!(outcome.diagnostics()[0].code, SCAN_RESOURCE_LIMIT_EXCEEDED);
}

#[test]
fn scanner_expired_and_cancelled_contexts_are_structured_errors() {
    let expired = ScanContext::new(ScanBudget::with_limits(
        32,
        1,
        1,
        1,
        1,
        1,
        1,
        Duration::ZERO,
    ));
    assert_eq!(expired.checkpoint(), Err(BudgetError::Cancelled));
    let cancel = Arc::new(AtomicBool::new(true));
    let context = ScanBudget::default().with_cancel(cancel).context();
    assert_eq!(context.checkpoint(), Err(BudgetError::Cancelled));
    assert_eq!(
        context.diagnostic("codex", BudgetError::Cancelled).code,
        SCAN_CANCELLED
    );
}

#[test]
fn scanner_bounded_enumeration_rejects_excess_depth_and_entries() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("a/b/c")).unwrap();
    std::fs::write(home.path().join("a/b/c/session.jsonl"), b"{}\n").unwrap();
    let depth =
        ScanBudget::with_limits(1, 100, 100, 100, 100, 100, 100, Duration::from_secs(1)).context();
    assert_eq!(
        bounded_recursive_files(home.path(), &depth),
        Err(BudgetError::Exceeded)
    );
    let entries =
        ScanBudget::with_limits(32, 1, 100, 100, 100, 100, 100, Duration::from_secs(1)).context();
    assert_eq!(
        bounded_recursive_files(home.path(), &entries),
        Err(BudgetError::Exceeded)
    );
}

#[test]
fn scanner_file_line_record_and_total_boundaries_fail_closed() {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let valid = format!(
        r#"{{"type":"session_meta","payload":{{"id":"s","cwd":{}}}}}"#,
        serde_json::to_string(&project.to_string_lossy()).unwrap()
    );
    codex_file(home.path(), &format!("{valid}\n{valid}\n"));
    let line_budget = ScanBudget::with_limits(
        32,
        200_000,
        100_000,
        64 * 1024 * 1024,
        512 * 1024 * 1024,
        valid.len(),
        1_000_000,
        Duration::from_secs(60),
    );
    let outcome = with_home(home.path(), || {
        CodexScanner::with_availability(|| true)
            .with_budget(line_budget)
            .scan()
    });
    assert_eq!(outcome.status(), ScanStatus::Failed);
    assert!(outcome.projects().is_empty());
    let record_budget = ScanBudget::with_limits(
        32,
        200_000,
        100_000,
        64 * 1024 * 1024,
        512 * 1024 * 1024,
        8 * 1024 * 1024,
        1,
        Duration::from_secs(60),
    );
    let outcome = with_home(home.path(), || {
        CodexScanner::with_availability(|| true)
            .with_budget(record_budget)
            .scan()
    });
    assert_eq!(outcome.status(), ScanStatus::Failed);
    assert!(outcome.projects().is_empty());
}

#[test]
fn scanner_real_scanner_deadline_and_cancel_are_failed() {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    codex_file(
        home.path(),
        &format!(
            r#"{{"type":"session_meta","payload":{{"id":"s","cwd":{}}}}}
"#,
            serde_json::to_string(&project.to_string_lossy()).unwrap()
        ),
    );
    let expired = ScanBudget::with_limits(
        32,
        200_000,
        100_000,
        64 * 1024 * 1024,
        512 * 1024 * 1024,
        8 * 1024 * 1024,
        1_000_000,
        Duration::ZERO,
    );
    let outcome = with_home(home.path(), || {
        CodexScanner::with_availability(|| true)
            .with_budget(expired)
            .scan()
    });
    assert_eq!(outcome.status(), ScanStatus::Failed);
    assert_eq!(outcome.diagnostics()[0].code, SCAN_CANCELLED);
    let cancelled = Arc::new(AtomicBool::new(true));
    let budget = ScanBudget::default().with_cancel(cancelled);
    let outcome = with_home(home.path(), || {
        CodexScanner::with_availability(|| true)
            .with_budget(budget)
            .scan()
    });
    assert_eq!(outcome.status(), ScanStatus::Failed);
    assert_eq!(outcome.diagnostics()[0].code, SCAN_CANCELLED);
}

#[test]
fn scanner_cache_hit_records_are_budgeted_before_extend() {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    codex_file(
        home.path(),
        &format!(
            r#"{{"type":"session_meta","payload":{{"id":"s","cwd":{}}}}}
"#,
            serde_json::to_string(&project.to_string_lossy()).unwrap()
        ),
    );
    let first = with_home(home.path(), || {
        CodexScanner::with_availability(|| true).scan()
    });
    assert_eq!(first.status(), ScanStatus::Succeeded);
    let budget = ScanBudget::with_limits(
        32,
        200_000,
        100_000,
        64 * 1024 * 1024,
        512 * 1024 * 1024,
        8 * 1024 * 1024,
        0,
        Duration::from_secs(60),
    );
    let outcome = with_home(home.path(), || {
        CodexScanner::with_availability(|| true)
            .with_budget(budget)
            .scan()
    });
    assert_eq!(outcome.status(), ScanStatus::Failed);
    assert!(outcome.projects().is_empty());
    assert_eq!(outcome.diagnostics()[0].code, SCAN_RESOURCE_LIMIT_EXCEEDED);
}

#[test]
fn scanner_source_file_count_and_oversize_whole_file_fail() {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let record = format!(
        r#"{{"type":"session_meta","payload":{{"id":"s","cwd":{}}}}}
"#,
        serde_json::to_string(&project.to_string_lossy()).unwrap()
    );
    codex_file(home.path(), &record);
    let second = home.path().join(".codex/sessions/2026/08/26/second.jsonl");
    std::fs::write(&second, &record).unwrap();
    let count_budget = ScanBudget::with_limits(
        32,
        200_000,
        1,
        64 * 1024 * 1024,
        512 * 1024 * 1024,
        8 * 1024 * 1024,
        1_000_000,
        Duration::from_secs(60),
    );
    let outcome = with_home(home.path(), || {
        CodexScanner::with_availability(|| true)
            .with_budget(count_budget)
            .scan()
    });
    assert_eq!(outcome.status(), ScanStatus::Failed);
    assert!(outcome.projects().is_empty());

    let state = home
        .path()
        .join(".kimi-code/sessions/worktree/session/state.json");
    std::fs::create_dir_all(state.parent().unwrap()).unwrap();
    std::fs::write(
        &state,
        br#"{"workDir":"C:\\project","updatedAt":"2026-08-26T00:00:00Z"}"#,
    )
    .unwrap();
    let oversize = ScanBudget::with_limits(
        32,
        200_000,
        100_000,
        8,
        512 * 1024 * 1024,
        8 * 1024 * 1024,
        1_000_000,
        Duration::from_secs(60),
    );
    let outcome = with_home(home.path(), || {
        KimiScanner::with_availability(|| true)
            .with_budget(oversize)
            .scan()
    });
    assert_eq!(outcome.status(), ScanStatus::Failed);
    assert!(outcome.projects().is_empty());
    assert_eq!(outcome.diagnostics()[0].code, SCAN_RESOURCE_LIMIT_EXCEEDED);
}

#[test]
fn scanner_jsonl_line_and_total_byte_exact_boundaries_are_allowed() {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let record = format!(
        r#"{{"type":"session_meta","payload":{{"id":"s","cwd":{}}}}}
"#,
        serde_json::to_string(&project.to_string_lossy()).unwrap()
    );
    let physical_bytes = record.len() as u64;
    codex_file(home.path(), &record);

    let mut exact = ScanBudget::default();
    exact.max_line_bytes = record.len();
    exact.max_total_bytes = physical_bytes;
    let outcome = with_home(home.path(), || {
        CodexScanner::with_availability(|| true)
            .with_budget(exact)
            .scan()
    });
    assert_eq!(outcome.status(), ScanStatus::Succeeded);
    assert_eq!(outcome.projects().len(), 1);

    let mut long_line = ScanBudget::default();
    long_line.max_line_bytes = record.len() - 1;
    let outcome = with_home(home.path(), || {
        CodexScanner::with_availability(|| true)
            .with_budget(long_line)
            .scan()
    });
    assert_eq!(outcome.status(), ScanStatus::Failed);
    assert_eq!(outcome.diagnostics()[0].code, SCAN_RESOURCE_LIMIT_EXCEEDED);

    let mut total = ScanBudget::default();
    total.max_line_bytes = record.len();
    total.max_total_bytes = physical_bytes - 1;
    let outcome = with_home(home.path(), || {
        CodexScanner::with_availability(|| true)
            .with_budget(total)
            .scan()
    });
    assert_eq!(outcome.status(), ScanStatus::Failed);
    assert_eq!(outcome.diagnostics()[0].code, SCAN_RESOURCE_LIMIT_EXCEEDED);
}

#[cfg(unix)]
#[test]
fn scanner_recursive_enumeration_skips_symlink_and_root_probe_rejects_it() {
    use std::os::unix::fs::symlink;

    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("source");
    let target = home.path().join("target");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    let sibling = root.join("sibling.jsonl");
    let linked = target.join("linked.jsonl");
    std::fs::write(&sibling, b"sibling\n").unwrap();
    std::fs::write(&linked, b"linked\n").unwrap();
    symlink(&target, root.join("unrelated-link")).unwrap();

    let context = ScanBudget::default().context();
    let files = bounded_recursive_files(&root, &context).unwrap();
    assert_eq!(files, vec![sibling]);
    assert_eq!(
        probe_directory(&root.join("unrelated-link")),
        SourceProbe::Failed
    );
}

#[cfg(unix)]
#[test]
fn scanner_custom_metadata_symlink_falls_back_to_directory() {
    use std::os::unix::fs::symlink;

    let home = tempfile::tempdir().unwrap();
    let data = home.path().join("custom-data");
    let project = data.join("project");
    let external = home.path().join("external-metadata.json");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(&external, br#"{"project_path":"/outside","id":"external"}"#).unwrap();
    symlink(&external, project.join("metadata.json")).unwrap();
    let tool = ToolSource {
        key: "custom-symlink-scanner".to_string(),
        name: "Custom symlink scanner".to_string(),
        cli_command: "custom".to_string(),
        data_directory: data.to_string_lossy().into_owned(),
        scanner_type: "custom".to_string(),
        is_installed: true,
        is_enabled: true,
        ..ToolSource::default()
    };

    let outcome = CustomToolScanner::with_availability(tool, || true).scan();
    assert_eq!(outcome.status(), ScanStatus::Succeeded);
    assert_eq!(outcome.projects().len(), 1);
    assert_eq!(outcome.projects()[0].path, project.to_string_lossy());
}
