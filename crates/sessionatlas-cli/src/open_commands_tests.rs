//! End-to-end tests for the R10 `open` CLI command.
//!
//! Every test seeds an isolated temporary database (and optional config file)
//! and drives `commands::open::run_open` with a recording process runner and a
//! fake PATH resolver, so no test reads or writes the real `~/.sessionatlas`,
//! starts a terminal, or launches an AI CLI, `where`, or `which`. Timestamps
//! are built from `SystemTime` and `Session::default()` so this crate's tests
//! never depend on chrono or uuid being direct dependencies.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use crate::cli::{Cli, Command, ListArgs, OpenArgs, RecentArgs, SearchArgs};
use crate::commands::open::{run_open, OpenEnvironment};
use crate::commands::recent::run_recent;
use crate::{run_with_open_environment, Io};
use sessionatlas_core::config::AppConfig;
use sessionatlas_core::model::{Project, Session, ToolSource, ToolUsage};
use sessionatlas_core::path;
use sessionatlas_core::process::{ProgramResolver, RecordingProcessRunner};
use sessionatlas_core::store::SqliteStore;

/// Unique temporary directory removed on drop. `tempfile` is not a dependency
/// of this crate (its Cargo.toml is frozen for this task), so tests create
/// their own disposable root.
struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!(
            "sessionatlas-open-test-{}-{}-{nonce}",
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

    fn db(&self) -> PathBuf {
        self.0.join(".sessionatlas").join("index.db")
    }

    fn config(&self) -> PathBuf {
        self.0.join(".sessionatlas").join("config.json")
    }

    /// A real, existing subdirectory under the temp root.
    fn existing_project(&self, name: &str) -> String {
        let dir = self.0.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        path::normalize_native(&dir.to_string_lossy()).unwrap()
    }

    /// Like [`TestDir::existing_project`] but usable in bulk; also used by
    /// `seed` so DB projects point at real directories.
    fn project_path(&self, name: &str) -> String {
        self.existing_project(name)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn now_minus(secs: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(secs)
}

fn usage(key: &str, last_used: SystemTime, count: i32, last_session_id: Option<&str>) -> ToolUsage {
    ToolUsage {
        tool_name: key.to_string(),
        tool_key: key.to_string(),
        last_used_at: last_used.into(),
        session_count: count,
        last_session_id: last_session_id.map(str::to_string),
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

fn session(
    id: &str,
    path_text: &str,
    tool_key: &str,
    tool_name: &str,
    started: SystemTime,
    session_id_from_tool: Option<&str>,
) -> Session {
    Session {
        id: id.to_string(),
        project_path: path_text.to_string(),
        tool_key: tool_key.to_string(),
        tool_name: tool_name.to_string(),
        started_at: started.into(),
        ended_at: None,
        session_id_from_tool: session_id_from_tool.map(str::to_string),
    }
}

/// Seeds two real projects (api-server: codex/claude, web-frontend: claude) and
/// two sessions (web/claude recent, api/codex older). Project paths exist on
/// disk because `open` requires the resolved directory to exist.
fn seed(dir: &TestDir) {
    let mut store = SqliteStore::new(dir.db()).unwrap();
    let api = dir.project_path("api-server");
    let web = dir.project_path("web-frontend");
    store
        .replace_tool_snapshots(
            &[
                project(
                    &api,
                    "api-id",
                    &[
                        usage("codex", now_minus(3600), 3, Some("codex-last")),
                        usage("claude", now_minus(7200), 1, None),
                    ],
                ),
                project(
                    &web,
                    "web-id",
                    &[usage("claude", now_minus(1800), 2, Some("claude-last"))],
                ),
            ],
            &["codex", "claude"],
        )
        .unwrap();
    store
        .record_session(&session(
            "s-new",
            &web,
            "claude",
            "Claude",
            now_minus(600),
            Some("claude-recent"),
        ))
        .unwrap();
    store
        .record_session(&session(
            "s-old",
            &api,
            "codex",
            "Codex",
            now_minus(3600),
            Some("codex-old"),
        ))
        .unwrap();
}

/// Resolver that pretends only the listed programs are on PATH.
struct FakeResolver {
    present: Vec<&'static str>,
}

impl ProgramResolver for FakeResolver {
    fn is_on_path(&self, program: &str) -> bool {
        self.present.contains(&program)
    }

    fn resolve(&self, program: &str) -> Option<PathBuf> {
        self.is_on_path(program).then(|| PathBuf::from(program))
    }
}

fn no_wt(_: &Path) -> bool {
    false
}

fn resolver_with_tools() -> FakeResolver {
    FakeResolver {
        present: vec!["claude", "codex", "gnome-terminal", "xterm"],
    }
}

/// Runs `open` against the directory's database/config with scripted stdin.
fn run_open_with(
    dir: &TestDir,
    args: &OpenArgs,
    input: &str,
    env: &OpenEnvironment<'_>,
) -> (i32, String, String) {
    let mut reader = Cursor::new(input.as_bytes().to_vec());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut io = Io {
        stdin: &mut reader,
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = run_open(&mut io, &dir.db(), &dir.config(), args, env);
    (
        code,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

fn run_recent_with(
    dir: &TestDir,
    args: &RecentArgs,
    input: &str,
    env: &OpenEnvironment<'_>,
) -> (i32, String, String) {
    let mut reader = Cursor::new(input.as_bytes().to_vec());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut io = Io {
        stdin: &mut reader,
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = run_recent(&mut io, &dir.db(), &dir.config(), args, env);
    (
        code,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

fn run_cli_with(
    dir: &TestDir,
    cli: Cli,
    input: &str,
    env: &OpenEnvironment<'_>,
) -> (i32, String, String) {
    let mut reader = Cursor::new(input.as_bytes().to_vec());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut io = Io {
        stdin: &mut reader,
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = run_with_open_environment(cli, &mut io, &dir.db(), &dir.config(), env);
    (
        code,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

fn open_env<'a>(
    runner: &'a RecordingProcessRunner,
    resolver: &'a FakeResolver,
    resolved_session_id: Option<String>,
) -> OpenEnvironment<'a> {
    OpenEnvironment {
        runner,
        resolver,
        wt_probe: no_wt,
        resolved_session_id,
    }
}

fn recent_args() -> OpenArgs {
    OpenArgs {
        project_path: None,
        tool: None,
        interactive: false,
        recent: true,
    }
}

fn path_args(input: &str, tool: Option<&str>) -> OpenArgs {
    OpenArgs {
        project_path: Some(input.to_string()),
        tool: tool.map(str::to_string),
        interactive: false,
        recent: false,
    }
}

#[test]
fn open_recent_launches_recent_project_and_resumes_recent_session_id() {
    let dir = TestDir::new();
    seed(&dir);
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let (code, stdout, stderr) = run_open_with(&dir, &recent_args(), "", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("正在启动 claude"), "{stdout}");
    assert!(stdout.contains("尝试恢复会话 claude-recent"), "{stdout}");

    assert_eq!(runner.start_count(), 1);
    let started = runner.started();
    let web = dir.project_path("web-frontend");
    assert_eq!(started[0].working_directory, web);

    let store = SqliteStore::new(dir.db()).unwrap();
    let sessions = store.get_recent_sessions(1).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].tool_key, "claude");
    assert_eq!(sessions[0].project_path, web);
    assert_eq!(
        sessions[0].session_id_from_tool.as_deref(),
        Some("claude-recent")
    );
}

#[test]
fn recent_open_launches_the_selected_session_through_the_injected_runner() {
    let dir = TestDir::new();
    seed(&dir);
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let args = RecentArgs {
        count: 10,
        open: true,
    };
    let (code, stdout, stderr) = run_recent_with(&dir, &args, "2\n", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("选择会话恢复:"), "{stdout}");
    assert!(stdout.contains("正在启动 codex"), "{stdout}");
    assert!(stdout.contains("尝试恢复会话 codex-old"), "{stdout}");
    assert_eq!(runner.start_count(), 1);
    let started = runner.started();
    assert_eq!(started[0].working_directory, dir.project_path("api-server"));
    let launched_arguments = started[0].arguments.join(" ");
    assert!(launched_arguments.contains("codex-old"));
    assert!(launched_arguments.contains("resume"));
    assert!(!launched_arguments.contains("--resume"));

    let store = SqliteStore::new(dir.db()).unwrap();
    let latest = store.get_recent_sessions(1).unwrap().remove(0);
    assert_eq!(latest.tool_key, "codex");
    assert_eq!(latest.session_id_from_tool.as_deref(), Some("codex-old"));
}

#[test]
fn open_recent_with_a_different_explicit_tool_does_not_reuse_foreign_session_id() {
    let dir = TestDir::new();
    seed(&dir);
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);
    let args = OpenArgs {
        project_path: None,
        tool: Some("codex".to_string()),
        interactive: false,
        recent: true,
    };

    let (code, stdout, stderr) = run_open_with(&dir, &args, "", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("正在启动 codex"), "{stdout}");
    assert!(!stdout.contains("claude-recent"), "{stdout}");
    assert!(!runner.started()[0]
        .arguments
        .join(" ")
        .contains("claude-recent"));

    let store = SqliteStore::new(dir.db()).unwrap();
    let latest = store.get_recent_sessions(1).unwrap().remove(0);
    assert_eq!(latest.tool_key, "codex");
    assert_eq!(latest.session_id_from_tool, None);
}

#[test]
fn list_interactive_delegates_to_open_with_recording_runner() {
    let dir = TestDir::new();
    seed(&dir);
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);
    let cli = Cli {
        command: Some(Command::List(ListArgs {
            tool: None,
            limit: 50,
            interactive: true,
        })),
    };

    let (code, stdout, stderr) = run_cli_with(&dir, cli, "2\n2\n", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("选择要打开的项目:"), "{stdout}");
    assert!(stdout.contains("正在启动 codex"), "{stdout}");
    assert_eq!(runner.start_count(), 1);
    assert_eq!(
        runner.started()[0].working_directory,
        dir.project_path("api-server")
    );
}

#[test]
fn search_interactive_delegates_to_open_with_recording_runner() {
    let dir = TestDir::new();
    seed(&dir);
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);
    let cli = Cli {
        command: Some(Command::Search(SearchArgs {
            query: "api".to_string(),
            interactive: true,
        })),
    };

    let (code, stdout, stderr) = run_cli_with(&dir, cli, "1\n2\n", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("找到 1 个匹配项目"), "{stdout}");
    assert!(stdout.contains("正在启动 codex"), "{stdout}");
    assert_eq!(runner.start_count(), 1);
    assert_eq!(
        runner.started()[0].working_directory,
        dir.project_path("api-server")
    );
}

#[test]
fn no_arguments_default_interactive_list_delegates_to_open() {
    let dir = TestDir::new();
    seed(&dir);
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let (code, stdout, stderr) = run_cli_with(&dir, Cli { command: None }, "1\n1\n", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("选择要打开的项目:"), "{stdout}");
    assert!(stdout.contains("正在启动 claude"), "{stdout}");
    assert_eq!(runner.start_count(), 1);
    assert_eq!(
        runner.started()[0].working_directory,
        dir.project_path("web-frontend")
    );
}

#[test]
fn open_existing_directory_launches_with_project_last_session_id() {
    let dir = TestDir::new();
    let project_path = dir.existing_project("workdir");
    {
        let mut store = SqliteStore::new(dir.db()).unwrap();
        store
            .replace_tool_snapshots(
                &[project(
                    &project_path,
                    "p-id",
                    &[usage("codex", now_minus(120), 4, Some("codex-last"))],
                )],
                &["codex"],
            )
            .unwrap();
    }
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let (code, stdout, stderr) =
        run_open_with(&dir, &path_args(&project_path, Some("codex")), "", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("正在启动 codex"), "{stdout}");
    assert!(stdout.contains("尝试恢复会话 codex-last"), "{stdout}");

    assert_eq!(runner.start_count(), 1);
    assert_eq!(runner.started()[0].working_directory, project_path);
    let launched_arguments = runner.started()[0].arguments.join(" ");
    assert!(launched_arguments.contains("codex-last"));
    assert!(launched_arguments.contains("resume"));
    assert!(!launched_arguments.contains("--resume"));

    let store = SqliteStore::new(dir.db()).unwrap();
    let sessions = store.get_recent_sessions(1).unwrap();
    assert_eq!(sessions[0].tool_key, "codex");
    assert_eq!(sessions[0].project_path, project_path);
    assert_eq!(
        sessions[0].session_id_from_tool.as_deref(),
        Some("codex-last")
    );
}

#[test]
fn open_fuzzy_single_match_launches_project() {
    let dir = TestDir::new();
    seed(&dir);
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let (code, stdout, stderr) = run_open_with(&dir, &path_args("api", Some("codex")), "", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("正在启动 codex"), "{stdout}");
    assert_eq!(runner.start_count(), 1);
    assert_eq!(
        runner.started()[0].working_directory,
        dir.project_path("api-server")
    );
}

#[test]
fn open_fuzzy_multi_match_prompts_and_launches_chosen() {
    let dir = TestDir::new();
    let api_server = dir.project_path("api-server");
    let api_gateway = dir.project_path("api-gateway");
    {
        let mut store = SqliteStore::new(dir.db()).unwrap();
        store
            .replace_tool_snapshots(
                &[
                    project(
                        &api_server,
                        "a1",
                        &[usage("codex", now_minus(120), 1, None)],
                    ),
                    project(
                        &api_gateway,
                        "a2",
                        &[usage("codex", now_minus(60), 1, None)],
                    ),
                ],
                &["codex"],
            )
            .unwrap();
    }
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let (code, stdout, stderr) = run_open_with(&dir, &path_args("api", Some("codex")), "1\n", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("找到多个匹配项目"), "{stdout}");
    assert_eq!(runner.start_count(), 1);
    // Recency order lists api-gateway (newer) first, so choice 1 is api-gateway.
    assert_eq!(runner.started()[0].working_directory, api_gateway);
}

#[test]
fn open_fuzzy_multi_match_cancel_exits_zero_without_launch() {
    let dir = TestDir::new();
    let api_server = dir.project_path("api-server");
    let api_gateway = dir.project_path("api-gateway");
    {
        let mut store = SqliteStore::new(dir.db()).unwrap();
        store
            .replace_tool_snapshots(
                &[
                    project(
                        &api_server,
                        "a1",
                        &[usage("codex", now_minus(120), 1, None)],
                    ),
                    project(
                        &api_gateway,
                        "a2",
                        &[usage("codex", now_minus(60), 1, None)],
                    ),
                ],
                &["codex"],
            )
            .unwrap();
    }
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let (code, _, _) = run_open_with(&dir, &path_args("api", Some("codex")), "0\n", &env);
    assert_eq!(code, 0);
    assert_eq!(runner.start_count(), 0);

    let store = SqliteStore::new(dir.db()).unwrap();
    assert!(
        store.get_recent_sessions(10).unwrap().is_empty(),
        "cancel must not record"
    );
}

#[test]
fn open_fuzzy_no_match_is_an_error() {
    let dir = TestDir::new();
    seed(&dir);
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let (code, _, stderr) = run_open_with(&dir, &path_args("nomatch", Some("codex")), "", &env);
    assert_eq!(code, 1);
    assert!(
        stderr.contains("未找到匹配路径 'nomatch' 的项目"),
        "{stderr}"
    );
    assert_eq!(runner.start_count(), 0);
}

#[test]
fn open_without_arguments_selects_project_and_tool_interactively() {
    let dir = TestDir::new();
    seed(&dir);
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let args = OpenArgs {
        project_path: None,
        tool: None,
        interactive: false,
        recent: false,
    };
    let (code, stdout, stderr) = run_open_with(&dir, &args, "1\n1\n", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("选择项目:"), "{stdout}");
    assert!(
        stdout.contains("选择要在 web-frontend 中使用的 CLI 工具:"),
        "{stdout}"
    );
    assert!(stdout.contains("正在启动 claude"), "{stdout}");
    assert_eq!(runner.start_count(), 1);
    assert_eq!(
        runner.started()[0].working_directory,
        dir.project_path("web-frontend")
    );
}

#[test]
fn open_without_arguments_cancel_exits_zero_without_launch() {
    let dir = TestDir::new();
    seed(&dir);
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let args = OpenArgs {
        project_path: None,
        tool: None,
        interactive: false,
        recent: false,
    };
    let (code, _, _) = run_open_with(&dir, &args, "0\n", &env);
    assert_eq!(code, 0);
    assert_eq!(runner.start_count(), 0);
}

#[test]
fn open_without_arguments_and_no_projects_prints_hint() {
    let dir = TestDir::new();
    {
        let _store = SqliteStore::new(dir.db()).unwrap();
    }
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let args = OpenArgs {
        project_path: None,
        tool: None,
        interactive: false,
        recent: false,
    };
    let (code, stdout, _) = run_open_with(&dir, &args, "", &env);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("暂无项目，请先运行 sessionatlas scan"),
        "{stdout}"
    );
    assert_eq!(runner.start_count(), 0);
}

#[test]
fn open_explicit_tool_with_interactive_flag_prompts_tool_selection() {
    let dir = TestDir::new();
    let project_path = dir.existing_project("workdir");
    {
        let mut store = SqliteStore::new(dir.db()).unwrap();
        store
            .replace_tool_snapshots(
                &[project(
                    &project_path,
                    "p-id",
                    &[
                        usage("codex", now_minus(60), 2, None),
                        usage("kimi", now_minus(120), 1, None),
                    ],
                )],
                &["codex", "kimi"],
            )
            .unwrap();
    }
    let runner = RecordingProcessRunner::new();
    // kimi is not on PATH; codex is. The prompt must offer only codex.
    let resolver = FakeResolver {
        present: vec!["claude", "codex", "gnome-terminal"],
    };
    let env = open_env(&runner, &resolver, None);

    let args = OpenArgs {
        project_path: Some(project_path.clone()),
        tool: Some("codex".to_string()),
        interactive: true,
        recent: false,
    };
    let (code, stdout, stderr) = run_open_with(&dir, &args, "1\n", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("选择要在"), "{stdout}");
    assert!(stdout.contains("正在启动 codex"), "{stdout}");
    assert_eq!(runner.start_count(), 1);
}

#[test]
fn open_no_available_tool_is_an_error() {
    let dir = TestDir::new();
    let project_path = dir.existing_project("workdir");
    {
        let mut store = SqliteStore::new(dir.db()).unwrap();
        store
            .replace_tool_snapshots(
                &[project(
                    &project_path,
                    "p-id",
                    &[usage("kimi", now_minus(60), 1, None)],
                )],
                &["kimi"],
            )
            .unwrap();
    }
    let runner = RecordingProcessRunner::new();
    // kimi is not on PATH.
    let resolver = FakeResolver {
        present: vec!["claude", "codex", "gnome-terminal"],
    };
    let env = open_env(&runner, &resolver, None);

    let (code, _, stderr) = run_open_with(&dir, &path_args(&project_path, None), "", &env);
    assert_eq!(code, 1);
    assert!(
        stderr.contains("未在 PATH 中找到任何可用的 AI CLI 工具命令"),
        "{stderr}"
    );
    assert_eq!(runner.start_count(), 0);
}

#[test]
fn open_start_failure_does_not_record_session() {
    let dir = TestDir::new();
    seed(&dir);
    let runner = RecordingProcessRunner::new();
    runner.fail_starts("terminal refused");
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let (code, _, stderr) = run_open_with(&dir, &recent_args(), "", &env);
    assert_eq!(code, 1);
    assert!(stderr.contains("启动失败"), "{stderr}");
    assert_eq!(runner.start_count(), 1, "the launch was attempted once");

    let store = SqliteStore::new(dir.db()).unwrap();
    let sessions = store.get_recent_sessions(10).unwrap();
    assert_eq!(
        sessions.len(),
        2,
        "no new session may be recorded on failure"
    );
    assert_eq!(sessions[0].id, "s-new");
    assert_eq!(sessions[1].id, "s-old");
}

#[test]
fn open_records_session_after_success() {
    let dir = TestDir::new();
    seed(&dir);
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let (code, _, stderr) = run_open_with(&dir, &recent_args(), "", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(runner.start_count(), 1);

    let store = SqliteStore::new(dir.db()).unwrap();
    let sessions = store.get_recent_sessions(10).unwrap();
    assert_eq!(
        sessions.len(),
        3,
        "success must record exactly one new session"
    );
    assert_eq!(sessions[0].tool_key, "claude");
    assert_eq!(sessions[0].project_path, dir.project_path("web-frontend"));
    assert_eq!(
        sessions[0].session_id_from_tool.as_deref(),
        Some("claude-recent")
    );
}

#[test]
fn open_invalid_stored_session_id_warns_and_starts_new_session() {
    let dir = TestDir::new();
    let project_path = dir.existing_project("workdir");
    {
        let mut store = SqliteStore::new(dir.db()).unwrap();
        store
            .replace_tool_snapshots(
                &[project(
                    &project_path,
                    "p-id",
                    // Space makes this a stored-but-invalid session ID.
                    &[usage("codex", now_minus(60), 3, Some("bad id"))],
                )],
                &["codex"],
            )
            .unwrap();
    }
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let (code, stdout, stderr) =
        run_open_with(&dir, &path_args(&project_path, Some("codex")), "", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stderr.contains("索引中的会话 ID 格式无效；将启动新会话。"),
        "{stderr}"
    );
    assert!(stdout.contains("正在启动 codex"), "{stdout}");
    assert!(!stdout.contains("尝试恢复会话"), "{stdout}");

    let started = runner.started();
    assert_eq!(runner.start_count(), 1);
    assert!(
        !started[0].arguments.iter().any(|arg| arg == "--resume"),
        "invalid stored ID must not reach --resume"
    );

    let store = SqliteStore::new(dir.db()).unwrap();
    let sessions = store.get_recent_sessions(1).unwrap();
    assert_eq!(sessions[0].tool_key, "codex");
    assert_eq!(sessions[0].session_id_from_tool, None);
}

#[test]
fn open_unknown_tool_key_is_an_error() {
    let dir = TestDir::new();
    let project_path = dir.existing_project("workdir");
    {
        let _store = SqliteStore::new(dir.db()).unwrap();
    }
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let (code, _, stderr) = run_open_with(&dir, &path_args(&project_path, Some("ghost")), "", &env);
    assert_eq!(code, 1);
    assert!(stderr.contains("未配置可启动的工具"), "{stderr}");
    assert_eq!(runner.start_count(), 0);

    let store = SqliteStore::new(dir.db()).unwrap();
    assert!(store.get_recent_sessions(10).unwrap().is_empty());
}

#[test]
fn open_option_shaped_tool_key_is_an_error() {
    let dir = TestDir::new();
    let project_path = dir.existing_project("workdir");
    {
        let _store = SqliteStore::new(dir.db()).unwrap();
    }
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let (code, _, stderr) =
        run_open_with(&dir, &path_args(&project_path, Some("-codex")), "", &env);
    assert_eq!(code, 1);
    assert!(stderr.contains("工具标识包含不支持的字符"), "{stderr}");
    assert_eq!(runner.start_count(), 0);
}

#[test]
fn open_recent_without_sessions_is_an_error() {
    let dir = TestDir::new();
    {
        let _store = SqliteStore::new(dir.db()).unwrap();
    }
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let (code, _, stderr) = run_open_with(&dir, &recent_args(), "", &env);
    assert_eq!(code, 1);
    assert!(stderr.contains("没有最近会话记录"), "{stderr}");
    assert_eq!(runner.start_count(), 0);
}

#[test]
fn open_explicit_resolved_session_id_wins_over_stored_values() {
    let dir = TestDir::new();
    let project_path = dir.existing_project("workdir");
    {
        let mut store = SqliteStore::new(dir.db()).unwrap();
        store
            .replace_tool_snapshots(
                &[project(
                    &project_path,
                    "p-id",
                    &[usage("codex", now_minus(60), 3, Some("codex-last"))],
                )],
                &["codex"],
            )
            .unwrap();
    }
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, Some("explicit-id".to_string()));

    let (code, stdout, stderr) =
        run_open_with(&dir, &path_args(&project_path, Some("codex")), "", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("尝试恢复会话 explicit-id"), "{stdout}");
    assert!(!stdout.contains("codex-last"), "{stdout}");
    let all_arguments: String = runner.started()[0].arguments.join(" ");
    assert!(
        all_arguments.contains("explicit-id"),
        "explicit resolved id must reach the argv: {all_arguments}"
    );
}

#[test]
fn open_custom_tool_from_config_launches() {
    let dir = TestDir::new();
    let project_path = dir.existing_project("workdir");
    {
        let _store = SqliteStore::new(dir.db()).unwrap();
        let mut config = AppConfig::default();
        config.custom_tools = vec![ToolSource {
            key: "myagent".to_string(),
            cli_command: "mycli".to_string(),
            is_enabled: true,
            ..ToolSource::default()
        }];
        std::fs::write(dir.config(), serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    }
    let runner = RecordingProcessRunner::new();
    let resolver = FakeResolver {
        present: vec!["mycli", "gnome-terminal"],
    };
    let env = open_env(&runner, &resolver, None);

    let (code, stdout, stderr) =
        run_open_with(&dir, &path_args(&project_path, Some("myagent")), "", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("正在启动 myagent"), "{stdout}");
    assert_eq!(runner.start_count(), 1);

    let store = SqliteStore::new(dir.db()).unwrap();
    let sessions = store.get_recent_sessions(1).unwrap();
    assert_eq!(sessions[0].tool_key, "myagent");
}

#[test]
fn open_custom_tool_cannot_override_built_in() {
    let dir = TestDir::new();
    let project_path = dir.existing_project("workdir");
    {
        let _store = SqliteStore::new(dir.db()).unwrap();
        let mut config = AppConfig::default();
        config.custom_tools = vec![ToolSource {
            key: "claude".to_string(),
            cli_command: "evil".to_string(),
            is_enabled: true,
            ..ToolSource::default()
        }];
        std::fs::write(dir.config(), serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    }
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let (code, stdout, stderr) =
        run_open_with(&dir, &path_args(&project_path, Some("claude")), "", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("正在启动 claude"), "{stdout}");
    // The recorded session proves the built-in identity was launched.
    let store = SqliteStore::new(dir.db()).unwrap();
    let sessions = store.get_recent_sessions(1).unwrap();
    assert_eq!(sessions[0].tool_key, "claude");
}

#[test]
fn open_fuzzy_matching_caps_at_ten_choices() {
    let dir = TestDir::new();
    let mut projects = Vec::new();
    for index in 1..=12 {
        let path_text = dir.project_path(&format!("proj-{index:02}"));
        projects.push(project(
            &path_text,
            &format!("proj-{index:02}"),
            &[usage("codex", now_minus(12 * 60 - index * 60), 1, None)],
        ));
    }
    {
        let mut store = SqliteStore::new(dir.db()).unwrap();
        store.replace_tool_snapshots(&projects, &["codex"]).unwrap();
    }
    let runner = RecordingProcessRunner::new();
    let resolver = resolver_with_tools();
    let env = open_env(&runner, &resolver, None);

    let (code, stdout, stderr) =
        run_open_with(&dir, &path_args("proj", Some("codex")), "3\n", &env);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("找到多个匹配项目"), "{stdout}");
    assert!(
        stdout.contains("10) "),
        "must offer at most ten fuzzy choices: {stdout}"
    );
    assert!(
        !stdout.contains("11) "),
        "must cap fuzzy matches at ten: {stdout}"
    );
    assert_eq!(runner.start_count(), 1);
}
