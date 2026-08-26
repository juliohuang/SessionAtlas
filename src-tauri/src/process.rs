use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessSpec {
    pub(crate) program: OsString,
    pub(crate) args: Vec<OsString>,
    pub(crate) current_dir: Option<PathBuf>,
    pub(crate) environment: Vec<(OsString, OsString)>,
}

impl ProcessSpec {
    pub(crate) fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            current_dir: None,
            environment: Vec::new(),
        }
    }

    pub(crate) fn arg(mut self, value: impl Into<OsString>) -> Self {
        self.args.push(value.into());
        self
    }

    pub(crate) fn args<I, S>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(values.into_iter().map(|value| value.as_ref().to_owned()));
        self
    }

    pub(crate) fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.environment.push((key.into(), value.into()));
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessOutput {
    pub(crate) success: bool,
    pub(crate) status_code: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

pub(crate) trait ProcessRunner {
    fn output(&self, spec: &ProcessSpec) -> Result<ProcessOutput, String>;
    fn spawn(&self, spec: &ProcessSpec) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemProcessRunner;

impl ProcessRunner for SystemProcessRunner {
    fn output(&self, spec: &ProcessSpec) -> Result<ProcessOutput, String> {
        let mut command = command_from_spec(spec);
        let output = command.output().map_err(|error| error.to_string())?;
        Ok(ProcessOutput {
            success: output.status.success(),
            status_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    fn spawn(&self, spec: &ProcessSpec) -> Result<(), String> {
        command_from_spec(spec)
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TimeoutProcessRunner {
    timeout: Duration,
    max_output_bytes: usize,
}

impl TimeoutProcessRunner {
    pub(crate) const fn new(timeout: Duration) -> Self {
        Self::with_limits(timeout, MAX_CAPTURED_OUTPUT_BYTES)
    }

    pub(crate) const fn with_limits(timeout: Duration, max_output_bytes: usize) -> Self {
        Self {
            timeout,
            max_output_bytes,
        }
    }
}

impl ProcessRunner for TimeoutProcessRunner {
    fn output(&self, spec: &ProcessSpec) -> Result<ProcessOutput, String> {
        output_with_timeout_limited(spec, self.timeout, self.max_output_bytes)
    }

    fn spawn(&self, spec: &ProcessSpec) -> Result<(), String> {
        SystemProcessRunner.spawn(spec)
    }
}

const MAX_CAPTURED_OUTPUT_BYTES: usize = 256 * 1024;
const OUTPUT_LIMIT_POLL_INTERVAL: Duration = Duration::from_millis(10);
static OUTPUT_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn create_private_output_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)
}

/// Execute an argument-array process with a hard wall-clock bound.
///
/// stdout and stderr are redirected to private temporary files instead of
/// pipes. That keeps the timeout path independent from pipe capacity and from
/// descendants (for example an SSH helper) inheriting a pipe handle. While the
/// child is running, the files are polled every 10ms and an overflow is
/// fail-fast terminated; they are checked again before and after reading to
/// close the remaining size/read race. The caller receives a bounded error on
/// timeout or output overflow.
pub(crate) fn output_with_timeout(
    spec: &ProcessSpec,
    timeout: Duration,
) -> Result<ProcessOutput, String> {
    output_with_timeout_limited(spec, timeout, MAX_CAPTURED_OUTPUT_BYTES)
}

fn output_with_timeout_limited(
    spec: &ProcessSpec,
    timeout: Duration,
    max_output_bytes: usize,
) -> Result<ProcessOutput, String> {
    let sequence = OUTPUT_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let prefix = format!(
        "sessionatlas-process-{}-{stamp}-{sequence}",
        std::process::id()
    );
    let directory = std::env::temp_dir();
    let stdout_path = directory.join(format!("{prefix}.stdout"));
    let stderr_path = directory.join(format!("{prefix}.stderr"));
    let stdout_file = create_private_output_file(&stdout_path)
        .map_err(|error| format!("failed to create process stdout file: {error}"))?;
    let stderr_file = match create_private_output_file(&stderr_path) {
        Ok(file) => file,
        Err(error) => {
            let cleanup = cleanup_output_files(&stdout_path, &stderr_path).err();
            return Err(match cleanup {
                Some(cleanup) => {
                    format!("failed to create process stderr file: {error}; {cleanup}")
                }
                None => format!("failed to create process stderr file: {error}"),
            });
        }
    };

    let mut command = command_from_spec(spec);
    let mut child = match command
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let cleanup = cleanup_output_files(&stdout_path, &stderr_path).err();
            return Err(match cleanup {
                Some(cleanup) => format!("{error}; {cleanup}"),
                None => error.to_string(),
            });
        }
    };

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if let Err(error) =
                    check_output_limits(&stdout_path, &stderr_path, max_output_bytes)
                {
                    return Err(terminate_child_and_cleanup(
                        &mut child,
                        &stdout_path,
                        &stderr_path,
                        error,
                    ));
                }
                break status;
            }
            Ok(None) if Instant::now() < deadline => {
                if let Err(error) =
                    check_output_limits(&stdout_path, &stderr_path, max_output_bytes)
                {
                    return Err(terminate_child_and_cleanup(
                        &mut child,
                        &stdout_path,
                        &stderr_path,
                        error,
                    ));
                }
                std::thread::sleep(OUTPUT_LIMIT_POLL_INTERVAL);
            }
            Ok(None) => {
                if let Err(error) =
                    check_output_limits(&stdout_path, &stderr_path, max_output_bytes)
                {
                    return Err(terminate_child_and_cleanup(
                        &mut child,
                        &stdout_path,
                        &stderr_path,
                        error,
                    ));
                }
                return Err(terminate_child_and_cleanup(
                    &mut child,
                    &stdout_path,
                    &stderr_path,
                    format!("process timed out after {} ms", timeout.as_millis()),
                ));
            }
            Err(error) => {
                return Err(terminate_child_and_cleanup(
                    &mut child,
                    &stdout_path,
                    &stderr_path,
                    format!("failed to wait for process: {error}"),
                ));
            }
        }
    };

    let stdout = read_checked_output(&stdout_path, max_output_bytes, "stdout");
    let stderr = read_checked_output(&stderr_path, max_output_bytes, "stderr");
    let cleanup = cleanup_output_files(&stdout_path, &stderr_path);
    let stdout = stdout?;
    let stderr = stderr?;
    cleanup?;
    Ok(ProcessOutput {
        success: status.success(),
        status_code: status.code(),
        stdout,
        stderr,
    })
}

