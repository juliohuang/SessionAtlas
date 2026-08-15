//! `sessionatlas list` — read-only project listing.

use std::path::Path;

use crate::cli::{ListArgs, LIST_LIMIT_MAX};
use crate::commands::open::OpenEnvironment;
use crate::render::render_list;
use crate::Io;

/// Runs `list` (non-interactive table or `--interactive` selection).
pub fn run_list(
    io: &mut Io<'_>,
    db_path: &Path,
    config_path: &Path,
    args: &ListArgs,
    env: &OpenEnvironment<'_>,
) -> i32 {
    if !(1..=LIST_LIMIT_MAX).contains(&args.limit) {
        io.err("--limit 必须在 1 到 10000 之间。\n");
        return 1;
    }
    let store = match crate::db::open_index_store(db_path) {
        Ok(store) => store,
        Err(message) => {
            io.err(&format!("{message}\n"));
            return 1;
        }
    };
    let projects = match store.list_projects(None, args.tool.as_deref(), args.limit) {
        Ok(projects) => projects,
        Err(error) => {
            io.err(&format!("读取索引失败: {error}\n"));
            return 1;
        }
    };
    if projects.is_empty() {
        io.out("暂无项目索引，请先运行 sessionatlas scan\n");
        return 0;
    }
    if args.interactive {
        drop(store);
        return super::interactive_projects(
            io,
            "选择要打开的项目:",
            &projects,
            db_path,
            config_path,
            env,
        );
    }
    io.out(&render_list(&projects));
    io.out(&format!(
        "\n共 {} 个项目 (使用 --interactive 交互式打开)\n",
        projects.len()
    ));
    0
}
