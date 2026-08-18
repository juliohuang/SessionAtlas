//! Contract tests for `sessionatlas_core::indexer` (task R06).
//!
//! These tests build synthetic project trees with the `tempfile` crate only —
//! they never read the real `~/.sessionatlas`, never touch the OS `Temp`
//! directory directly, and never launch a real git process.

use std::path::Path;

use chrono::{DateTime, Utc};
use sessionatlas_core::indexer::{build_index, read_git_metadata, GitMetadata, IndexedToolScan};
use sessionatlas_core::scanner::ScannedProject;

fn rfc3339(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

fn scanned(
    path: &str,
    at: DateTime<Utc>,
    session_id: Option<&str>,
    branch: Option<&str>,
) -> ScannedProject {
    ScannedProject {
        path: path.to_string(),
        last_accessed_at: at,
        session_id: session_id.map(str::to_string),
        git_branch: branch.map(str::to_string),
    }
}

fn tool(key: &str, name: &str, projects: Vec<ScannedProject>) -> IndexedToolScan {
    IndexedToolScan {
        tool_key: key.to_string(),
        tool_name: name.to_string(),
        projects,
    }
}

/// Creates the project directory (no `.git`) under a temp root.
fn make_project_dir(root: &Path, name: &str) -> std::path::PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes a `.git` directory with the given `HEAD` content.
fn make_git_dir(root: &Path, name: &str, head: &str) -> std::path::PathBuf {
    let dir = make_project_dir(root, name);
    let git = dir.join(".git");
    std::fs::create_dir_all(&git).unwrap();
    std::fs::write(git.join("HEAD"), head).unwrap();
    dir
}

/// Appends a git remote to the repo's `config`.
fn add_git_remote(project_dir: &Path, remote_section: &str, url: &str) {
    let config = project_dir.join(".git").join("config");
    let mut content = std::fs::read_to_string(&config).unwrap_or_default();
    content.push_str(remote_section);
    content.push('\n');
    content.push_str(&format!("url = {url}\n"));
    std::fs::write(config, content).unwrap();
}

#[test]
fn indexer_merges_tools_and_counts_distinct_sessions_per_project() {
    let root = tempfile::tempdir().unwrap();
    let project_path = make_project_dir(root.path(), "sample-project");
    let path = project_path.to_string_lossy().into_owned();
    let t1 = rfc3339("2026-07-29T10:00:00Z");
    let t2 = rfc3339("2026-07-30T10:00:00Z");

    let result = build_index(&[
        tool(
            "claude",
            "Claude Code",
            vec![
                scanned(&path, t1, Some("claude-1"), None),
                scanned(&path, t2, Some("claude-2"), None),
            ],
        ),
        tool(
            "codex",
            "Codex CLI",
            vec![scanned(&path, t1, Some("codex-1"), None)],
        ),
    ]);

    assert_eq!(result.len(), 1);
    let project = &result[0];
    assert_eq!(project.path, path);
    assert_eq!(project.last_accessed_at, t2);
    assert_eq!(project.tool_usages.len(), 2);
    let claude = project
        .tool_usages
        .iter()
        .find(|usage| usage.tool_key == "claude")
        .unwrap();
    let codex = project
        .tool_usages
        .iter()
        .find(|usage| usage.tool_key == "codex")
        .unwrap();
    assert_eq!(claude.session_count, 2);
    assert_eq!(codex.session_count, 1);
}

#[test]
fn indexer_counts_distinct_native_session_ids_and_keeps_latest_identity() {
    let root = tempfile::tempdir().unwrap();
    let project_path = make_project_dir(root.path(), "sample-project");
    let path = project_path.to_string_lossy().into_owned();
    let older = rfc3339("2026-07-29T10:00:00Z");
    let newer = rfc3339("2026-07-30T10:00:00Z");

    let result = build_index(&[tool(
        "codex",
        "Codex CLI",
        vec![
            scanned(&path, newer, Some("latest"), None),
            scanned(&path, older, Some("duplicate"), None),
            scanned(&path, older, Some("duplicate"), None),
        ],
    )]);

    let usage = &result[0].tool_usages[0];
    assert_eq!(usage.session_count, 2);
    assert_eq!(usage.last_session_id.as_deref(), Some("latest"));
    assert_eq!(usage.last_used_at, newer);
    assert_eq!(result[0].last_accessed_at, newer);
}

#[test]
fn indexer_reports_zero_known_sessions_when_source_has_no_native_session_id() {
    let root = tempfile::tempdir().unwrap();
    let project_path = make_project_dir(root.path(), "sample-project");
    let path = project_path.to_string_lossy().into_owned();

    let result = build_index(&[tool(
        "aider",
        "Aider",
        vec![scanned(&path, rfc3339("2026-07-30T10:00:00Z"), None, None)],
    )]);

    assert_eq!(result.len(), 1);
    let usage = &result[0].tool_usages[0];
    assert_eq!(usage.session_count, 0);
    assert_eq!(usage.last_session_id, None);
}

#[test]
fn indexer_keeps_latest_resumable_identity_when_newer_activity_has_no_session_id() {
    let root = tempfile::tempdir().unwrap();
    let project_path = make_project_dir(root.path(), "sample-project");
    let path = project_path.to_string_lossy().into_owned();
    let older = rfc3339("2026-07-29T10:00:00Z");
    let newer = rfc3339("2026-07-30T10:00:00Z");

    for observations in [
        vec![
            scanned(&path, newer, None, None),
            scanned(&path, older, Some("main-session"), None),
        ],
        vec![
            scanned(&path, older, Some("main-session"), None),
            scanned(&path, newer, None, None),
        ],
    ] {
        let result = build_index(&[tool("opencode", "OpenCode", observations)]);
        let usage = &result[0].tool_usages[0];
        assert_eq!(usage.last_used_at, newer, "activity time remains current");
        assert_eq!(usage.session_count, 1);
        assert_eq!(
            usage.last_session_id.as_deref(),
            Some("main-session"),
            "activity-only observations must not erase the resume target"
        );
    }
}

#[test]
fn indexer_blank_session_ids_are_not_counted() {
    let root = tempfile::tempdir().unwrap();
    let project_path = make_project_dir(root.path(), "sample-project");
    let path = project_path.to_string_lossy().into_owned();
    let at = rfc3339("2026-07-30T10:00:00Z");

    let result = build_index(&[tool(
        "kimi",
        "Kimi",
        vec![
            scanned(&path, at, Some("   "), None),
            scanned(&path, at, Some("real"), None),
        ],
    )]);

    let usage = &result[0].tool_usages[0];
    assert_eq!(usage.session_count, 1);
    assert_eq!(usage.last_session_id.as_deref(), Some("real"));

    let blank_only = build_index(&[tool(
        "kimi",
        "Kimi",
        vec![scanned(&path, at, Some("   "), None)],
    )]);
    let usage = &blank_only[0].tool_usages[0];
    assert_eq!(usage.session_count, 0);
    assert_eq!(usage.last_session_id, None);
}

#[test]
fn indexer_same_timestamp_session_id_breaks_ties() {
    let root = tempfile::tempdir().unwrap();
    let project_path = make_project_dir(root.path(), "sample-project");
    let path = project_path.to_string_lossy().into_owned();
    let at = rfc3339("2026-07-30T10:00:00Z");

    let result = build_index(&[tool(
        "codex",
        "Codex CLI",
        vec![
            scanned(&path, at, Some("a"), None),
            scanned(&path, at, Some("b"), None),
        ],
    )]);

    let usage = &result[0].tool_usages[0];
    assert_eq!(usage.session_count, 2);
    assert_eq!(usage.last_session_id.as_deref(), Some("b"));

    let reversed = build_index(&[tool(
        "codex",
        "Codex CLI",
        vec![
            scanned(&path, at, Some("b"), None),
            scanned(&path, at, Some("a"), None),
        ],
    )]);
    assert_eq!(
        reversed[0].tool_usages[0].last_session_id.as_deref(),
        Some("b")
    );
}

#[test]
fn indexer_merges_same_tool_with_whitespace_and_relative_paths_skipped() {
    let root = tempfile::tempdir().unwrap();
    let project_path = make_project_dir(root.path(), "sample-project");
    let path = project_path.to_string_lossy().into_owned();
    let at = rfc3339("2026-07-30T10:00:00Z");

    let result = build_index(&[tool(
        "codex",
        "Codex CLI",
        vec![
            scanned("", at, Some("x"), None),
            scanned("   ", at, Some("x"), None),
            scanned("relative/path", at, Some("x"), None),
            scanned(&path, at, Some("valid"), None),
        ],
    )]);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].path, path);
    assert_eq!(result[0].tool_usages[0].session_count, 1);
    assert_eq!(
        result[0].tool_usages[0].last_session_id.as_deref(),
        Some("valid")
    );
}

