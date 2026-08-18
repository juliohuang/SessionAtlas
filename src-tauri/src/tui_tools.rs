use crate::process::{ProcessOutput, ProcessRunner, ProcessSpec};
use semver::Version;
use serde::Serialize;
#[cfg(test)]
use sessionatlas_core::adapter::AdapterRegistry;
use sessionatlas_core::adapter::{AdapterSource, RegisteredAdapter};
use sessionatlas_core::process::resolve_program;
use sessionatlas_core::security::{build_posix_command, parse_safe_command, quote_posix};
use std::collections::HashMap;
use std::path::PathBuf;

const REMOTE_PREFIX: &str = "SESSIONATLAS_TUI:";

// OpenSSH executes remote commands in a non-interactive shell, which commonly
// skips the profile snippets that add user-managed toolchains to PATH. Keep
// this bootstrap deterministic and side-effect free: it only adds existing,
// well-known install locations and never sources remote startup files.
const REMOTE_PATH_BOOTSTRAP: &str = concat!(
    "sa_path_add() { [ -d \"$1\" ] || return 0; ",
    "case \":$PATH:\" in *\":$1:\"*) ;; *) PATH=\"$1${PATH:+:$PATH}\" ;; esac; }; ",
    "for sa_dir in /opt/homebrew/bin /home/linuxbrew/.linuxbrew/bin /snap/bin; ",
    "do sa_path_add \"$sa_dir\"; done; ",
    "if [ -n \"${HOME:-}\" ]; then for sa_dir in ",
    "\"$HOME/.local/bin\" \"$HOME/bin\" \"$HOME/.npm-global/bin\" \"$HOME/.npm/bin\" ",
    "\"$HOME/.local/share/pnpm\" \"$HOME/.pnpm\" \"$HOME/.bun/bin\" ",
    "\"$HOME/.volta/bin\" \"$HOME/.cargo/bin\" \"$HOME/.asdf/shims\" ",
    "\"$HOME/.local/share/mise/shims\" \"$HOME\"/.nvm/versions/node/*/bin ",
    "\"$HOME\"/.local/share/fnm/node-versions/*/installation/bin ",
    "\"$HOME\"/.fnm/node-versions/*/installation/bin ",
    "\"$HOME\"/.asdf/installs/nodejs/*/bin ",
    "\"$HOME\"/.local/share/mise/installs/node/*/bin; ",
    "do sa_path_add \"$sa_dir\"; done; fi; export PATH; ",
);

