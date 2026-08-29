//! `sessionatlas open` — resolve a project, pick a tool, launch it in a
//! terminal through the injectable process boundary, and record the session
//! only after the launch was accepted.
//!
//! All interactive selection goes
//! through [`crate::select::prompt_select`], every external interaction
//! (process start, PATH probing, Windows Terminal detection) is injected via
//! [`OpenEnvironment`], and every value originating in the database, config,
//! paths, or errors passes through [`crate::render::sanitize`] before output.

use std::path::Path;

use sessionatlas_core::adapter::{adapter_root_for_config, AdapterRegistry};
use sessionatlas_core::launcher::{Launcher, ToolCommands};
use sessionatlas_core::model::Session;
use sessionatlas_core::path;
use sessionatlas_core::process::{ProcessRunner, ProgramResolver};
use sessionatlas_core::security;
use sessionatlas_core::store::SqliteStore;

use crate::cli::OpenArgs;
use crate::render::sanitize;
use crate::select::{prompt_select, Selection};
use crate::Io;

/// External interactions `open` needs. Production wires
/// `SystemProcessRunner`, `PathProgramResolver` and
/// `sessionatlas_core::launcher::default_wt_probe`; tests inject recording and
/// fake implementations so nothing external is ever started.
pub struct OpenEnvironment<'a> {
    pub runner: &'a dyn ProcessRunner,
    pub resolver: &'a dyn ProgramResolver,
    pub wt_probe: fn(&Path) -> bool,
    /// An already-resolved session ID supplied by a typed caller. It wins over
    /// the recent-session record and the project's `last_session_id`.
    pub resolved_session_id: Option<String>,
}

