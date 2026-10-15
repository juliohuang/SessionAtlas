//! Bounded, read-only Git status and diff inspection for the desktop review UI.
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::process::{git_read_spec, ProcessOutput, ProcessRunner, TimeoutProcessRunner};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(8);
const STATUS_OUTPUT_LIMIT: usize = 1024 * 1024;
const DIFF_OUTPUT_LIMIT: usize = 1024 * 1024;
const UNTRACKED_PREVIEW_LIMIT: usize = 256 * 1024;
const MAX_CHANGED_FILES: usize = 2_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GitReview {
    pub(crate) files: Vec<GitReviewFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GitReviewFile {
    pub(crate) path: String,
    pub(crate) status: String,
    pub(crate) staged: bool,
    pub(crate) unstaged: bool,
    pub(crate) untracked: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GitDiff {
    pub(crate) path: String,
    pub(crate) staged: bool,
    pub(crate) text: String,
    pub(crate) truncated: bool,
    pub(crate) binary: bool,
}

#[derive(Clone, Debug)]
struct RepositoryContext {
    project: PathBuf,
    root: PathBuf,
}

#[derive(Clone, Debug, Default)]
struct Change {
    index_status: u8,
    worktree_status: u8,
    untracked: bool,
    previous_path: Option<String>,
}

pub(crate) fn list_changes(project_path: &str) -> Result<GitReview, String> {
    list_changes_with(
        &TimeoutProcessRunner::with_limits(COMMAND_TIMEOUT, STATUS_OUTPUT_LIMIT),
        project_path,
    )
}

pub(crate) fn read_diff(
    project_path: &str,
    file_path: &str,
    staged: bool,
) -> Result<GitDiff, String> {
    read_diff_with(
        &TimeoutProcessRunner::with_limits(COMMAND_TIMEOUT, DIFF_OUTPUT_LIMIT),
        project_path,
        file_path,
        staged,
    )
}

fn list_changes_with(runner: &dyn ProcessRunner, project_path: &str) -> Result<GitReview, String> {
    let context = repository_context(runner, project_path)?;
    let parsed = status_changes(runner, &context)?;
    Ok(GitReview {
        files: parsed
            .into_iter()
            .map(|(path, change)| GitReviewFile {
                path,
                status: String::from_utf8_lossy(&[change.index_status, change.worktree_status])
                    .into_owned(),
                staged: change.index_status != b' ' && change.index_status != b'?',
                unstaged: change.worktree_status != b' ' && change.worktree_status != b'?',
                untracked: change.untracked,
            })
            .collect(),
    })
}

fn status_changes(
    runner: &dyn ProcessRunner,
    context: &RepositoryContext,
) -> Result<BTreeMap<String, Change>, String> {
    let output = run_git(
        runner,
        &context.project,
        &[
            "--no-pager",
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
        ],
    )?;
    let parsed = parse_status(&output.stdout)?;
    if parsed.len() > MAX_CHANGED_FILES {
        return Err("Too many changed files to review at once".into());
    }
    Ok(parsed)
}

fn read_diff_with(
    runner: &dyn ProcessRunner,
    project_path: &str,
    file_path: &str,
    staged: bool,
) -> Result<GitDiff, String> {
    let context = repository_context(runner, project_path)?;
    let changes = status_changes(runner, &context)?;
    let selected = changes
        .get(file_path)
        .ok_or_else(|| "That file is no longer in the Git change list".to_string())?;
    let pathspec = top_literal_pathspec(file_path)?;

    if selected.untracked && !staged {
        return untracked_preview(&context, file_path);
    }

    let mut args = vec![
        "--no-pager",
        "--no-optional-locks",
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
    ];
    if staged {
        args.push("--cached");
    }
    args.push("--");
    let mut spec = git_read_spec(&context.root, &args);
    spec.args.push(OsString::from(pathspec));
    if let Some(previous_path) = &selected.previous_path {
        spec.args
            .push(OsString::from(top_literal_pathspec(previous_path)?));
    }
    let output = runner
        .output(&spec)
        .map_err(|_| "Could not read this Git diff within the time and size limits".to_string())?;
    let output = ensure_git_success(output, "Git could not read this diff")?;
    let binary = is_binary_diff(&output.stdout);
    let text = if binary {
        "Binary file changes are not shown.".to_string()
    } else {
        String::from_utf8(output.stdout)
            .map_err(|_| "Git returned non-text diff data".to_string())?
    };
    Ok(GitDiff {
        path: file_path.to_string(),
        staged,
        text,
        truncated: false,
        binary,
    })
}

fn repository_context(
    runner: &dyn ProcessRunner,
    project_path: &str,
) -> Result<RepositoryContext, String> {
    let project = Path::new(project_path);
    if !project.is_absolute() || !project.is_dir() {
        return Err("Project directory is unavailable".into());
    }
    let project = fs::canonicalize(project).map_err(|_| "Project directory is unavailable")?;
    let root_output = run_git(
        runner,
        &project,
        &[
            "--no-pager",
            "--no-optional-locks",
            "rev-parse",
            "--show-toplevel",
        ],
    )?;
    let root_text = String::from_utf8(root_output.stdout)
        .map_err(|_| "Git returned an invalid repository path".to_string())?;
    let root_text = root_text.trim_end_matches(['\r', '\n']);
    let root = PathBuf::from(root_text);
    if !root.is_absolute() || !root.is_dir() {
        return Err("Git repository directory is unavailable".into());
    }
    let root = fs::canonicalize(root).map_err(|_| "Git repository directory is unavailable")?;
    if !project.starts_with(&root) {
        return Err("Project directory is outside its Git repository".into());
    }
    Ok(RepositoryContext { project, root })
}

fn run_git(
    runner: &dyn ProcessRunner,
    path: &Path,
    args: &[&str],
) -> Result<ProcessOutput, String> {
    let output = runner
        .output(&git_read_spec(path, args))
        .map_err(|_| "Git review timed out or exceeded its output limit".to_string())?;
    ensure_git_success(output, "Git review could not read this repository")
}

fn ensure_git_success(output: ProcessOutput, message: &str) -> Result<ProcessOutput, String> {
    if output.success {
        Ok(output)
    } else {
        Err(message.to_string())
    }
}

fn parse_status(bytes: &[u8]) -> Result<BTreeMap<String, Change>, String> {
    let mut records = bytes.split(|byte| *byte == 0).peekable();
    let mut changes = BTreeMap::<String, Change>::new();
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        if record.len() < 4 || record[2] != b' ' {
            return Err("Git returned malformed status data".into());
        }
        let index_status = record[0];
        let worktree_status = record[1];
        let path = std::str::from_utf8(&record[3..])
            .map_err(|_| "A changed filename cannot be represented safely".to_string())?
            .to_string();
        if path.is_empty() {
            return Err("Git returned an empty changed filename".into());
        }
        let previous_path = if index_status == b'R'
            || index_status == b'C'
            || worktree_status == b'R'
            || worktree_status == b'C'
        {
            // In porcelain v1 -z output the second NUL-terminated pathname is the
            // rename/copy source. It is metadata for the destination row.
            let source = records
                .next()
                .filter(|source| !source.is_empty())
                .ok_or_else(|| "Git returned incomplete rename status data".to_string())?;
            Some(
                std::str::from_utf8(source)
                    .map_err(|_| "A changed filename cannot be represented safely".to_string())?
                    .to_string(),
            )
        } else {
            None
        };
        let change = changes.entry(path).or_default();
        change.index_status = index_status;
        change.worktree_status = worktree_status;
        change.untracked = index_status == b'?' && worktree_status == b'?';
        change.previous_path = previous_path;
    }
    Ok(changes)
}

fn top_literal_pathspec(file_path: &str) -> Result<String, String> {
    let path = Path::new(file_path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("Invalid Git file path".into());
    }
    if file_path.is_empty() || file_path.starts_with('/') {
        return Err("Invalid Git file path".into());
    }
    Ok(format!(":(top,literal){file_path}"))
}

fn untracked_preview(context: &RepositoryContext, file_path: &str) -> Result<GitDiff, String> {
    let relative = Path::new(file_path);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || is_sensitive_filename(relative)
    {
        return Ok(GitDiff {
            path: file_path.to_string(),
            staged: false,
            text: "Preview hidden for this file name.".into(),
            truncated: false,
            binary: false,
        });
    }
    let candidate = context.root.join(relative);
    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|_| "Untracked file is no longer available".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Only regular untracked files can be previewed".into());
    }
    let canonical = fs::canonicalize(&candidate)
        .map_err(|_| "Untracked file is no longer available".to_string())?;
    if !canonical.starts_with(&context.root) {
        return Err("Untracked file is outside its Git repository".into());
    }
    let mut bytes = Vec::with_capacity(UNTRACKED_PREVIEW_LIMIT + 1);
    File::open(&canonical)
        .and_then(|file| {
            file.take((UNTRACKED_PREVIEW_LIMIT + 1) as u64)
                .read_to_end(&mut bytes)
        })
        .map_err(|_| "Could not read untracked file".to_string())?;
    let truncated = bytes.len() > UNTRACKED_PREVIEW_LIMIT;
    let shown = &bytes[..bytes.len().min(UNTRACKED_PREVIEW_LIMIT)];
    let text = if bytes.contains(&0) {
        None
    } else if truncated {
        match std::str::from_utf8(shown) {
            Ok(text) => Some(text),
            Err(error) if error.error_len().is_none() => {
                std::str::from_utf8(&shown[..error.valid_up_to()]).ok()
            }
            Err(_) => None,
        }
    } else {
        std::str::from_utf8(shown).ok()
    };
    let Some(text) = text else {
        return Ok(GitDiff {
            path: file_path.to_string(),
            staged: false,
            text: "Binary file changes are not shown.".into(),
            truncated: false,
            binary: true,
        });
    };
    let mut text = text.to_string();
    if truncated {
        text.push_str("\n\n[Preview truncated at 256 KiB]");
    }
    Ok(GitDiff {
        path: file_path.to_string(),
        staged: false,
        text,
        truncated,
        binary: false,
    })
}

