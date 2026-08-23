//! End-to-end tests for the R09 `scan` and `config` commands.
//!
//! `scan` tests drive `commands::scan::run_scan` with injected fake scanners so
//! no test reads a real tool data directory, starts an AI CLI, or touches the
//! real `~/.sessionatlas`. `config` tests drive `commands::config::run_config`
//! against an explicit temporary `config.json` under an isolated temporary
//! home. All timestamps are built from `SystemTime` (converted through the
//! model field's `From<SystemTime>` impl), so this crate's tests never depend
//! on chrono being a direct dependency.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use crate::cli::{Cli, Command, ConfigAction, ConfigArgs, ScanArgs};
use crate::commands::config::run_config;
use crate::commands::scan::{build_default_scanners, run_scan};
use crate::{run_with_db, Io};
use sessionatlas_core::model::{Project, ToolSource, ToolUsage};
use sessionatlas_core::scanner::{
    ScanDiagnostic, ScanDiagnosticSeverity, ScanOutcome, ScannedProject, Scanner,
};
use sessionatlas_core::store::SqliteStore;

/// Serializes tests that mutate the process-global `SESSIONATLAS_HOME`.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Unique temporary directory removed on drop. `tempfile` is not a dependency
/// of this crate (its Cargo.toml is frozen for this task), so tests create
/// their own disposable root.
struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!(
            "sessionatlas-cli-r09-{}-{}-{nonce}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// The production layout: `<root>/.sessionatlas/index.db`.
    fn db(&self) -> PathBuf {
        self.0.join(".sessionatlas").join("index.db")
    }

    /// The config file beside the database: `<root>/.sessionatlas/config.json`.
    fn config(&self) -> PathBuf {
        self.0.join(".sessionatlas").join("config.json")
    }

    /// Whether any `.sessionatlas` artifact exists yet.
    fn data_created(&self) -> bool {
        self.0.join(".sessionatlas").exists()
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn abs_path(segments: &[&str]) -> String {
    if cfg!(windows) {
        format!("C:\\{}", segments.join("\\"))
    } else {
        format!("/{}", segments.join("/"))
    }
}

fn now_minus(secs: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(secs)
}

fn usage(key: &str, last_used: SystemTime, count: i32) -> ToolUsage {
    ToolUsage {
        tool_name: key.to_string(),
        tool_key: key.to_string(),
        last_used_at: last_used.into(),
        session_count: count,
        last_session_id: None,
    }
}

fn project(path_text: &str, id: &str, usages: &[ToolUsage]) -> Project {
    let last_accessed = usages.iter().map(|usage| usage.last_used_at).max().unwrap();
    Project {
        id: id.to_string(),
        path: path_text.to_string(),
        last_accessed_at: last_accessed,
        tool_usages: usages.to_vec(),
        ..Project::default()
    }
}

fn scanned_project(path_text: &str, last_accessed: SystemTime, session_id: &str) -> ScannedProject {
    ScannedProject {
        path: path_text.to_string(),
        last_accessed_at: last_accessed.into(),
        session_id: Some(session_id.to_string()),
        git_branch: None,
    }
}

/// Injected fake scanner: returns a canned outcome (or panics) without ever
/// touching a real tool data directory or launching an AI CLI.
struct FakeScanner {
    key: &'static str,
    name: &'static str,
    outcome: ScanOutcome,
    panics: bool,
}

impl FakeScanner {
    fn succeeded(key: &'static str, projects: Vec<ScannedProject>) -> Self {
        Self {
            key,
            name: key,
            outcome: ScanOutcome::succeeded(projects, []),
            panics: false,
        }
    }

    fn succeeded_with_diagnostics(
        key: &'static str,
        projects: Vec<ScannedProject>,
        diagnostics: Vec<ScanDiagnostic>,
    ) -> Self {
        Self {
            key,
            name: key,
            outcome: ScanOutcome::succeeded(projects, diagnostics),
            panics: false,
        }
    }

    fn failed(key: &'static str) -> Self {
        Self {
            key,
            name: key,
            outcome: ScanOutcome::failed([ScanDiagnostic::new(
                key,
                ScanDiagnosticSeverity::Error,
                "source_read_failed",
                "unreadable source",
            )]),
            panics: false,
        }
    }

    fn unavailable(key: &'static str) -> Self {
        Self {
            key,
            name: key,
            outcome: ScanOutcome::unavailable([ScanDiagnostic::new(
                key,
                ScanDiagnosticSeverity::Info,
                "source_unavailable",
                "no source",
            )]),
            panics: false,
        }
    }

    fn panics(key: &'static str) -> Self {
        Self {
            key,
            name: key,
            outcome: ScanOutcome::unavailable([]),
            panics: true,
        }
    }
}

impl Scanner for FakeScanner {
    fn tool_key(&self) -> &str {
        self.key
    }

    fn tool_name(&self) -> &str {
        self.name
    }

    fn is_available(&self) -> bool {
        true
    }

    fn scan(&self) -> ScanOutcome {
        if self.panics {
            panic!("injected fake scanner panic");
        }
        self.outcome.clone()
    }
}

fn boxed(scanner: FakeScanner) -> Box<dyn Scanner> {
    Box::new(scanner)
}

fn scan_args(tool: Option<&str>) -> ScanArgs {
    ScanArgs {
        tool: tool.map(str::to_string),
    }
}

/// Runs `scan` with fake scanners and empty initial diagnostics.
fn run_scan_with(
    dir: &TestDir,
    args: ScanArgs,
    scanners: &[Box<dyn Scanner>],
) -> (i32, String, String) {
    run_scan_full(dir, args, scanners, &[])
}

fn run_scan_full(
    dir: &TestDir,
    args: ScanArgs,
    scanners: &[Box<dyn Scanner>],
    initial: &[ScanDiagnostic],
) -> (i32, String, String) {
    let mut reader = Cursor::new(Vec::new());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut io = Io {
        stdin: &mut reader,
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = run_scan(&mut io, &dir.db(), &args, scanners, initial);
    (
        code,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

/// Runs `config` against the directory's temporary `config.json`.
fn run_config_with(dir: &TestDir, args: ConfigArgs, input: &str) -> (i32, String, String) {
    let mut reader = Cursor::new(input.as_bytes().to_vec());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut io = Io {
        stdin: &mut reader,
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = run_config(&mut io, &dir.config(), &args);
    (
        code,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

fn config_action(action: ConfigAction) -> ConfigArgs {
    ConfigArgs {
        action: Some(action),
    }
}

fn read_config(dir: &TestDir) -> sessionatlas_core::config::AppConfig {
    sessionatlas_core::config::load(dir.config()).unwrap()
}

fn read_db_projects(dir: &TestDir) -> Vec<Project> {
    let store = SqliteStore::new(dir.db()).unwrap();
    store.list_projects(None, None, 10_000).unwrap()
}

// ---------------------------------------------------------------------------
// scan contract
// ---------------------------------------------------------------------------

#[test]
fn scan_all_successful_tools_writes_snapshot_and_exits_zero() {
    let dir = TestDir::new();
    let scanners: Vec<Box<dyn Scanner>> = vec![boxed(FakeScanner::succeeded(
        "codex",
        vec![scanned_project(
            &abs_path(&["work", "web-frontend"]),
            now_minus(600),
            "codex-session-1",
        )],
    ))];
    let (code, stdout, stderr) = run_scan_with(&dir, scan_args(None), &scanners);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("将扫描 1 个工具"), "{stdout}");
    assert!(stdout.contains("原始扫描结果: 1 条"), "{stdout}");
    assert!(stdout.contains("去重合并后: 1 个项目"), "{stdout}");
    assert!(stdout.contains("索引已原子更新到本地数据库。"), "{stdout}");
    let projects = read_db_projects(&dir);
    assert_eq!(projects.len(), 1);
    assert!(projects[0].path.ends_with("web-frontend"), "{:?}", projects);
}

#[test]
fn scan_unspecified_tool_runs_every_provided_scanner() {
    let dir = TestDir::new();
    let scanners: Vec<Box<dyn Scanner>> = vec![
        boxed(FakeScanner::succeeded(
            "codex",
            vec![scanned_project(
                &abs_path(&["work", "a"]),
                now_minus(600),
                "s1",
            )],
        )),
        boxed(FakeScanner::succeeded(
            "claude",
            vec![scanned_project(
                &abs_path(&["work", "b"]),
                now_minus(300),
                "s2",
            )],
        )),
    ];
    let (code, stdout, stderr) = run_scan_with(&dir, scan_args(None), &scanners);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("将扫描 2 个工具"), "{stdout}");
    assert_eq!(read_db_projects(&dir).len(), 2);
}

#[test]
fn scan_tool_filter_is_case_insensitive() {
    let dir = TestDir::new();
    let scanners: Vec<Box<dyn Scanner>> = vec![boxed(FakeScanner::succeeded(
        "codex",
        vec![scanned_project(
            &abs_path(&["work", "a"]),
            now_minus(60),
            "s1",
        )],
    ))];
    let (code, stdout, stderr) = run_scan_with(&dir, scan_args(Some("CODEX")), &scanners);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("将扫描 1 个工具"), "{stdout}");
    assert_eq!(read_db_projects(&dir).len(), 1);
}

#[test]
fn scan_unknown_tool_exits_nonzero_before_touching_database() {
    let dir = TestDir::new();
    let scanners: Vec<Box<dyn Scanner>> = vec![boxed(FakeScanner::succeeded(
        "codex",
        vec![scanned_project(
            &abs_path(&["work", "a"]),
            now_minus(60),
            "s1",
        )],
    ))];
    let (code, stdout, stderr) = run_scan_with(&dir, scan_args(Some("nope")), &scanners);
    assert_eq!(code, 1);
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("未检测到可扫描的工具：nope"), "{stderr}");
    assert!(
        !dir.data_created(),
        "unknown tool must not create the data directory"
    );
}

#[test]
fn scan_successful_empty_scan_cleans_the_tool_old_snapshot() {
    let dir = TestDir::new();
    {
        let mut store = SqliteStore::new(dir.db()).unwrap();
        let web = abs_path(&["work", "web-frontend"]);
        store
            .replace_tool_snapshots(
                &[project(
                    &web,
                    "web-id",
                    &[usage("codex", now_minus(3600), 3)],
                )],
                &["codex"],
            )
            .unwrap();
    }
    assert_eq!(read_db_projects(&dir).len(), 1);

    let scanners: Vec<Box<dyn Scanner>> = vec![boxed(FakeScanner::succeeded("codex", vec![]))];
    let (code, stdout, stderr) = run_scan_with(&dir, scan_args(None), &scanners);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("原始扫描结果: 0 条"), "{stdout}");
    assert!(stdout.contains("去重合并后: 0 个项目"), "{stdout}");
    assert!(
        read_db_projects(&dir).is_empty(),
        "a successful empty scan must clear the tool's old snapshot"
    );
}

#[test]
fn scan_partial_failure_writes_successes_and_preserves_failed_snapshots() {
    let dir = TestDir::new();
    {
        let mut store = SqliteStore::new(dir.db()).unwrap();
        let old = abs_path(&["work", "old-project"]);
        store
            .replace_tool_snapshots(
                &[project(
                    &old,
                    "old-id",
                    &[usage("claude", now_minus(7200), 2)],
                )],
                &["claude"],
            )
            .unwrap();
    }

    let scanners: Vec<Box<dyn Scanner>> = vec![
        boxed(FakeScanner::succeeded(
            "codex",
            vec![scanned_project(
                &abs_path(&["work", "new"]),
                now_minus(60),
                "s1",
            )],
        )),
        boxed(FakeScanner::failed("claude")),
    ];
    let (code, _stdout, stderr) = run_scan_with(&dir, scan_args(None), &scanners);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("1 个工具保留了上一份索引。"), "{stderr}");

    let projects = read_db_projects(&dir);
    let paths: Vec<&str> = projects
        .iter()
        .map(|project| project.path.as_str())
        .collect();
    assert!(paths.iter().any(|path| path.ends_with("new")), "{paths:?}");
    assert!(
        paths.iter().any(|path| path.ends_with("old-project")),
        "failed tool snapshot must be preserved: {paths:?}"
    );
}

#[test]
fn scan_all_fail_never_touches_the_database() {
    let dir = TestDir::new();
    let scanners: Vec<Box<dyn Scanner>> = vec![
        boxed(FakeScanner::failed("codex")),
        boxed(FakeScanner::unavailable("claude")),
    ];
    let (code, stdout, stderr) = run_scan_with(&dir, scan_args(None), &scanners);
    assert_eq!(code, 1);
    assert!(stdout.contains("将扫描 2 个工具"), "{stdout}");
    assert!(
        stderr.contains("没有工具产生可信快照，索引未发生变化。"),
        "{stderr}"
    );
    assert!(
        !dir.data_created(),
        "zero successful tools must not create the index"
    );
}

#[test]
fn scan_zero_successful_tools_does_not_modify_an_existing_database() {
    let dir = TestDir::new();
    {
        let mut store = SqliteStore::new(dir.db()).unwrap();
        let web = abs_path(&["work", "web-frontend"]);
        store
            .replace_tool_snapshots(
                &[project(
                    &web,
                    "web-id",
                    &[usage("codex", now_minus(3600), 3)],
                )],
                &["codex"],
            )
            .unwrap();
    }

    let scanners: Vec<Box<dyn Scanner>> = vec![boxed(FakeScanner::unavailable("codex"))];
    let (code, _, stderr) = run_scan_with(&dir, scan_args(None), &scanners);
    assert_eq!(code, 1);
    assert!(stderr.contains("没有工具产生可信快照"), "{stderr}");
    let projects = read_db_projects(&dir);
    assert_eq!(
        projects.len(),
        1,
        "existing snapshot must be preserved untouched"
    );
    assert!(projects[0].path.ends_with("web-frontend"));
}

#[test]
fn scan_scanner_panic_becomes_failure_and_preserves_old_data() {
    let dir = TestDir::new();
    {
        let mut store = SqliteStore::new(dir.db()).unwrap();
        let web = abs_path(&["work", "web-frontend"]);
        store
            .replace_tool_snapshots(
                &[project(
                    &web,
                    "web-id",
                    &[usage("codex", now_minus(3600), 3)],
                )],
                &["codex"],
            )
            .unwrap();
    }

    let scanners: Vec<Box<dyn Scanner>> = vec![boxed(FakeScanner::panics("codex"))];
    let (code, _, stderr) = run_scan_with(&dir, scan_args(Some("codex")), &scanners);
    assert_eq!(code, 1);
    assert!(stderr.contains("unexpected_scanner_failure"), "{stderr}");
    let projects = read_db_projects(&dir);
    assert_eq!(projects.len(), 1, "panic must preserve the old snapshot");
}

#[test]
fn scan_scanner_panic_does_not_block_other_successful_tools() {
    let dir = TestDir::new();
    let scanners: Vec<Box<dyn Scanner>> = vec![
        boxed(FakeScanner::panics("codex")),
        boxed(FakeScanner::succeeded(
            "claude",
            vec![scanned_project(
                &abs_path(&["work", "ok"]),
                now_minus(60),
                "s1",
            )],
        )),
    ];
    let (code, stdout, stderr) = run_scan_with(&dir, scan_args(None), &scanners);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("unexpected_scanner_failure"), "{stderr}");
    assert!(stdout.contains("去重合并后: 1 个项目"), "{stdout}");
    let projects = read_db_projects(&dir);
    assert_eq!(projects.len(), 1);
    assert!(projects[0].path.ends_with("ok"), "{:?}", projects);
}

#[test]
fn scan_diagnostics_and_output_are_terminal_sanitized() {
    let dir = TestDir::new();
    let scanners: Vec<Box<dyn Scanner>> = vec![boxed(FakeScanner::succeeded_with_diagnostics(
        "codex",
        vec![scanned_project(
            &abs_path(&["work", "a"]),
            now_minus(60),
            "s1",
        )],
        vec![ScanDiagnostic::new(
            "codex",
            ScanDiagnosticSeverity::Warning,
            "malformed_session_record",
            "bad \u{001B}[31mline\u{0007}\u{001B}[K in a\u{001B}]0;hostile\u{0007} file",
        )],
    ))];
    let (code, _, stderr) = run_scan_with(&dir, scan_args(None), &scanners);
    assert_eq!(code, 0);
    assert!(stderr.contains("bad line in a file"), "{stderr}");
    assert!(
        !stderr.contains('\u{001B}'),
        "diagnostics leaked ESC: {stderr}"
    );
    assert!(
        !stderr.contains('\u{0007}'),
        "diagnostics leaked BEL: {stderr}"
    );
}

#[test]
fn scan_unreadable_config_keeps_builtins_and_reports_config_read_failed() {
    let dir = TestDir::new();
    std::fs::create_dir_all(dir.config().parent().unwrap()).unwrap();
    std::fs::write(dir.config(), "{ not valid json").unwrap();
    let (scanners, diagnostics) = build_default_scanners(&dir.config());
    assert_eq!(scanners.len(), 6, "built-ins remain available");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "config_read_failed");
    assert_eq!(diagnostics[0].severity, ScanDiagnosticSeverity::Warning);
}

