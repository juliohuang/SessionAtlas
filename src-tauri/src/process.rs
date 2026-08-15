use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessSpec {
    pub(crate) program: OsString,
    pub(crate) args: Vec<OsString>,
    pub(crate) current_dir: Option<PathBuf>,
}

impl ProcessSpec {
    pub(crate) fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            current_dir: None,
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

fn command_from_spec(spec: &ProcessSpec) -> Command {
    let mut command = Command::new(&spec.program);
    command.args(&spec.args);
    if let Some(current_dir) = &spec.current_dir {
        command.current_dir(current_dir);
    }
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
}
