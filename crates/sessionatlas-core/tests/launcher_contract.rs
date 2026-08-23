//! Contract tests for `sessionatlas_core::launcher`: tool identity resolution
//! (built-ins, enabled non-overriding custom tools), argv construction with
//! independent tool-specific resume arguments, cross-platform terminal shapes, and the
//! injectable launch boundary. Recording runners and fake resolvers are used
//! throughout so no test starts a real terminal, AI CLI, `where`, or `which`.

use std::path::{Path, PathBuf};

use sessionatlas_core::config::AppConfig;
use sessionatlas_core::launcher::{
    build_process_spec, is_reserved_tool_key, native_terminal_platform, Launcher, LauncherError,
    TerminalPlatform, ToolCommands, BUILT_IN_TOOL_KEYS,
};
use sessionatlas_core::model::ToolSource;
use sessionatlas_core::process::{ProgramResolver, RecordingProcessRunner};

/// Resolver that pretends only the listed programs are on PATH.
struct FakeResolver {
    present: Vec<&'static str>,
}

impl ProgramResolver for FakeResolver {
    fn is_on_path(&self, program: &str) -> bool {
        self.present.contains(&program)
    }

    fn resolve(&self, program: &str) -> Option<PathBuf> {
        self.is_on_path(program).then(|| PathBuf::from(program))
    }
}

/// Probe that pretends Windows Terminal is never installed.
fn no_wt(_: &Path) -> bool {
    false
}

