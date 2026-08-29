//! Data-driven AI TUI adapter manifests and the local adapter registry.
//!
//! Adapters are deliberately declarative. They may select a known package
//! manager, provide validated argv templates, and select a bounded scanner
//! handler. They never carry shell text or native code. Six official manifests
//! are compiled in as an offline fallback; a user may explicitly install a
//! newer manifest under `~/.sessionatlas/adapters/<id>/<version>/adapter.json`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::ops::Deref;
use std::path::{Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::private_fs;
use crate::process::is_bare_program_name;
use crate::security;

pub const ADAPTER_API_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const SESSION_ID_PLACEHOLDER: &str = "{sessionId}";
const BUILTIN_MANIFESTS: [&str; 6] = [
    include_str!("../../../adapters/official/claude.json"),
    include_str!("../../../adapters/official/kimi.json"),
    include_str!("../../../adapters/official/codex.json"),
    include_str!("../../../adapters/official/opencode.json"),
    include_str!("../../../adapters/official/aider.json"),
    include_str!("../../../adapters/official/pi.json"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterError(String);

impl AdapterError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for AdapterError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdapterLaunch {
    #[serde(default)]
    pub new_args: Vec<String>,
    #[serde(default)]
    pub resume_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdapterScanner {
    pub handler: String,
    #[serde(default)]
    pub data_directory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdapterManifest {
    pub api_version: u32,
    pub id: String,
    pub name: String,
    pub adapter_version: String,
    pub command: String,
    #[serde(default = "default_version_args")]
    pub version_args: Vec<String>,
    pub launch: AdapterLaunch,
    #[serde(default)]
    pub manager: Option<String>,
    #[serde(default)]
    pub package: Option<String>,
    pub scanner: AdapterScanner,
    #[serde(default = "default_platforms")]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub supports_remote: bool,
}

fn default_version_args() -> Vec<String> {
    vec!["--version".to_string()]
}

fn default_platforms() -> Vec<String> {
    vec![
        "windows".to_string(),
        "macos".to_string(),
        "linux".to_string(),
    ]
}

impl AdapterManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, AdapterError> {
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(AdapterError::new("adapter manifest exceeds 256 KiB"));
        }
        let manifest: Self = serde_json::from_slice(bytes).map_err(|error| {
            AdapterError::new(format!("invalid adapter manifest JSON: {error}"))
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), AdapterError> {
        if self.api_version != ADAPTER_API_VERSION {
            return Err(AdapterError::new(format!(
                "unsupported adapter API version {}; expected {}",
                self.api_version, ADAPTER_API_VERSION
            )));
        }
        let id = security::validate_tool_key(&self.id)
            .map_err(|_| AdapterError::new("adapter id is invalid"))?;
        if id != self.id {
            return Err(AdapterError::new(
                "adapter id must not contain surrounding whitespace",
            ));
        }
        if id != id.to_ascii_lowercase() {
            return Err(AdapterError::new("adapter id must be lowercase"));
        }
        let name = security::validate_display_label(&self.name)
            .map_err(|_| AdapterError::new("adapter display name is invalid"))?;
        if name != self.name {
            return Err(AdapterError::new(
                "adapter display name must not contain surrounding whitespace",
            ));
        }
        Version::parse(&self.adapter_version)
            .map_err(|_| AdapterError::new("adapterVersion must be valid semantic versioning"))?;
        let command = security::parse_safe_command(&self.command)
            .map_err(|error| AdapterError::new(format!("adapter command is unsafe: {error}")))?;
        if command.len() != 1 {
            return Err(AdapterError::new(
                "adapter command must name exactly one executable; put launch arguments in launch.newArgs or launch.resumeArgs",
            ));
        }
        if !is_bare_program_name(&command[0]) {
            return Err(AdapterError::new(
                "adapter command must be a bare executable name resolved through PATH",
            ));
        }
        validate_version_probe(&self.version_args)?;
        validate_tokens("launch.newArgs", &self.launch.new_args, false)?;
        validate_tokens("launch.resumeArgs", &self.launch.resume_args, true)?;
        if !self.launch.resume_args.is_empty()
            && self
                .launch
                .resume_args
                .iter()
                .filter(|token| token.as_str() == SESSION_ID_PLACEHOLDER)
                .count()
                != 1
        {
            return Err(AdapterError::new(
                "launch.resumeArgs must contain exactly one {sessionId} token",
            ));
        }
        match (self.manager.as_deref(), self.package.as_deref()) {
            (None, None) => {}
            (Some(manager @ ("npm" | "uv")), Some(package)) => {
                validate_package_name(manager, package)?;
            }
            (Some(_), Some(_)) => {
                return Err(AdapterError::new("adapter manager must be one of: npm, uv"));
            }
            _ => {
                return Err(AdapterError::new(
                    "adapter manager and package must be supplied together",
                ));
            }
        }
        validate_scanner(self)?;
        if self.platforms.is_empty() {
            return Err(AdapterError::new("adapter platforms cannot be empty"));
        }
        let mut seen = BTreeSet::new();
        for platform in &self.platforms {
            if !matches!(platform.as_str(), "windows" | "macos" | "linux") {
                return Err(AdapterError::new(format!(
                    "unsupported adapter platform: {platform}"
                )));
            }
            if !seen.insert(platform) {
                return Err(AdapterError::new("adapter platforms contain duplicates"));
            }
        }
        Ok(())
    }

    pub fn launch_argv(&self, session_id: Option<&str>) -> Result<Vec<String>, AdapterError> {
        let mut argv = security::parse_safe_command(&self.command)
            .map_err(|error| AdapterError::new(format!("adapter command is unsafe: {error}")))?;
        if let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) {
            let session_id = security::validate_session_id(session_id)
                .map_err(|_| AdapterError::new("session id is invalid"))?;
            if self.launch.resume_args.is_empty() {
                return Err(AdapterError::new(format!(
                    "{} does not declare session resume support",
                    self.name
                )));
            }
            argv.extend(self.launch.resume_args.iter().map(|token| {
                if token == SESSION_ID_PLACEHOLDER {
                    session_id.clone()
                } else {
                    token.clone()
                }
            }));
        } else {
            argv.extend(self.launch.new_args.iter().cloned());
        }
        security::build_posix_command(&argv)
            .map_err(|error| AdapterError::new(format!("adapter argv is unsafe: {error}")))?;
        Ok(argv)
    }

    pub fn adapter_semver(&self) -> Version {
        Version::parse(&self.adapter_version)
            .expect("validated adapter manifests always contain semantic versions")
    }

    pub fn supports_platform(&self, platform: &str) -> bool {
        self.platforms.iter().any(|candidate| candidate == platform)
    }
}

fn validate_tokens(
    label: &str,
    tokens: &[String],
    allow_session_id: bool,
) -> Result<(), AdapterError> {
    let mut substituted = Vec::with_capacity(tokens.len());
    for token in tokens {
        if token == SESSION_ID_PLACEHOLDER {
            if !allow_session_id {
                return Err(AdapterError::new(format!(
                    "{label} cannot contain {{sessionId}}"
                )));
            }
            substituted.push("session-123".to_string());
        } else {
            if token.contains('{') || token.contains('}') {
                return Err(AdapterError::new(format!(
                    "{label} contains an unsupported placeholder"
                )));
            }
            substituted.push(token.clone());
        }
    }
    if !substituted.is_empty() {
        security::build_posix_command(&substituted)
            .map_err(|error| AdapterError::new(format!("{label} is unsafe: {error}")))?;
    }
    Ok(())
}

fn validate_version_probe(tokens: &[String]) -> Result<(), AdapterError> {
    validate_tokens("versionArgs", tokens, false)?;
    let supported = matches!(
        tokens,
        [argument]
            if matches!(argument.as_str(), "--version" | "-V" | "-v" | "version")
    );
    if !supported {
        return Err(AdapterError::new(
            "versionArgs must contain exactly one safe version argument: --version, -V, -v, or version",
        ));
    }
    Ok(())
}

fn validate_package_name(manager: &str, package: &str) -> Result<(), AdapterError> {
    if package.is_empty()
        || package.len() > 200
        || package.starts_with('-')
        || package.chars().any(|character| {
            !character.is_ascii_alphanumeric()
                && !matches!(character, '@' | '/' | '.' | '_' | '+' | '-')
        })
    {
        return Err(AdapterError::new(format!("invalid {manager} package name")));
    }
    let valid_component = |component: &str| {
        !component.is_empty()
            && !component.starts_with('-')
            && component.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '+' | '-')
            })
    };
    match manager {
        "npm" if package.starts_with('@') => {
            let Some((scope, name)) = package[1..].split_once('/') else {
                return Err(AdapterError::new("invalid scoped npm package name"));
            };
            if name.contains('/') || !valid_component(scope) || !valid_component(name) {
                return Err(AdapterError::new("invalid scoped npm package name"));
            }
        }
        "npm"
            if package
                .chars()
                .any(|character| matches!(character, '@' | '/'))
                || !valid_component(package) =>
        {
            return Err(AdapterError::new("invalid npm package name"));
        }
        "uv" if package
            .chars()
            .any(|character| matches!(character, '@' | '/'))
            || !valid_component(package) =>
        {
            return Err(AdapterError::new("invalid uv package name"));
        }
        _ => {}
    }
    Ok(())
}

fn validate_scanner(manifest: &AdapterManifest) -> Result<(), AdapterError> {
    let handler = manifest.scanner.handler.as_str();
    if let Some(id) = handler.strip_prefix("builtin.") {
        if id != manifest.id
            || !matches!(
                id,
                "claude" | "codex" | "kimi" | "opencode" | "aider" | "pi"
            )
        {
            return Err(AdapterError::new(
                "builtin scanner handler must match an official adapter id",
            ));
        }
        if manifest.scanner.data_directory.is_some() {
            return Err(AdapterError::new(
                "builtin scanners cannot override their data directory",
            ));
        }
        return Ok(());
    }
    if handler != "metadata-v1" {
        return Err(AdapterError::new(format!(
            "unsupported scanner handler: {handler}"
        )));
    }
    let directory = manifest
        .scanner
        .data_directory
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AdapterError::new("metadata-v1 requires scanner.dataDirectory"))?;
    if directory.len() > 1024 || directory.chars().any(char::is_control) {
        return Err(AdapterError::new("scanner data directory is invalid"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AdapterSource {
    Bundled,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredAdapter {
    pub manifest: AdapterManifest,
    pub source: AdapterSource,
    pub available_versions: Vec<String>,
    pub rollback_version: Option<String>,
    pub newest_version: String,
}

impl Deref for RegisteredAdapter {
    type Target = AdapterManifest;

    fn deref(&self) -> &Self::Target {
        &self.manifest
    }
}

impl RegisteredAdapter {
    pub fn update_available(&self) -> bool {
        Version::parse(&self.newest_version)
            .is_ok_and(|newest| newest > self.manifest.adapter_semver())
    }
}

#[derive(Debug, Clone)]
struct Candidate {
    manifest: AdapterManifest,
    source: AdapterSource,
}

#[derive(Debug, Clone, Default)]
pub struct AdapterRegistry {
    adapters: Vec<RegisteredAdapter>,
    diagnostics: Vec<String>,
}

impl AdapterRegistry {
    pub fn bundled() -> Result<Self, AdapterError> {
        Self::from_candidates(bundled_candidates()?, &AppConfig::default(), Vec::new())
    }

    pub fn load(root: &Path, config: &AppConfig) -> Result<Self, AdapterError> {
        let mut candidates = bundled_candidates()?;
        let mut diagnostics = Vec::new();
        load_local_candidates(root, &mut candidates, &mut diagnostics);
        Self::from_candidates(candidates, config, diagnostics)
    }

    fn from_candidates(
        candidates: Vec<Candidate>,
        config: &AppConfig,
        mut diagnostics: Vec<String>,
    ) -> Result<Self, AdapterError> {
        let mut grouped: BTreeMap<String, Vec<Candidate>> = BTreeMap::new();
        for candidate in candidates {
            grouped
                .entry(candidate.manifest.id.clone())
                .or_default()
                .push(candidate);
        }
        let mut adapters = Vec::new();
        for (id, mut versions) in grouped {
            versions.sort_by(|left, right| {
                right
                    .manifest
                    .adapter_semver()
                    .cmp(&left.manifest.adapter_semver())
                    // A manually placed local file must not replace a bundled
                    // official contract without increasing adapterVersion.
                    .then_with(|| source_rank(left.source).cmp(&source_rank(right.source)))
            });
            versions.dedup_by(|left, right| {
                left.manifest.adapter_version == right.manifest.adapter_version
            });
            let requested = config.active_adapter_versions.get(&id);
            let requested_index = requested.and_then(|version| {
                versions
                    .iter()
                    .position(|candidate| &candidate.manifest.adapter_version == version)
            });
            if let Some(version) = requested.filter(|_| requested_index.is_none()) {
                diagnostics.push(format!(
                    "configured adapter version is not installed: {id}/{version}"
                ));
            }
            let active_index = requested_index
                .or_else(|| {
                    versions
                        .iter()
                        .position(|candidate| candidate.source == AdapterSource::Bundled)
                })
                .unwrap_or(0);
            let active = versions[active_index].clone();
            let active_version = active.manifest.adapter_semver();
            let mut available_versions = versions
                .iter()
                .map(|candidate| candidate.manifest.adapter_version.clone())
                .collect::<Vec<_>>();
            available_versions.sort_by(|left, right| {
                Version::parse(right)
                    .expect("candidate versions were validated")
                    .cmp(&Version::parse(left).expect("candidate versions were validated"))
            });
            let rollback_version = versions
                .iter()
                .filter(|candidate| candidate.manifest.adapter_semver() < active_version)
                .max_by_key(|candidate| candidate.manifest.adapter_semver())
                .map(|candidate| candidate.manifest.adapter_version.clone());
            let newest_version = available_versions
                .first()
                .cloned()
                .expect("every registry group has at least one candidate");
            adapters.push(RegisteredAdapter {
                manifest: active.manifest,
                source: active.source,
                available_versions,
                rollback_version,
                newest_version,
            });
        }
        adapters.sort_by(|left, right| adapter_order(&left.id).cmp(&adapter_order(&right.id)));
        for id in config.active_adapter_versions.keys() {
            if !adapters
                .iter()
                .any(|adapter| adapter.id.eq_ignore_ascii_case(id))
            {
                diagnostics.push(format!("configured active adapter is not installed: {id}"));
            }
        }
        if let Some(enabled) = &config.enabled_adapters {
            for id in enabled {
                if !adapters
                    .iter()
                    .any(|adapter| adapter.id.eq_ignore_ascii_case(id))
                {
                    diagnostics.push(format!("enabled adapter is not installed: {id}"));
                }
            }
        }
        Ok(Self {
            adapters,
            diagnostics,
        })
    }

    pub fn adapters(&self) -> &[RegisteredAdapter] {
        &self.adapters
    }

    pub fn find(&self, id: &str) -> Option<&RegisteredAdapter> {
        self.adapters
            .iter()
            .find(|adapter| adapter.id.eq_ignore_ascii_case(id.trim()))
    }

    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    pub fn enabled<'a>(
        &'a self,
        config: &'a AppConfig,
    ) -> impl Iterator<Item = &'a RegisteredAdapter> {
        self.adapters.iter().filter(move |adapter| {
            config.enabled_adapters.as_ref().map_or(
                adapter.source == AdapterSource::Bundled,
                |enabled| {
                    enabled
                        .iter()
                        .any(|id| id.eq_ignore_ascii_case(&adapter.id))
                },
            )
        })
    }
}

fn source_rank(source: AdapterSource) -> u8 {
    match source {
        AdapterSource::Bundled => 0,
        AdapterSource::Local => 1,
    }
}

fn adapter_order(id: &str) -> (u8, &str) {
    let order = match id {
        "claude" => 0,
        "kimi" => 1,
        "codex" => 2,
        "opencode" => 3,
        "aider" => 4,
        "pi" => 5,
        _ => 100,
    };
    (order, id)
}

fn bundled_candidates() -> Result<Vec<Candidate>, AdapterError> {
    BUILTIN_MANIFESTS
        .iter()
        .map(|json| {
            AdapterManifest::parse(json.as_bytes()).map(|manifest| Candidate {
                manifest,
                source: AdapterSource::Bundled,
            })
        })
        .collect()
}

fn load_local_candidates(
    root: &Path,
    candidates: &mut Vec<Candidate>,
    diagnostics: &mut Vec<String>,
) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            diagnostics.push(format!("could not read adapter directory: {error}"));
            return;
        }
    };
    for id_entry in entries.flatten() {
        let Ok(file_type) = id_entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let id = id_entry.file_name().to_string_lossy().into_owned();
        if security::validate_tool_key(&id).is_err() {
            diagnostics.push(format!("ignored adapter directory with invalid id: {id}"));
            continue;
        }
        let Ok(version_entries) = fs::read_dir(id_entry.path()) else {
            diagnostics.push(format!(
                "could not read installed adapter versions for {id}"
            ));
            continue;
        };
        for version_entry in version_entries.flatten() {
            let Ok(file_type) = version_entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let version = version_entry.file_name().to_string_lossy().into_owned();
            if Version::parse(&version).is_err() {
                diagnostics.push(format!(
                    "ignored invalid adapter version directory: {id}/{version}"
                ));
                continue;
            }
            let manifest_path = version_entry.path().join("adapter.json");
            let bytes = match read_bounded_manifest_file(&manifest_path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    diagnostics.push(format!("could not read adapter {id}/{version}: {error}"));
                    continue;
                }
            };
            match AdapterManifest::parse(&bytes) {
                Ok(manifest) if manifest.id == id && manifest.adapter_version == version => {
                    candidates.push(Candidate {
                        manifest,
                        source: AdapterSource::Local,
                    });
                }
                Ok(_) => diagnostics.push(format!(
                    "adapter identity does not match its directory: {id}/{version}"
                )),
                Err(error) => diagnostics.push(format!("ignored adapter {id}/{version}: {error}")),
            }
        }
    }
}