pub(crate) type TuiDefinition = RegisteredAdapter;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiCapability {
    #[serde(rename = "toolKey")]
    pub(crate) tool_key: String,
    #[serde(rename = "toolName")]
    pub(crate) tool_name: String,
    pub(crate) installed: bool,
    pub(crate) version: Option<String>,
    pub(crate) supported: bool,
    #[serde(rename = "supportError")]
    pub(crate) support_error: Option<String>,
    pub(crate) enabled: bool,
    #[serde(rename = "adapterEnabled")]
    pub(crate) adapter_enabled: bool,
    #[serde(rename = "adapterVersion")]
    pub(crate) adapter_version: String,
    #[serde(rename = "adapterSource")]
    pub(crate) adapter_source: AdapterSource,
    #[serde(rename = "adapterNewestVersion")]
    pub(crate) adapter_newest_version: String,
    #[serde(rename = "adapterUpdateAvailable")]
    pub(crate) adapter_update_available: bool,
    #[serde(rename = "adapterRollbackVersion")]
    pub(crate) adapter_rollback_version: Option<String>,
    #[serde(rename = "installAvailable")]
    pub(crate) install_available: bool,
    #[serde(rename = "installManager")]
    pub(crate) install_manager: String,
    #[serde(rename = "installPackage")]
    pub(crate) install_package: String,
    #[serde(rename = "latestVersion")]
    pub(crate) latest_version: Option<String>,
    #[serde(rename = "updateChecked")]
    pub(crate) update_checked: bool,
    #[serde(rename = "updateAvailable")]
    pub(crate) update_available: bool,
    #[serde(rename = "updateCheckError")]
    pub(crate) update_check_error: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiMachineCapabilities {
    pub(crate) source: String,
    #[serde(rename = "serverId")]
    pub(crate) server_id: Option<i64>,
    pub(crate) label: String,
    pub(crate) tools: Vec<TuiCapability>,
    #[serde(rename = "adapterDiagnostics")]
    pub(crate) adapter_diagnostics: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DetectedTui {
    pub(crate) installed: bool,
    pub(crate) version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiUpdateCheck {
    pub(crate) latest_version: Option<String>,
    pub(crate) error: Option<String>,
}

#[cfg(test)]
pub(crate) fn bundled_catalog() -> Vec<TuiDefinition> {
    AdapterRegistry::bundled()
        .expect("compiled adapter manifests must be valid")
        .adapters()
        .to_vec()
}

pub(crate) fn definition<'a>(
    catalog: &'a [TuiDefinition],
    tool_key: &str,
) -> Result<&'a TuiDefinition, String> {
    catalog
        .iter()
        .find(|item| item.id.eq_ignore_ascii_case(tool_key.trim()))
        .ok_or_else(|| format!("unsupported TUI tool: {tool_key}"))
}

fn package_manager(definition: &TuiDefinition) -> Result<&str, String> {
    definition
        .manager
        .as_deref()
        .ok_or_else(|| format!("{} has no package manager", definition.name))
}

fn package_name(definition: &TuiDefinition) -> Result<&str, String> {
    definition
        .package
        .as_deref()
        .ok_or_else(|| format!("{} has no installable package", definition.name))
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

fn bounded_output_text(output: &ProcessOutput) -> String {
    let mut bytes = Vec::with_capacity(output.stdout.len() + output.stderr.len() + 1);
    bytes.extend_from_slice(&output.stdout);
    bytes.push(b'\n');
    bytes.extend_from_slice(&output.stderr);
    String::from_utf8_lossy(&bytes)
        .chars()
        .filter(|character| !character.is_control() || *character == '\n')
        .take(2_000)
        .collect()
}

fn extract_semver(text: &str) -> Option<Version> {
    let bytes = text.as_bytes();
    for start in 0..bytes.len() {
        if !bytes[start].is_ascii_digit() {
            continue;
        }
        if start > 0 && bytes[start - 1].is_ascii_digit() {
            continue;
        }
        let mut end = start;
        while end < bytes.len()
            && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'.' | b'-' | b'+'))
        {
            end += 1;
        }
        let candidate = text.get(start..end)?.trim_end_matches(['.', '-', '+']);
        if let Ok(version) = Version::parse(candidate) {
            return Some(version);
        }
    }
    None
}

fn latest_version_from_output(
    definition: &TuiDefinition,
    output: &ProcessOutput,
) -> Result<String, String> {
    let text = bounded_output_text(output);
    if !output.success {
        let detail = first_output_line(output)
            .unwrap_or_else(|| "package registry query exited with an error".to_string());
        return Err(detail);
    }
    let manager = package_manager(definition)?;
    let package = package_name(definition)?;
    let relevant = if manager == "uv" {
        text.lines()
            .find(|line| line.to_ascii_lowercase().contains(package))
            .unwrap_or(&text)
    } else {
        &text
    };
    extract_semver(relevant)
        .map(|version| version.to_string())
        .ok_or_else(|| format!("could not parse the latest {} version", definition.name))
}

fn installed_package_version_from_output(
    definition: &TuiDefinition,
    output: &ProcessOutput,
) -> Option<String> {
    let text = bounded_output_text(output);
    let lower = text.to_ascii_lowercase();
    let relevant = lower
        .find(package_name(definition).ok()?)
        .and_then(|index| text.get(index..))
        .unwrap_or(&text);
    extract_semver(relevant).map(|version| version.to_string())
}

