//! Command handlers. `list`, `search` and `recent` are the R08 read-only
//! commands; `scan` and `config` are implemented by R09; `open` is implemented
//! by R10.

pub mod config;
pub mod list;
pub mod open;
pub mod recent;
pub mod scan;
pub mod search;
pub mod tools;

use sessionatlas_core::model::Project;

use crate::cli::OpenArgs;
use crate::commands::open::OpenEnvironment;
use crate::render;
use crate::select::{prompt_select, Selection};
use crate::Io;

/// Sanitized 1-based choice labels for a project list.
fn project_choices(projects: &[Project]) -> Vec<String> {
    projects.iter().map(project_choice_label).collect()
}

fn project_choice_label(project: &Project) -> String {
    let name = render::sanitize(&project.display_name().unwrap_or_default());
    let tools = render::sanitize(&project.tool_tags());
    let path = render::truncate(&render::sanitize(&project.path), 40);
    format!("{name} ({tools}) {path}")
}

/// Shared `list` / `search` interactive flow. A selected project delegates to
/// the injected `open` implementation, including interactive tool choice.
fn interactive_projects(
    io: &mut Io<'_>,
    title: &str,
    projects: &[Project],
    db_path: &std::path::Path,
    config_path: &std::path::Path,
    env: &OpenEnvironment<'_>,
) -> i32 {
    let choices = project_choices(projects);
    let selection = match prompt_select(&mut *io.stdin, &mut *io.stdout, title, &choices) {
        Ok(selection) => selection,
        Err(error) => {
            io.err(&format!("交互输入失败: {error}\n"));
            return 1;
        }
    };
    match selection {
        Selection::Cancel => 0,
        Selection::Chosen(index) => {
            let args = OpenArgs {
                project_path: Some(projects[index].path.clone()),
                tool: None,
                interactive: true,
                recent: false,
            };
            let selected_env = OpenEnvironment {
                runner: env.runner,
                resolver: env.resolver,
                wt_probe: env.wt_probe,
                resolved_session_id: None,
            };
            open::run_open(io, db_path, config_path, &args, &selected_env)
        }
    }
}
