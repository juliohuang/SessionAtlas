//! Data model types: `Project`, `ToolUsage`, `Session`, `ToolSource`.
//!
//! Types mirror `SessionAtlas.Models`; defaults match the C# initializers.
//! `Project`/`ToolUsage`/`Session` serialize timestamps as RFC 3339 strings;
//! `ToolSource` serializes with PascalCase keys to match `config.json`.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::path;

/// Default open-command template, matching `ToolSource.OpenCommandTemplate`.
pub const DEFAULT_OPEN_COMMAND_TEMPLATE: &str = "cd \"{projectPath}\" && {cliCommand}";

/// Unique identifier matching C# `Guid.NewGuid().ToString("N")`.
fn generate_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Serde helpers that render `DateTime<Utc>` as RFC 3339 strings without
/// requiring chrono's `serde` feature.
mod rfc3339 {
    use chrono::{DateTime, SecondsFormat, Utc};
    use serde::{de, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_rfc3339_opts(SecondsFormat::AutoSi, true))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        DateTime::parse_from_rfc3339(&text)
            .map(|value| value.with_timezone(&Utc))
            .map_err(de::Error::custom)
    }
}

/// Serde helpers for optional RFC 3339 timestamps.
mod opt_rfc3339 {
    use chrono::{DateTime, SecondsFormat, Utc};
    use serde::{de, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(value) => {
                serializer.serialize_some(&value.to_rfc3339_opts(SecondsFormat::AutoSi, true))
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Option::<String>::deserialize(deserializer)? {
            Some(text) => DateTime::parse_from_rfc3339(&text)
                .map(|value| Some(value.with_timezone(&Utc)))
                .map_err(de::Error::custom),
            None => Ok(None),
        }
    }
}

/// Project entity — aggregates work records across multiple AI CLI tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    /// Absolute project path (unique identity).
    pub path: String,
    /// The tool still references this project, but its local directory is gone.
    /// Recomputed from the filesystem whenever a snapshot is built or read.
    #[serde(default)]
    pub path_missing: bool,
    #[serde(with = "rfc3339")]
    pub last_accessed_at: DateTime<Utc>,
    #[serde(with = "rfc3339")]
    pub first_seen_at: DateTime<Utc>,
    #[serde(default)]
    pub git_branch: Option<String>,
    #[serde(default)]
    pub git_remote_url: Option<String>,
    pub tool_usages: Vec<ToolUsage>,
}

impl Project {
    /// Directory-name display label for the project path (native flavor).
    pub fn display_name(&self) -> Option<String> {
        path::display_name_native(&self.path)
    }

    /// Deduplicated, sorted display tags: `"a, b"` over distinct tool names.
    pub fn tool_tags(&self) -> String {
        let mut names: Vec<&str> = self
            .tool_usages
            .iter()
            .map(|usage| usage.tool_name.as_str())
            .collect();
        names.sort_unstable();
        names.dedup();
        names.join(", ")
    }
}

impl Default for Project {
    fn default() -> Self {
        Self {
            id: generate_id(),
            path: String::new(),
            path_missing: false,
            last_accessed_at: DateTime::<Utc>::MIN_UTC,
            first_seen_at: Utc::now(),
            git_branch: None,
            git_remote_url: None,
            tool_usages: Vec::new(),
        }
    }
}

/// Returns `true` only when a project path is conclusively absent or is no
/// longer a directory. Permission and transient I/O failures are not labelled
/// as missing because the directory may still exist but be temporarily
/// inaccessible.
pub fn project_path_missing(path: &str) -> bool {
    match std::fs::metadata(path) {
        Ok(metadata) => !metadata.is_dir(),
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

impl fmt::Display for Project {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} [{}] @ {}",
            self.display_name().unwrap_or_default(),
            self.tool_tags(),
            self.path
        )
    }
}

/// Usage record of one AI CLI tool on a specific project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolUsage {
    pub tool_name: String,
    pub tool_key: String,
    #[serde(with = "rfc3339")]
    pub last_used_at: DateTime<Utc>,
    pub session_count: i32,
    #[serde(default)]
    pub last_session_id: Option<String>,
}

impl Default for ToolUsage {
    fn default() -> Self {
        Self {
            tool_name: String::new(),
            tool_key: String::new(),
            last_used_at: DateTime::<Utc>::MIN_UTC,
            session_count: 0,
            last_session_id: None,
        }
    }
}

/// Session record — used for recent lists and resume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub project_path: String,
    pub tool_key: String,
    pub tool_name: String,
    #[serde(with = "rfc3339")]
    pub started_at: DateTime<Utc>,
    #[serde(with = "opt_rfc3339", default)]
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub session_id_from_tool: Option<String>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            id: generate_id(),
            project_path: String::new(),
            tool_key: String::new(),
            tool_name: String::new(),
            started_at: Utc::now(),
            ended_at: None,
            session_id_from_tool: None,
        }
    }
}

/// AI CLI tool source definition; serialized PascalCase to match `config.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ToolSource {
    /// Unique key, e.g. `claude`, `codex`, `kimi`.
    pub key: String,
    /// Display name.
    pub name: String,
    /// CLI command, e.g. `claude`, `codex`, `kimi`.
    pub cli_command: String,
    /// Data directory (absolute path after platform resolution).
    pub data_directory: String,
    /// Scanner type.
    pub scanner_type: String,
    /// Whether the command is on PATH.
    pub is_installed: bool,
    /// Whether scanning is enabled.
    pub is_enabled: bool,
    /// Open-command template with `{projectPath}` and `{sessionId}` variables.
    pub open_command_template: String,
}

impl Default for ToolSource {
    fn default() -> Self {
        Self {
            key: String::new(),
            name: String::new(),
            cli_command: String::new(),
            data_directory: String::new(),
            scanner_type: String::new(),
            is_installed: false,
            is_enabled: true,
            open_command_template: DEFAULT_OPEN_COMMAND_TEMPLATE.to_string(),
        }
    }
}