#[cfg(windows)]
#[test]
fn indexer_merges_projects_across_path_case_variants_on_windows() {
    let root = tempfile::tempdir().unwrap();
    let project_path = make_project_dir(root.path(), "sample-project");
    let upper = project_path
        .to_string_lossy()
        .replace("sample-project", "SAMPLE-PROJECT");
    let lower = project_path.to_string_lossy().into_owned();
    let older = rfc3339("2026-07-29T10:00:00Z");
    let newer = rfc3339("2026-07-30T10:00:00Z");

    let result = build_index(&[tool(
        "claude",
        "Claude Code",
        vec![
            scanned(&upper, newer, Some("one"), None),
            scanned(&lower, older, Some("two"), None),
        ],
    )]);

    assert_eq!(result.len(), 1, "case variants must merge into one project");
    assert_eq!(result[0].tool_usages[0].session_count, 2);
    assert_eq!(result[0].last_accessed_at, newer);
}

#[test]
fn indexer_tool_key_matching_is_case_insensitive() {
    let root = tempfile::tempdir().unwrap();
    let project_path = make_project_dir(root.path(), "sample-project");
    let path = project_path.to_string_lossy().into_owned();
    let t1 = rfc3339("2026-07-29T10:00:00Z");
    let t2 = rfc3339("2026-07-30T10:00:00Z");

    let result = build_index(&[
        tool(
            "Codex",
            "Codex CLI",
            vec![scanned(&path, t2, Some("codex-1"), None)],
        ),
        tool(
            "codex",
            "Codex CLI",
            vec![scanned(&path, t1, Some("codex-2"), None)],
        ),
    ]);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].tool_usages.len(), 1, "one usage per case variant");
    let usage = &result[0].tool_usages[0];
    assert_eq!(usage.tool_key, "Codex", "first-seen key is preserved");
    assert_eq!(usage.session_count, 2);
}

