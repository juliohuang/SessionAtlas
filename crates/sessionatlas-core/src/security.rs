//! Command and execution security: safe argv parsing, tool/session-id
//! validation, executable resolution, shell-meta / control-character
//! rejection.
//!
//! Implements the execution-security contract in
//! `docs/execution-security-contract.md`. Everything here is pure:
//! validators and quoting never touch the filesystem, never read the
//! environment, and never start a process, so they are testable on every host
//! and reusable by both the CLI and the Tauri console.
//!
//! Local processes are represented as program + argument array + working
//! directory. Shell text is constructed only where an interactive terminal
//! inherently requires it (Windows `cmd /K`, macOS `osascript`), and every
//! inserted value first passes its dedicated validator or lossless quoting
//! function.

use std::fmt;
use std::path::Path;

/// Maximum accepted tool-key length, mirroring `MaxToolKeyLength`.
pub const MAX_TOOL_KEY_LENGTH: usize = 64;
/// Maximum accepted session-id length, mirroring `MaxSessionIdLength`.
pub const MAX_SESSION_ID_LENGTH: usize = 512;
/// Maximum accepted display-label length, mirroring `CommandSecurity`.
pub const MAX_DISPLAY_LABEL_LENGTH: usize = 128;
/// Shell metacharacters that are never allowed inside a configured CLI command
/// or a built argument token, mirroring `ShellMetacharacters`.
pub const SHELL_METACHARACTERS: [char; 7] = ['&', '|', '<', '>', '^', '%', '!'];

/// Typed rejection reasons for command and identity validation. The variants
/// exist so tests can assert the exact failure without parsing messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityError {
    /// Tool key is blank, too long, or contains unsupported characters.
    InvalidToolKey,
    /// Session ID is blank, too long, option-shaped, or unsupported.
    InvalidSessionId,
    /// Display label is blank, too long, or contains control characters.
    InvalidDisplayLabel,
    /// A CLI command was blank/whitespace-only.
    BlankCommand,
    /// A CLI command contained control characters or shell metacharacters.
    CommandControlCharacters,
    /// A CLI command contained unbalanced double quotes.
    UnclosedQuote,
    /// The parsed executable token was empty.
    EmptyExecutable,
    /// The parsed executable token started with `-`.
    OptionLikeExecutable,
    /// The parsed executable is a shell or a script wrapper.
    ShellWrapper,
    /// A command argument contained a NUL byte.
    NulByte,
    /// A command argument was empty or contained unsupported shell characters.
    UnsupportedShellCharacters,
}

impl fmt::Display for SecurityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            SecurityError::InvalidToolKey => "工具标识包含不支持的字符",
            SecurityError::InvalidSessionId => "会话 ID 包含不支持的字符",
            SecurityError::InvalidDisplayLabel => "显示名称包含不支持的字符",
            SecurityError::BlankCommand => "CLI 命令不能为空",
            SecurityError::CommandControlCharacters => "CLI 命令不能包含 shell 控制字符",
            SecurityError::UnclosedQuote => "CLI 命令包含未闭合的双引号",
            SecurityError::EmptyExecutable => "CLI 命令的可执行文件不能为空",
            SecurityError::OptionLikeExecutable => "CLI 命令的可执行文件不能是选项",
            SecurityError::ShellWrapper => "CLI 命令不能直接调用 shell 或脚本包装器",
            SecurityError::NulByte => "命令参数包含 NUL 字节",
            SecurityError::UnsupportedShellCharacters => "命令参数包含不支持的 shell 字符",
        };
        f.write_str(message)
    }
}

impl std::error::Error for SecurityError {}

/// Validates a tool key, mirroring `CommandSecurity.ValidateToolKey`: trimmed,
/// `1..=64` ASCII-safe characters, first character alphanumeric, the rest
/// alphanumeric or `.` `_` `+` `-`. Leading `-` (option-shaped) and any
/// control character are rejected. Returns the trimmed key on success.
pub fn validate_tool_key(value: &str) -> Result<String, SecurityError> {
    let trimmed = value.trim();
    let mut characters = trimmed.chars();
    let Some(first) = characters.next() else {
        return Err(SecurityError::InvalidToolKey);
    };
    if trimmed.chars().count() > MAX_TOOL_KEY_LENGTH || !first.is_ascii_alphanumeric() {
        return Err(SecurityError::InvalidToolKey);
    }
    if characters.any(|character| {
        !character.is_ascii_alphanumeric()
            && character != '.'
            && character != '_'
            && character != '+'
            && character != '-'
    }) {
        return Err(SecurityError::InvalidToolKey);
    }
    Ok(trimmed.to_string())
}

/// Validates a session ID, mirroring `CommandSecurity.ValidateSessionId`:
/// trimmed, `1..=512` ASCII-safe characters chosen from alphanumerics and
/// `.` `_` `:` `+` `-`. A leading `-` is rejected so an ID can never be
/// mistaken for an option when the trusted backend appends tool-specific
/// resume arguments.
/// Returns the trimmed ID on success.
pub fn validate_session_id(value: &str) -> Result<String, SecurityError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_SESSION_ID_LENGTH
        || trimmed.starts_with('-')
    {
        return Err(SecurityError::InvalidSessionId);
    }
    if trimmed.chars().any(|character| {
        !character.is_ascii_alphanumeric()
            && character != '.'
            && character != '_'
            && character != ':'
            && character != '+'
            && character != '-'
    }) {
        return Err(SecurityError::InvalidSessionId);
    }
    Ok(trimmed.to_string())
}

