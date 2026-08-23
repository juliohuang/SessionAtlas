//! Contract tests for `sessionatlas_core::model`.

use chrono::{DateTime, Utc};
use sessionatlas_core::model::{project_path_missing, Project, Session, ToolSource, ToolUsage};

#[test]
fn model_contract_tool_source_defaults_match_csharp() {
    let source = ToolSource::default();
    assert!(source.is_enabled);
    assert!(!source.is_installed);
    assert_eq!(
        source.open_command_template,
        "cd \"{projectPath}\" && {cliCommand}"
    );
    assert!(source.key.is_empty());
    assert!(source.data_directory.is_empty());
}

#[test]
fn model_contract_tool_source_round_trips_pascal_case_json() {
    let source = ToolSource {
        key: "codex".to_string(),
        name: "Codex".to_string(),
        cli_command: "codex".to_string(),
        data_directory: r"C:\Users\me\.codex".to_string(),
        scanner_type: "codex".to_string(),
        is_installed: true,
        is_enabled: true,
        open_command_template: "cd \"{projectPath}\" && {cliCommand}".to_string(),
    };
    let json = serde_json::to_string(&source).unwrap();
    assert!(json.contains("\"Key\":\"codex\""));
    assert!(json.contains("\"OpenCommandTemplate\":\"cd \\\"{projectPath}\\\" && {cliCommand}\""));
    assert!(json.contains("\"IsEnabled\":true"));

    let decoded: ToolSource = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, source);
}

#[test]
fn model_contract_tool_source_missing_fields_use_defaults() {
    let decoded: ToolSource =
        serde_json::from_str(r#"{"Key":"kimi","CliCommand":"kimi"}"#).unwrap();
    assert_eq!(decoded.key, "kimi");
    assert_eq!(decoded.cli_command, "kimi");
    assert!(decoded.is_enabled);
    assert!(!decoded.is_installed);
    assert_eq!(
        decoded.open_command_template,
        "cd \"{projectPath}\" && {cliCommand}"
    );
}

#[test]
fn model_contract_project_derived_members_match_csharp() {
    let project = Project {
        path: if cfg!(windows) {
            r"C:\repo".to_string()
        } else {
            "/repo".to_string()
        },
        tool_usages: vec![
            ToolUsage {
                tool_name: "codex".to_string(),
                ..ToolUsage::default()
            },
            ToolUsage {
                tool_name: "kimi".to_string(),
                ..ToolUsage::default()
            },
            ToolUsage {
                tool_name: "codex".to_string(),
                ..ToolUsage::default()
            },
        ],
        ..Project::default()
    };

    assert_eq!(project.tool_tags(), "codex, kimi");
    assert_eq!(project.display_name().as_deref(), Some("repo"));
    assert!(project.to_string().contains("[codex, kimi] @"));
}

#[test]
fn model_contract_project_round_trips_iso8601_timestamps() {
    let project = Project {
        id: "fixed-project-id".to_string(),
        path: "/repo".to_string(),
        path_missing: true,
        last_accessed_at: DateTime::parse_from_rfc3339("2026-08-15T10:20:30Z")
            .unwrap()
            .into(),
        first_seen_at: DateTime::parse_from_rfc3339("2026-08-15T09:00:00Z")
            .unwrap()
            .into(),
        git_branch: Some("main".to_string()),
        git_remote_url: None,
        tool_usages: vec![ToolUsage {
            tool_name: "claude".to_string(),
            tool_key: "claude".to_string(),
            last_used_at: DateTime::parse_from_rfc3339("2026-08-15T10:00:00Z")
                .unwrap()
                .into(),
            session_count: 3,
            last_session_id: Some("sess-1".to_string()),
        }],
    };

    let json = serde_json::to_string(&project).unwrap();
    assert!(json.contains("\"last_accessed_at\":\"2026-08-15T10:20:30Z\""));
    assert!(json.contains("\"first_seen_at\":\"2026-08-15T09:00:00Z\""));
    assert!(json.contains("\"path_missing\":true"));

    let decoded: Project = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, project);
    assert_eq!(decoded.tool_usages[0].session_count, 3);
    assert_eq!(
        decoded.tool_usages[0].last_session_id.as_deref(),
        Some("sess-1")
    );
}

#[test]
fn model_contract_missing_path_is_live_and_legacy_json_defaults_to_present() {
    let root = tempfile::tempdir().unwrap();
    let present = root.path().join("present");
    let missing = root.path().join("missing");
    let file = root.path().join("file-project");
    std::fs::create_dir(&present).unwrap();
    std::fs::write(&file, b"not a directory").unwrap();

    assert!(!project_path_missing(&present.to_string_lossy()));
    assert!(project_path_missing(&missing.to_string_lossy()));
    assert!(project_path_missing(&file.to_string_lossy()));

    let legacy = r#"{
        "id":"legacy",
        "path":"/legacy",
        "last_accessed_at":"2026-08-15T10:20:30Z",
        "first_seen_at":"2026-08-15T09:00:00Z",
        "tool_usages":[]
    }"#;
    let decoded: Project = serde_json::from_str(legacy).unwrap();
    assert!(!decoded.path_missing);
}

#[test]
fn model_contract_session_round_trips_and_defaults() {
    let session = Session {
        id: "session-id".to_string(),
        project_path: "/repo".to_string(),
        tool_key: "codex".to_string(),
        tool_name: "Codex".to_string(),
        started_at: DateTime::parse_from_rfc3339("2026-08-15T10:00:00Z")
            .unwrap()
            .into(),
        ended_at: None,
        session_id_from_tool: None,
    };
    let json = serde_json::to_string(&session).unwrap();
    assert!(json.contains("\"started_at\":\"2026-08-15T10:00:00Z\""));
    let decoded: Session = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, session);

    let fresh = Session::default();
    assert!(fresh.started_at <= Utc::now());
    assert!(fresh.ended_at.is_none());
    assert!(fresh.session_id_from_tool.is_none());
}

#[test]
fn model_contract_defaults_generate_unique_ids_and_utc_timestamps() {
    let first = Project::default();
    let second = Project::default();
    assert!(!first.id.is_empty());
    assert_eq!(first.id.len(), 32);
    assert_ne!(first.id, second.id);
    let parsed = uuid::Uuid::parse_str(&first.id).expect("project ID must be a UUID");
    assert_eq!(parsed.get_version_num(), 4);
    assert_eq!(first.first_seen_at.timezone(), Utc);

    assert_eq!(ToolUsage::default().session_count, 0);
    assert!(ToolUsage::default().last_session_id.is_none());
}