// ---------------------------------------------------------------------------
// default scanner set (built-ins + custom tools)
// ---------------------------------------------------------------------------

#[test]
fn default_scanner_set_instantiates_the_six_built_ins_in_canonical_order() {
    let dir = TestDir::new();
    let (scanners, diagnostics) = build_default_scanners(&dir.config());
    let keys: Vec<&str> = scanners.iter().map(|scanner| scanner.tool_key()).collect();
    assert_eq!(
        keys,
        ["claude", "kimi", "codex", "opencode", "aider", "pi"],
        "built-ins must be instantiated in canonical registration order"
    );
    assert!(diagnostics.is_empty());
}

#[test]
fn default_scanner_set_loads_enabled_non_conflicting_custom_tools_only() {
    let dir = TestDir::new();
    std::fs::create_dir_all(dir.config().parent().unwrap()).unwrap();
    std::fs::write(
        dir.config(),
        r#"{
            "CustomTools": [
                { "Key": "my-agent", "Name": "My Agent", "CliCommand": "myagent", "DataDirectory": "/tmp/agent" },
                { "Key": "off-agent", "Name": "Off", "CliCommand": "off", "DataDirectory": "/tmp/off", "IsEnabled": false },
                { "Key": "CODEX", "Name": "Fake Codex", "CliCommand": "fake", "DataDirectory": "/tmp/fake" }
            ]
        }"#,
    )
    .unwrap();

    let (scanners, _) = build_default_scanners(&dir.config());
    let keys: Vec<&str> = scanners.iter().map(|scanner| scanner.tool_key()).collect();
    assert!(
        keys.contains(&"my-agent"),
        "enabled non-conflicting custom tool loaded: {keys:?}"
    );
    assert!(
        !keys.contains(&"off-agent"),
        "disabled custom tool skipped: {keys:?}"
    );
    let codex_count = keys
        .iter()
        .filter(|key| key.eq_ignore_ascii_case("codex"))
        .count();
    assert_eq!(codex_count, 1, "conflicting custom tool skipped: {keys:?}");
    assert_eq!(scanners.len(), 7, "6 built-ins + my-agent");
}