pub fn adapter_root_for_home(home: impl AsRef<Path>) -> PathBuf {
    home.as_ref().join(".sessionatlas").join("adapters")
}

pub fn adapter_root_for_config(config_path: impl AsRef<Path>) -> PathBuf {
    config_path
        .as_ref()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("adapters")
}

pub fn install_manifest_file(source: &Path, root: &Path) -> Result<AdapterManifest, AdapterError> {
    let (manifest, bytes) = read_manifest_document(source)?;
    persist_manifest_document(manifest, bytes, root)
}

/// Installs a manifest only when it is newer than the supplied active version.
/// The version check and persisted bytes come from the same bounded read, so a
/// source-file race cannot validate one manifest and install another.
pub fn install_manifest_upgrade_file(
    source: &Path,
    root: &Path,
    active_versions: &BTreeMap<String, String>,
) -> Result<AdapterManifest, AdapterError> {
    let (manifest, bytes) = read_manifest_document(source)?;
    if let Some(active_version) = active_versions.get(&manifest.id) {
        let active = Version::parse(active_version)
            .map_err(|_| AdapterError::new("active adapter version is invalid"))?;
        if manifest.adapter_semver() <= active {
            return Err(AdapterError::new(format!(
                "adapter {} must be newer than active version {}; use rollback for older versions",
                manifest.id, active_version
            )));
        }
    }
    persist_manifest_document(manifest, bytes, root)
}

