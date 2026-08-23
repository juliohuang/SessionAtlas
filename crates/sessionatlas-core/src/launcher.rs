//! Cross-platform terminal launcher backing `sessionatlas open`.
//!
//! Mirrors `Core/Launcher/CliLauncher.cs`. The launcher resolves a tool key to
//! a validated argv (built-ins plus enabled custom tools that never override a
//! built-in), verifies the project directory exists, picks a platform terminal
//! through injectable probes, and hands a [`ProcessSpec`] to an injectable
//! [`ProcessRunner`]. No shell text is built from project paths or unvalidated
//! scanner metadata; only the interactive-terminal cases (Windows `cmd /K`,
//! macOS `osascript`) construct shell text from validated tokens.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::config::AppConfig;
use crate::process::{ProcessError, ProcessRunner, ProcessSpec, ProgramResolver};
use crate::security;

/// The six built-in tool keys. A custom tool
/// whose key collides with one of these (case-insensitive) is never allowed to
/// override a built-in identity.
pub const BUILT_IN_TOOL_KEYS: [&str; 6] = ["claude", "kimi", "codex", "opencode", "aider", "pi"];

/// Linux terminals probed in order, mirroring the C# launcher.
const LINUX_TERMINALS: [&str; 6] = [
    "gnome-terminal",
    "konsole",
    "xfce4-terminal",
    "alacritty",
    "kitty",
    "xterm",
];

/// AppleScript that opens a new macOS Terminal window and runs the single
/// argument in it, mirroring `CliLauncher.LaunchOnMac`.
const MAC_OS_SCRIPT: &str = r#"on run argv
  tell application "Terminal"
    activate
    do script (item 1 of argv)
  end tell
end run"#;

/// Typed failure modes of tool resolution and terminal launching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherError {
    /// The project directory does not exist.
    ProjectDirectoryMissing(String),
    /// The tool key is not a configured, launchable tool.
    UnknownToolKey(String),
    /// The tool key failed validation.
    InvalidToolKey(String),
    /// The configured CLI command failed safety parsing.
    InvalidCommand(String),
    /// The session ID failed validation.
    InvalidSessionId(String),
    /// No usable terminal was found on Linux.
    NoTerminal,
    /// The terminal process could not be started.
    StartFailed(ProcessError),
}

impl fmt::Display for LauncherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LauncherError::ProjectDirectoryMissing(path) => {
                write!(f, "项目目录不存在: {path}")
            }
            LauncherError::UnknownToolKey(key) => write!(f, "未配置可启动的工具: {key}"),
            LauncherError::InvalidToolKey(_) => f.write_str("工具标识包含不支持的字符"),
            LauncherError::InvalidCommand(detail) => f.write_str(detail),
            LauncherError::InvalidSessionId(_) => f.write_str("会话 ID 包含不支持的字符"),
            LauncherError::NoTerminal => {
                write!(f, "无法找到图形终端。请在项目目录中手动运行对应 CLI 命令。")
            }
            LauncherError::StartFailed(error) => write!(f, "启动失败: {error}"),
        }
    }
}

impl std::error::Error for LauncherError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LauncherError::StartFailed(error) => Some(error),
            _ => None,
        }
    }
}

/// Whether `key` collides with a built-in identity, compared
/// ASCII-case-insensitively. Custom tools may never override built-ins.
pub fn is_reserved_tool_key(key: &str) -> bool {
    BUILT_IN_TOOL_KEYS
        .iter()
        .any(|built_in| built_in.eq_ignore_ascii_case(key))
}

/// Launchable tool identities and their CLI command text. Keys are matched
/// ASCII-case-insensitively; lookups go through a lowercased map.
pub struct ToolCommands {
    commands: HashMap<String, String>,
}

impl ToolCommands {
    /// The six built-in tools, each invoking its own binary.
    pub fn built_in() -> Self {
        let mut commands = HashMap::new();
        for key in BUILT_IN_TOOL_KEYS {
            commands.insert(key.to_string(), key.to_string());
        }
        Self { commands }
    }

    /// Built-ins plus every custom tool that is enabled, has a valid key, has
    /// a safely parseable CLI command, and does not override a built-in key.
    /// Invalid or hand-edited entries are ignored, mirroring
    /// `CliLauncher`'s constructor.
    pub fn from_config(config: &AppConfig) -> Self {
        let mut commands = ToolCommands::built_in();
        for tool in &config.custom_tools {
            if !tool.is_enabled || tool.key.trim().is_empty() || tool.cli_command.trim().is_empty()
            {
                continue;
            }
            let Ok(key) = security::validate_tool_key(&tool.key) else {
                continue;
            };
            if security::parse_safe_command(&tool.cli_command).is_err() {
                continue;
            }
            if is_reserved_tool_key(&key) {
                continue;
            }
            commands
                .commands
                .insert(key.to_lowercase(), tool.cli_command.clone());
        }
        commands
    }

