//! Built-in tool identities and input validation shared by `scan` and `config`.
//!
//! Task R09 owns the `scan`/`config` surface but not `sessionatlas-core`, whose
//! `security` module is reserved for R10. The six built-in keys and the
//! value validators therefore live here, mirroring
//! `Core/Process/CommandSecurity.cs` and the `reservedKeys` list in
//! `CLI/Commands/ConfigCommand.cs` so `config add-tool` and the custom-tool
//! conflict filtering in `scan` agree on exactly one set of identities.

/// Shell metacharacters that are never allowed inside a configured CLI command.
const SHELL_METACHARACTERS: [char; 7] = ['&', '|', '<', '>', '^', '%', '!'];

/// The six built-in tool keys. A custom tool
/// whose key collides with one of these (case-insensitive) is rejected by
/// `config add-tool` and skipped by `scan`'s custom-tool loading.
pub const BUILT_IN_TOOL_KEYS: [&str; 6] = ["claude", "kimi", "codex", "opencode", "aider", "pi"];

/// Whether `key` is a reserved built-in key, compared ASCII-case-insensitively.
pub fn is_reserved_tool_key(key: &str) -> bool {
    BUILT_IN_TOOL_KEYS
        .iter()
        .any(|built_in| built_in.eq_ignore_ascii_case(key))
}

/// Validates a custom-tool key, mirroring `CommandSecurity.ValidateToolKey`:
/// trimmed, 1..=64 ASCII-safe characters, first character alphanumeric, the
/// rest alphanumeric or `.` `_` `+` `-`. Returns the trimmed key on success.
pub fn validate_tool_key(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    let mut characters = trimmed.chars();
    let Some(first) = characters.next() else {
        return Err("工具标识包含不支持的字符".to_string());
    };
    if trimmed.chars().count() > 64 || !first.is_ascii_alphanumeric() {
        return Err("工具标识包含不支持的字符".to_string());
    }
    if characters.any(|character| {
        !character.is_ascii_alphanumeric()
            && character != '.'
            && character != '_'
            && character != '+'
            && character != '-'
    }) {
        return Err("工具标识包含不支持的字符".to_string());
    }
    Ok(trimmed.to_string())
}

/// Validates a display label, mirroring `CommandSecurity.ValidateDisplayLabel`:
/// trimmed, 1..=128 characters, no control characters. Returns the trimmed
/// label on success.
pub fn validate_display_label(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 128 {
        return Err("显示名称包含不支持的字符".to_string());
    }
    if trimmed.chars().any(char::is_control) {
        return Err("显示名称包含不支持的字符".to_string());
    }
    Ok(trimmed.to_string())
}

/// Parses a configured CLI command into argv tokens, mirroring
/// `CommandSecurity.ParseSafeCommand`. Rejects blank commands, NUL/newlines,
/// shell metacharacters, unbalanced double quotes, option-like executables and
/// shell/script wrappers. The tokens are not used to launch anything in this
/// task; the parse is the validation gate `config add-tool` relies on.
pub fn parse_safe_command(command: &str) -> Result<Vec<String>, String> {
    if command.trim().is_empty() {
        return Err("CLI 命令不能为空".to_string());
    }
    if command.chars().any(|character| {
        character == '\0'
            || character == '\r'
            || character == '\n'
            || SHELL_METACHARACTERS.contains(&character)
    }) {
        return Err("CLI 命令不能包含 shell 控制字符".to_string());
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
        return Err("CLI 命令包含未闭合的双引号".to_string());
    }
    if started {
        tokens.push(current);
    }

    if tokens.is_empty()
        || tokens[0].is_empty()
        || tokens[0].starts_with('-')
        || is_shell_program(&tokens[0])
    {
        return Err("CLI 命令不能直接调用 shell 或脚本包装器".to_string());
    }
    Ok(tokens)
}

