//! Contract tests for `sessionatlas_core::process`: the process spec, the
//! injectable runner boundary, the recording test double, and pure PATH
//! resolution. No test starts a terminal, AI CLI, `where`, or `which`.

use std::path::PathBuf;

use sessionatlas_core::process::{
    search_path, ProcessError, ProcessOutput, ProcessRunner, ProcessSpec, ProgramResolver,
    RecordingProcessRunner,
};

fn spec(program: &str) -> ProcessSpec {
    ProcessSpec {
        program: program.to_string(),
        arguments: vec!["--flag".to_string(), "value".to_string()],
        working_directory: "/synthetic/project".to_string(),
    }
}

#[test]
fn process_contract_spec_carries_program_arguments_and_cwd() {
    let spec = spec("claude");
    assert_eq!(spec.program, "claude");
    assert_eq!(spec.arguments, vec!["--flag", "value"]);
    assert_eq!(spec.working_directory, "/synthetic/project");
}

#[test]
fn process_contract_recording_runner_captures_starts() {
    let runner = RecordingProcessRunner::new();
    assert_eq!(runner.start_count(), 0);
    assert!(runner.started().is_empty());

    runner.start(&spec("claude")).unwrap();
    runner.start(&spec("codex")).unwrap();
    assert_eq!(runner.start_count(), 2);
    let started = runner.started();
    assert_eq!(started[0].program, "claude");
    assert_eq!(started[1].program, "codex");
    assert_eq!(started[1].working_directory, "/synthetic/project");
}

#[test]
fn process_contract_recording_runner_can_be_scripted_to_fail() {
    let runner = RecordingProcessRunner::new();
    runner.fail_starts("no terminal here");
    let error = runner.start(&spec("claude")).unwrap_err();
    assert!(error.detail.contains("no terminal here"));
    assert_eq!(runner.start_count(), 1, "failed start is still recorded");
}

#[test]
fn process_contract_recording_runner_run_uses_queued_outcomes() {
    let runner = RecordingProcessRunner::new();
    let outcome = ProcessOutput {
        exit_code: 0,
        stdout: "ok".to_string(),
        stderr: String::new(),
    };
    runner.queue_run(Ok(outcome.clone()));
    runner.queue_run(Err(ProcessError {
        program: "claude".to_string(),
        detail: "boom".to_string(),
    }));

    assert_eq!(runner.run(&spec("claude")).unwrap(), outcome);
    let failed = runner.run(&spec("codex")).unwrap_err();
    assert!(failed.detail.contains("boom"));

    // Unscripted runs record the spec and succeed with empty output.
    let empty = runner.run(&spec("kimi")).unwrap();
    assert_eq!(empty.exit_code, 0);
    assert_eq!(runner.start_count(), 1);
}

#[test]
fn process_contract_recording_runner_supports_start_failure_after_success() {
    let runner = RecordingProcessRunner::new();
    runner.start(&spec("claude")).unwrap();
    assert_eq!(runner.start_count(), 1);
    runner.fail_starts("failing now");
    assert!(runner.start(&spec("claude")).is_err());
}

#[cfg(unix)]
#[test]
fn process_contract_search_path_finds_executables_on_posix() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let tool = dir.path().join("tool");
    std::fs::write(&tool, b"#!/bin/sh\n").unwrap();
    let mut permissions = std::fs::metadata(&tool).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&tool, permissions).unwrap();

    let no_exec = dir.path().join("noexec");
    std::fs::write(&no_exec, b"x").unwrap();

    let path_var = dir.path().to_string_lossy().into_owned();
    assert_eq!(
        search_path("tool", &path_var, ""),
        Some(tool.clone()),
        "executable on PATH must resolve"
    );
    assert_eq!(
        search_path("noexec", &path_var, ""),
        None,
        "non-executable must not resolve"
    );
    assert_eq!(search_path("missing", &path_var, ""), None);
    assert_eq!(
        search_path(&tool.to_string_lossy(), "", ""),
        Some(tool.clone()),
        "direct path must resolve without PATH"
    );
    let absent = dir.path().join("sub").join("tool");
    assert_eq!(search_path(&absent.to_string_lossy(), "", ""), None);
    assert_eq!(search_path("", &path_var, ""), None);
}

#[cfg(windows)]
#[test]
fn process_contract_search_path_uses_pathext_on_windows() {
    let dir = tempfile::tempdir().unwrap();
    let tool = dir.path().join("tool.exe");
    std::fs::write(&tool, b"x").unwrap();

    let path_var = dir.path().to_string_lossy().into_owned();
    assert_eq!(
        search_path("tool.exe", &path_var, ".EXE;.BAT"),
        Some(tool.clone())
    );
    assert_eq!(
        search_path("tool", &path_var, ".EXE;.BAT"),
        Some(dir.path().join("tool.EXE")),
        "PATHEXT extension fallback must resolve"
    );
    assert_eq!(search_path("missing", &path_var, ".EXE"), None);
    assert_eq!(search_path("tool", &path_var, ""), None);
}

#[test]
fn process_contract_unknown_program_resolves_nowhere_with_empty_path() {
    assert_eq!(search_path("claude", "", ""), None);
    assert_eq!(search_path("a/b", "", ""), None);
}

#[test]
fn process_contract_path_resolver_never_launches() {
    // The production resolver is exercised only through the pure search
    // function; constructing it is harmless and it must never spawn a process.
    let resolver = sessionatlas_core::process::PathProgramResolver;
    assert_eq!(resolver.resolve("definitely-not-a-real-tool-xyz"), None);
    let _ = PathBuf::new();
}