fn read_manifest_document(source: &Path) -> Result<(AdapterManifest, Vec<u8>), AdapterError> {
    let bytes = read_bounded_manifest_file(source)?;
    let manifest = AdapterManifest::parse(&bytes)?;
    Ok((manifest, bytes))
}

fn read_bounded_manifest_file(path: &Path) -> Result<Vec<u8>, AdapterError> {
    let path_metadata = fs::symlink_metadata(path).map_err(|error| {
        AdapterError::new(format!("could not inspect adapter manifest: {error}"))
    })?;
    if !path_metadata.file_type().is_file()
        || path_metadata.file_type().is_symlink()
        || path_metadata.len() > MAX_MANIFEST_BYTES
    {
        return Err(AdapterError::new(
            "adapter manifest must be a regular file no larger than 256 KiB",
        ));
    }

    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|error| AdapterError::new(format!("could not read adapter manifest: {error}")))?;
    let opened_metadata = file.metadata().map_err(|error| {
        AdapterError::new(format!(
            "could not inspect opened adapter manifest: {error}"
        ))
    })?;
    if !opened_metadata.is_file() || opened_metadata.len() > MAX_MANIFEST_BYTES {
        return Err(AdapterError::new(
            "adapter manifest must be a regular file no larger than 256 KiB",
        ));
    }

    let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
    file.take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| AdapterError::new(format!("could not read adapter manifest: {error}")))?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(AdapterError::new("adapter manifest exceeds 256 KiB"));
    }
    Ok(bytes)
}