pub(crate) fn update_is_available(installed: &str, latest: &str) -> Result<bool, String> {
    let installed = extract_semver(installed)
        .ok_or_else(|| "could not parse the installed version".to_string())?;
    let latest =
        extract_semver(latest).ok_or_else(|| "could not parse the latest version".to_string())?;
    Ok(latest > installed)
}

pub(crate) fn detect_local<R: ProcessRunner>(
    definition: &TuiDefinition,
    runner: &R,
) -> DetectedTui {
    let command = parse_safe_command(&definition.command).ok();
    let Some(program_name) = command.as_ref().and_then(|argv| argv.first()) else {
        return DetectedTui {
            installed: false,
            version: None,
        };
    };
    let Some(program) = resolve_program(program_name) else {
        return DetectedTui {
            installed: false,
            version: None,
        };
    };
    let output = runner.output(
        &ProcessSpec::new(program)
            .args(command.unwrap_or_default().into_iter().skip(1))
            .args(&definition.version_args),
    );
    let command_version = output.ok().and_then(|value| {
        value
            .success
            .then(|| first_output_line(&value))
            .flatten()
            .filter(|line| extract_semver(line).is_some())
    });
    let package_version = || {
        local_installed_package_spec(definition)
            .ok()
            .and_then(|spec| runner.output(&spec).ok())
            .and_then(|value| installed_package_version_from_output(definition, &value))
    };
    DetectedTui {
        installed: true,
        version: command_version.or_else(package_version),
    }
}

/// Fast launch-time availability check. Version probes belong to the settings
/// capability refresh; clicking a tool should not spawn a second short-lived
/// CLI process before its in-app PTY is ready.
pub(crate) fn local_command_available(definition: &TuiDefinition) -> bool {
    parse_safe_command(&definition.command)
        .ok()
        .and_then(|argv| argv.into_iter().next())
        .and_then(|program| resolve_program(&program))
        .is_some()
}

pub(crate) fn local_manager_path(definition: &TuiDefinition) -> Option<PathBuf> {
    definition.manager.as_deref().and_then(resolve_program)
}

fn local_installed_package_spec(definition: &TuiDefinition) -> Result<ProcessSpec, String> {
    let manager_name = package_manager(definition)?;
    let package = package_name(definition)?;
    let manager =
        local_manager_path(definition).ok_or_else(|| format!("{manager_name} is not installed"))?;
    match manager_name {
        "npm" => Ok(ProcessSpec::new(manager)
            .arg("list")
            .arg("--global")
            .arg(package)
            .arg("--depth=0")
            .arg("--json")
            .arg("--color=false")),
        "uv" => Ok(ProcessSpec::new(manager).arg("tool").arg("list")),
        _ => Err("unsupported TUI package manager".to_string()),
    }
}

pub(crate) fn local_install_spec(definition: &TuiDefinition) -> Result<ProcessSpec, String> {
    let manager_name = package_manager(definition)?;
    let package = package_name(definition)?;
    let manager = local_manager_path(definition).ok_or_else(|| {
        format!(
            "{} is required to install {}. Install {} first, then retry.",
            manager_name, definition.name, manager_name
        )
    })?;
    match manager_name {
        "npm" => Ok(ProcessSpec::new(manager)
            .arg("install")
            .arg("-g")
            .arg(format!("{package}@latest"))),
        "uv" => Ok(ProcessSpec::new(manager)
            .arg("tool")
            .arg("install")
            .arg("--force")
            .arg("--python")
            .arg("python3.12")
            .arg(package)),
        _ => Err("unsupported TUI installer".to_string()),
    }
}