/// Shell and script-wrapper detection mirroring
/// `CommandSecurity.IsShellProgram`: a `.cmd`/`.bat`/`.ps1` extension or a
/// known interactive shell executable is never an acceptable CLI command.
fn is_shell_program(program: &str) -> bool {
    let file_name = program
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_lowercase();
    let extension = std::path::Path::new(&file_name)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_keys_are_the_six_supported_identities() {
        let mut keys: Vec<&str> = BUILT_IN_TOOL_KEYS.to_vec();
        keys.sort_unstable();
        assert_eq!(keys, ["aider", "claude", "codex", "kimi", "opencode", "pi"]);
        for key in BUILT_IN_TOOL_KEYS {
            assert!(is_reserved_tool_key(key), "{key} must be reserved");
            assert!(
                is_reserved_tool_key(&key.to_uppercase()),
                "reservation is case-insensitive"
            );
        }
        assert!(!is_reserved_tool_key("my-custom-agent"));
        assert!(!is_reserved_tool_key(""));
    }

    #[test]
    fn tool_key_validation_accepts_valid_keys_and_rejects_bad_shapes() {
        assert_eq!(validate_tool_key(" my-tool.1 ").unwrap(), "my-tool.1");
        assert!(validate_tool_key("cli").is_ok());
        assert!(validate_tool_key("a").is_ok());
        let long: String = "k".repeat(65);
        assert!(validate_tool_key(&long).is_err());
        assert!(validate_tool_key("").is_err());
        assert!(validate_tool_key("   ").is_err());
        assert!(validate_tool_key("-cli").is_err());
        assert!(validate_tool_key("a b").is_err());
        assert!(validate_tool_key("a;b").is_err());
        assert!(validate_tool_key("a\u{0007}b").is_err());
        assert!(validate_tool_key("é").is_err());
    }

    #[test]
    fn display_label_validation_trims_and_rejects_control_characters() {
        assert_eq!(validate_display_label("  My Agent  ").unwrap(), "My Agent");
        assert!(validate_display_label("ok-名字").is_ok());
        assert!(validate_display_label("").is_err());
        assert!(validate_display_label("   ").is_err());
        assert!(validate_display_label("bad\u{0000}label").is_err());
        assert!(validate_display_label("bad\nlabel").is_err());
        let long: String = "x".repeat(129);
        assert!(validate_display_label(&long).is_err());
    }

    #[test]
    fn safe_command_accepts_program_with_quoted_arguments() {
        assert_eq!(
            parse_safe_command("claude").unwrap(),
            vec!["claude".to_string()]
        );
        assert_eq!(
            parse_safe_command("mycli arg \"two words\"").unwrap(),
            vec![
                "mycli".to_string(),
                "arg".to_string(),
                "two words".to_string()
            ]
        );
    }

    #[test]
    fn safe_command_rejects_blank_control_and_shell_metacharacters() {
        for bad in [
            "",
            "   ",
            "a\u{0000}b",
            "a\rb",
            "a\nb",
            "a&b",
            "a|b",
            "a<b",
            "a>b",
            "a^b",
            "a%b",
            "a!b",
        ] {
            assert!(parse_safe_command(bad).is_err(), "must reject {bad:?}");
        }
    }

    #[test]
    fn safe_command_rejects_unbalanced_quotes_and_option_executables() {
        assert!(parse_safe_command("\"unclosed").is_err());
        assert!(parse_safe_command("\"\"").is_err());
        assert!(parse_safe_command("\"\" argument").is_err());
        assert!(parse_safe_command("-flag").is_err());
    }

    #[test]
    fn safe_command_rejects_shells_and_script_wrappers() {
        for bad in [
            "cmd",
            "cmd.exe",
            "powershell",
            "powershell.exe",
            "pwsh",
            "pwsh.exe",
            "sh",
            "bash",
            "zsh",
            "fish",
            "wsl",
            "wsl.exe",
            "osascript",
            "C:\\Windows\\System32\\cmd.exe",
            "run.bat",
            "deploy.cmd",
            "setup.ps1",
        ] {
            assert!(parse_safe_command(bad).is_err(), "must reject {bad:?}");
        }
        assert!(parse_safe_command("/usr/bin/bash -c").is_err());
    }
}