fn persist_manifest_document(
    manifest: AdapterManifest,
    bytes: Vec<u8>,
    root: &Path,
) -> Result<AdapterManifest, AdapterError> {
    if let Some(data_root) = root
        .parent()
        .filter(|path| path.file_name().is_some_and(|name| name == ".sessionatlas"))
    {
        private_fs::ensure_private_directory(data_root).map_err(|error| {
            AdapterError::new(format!("could not prepare private data root: {error}"))
        })?;
    }
    private_fs::ensure_private_directory(root)
        .map_err(|error| AdapterError::new(format!("could not create adapter root: {error}")))?;
    verify_adapter_directory(root)?;
    let id_directory = root.join(&manifest.id);
    create_adapter_directory(&id_directory)?;
    let target_directory = id_directory.join(&manifest.adapter_version);
    create_adapter_directory(&target_directory)?;
    let target = target_directory.join("adapter.json");
    match fs::symlink_metadata(&target) {
        Ok(metadata)
            if metadata.file_type().is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() <= MAX_MANIFEST_BYTES =>
        {
            let current = fs::read(&target).map_err(|error| {
                AdapterError::new(format!("could not read existing adapter manifest: {error}"))
            })?;
            if current == bytes {
                return Ok(manifest);
            }
            return Err(AdapterError::new(
                "this adapter version is already installed with different contents",
            ));
        }
        Ok(_) => {
            return Err(AdapterError::new(
                "existing adapter manifest target is not a safe regular file",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AdapterError::new(format!(
                "could not inspect existing adapter manifest: {error}"
            )));
        }
    }
    let temporary = target_directory.join(format!("adapter.json.tmp.{}", Uuid::new_v4()));
    let write_result = (|| -> Result<(), AdapterError> {
        let mut file = private_fs::open_private_create_new(&temporary).map_err(|error| {
            AdapterError::new(format!("could not create adapter temp file: {error}"))
        })?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| {
                AdapterError::new(format!("could not persist adapter manifest: {error}"))
            })?;
        fs::rename(&temporary, &target).map_err(|error| {
            AdapterError::new(format!("could not activate adapter manifest: {error}"))
        })?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result?;
    Ok(manifest)
}

fn create_adapter_directory(path: &Path) -> Result<(), AdapterError> {
    private_fs::ensure_private_directory(path).map_err(|error| {
        AdapterError::new(format!("could not create adapter directory: {error}"))
    })?;
    verify_adapter_directory(path)
}

fn verify_adapter_directory(path: &Path) -> Result<(), AdapterError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        AdapterError::new(format!("could not inspect adapter directory: {error}"))
    })?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(AdapterError::new(
            "adapter registry directories must not be files or symbolic links",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_adapters_define_the_six_launch_contracts() {
        let registry = AdapterRegistry::bundled().unwrap();
        assert_eq!(
            registry
                .adapters()
                .iter()
                .map(|adapter| adapter.id.as_str())
                .collect::<Vec<_>>(),
            vec!["claude", "kimi", "codex", "opencode", "aider", "pi"]
        );
        assert_eq!(
            registry
                .find("codex")
                .unwrap()
                .launch_argv(Some("session-123"))
                .unwrap(),
            vec!["codex", "resume", "session-123"]
        );
        assert_eq!(
            registry
                .find("pi")
                .unwrap()
                .launch_argv(Some("session-123"))
                .unwrap(),
            vec!["pi", "--session", "session-123"]
        );
        let opencode = registry.find("opencode").unwrap();
        assert_eq!(opencode.adapter_version, "1.0.1");
        assert_eq!(
            opencode.launch_argv(Some("session-123")).unwrap(),
            vec!["opencode", "--session", "session-123"]
        );
    }

    #[test]
    fn manifest_rejects_shells_unknown_handlers_and_injected_packages() {
        let base = BUILTIN_MANIFESTS[2];
        for (from, to) in [
            ("\"command\": \"codex\"", "\"command\": \"cmd.exe /C calc\""),
            ("\"builtin.codex\"", "\"native.dll\""),
            ("\"@openai/codex\"", "\"@openai/codex;calc\""),
            ("\"@openai/codex\"", "\"@openai/codex@beta\""),
            ("\"@openai/codex\"", "\"codex@beta\""),
            ("\"@openai/codex\"", "\"scope/codex\""),
        ] {
            let changed = base.replace(from, to);
            assert!(AdapterManifest::parse(changed.as_bytes()).is_err());
        }
        let invalid_uv = BUILTIN_MANIFESTS[4].replace("\"aider-chat\"", "\"aider@beta\"");
        assert!(AdapterManifest::parse(invalid_uv.as_bytes()).is_err());
    }

    #[test]
    fn manifest_restricts_automatic_version_probes_to_safe_arguments() {
        let base = BUILTIN_MANIFESTS[2];
        let unsafe_probes = [
            base.replace("\"command\": \"codex\"", "\"command\": \"codex --quiet\""),
            base.replace(
                "\"versionArgs\": [\"--version\"]",
                "\"versionArgs\": [\"-c\", \"print('executed')\"]",
            ),
            base.replace("\"versionArgs\": [\"--version\"]", "\"versionArgs\": []"),
            base.replace(
                "\"versionArgs\": [\"--version\"]",
                "\"versionArgs\": [\"--help\"]",
            ),
        ];
        for manifest in unsafe_probes {
            assert!(AdapterManifest::parse(manifest.as_bytes()).is_err());
        }

        for safe_argument in ["--version", "-V", "-v", "version"] {
            let changed = base.replace(
                "\"versionArgs\": [\"--version\"]",
                &format!("\"versionArgs\": [\"{safe_argument}\"]"),
            );
            assert!(AdapterManifest::parse(changed.as_bytes()).is_ok());
        }
    }

    #[test]
    fn manifest_commands_reject_absolute_relative_and_multi_component_paths() {
        let base = BUILTIN_MANIFESTS[2];
        for command in [
            r#"C:\tools\codex.exe"#,
            r#"C:/tools/codex.exe"#,
            r#"\\server\share\codex.exe"#,
            "/opt/tools/codex",
            "./tools/codex",
            "../tools/codex",
            "tools/codex",
            r#"tools\codex"#,
            "C:codex",
        ] {
            let changed = base.replace(
                "\"command\": \"codex\"",
                &format!("\"command\": \"{command}\""),
            );
            assert!(
                AdapterManifest::parse(changed.as_bytes()).is_err(),
                "path command accepted: {command}"
            );
        }

        let quoted_path = base.replace(
            "\"command\": \"codex\"",
            "\"command\": \"\\\"C:/tools/codex.exe\\\"\"",
        );
        assert!(AdapterManifest::parse(quoted_path.as_bytes()).is_err());
        assert!(AdapterManifest::parse(base.as_bytes()).is_ok());
    }

    #[test]
    fn manifest_file_reads_are_bounded_before_json_parsing() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("oversized.json");
        fs::write(&source, vec![b' '; MAX_MANIFEST_BYTES as usize + 1]).unwrap();
        assert!(install_manifest_file(&source, &temporary.path().join("adapters")).is_err());
    }

    #[test]
    fn local_manifest_install_is_immutable_and_registry_supports_rollback() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("adapters");
        let source = temporary.path().join("codex-1.1.0.json");
        let upgraded = BUILTIN_MANIFESTS[2].replace(
            "\"adapterVersion\": \"1.0.0\"",
            "\"adapterVersion\": \"1.1.0\"",
        );
        fs::write(&source, &upgraded).unwrap();
        let manifest = install_manifest_file(&source, &root).unwrap();
        assert_eq!(manifest.adapter_version, "1.1.0");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let version = root.join("codex").join("1.1.0");
            assert_eq!(
                fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&version).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(version.join("adapter.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

        let mut config = AppConfig::default();
        config
            .active_adapter_versions
            .insert("codex".to_string(), "1.1.0".to_string());
        let registry = AdapterRegistry::load(&root, &config).unwrap();
        let codex = registry.find("codex").unwrap();
        assert_eq!(codex.adapter_version, "1.1.0");
        assert_eq!(codex.source, AdapterSource::Local);
        assert_eq!(codex.rollback_version.as_deref(), Some("1.0.0"));

        let older_source = temporary.path().join("codex-1.0.0.json");
        fs::write(&older_source, BUILTIN_MANIFESTS[2]).unwrap();
        let active_versions = BTreeMap::from([("codex".to_string(), "1.1.0".to_string())]);
        assert!(install_manifest_upgrade_file(&older_source, &root, &active_versions).is_err());
        assert!(!root.join("codex").join("1.0.0").exists());

        fs::write(&source, upgraded.replace("Codex CLI", "Changed")).unwrap();
        assert!(install_manifest_file(&source, &root).is_err());
    }

    #[test]
    fn local_same_version_cannot_shadow_a_bundled_adapter() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("adapters");
        let source = temporary.path().join("codex-1.0.0.json");
        let changed = BUILTIN_MANIFESTS[2].replace("Codex CLI", "Shadowed Codex");
        fs::write(&source, changed).unwrap();
        install_manifest_file(&source, &root).unwrap();

        let registry = AdapterRegistry::load(&root, &AppConfig::default()).unwrap();
        let codex = registry.find("codex").unwrap();
        assert_eq!(codex.source, AdapterSource::Bundled);
        assert_eq!(codex.name, "Codex CLI");
    }

    #[test]
    fn user_adapters_are_disabled_by_default_until_selected() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("adapters");
        let source = temporary.path().join("demo.json");
        let manifest = r#"{
          "apiVersion": 1,
          "id": "demo",
          "name": "Demo",
          "adapterVersion": "1.0.0",
          "command": "demo",
          "launch": { "newArgs": [], "resumeArgs": [] },
          "scanner": { "handler": "metadata-v1", "dataDirectory": "~/.demo/sessions" }
        }"#;
        fs::write(&source, manifest).unwrap();
        install_manifest_file(&source, &root).unwrap();
        let config = AppConfig::default();
        let registry = AdapterRegistry::load(&root, &config).unwrap();
        assert!(registry.find("demo").is_some());
        assert!(!registry
            .enabled(&config)
            .any(|adapter| adapter.id == "demo"));
    }

    #[test]
    fn missing_configured_versions_fall_back_with_diagnostics() {
        let mut config = AppConfig::default();
        config
            .active_adapter_versions
            .insert("codex".to_string(), "9.9.9".to_string());
        config.enabled_adapters = Some(vec!["codex".to_string(), "ghost".to_string()]);
        let registry =
            AdapterRegistry::from_candidates(bundled_candidates().unwrap(), &config, Vec::new())
                .unwrap();
        assert_eq!(registry.find("codex").unwrap().adapter_version, "1.0.0");
        assert!(registry
            .diagnostics()
            .iter()
            .any(|message| message.contains("codex/9.9.9")));
        assert!(registry
            .diagnostics()
            .iter()
            .any(|message| message.contains("ghost")));
    }
}
