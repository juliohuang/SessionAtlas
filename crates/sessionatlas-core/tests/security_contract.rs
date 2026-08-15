//! Contract tests for `sessionatlas_core::security`.
//!
//! Mirrors the R10 pass conditions and `CommandSecurityTests.cs`: tool keys,
//! session IDs, display labels, safe command parsing, Windows/Posix command
//! construction, Posix single-quoting, and shell/script-wrapper rejection.
//! Every test is pure — no process is ever started and no environment or
//! filesystem state is touched.

use sessionatlas_core::security::{
    build_posix_command, build_windows_command, is_shell_program, parse_safe_command, quote_posix,
    validate_display_label, validate_session_id, validate_tool_key, SecurityError,
};

fn ok(value: &str) -> String {
    String::from(value)
}

#[test]
fn security_contract_tool_key_accepts_valid_shapes() {
    assert_eq!(validate_tool_key("claude").unwrap(), ok("claude"));
    assert_eq!(validate_tool_key(" my-tool.1 ").unwrap(), ok("my-tool.1"));
    assert_eq!(validate_tool_key("a").unwrap(), ok("a"));
    assert_eq!(validate_tool_key("a-b_c.d+e").unwrap(), ok("a-b_c.d+e"));
}

#[test]
fn security_contract_tool_key_rejects_bad_shapes() {
    let long: String = "k".repeat(65);
    for bad in [
        "",
        "   ",
        &long,
        "-cli",
        "--codex",
        "a b",
        "a;b",
        "a&b",
        "a\nb",
        "a\u{0007}b",
        "\u{0000}",
        "\u{0001}b",
        "\u{007F}",
        "é",
        "工具",
    ] {
        assert_eq!(
            validate_tool_key(bad),
            Err(SecurityError::InvalidToolKey),
            "must reject tool key {bad:?}"
        );
    }
}

#[test]
fn security_contract_session_id_accepts_valid_shapes() {
    assert_eq!(validate_session_id("abc123").unwrap(), ok("abc123"));
    assert_eq!(
        validate_session_id(" id-abc:def_ghi.jkl+2 ").unwrap(),
        ok("id-abc:def_ghi.jkl+2")
    );
    assert_eq!(validate_session_id("a-b").unwrap(), ok("a-b"));
    assert_eq!(validate_session_id("a_b.c:d+e").unwrap(), ok("a_b.c:d+e"));
}

#[test]
fn security_contract_session_id_rejects_option_shaped_control_and_unknown() {
    let long: String = "x".repeat(513);
    for bad in [
        "",
        "   ",
        &long,
        "-abc",
        "--resume",
        "-",
        "a b",
        "a;b",
        "a&b",
        "a|b",
        "a<b",
        "a>b",
        "a^b",
        "a%b",
        "a!b",
        "a\nb",
        "a\u{0007}b",
        "\u{0000}",
        "é",
    ] {
        assert_eq!(
            validate_session_id(bad),
            Err(SecurityError::InvalidSessionId),
            "must reject session ID {bad:?}"
        );
    }
}

#[test]
fn security_contract_display_label_trims_and_rejects_control() {
    assert_eq!(
        validate_display_label("  My Agent  ").unwrap(),
        ok("My Agent")
    );
    assert_eq!(validate_display_label("ok-名字").unwrap(), ok("ok-名字"));
    let long: String = "x".repeat(129);
    for bad in [
        "",
        "   ",
        &long,
        "bad\u{0000}label",
        "bad\nlabel",
        "a\u{001B}b",
    ] {
        assert_eq!(
            validate_display_label(bad),
            Err(SecurityError::InvalidDisplayLabel),
            "must reject display label {bad:?}"
        );
    }
}

#[test]
fn security_contract_safe_command_accepts_program_with_quoted_arguments() {
    assert_eq!(parse_safe_command("claude").unwrap(), vec![ok("claude")]);
    assert_eq!(
        parse_safe_command("mycli arg \"two words\"").unwrap(),
        vec![ok("mycli"), ok("arg"), ok("two words")]
    );
    assert_eq!(
        parse_safe_command("  cli --flag value  ").unwrap(),
        vec![ok("cli"), ok("--flag"), ok("value")]
    );
}

#[test]
fn security_contract_safe_command_rejects_blank_command() {
    for bad in ["", "   ", "\t"] {
        assert_eq!(
            parse_safe_command(bad),
            Err(SecurityError::BlankCommand),
            "must reject blank command {bad:?}"
        );
    }
}

#[test]
fn security_contract_safe_command_rejects_control_and_metacharacters() {
    for bad in [
        "a\u{0000}b",
        "a\rb",
        "a\nb",
        "a\u{0007}b",
        "\u{001B}[31m",
        "a&b",
        "a|b",
        "a<b",
        "a>b",
        "a^b",
        "a%b",
        "a!b",
    ] {
        assert_eq!(
            parse_safe_command(bad),
            Err(SecurityError::CommandControlCharacters),
            "must reject command {bad:?}"
        );
    }
}

