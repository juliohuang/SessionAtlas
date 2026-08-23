//! Project indexer: merge scan results by native path semantics, deduplicate
//! session IDs per `(project, tool)`, and read git branch/remote metadata.
//!
//! Mirrors `Core/Indexer/ProjectIndexer.cs`. The indexer is injectable: it
//! consumes plain [`ScannedProject`] rows grouped by tool (`[IndexedToolScan]`)
//! and never touches `~/.sessionatlas`. Git metadata is read directly from the
//! repository's `.git` directory (or the `.git` worktree file form) without
//! launching a real git process, and degrades to `None` on any read failure.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::{DateTime, Utc};

use crate::model::{project_path_missing, Project, ToolUsage};
use crate::scanner::ScannedProject;

/// A scanned tool snapshot fed into the indexer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedToolScan {
    pub tool_key: String,
    pub tool_name: String,
    pub projects: Vec<ScannedProject>,
}

/// Git branch and remote URL read from a project's repository metadata.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitMetadata {
    pub branch: Option<String>,
    pub remote_url: Option<String>,
}

/// Merges scan results across tools into a project index.
///
/// Observations are grouped by native path identity (case-insensitive on
/// Windows); session IDs are deduplicated per `(project, tool)` using
/// case-insensitive tool keys. Only non-blank native session IDs are counted,
/// so a source without native IDs yields `session_count == 0`. Projects are
/// then enriched with git branch/remote metadata read from the working tree
/// and returned ordered by `last_accessed_at` descending (stable for ties).
pub fn build_index(tool_scans: &[IndexedToolScan]) -> Vec<Project> {
    let mut projects: Vec<Project> = Vec::new();
    let mut project_index: HashMap<String, usize> = HashMap::new();
    let mut session_ids: HashMap<(String, String), HashSet<String>> = HashMap::new();

    for tool in tool_scans {
        for result in &tool.projects {
            let Some(normalized_path) = normalize_index_path(&result.path) else {
                continue;
            };
            let last_accessed_at = result.last_accessed_at;
            let path_missing = project_path_missing(&normalized_path);
            let path_key = path_identity_key(&normalized_path);

            let project_idx = match project_index.get(&path_key) {
                Some(&index) => {
                    let project = &mut projects[index];
                    if last_accessed_at > project.last_accessed_at {
                        project.last_accessed_at = last_accessed_at;
                        if result.git_branch.is_some() {
                            project.git_branch = result.git_branch.clone();
                        }
                    }
                    index
                }
                None => {
                    let index = projects.len();
                    project_index.insert(path_key.clone(), index);
                    projects.push(Project {
                        id: uuid::Uuid::new_v4().simple().to_string(),
                        path: normalized_path,
                        path_missing,
                        last_accessed_at,
                        first_seen_at: Utc::now(),
                        git_branch: result.git_branch.clone(),
                        git_remote_url: None,
                        tool_usages: Vec::new(),
                    });
                    index
                }
            };

            let normalized_session_id = result
                .session_id
                .as_deref()
                .filter(|session_id| !session_id.trim().is_empty());
            let known = session_ids
                .entry((path_key, fold_case(&tool.tool_key)))
                .or_default();
            if let Some(session_id) = normalized_session_id {
                known.insert(session_id.to_string());
            }

            let project = &mut projects[project_idx];
            let existing = project
                .tool_usages
                .iter_mut()
                .find(|usage| usage.tool_key.eq_ignore_ascii_case(&tool.tool_key));
            if let Some(existing) = existing {
                existing.session_count = known.len() as i32;
                if is_later_observation(
                    last_accessed_at,
                    normalized_session_id,
                    existing.last_used_at,
                    existing.last_session_id.as_deref(),
                ) {
                    existing.last_used_at = last_accessed_at;
                    existing.last_session_id = normalized_session_id.map(str::to_string);
                }
            } else {
                project.tool_usages.push(ToolUsage {
                    tool_name: tool.tool_name.clone(),
                    tool_key: tool.tool_key.clone(),
                    last_used_at: last_accessed_at,
                    session_count: known.len() as i32,
                    last_session_id: normalized_session_id.map(str::to_string),
                });
            }
        }
    }

    for project in &mut projects {
        let metadata = read_git_metadata(Path::new(&project.path));
        if let Some(branch) = metadata.branch {
            project.git_branch = Some(branch);
        }
        if let Some(remote_url) = metadata.remote_url {
            project.git_remote_url = Some(remote_url);
        }
    }

    projects.sort_by_key(|project| std::cmp::Reverse(project.last_accessed_at));
    projects
}