/// Runs `open` against an explicit database and config path.
pub fn run_open(
    io: &mut Io<'_>,
    db_path: &Path,
    config_path: &Path,
    args: &OpenArgs,
    env: &OpenEnvironment<'_>,
) -> i32 {
    // `open` records sessions, so it may create the index database.
    let store = match SqliteStore::new(db_path) {
        Ok(store) => store,
        Err(error) => {
            io.err(&format!(
                "打开索引数据库失败: {}\n",
                sanitize(&error.to_string())
            ));
            return 1;
        }
    };
    let config = sessionatlas_core::config::try_load(config_path).unwrap_or_default();
    let registry = AdapterRegistry::load(&adapter_root_for_config(config_path), &config)
        .or_else(|_| AdapterRegistry::bundled())
        .expect("compiled official adapter manifests must be valid");
    let launcher = Launcher::new(
        ToolCommands::from_registry(&config, &registry),
        env.resolver,
        env.runner,
        &env.wt_probe,
    );

    let mut recent_session: Option<Session> = None;
    let resolved_path = if args.recent {
        match store.get_recent_sessions(1) {
            Ok(sessions) => match sessions.into_iter().next() {
                Some(session) => {
                    recent_session = Some(session.clone());
                    session.project_path
                }
                None => {
                    io.err("没有最近会话记录\n");
                    return 1;
                }
            },
            Err(error) => {
                io.err(&format!(
                    "读取会话记录失败: {}\n",
                    sanitize(&error.to_string())
                ));
                return 1;
            }
        }
    } else if let Some(input) = args.project_path.as_deref() {
        match resolve_existing_or_fuzzy(&store, io, input) {
            ResolveOutcome::Path(path) => path,
            ResolveOutcome::Exit(code) => return code,
        }
    } else {
        match select_project_interactively(&store, io) {
            ResolveOutcome::Path(path) => path,
            ResolveOutcome::Exit(code) => return code,
        }
    };

    if !Path::new(&resolved_path).is_dir() {
        io.err("无法解析项目路径\n");
        return 1;
    }

    // Determine the tool: explicit `--tool` wins unless `--interactive` forces
    // a prompt; `--recent` defaults to the recent session's tool.
    let mut tool_key = args.tool.clone();
    if args.recent && tool_key.is_none() {
        tool_key = recent_session.as_ref().and_then(|session| {
            (!session.tool_key.trim().is_empty()).then(|| session.tool_key.clone())
        });
    }
    let tool_key = match tool_key {
        Some(key) if !args.interactive => key,
        _ => {
            let project = match store.get_project_by_path(&resolved_path) {
                Ok(project) => project,
                Err(error) => {
                    io.err(&format!(
                        "读取项目信息失败: {}\n",
                        sanitize(&error.to_string())
                    ));
                    return 1;
                }
            };
            let mut candidates: Vec<String> = match &project {
                Some(project) => project
                    .tool_usages
                    .iter()
                    .map(|usage| usage.tool_key.clone())
                    .collect(),
                None => registry
                    .enabled(&config)
                    .map(|adapter| adapter.id.clone())
                    .collect(),
            };
            candidates.retain(|key| launcher.is_tool_available(key));
            if candidates.is_empty() {
                io.err("未在 PATH 中找到任何可用的 AI CLI 工具命令\n");
                return 1;
            }
            let title = format!(
                "选择要在 {} 中使用的 CLI 工具:",
                sanitize(&path::display_name_native(&resolved_path).unwrap_or_default())
            );
            let choices: Vec<String> = candidates.iter().map(|key| sanitize(key)).collect();
            match choose(io, &title, &choices) {
                SelectionOutcome::Chosen(index) => candidates[index].clone(),
                SelectionOutcome::Cancelled => return 0,
                SelectionOutcome::InputFailed => return 1,
            }
        }
    };
    let tool_key = match security::validate_tool_key(&tool_key) {
        Ok(key) => key,
        Err(_) => {
            io.err("工具标识包含不支持的字符\n");
            return 1;
        }
    };

    // Session ID priority: explicitly resolved value > recent record >
    // project's `last_session_id` for the chosen tool. An invalid ID from
    // storage only warns and starts a new session.
    let resume_session_id = if let Some(explicit) = env.resolved_session_id.clone() {
        Some(explicit)
    } else if args.recent
        && recent_session
            .as_ref()
            .is_some_and(|session| session.tool_key.eq_ignore_ascii_case(&tool_key))
    {
        recent_session
            .as_ref()
            .and_then(|session| session.session_id_from_tool.clone())
    } else {
        match store.get_project_by_path(&resolved_path) {
            Ok(Some(project)) => project
                .tool_usages
                .iter()
                .find(|usage| usage.tool_key.eq_ignore_ascii_case(&tool_key))
                .and_then(|usage| usage.last_session_id.clone()),
            Ok(None) => None,
            Err(error) => {
                io.err(&format!(
                    "读取项目信息失败: {}\n",
                    sanitize(&error.to_string())
                ));
                return 1;
            }
        }
    };
    let resume_session_id = match resume_session_id {
        Some(id) if !id.trim().is_empty() => match security::validate_session_id(&id) {
            Ok(valid) => Some(valid),
            Err(_) => {
                io.err("索引中的会话 ID 格式无效；将启动新会话。\n");
                None
            }
        },
        _ => None,
    };

    io.out(&format!(
        "正在启动 {} 在 {}...\n",
        sanitize(&tool_key),
        sanitize(&resolved_path)
    ));
    if let Some(id) = &resume_session_id {
        io.out(&format!(
            "尝试恢复会话 {}（使用对应工具的恢复语法）\n",
            sanitize(id)
        ));
    }

    if let Err(error) = launcher.launch(&resolved_path, &tool_key, resume_session_id.as_deref()) {
        io.err(&format!("启动失败: {}\n", sanitize(&error.to_string())));
        return 1;
    }

    // Record only after the terminal process accepted the launch request.
    let session = Session {
        project_path: resolved_path.clone(),
        tool_key: tool_key.clone(),
        tool_name: tool_key.clone(),
        session_id_from_tool: resume_session_id.clone(),
        ..Session::default()
    };
    if let Err(error) = store.record_session(&session) {
        io.err(&format!("记录会话失败: {}\n", sanitize(&error.to_string())));
        return 1;
    }
    0
}

/// Result of resolving a project path.
enum ResolveOutcome {
    Path(String),
    Exit(i32),
}