fn is_sensitive_filename(path: &Path) -> bool {
    path.components().any(|component| {
        let Some(name) = component.as_os_str().to_str() else {
            return true;
        };
        let lower = name.to_ascii_lowercase();
        lower == ".npmrc"
            || lower == ".pypirc"
            || lower == ".netrc"
            || lower.starts_with(".env")
            || lower.starts_with("credentials.")
            || lower.starts_with("secrets.")
            || lower == "id_rsa"
            || lower == "id_ed25519"
            || [".pem", ".key", ".p12", ".pfx", ".jks", ".keystore"]
                .iter()
                .any(|suffix| lower.ends_with(suffix))
    })
}

fn is_binary_diff(bytes: &[u8]) -> bool {
    bytes
        .windows(b"Binary files ".len())
        .any(|window| window == b"Binary files ")
        || bytes
            .windows(b"GIT binary patch".len())
            .any(|window| window == b"GIT binary patch")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingRunner {
        specs: Mutex<Vec<crate::process::ProcessSpec>>,
        outputs: Mutex<Vec<Result<ProcessOutput, String>>>,
    }

    impl RecordingRunner {
        fn queue(&self, output: Result<ProcessOutput, String>) {
            self.outputs.lock().unwrap().push(output);
        }
    }

    impl ProcessRunner for RecordingRunner {
        fn output(&self, spec: &crate::process::ProcessSpec) -> Result<ProcessOutput, String> {
            self.specs.lock().unwrap().push(spec.clone());
            let mut outputs = self.outputs.lock().unwrap();
            if outputs.is_empty() {
                return Err("unexpected process call".into());
            }
            outputs.remove(0)
        }

        fn spawn(&self, _: &crate::process::ProcessSpec) -> Result<(), String> {
            Err("spawn is not used by Git review".into())
        }
    }

    fn ok(stdout: impl Into<Vec<u8>>) -> Result<ProcessOutput, String> {
        Ok(ProcessOutput {
            success: true,
            status_code: Some(0),
            stdout: stdout.into(),
            stderr: Vec::new(),
        })
    }

    #[test]
    fn parses_porcelain_nul_names_and_consumes_rename_source() {
        let parsed = parse_status(
            b" M line\nbreak.txt\0R  new name.txt\0old name.txt\0?? loose\nname.txt\0",
        )
        .unwrap();
        assert_eq!(parsed.len(), 3);
        assert!(parsed["line\nbreak.txt"].worktree_status == b'M');
        assert_eq!(parsed["new name.txt"].index_status, b'R');
        assert!(parsed.contains_key("loose\nname.txt"));
    }

    #[test]
    fn status_command_is_read_only_and_reports_changes() {
        let directory = tempfile::tempdir().unwrap();
        let runner = RecordingRunner::default();
        runner.queue(ok(format!("{}\n", directory.path().display()).into_bytes()));
        runner.queue(ok(b" M file.txt\0?? new.txt\0".to_vec()));
        let result = list_changes_with(&runner, directory.path().to_str().unwrap()).unwrap();
        assert_eq!(result.files.len(), 2);
        assert_eq!(result.files[0].path, "file.txt");
        assert!(result.files[0].unstaged);
        assert_eq!(result.files[1].path, "new.txt");
        assert!(result.files[1].untracked);
        let specs = runner.specs.lock().unwrap();
        assert!(specs
            .iter()
            .all(|spec| spec.args.iter().any(|arg| arg == "--no-optional-locks")));
        assert!(specs
            .iter()
            .all(|spec| !spec.args.iter().any(|arg| arg == "fetch")));
    }

    #[test]
    fn sensitive_untracked_names_are_hidden() {
        for name in [
            ".env.local",
            "credentials.json",
            "secrets.txt",
            "id_rsa",
            "token.pem",
        ] {
            assert!(is_sensitive_filename(Path::new(name)), "{name}");
        }
        assert!(!is_sensitive_filename(Path::new("notes.txt")));
        assert!(is_sensitive_filename(Path::new(".ssh/id_ed25519")));
    }

    #[test]
    fn actual_repository_review_handles_unborn_head_and_untracked_preview() {
        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("subdir");
        fs::create_dir(&nested).unwrap();
        let init = std::process::Command::new("git")
            .arg("init")
            .arg("--quiet")
            .current_dir(directory.path())
            .status()
            .unwrap();
        assert!(init.success());
        fs::write(nested.join("new file.txt"), "preview body\n").unwrap();
        let path = nested.to_str().unwrap();
        let review = list_changes(path).unwrap();
        assert_eq!(review.files.len(), 1);
        assert_eq!(review.files[0].path, "subdir/new file.txt");
        let diff = read_diff(path, "subdir/new file.txt", false).unwrap();
        assert_eq!(diff.text, "preview body\n");
        assert!(!diff.binary);
        assert!(!diff.truncated);
    }

    #[test]
    fn staged_rename_diff_includes_both_paths() {
        let directory = tempfile::tempdir().unwrap();
        let run_git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(directory.path())
                .env("GIT_AUTHOR_NAME", "SessionAtlas test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "SessionAtlas test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .output()
                .unwrap()
        };
        assert!(run_git(&["init", "--quiet"]).status.success());
        fs::write(directory.path().join("old.txt"), "rename body\n").unwrap();
        assert!(run_git(&["add", "old.txt"]).status.success());
        assert!(run_git(&["commit", "--quiet", "-m", "base"])
            .status
            .success());
        assert!(run_git(&["mv", "old.txt", "new.txt"]).status.success());

        let path = directory.path().to_str().unwrap();
        let review = list_changes(path).unwrap();
        assert_eq!(review.files.len(), 1);
        assert_eq!(review.files[0].path, "new.txt");
        let diff = read_diff(path, "new.txt", true).unwrap();
        assert!(diff.text.contains("a/old.txt"));
        assert!(diff.text.contains("b/new.txt"));
        assert!(diff.text.contains("rename from old.txt"));
        assert!(diff.text.contains("rename to new.txt"));
    }

    #[test]
    fn staged_and_unstaged_diffs_show_separate_file_states_without_writing_index() {
        let directory = tempfile::tempdir().unwrap();
        let run_git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(directory.path())
                .env("GIT_AUTHOR_NAME", "SessionAtlas test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "SessionAtlas test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .output()
                .unwrap()
        };
        assert!(run_git(&["init", "--quiet"]).status.success());
        fs::write(directory.path().join("file.txt"), "base\n").unwrap();
        assert!(run_git(&["add", "file.txt"]).status.success());
        assert!(run_git(&["commit", "--quiet", "-m", "base"])
            .status
            .success());
        fs::write(directory.path().join("file.txt"), "staged line\n").unwrap();
        assert!(run_git(&["add", "file.txt"]).status.success());
        fs::write(directory.path().join("file.txt"), "unstaged line\n").unwrap();

        let index_path = directory.path().join(".git").join("index");
        let index_before = fs::read(&index_path).unwrap();
        let status_before = run_git(&["status", "--porcelain=v1", "-z"]).stdout;
        let path = directory.path().to_str().unwrap();
        let review = list_changes(path).unwrap();
        assert_eq!(review.files.len(), 1);
        assert!(review.files[0].staged && review.files[0].unstaged);
        let staged_diff = read_diff(path, "file.txt", true).unwrap();
        let unstaged_diff = read_diff(path, "file.txt", false).unwrap();
        assert!(staged_diff.text.contains("+staged line"));
        assert!(!staged_diff.text.contains("unstaged line"));
        assert!(unstaged_diff.text.contains("+unstaged line"));
        assert!(unstaged_diff.text.contains("-staged line"));
        assert_eq!(fs::read(index_path).unwrap(), index_before);
        assert_eq!(
            run_git(&["status", "--porcelain=v1", "-z"]).stdout,
            status_before
        );
    }

    #[cfg(unix)]
    #[test]
    fn literal_magic_and_backslash_filenames_are_selected_exactly() {
        let directory = tempfile::tempdir().unwrap();
        let run_git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(directory.path())
                .env("GIT_AUTHOR_NAME", "SessionAtlas test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "SessionAtlas test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .output()
                .unwrap()
        };
        assert!(run_git(&["init", "--quiet"]).status.success());
        let magic_name = ":(glob)*.txt";
        let backslash_name = "back\\slash.txt";
        fs::write(directory.path().join(magic_name), "base magic\n").unwrap();
        fs::write(directory.path().join(backslash_name), "base slash\n").unwrap();
        assert!(run_git(&["add", "--", magic_name, backslash_name])
            .status
            .success());
        assert!(run_git(&["commit", "--quiet", "-m", "base"])
            .status
            .success());
        fs::write(directory.path().join(magic_name), "changed magic\n").unwrap();
        fs::write(directory.path().join(backslash_name), "changed slash\n").unwrap();

        let path = directory.path().to_str().unwrap();
        let magic_diff = read_diff(path, magic_name, false).unwrap();
        let backslash_diff = read_diff(path, backslash_name, false).unwrap();
        assert!(magic_diff.text.contains("+changed magic"));
        assert!(backslash_diff.text.contains("+changed slash"));
    }

    #[test]
    fn untracked_previews_handle_binary_symlink_and_utf8_boundary() {
        let directory = tempfile::tempdir().unwrap();
        let init = std::process::Command::new("git")
            .arg("init")
            .arg("--quiet")
            .current_dir(directory.path())
            .status()
            .unwrap();
        assert!(init.success());
        let root = fs::canonicalize(directory.path()).unwrap();
        let context = RepositoryContext {
            project: root.clone(),
            root,
        };

        fs::write(directory.path().join("binary.dat"), b"head\0tail").unwrap();
        let binary = untracked_preview(&context, "binary.dat").unwrap();
        assert!(binary.binary);
        assert_eq!(binary.text, "Binary file changes are not shown.");

        let mut large_text = vec![b'a'; UNTRACKED_PREVIEW_LIMIT - 1];
        large_text.extend_from_slice("中tail".as_bytes());
        fs::write(directory.path().join("large.txt"), large_text).unwrap();
        let large = untracked_preview(&context, "large.txt").unwrap();
        assert!(large.truncated);
        assert!(!large.binary);
        assert!(large.text.is_char_boundary(large.text.len()));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = directory
                .path()
                .parent()
                .unwrap()
                .join("git-review-outside");
            fs::write(&outside, "outside secret").unwrap();
            symlink(&outside, directory.path().join("linked.txt")).unwrap();
            assert!(untracked_preview(&context, "linked.txt").is_err());
            fs::remove_file(outside).unwrap();
        }
    }

    #[test]
    fn diff_output_limit_errors_are_reported() {
        let directory = tempfile::tempdir().unwrap();
        let runner = RecordingRunner::default();
        runner.queue(ok(format!("{}\n", directory.path().display()).into_bytes()));
        runner.queue(ok(b" M file.txt\0".to_vec()));
        runner.queue(Err("stdout exceeded output limit".into()));
        let error = read_diff_with(
            &runner,
            directory.path().to_str().unwrap(),
            "file.txt",
            false,
        )
        .unwrap_err();
        assert!(error.contains("time and size limits"));
        let specs = runner.specs.lock().unwrap();
        let diff_args: Vec<_> = specs
            .last()
            .unwrap()
            .args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect();
        assert!(diff_args.iter().any(|arg| arg == "--no-ext-diff"));
        assert!(diff_args.iter().any(|arg| arg == "--no-textconv"));
    }
}
