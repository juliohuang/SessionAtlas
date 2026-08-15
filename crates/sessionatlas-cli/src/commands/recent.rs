//! `sessionatlas recent` — read-only recent session listing.

use std::path::Path;

use sessionatlas_core::model::Session;

use crate::cli::{OpenArgs, RecentArgs, RECENT_COUNT_MAX};
use crate::commands::open::OpenEnvironment;
use crate::render::{render_recent, sanitize};
use crate::select::{prompt_select, Selection};
use crate::Io;

/// Runs `recent`: prints recent sessions, then (with `--open`) selects one and
/// delegates to the same injected, validated launch flow as `sessionatlas open`.
pub fn run_recent(
    io: &mut Io<'_>,
    db_path: &Path,
    config_path: &Path,
    args: &RecentArgs,
    env: &OpenEnvironment<'_>,
) -> i32 {
    if !(1..=RECENT_COUNT_MAX).contains(&args.count) {
        io.err("--count 必须在 1 到 1000 之间。\n");
        return 1;
    }
    let store = match crate::db::open_index_store(db_path) {
        Ok(store) => store,
        Err(message) => {
            io.err(&format!("{message}\n"));
            return 1;
        }
    };
    let sessions = match store.get_recent_sessions(args.count) {
        Ok(sessions) => sessions,
        Err(error) => {
            io.err(&format!("读取会话记录失败: {error}\n"));
            return 1;
        }
    };
    if sessions.is_empty() {
        io.out("暂无会话记录\n");
        return 0;
    }
    io.out(&render_recent(&sessions));
    io.out("\n");
    if !args.open {
        return 0;
    }
    let choices: Vec<String> = sessions.iter().map(session_label).collect();
    let selected = match prompt_select(&mut *io.stdin, &mut *io.stdout, "选择会话恢复:", &choices)
    {
        Ok(Selection::Chosen(index)) => sessions[index].clone(),
        Ok(Selection::Cancel) => return 0,
        Err(error) => {
            io.err(&format!("交互输入失败: {}\n", sanitize(&error.to_string())));
            return 1;
        }
    };
    drop(store);

    let open_args = OpenArgs {
        project_path: Some(selected.project_path),
        tool: Some(selected.tool_key),
        interactive: false,
        recent: false,
    };
    let selected_env = OpenEnvironment {
        runner: env.runner,
        resolver: env.resolver,
        wt_probe: env.wt_probe,
        resolved_session_id: selected.session_id_from_tool,
    };
    super::open::run_open(io, db_path, config_path, &open_args, &selected_env)
}

fn session_label(session: &Session) -> String {
    format!(
        "{} | {}",
        sanitize(&session.tool_name),
        sanitize(&session.project_path)
    )
}