/// Resolves a typed project argument: an existing directory is used directly
/// (normalized to its native absolute form); otherwise at most 10 fuzzy matches
/// are considered, a single match wins, multiple matches prompt, and none is an
/// error.
fn resolve_existing_or_fuzzy(store: &SqliteStore, io: &mut Io<'_>, input: &str) -> ResolveOutcome {
    if Path::new(input).is_dir() {
        return match absolute_normalize(input) {
            Some(path) => ResolveOutcome::Path(path),
            None => {
                io.err("无法解析项目路径\n");
                ResolveOutcome::Exit(1)
            }
        };
    }
    let matches = match store.list_projects(Some(input), None, 10) {
        Ok(matches) => matches,
        Err(error) => {
            io.err(&format!("搜索项目失败: {}\n", sanitize(&error.to_string())));
            return ResolveOutcome::Exit(1);
        }
    };
    if matches.len() == 1 {
        return ResolveOutcome::Path(matches[0].path.clone());
    }
    if matches.len() > 1 {
        let choices: Vec<String> = matches
            .iter()
            .map(|project| {
                format!(
                    "{} ({})",
                    sanitize(&project.display_name().unwrap_or_default()),
                    sanitize(&project.path)
                )
            })
            .collect();
        return match choose(io, "找到多个匹配项目，请选择:", &choices) {
            SelectionOutcome::Chosen(index) => ResolveOutcome::Path(matches[index].path.clone()),
            SelectionOutcome::Cancelled => ResolveOutcome::Exit(0),
            SelectionOutcome::InputFailed => ResolveOutcome::Exit(1),
        };
    }
    io.err(&format!(
        "未找到匹配路径 '{}' 的项目，请运行 'sessionatlas scan' 或检查路径\n",
        sanitize(input)
    ));
    ResolveOutcome::Exit(1)
}

/// Fully interactive project selection when no path and `--recent` are given.
fn select_project_interactively(store: &SqliteStore, io: &mut Io<'_>) -> ResolveOutcome {
    let projects = match store.list_projects(None, None, 100) {
        Ok(projects) => projects,
        Err(error) => {
            io.err(&format!("读取索引失败: {}\n", sanitize(&error.to_string())));
            return ResolveOutcome::Exit(1);
        }
    };
    if projects.is_empty() {
        io.out("暂无项目，请先运行 sessionatlas scan\n");
        return ResolveOutcome::Exit(0);
    }
    let choices: Vec<String> = projects
        .iter()
        .map(|project| {
            format!(
                "{} ({}) {}",
                sanitize(&project.display_name().unwrap_or_default()),
                sanitize(&project.tool_tags()),
                sanitize(&project.path)
            )
        })
        .collect();
    match choose(io, "选择项目:", &choices) {
        SelectionOutcome::Chosen(index) => ResolveOutcome::Path(projects[index].path.clone()),
        SelectionOutcome::Cancelled => ResolveOutcome::Exit(0),
        SelectionOutcome::InputFailed => ResolveOutcome::Exit(1),
    }
}

/// Outcome of one interactive selection.
enum SelectionOutcome {
    Chosen(usize),
    Cancelled,
    InputFailed,
}

fn choose(io: &mut Io<'_>, title: &str, choices: &[String]) -> SelectionOutcome {
    match prompt_select(&mut *io.stdin, &mut *io.stdout, title, choices) {
        Ok(Selection::Chosen(index)) => SelectionOutcome::Chosen(index),
        Ok(Selection::Cancel) => SelectionOutcome::Cancelled,
        Err(error) => {
            io.err(&format!("交互输入失败: {}\n", sanitize(&error.to_string())));
            SelectionOutcome::InputFailed
        }
    }
}

/// Resolves a possibly-relative existing directory to its native normalized
/// absolute form.
fn absolute_normalize(candidate: &str) -> Option<String> {
    let rooted = if Path::new(candidate).has_root() {
        candidate.to_string()
    } else {
        std::path::absolute(candidate)
            .ok()?
            .to_string_lossy()
            .into_owned()
    };
    path::normalize_native(&rooted)
}
