//! Thin CLI adapter over the shared, coordinated content-index controls.

use crate::cli::{ContentIndexAction as Action, ContentIndexArgs};
use crate::{render::sanitize, Io};
use sessionatlas_core::store::{
    change_content_index_at, content_index_status_at, ContentIndexAction,
};
use std::path::Path;

pub fn run(io: &mut Io<'_>, path: &Path, args: &ContentIndexArgs) -> i32 {
    let action = match args.action {
        Action::Status => None,
        Action::Enable => Some(ContentIndexAction::Enable),
        Action::Disable => Some(ContentIndexAction::Disable),
        Action::Clear => Some(ContentIndexAction::Clear),
        Action::Rebuild => Some(ContentIndexAction::Rebuild),
    };
    let result = match action {
        Some(action) => change_content_index_at(path, action),
        None => content_index_status_at(path),
    };
    let status = match result {
        Ok(status) => status,
        Err(error) => {
            io.err(&format!("内容索引操作失败: {}\n请运行 sessionatlas content-index status 查看实际状态；清理未完成时可重试 clear。\n", sanitize(&error.to_string())));
            return 1;
        }
    };
    if args.json {
        match serde_json::to_string(&status) {
            Ok(json) => io.out(&format!("{json}\n")),
            Err(error) => {
                io.err(&format!("无法输出内容索引状态: {error}\n"));
                return 1;
            }
        }
    } else {
        io.out(&format!(
            "内容索引: {}；状态 {:?}；文件 {}；字节 {}；可搜索 {}。\n",
            if status.enabled {
                "已开启"
            } else {
                "已关闭"
            },
            status.phase,
            status.indexed_files,
            status.indexed_bytes,
            status.searchable
        ));
        if !status.index_exists {
            io.out("项目索引尚未创建，请先运行 sessionatlas scan。\n");
        } else if matches!(args.action, Action::Clear | Action::Enable) && status.enabled {
            io.out("下次 scan 会重新索引内容，也可运行 content-index rebuild 立即重建。\n");
        }
    }
    0
}
