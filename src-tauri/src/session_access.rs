//! Read-only history source resolution and recovery checks.
use std::path::Path;

use serde::Serialize;
use sessionatlas_core::adapter::AdapterRegistry;
use sessionatlas_core::scanner::{kimi, opencode, pi};
use sessionatlas_core::session_history::{
    read_project_history, HistoryIssue, HistorySource, SessionHistory,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResumeStatus {
    pub(crate) ready: bool,
    pub(crate) reason: String,
    pub(crate) detail: String,
    pub(crate) verified: bool,
}

impl ResumeStatus {
    pub(crate) fn blocked(reason: &str, detail: impl Into<String>) -> Self {
        Self {
            ready: false,
            reason: reason.into(),
            detail: detail.into(),
            verified: false,
        }
    }

    pub(crate) fn ready(verified: bool) -> Self {
        Self {
            ready: true,
            reason: if verified {
                "ready"
            } else {
                "remote_session_unverified"
            }
            .into(),
            detail: String::new(),
            verified,
        }
    }
}

pub(crate) fn project_history(path: &str, registry: &AdapterRegistry) -> SessionHistory {
    project_history_for(path, registry, None)
}

pub(crate) fn project_history_for(
    path: &str,
    registry: &AdapterRegistry,
    tool: Option<&str>,
) -> SessionHistory {
    let home = crate::app_home_directory();
    let mut sources = Vec::new();
    let mut issues = Vec::new();
    for adapter in registry.adapters() {
        if tool.is_some_and(|tool| tool != adapter.id) {
            continue;
        }
        let root = match adapter.scanner.handler.as_str() {
            "builtin.claude" => home.join(".claude/projects"),
            "builtin.codex" => home.join(".codex/sessions"),
            "builtin.kimi" => kimi::resolve_kimi_home()
                .unwrap_or_else(|| home.join(".kimi-code"))
                .join("sessions"),
            "builtin.pi" => match pi::history_source_directory() {
                Ok(root) => root,
                Err(_) => {
                    issues.push(HistoryIssue {
                        tool_key: adapter.id.clone(),
                        code: "unreadable_source".into(),
                    });
                    continue;
                }
            },
            "builtin.opencode" => {
                // Use every existing official store, never create a database.
                for root in opencode::candidate_database_paths()
                    .into_iter()
                    .filter(|root| root.exists())
                {
                    sources.push(HistorySource {
                        tool_key: adapter.id.clone(),
                        root,
                    });
                }
                if !sources.iter().any(|source| source.tool_key == adapter.id) {
                    issues.push(HistoryIssue {
                        tool_key: adapter.id.clone(),
                        code: "source_unavailable".into(),
                    });
                }
                continue;
            }
            "builtin.aider" => Path::new(path).join(".aider.chat.history"),
            _ => {
                issues.push(HistoryIssue {
                    tool_key: adapter.id.clone(),
                    code: "unsupported_format".into(),
                });
                continue;
            }
        };
        sources.push(HistorySource {
            tool_key: adapter.id.clone(),
            root,
        });
    }
    let mut history = read_project_history(path, &sources);
    for session in &mut history.sessions {
        if !Path::new(path).is_dir() {
            session.can_resume = false;
            session.reason = Some("path_missing".into());
        } else if session.can_resume
            && registry
                .find(&session.tool_key)
                .is_none_or(|adapter| adapter.launch_argv(session.session_id.as_deref()).is_err())
        {
            session.can_resume = false;
            session.reason = Some("unsupported_resume".into());
        }
    }
    history.issues.extend(issues);
    history
        .issues
        .sort_by(|a, b| (&a.tool_key, &a.code).cmp(&(&b.tool_key, &b.code)));
    history
        .issues
        .dedup_by(|a, b| a.tool_key == b.tool_key && a.code == b.code);
    history
}

pub(crate) fn history_resume_status(
    history: &SessionHistory,
    tool_key: &str,
    session_id: &str,
) -> ResumeStatus {
    if let Some(session) = history.sessions.iter().find(|session| {
        session.tool_key == tool_key && session.session_id.as_deref() == Some(session_id)
    }) {
        return if session.can_resume {
            ResumeStatus::ready(true)
        } else {
            ResumeStatus::blocked(
                session.reason.as_deref().unwrap_or("unsupported_resume"),
                "",
            )
        };
    }
    let incomplete = history.issues.iter().any(|issue| {
        (issue.tool_key.is_empty() || issue.tool_key == tool_key)
            && issue.code != "auxiliary_filtered"
    });
    ResumeStatus::blocked(
        if incomplete {
            "history_incomplete"
        } else {
            "session_missing"
        },
        "",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sessionatlas_core::session_history::HistorySession;

    #[test]
    fn recovery_requires_the_requested_tool_and_session() {
        let history = SessionHistory {
            sessions: vec![HistorySession {
                tool_key: "codex".into(),
                session_id: Some("primary".into()),
                title: "Task".into(),
                preview: String::new(),
                last_used_at: String::new(),
                can_resume: true,
                reason: None,
            }],
            issues: vec![],
        };
        assert!(history_resume_status(&history, "codex", "primary").ready);
        assert_eq!(
            history_resume_status(&history, "claude", "primary").reason,
            "session_missing"
        );
        assert_eq!(
            history_resume_status(&history, "codex", "missing").reason,
            "session_missing"
        );
    }

    #[test]
    fn incomplete_history_cannot_claim_a_session_was_deleted() {
        let history = SessionHistory {
            sessions: vec![],
            issues: vec![HistoryIssue {
                tool_key: "codex".into(),
                code: "time_limit".into(),
            }],
        };
        assert_eq!(
            history_resume_status(&history, "codex", "missing").reason,
            "history_incomplete"
        );
    }
}