fn check_output_limits(
    stdout_path: &Path,
    stderr_path: &Path,
    max_output_bytes: usize,
) -> Result<(), String> {
    for (path, stream) in [(stdout_path, "stdout"), (stderr_path, "stderr")] {
        let size = fs::metadata(path)
            .map_err(|error| format!("failed to inspect process {stream}: {error}"))?
            .len();
        if size > max_output_bytes as u64 {
            return Err(format!(
                "process {stream} exceeded output limit of {max_output_bytes} bytes"
            ));
        }
    }
    Ok(())
}

fn terminate_child_and_cleanup(
    child: &mut Child,
    stdout_path: &Path,
    stderr_path: &Path,
    reason: String,
) -> String {
    let kill_error = child.kill().err();
    let wait_error = child.wait().err();
    let cleanup_error = cleanup_output_files(stdout_path, stderr_path).err();
    let mut details = Vec::new();
    if let Some(error) = kill_error {
        if wait_error.is_some() {
            details.push(format!("failed to terminate process: {error}"));
        }
    }
    if let Some(error) = wait_error {
        details.push(format!("failed to wait for process termination: {error}"));
    }
    if let Some(error) = cleanup_error {
        details.push(error);
    }
    if details.is_empty() {
        reason
    } else {
        format!("{reason}; {}", details.join("; "))
    }
}