#[test]
fn indexer_sorts_projects_by_last_accessed_descending() {
    let root = tempfile::tempdir().unwrap();
    let older_path = make_project_dir(root.path(), "older-project");
    let newer_path = make_project_dir(root.path(), "newer-project");
    let older = rfc3339("2026-07-29T10:00:00Z");
    let newer = rfc3339("2026-07-30T10:00:00Z");

    let result = build_index(&[tool(
        "codex",
        "Codex CLI",
        vec![
            scanned(&older_path.to_string_lossy(), older, Some("older"), None),
            scanned(&newer_path.to_string_lossy(), newer, Some("newer"), None),
        ],
    )]);

    assert_eq!(result.len(), 2);
    assert_eq!(result[0].last_accessed_at, newer);
    assert_eq!(result[1].last_accessed_at, older);
}

#[test]
fn indexer_git_branch_merge_follows_latest_observation() {
    let root = tempfile::tempdir().unwrap();
    let project_path = make_project_dir(root.path(), "sample-project");
    let path = project_path.to_string_lossy().into_owned();
    let older = rfc3339("2026-07-29T10:00:00Z");
    let newer = rfc3339("2026-07-30T10:00:00Z");

    let keeps_scanner_branch = build_index(&[
        tool(
            "claude",
            "Claude Code",
            vec![scanned(&path, older, Some("c1"), Some("old-branch"))],
        ),
        tool(
            "codex",
            "Codex CLI",
            vec![scanned(&path, newer, Some("x1"), None)],
        ),
    ]);
    assert_eq!(
        keeps_scanner_branch[0].git_branch.as_deref(),
        Some("old-branch"),
        "a later None observation keeps the earlier branch"
    );

    let takes_newer_branch = build_index(&[
        tool(
            "claude",
            "Claude Code",
            vec![scanned(&path, older, Some("c1"), None)],
        ),
        tool(
            "codex",
            "Codex CLI",
            vec![scanned(&path, newer, Some("x1"), Some("new-branch"))],
        ),
    ]);
    assert_eq!(
        takes_newer_branch[0].git_branch.as_deref(),
        Some("new-branch")
    );
}