fn built_ins_are_the_six_supported_identities() {
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
fn launcher_contract_built_in_commands_map_to_themselves() {
    built_ins_are_the_six_supported_identities();
    let commands = ToolCommands::built_in();
    for key in BUILT_IN_TOOL_KEYS {
        assert!(commands.known_key(key), "{key} must be a known key");
        assert_eq!(
            commands.build_arguments(key, None).unwrap(),
            vec![key.to_string()]
        );
    }
    assert!(!commands.known_key("ghost"));
}

#[test]
fn launcher_contract_custom_tools_added_only_when_valid_enabled_non_override() {
    let mut config = AppConfig::default();
    config.custom_tools = vec![
        ToolSource {
            key: "myagent".to_string(),
            cli_command: "mycli --flag".to_string(),
            is_enabled: true,
            ..ToolSource::default()
        },
        // Attempts to override a built-in identity are ignored.
        ToolSource {
            key: "claude".to_string(),
            cli_command: "evil".to_string(),
            is_enabled: true,
            ..ToolSource::default()
        },
        // Invalid key: ignored.
        ToolSource {
            key: "bad key".to_string(),
            cli_command: "cli".to_string(),
            is_enabled: true,
            ..ToolSource::default()
        },
        // Unsafe command (shell wrapper): ignored.
        ToolSource {
            key: "unsafe".to_string(),
            cli_command: "sh -c x".to_string(),
            is_enabled: true,
            ..ToolSource::default()
        },
        // Disabled: ignored.
        ToolSource {
            key: "disabled".to_string(),
            cli_command: "cli".to_string(),
            is_enabled: false,
            ..ToolSource::default()
        },
        // Empty command: ignored.
        ToolSource {
            key: "emptycmd".to_string(),
            cli_command: "   ".to_string(),
            is_enabled: true,
            ..ToolSource::default()
        },
    ];
    let commands = ToolCommands::from_config(&config);

    assert!(commands.known_key("myagent"));
    assert_eq!(
        commands.build_arguments("myagent", None).unwrap(),
        vec!["mycli", "--flag"]
    );
    assert!(!commands.known_key("bad key"));
    assert!(!commands.known_key("unsafe"));
    assert!(!commands.known_key("disabled"));
    assert!(!commands.known_key("emptycmd"));
    // The built-in identity is untouched by the override attempt.
    assert_eq!(
        commands.build_arguments("claude", None).unwrap(),
        vec!["claude"]
    );
}

#[test]
fn launcher_contract_build_arguments_appends_resume_as_independent_arguments() {
    let commands = ToolCommands::built_in();
    assert_eq!(
        commands.build_arguments("claude", None).unwrap(),
        vec!["claude"]
    );
    assert_eq!(
        commands.build_arguments("claude", Some("abc-123")).unwrap(),
        vec!["claude", "--resume", "abc-123"]
    );
    assert_eq!(
        commands.build_arguments("claude", Some("  ")).unwrap(),
        vec!["claude"],
        "blank session id is dropped"
    );
    assert_eq!(
        commands
            .build_arguments("codex", Some("s:1.2_3+4-5"))
            .unwrap(),
        vec!["codex", "resume", "s:1.2_3+4-5"]
    );
    assert_eq!(
        commands.build_arguments("pi", Some("session-123")).unwrap(),
        vec!["pi", "--session", "session-123"]
    );
    assert_eq!(
        commands.build_arguments("pi", Some("pi-session")).unwrap(),
        vec!["pi", "--session", "pi-session"],
        "Pi uses its native --session flag"
    );
}

#[test]
fn launcher_contract_build_arguments_rejects_unknown_and_invalid_inputs() {
    let commands = ToolCommands::built_in();
    assert_eq!(
        commands.build_arguments("ghost", None).unwrap_err(),
        LauncherError::UnknownToolKey("ghost".to_string())
    );
    assert_eq!(
        commands.build_arguments("-claude", None).unwrap_err(),
        LauncherError::InvalidToolKey("-claude".to_string())
    );
    assert_eq!(
        commands
            .build_arguments("claude", Some("-resume"))
            .unwrap_err(),
        LauncherError::InvalidSessionId("-resume".to_string())
    );
    assert_eq!(
        commands.build_arguments("claude", Some("a b")).unwrap_err(),
        LauncherError::InvalidSessionId("a b".to_string())
    );
    assert_eq!(
        commands
            .build_arguments("claude", Some("a\u{0007}b"))
            .unwrap_err(),
        LauncherError::InvalidSessionId("a\u{0007}b".to_string())
    );
}

#[test]
fn launcher_contract_availability_uses_injectable_resolver() {
    let resolver = FakeResolver {
        present: vec!["claude", "codex"],
    };
    let commands = ToolCommands::built_in();
    assert!(commands.is_tool_available("claude", &resolver));
    assert!(commands.is_tool_available("codex", &resolver));
    assert!(!commands.is_tool_available("kimi", &resolver));
    assert!(!commands.is_tool_available("ghost", &resolver));
    assert!(!commands.is_tool_available("-claude", &resolver));
}

#[test]
fn launcher_contract_windows_shape_uses_wt_with_cmd_dash_k() {
    let arguments = vec![
        "claude".to_string(),
        "--resume".to_string(),
        "abc-123".to_string(),
    ];
    let spec = build_process_spec(
        TerminalPlatform::Windows,
        "C:\\work\\proj",
        &arguments,
        Some("C:\\fake\\wt.exe"),
        None,
    )
    .unwrap();
    assert_eq!(spec.program, "C:\\fake\\wt.exe");
    assert_eq!(
        spec.arguments,
        vec![
            "-d",
            "C:\\work\\proj",
            "cmd.exe",
            "/D",
            "/K",
            "\"claude\" \"--resume\" \"abc-123\""
        ]
    );
    assert_eq!(spec.working_directory, "C:\\work\\proj");
}

#[test]
fn launcher_contract_windows_shape_falls_back_to_cmd() {
    let arguments = vec!["claude".to_string()];
    let spec = build_process_spec(
        TerminalPlatform::Windows,
        "C:\\work\\proj",
        &arguments,
        None,
        None,
    )
    .unwrap();
    assert_eq!(spec.program, "cmd.exe");
    assert_eq!(spec.arguments, vec!["/D", "/K", "\"claude\""]);
    assert_eq!(spec.working_directory, "C:\\work\\proj");
}

#[test]
fn launcher_contract_macos_shape_uses_osascript_argv() {
    let arguments = vec![
        "claude".to_string(),
        "--resume".to_string(),
        "abc-123".to_string(),
    ];
    let spec = build_process_spec(
        TerminalPlatform::MacOs,
        "/work/proj",
        &arguments,
        None,
        None,
    )
    .unwrap();
    assert_eq!(spec.program, "osascript");
    assert_eq!(spec.arguments.len(), 4);
    assert_eq!(spec.arguments[0], "-e");
    assert!(
        spec.arguments[1].contains("tell application \"Terminal\""),
        "script must target Terminal"
    );
    assert_eq!(spec.arguments[2], "--");
    assert_eq!(
        spec.arguments[3],
        "cd '/work/proj' && exec 'claude' '--resume' 'abc-123'"
    );
    assert_eq!(spec.working_directory, "/work/proj");
}

#[test]
fn launcher_contract_macos_shape_preserves_apostrophes_in_path() {
    let arguments = vec!["claude".to_string()];
    let spec = build_process_spec(
        TerminalPlatform::MacOs,
        "/it's/proj",
        &arguments,
        None,
        None,
    )
    .unwrap();
    assert_eq!(spec.arguments[3], "cd '/it'\"'\"'s/proj' && exec 'claude'");
}

#[test]
fn launcher_contract_linux_shape_uses_separator_per_terminal() {
    let arguments = vec![
        "claude".to_string(),
        "--resume".to_string(),
        "abc-123".to_string(),
    ];

    let gnome = build_process_spec(
        TerminalPlatform::Linux,
        "/work/proj",
        &arguments,
        None,
        Some("gnome-terminal"),
    )
    .unwrap();
    assert_eq!(gnome.program, "gnome-terminal");
    assert_eq!(gnome.arguments, vec!["--", "claude", "--resume", "abc-123"]);
    assert_eq!(gnome.working_directory, "/work/proj");

    let xterm = build_process_spec(
        TerminalPlatform::Linux,
        "/work/proj",
        &arguments,
        None,
        Some("xterm"),
    )
    .unwrap();
    assert_eq!(xterm.program, "xterm");
    assert_eq!(xterm.arguments, vec!["-e", "claude", "--resume", "abc-123"]);
}

#[test]
fn launcher_contract_linux_shape_without_terminal_is_an_error() {
    let arguments = vec!["claude".to_string()];
    let error = build_process_spec(
        TerminalPlatform::Linux,
        "/work/proj",
        &arguments,
        None,
        None,
    )
    .unwrap_err();
    assert_eq!(error, LauncherError::NoTerminal);
}

#[test]
fn launcher_contract_launch_rejects_missing_project_directory() {
    let runner = RecordingProcessRunner::new();
    let resolver = FakeResolver {
        present: vec!["claude", "gnome-terminal"],
    };
    let launcher = Launcher::new(ToolCommands::built_in(), &resolver, &runner, &no_wt);
    let error = launcher
        .launch("/definitely/missing/proj", "claude", None)
        .unwrap_err();
    assert_eq!(
        error,
        LauncherError::ProjectDirectoryMissing("/definitely/missing/proj".to_string())
    );
    assert_eq!(runner.start_count(), 0, "nothing may be started");
}

#[test]
fn launcher_contract_launch_rejects_unknown_tool_and_invalid_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().to_string_lossy().into_owned();
    let runner = RecordingProcessRunner::new();
    let resolver = FakeResolver {
        present: vec!["claude", "gnome-terminal"],
    };
    let launcher = Launcher::new(ToolCommands::built_in(), &resolver, &runner, &no_wt);

    assert_eq!(
        launcher.launch(&project, "ghost", None).unwrap_err(),
        LauncherError::UnknownToolKey("ghost".to_string())
    );
    assert_eq!(
        launcher.launch(&project, "-claude", None).unwrap_err(),
        LauncherError::InvalidToolKey("-claude".to_string())
    );
    assert_eq!(
        launcher
            .launch(&project, "claude", Some("-resume"))
            .unwrap_err(),
        LauncherError::InvalidSessionId("-resume".to_string())
    );
    assert_eq!(
        launcher
            .launch(&project, "claude", Some("a;b"))
            .unwrap_err(),
        LauncherError::InvalidSessionId("a;b".to_string())
    );
    assert_eq!(runner.start_count(), 0, "rejected launches never start");
}

