use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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
}

impl TimeoutProcessRunner {
    pub(crate) const fn new(timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl ProcessRunner for TimeoutProcessRunner {
    fn output(&self, spec: &ProcessSpec) -> Result<ProcessOutput, String> {
        output_with_timeout(spec, self.timeout)
    }

    fn spawn(&self, spec: &ProcessSpec) -> Result<(), String> {
        SystemProcessRunner.spawn(spec)
    }
}

const MAX_CAPTURED_OUTPUT_BYTES: usize = 256 * 1024;
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
/// descendants (for example an SSH helper) inheriting a pipe handle. The
/// caller receives a bounded error on timeout; temporary files are cleaned up
/// on every path where the OS permits it.
pub(crate) fn output_with_timeout(
    spec: &ProcessSpec,
    timeout: Duration,
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
            let _ = fs::remove_file(&stdout_path);
            return Err(format!("failed to create process stderr file: {error}"));
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
            let _ = fs::remove_file(&stdout_path);
            let _ = fs::remove_file(&stderr_path);
            return Err(error.to_string());
        }
    };

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(&stdout_path);
                let _ = fs::remove_file(&stderr_path);
                return Err(format!(
                    "process timed out after {} ms",
                    timeout.as_millis()
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(&stdout_path);
                let _ = fs::remove_file(&stderr_path);
                return Err(format!("failed to wait for process: {error}"));
            }
        }
    };

    let stdout = read_bounded_output(&stdout_path);
    let stderr = read_bounded_output(&stderr_path);
    let _ = fs::remove_file(&stdout_path);
    let _ = fs::remove_file(&stderr_path);
    Ok(ProcessOutput {
        success: status.success(),
        status_code: status.code(),
        stdout,
        stderr,
    })
}

fn read_bounded_output(path: &Path) -> Vec<u8> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let mut output = Vec::new();
    let mut limited = (&mut file).take(MAX_CAPTURED_OUTPUT_BYTES as u64);
    let _ = limited.read_to_end(&mut output);
    output
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
    ProcessSpec::new("git")
        .arg("-C")
        .arg(path.as_os_str())
        .args(args)
}

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
        assert_eq!(spec.args[2], OsString::from("log"));
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
        #[cfg(windows)]
        let spec = ProcessSpec::new("cmd").args(["/C", "ping", "-n", "20", "127.0.0.1"]);
        #[cfg(not(windows))]
        let spec = ProcessSpec::new("sh").args(["-c", "sleep 20"]);
        let started = std::time::Instant::now();
        let error = output_with_timeout(&spec, Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(5));
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