#[test]
fn indexer_reads_git_head_symbolic_branch() {
    let root = tempfile::tempdir().unwrap();
    let project_path = make_git_dir(root.path(), "sample-project", "ref: refs/heads/main\n");
    let path = project_path.to_string_lossy().into_owned();

    let result = build_index(&[tool(
        "codex",
        "Codex CLI",
        vec![scanned(
            &path,
            rfc3339("2026-07-30T10:00:00Z"),
            Some("x1"),
            None,
        )],
    )]);

    assert_eq!(result[0].git_branch.as_deref(), Some("main"));
    assert_eq!(result[0].git_remote_url, None);
}

#[test]
fn indexer_reads_detached_head_short_hash() {
    let root = tempfile::tempdir().unwrap();
    let hash = "0123456789abcdef0123456789abcdef01234567";
    let project_path = make_git_dir(root.path(), "sample-project", hash);
    let path = project_path.to_string_lossy().into_owned();

    let result = build_index(&[tool(
        "codex",
        "Codex CLI",
        vec![scanned(
            &path,
            rfc3339("2026-07-30T10:00:00Z"),
            Some("x1"),
            None,
        )],
    )]);

    assert_eq!(result[0].git_branch.as_deref(), Some(&hash[..7]));

    let short_root = tempfile::tempdir().unwrap();
    let short_path = make_git_dir(short_root.path(), "short-project", "abc");
    let short_result = build_index(&[tool(
        "codex",
        "Codex CLI",
        vec![scanned(
            &short_path.to_string_lossy(),
            rfc3339("2026-07-30T10:00:00Z"),
            Some("x1"),
            None,
        )],
    )]);
    assert_eq!(short_result[0].git_branch.as_deref(), Some("abc"));
}

#[test]
fn indexer_reads_git_remote_url_from_config() {
    let root = tempfile::tempdir().unwrap();
    let project_path = make_git_dir(root.path(), "sample-project", "ref: refs/heads/main\n");
    add_git_remote(
        &project_path,
        "[remote \"origin\"]",
        "https://github.com/acme/repo.git",
    );
    let path = project_path.to_string_lossy().into_owned();

    let result = build_index(&[tool(
        "codex",
        "Codex CLI",
        vec![scanned(
            &path,
            rfc3339("2026-07-30T10:00:00Z"),
            Some("x1"),
            None,
        )],
    )]);

    assert_eq!(
        result[0].git_remote_url.as_deref(),
        Some("https://github.com/acme/repo.git")
    );
    assert_eq!(result[0].git_branch.as_deref(), Some("main"));
}

#[test]
fn indexer_uses_first_remote_section_url_and_supports_quoted_values() {
    let root = tempfile::tempdir().unwrap();
    let project_path = make_git_dir(root.path(), "sample-project", "ref: refs/heads/main\n");
    add_git_remote(
        &project_path,
        "[remote \"origin\"]",
        "\"https://github.com/acme/repo with spaces.git\"",
    );
    add_git_remote(
        &project_path,
        "[remote \"backup\"]",
        "https://github.com/acme/backup.git",
    );
    let path = project_path.to_string_lossy().into_owned();

    let result = build_index(&[tool(
        "codex",
        "Codex CLI",
        vec![scanned(
            &path,
            rfc3339("2026-07-30T10:00:00Z"),
            Some("x1"),
            None,
        )],
    )]);

    assert_eq!(
        result[0].git_remote_url.as_deref(),
        Some("https://github.com/acme/repo with spaces.git")
    );
}

#[test]
fn indexer_supports_git_file_worktree_form() {
    let root = tempfile::tempdir().unwrap();
    let project_dir = make_project_dir(root.path(), "sample-project");
    let worktree_git = project_dir.join("worktrees").join("main");
    std::fs::create_dir_all(&worktree_git).unwrap();
    std::fs::write(worktree_git.join("HEAD"), "ref: refs/heads/dev\n").unwrap();
    std::fs::write(
        worktree_git.join("config"),
        "[remote \"origin\"]\nurl = git@github.com:acme/repo.git\n",
    )
    .unwrap();
    std::fs::write(project_dir.join(".git"), "gitdir: worktrees/main\n").unwrap();

    let path = project_dir.to_string_lossy().into_owned();
    let result = build_index(&[tool(
        "claude",
        "Claude Code",
        vec![scanned(
            &path,
            rfc3339("2026-07-30T10:00:00Z"),
            Some("c1"),
            None,
        )],
    )]);

    assert_eq!(result[0].git_branch.as_deref(), Some("dev"));
    assert_eq!(
        result[0].git_remote_url.as_deref(),
        Some("git@github.com:acme/repo.git")
    );
}