// ---------------------------------------------------------------------------
// config contract
// ---------------------------------------------------------------------------

#[test]
fn config_show_missing_file_renders_defaults_without_creating_it() {
    let dir = TestDir::new();
    let (code, stdout, stderr) = run_config_with(&dir, config_action(ConfigAction::Show), "");
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("当前配置:"), "{stdout}");
    assert!(stdout.contains("默认终端: auto"), "{stdout}");
    assert!(stdout.contains("自定义工具数量: 0"), "{stdout}");
    assert!(!dir.config().exists(), "show must not create config.json");
    assert!(
        !dir.data_created(),
        "show must not create the data directory"
    );
}

#[test]
fn config_show_renders_existing_values_and_tools() {
    let dir = TestDir::new();
    std::fs::create_dir_all(dir.config().parent().unwrap()).unwrap();
    std::fs::write(
        dir.config(),
        r#"{
            "DefaultTerminal": "windows-terminal",
            "CustomTools": [ { "Key": "my-agent", "Name": "My Agent", "CliCommand": "myagent", "DataDirectory": "C:\\data\\agent" } ]
        }"#,
    )
    .unwrap();
    let (code, stdout, stderr) = run_config_with(&dir, config_action(ConfigAction::Show), "");
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("默认终端: windows-terminal"), "{stdout}");
    assert!(stdout.contains("自定义工具数量: 1"), "{stdout}");
    assert!(stdout.contains("My Agent (my-agent):"), "{stdout}");
}

