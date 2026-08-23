//! `sessionatlas config` — show / add-tool / set-default-terminal.
//!
//! Task R09 implements the configuration surface R08 left as a stub. `show`
//! never creates the config file and reports a missing file as defaults;
//! `add-tool` and `set-default-terminal` persist through the core config's
//! locked atomic `update`, so concurrent writers cannot lose each other's
//! changes and an unreadable/invalid file is never overwritten.

use std::cell::Cell;
use std::path::Path;

use sessionatlas_core::config::{load as load_config, update, ConfigError};
use sessionatlas_core::model::ToolSource;

use crate::cli::ConfigAction;
use crate::render::sanitize;
use crate::select::{prompt_select, Selection};
use crate::Io;

use super::tools::{
    is_reserved_tool_key, parse_safe_command, validate_display_label, validate_tool_key,
};

/// Terminal choices accepted by `set-default-terminal`.
const DEFAULT_TERMINALS: [&str; 7] = [
    "auto",
    "windows-terminal",
    "cmd",
    "iterm2",
    "terminal",
    "gnome-terminal",
    "konsole",
];

/// Runs a config action against an explicit config path (tests point this at
/// a temporary `config.json`; the default action is `show`).
pub fn run_config(io: &mut Io<'_>, config_path: &Path, args: &crate::cli::ConfigArgs) -> i32 {
    match args.action {
        None | Some(ConfigAction::Show) => show_config(io, config_path),
        Some(ConfigAction::AddTool) => add_tool(io, config_path),
        Some(ConfigAction::SetDefaultTerminal) => set_default_terminal(io, config_path),
    }
}

/// Prints the effective configuration. A missing file renders defaults without
/// creating the file; an unreadable or invalid file fails without overwriting.
fn show_config(io: &mut Io<'_>, config_path: &Path) -> i32 {
    let config = match load_config(config_path) {
        Ok(config) => config,
        Err(_) => {
            io.err("配置文件无法读取或格式无效；为避免覆盖，未执行任何修改。\n");
            return 1;
        }
    };
    io.out("当前配置:\n");
    io.out(&format!(
        "默认终端: {}\n",
        sanitize(&config.default_terminal)
    ));
    io.out(&format!("自定义工具数量: {}\n", config.custom_tools.len()));
    for tool in &config.custom_tools {
        io.out(&format!(
            "  - {} ({}): {}\n",
            sanitize(&tool.name),
            sanitize(&tool.key),
            sanitize(&tool.data_directory)
        ));
    }
    0
}

/// Interactively registers a custom tool. Each value is validated before any
/// write; the key is re-checked inside the core config's locked atomic update
/// so a key added by a concurrent writer cannot be duplicated. `~` in the data
/// directory resolves against the sessionatlas home (`SESSIONATLAS_HOME`).
fn add_tool(io: &mut Io<'_>, config_path: &Path) -> i32 {
    let name = match read_line(io, "工具显示名称:") {
        Some(value) => value,
        None => return 0,
    };
    let key = match read_line(io, "工具唯一标识 (如 my-custom-agent):") {
        Some(value) => value,
        None => return 0,
    };
    let cli = match read_line(io, "CLI 命令:") {
        Some(value) => value,
        None => return 0,
    };
    let directory = match read_line(io, "数据目录 (绝对路径，可用 ~ 表示 home):") {
        Some(value) => value,
        None => return 0,
    };

    let name = match validate_display_label(&name) {
        Ok(valid) => valid,
        Err(message) => {
            io.err(&format!("{message}\n"));
            return 1;
        }
    };
    let key = match validate_tool_key(&key) {
        Ok(valid) => valid,
        Err(message) => {
            io.err(&format!("{message}\n"));
            return 1;
        }
    };
    if let Err(message) = parse_safe_command(&cli) {
        io.err(&format!("{message}\n"));
        return 1;
    }
    if is_reserved_tool_key(&key) {
        io.err(&format!(
            "工具标识 '{}' 已存在或属于内置工具\n",
            sanitize(&key)
        ));
        return 1;
    }
    if let Ok(config) = load_config(config_path) {
        if config
            .custom_tools
            .iter()
            .any(|tool| tool.key.eq_ignore_ascii_case(&key))
        {
            io.err(&format!(
                "工具标识 '{}' 已存在或属于内置工具\n",
                sanitize(&key)
            ));
            return 1;
        }
    }

    let data_directory =
        resolve_data_directory(&directory, &sessionatlas_core::config::home_directory());
    let tool = ToolSource {
        key: key.clone(),
        name,
        cli_command: cli,
        data_directory,
        is_enabled: true,
        ..ToolSource::default()
    };

    // The mutation runs under the core config's exclusive cross-process lock on
    // freshly-read bytes, so a key added by another writer in the meantime is
    // detected instead of producing a duplicate.
    let duplicate = Cell::new(false);
    let result = update(config_path, None, |latest| {
        if is_reserved_tool_key(&key)
            || latest
                .custom_tools
                .iter()
                .any(|existing| existing.key.eq_ignore_ascii_case(&key))
        {
            duplicate.set(true);
            return;
        }
        latest.custom_tools.push(tool.clone());
    });

    match result {
        Ok(_) if duplicate.get() => {
            io.err(&format!(
                "工具标识 '{}' 已存在或属于内置工具\n",
                sanitize(&key)
            ));
            1
        }
        Ok(_) => {
            io.out("自定义工具已添加并保存\n");
            0
        }
        Err(error) => report_config_error(io, error),
    }
}

