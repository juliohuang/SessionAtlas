//! End-to-end tests for the R08 read-only CLI commands.
//!
//! Every test seeds an isolated temporary database through the core store and
//! drives the command handlers through `sessionatlas_cli::run_with_db` with
//! scripted stdin, so no test reads or writes the real `~/.sessionatlas`,
//! starts a terminal, or launches an AI CLI. Timestamps are built from
//! `SystemTime` (converted via `From<SystemTime>` on the model fields) so this
//! crate's tests never depend on chrono being a direct dependency.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use crate::cli::{Cli, Command, ListArgs, RecentArgs, SearchArgs};
use crate::{run_with_db, Io};
use sessionatlas_core::model::{Project, Session, ToolUsage};
use sessionatlas_core::store::SqliteStore;

/// Serializes tests that mutate the process-global `SESSIONATLAS_HOME`.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Unique temporary directory removed on drop. `tempfile` is not a dependency
/// of this crate (its Cargo.toml is frozen for this task), so tests create
/// their own disposable root.
struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!(
            "sessionatlas-cli-test-{}-{}-{nonce}",
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

fn session(
    id: &str,
    path_text: &str,
    tool_key: &str,
    tool_name: &str,
    started: SystemTime,
) -> Session {
    Session {
        id: id.to_string(),
        project_path: path_text.to_string(),
        tool_key: tool_key.to_string(),
        tool_name: tool_name.to_string(),
        started_at: started.into(),
        ended_at: None,
        session_id_from_tool: None,
    }
}

/// Seeds an index with two projects (recency order: `web-frontend` then
/// `api-server`) and two sessions.
fn seed_db(dir: &TestDir) {
    let mut store = SqliteStore::new(dir.db()).unwrap();
    let api = abs_path(&["work", "api-server"]);
    let web = abs_path(&["work", "web-frontend"]);
    store
        .replace_tool_snapshots(
            &[
                project(
                    &api,
                    "api-id",
                    &[
                        usage("codex", now_minus(3600), 3),
                        usage("claude", now_minus(7200), 1),
                    ],
                ),
                project(&web, "web-id", &[usage("codex", now_minus(1800), 2)]),
            ],
            &["codex", "claude"],
        )
        .unwrap();
    store
        .record_session(&session("s-new", &web, "codex", "Codex", now_minus(600)))
        .unwrap();
    store
        .record_session(&session("s-old", &api, "claude", "Claude", now_minus(3600)))
        .unwrap();
}

/// Runs a parsed CLI against the directory's database with scripted stdin.
fn run_with(dir: &TestDir, cli: Cli, input: &str) -> (i32, String, String) {
    let mut reader = Cursor::new(input.as_bytes().to_vec());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut io = Io {
        stdin: &mut reader,
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = run_with_db(cli, &mut io, &dir.db());
    (
        code,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

fn list_args(interactive: bool) -> ListArgs {
    ListArgs {
        tool: None,
        limit: 50,
        interactive,
    }
}

#[test]
fn help_surface_declares_all_public_commands() {
    use clap::CommandFactory;
    let command = Cli::command();
    let names: Vec<String> = command
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_string())
        .collect();
    for expected in ["scan", "list", "search", "open", "recent", "config"] {
        assert!(
            names.iter().any(|name| name == expected),
            "public command {expected} missing from --help: {names:?}"
        );
    }
}

#[test]
fn missing_database_returns_guidance_and_creates_nothing() {
    let dir = TestDir::new();
    let (code, stdout, stderr) = run_with(
        &dir,
        Cli {
            command: Some(Command::List(list_args(false))),
        },
        "",
    );
    assert_eq!(code, 1);
    assert!(stdout.is_empty());
    assert!(stderr.contains("请先运行 `sessionatlas scan`"), "{stderr}");
    assert!(
        !dir.0.join(".sessionatlas").exists(),
        "a read-only command must not create the data directory"
    );
}

#[test]
fn list_on_empty_index_prints_scan_hint() {
    let dir = TestDir::new();
    {
        let _store = SqliteStore::new(dir.db()).unwrap();
    }
    let (code, stdout, _) = run_with(
        &dir,
        Cli {
            command: Some(Command::List(list_args(false))),
        },
        "",
    );
    assert_eq!(code, 0);
    assert!(
        stdout.contains("暂无项目索引，请先运行 sessionatlas scan"),
        "{stdout}"
    );
}

#[test]
fn list_renders_seeded_projects_without_terminal_control_bytes() {
    let dir = TestDir::new();
    seed_db(&dir);
    let (code, stdout, _) = run_with(
        &dir,
        Cli {
            command: Some(Command::List(list_args(false))),
        },
        "",
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("api-server"), "{stdout}");
    assert!(stdout.contains("web-frontend"), "{stdout}");
    assert!(stdout.contains("codex"), "{stdout}");
    assert!(stdout.contains("共 2 个项目"), "{stdout}");
    assert!(!stdout.contains('\u{001B}'), "{stdout}");
}

#[test]
fn list_filters_by_tool_key() {
    let dir = TestDir::new();
    seed_db(&dir);
    let mut args = list_args(false);
    args.tool = Some("claude".to_string());
    let (code, stdout, _) = run_with(
        &dir,
        Cli {
            command: Some(Command::List(args)),
        },
        "",
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("api-server"), "{stdout}");
    assert!(!stdout.contains("web-frontend"), "{stdout}");
    assert!(stdout.contains("共 1 个项目"), "{stdout}");
}

#[test]
fn list_zero_limit_fails_before_touching_database() {
    let dir = TestDir::new();
    let mut args = list_args(false);
    args.limit = 0;
    let (code, _, stderr) = run_with(
        &dir,
        Cli {
            command: Some(Command::List(args)),
        },
        "",
    );
    assert_eq!(code, 1);
    assert!(
        stderr.contains("--limit 必须在 1 到 10000 之间。"),
        "{stderr}"
    );
    assert!(!dir.0.join(".sessionatlas").exists());
}

#[test]
fn list_interactive_cancel_exits_zero() {
    let dir = TestDir::new();
    seed_db(&dir);
    let (code, stdout, _) = run_with(
        &dir,
        Cli {
            command: Some(Command::List(list_args(true))),
        },
        "0\n",
    );
    assert_eq!(code, 0);
    assert!(!stdout.contains("已选择:"), "{stdout}");
}

#[test]
fn search_without_match_prints_message() {
    let dir = TestDir::new();
    seed_db(&dir);
    let (code, stdout, _) = run_with(
        &dir,
        Cli {
            command: Some(Command::Search(SearchArgs {
                query: "nomatch".to_string(),
                interactive: true,
            })),
        },
        "",
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("未找到匹配 'nomatch' 的项目"), "{stdout}");
}

#[test]
fn search_non_interactive_prints_table_without_prompt() {
    let dir = TestDir::new();
    seed_db(&dir);
    let (code, stdout, _) = run_with(
        &dir,
        Cli {
            command: Some(Command::Search(SearchArgs {
                query: "api".to_string(),
                interactive: false,
            })),
        },
        "",
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("找到 1 个匹配项目"), "{stdout}");
    assert!(!stdout.contains("已选择:"), "{stdout}");
    assert!(!stdout.contains("请输入选择"), "{stdout}");
}

#[test]
fn recent_lists_seeded_sessions() {
    let dir = TestDir::new();
    seed_db(&dir);
    let (code, stdout, _) = run_with(
        &dir,
        Cli {
            command: Some(Command::Recent(RecentArgs {
                count: 10,
                open: false,
            })),
        },
        "",
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("Codex"), "{stdout}");
    assert!(stdout.contains("Claude"), "{stdout}");
    assert!(stdout.contains("web-frontend"), "{stdout}");
    assert!(stdout.contains("api-server"), "{stdout}");
}

#[test]
fn recent_zero_count_fails_before_touching_database() {
    let dir = TestDir::new();
    let (code, _, stderr) = run_with(
        &dir,
        Cli {
            command: Some(Command::Recent(RecentArgs {
                count: 0,
                open: false,
            })),
        },
        "",
    );
    assert_eq!(code, 1);
    assert!(
        stderr.contains("--count 必须在 1 到 1000 之间。"),
        "{stderr}"
    );
    assert!(!dir.0.join(".sessionatlas").exists());
}

#[test]
fn output_strips_ansi_and_control_bytes_from_indexed_data() {
    let dir = TestDir::new();
    let hostile = abs_path(&["work", "hostile"]);
    {
        let mut store = SqliteStore::new(dir.db()).unwrap();
        let mut hostile_project =
            project(&hostile, "hostile-id", &[usage("codex", now_minus(60), 1)]);
        hostile_project.git_branch = Some("\u{001B}[31mred\u{0007}\u{001B}[K".to_string());
        store
            .replace_tool_snapshots(&[hostile_project], &["codex"])
            .unwrap();
        store
            .record_session(&session(
                "s-host",
                &hostile,
                "codex",
                "Codex\u{001B}[31m",
                now_minus(30),
            ))
            .unwrap();
    }

    let (code, stdout, _) = run_with(
        &dir,
        Cli {
            command: Some(Command::List(list_args(false))),
        },
        "",
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("hostile"), "{stdout}");
    assert!(stdout.contains("red"), "{stdout}");
    assert!(!stdout.contains('\u{001B}'), "list leaked ESC: {stdout}");
    assert!(!stdout.contains('\u{0007}'), "list leaked BEL: {stdout}");

    let (code, stdout, _) = run_with(
        &dir,
        Cli {
            command: Some(Command::Recent(RecentArgs {
                count: 10,
                open: false,
            })),
        },
        "",
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("Codex"), "{stdout}");
    assert!(!stdout.contains('\u{001B}'), "recent leaked ESC: {stdout}");
}

#[test]
fn default_db_path_follows_sessionatlas_home() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = TestDir::new();
    let previous = std::env::var_os("SESSIONATLAS_HOME");
    std::env::set_var("SESSIONATLAS_HOME", &dir.0);
    let resolved = crate::db::default_db_path();
    restore_sessionatlas_home(previous);
    assert_eq!(resolved, dir.0.join(".sessionatlas").join("index.db"));
}

fn restore_sessionatlas_home(previous: Option<std::ffi::OsString>) {
    match previous {
        Some(value) => std::env::set_var("SESSIONATLAS_HOME", value),
        None => std::env::remove_var("SESSIONATLAS_HOME"),
    }
}

#[test]
fn list_supports_explicit_absolute_database_path_argument_flow() {
    // Regression guard: handlers take the database path directly; a caller may
    // point them at an isolated file that follows the production layout.
    let dir = TestDir::new();
    seed_db(&dir);
    let db_path: &Path = &dir.db();
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
            command: Some(Command::List(list_args(false))),
        },
        &mut io,
        db_path,
    );
    assert_eq!(code, 0);
    let stdout = String::from_utf8(stdout).unwrap();
    assert!(stdout.contains("api-server"), "{stdout}");
    assert!(stdout.contains("web-frontend"), "{stdout}");
}