#[test]
fn indexer_supports_absolute_gitdir_in_worktree_file() {
    let root = tempfile::tempdir().unwrap();
    let project_dir = make_project_dir(root.path(), "sample-project");
    let worktree_git = root.path().join("linked-git");
    std::fs::create_dir_all(&worktree_git).unwrap();
    std::fs::write(worktree_git.join("HEAD"), "ref: refs/heads/topic\n").unwrap();
    std::fs::write(
        project_dir.join(".git"),
        format!("gitdir: {}\n", worktree_git.to_string_lossy()),
    )
    .unwrap();

    let metadata = read_git_metadata(&project_dir);
    assert_eq!(metadata.branch.as_deref(), Some("topic"));
}

#[test]
fn indexer_reads_remote_from_realistic_worktree_commondir() {
    let root = tempfile::tempdir().unwrap();
    let project_dir = make_project_dir(root.path(), "linked-worktree");
    let common_git = root.path().join("main-repo.git");
    let worktree_git = common_git.join("worktrees").join("linked");
    std::fs::create_dir_all(&worktree_git).unwrap();
    std::fs::write(worktree_git.join("HEAD"), "ref: refs/heads/linked\n").unwrap();
    std::fs::write(worktree_git.join("commondir"), "../..\n").unwrap();
    std::fs::write(
        common_git.join("config"),
        "[remote \"origin\"]\nurl = https://github.com/acme/worktree.git\n",
    )
    .unwrap();
    std::fs::write(
        project_dir.join(".git"),
        format!("gitdir: {}\n", worktree_git.to_string_lossy()),
    )
    .unwrap();

    let metadata = read_git_metadata(&project_dir);

    assert_eq!(metadata.branch.as_deref(), Some("linked"));
    assert_eq!(
        metadata.remote_url.as_deref(),
        Some("https://github.com/acme/worktree.git")
    );
}

#[test]
fn indexer_degrades_gracefully_when_git_metadata_is_missing() {
    let root = tempfile::tempdir().unwrap();
    let project_path = make_project_dir(root.path(), "sample-project");
    let path = project_path.to_string_lossy().into_owned();

    let result = build_index(&[tool(
        "aider",
        "Aider",
        vec![scanned(&path, rfc3339("2026-07-30T10:00:00Z"), None, None)],
    )]);

    assert_eq!(result[0].git_branch, None);
    assert_eq!(result[0].git_remote_url, None);

    let empty_metadata = read_git_metadata(&root.path().join("does-not-exist"));
    assert_eq!(empty_metadata, GitMetadata::default());
}

#[test]
fn indexer_degrades_gracefully_on_unreadable_or_invalid_git_shape() {
    let root = tempfile::tempdir().unwrap();

    let broken_git = make_project_dir(root.path(), "broken-git");
    std::fs::write(broken_git.join(".git"), "not a gitdir pointer\n").unwrap();
    let metadata = read_git_metadata(&broken_git);
    assert_eq!(metadata, GitMetadata::default());

    let dir_git = make_project_dir(root.path(), "empty-git-dir");
    std::fs::create_dir_all(dir_git.join(".git")).unwrap();
    let metadata = read_git_metadata(&dir_git);
    assert_eq!(metadata, GitMetadata::default());

    let no_head = make_project_dir(root.path(), "no-head");
    std::fs::create_dir_all(no_head.join(".git")).unwrap();
    let metadata = read_git_metadata(&no_head);
    assert_eq!(metadata, GitMetadata::default());
}

#[test]
fn indexer_git_head_read_overrides_scanner_branch() {
    let root = tempfile::tempdir().unwrap();
    let project_path = make_git_dir(root.path(), "sample-project", "ref: refs/heads/main\n");
    let path = project_path.to_string_lossy().into_owned();

    let result = build_index(&[tool(
        "codex",
        "Codex CLI",
        vec![scanned(
            &path,
            rfc3339("2026-07-30T10:00:00Z"),
            Some("x1"),
            Some("scanner-branch"),
        )],
    )]);

    assert_eq!(
        result[0].git_branch.as_deref(),
        Some("main"),
        "the on-disk HEAD wins over the scanner-provided branch"
    );
}