#[test]
fn launcher_contract_launch_start_failure_is_an_error_not_success() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().to_string_lossy().into_owned();
    let runner = RecordingProcessRunner::new();
    let resolver = FakeResolver {
        present: vec!["claude", "gnome-terminal"],
    };
    let launcher = Launcher::new(ToolCommands::built_in(), &resolver, &runner, &no_wt);
    runner.fail_starts("terminal refused");

    assert!(matches!(
        launcher.launch(&project, "claude", None),
        Err(LauncherError::StartFailed(_))
    ));
}

#[test]
fn launcher_contract_launch_records_native_terminal_shape() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().to_string_lossy().into_owned();
    let runner = RecordingProcessRunner::new();
    let resolver = FakeResolver {
        present: vec!["claude", "gnome-terminal", "xterm"],
    };
    let launcher = Launcher::new(ToolCommands::built_in(), &resolver, &runner, &no_wt);

    let spec = launcher
        .launch(&project, "claude", Some("abc-123"))
        .unwrap();
    assert_eq!(runner.start_count(), 1);
    assert_eq!(spec, runner.started()[0]);
    assert_eq!(spec.working_directory, project);

    match native_terminal_platform() {
        TerminalPlatform::Windows => {
            assert_eq!(spec.program, "cmd.exe", "no wt probe means cmd fallback");
            assert_eq!(spec.arguments[..2], ["/D".to_string(), "/K".to_string()]);
            assert!(
                spec.arguments.last().unwrap().contains("claude"),
                "command string must contain tool argv"
            );
        }
        TerminalPlatform::MacOs => {
            assert_eq!(spec.program, "osascript");
            assert_eq!(spec.arguments[2], "--");
            assert!(spec.arguments[3].contains("claude"));
        }
        TerminalPlatform::Linux => {
            assert_eq!(spec.program, "gnome-terminal");
            assert_eq!(spec.arguments, ["--", "claude", "--resume", "abc-123"]);
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
#[test]
fn launcher_contract_launch_without_terminal_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().to_string_lossy().into_owned();
    let runner = RecordingProcessRunner::new();
    let resolver = FakeResolver {
        present: vec!["claude"],
    };
    let launcher = Launcher::new(ToolCommands::built_in(), &resolver, &runner, &no_wt);
    let error = launcher.launch(&project, "claude", None).unwrap_err();
    assert_eq!(error, LauncherError::NoTerminal);
    assert_eq!(runner.start_count(), 0);
}