pub(crate) fn local_update_check_spec(definition: &TuiDefinition) -> Result<ProcessSpec, String> {
    let manager_name = package_manager(definition)?;
    let package = package_name(definition)?;
    let manager = local_manager_path(definition).ok_or_else(|| {
        format!(
            "{} is required to check updates for {}",
            manager_name, definition.name
        )
    })?;
    match manager_name {
        "npm" => Ok(ProcessSpec::new(manager)
            .arg("view")
            .arg(package)
            .arg("version")
            .arg("--json")
            .arg("--color=false")
            .arg("--fetch-timeout=15000")
            .arg("--fetch-retries=1")),
        "uv" => Ok(ProcessSpec::new(manager)
            .arg("pip")
            .arg("install")
            .arg("--dry-run")
            .arg("--python")
            .arg("python3.12")
            .arg("--no-deps")
            .arg("--no-progress")
            .arg("--refresh-package")
            .arg(package)
            .arg("--reinstall-package")
            .arg(package)
            .arg(package)),
        _ => Err("unsupported TUI update checker".to_string()),
    }
}

pub(crate) fn run_local_update_check<R: ProcessRunner>(
    definition: &TuiDefinition,
    runner: &R,
) -> TuiUpdateCheck {
    let result = local_update_check_spec(definition)
        .and_then(|spec| runner.output(&spec))
        .and_then(|output| latest_version_from_output(definition, &output));
    match result {
        Ok(latest_version) => TuiUpdateCheck {
            latest_version: Some(latest_version),
            error: None,
        },
        Err(error) => TuiUpdateCheck {
            latest_version: None,
            error: Some(error),
        },
    }
}

pub(crate) fn local_upgrade_spec(definition: &TuiDefinition) -> Result<ProcessSpec, String> {
    let manager_name = package_manager(definition)?;
    let package = package_name(definition)?;
    let manager = local_manager_path(definition).ok_or_else(|| {
        format!(
            "{} is required to upgrade {}. Install {} first, then retry.",
            manager_name, definition.name, manager_name
        )
    })?;
    match manager_name {
        "npm" => Ok(ProcessSpec::new(manager)
            .arg("install")
            .arg("-g")
            .arg(format!("{package}@latest"))),
        "uv" => Ok(ProcessSpec::new(manager)
            .arg("tool")
            .arg("install")
            .arg("--force")
            .arg("--python")
            .arg("python3.12")
            .arg(package)),
        _ => Err("unsupported TUI upgrader".to_string()),
    }
}

