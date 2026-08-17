//! Command-line surface for the `sessionatlas` binary.
//!
//! Task R08 implemented the read-only commands `list`, `search` and `recent`
//! plus the no-argument default interactive list; task R09 implemented `scan`
//! and `config`; task R10 implemented `open`.

use clap::{Args, Parser, Subcommand};

/// Largest `--limit` accepted by `list`.
pub const LIST_LIMIT_MAX: usize = 10_000;
/// Largest `--count` accepted by `recent`.
pub const RECENT_COUNT_MAX: usize = 1_000;
/// Search result cap.
pub const SEARCH_LIMIT: usize = 50;

/// The complete parse result of `sessionatlas [COMMAND] ...`.
#[derive(Debug, Parser)]
#[command(
    name = "sessionatlas",
    version,
    about = "聚合多个 AI CLI 工具的项目与会话索引",
    propagate_version = true
)]
pub struct Cli {
    /// Omitting the command starts the interactive list.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Public command surface. Only `open` is a stub in this task; the rest are
/// wired to handlers.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// 扫描所有已安装的 AI CLI 工具，更新项目索引
    Scan(ScanArgs),
    /// 列出已索引的所有项目
    List(ListArgs),
    /// 模糊搜索项目
    Search(SearchArgs),
    /// 打开项目并启动指定 AI CLI 工具
    Open(OpenArgs),
    /// 查看最近会话记录
    Recent(RecentArgs),
    /// 管理配置和自定义工具
    Config(ConfigArgs),
}

/// Arguments for `scan`.
#[derive(Debug, Args)]
pub struct ScanArgs {
    /// 仅扫描指定工具，如 claude, codex, kimi, pi
    #[arg(long)]
    pub tool: Option<String>,
}

/// Arguments for `list`.
#[derive(Debug, Args)]
pub struct ListArgs {
    /// 只显示指定工具的项目
    #[arg(short = 't', long)]
    pub tool: Option<String>,
    /// 显示数量限制
    #[arg(short = 'l', long, default_value_t = 50, value_parser = parse_list_limit)]
    pub limit: usize,
    /// 交互式选择并打开
    #[arg(long)]
    pub interactive: bool,
}

/// Arguments for `search`.
#[derive(Debug, Args)]
pub struct SearchArgs {
    /// 搜索关键词
    pub query: String,
    /// 交互式选择并打开（为兼容旧 CLI，默认启用）
    #[arg(long, action = clap::ArgAction::SetTrue, default_value_t = true)]
    pub interactive: bool,
}

/// Arguments for `open`.
#[derive(Debug, Args)]
pub struct OpenArgs {
    /// 项目路径（支持模糊匹配，留空进入交互选择）
    pub project_path: Option<String>,
    /// 指定工具: claude, codex, kimi, opencode, aider, pi
    #[arg(short = 't', long)]
    pub tool: Option<String>,
    /// 交互式选择工具
    #[arg(long)]
    pub interactive: bool,
    /// 直接打开最近使用的项目
    #[arg(long)]
    pub recent: bool,
}

/// Arguments for `recent`.
#[derive(Debug, Args)]
pub struct RecentArgs {
    /// 显示数量
    #[arg(short = 'n', long, default_value_t = 10, value_parser = parse_recent_count)]
    pub count: usize,
    /// 交互式选择并打开
    #[arg(long)]
    pub open: bool,
}

/// Configuration action; `show` is the default when no action is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ConfigAction {
    /// 显示当前配置
    Show,
    /// 添加自定义工具
    AddTool,
    /// 设置默认终端
    SetDefaultTerminal,
}

/// Arguments for `config`.
#[derive(Debug, Args)]
pub struct ConfigArgs {
    /// show | add-tool | set-default-terminal
    #[arg(value_enum)]
    pub action: Option<ConfigAction>,
}

/// Clap value parser for `--limit`, accepting `1..=10000`.
pub fn parse_list_limit(value: &str) -> Result<usize, String> {
    match value.parse::<usize>() {
        Ok(limit) if (1..=LIST_LIMIT_MAX).contains(&limit) => Ok(limit),
        _ => Err("--limit 必须在 1 到 10000 之间。".to_string()),
    }
}

/// Clap value parser for `--count`, accepting `1..=1000`.
pub fn parse_recent_count(value: &str) -> Result<usize, String> {
    match value.parse::<usize>() {
        Ok(count) if (1..=RECENT_COUNT_MAX).contains(&count) => Ok(count),
        _ => Err("--count 必须在 1 到 1000 之间。".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn search_query_remains_required_and_interactive_by_default() {
        assert!(Cli::try_parse_from(["sessionatlas", "search"]).is_err());
        let parsed = Cli::try_parse_from(["sessionatlas", "search", "api"]).unwrap();
        let Some(Command::Search(args)) = parsed.command else {
            panic!("search command was not parsed");
        };
        assert_eq!(args.query, "api");
        assert!(args.interactive);
    }

    #[test]
    fn list_limit_parser_enforces_positive_bounded_range() {
        assert!(parse_list_limit("1").is_ok());
        assert!(parse_list_limit("10000").is_ok());
        assert!(parse_list_limit("0").is_err());
        assert!(parse_list_limit("10001").is_err());
        assert!(parse_list_limit("-5").is_err());
        assert!(parse_list_limit("abc").is_err());
        assert_eq!(
            parse_list_limit("0").unwrap_err(),
            "--limit 必须在 1 到 10000 之间。"
        );
    }

    #[test]
    fn recent_count_parser_enforces_positive_bounded_range() {
        assert!(parse_recent_count("1").is_ok());
        assert!(parse_recent_count("1000").is_ok());
        assert!(parse_recent_count("0").is_err());
        assert!(parse_recent_count("1001").is_err());
        assert!(parse_recent_count("xyz").is_err());
        assert_eq!(
            parse_recent_count("0").unwrap_err(),
            "--count 必须在 1 到 1000 之间。"
        );
    }
}
