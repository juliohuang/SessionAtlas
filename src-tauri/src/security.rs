use std::net::IpAddr;
use std::path::Path;

const MAX_TOOL_KEY_LEN: usize = 64;
const MAX_SESSION_ID_LEN: usize = 512;
const MAX_SSH_USER_LEN: usize = 64;
const MAX_SSH_HOST_LEN: usize = 253;
const SHELL_METACHARACTERS: &[char] = &['&', '|', '<', '>', '^', '%', '!', '$', '`'];

pub(crate) fn validate_tool_key(value: &str) -> Result<&str, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_TOOL_KEY_LEN {
        return Err("invalid tool key length".to_string());
    }
    let mut chars = value.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
    {
        return Err("tool key contains unsupported characters".to_string());
    }
    Ok(value)
}

pub(crate) fn validate_session_id(value: &str) -> Result<&str, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_SESSION_ID_LEN {
        return Err("invalid session id length".to_string());
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '+' | '-'))
    {
        return Err("session id contains unsupported characters".to_string());
    }
    Ok(value)
}

#[cfg(test)]
pub(crate) fn build_tool_launch_input(
    tool_key: Option<&str>,
    session_id: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(tool_key) = tool_key else {
        if session_id.is_some_and(|value| !value.trim().is_empty()) {
            return Err("session id requires a tool key".to_string());
        }
        return Ok(None);
    };

    let args = tool_launch_argv(tool_key, session_id)?;
    Ok(Some(build_argv_launch_input(&args)?))
}

pub(crate) fn tool_launch_argv(
    tool_key: &str,
    session_id: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut args = vec![validate_tool_key(tool_key)?.to_string()];
    if let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) {
        args.push("--resume".to_string());
        args.push(validate_session_id(session_id)?.to_string());
    }
    Ok(args)
}

pub(crate) fn validate_cli_argv(args: &[String]) -> Result<(), String> {
    let Some(program) = args.first() else {
        return Err("CLI command is empty".to_string());
    };
    if program.starts_with('-') || is_shell_program(program) {
        return Err("CLI command cannot invoke a shell or script wrapper".to_string());
    }
    for argument in args {
        if argument.is_empty()
            || argument.contains('"')
            || argument
                .chars()
                .any(|c| c.is_control() || SHELL_METACHARACTERS.contains(&c))
        {
            return Err("CLI command contains unsupported shell characters".to_string());
        }
    }
    Ok(())
}

pub(crate) fn render_shell_command(args: &[String]) -> Result<String, String> {
    validate_cli_argv(args)?;
    args.iter()
        .map(|argument| {
            if shell_token_is_plain(argument) {
                return Ok(argument.clone());
            }
            #[cfg(windows)]
            {
                Ok(format!("\"{argument}\""))
            }
            #[cfg(not(windows))]
            {
                posix_shell_quote(argument)
            }
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|arguments| arguments.join(" "))
}

pub(crate) fn build_argv_launch_input(args: &[String]) -> Result<String, String> {
    let mut command = render_shell_command(args)?;
    command.push('\r');
    Ok(command)
}

fn shell_token_is_plain(value: &str) -> bool {
    value.chars().all(|character| {
        character.is_ascii_alphanumeric()
            || matches!(character, '.' | '_' | '+' | '-' | '/' | ':' | '@' | ',')
            || cfg!(windows) && character == '\\'
    })
}

pub(crate) fn validate_ssh_user(value: &str) -> Result<&str, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_SSH_USER_LEN || value.starts_with('-') {
        return Err("invalid SSH user".to_string());
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err("SSH user contains unsupported characters".to_string());
    }
    Ok(value)
}

pub(crate) fn validate_ssh_host(value: &str) -> Result<&str, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_SSH_HOST_LEN
        || value.starts_with('-')
        || value.chars().any(char::is_whitespace)
    {
        return Err("invalid SSH host".to_string());
    }

    let unbracketed = value
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(value);
    if unbracketed.parse::<IpAddr>().is_ok() {
        return Ok(value);
    }

    let valid_dns_name = value.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric())
            && label
                .chars()
                .last()
                .is_some_and(|c| c.is_ascii_alphanumeric())
            && label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    });
    if !valid_dns_name {
        return Err("SSH host contains unsupported characters".to_string());
    }
    Ok(value)
}

pub(crate) fn ssh_destination(user: &str, host: &str) -> Result<String, String> {
    Ok(format!(
        "{}@{}",
        validate_ssh_user(user)?,
        validate_ssh_host(host)?
    ))
}

pub(crate) fn validate_display_label(value: &str) -> Result<&str, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err("invalid display label".to_string());
    }
    Ok(value)
}

pub(crate) fn posix_shell_quote(value: &str) -> Result<String, String> {
    if value.contains('\0') {
        return Err("value contains a NUL byte".to_string());
    }
    Ok(format!("'{}'", value.replace('\'', "'\"'\"'")))
}