pub(crate) fn run_local_upgrade<R: ProcessRunner>(
    definition: &TuiDefinition,
    runner: &R,
) -> Result<(), String> {
    let spec = local_upgrade_spec(definition)?;
    let output = runner.output(&spec)?;
    if output.success {
        return Ok(());
    }
    let detail =
        first_output_line(&output).unwrap_or_else(|| "upgrader exited with an error".to_string());
    Err(format!("could not upgrade {}: {detail}", definition.name))
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

fn remote_version_command(definition: &TuiDefinition) -> Result<(String, String), String> {
    let mut argv = parse_safe_command(&definition.command).map_err(|error| error.to_string())?;
    let executable = argv
        .first()
        .cloned()
        .ok_or_else(|| "adapter command is empty".to_string())?;
    argv.extend(definition.version_args.iter().cloned());
    let command = build_posix_command(&argv).map_err(|error| error.to_string())?;
    Ok((
        quote_posix(&executable).map_err(|error| error.to_string())?,
        command,
    ))
}

pub(crate) fn remote_probe_script(catalog: &[TuiDefinition]) -> Result<String, String> {
    let mut script = String::from(REMOTE_PATH_BOOTSTRAP);
    for definition in catalog
        .iter()
        .filter(|definition| definition.supports_remote)
    {
        let manager = definition.manager.as_deref();
        let package = definition.package.as_deref();
        let fallback = match (manager, package) {
            (Some("npm"), Some(package)) => format!(
                "npm list --global '{}' --depth=0 --json --color=false 2>/dev/null | sed -n 's/.*\"version\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p' | head -n 1",
                package
            ),
            (Some("uv"), Some(package)) => format!(
                "uv tool list 2>/dev/null | sed -n 's/^{} v\\([^ ]*\\).*/\\1/p' | head -n 1",
                package
            ),
            _ => String::new(),
        };
        let (executable, version_command) = remote_version_command(definition)?;
        script.push_str(&format!(
            "if command -v {} >/dev/null 2>&1; then sa_ver=$({} 2>&1); sa_code=$?; sa_ver=$(printf '%s' \"$sa_ver\" | head -n 1 | tr '\\r\\n' '  ' | cut -c 1-160); if [ \"$sa_code\" -ne 0 ] || [ -z \"$sa_ver\" ]; then sa_ver=$({}); fi; printf 'SESSIONATLAS_TUI:{}:1:%s\\n' \"$sa_ver\"; else printf 'SESSIONATLAS_TUI:{}:0:\\n'; fi; ",
            executable,
            version_command,
            fallback,
            definition.id,
            definition.id
        ));
    }
    script.push_str(
        "command -v npm >/dev/null 2>&1 && printf 'SESSIONATLAS_MANAGER:npm:1\\n' || printf 'SESSIONATLAS_MANAGER:npm:0\\n'; command -v uv >/dev/null 2>&1 && printf 'SESSIONATLAS_MANAGER:uv:1\\n' || printf 'SESSIONATLAS_MANAGER:uv:0\\n'",
    );
    Ok(script)
}

pub(crate) fn remote_install_script(definition: &TuiDefinition) -> Result<String, String> {
    let manager = package_manager(definition)?;
    let package = quote_posix(package_name(definition)?).map_err(|error| error.to_string())?;
    match manager {
        "npm" => Ok(format!(
            "{REMOTE_PATH_BOOTSTRAP}if ! command -v npm >/dev/null 2>&1; then printf 'npm is required\\n' >&2; exit 127; fi; npm install -g {}@latest",
            package
        )),
        "uv" => Ok(format!(
            "{REMOTE_PATH_BOOTSTRAP}if ! command -v uv >/dev/null 2>&1; then printf 'uv is required\\n' >&2; exit 127; fi; uv tool install --force --python python3.12 {}",
            package
        )),
        _ => Err("unsupported TUI installer".to_string()),
    }
}

pub(crate) fn remote_update_check_script(
    catalog: &[TuiDefinition],
    tool_keys: &[String],
) -> Result<String, String> {
    let mut script = String::from(REMOTE_PATH_BOOTSTRAP);
    for tool_key in tool_keys {
        let definition = definition(catalog, tool_key)?;
        let manager = package_manager(definition)?;
        let package = quote_posix(package_name(definition)?).map_err(|error| error.to_string())?;
        let command = match manager {
            "npm" => format!(
                "npm view {} version --json --color=false --fetch-timeout=15000 --fetch-retries=1",
                package
            ),
            "uv" => format!(
                "NO_COLOR=1 UV_NO_PROGRESS=1 uv pip install --dry-run --python python3.12 --no-deps --no-progress --refresh-package {} --reinstall-package {} {}",
                package, package, package
            ),
            _ => continue,
        };
        script.push_str(&format!(
            "if command -v '{}' >/dev/null 2>&1; then sa_out=$({} 2>&1); sa_code=$?; sa_out=$(printf '%s' \"$sa_out\" | tr '\\r\\n' '  ' | cut -c 1-400); printf 'SESSIONATLAS_UPDATE:{}:%s:%s\\n' \"$sa_code\" \"$sa_out\"; else printf 'SESSIONATLAS_UPDATE:{}:127:{} is not installed\\n'; fi; ",
            manager,
            command,
            definition.id,
            definition.id,
            manager
        ));
    }
    Ok(script)
}

pub(crate) fn remote_upgrade_script(definition: &TuiDefinition) -> Result<String, String> {
    let manager = package_manager(definition)?;
    let package = quote_posix(package_name(definition)?).map_err(|error| error.to_string())?;
    match manager {
        "npm" => Ok(format!(
            "{REMOTE_PATH_BOOTSTRAP}if ! command -v npm >/dev/null 2>&1; then printf 'npm is required\\n' >&2; exit 127; fi; npm install -g {}@latest",
            package
        )),
        "uv" => Ok(format!(
            "{REMOTE_PATH_BOOTSTRAP}if ! command -v uv >/dev/null 2>&1; then printf 'uv is required\\n' >&2; exit 127; fi; uv tool install --force --python python3.12 {}",
            package
        )),
        _ => Err("unsupported TUI upgrader".to_string()),
    }
}

pub(crate) fn parse_remote_update_checks(
    catalog: &[TuiDefinition],
    stdout: &str,
) -> HashMap<String, TuiUpdateCheck> {
    let mut checks = HashMap::new();
    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("SESSIONATLAS_UPDATE:") else {
            continue;
        };
        let mut fields = rest.splitn(3, ':');
        let Some(key) = fields.next() else { continue };
        let Some(status) = fields.next() else {
            continue;
        };
        let detail = fields.next().unwrap_or("").trim();
        let Ok(definition) = definition(catalog, key) else {
            continue;
        };
        let check = if status == "0" {
            let relevant = if definition.manager.as_deref() == Some("uv") {
                detail
                    .to_ascii_lowercase()
                    .find(definition.package.as_deref().unwrap_or_default())
                    .and_then(|index| detail.get(index..))
                    .unwrap_or(detail)
            } else {
                detail
            };
            match extract_semver(relevant) {
                Some(version) => TuiUpdateCheck {
                    latest_version: Some(version.to_string()),
                    error: None,
                },
                None => TuiUpdateCheck {
                    latest_version: None,
                    error: Some(format!(
                        "could not parse the latest {} version",
                        definition.name
                    )),
                },
            }
        } else {
            TuiUpdateCheck {
                latest_version: None,
                error: Some(if detail.is_empty() {
                    "package registry query exited with an error".to_string()
                } else {
                    detail.chars().take(300).collect()
                }),
            }
        };
        checks.insert(definition.id.clone(), check);
    }
    checks
}