    /// Whether `key` resolves to a configured tool command.
    pub fn known_key(&self, key: &str) -> bool {
        self.lookup(key).is_some()
    }

    /// Whether the tool's executable can currently be found through the
    /// injectable resolver. Unknown or invalid tools are never available.
    pub fn is_tool_available(&self, key: &str, resolver: &dyn ProgramResolver) -> bool {
        let Some(command_text) = self.lookup(key) else {
            return false;
        };
        match security::parse_safe_command(command_text) {
            Ok(tokens) if !tokens.is_empty() => resolver.is_on_path(&tokens[0]),
            _ => false,
        }
    }

    /// Builds the validated tool argv. `--resume` and the validated session ID
    /// are appended as separate independent arguments by trusted code.
    pub fn build_arguments(
        &self,
        key: &str,
        session_id: Option<&str>,
    ) -> Result<Vec<String>, LauncherError> {
        let validated_key = security::validate_tool_key(key)
            .map_err(|_| LauncherError::InvalidToolKey(key.to_string()))?;
        let Some(command_text) = self.lookup(&validated_key) else {
            return Err(LauncherError::UnknownToolKey(validated_key));
        };
        let mut arguments = security::parse_safe_command(command_text)
            .map_err(|error| LauncherError::InvalidCommand(error.to_string()))?;
        if let Some(id) = session_id.filter(|id| !id.trim().is_empty()) {
            let validated_id = security::validate_session_id(id)
                .map_err(|_| LauncherError::InvalidSessionId(id.to_string()))?;
            arguments.push(if validated_key.eq_ignore_ascii_case("pi") {
                "--session".to_string()
            } else {
                "--resume".to_string()
            });
            arguments.push(validated_id);
        }
        Ok(arguments)
    }

    fn lookup(&self, key: &str) -> Option<&str> {
        self.commands.get(&key.to_lowercase()).map(String::as_str)
    }
}

/// Interactive terminal platform, made explicit so every platform shape can be
/// asserted on any host (mirroring `path::PathFlavor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalPlatform {
    Windows,
    MacOs,
    Linux,
}

/// The terminal platform this process runs on.
pub fn native_terminal_platform() -> TerminalPlatform {
    if cfg!(target_os = "macos") {
        TerminalPlatform::MacOs
    } else if cfg!(windows) {
        TerminalPlatform::Windows
    } else {
        TerminalPlatform::Linux
    }
}

/// The well-known Windows Terminal executable under `%LOCALAPPDATA%`, checked
/// for existence before preferring `wt.exe` over the `cmd.exe` fallback.
pub fn default_wt_path() -> PathBuf {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(dirs::data_local_dir)
        .unwrap_or_default();
    local_app_data
        .join("Microsoft")
        .join("WindowsApps")
        .join("wt.exe")
}

/// Production Windows Terminal probe: the executable file exists.
pub fn default_wt_probe(path: &Path) -> bool {
    path.is_file()
}