/// Quote a remote path while preserving expansion for `~` and `~/...`.
/// Named-user tildes are intentionally rejected because an unquoted username
/// would otherwise become part of shell syntax.
pub(crate) fn quote_remote_path(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("empty remote path".to_string());
    }
    if value.chars().any(char::is_control) {
        return Err("remote path contains a control character".to_string());
    }
    if value == "~" {
        return Ok("~".to_string());
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return Ok(format!("~/{}", posix_shell_quote(rest)?));
    }
    if value.starts_with('~') {
        return Err("named-user home paths are not supported".to_string());
    }
    posix_shell_quote(value)
}

pub(crate) fn parse_command_template(template: &str) -> Result<Vec<String>, String> {
    if template.chars().any(char::is_control) {
        return Err("command template contains a control character".to_string());
    }

    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for character in template.chars() {
        match character {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return Err("command template contains an unmatched quote".to_string());
    }
    if started {
        args.push(current);
    }
    if args.is_empty() {
        return Err("empty command template".to_string());
    }
    Ok(args)
}

pub(crate) fn is_shell_program(program: &str) -> bool {
    let file_name = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    if matches!(
        Path::new(&file_name)
            .extension()
            .and_then(|value| value.to_str()),
        Some("cmd" | "bat" | "ps1")
    ) {
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

pub(crate) fn validate_external_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.chars().any(char::is_control) {
        return Err("URL contains a control character".to_string());
    }
    let parsed = url::Url::parse(value).map_err(|_| "invalid URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("only absolute http(s) URLs are allowed".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URLs containing credentials are not allowed".to_string());
    }
    Ok(parsed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_launch_input_rejects_shell_syntax() {
        assert_eq!(
            build_tool_launch_input(Some("codex"), Some("session-123")).unwrap(),
            Some("codex --resume session-123\r".to_string())
        );
        assert!(build_tool_launch_input(Some("codex & calc"), None).is_err());
        assert!(build_tool_launch_input(Some("codex"), Some("ok\rwhoami")).is_err());
    }

    #[test]
    fn configured_cli_argv_quotes_spaces_without_accepting_shells() {
        let input = build_argv_launch_input(&[
            "fixture-cli".to_string(),
            "--profile".to_string(),
            "safe profile".to_string(),
        ])
        .unwrap();
        assert!(input.starts_with("fixture-cli --profile "));
        assert!(input.contains("safe profile"));
        assert!(input.ends_with('\r'));

        assert!(build_argv_launch_input(&["cmd.exe".to_string(), "/C".to_string()]).is_err());
        assert!(
            build_argv_launch_input(&["fixture-cli".to_string(), "safe & calc".to_string()])
                .is_err()
        );
    }

    #[test]
    fn ssh_identity_fields_reject_option_and_command_injection() {
        assert_eq!(
            ssh_destination("demo-user", "example.test").unwrap(),
            "demo-user@example.test"
        );
        assert!(ssh_destination("-oProxyCommand=calc", "example.test").is_err());
        assert!(ssh_destination("demo", "host; touch /tmp/pwned").is_err());
        assert!(validate_ssh_host("[::1]").is_ok());
    }

    #[test]
    fn remote_paths_preserve_apostrophes_without_creating_shell_syntax() {
        assert_eq!(
            quote_remote_path("/srv/alice's repo").unwrap(),
            "'/srv/alice'\"'\"'s repo'"
        );
        assert_eq!(quote_remote_path("~/projects").unwrap(), "~/'projects'");
        assert!(quote_remote_path("~root/projects").is_err());
        assert!(quote_remote_path("/srv/repo\nnext").is_err());
    }

    #[test]
    fn command_templates_require_balanced_quotes() {
        assert_eq!(
            parse_command_template("code \"{path}\"").unwrap(),
            vec!["code", "{path}"]
        );
        assert!(parse_command_template("code \"{path}").is_err());
        assert!(parse_command_template("code\n{path}").is_err());
    }

    #[test]
    fn shell_program_detection_handles_absolute_paths() {
        assert!(is_shell_program(r"C:\Windows\System32\cmd.exe"));
        assert!(is_shell_program("/bin/bash"));
        assert!(!is_shell_program("code"));
    }

    #[test]
    fn external_urls_are_absolute_http_without_credentials() {
        assert_eq!(
            validate_external_url("HTTPS://example.test/a?x=1&y=2").unwrap(),
            "https://example.test/a?x=1&y=2"
        );
        assert!(validate_external_url("file:///tmp/demo").is_err());
        assert!(validate_external_url("https://user:secret@example.test").is_err());
        assert!(validate_external_url("https://").is_err());
        assert!(validate_external_url("https://example.test/\nnext").is_err());
    }
}