#[test]
fn config_show_with_invalid_json_fails_without_overwriting() {
    let dir = TestDir::new();
    std::fs::create_dir_all(dir.config().parent().unwrap()).unwrap();
    std::fs::write(dir.config(), "{ not valid json").unwrap();
    let (code, stdout, stderr) = run_config_with(&dir, config_action(ConfigAction::Show), "");
    assert_eq!(code, 1);
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("配置文件无法读取或格式无效"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(dir.config()).unwrap(),
        "{ not valid json"
    );
}

#[test]
fn config_add_tool_persists_and_keeps_existing_tools() {
    let dir = TestDir::new();
    std::fs::create_dir_all(dir.config().parent().unwrap()).unwrap();
    std::fs::write(
        dir.config(),
        r#"{
            "CustomTools": [ { "Key": "first", "Name": "First", "CliCommand": "first", "DataDirectory": "C:\\data\\first" } ]
        }"#,
    )
    .unwrap();

    let (code, stdout, stderr) = run_config_with(
        &dir,
        config_action(ConfigAction::AddTool),
        "My Agent\nmy-agent\nmyagent --flag\nC:\\data\\agent\n",
    );
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("自定义工具已添加并保存"), "{stdout}");

    let config = read_config(&dir);
    assert_eq!(
        config.custom_tools.len(),
        2,
        "existing tool must be preserved"
    );
    assert_eq!(config.custom_tools[0].key, "first");
    let added = &config.custom_tools[1];
    assert_eq!(added.key, "my-agent");
    assert_eq!(added.name, "My Agent");
    assert_eq!(added.cli_command, "myagent --flag");
    assert_eq!(added.data_directory, "C:\\data\\agent");
    assert!(added.is_enabled);
}