/// Builds the terminal [`ProcessSpec`] for a platform, purely and without any
/// I/O, so tests can assert the exact program/argument/working-directory shape
/// of every platform on any host.
///
/// `windows_wt_path` is the resolved `wt.exe` path when it should be used; when
/// `None` the Windows shape falls back to `cmd.exe /D /K`. `linux_terminal` is
/// the probed terminal program; `None` is a hard error (never a fake success).
pub fn build_process_spec(
    platform: TerminalPlatform,
    project_path: &str,
    tool_arguments: &[String],
    windows_wt_path: Option<&str>,
    linux_terminal: Option<&str>,
) -> Result<ProcessSpec, LauncherError> {
    let working_directory = project_path.to_string();
    match platform {
        TerminalPlatform::Windows => {
            let command = security::build_windows_command(tool_arguments)
                .map_err(|error| LauncherError::InvalidCommand(error.to_string()))?;
            match windows_wt_path {
                Some(wt) => Ok(ProcessSpec {
                    program: wt.to_string(),
                    arguments: vec![
                        "-d".to_string(),
                        project_path.to_string(),
                        "cmd.exe".to_string(),
                        "/D".to_string(),
                        "/K".to_string(),
                        command,
                    ],
                    working_directory,
                }),
                None => Ok(ProcessSpec {
                    program: "cmd.exe".to_string(),
                    arguments: vec!["/D".to_string(), "/K".to_string(), command],
                    working_directory,
                }),
            }
        }
        TerminalPlatform::MacOs => {
            let cd = security::quote_posix(project_path)
                .map_err(|error| LauncherError::InvalidCommand(error.to_string()))?;
            let tool_command = security::build_posix_command(tool_arguments)
                .map_err(|error| LauncherError::InvalidCommand(error.to_string()))?;
            let command = format!("cd {cd} && exec {tool_command}");
            Ok(ProcessSpec {
                program: "osascript".to_string(),
                arguments: vec![
                    "-e".to_string(),
                    MAC_OS_SCRIPT.to_string(),
                    "--".to_string(),
                    command,
                ],
                working_directory,
            })
        }
        TerminalPlatform::Linux => {
            let Some(terminal) = linux_terminal else {
                return Err(LauncherError::NoTerminal);
            };
            let separator = if terminal == "gnome-terminal" {
                "--"
            } else {
                "-e"
            };
            let mut arguments = Vec::with_capacity(tool_arguments.len() + 1);
            arguments.push(separator.to_string());
            arguments.extend(tool_arguments.iter().cloned());
            Ok(ProcessSpec {
                program: terminal.to_string(),
                arguments,
                working_directory,
            })
        }
    }
}

/// Launcher with every external interaction injected: process starting, PATH
/// probing, and Windows Terminal detection. Production wires
/// [`crate::process::SystemProcessRunner`], [`crate::process::PathProgramResolver`]
/// and [`default_wt_probe`]; tests inject recording and fake implementations.
pub struct Launcher<'a> {
    commands: ToolCommands,
    resolver: &'a dyn ProgramResolver,
    runner: &'a dyn ProcessRunner,
    wt_probe: &'a dyn Fn(&Path) -> bool,
}

impl<'a> Launcher<'a> {
    pub fn new(
        commands: ToolCommands,
        resolver: &'a dyn ProgramResolver,
        runner: &'a dyn ProcessRunner,
        wt_probe: &'a dyn Fn(&Path) -> bool,
    ) -> Self {
        Self {
            commands,
            resolver,
            runner,
            wt_probe,
        }
    }

    /// Whether the tool's executable is currently launchable, using the
    /// injected resolver.
    pub fn is_tool_available(&self, key: &str) -> bool {
        self.commands.is_tool_available(key, self.resolver)
    }

    /// Resolves the validated tool argv, appending `--resume <sessionId>` as
    /// independent arguments when a session ID is supplied.
    pub fn build_arguments(
        &self,
        key: &str,
        session_id: Option<&str>,
    ) -> Result<Vec<String>, LauncherError> {
        self.commands.build_arguments(key, session_id)
    }

    /// Launches the tool in a terminal for `project_path`. The directory must
    /// exist, the tool must resolve, a terminal must be available, and the
    /// runner must accept the start — any failure is an error, never a fake
    /// success. On success the started [`ProcessSpec`] is returned so callers
    /// can record a session only after the launch was accepted.
    pub fn launch(
        &self,
        project_path: &str,
        tool_key: &str,
        session_id: Option<&str>,
    ) -> Result<ProcessSpec, LauncherError> {
        if !Path::new(project_path).is_dir() {
            return Err(LauncherError::ProjectDirectoryMissing(
                project_path.to_string(),
            ));
        }
        let tool_arguments = self.build_arguments(tool_key, session_id)?;
        let platform = native_terminal_platform();
        let windows_wt_path = if platform == TerminalPlatform::Windows {
            let path = default_wt_path();
            if (self.wt_probe)(&path) {
                Some(path.to_string_lossy().into_owned())
            } else {
                None
            }
        } else {
            None
        };
        let linux_terminal = if platform == TerminalPlatform::Linux {
            probe_linux_terminal(self.resolver)
        } else {
            None
        };
        let spec = build_process_spec(
            platform,
            project_path,
            &tool_arguments,
            windows_wt_path.as_deref(),
            linux_terminal.as_deref(),
        )?;
        self.runner
            .start(&spec)
            .map_err(LauncherError::StartFailed)?;
        Ok(spec)
    }
}

/// Picks the first Linux terminal resolvable through the injectable resolver.
fn probe_linux_terminal(resolver: &dyn ProgramResolver) -> Option<String> {
    LINUX_TERMINALS
        .iter()
        .copied()
        .find(|terminal| resolver.is_on_path(terminal))
        .map(str::to_string)
}