/// Validates a display label, mirroring `CommandSecurity.ValidateDisplayLabel`:
/// trimmed, `1..=128` characters, no control characters. Returns the trimmed
/// label on success.
pub fn validate_display_label(value: &str) -> Result<String, SecurityError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_DISPLAY_LABEL_LENGTH {
        return Err(SecurityError::InvalidDisplayLabel);
    }
    if trimmed.chars().any(char::is_control) {
        return Err(SecurityError::InvalidDisplayLabel);
    }
    Ok(trimmed.to_string())
}

/// Parses a configured CLI command into argv tokens, mirroring
/// `CommandSecurity.ParseSafeCommand`. Rejects blank commands, control
/// characters, shell metacharacters, unbalanced double quotes, empty
/// executable tokens, option-like executables and shell/script wrappers.
pub fn parse_safe_command(command: &str) -> Result<Vec<String>, SecurityError> {
    if command.trim().is_empty() {
        return Err(SecurityError::BlankCommand);
    }
    if command
        .chars()
        .any(|character| character.is_control() || SHELL_METACHARACTERS.contains(&character))
    {
        return Err(SecurityError::CommandControlCharacters);
    }

    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for character in command.chars() {
        if character == '"' {
            quoted = !quoted;
            started = true;
        } else if character.is_whitespace() && !quoted {
            if !started {
                continue;
            }
            tokens.push(std::mem::take(&mut current));
            started = false;
        } else {
            current.push(character);
            started = true;
        }
    }
    if quoted {
        return Err(SecurityError::UnclosedQuote);
    }
    if started {
        tokens.push(current);
    }

    if tokens.iter().any(String::is_empty) {
        return Err(SecurityError::EmptyExecutable);
    }
    if tokens[0].starts_with('-') {
        return Err(SecurityError::OptionLikeExecutable);
    }
    if is_shell_program(&tokens[0]) {
        return Err(SecurityError::ShellWrapper);
    }
    Ok(tokens)
}

/// Builds a Windows command-line string by wrapping every argument in double
/// quotes, mirroring `CommandSecurity.BuildWindowsCommand`. All tokens must
/// pass [`ensure_safe_tokens`].
pub fn build_windows_command(arguments: &[String]) -> Result<String, SecurityError> {
    ensure_safe_tokens(arguments)?;
    Ok(arguments
        .iter()
        .map(|token| format!("\"{token}\""))
        .collect::<Vec<_>>()
        .join(" "))
}

/// Builds a POSIX command line with every argument POSIX-single-quoted,
/// mirroring `CommandSecurity.BuildPosixCommand`. All tokens must pass
/// [`ensure_safe_tokens`].
pub fn build_posix_command(arguments: &[String]) -> Result<String, SecurityError> {
    ensure_safe_tokens(arguments)?;
    let mut out = String::new();
    for (index, token) in arguments.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push_str(&quote_posix(token)?);
    }
    Ok(out)
}

/// POSIX single-quote escaping that preserves apostrophes, mirroring
/// `CommandSecurity.QuotePosix`: `'` becomes `'"'"'`. Only NUL is rejected —
/// anything else, including shell punctuation, becomes literal inside the
/// quotes. This is used for values (project paths) that may legitimately
/// contain shell punctuation.
pub fn quote_posix(value: &str) -> Result<String, SecurityError> {
    if value.contains('\0') {
        return Err(SecurityError::NulByte);
    }
    Ok(format!("'{}'", value.replace('\'', "'\"'\"'")))
}

/// Rejects any token that could break out of an embedded shell command: empty
/// tokens, double quotes, control characters and shell metacharacters.
fn ensure_safe_tokens(arguments: &[String]) -> Result<(), SecurityError> {
    for token in arguments {
        if token.is_empty()
            || token.contains('"')
            || token.chars().any(|character| {
                character.is_control() || SHELL_METACHARACTERS.contains(&character)
            })
        {
            return Err(SecurityError::UnsupportedShellCharacters);
        }
    }
    Ok(())
}

/// Shell and script-wrapper detection mirroring `CommandSecurity.IsShellProgram`:
/// a `.cmd`/`.bat`/`.ps1` extension or a known interactive shell executable is
/// never an acceptable CLI command.
pub fn is_shell_program(program: &str) -> bool {
    let file_name = program
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_lowercase();
    let extension = Path::new(&file_name)
        .extension()
        .and_then(|extension| extension.to_str());
    if matches!(extension, Some("cmd") | Some("bat") | Some("ps1")) {
        return true;
    }
    matches!(
        file_name.as_str(),
        "cmd"
            | "cmd.exe"
            | "powershell"
            | "powershell.exe"
            | "pwsh"
            | "pwsh.exe"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "wsl"
            | "wsl.exe"
            | "osascript"
    )
}