#[test]
fn config_add_tool_tilde_data_directory_resolves_against_sessionatlas_home() {
    let dir = TestDir::new();
    let _guard = ENV_LOCK.lock().unwrap();
    let previous = std::env::var_os("SESSIONATLAS_HOME");
    std::env::set_var("SESSIONATLAS_HOME", &dir.0);
    let (code, _, stderr) = run_config_with(
        &dir,
        config_action(ConfigAction::AddTool),
        "Agent\nagent\nagentcli\n~/data/agent\n",
    );
    restore_sessionatlas_home(previous);
    assert_eq!(code, 0, "{stderr}");
    let config = read_config(&dir);
    assert_eq!(
        config.custom_tools[0].data_directory,
        dir.0.join("data/agent").to_string_lossy()
    );
}

#[test]
fn config_add_tool_rejects_reserved_and_duplicate_keys() {
    let dir = TestDir::new();
    std::fs::create_dir_all(dir.config().parent().unwrap()).unwrap();
    std::fs::write(
        dir.config(),
        r#"{
            "CustomTools": [ { "Key": "my-agent", "Name": "My Agent", "CliCommand": "myagent", "DataDirectory": "C:\\data\\agent" } ]
        }"#,
    )
    .unwrap();

    let (code, _, stderr) = run_config_with(
        &dir,
        config_action(ConfigAction::AddTool),
        "Fake\ncodex\nfakecli\nC:\\data\n",
    );
    assert_eq!(code, 1);
    assert!(stderr.contains("已存在或属于内置工具"), "{stderr}");
    assert_eq!(read_config(&dir).custom_tools.len(), 1);

    let (code, _, stderr) = run_config_with(
        &dir,
        config_action(ConfigAction::AddTool),
        "Fake\nMY-AGENT\nfakecli\nC:\\data\n",
    );
    assert_eq!(code, 1);
    assert!(stderr.contains("已存在或属于内置工具"), "{stderr}");
    assert_eq!(
        read_config(&dir).custom_tools.len(),
        1,
        "duplicate must not be added"
    );
}