/// Reads git branch and remote URL for a project working tree.
///
/// Accepts both the `.git` directory form and the linked-worktree `.git` file
/// form (`gitdir: <path>`, resolved relative to the project root). Any read,
/// parse or shape problem degrades to `None` for both fields.
pub fn read_git_metadata(project_dir: &Path) -> GitMetadata {
    let Some(git_dir) = resolve_git_dir(project_dir) else {
        return GitMetadata::default();
    };
    let common_git_dir = resolve_common_git_dir(&git_dir).unwrap_or_else(|| git_dir.clone());
    GitMetadata {
        branch: read_head(&git_dir.join("HEAD")),
        remote_url: read_remote_url(&common_git_dir.join("config")),
    }
}

/// Normalizes a candidate project path. Blank/relative/malformed input yields
/// `None`; a leading `~` is expanded through the sessionatlas home.
fn normalize_index_path(candidate: &str) -> Option<String> {
    if candidate.trim().is_empty() {
        return None;
    }
    let expanded = crate::scanner::expand_tilde(candidate).unwrap_or_else(|| candidate.to_string());
    crate::path::normalize_native(&expanded)
}

/// Map key for native path identity: case-folded on Windows, byte-exact on
/// Unix, matching `path::paths_equal`.
fn path_identity_key(path: &str) -> String {
    if cfg!(windows) {
        fold_case(path)
    } else {
        path.to_string()
    }
}

/// Unicode case folding closest to C# `OrdinalIgnoreCase`.
fn fold_case(value: &str) -> String {
    value.chars().flat_map(char::to_uppercase).collect()
}

/// Later-observation rule: strictly later time wins; on equal times the
/// ordinal-greater session ID wins (`None` sorts before any value).
fn is_later_observation(
    candidate_time: DateTime<Utc>,
    candidate_session_id: Option<&str>,
    current_time: DateTime<Utc>,
    current_session_id: Option<&str>,
) -> bool {
    match candidate_time.cmp(&current_time) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal => candidate_session_id.cmp(&current_session_id) == Ordering::Greater,
    }
}

/// Resolves the git directory for a project root. A `.git` directory is used
/// as-is; a `.git` file carries a `gitdir:` pointer (relative paths resolve
/// against the project root).
fn resolve_git_dir(project_dir: &Path) -> Option<std::path::PathBuf> {
    let dot_git = project_dir.join(".git");
    match std::fs::metadata(&dot_git) {
        Ok(metadata) if metadata.is_dir() => Some(dot_git),
        Ok(_) => {
            let content = std::fs::read_to_string(&dot_git).ok()?;
            let line = content.lines().next()?.trim();
            let path_text = line.strip_prefix("gitdir:")?.trim();
            if path_text.is_empty() {
                return None;
            }
            let git_dir = std::path::PathBuf::from(path_text);
            if git_dir.is_absolute() {
                Some(git_dir)
            } else {
                Some(project_dir.join(git_dir))
            }
        }
        Err(_) => None,
    }
}

/// Resolves the shared repository directory for linked worktrees. Git writes
/// a relative path such as `../..` to `<worktree-gitdir>/commondir`; remotes
/// remain in the shared `config`, while the worktree-specific `HEAD` stays in
/// `git_dir`. Ordinary repositories have no `commondir` and use `git_dir` for
/// both.
fn resolve_common_git_dir(git_dir: &Path) -> Option<std::path::PathBuf> {
    let content = match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(git_dir.to_path_buf());
        }
        Err(_) => return None,
    };
    let path_text = content.trim();
    if path_text.is_empty() {
        return None;
    }
    let common_dir = std::path::PathBuf::from(path_text);
    Some(if common_dir.is_absolute() {
        common_dir
    } else {
        git_dir.join(common_dir)
    })
}

/// Reads the branch from `HEAD`. A symbolic ref (`ref: ...`) becomes the
/// branch name with the `refs/heads/` prefix stripped; a detached HEAD is
/// shortened to its first seven characters (matching the C# behavior).
fn read_head(head_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(head_path).ok()?;
    let trimmed = content.trim();
    if let Some(rest) = trimmed.strip_prefix("ref:") {
        Some(rest.trim_start().replace("refs/heads/", ""))
    } else {
        let shortened: String = trimmed.chars().take(7).collect();
        Some(shortened)
    }
}

/// Reads the first `url` found under a `[remote ...]` section of a git config.
fn read_remote_url(config_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(config_path).ok()?;
    let mut in_remote = false;
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let section = &line[1..line.len() - 1];
            let keyword = section.split_whitespace().next().unwrap_or_default();
            in_remote = keyword.eq_ignore_ascii_case("remote");
            continue;
        }
        if !in_remote {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("url") {
            let url = unquote_git_value(value.trim());
            if !url.is_empty() {
                return Some(url);
            }
        }
    }
    None
}

/// Unescapes a quoted git config value. Unquoted values pass through as-is.
fn unquote_git_value(value: &str) -> String {
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return value.to_string();
    }
    let inner = &value[1..value.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('b') => out.push('\u{0008}'),
            Some('f') => out.push('\u{000C}'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}
