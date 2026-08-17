use crate::process::{ProcessOutput, ProcessRunner, ProcessSpec};
use serde::Serialize;
use sessionatlas_core::process::resolve_program;
use std::path::PathBuf;

const REMOTE_PREFIX: &str = "SESSIONATLAS_TUI:";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TuiDefinition {
    pub(crate) key: &'static str,
    pub(crate) name: &'static str,
    pub(crate) command: &'static str,
    pub(crate) manager: &'static str,
    pub(crate) package: &'static str,
}

pub(crate) const TUI_CATALOG: [TuiDefinition; 6] = [
    TuiDefinition {
        key: "claude",
        name: "Claude Code",
        command: "claude",
        manager: "npm",
        package: "@anthropic-ai/claude-code",
    },
    TuiDefinition {
        key: "codex",
        name: "Codex CLI",
        command: "codex",
        manager: "npm",
        package: "@openai/codex",
    },
    TuiDefinition {
        key: "kimi",
        name: "Kimi Code",
        command: "kimi",
        manager: "npm",
        package: "@moonshot-ai/kimi-code",
    },
    TuiDefinition {
        key: "opencode",
        name: "OpenCode",
        command: "opencode",
        manager: "npm",
        package: "opencode-ai@latest",
    },
    TuiDefinition {
        key: "aider",
        name: "Aider",
        command: "aider",
        manager: "uv",
        package: "aider-chat@latest",
    },
    TuiDefinition {
        key: "pi",
        name: "Pi Coding Agent",
        command: "pi",
        manager: "npm",
        package: "@earendil-works/pi-coding-agent",
    },
];

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiCapability {
    #[serde(rename = "toolKey")]
    pub(crate) tool_key: String,
    #[serde(rename = "toolName")]
    pub(crate) tool_name: String,
    pub(crate) installed: bool,
    pub(crate) version: Option<String>,
    pub(crate) enabled: bool,
    #[serde(rename = "installAvailable")]
    pub(crate) install_available: bool,
    #[serde(rename = "installManager")]
    pub(crate) install_manager: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiMachineCapabilities {
    pub(crate) source: String,
    #[serde(rename = "serverId")]
    pub(crate) server_id: Option<i64>,
    pub(crate) label: String,
    pub(crate) tools: Vec<TuiCapability>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DetectedTui {
    pub(crate) installed: bool,
    pub(crate) version: Option<String>,
}

pub(crate) fn definition(tool_key: &str) -> Result<&'static TuiDefinition, String> {
    TUI_CATALOG
        .iter()
        .find(|item| item.key.eq_ignore_ascii_case(tool_key.trim()))
        .ok_or_else(|| format!("unsupported TUI tool: {tool_key}"))
}

fn first_output_line(output: &ProcessOutput) -> Option<String> {
    let bytes = if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };
    let text = String::from_utf8_lossy(bytes);
    text.lines().find_map(|line| {
        let clean: String = line
            .chars()
            .filter(|character| !character.is_control())
            .take(160)
            .collect();
        let clean = clean.trim();
        (!clean.is_empty()).then(|| clean.to_string())
    })
}

pub(crate) fn detect_local<R: ProcessRunner>(
    definition: &TuiDefinition,
    runner: &R,
) -> DetectedTui {
    let Some(program) = resolve_program(definition.command) else {
        return DetectedTui {
            installed: false,
            version: None,
        };
    };
    let output = runner.output(&ProcessSpec::new(program).arg("--version"));
    DetectedTui {
        installed: true,
        version: output.ok().and_then(|value| first_output_line(&value)),
    }
}

pub(crate) fn local_manager_path(definition: &TuiDefinition) -> Option<PathBuf> {
    resolve_program(definition.manager)
}

pub(crate) fn local_install_spec(definition: &TuiDefinition) -> Result<ProcessSpec, String> {
    let manager = local_manager_path(definition).ok_or_else(|| {
        format!(
            "{} is required to install {}. Install {} first, then retry.",
            definition.manager, definition.name, definition.manager
        )
    })?;
    match definition.manager {
        "npm" => Ok(ProcessSpec::new(manager)
            .arg("install")
            .arg("-g")
            .arg(definition.package)),
        "uv" => Ok(ProcessSpec::new(manager)
            .arg("tool")
            .arg("install")
            .arg("--force")
            .arg("--python")
            .arg("python3.12")
            .arg(definition.package)),
        _ => Err("unsupported TUI installer".to_string()),
    }
}

pub(crate) fn run_local_install<R: ProcessRunner>(
    definition: &TuiDefinition,
    runner: &R,
) -> Result<(), String> {
    let spec = local_install_spec(definition)?;
    let output = runner.output(&spec)?;
    if output.success {
        return Ok(());
    }
    let detail =
        first_output_line(&output).unwrap_or_else(|| "installer exited with an error".to_string());
    Err(format!("could not install {}: {detail}", definition.name))
}