fn cleanup_output_files(stdout_path: &Path, stderr_path: &Path) -> Result<(), String> {
    let mut errors = Vec::new();
    for path in [stdout_path, stderr_path] {
        if let Err(error) = fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                errors.push(format!("failed to clean process output file: {error}"));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn read_checked_output(
    path: &Path,
    max_output_bytes: usize,
    stream: &str,
) -> Result<Vec<u8>, String> {
    check_output_limits_for_stream(path, max_output_bytes, stream)?;
    let output = read_bounded_output(path, max_output_bytes, stream)?;
    check_output_limits_for_stream(path, max_output_bytes, stream)?;
    Ok(output)
}

fn check_output_limits_for_stream(
    path: &Path,
    max_output_bytes: usize,
    stream: &str,
) -> Result<(), String> {
    let size = fs::metadata(path)
        .map_err(|error| format!("failed to inspect process {stream}: {error}"))?
        .len();
    if size > max_output_bytes as u64 {
        return Err(format!(
            "process {stream} exceeded output limit of {max_output_bytes} bytes"
        ));
    }
    Ok(())
}

fn read_bounded_output(
    path: &Path,
    max_output_bytes: usize,
    stream: &str,
) -> Result<Vec<u8>, String> {
    let mut file =
        File::open(path).map_err(|error| format!("failed to read process {stream}: {error}"))?;
    let mut output = Vec::new();
    let read_limit = max_output_bytes
        .checked_add(1)
        .ok_or_else(|| format!("process {stream} output limit is too large"))?;
    let mut limited = (&mut file).take(read_limit as u64);
    limited
        .read_to_end(&mut output)
        .map_err(|error| format!("failed to read process {stream}: {error}"))?;
    if output.len() > max_output_bytes {
        return Err(format!(
            "process {stream} exceeded output limit of {max_output_bytes} bytes"
        ));
    }
    Ok(output)
}

fn command_from_spec(spec: &ProcessSpec) -> Command {
    let mut command = Command::new(&spec.program);
    command.args(&spec.args);
    if let Some(current_dir) = &spec.current_dir {
        command.current_dir(current_dir);
    }
    command.envs(spec.environment.iter().map(|(key, value)| (key, value)));
    // SessionAtlas is a GUI-subsystem process on Windows. Console children
    // such as npm-generated `claude.cmd`, git, ssh, and package-manager probes
    // would otherwise create a short-lived console window. These ProcessSpec
    // calls are background operations whose output is captured by the app, so
    // they must never allocate a visible console.
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

pub(crate) fn git_read_spec(path: &Path, args: &[&str]) -> ProcessSpec {
    // Git reads repository configuration for every invocation.  These
    // command-line overrides keep background metadata reads and fetches from
    // turning repository-controlled settings into local process execution:
    // fsmonitor hooks, credential helpers, hooks, custom transports, and
    // alternate SSH/askpass programs.  HTTPS and SSH remain the only network
    // protocols explicitly allowed for the background fetch path.
    let mut spec = ProcessSpec::new("git").arg("-C").arg(path.as_os_str());
    for (key, value) in git_safety_config() {
        spec = spec.arg("-c").arg(format!("{key}={value}"));
    }
    spec.args(args)
}

/// Construct a Git command for an explicit user operation.  Unlike
/// `git_read_spec`, this intentionally preserves repository hooks, transports,
/// and other normal Git behavior (for example post-checkout hooks).
pub(crate) fn git_user_operation_spec(path: &Path, args: &[&str]) -> ProcessSpec {
    ProcessSpec::new("git")
        .arg("-C")
        .arg(path.as_os_str())
        .args(args)
}

fn git_safety_config() -> [(&'static str, &'static str); 11] {
    [
        ("core.fsmonitor", "false"),
        ("core.hooksPath", DISABLED_GIT_HOOKS_PATH),
        ("credential.helper", ""),
        ("core.askPass", ""),
        ("protocol.allow", "never"),
        ("protocol.ext.allow", "never"),
        ("protocol.https.allow", "always"),
        ("protocol.ssh.allow", "always"),
        ("core.sshCommand", "ssh"),
        ("fetch.recurseSubmodules", "false"),
        ("submodule.recurse", "false"),
    ]
}

#[cfg(windows)]
const DISABLED_GIT_HOOKS_PATH: &str = "NUL";

#[cfg(not(windows))]
const DISABLED_GIT_HOOKS_PATH: &str = "/dev/null";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_spec_preserves_paths_and_arguments_as_separate_values() {
        let spec = git_read_spec(
            Path::new(r"C:\fixture workspace\repo & safe"),
            &["log", "-1", "--pretty=%s"],
        );

        assert_eq!(spec.program, OsString::from("git"));
        assert_eq!(spec.args[0], OsString::from("-C"));
        assert_eq!(
            spec.args[1],
            OsString::from(r"C:\fixture workspace\repo & safe")
        );
        assert_eq!(spec.args.last(), Some(&OsString::from("--pretty=%s")));
        assert!(spec.args.windows(2).any(|args| {
            args == [OsString::from("-c"), OsString::from("core.fsmonitor=false")]
        }));
    }

    #[test]
    fn git_spec_disables_repository_controlled_execution_paths() {
        let spec = git_read_spec(Path::new("repo"), &["status"]);
        let has_pair = |key: &str, value: &str| {
            spec.args.windows(2).any(|args| {
                args == [
                    OsString::from("-c"),
                    OsString::from(format!("{key}={value}")),
                ]
            })
        };

        assert!(has_pair("core.fsmonitor", "false"));
        assert!(has_pair("core.hooksPath", DISABLED_GIT_HOOKS_PATH));
        assert!(has_pair("credential.helper", ""));
        assert!(has_pair("core.askPass", ""));
        assert!(has_pair("protocol.allow", "never"));
        assert!(has_pair("protocol.ext.allow", "never"));
        assert!(has_pair("protocol.https.allow", "always"));
        assert!(has_pair("protocol.ssh.allow", "always"));
        assert!(has_pair("core.sshCommand", "ssh"));
        assert!(has_pair("fetch.recurseSubmodules", "false"));
        assert!(has_pair("submodule.recurse", "false"));
    }

    #[test]
    fn user_git_spec_preserves_normal_git_behavior() {
        let spec = git_user_operation_spec(
            Path::new(r"C:\fixture workspace\repo & safe"),
            &["switch", "feature"],
        );

        assert_eq!(spec.program, OsString::from("git"));
        assert_eq!(
            spec.args,
            vec![
                OsString::from("-C"),
                OsString::from(r"C:\fixture workspace\repo & safe"),
                OsString::from("switch"),
                OsString::from("feature"),
            ]
        );
        assert!(spec.environment.is_empty());
    }

    #[test]
    fn process_spec_preserves_background_environment_overrides() {
        let spec = ProcessSpec::new("git")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "Never");

        assert_eq!(
            spec.environment,
            vec![
                (OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0")),
                (OsString::from("GCM_INTERACTIVE"), OsString::from("Never")),
            ]
        );
    }

    #[test]
    fn bounded_output_kills_a_process_that_exceeds_the_deadline() {
        let spec = process_helper_spec("hang");
        let started = std::time::Instant::now();
        let error = output_with_timeout(&spec, Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn bounded_process_output_accepts_exact_stdout_limit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("stdout");
        fs::write(&path, b"12345").unwrap();
        assert_eq!(read_bounded_output(&path, 5, "stdout").unwrap(), b"12345");
    }

    #[test]
    fn bounded_process_output_rejects_stdout_one_byte_over_limit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("stdout");
        fs::write(&path, b"123456").unwrap();
        let error = read_bounded_output(&path, 5, "stdout").unwrap_err();
        assert!(error.contains("stdout exceeded output limit of 5 bytes"));
    }

    #[test]
    fn bounded_process_output_rejects_stderr_one_byte_over_limit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("stderr");
        fs::write(&path, b"123456").unwrap();
        let error = read_bounded_output(&path, 5, "stderr").unwrap_err();
        assert!(error.contains("stderr exceeded output limit of 5 bytes"));
    }

    #[test]
    fn bounded_runner_fails_fast_when_stdout_overflows_while_running() {
        let started = Instant::now();
        let error = TimeoutProcessRunner::with_limits(Duration::from_secs(30), 1024)
            .output(&process_helper_spec("stdout-hang:65536"))
            .unwrap_err();
        assert!(error.contains("stdout exceeded output limit of 1024 bytes"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn bounded_runner_fails_fast_when_stderr_overflows_while_running() {
        let started = Instant::now();
        let error = TimeoutProcessRunner::with_limits(Duration::from_secs(30), 1024)
            .output(&process_helper_spec("stderr-hang:65536"))
            .unwrap_err();
        assert!(error.contains("stderr exceeded output limit of 1024 bytes"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn bounded_runner_accepts_empty_output_with_zero_limit() {
        let directory = tempfile::tempdir().unwrap();
        let stdout_path = directory.path().join("stdout");
        fs::write(&stdout_path, []).unwrap();
        assert!(read_checked_output(&stdout_path, 0, "stdout")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn bounded_output_reports_missing_metadata_as_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing");
        let error = read_checked_output(&missing, 5, "stdout").unwrap_err();
        assert!(error.contains("failed to inspect process stdout"));
    }

    fn process_helper_spec(mode: &str) -> ProcessSpec {
        ProcessSpec::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process::tests::process_output_test_helper",
                "--nocapture",
            ])
            .env("SESSIONATLAS_PROCESS_TEST_HELPER", mode)
    }

    #[test]
    fn process_output_test_helper() {
        let Ok(mode) = std::env::var("SESSIONATLAS_PROCESS_TEST_HELPER") else {
            return;
        };
        if mode == "hang" {
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        let (stream, count) = mode
            .split_once(':')
            .and_then(|(stream, count)| count.parse::<usize>().ok().map(|count| (stream, count)))
            .unwrap_or(("", 0));
        let keep_alive = stream.ends_with("-hang");
        let stream = stream.strip_suffix("-hang").unwrap_or(stream);
        if stream == "stdout" || stream == "stderr" {
            let bytes = vec![b'x'; count];
            use std::io::Write;
            if stream == "stdout" {
                let mut output = std::io::stdout();
                output.write_all(&bytes).unwrap();
                output.flush().unwrap();
            } else {
                let mut output = std::io::stderr();
                output.write_all(&bytes).unwrap();
                output.flush().unwrap();
            }
            if keep_alive {
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
            std::process::exit(0);
        }
    }

    #[cfg(windows)]
    #[test]
    fn background_runner_executes_cmd_shims_without_a_console_window() {
        let directory = tempfile::tempdir().unwrap();
        let shim = directory.path().join("probe.cmd");
        fs::write(&shim, "@echo off\r\necho shim-ok\r\n").unwrap();

        let output = SystemProcessRunner.output(&ProcessSpec::new(shim)).unwrap();
        assert!(output.success);
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "shim-ok");
    }

    #[cfg(unix)]
    #[test]
    fn captured_output_files_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("captured-output");
        drop(create_private_output_file(&path).unwrap());
        let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
