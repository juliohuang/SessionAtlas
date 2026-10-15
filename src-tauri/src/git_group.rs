//! Read repository remotes for grouping without status scans or Git fetches.
use std::{path::Path, time::Duration};

use crate::process::{git_read_spec, ProcessRunner, TimeoutProcessRunner};

fn repository_key(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.chars().any(char::is_control) {
        return None;
    }
    let (host, path, port) = if raw.contains("://") {
        let url = url::Url::parse(raw).ok()?;
        if !matches!(url.scheme(), "https" | "http" | "ssh" | "git") {
            return None;
        }
        let port = match (url.scheme(), url.port()) {
            ("ssh", Some(22)) | ("git", Some(9418)) => None,
            (_, port) => port,
        };
        (url.host_str()?.to_lowercase(), url.path().to_string(), port)
    } else {
        let (authority, path) = raw.split_once(':')?;
        let host = authority.rsplit('@').next()?;
        if host.len() < 2 || host.contains(['/', '\\', ' ']) || path.starts_with('\\') {
            return None;
        }
        (host.to_lowercase(), path.to_string(), None)
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty() || path.starts_with(':') || path.contains(['?', '#', ' ']) {
        return None;
    }
    Some(match port {
        Some(port) => format!("{host}:{port}/{path}"),
        None => format!("{host}/{path}"),
    })
}

fn group_from_config(output: &str) -> Option<String> {
    let remotes: Vec<_> = output
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once(char::is_whitespace)?;
            Some((name, value.trim()))
        })
        .collect();
    let selected = remotes
        .iter()
        .find(|(name, _)| *name == "remote.origin.url")
        .or_else(|| remotes.first())?;
    repository_key(selected.1)
}

#[tauri::command]
pub(crate) async fn get_git_repository_group(
    path: String,
    server_id: Option<i64>,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let spec = if let Some(id) = server_id {
            let server = crate::load_remote_server(id)?;
            let port = u16::try_from(server.port).map_err(|_| "Invalid SSH port")?;
            let path = crate::quote_remote_path(&path)?;
            crate::build_ssh_command(&server.user, &server.host, port, server.identity_file.as_deref(),
                &format!("cd -- {path} 2>/dev/null || exit 20; git config --local --get-regexp '^remote\\..*\\.url$'"))?
        } else {
            if !Path::new(&path).is_dir() { return Err("Project directory is unavailable".into()); }
            git_read_spec(Path::new(&path), &["config", "--local", "--get-regexp", "^remote\\..*\\.url$"])
        };
        let output = TimeoutProcessRunner::new(Duration::from_secs(8)).output(&spec)
            .map_err(|_| "Could not read repository remotes".to_string())?;
        if output.success {
            Ok(group_from_config(&String::from_utf8_lossy(&output.stdout)))
        } else if output.status_code == Some(1) {
            Ok(None)
        } else {
            Err("Repository remotes are unavailable".into())
        }
    }).await.map_err(|_| "Repository grouping worker failed".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_actual_repository_config_from_a_subdirectory() {
        let root = tempfile::tempdir().unwrap();
        let git = root.path().join(".git");
        std::fs::create_dir_all(git.join("objects")).unwrap();
        std::fs::create_dir_all(git.join("refs")).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join("config"), "[core]\nrepositoryformatversion = 0\n[remote \"origin\"]\nurl = https://user:secret@example.test/team/repo.git\n").unwrap();
        let child = root.path().join("nested project");
        std::fs::create_dir(&child).unwrap();
        let result = tauri::async_runtime::block_on(get_git_repository_group(
            child.to_string_lossy().into_owned(),
            None,
        ))
        .unwrap();
        assert_eq!(result.as_deref(), Some("example.test/team/repo"));
        std::fs::write(git.join("config"), "[core]\nrepositoryformatversion = 0\n").unwrap();
        assert_eq!(
            tauri::async_runtime::block_on(get_git_repository_group(
                child.to_string_lossy().into_owned(),
                None,
            ))
            .unwrap(),
            None
        );
    }

    #[test]
    fn transports_share_identity_without_credentials() {
        for remote in [
            "git@GitHub.com:org/repo.git",
            "https://user:secret@github.com/org/repo.git?token=secret",
            "ssh://git@github.com:22/org/repo/",
        ] {
            assert_eq!(
                repository_key(remote).as_deref(),
                Some("github.com/org/repo")
            );
        }
        assert_eq!(
            repository_key("ssh://git@example.com:2222/Org/repo.git").as_deref(),
            Some("example.com:2222/Org/repo")
        );
        for remote in [
            "C:\\repo",
            "../repo",
            "file:///tmp/repo",
            "ext::command",
            "https://example.com/",
        ] {
            assert!(repository_key(remote).is_none());
        }
    }

    #[test]
    fn origin_wins_and_fallback_is_deterministic() {
        assert_eq!(group_from_config("remote.upstream.url https://github.com/up/repo.git\nremote.origin.url git@github.com:me/repo.git").as_deref(), Some("github.com/me/repo"));
        assert_eq!(
            group_from_config("remote.upstream.url https://github.com/up/repo.git").as_deref(),
            Some("github.com/up/repo")
        );
        assert_eq!(group_from_config(""), None);
    }
}