pub(crate) fn remote_probe_script() -> &'static str {
    "for sa_pair in 'claude:claude' 'codex:codex' 'kimi:kimi' 'opencode:opencode' 'aider:aider' 'pi:pi'; do \
     sa_key=${sa_pair%%:*}; sa_cmd=${sa_pair#*:}; \
     if command -v \"$sa_cmd\" >/dev/null 2>&1; then \
       sa_ver=$(\"$sa_cmd\" --version 2>&1 | head -n 1 | tr '\\r\\n' '  ' | cut -c 1-160); \
       printf 'SESSIONATLAS_TUI:%s:1:%s\\n' \"$sa_key\" \"$sa_ver\"; \
     else printf 'SESSIONATLAS_TUI:%s:0:\\n' \"$sa_key\"; fi; \
     done; \
     command -v npm >/dev/null 2>&1 && printf 'SESSIONATLAS_MANAGER:npm:1\\n' || printf 'SESSIONATLAS_MANAGER:npm:0\\n'; \
     command -v uv >/dev/null 2>&1 && printf 'SESSIONATLAS_MANAGER:uv:1\\n' || printf 'SESSIONATLAS_MANAGER:uv:0\\n'"
}

pub(crate) fn remote_install_script(definition: &TuiDefinition) -> Result<String, String> {
    match definition.manager {
        "npm" => Ok(format!(
            "if ! command -v npm >/dev/null 2>&1; then printf 'npm is required to install {}\\n' >&2; exit 127; fi; npm install -g {}",
            definition.name, definition.package
        )),
        "uv" => Ok(format!(
            "if ! command -v uv >/dev/null 2>&1; then printf 'uv is required to install {}\\n' >&2; exit 127; fi; uv tool install --force --python python3.12 {}",
            definition.name, definition.package
        )),
        _ => Err("unsupported TUI installer".to_string()),
    }
}

pub(crate) fn parse_remote_probe(
    stdout: &str,
) -> (
    std::collections::HashMap<String, DetectedTui>,
    std::collections::HashMap<String, bool>,
) {
    let mut tools = std::collections::HashMap::new();
    let mut managers = std::collections::HashMap::new();
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix(REMOTE_PREFIX) {
            let mut fields = rest.splitn(3, ':');
            let Some(key) = fields.next() else { continue };
            let Some(installed) = fields.next() else {
                continue;
            };
            let version = fields.next().unwrap_or("").trim();
            if definition(key).is_ok() {
                tools.insert(
                    key.to_string(),
                    DetectedTui {
                        installed: installed == "1",
                        version: (!version.is_empty()).then(|| version.to_string()),
                    },
                );
            }
        } else if let Some(rest) = line.strip_prefix("SESSIONATLAS_MANAGER:") {
            let mut fields = rest.splitn(2, ':');
            if let (Some(manager), Some(available)) = (fields.next(), fields.next()) {
                if matches!(manager, "npm" | "uv") {
                    managers.insert(manager.to_string(), available.trim() == "1");
                }
            }
        }
    }
    (tools, managers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_a_fixed_six_tool_allowlist() {
        assert_eq!(
            TUI_CATALOG.iter().map(|item| item.key).collect::<Vec<_>>(),
            vec!["claude", "codex", "kimi", "opencode", "aider", "pi"]
        );
        let pi = definition("pi").unwrap();
        assert_eq!(pi.command, "pi");
        assert_eq!(pi.package, "@earendil-works/pi-coding-agent");
        assert!(remote_probe_script().contains("'pi:pi'"));
        assert!(definition("shell").is_err());
        assert!(definition("npm install evil").is_err());
    }

    #[test]
    fn remote_probe_parser_is_bounded_to_known_tools_and_managers() {
        let (tools, managers) = parse_remote_probe(
            "SESSIONATLAS_TUI:codex:1:codex-cli 1.2.3\n\
             SESSIONATLAS_TUI:unknown:1:bad\n\
             SESSIONATLAS_MANAGER:npm:1\n\
             SESSIONATLAS_MANAGER:sh:1\n",
        );
        assert_eq!(tools["codex"].version.as_deref(), Some("codex-cli 1.2.3"));
        assert!(!tools.contains_key("unknown"));
        assert_eq!(managers.get("npm"), Some(&true));
        assert!(!managers.contains_key("sh"));
    }

    #[test]
    fn remote_install_commands_are_selected_only_from_the_catalog() {
        assert_eq!(
            remote_install_script(definition("codex").unwrap()).unwrap(),
            "if ! command -v npm >/dev/null 2>&1; then printf 'npm is required to install Codex CLI\\n' >&2; exit 127; fi; npm install -g @openai/codex"
        );
        assert_eq!(
            remote_install_script(definition("pi").unwrap()).unwrap(),
            "if ! command -v npm >/dev/null 2>&1; then printf 'npm is required to install Pi Coding Agent\\n' >&2; exit 127; fi; npm install -g @earendil-works/pi-coding-agent"
        );
        assert!(definition("codex; touch /tmp/x").is_err());
    }
}