#[test]
fn security_contract_safe_command_rejects_unbalanced_quotes() {
    for bad in ["\"unclosed", "\"a b", "a\"b", "a ' b \" c"] {
        assert_eq!(
            parse_safe_command(bad),
            Err(SecurityError::UnclosedQuote),
            "must reject unbalanced quotes {bad:?}"
        );
    }
}

#[test]
fn security_contract_safe_command_rejects_empty_executable() {
    for bad in ["\"\"", "\"\" argument", " \"\" "] {
        assert_eq!(
            parse_safe_command(bad),
            Err(SecurityError::EmptyExecutable),
            "must reject empty executable {bad:?}"
        );
    }
}

#[test]
fn security_contract_safe_command_rejects_option_like_executable() {
    for bad in ["-flag", "--help", "-", "-cli arg"] {
        assert_eq!(
            parse_safe_command(bad),
            Err(SecurityError::OptionLikeExecutable),
            "must reject option-like executable {bad:?}"
        );
    }
}

#[test]
fn security_contract_safe_command_rejects_shells_and_script_wrappers() {
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
        "/usr/bin/bash",
        "run.bat",
        "deploy.cmd",
        "setup.ps1",
        "bash -c echo hi",
    ] {
        assert_eq!(
            parse_safe_command(bad),
            Err(SecurityError::ShellWrapper),
            "must reject shell wrapper {bad:?}"
        );
    }
}

#[test]
fn security_contract_is_shell_program_detects_wrappers_by_name_and_extension() {
    assert!(is_shell_program("cmd"));
    assert!(is_shell_program("Cmd.EXE"));
    assert!(is_shell_program("powershell"));
    assert!(is_shell_program("pwsh"));
    assert!(is_shell_program("sh"));
    assert!(is_shell_program("bash"));
    assert!(is_shell_program("zsh"));
    assert!(is_shell_program("fish"));
    assert!(is_shell_program("wsl"));
    assert!(is_shell_program("osascript"));
    assert!(is_shell_program("C:/Windows/System32/cmd.exe"));
    assert!(is_shell_program("run.bat"));
    assert!(is_shell_program("deploy.cmd"));
    assert!(is_shell_program("setup.ps1"));
    assert!(!is_shell_program("claude"));
    assert!(!is_shell_program("codex"));
    assert!(!is_shell_program("/usr/local/bin/claude"));
    assert!(!is_shell_program("my-ai.exe"));
}

#[test]
fn security_contract_windows_command_quotes_every_token() {
    let tokens = vec![ok("claude"), ok("--resume"), ok("abc-123")];
    assert_eq!(
        build_windows_command(&tokens).unwrap(),
        "\"claude\" \"--resume\" \"abc-123\""
    );
}

#[test]
fn security_contract_windows_command_rejects_unsafe_tokens() {
    for bad in [
        vec![ok("")],
        vec![ok("claude"), ok("a\"b")],
        vec![ok("claude"), ok("a&b")],
        vec![ok("claude"), ok("a\nb")],
        vec![ok("claude"), ok("a|b")],
        vec![ok("claude"), ok("a\u{0007}b")],
    ] {
        assert_eq!(
            build_windows_command(&bad),
            Err(SecurityError::UnsupportedShellCharacters),
            "must reject windows tokens {bad:?}"
        );
    }
}

#[test]
fn security_contract_posix_command_quotes_tokens() {
    let tokens = vec![ok("claude"), ok("--resume"), ok("abc-123")];
    assert_eq!(
        build_posix_command(&tokens).unwrap(),
        "'claude' '--resume' 'abc-123'"
    );
}

#[test]
fn security_contract_quote_posix_preserves_apostrophes() {
    assert_eq!(quote_posix("plain").unwrap(), "'plain'");
    assert_eq!(quote_posix("it's a path").unwrap(), "'it'\"'\"'s a path'");
    assert_eq!(quote_posix("a'b'c").unwrap(), "'a'\"'\"'b'\"'\"'c'");
    assert_eq!(
        quote_posix("/path with spaces/!&()").unwrap(),
        "'/path with spaces/!&()'"
    );
}

#[test]
fn security_contract_quote_posix_rejects_nul() {
    assert_eq!(quote_posix("a\u{0000}b"), Err(SecurityError::NulByte));
}

#[test]
fn security_contract_posix_command_rejects_unsafe_tokens() {
    let bad = vec![ok("claude"), ok("a\"b")];
    assert_eq!(
        build_posix_command(&bad),
        Err(SecurityError::UnsupportedShellCharacters)
    );
}