#[test]
fn config_add_tool_rejects_unsafe_names_keys_and_commands_without_writing() {
    let dir = TestDir::new();
    std::fs::create_dir_all(dir.config().parent().unwrap()).unwrap();

    let (code, _, stderr) = run_config_with(
        &dir,
        config_action(ConfigAction::AddTool),
        "bad\u{0007}name\nagent\nagentcli\nC:\\data\n",
    );
    assert_eq!(code, 1);
    assert!(stderr.contains("显示名称包含不支持的字符"), "{stderr}");

    let (code, _, stderr) = run_config_with(
        &dir,
        config_action(ConfigAction::AddTool),
        "Good\nbad;key\nagentcli\nC:\\data\n",
    );
    assert_eq!(code, 1);
    assert!(stderr.contains("工具标识包含不支持的字符"), "{stderr}");

    for bad_cli in ["agent&evil", "bash", "cmd.exe", "run.bat", "-flag"] {
        let (code, _, stderr) = run_config_with(
            &dir,
            config_action(ConfigAction::AddTool),
            &format!("Good\nagent\n{bad_cli}\nC:\\data\n"),
        );
        assert_eq!(code, 1, "CLI {bad_cli:?} must be rejected");
        assert!(stderr.contains("CLI 命令"), "{stderr}");
    }

    assert!(
        !dir.config().exists(),
        "rejected input must not create config.json"
    );
}