pub(crate) fn parse_remote_probe(
    catalog: &[TuiDefinition],
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
            if definition(catalog, key).is_ok() {
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

pub(crate) fn validate_remote_probe(
    catalog: &[TuiDefinition],
    tools: &HashMap<String, DetectedTui>,
    managers: &HashMap<String, bool>,
) -> Result<(), String> {
    let missing_tools = catalog
        .iter()
        .filter(|definition| definition.supports_remote && !tools.contains_key(&definition.id))
        .map(|definition| definition.id.as_str())
        .collect::<Vec<_>>();
    let missing_managers = ["npm", "uv"]
        .into_iter()
        .filter(|manager| !managers.contains_key(*manager))
        .collect::<Vec<_>>();
    if missing_tools.is_empty() && missing_managers.is_empty() {
        return Ok(());
    }

    let mut missing = Vec::new();
    if !missing_tools.is_empty() {
        missing.push(format!("tools: {}", missing_tools.join(", ")));
    }
    if !missing_managers.is_empty() {
        missing.push(format!("package managers: {}", missing_managers.join(", ")));
    }
    Err(format!(
        "remote TUI probe returned incomplete results ({}); verify that the account's default SSH shell is POSIX-compatible",
        missing.join("; ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_catalog() -> Vec<TuiDefinition> {
        bundled_catalog()
    }

    #[test]
    fn catalog_is_loaded_from_the_six_official_adapters() {
        let catalog = test_catalog();
        assert_eq!(
            catalog
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["claude", "kimi", "codex", "opencode", "aider", "pi"]
        );
        let pi = definition(&catalog, "pi").unwrap();
        assert_eq!(pi.command, "pi");
        assert_eq!(
            pi.package.as_deref(),
            Some("@earendil-works/pi-coding-agent")
        );
        let probe = remote_probe_script(&catalog).unwrap();
        assert!(probe.contains("'pi' '--version'"));
        assert!(probe.contains("npm list --global '@earendil-works/pi-coding-agent'"));
        assert!(definition(&catalog, "shell").is_err());
        assert!(definition(&catalog, "npm install evil").is_err());
    }

    #[test]
    fn remote_probe_parser_is_bounded_to_known_tools_and_managers() {
        let catalog = test_catalog();
        let (tools, managers) = parse_remote_probe(
            &catalog,
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
    fn remote_probe_validation_rejects_silent_or_partial_shell_output() {
        let catalog = test_catalog();
        let (tools, managers) = parse_remote_probe(
            &catalog,
            "SESSIONATLAS_TUI:codex:1:codex-cli 1.2.3\n\
             SESSIONATLAS_MANAGER:npm:1\n",
        );
        let error = validate_remote_probe(&catalog, &tools, &managers).unwrap_err();
        assert!(error.contains("tools: claude"));
        assert!(error.contains("package managers: uv"));
        assert!(error.contains("POSIX-compatible"));

        let full_probe = remote_probe_script(&[]).unwrap();
        assert!(full_probe.contains("SESSIONATLAS_MANAGER:npm"));
        assert!(full_probe.contains("SESSIONATLAS_MANAGER:uv"));
    }

    #[test]
    fn remote_install_commands_are_selected_only_from_the_catalog() {
        let catalog = test_catalog();
        let codex = remote_install_script(definition(&catalog, "codex").unwrap()).unwrap();
        assert!(codex.starts_with(REMOTE_PATH_BOOTSTRAP));
        assert!(codex.ends_with(
            "if ! command -v npm >/dev/null 2>&1; then printf 'npm is required\\n' >&2; exit 127; fi; npm install -g '@openai/codex'@latest"
        ));
        let pi = remote_install_script(definition(&catalog, "pi").unwrap()).unwrap();
        assert!(pi.starts_with(REMOTE_PATH_BOOTSTRAP));
        assert!(pi.ends_with(
            "if ! command -v npm >/dev/null 2>&1; then printf 'npm is required\\n' >&2; exit 127; fi; npm install -g '@earendil-works/pi-coding-agent'@latest"
        ));
        assert!(definition(&catalog, "codex; touch /tmp/x").is_err());
    }

    #[test]
    fn remote_scripts_bootstrap_non_interactive_user_paths_without_profiles() {
        let catalog = test_catalog();
        let probe = remote_probe_script(&catalog).unwrap();
        let install = remote_install_script(definition(&catalog, "codex").unwrap()).unwrap();
        let checks = remote_update_check_script(&catalog, &["codex".to_string()]).unwrap();
        let upgrade = remote_upgrade_script(definition(&catalog, "aider").unwrap()).unwrap();

        for script in [&probe, &install, &checks, &upgrade] {
            assert!(script.starts_with(REMOTE_PATH_BOOTSTRAP));
            assert!(script.contains("$HOME/.local/bin"));
            assert!(script.contains("\"$HOME\"/.nvm/versions/node/*/bin"));
            assert!(script.contains("$HOME/.volta/bin"));
            assert!(script.contains("/opt/homebrew/bin"));
            assert!(!script.contains("source "));
            assert!(!script.contains("eval "));
            assert!(!script.contains(".profile"));
            assert!(!script.contains(".bashrc"));
        }
    }

    #[test]
    fn version_comparison_extracts_tool_output_and_orders_semver() {
        assert_eq!(
            extract_semver("codex-cli 1.2.3 (build 9)").unwrap(),
            Version::parse("1.2.3").unwrap()
        );
        assert!(update_is_available("aider 0.84.2", "0.85.0").unwrap());
        assert!(!update_is_available("opencode 1.18.18", "1.18.18").unwrap());
        assert!(!update_is_available("pi 2.0.0", "1.99.0").unwrap());
        assert!(update_is_available("unknown", "1.0.0").is_err());
    }

    #[test]
    fn latest_output_prefers_package_versions_over_runtime_and_manager_notices() {
        let catalog = test_catalog();
        let npm = ProcessOutput {
            success: true,
            status_code: Some(0),
            stdout: b"\"0.147.0\"\n".to_vec(),
            stderr: b"npm notice new npm 12.0.2\n".to_vec(),
        };
        assert_eq!(
            latest_version_from_output(definition(&catalog, "codex").unwrap(), &npm).unwrap(),
            "0.147.0"
        );

        let uv = ProcessOutput {
            success: true,
            status_code: Some(0),
            stdout: Vec::new(),
            stderr: b"Using CPython 3.12.9\nWould install aider-chat-0.85.1\n".to_vec(),
        };
        assert_eq!(
            latest_version_from_output(definition(&catalog, "aider").unwrap(), &uv).unwrap(),
            "0.85.1"
        );

        let installed = ProcessOutput {
            success: true,
            status_code: Some(0),
            stdout: b"{\"dependencies\":{\"opencode-ai\":{\"version\":\"1.18.18\"}}}".to_vec(),
            stderr: Vec::new(),
        };
        assert_eq!(
            installed_package_version_from_output(
                definition(&catalog, "opencode").unwrap(),
                &installed,
            )
            .as_deref(),
            Some("1.18.18")
        );
    }

    #[test]
    fn update_specs_and_upgrade_scripts_stay_on_the_validated_adapter_catalog() {
        let catalog = test_catalog();
        let codex = definition(&catalog, "codex").unwrap();
        let spec = local_update_check_spec(codex).unwrap();
        assert!(spec.args.contains(&"@openai/codex".into()));
        assert!(spec.args.contains(&"--fetch-timeout=15000".into()));
        let upgrade = remote_upgrade_script(codex).unwrap();
        assert!(upgrade.ends_with(
            "if ! command -v npm >/dev/null 2>&1; then printf 'npm is required\\n' >&2; exit 127; fi; npm install -g '@openai/codex'@latest"
        ));
        let aider = remote_upgrade_script(definition(&catalog, "aider").unwrap()).unwrap();
        assert!(aider.ends_with("uv tool install --force --python python3.12 'aider-chat'"));
        let checks =
            remote_update_check_script(&catalog, &["codex".to_string(), "aider".to_string()])
                .unwrap();
        assert!(checks.contains("npm view '@openai/codex' version"));
        assert!(checks.contains("uv pip install --dry-run"));
        assert!(checks.contains("--reinstall-package 'aider-chat'"));
        assert!(!checks.contains("@moonshot-ai/kimi-code"));
        assert!(!checks.contains("touch "));
        assert!(
            remote_update_check_script(&catalog, &["codex; touch /tmp/x".to_string()]).is_err()
        );
    }

    #[test]
    fn remote_update_parser_ignores_unknown_tools_and_captures_errors() {
        let catalog = test_catalog();
        let checks = parse_remote_update_checks(
            &catalog,
            "SESSIONATLAS_UPDATE:codex:0:\"1.4.0\"\n\
             SESSIONATLAS_UPDATE:aider:0:Using CPython 3.12.9 Would install aider-chat-0.85.1\n\
             SESSIONATLAS_UPDATE:pi:1:registry unavailable: timed out\n\
             SESSIONATLAS_UPDATE:unknown:0:9.9.9\n",
        );
        assert_eq!(checks["codex"].latest_version.as_deref(), Some("1.4.0"));
        assert_eq!(checks["aider"].latest_version.as_deref(), Some("0.85.1"));
        assert!(checks["pi"]
            .error
            .as_deref()
            .unwrap()
            .contains("registry unavailable"));
        assert!(!checks.contains_key("unknown"));
    }
}
