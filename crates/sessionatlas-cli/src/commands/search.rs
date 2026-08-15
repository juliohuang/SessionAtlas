//! `sessionatlas search` — read-only fuzzy search.

use std::path::Path;

use crate::cli::{SearchArgs, SEARCH_LIMIT};
use crate::commands::open::OpenEnvironment;
use crate::render::{render_search, sanitize};
use crate::Io;

/// Runs `search`: prints the matching projects, then (unless
/// interactive mode prompts to select one.
pub fn run_search(
    io: &mut Io<'_>,
    db_path: &Path,
    config_path: &Path,
    args: &SearchArgs,
    env: &OpenEnvironment<'_>,
) -> i32 {
    let store = match crate::db::open_index_store(db_path) {
        Ok(store) => store,
        Err(message) => {
            io.err(&format!("{message}\n"));
            return 1;
        }
    };
    let projects = match store.list_projects(Some(&args.query), None, SEARCH_LIMIT) {
        Ok(projects) => projects,
        Err(error) => {
            io.err(&format!("读取索引失败: {error}\n"));
            return 1;
        }
    };
    if projects.is_empty() {
        let query = sanitize(&args.query);
        io.out(&format!("未找到匹配 '{query}' 的项目\n"));
        return 0;
    }
    io.out(&format!("找到 {} 个匹配项目:\n\n", projects.len()));
    io.out(&render_search(&projects));
    io.out("\n");
    if !args.interactive {
        return 0;
    }
    drop(store);
    super::interactive_projects(
        io,
        "选择要打开的项目 (或取消):",
        &projects,
        db_path,
        config_path,
        env,
    )
}