#[test]
fn config_add_tool_does_not_overwrite_invalid_json() {
    let dir = TestDir::new();
    std::fs::create_dir_all(dir.config().parent().unwrap()).unwrap();
    std::fs::write(dir.config(), "{ invalid").unwrap();
    let (code, _, stderr) = run_config_with(
        &dir,
        config_action(ConfigAction::AddTool),
        "Good\nagent\nagentcli\nC:\\data\n",
    );
    assert_eq!(code, 1);
    assert!(
        stderr.contains("配置格式无效，未写入任何修改。"),
        "{stderr}"
    );
    assert_eq!(std::fs::read_to_string(dir.config()).unwrap(), "{ invalid");
}

#[test]
fn config_add_tool_uses_atomic_update_so_concurrent_writers_are_not_lost() {
    let dir = TestDir::new();
    sessionatlas_core::config::update(dir.config(), None, |config| {
        config.custom_tools.push(ToolSource {
            key: "writer-a".to_string(),
            name: "Writer A".to_string(),
            cli_command: "writer-a".to_string(),
            data_directory: "C:\\data\\a".to_string(),
            is_enabled: true,
            ..ToolSource::default()
        });
    })
    .unwrap();

    let (code, _, stderr) = run_config_with(
        &dir,
        config_action(ConfigAction::AddTool),
        "Writer B\nwriter-b\nwriter-b\nC:\\data\n",
    );
    assert_eq!(code, 0, "{stderr}");
    let config = read_config(&dir);
    assert_eq!(
        config.custom_tools.len(),
        2,
        "no writer's change may be lost"
    );
    assert!(config
        .custom_tools
        .iter()
        .any(|tool| tool.key == "writer-a"));
    assert!(config
        .custom_tools
        .iter()
        .any(|tool| tool.key == "writer-b"));
}

#[test]
fn config_set_default_terminal_accepts_each_supported_choice() {
    for (index, expected) in [
        (1, "auto"),
        (2, "windows-terminal"),
        (3, "cmd"),
        (4, "iterm2"),
        (5, "terminal"),
        (6, "gnome-terminal"),
        (7, "konsole"),
    ] {
        let dir = TestDir::new();
        let (code, stdout, stderr) = run_config_with(
            &dir,
            config_action(ConfigAction::SetDefaultTerminal),
            &format!("{index}\n"),
        );
        assert_eq!(code, 0, "choice {index}: {stderr}");
        assert!(
            stdout.contains(&format!("默认终端已设置为: {expected}")),
            "{stdout}"
        );
        assert_eq!(read_config(&dir).default_terminal, expected);
    }
}

#[test]
fn config_set_default_terminal_invalid_choice_reprompts_and_rejects_unknown_values() {
    let dir = TestDir::new();
    let (code, stdout, _) = run_config_with(
        &dir,
        config_action(ConfigAction::SetDefaultTerminal),
        "8\n999\n2\n",
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("无效选择，请重新输入。"), "{stdout}");
    assert_eq!(read_config(&dir).default_terminal, "windows-terminal");
}

#[test]
fn config_set_default_terminal_cancel_leaves_config_untouched() {
    let dir = TestDir::new();
    let (code, _, _) =
        run_config_with(&dir, config_action(ConfigAction::SetDefaultTerminal), "0\n");
    assert_eq!(code, 0);
    assert!(
        !dir.config().exists(),
        "cancellation must not create config.json"
    );
}

#[test]
fn config_default_action_is_show_via_run_with_db_dispatch() {
    let dir = TestDir::new();
    let (code, stdout, stderr) = {
        let mut reader = Cursor::new(Vec::new());
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut io = Io {
            stdin: &mut reader,
            stdout: &mut stdout,
            stderr: &mut stderr,
        };
        let code = run_with_db(
            Cli {
                command: Some(Command::Config(ConfigArgs { action: None })),
            },
            &mut io,
            &dir.db(),
        );
        (
            code,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    };
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("默认终端: auto"), "{stdout}");
    assert!(!dir.config().exists());
}

fn restore_sessionatlas_home(previous: Option<std::ffi::OsString>) {
    match previous {
        Some(value) => std::env::set_var("SESSIONATLAS_HOME", value),
        None => std::env::remove_var("SESSIONATLAS_HOME"),
    }
}