/// Interactively selects one of the seven supported terminals and persists it
/// through the same atomic update. Cancellation leaves the config untouched.
fn set_default_terminal(io: &mut Io<'_>, config_path: &Path) -> i32 {
    let choices: Vec<String> = DEFAULT_TERMINALS
        .iter()
        .map(|value| value.to_string())
        .collect();
    let selection = match prompt_select(&mut *io.stdin, &mut *io.stdout, "选择默认终端:", &choices)
    {
        Ok(selection) => selection,
        Err(error) => {
            io.err(&format!("交互输入失败: {error}\n"));
            return 1;
        }
    };
    let term = match selection {
        Selection::Cancel => return 0,
        Selection::Chosen(index) => DEFAULT_TERMINALS[index].to_string(),
    };

    match update(config_path, None, |config| {
        config.default_terminal = term.clone()
    }) {
        Ok(_) => {
            io.out(&format!("默认终端已设置为: {}\n", sanitize(&term)));
            0
        }
        Err(error) => report_config_error(io, error),
    }
}

/// Maps core config failures to user-facing command messages. Every path leaves
/// any existing file untouched.
fn report_config_error(io: &mut Io<'_>, error: ConfigError) -> i32 {
    match error {
        ConfigError::Busy(_) => {
            io.err("配置正被其他进程使用，请稍后重试。\n");
        }
        ConfigError::Conflict => {
            io.err("配置已被其他进程更新，请重新运行命令。\n");
        }
        ConfigError::Json(_) => {
            io.err("配置格式无效，未写入任何修改。\n");
        }
        ConfigError::Io(_) | ConfigError::InvalidPath(_) => {
            io.err("配置保存失败，请检查文件权限和磁盘状态。\n");
        }
    }
    1
}

/// Reads one interactive line after prompting, mirroring `AnsiConsole.Ask`.
/// EOF or a read failure cancels the flow (returning `None`).
fn read_line(io: &mut Io<'_>, prompt: &str) -> Option<String> {
    io.out(prompt);
    let mut line = String::new();
    match io.stdin.read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => Some(line.trim_end_matches(['\r', '\n']).to_string()),
        Err(_) => None,
    }
}

/// Resolves the entered data directory: a bare `~` is the SessionAtlas home and a
/// `~/`/`~\` prefix is joined to it; everything else is stored as typed.
fn resolve_data_directory(directory: &str, home: &Path) -> String {
    if directory == "~" {
        return home.to_string_lossy().into_owned();
    }
    if let Some(rest) = directory
        .strip_prefix("~/")
        .or_else(|| directory.strip_prefix("~\\"))
    {
        return home.join(rest).to_string_lossy().into_owned();
    }
    directory.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn data_directory_expands_tilde_against_the_supplied_home() {
        let home = PathBuf::from("home").join("test");
        assert_eq!(resolve_data_directory("~", &home), home.to_string_lossy());
        assert_eq!(
            resolve_data_directory("~/agent", &home),
            home.join("agent").to_string_lossy()
        );
        assert_eq!(
            resolve_data_directory("~\\agent", &home),
            home.join("agent").to_string_lossy()
        );
        assert_eq!(
            resolve_data_directory("~/a/b", &home),
            home.join("a/b").to_string_lossy(),
            "embedded separators are preserved like Path.Combine"
        );
    }

    #[test]
    fn data_directory_keeps_absolute_and_other_paths_as_typed() {
        let home = PathBuf::from("/home/test");
        assert_eq!(resolve_data_directory("/srv/data", &home), "/srv/data");
        assert_eq!(resolve_data_directory("~other", &home), "~other");
        assert_eq!(resolve_data_directory("", &home), "");
        assert_eq!(
            resolve_data_directory("/home/test/agent", &home),
            "/home/test/agent"
        );
    }
}
