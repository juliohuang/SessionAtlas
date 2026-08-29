//! `~/.sessionatlas/config.json` read/write: genuinely case-insensitive JSON
//! property lookup, `SESSIONATLAS_HOME` resolution, a bounded cross-process
//! exclusive lock, fingerprint conflict detection, crash-safe atomic replace
//! and strict stale temp cleanup.
//!
//! Persisted configuration contract:
//! * property names are matched ASCII-case-insensitively on read at both the
//!   `AppConfig` and the `ToolSource` levels. Serde's derive only matches
//!   exact names, so deserialization goes through `serde_json::Value` and a
//!   case-insensitive member lookup; serialization emits the established
//!   PascalCase keys;
//! * `~/.sessionatlas/config.json` resolves against `SESSIONATLAS_HOME` when
//!   set, otherwise the user profile on Windows and `$HOME` (or `/`) on POSIX;
//! * writers run under a `config.json.lock` exclusive lock acquired with a
//!   bounded retry (`ConfigError::Busy`);
//! * a save of a previously loaded config is rejected when the SHA-256
//!   fingerprint of the on-disk file no longer matches the one captured at
//!   load time (`ConfigError::Conflict`);
//! * replacement is atomic: bytes are written to a sibling temp file, flushed
//!   to disk, then renamed over the target (Windows `MoveFileExW`, POSIX
//!   `rename`); a failed replace leaves the previous file intact;
//! * stale generated temps (`<config>.tmp.<pid>.<guid>`) older than 24 h are
//!   reclaimed only under the lock and only for real generated names — never
//!   for reparse/symlink entries and never for look-alike names.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::model::{ToolSource, DEFAULT_OPEN_COMMAND_TEMPLATE};
use crate::private_fs;

/// Default terminal used when `config.json` omits `DefaultTerminal`.
pub const DEFAULT_TERMINAL: &str = "auto";

/// Lock timeout used when callers do not supply one.
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_millis(5000);

/// A generated temp file is reclaimed only when it is strictly older than
/// this age.
const STALE_TEMP_HOURS: i64 = 24;

/// Typed failure modes for config reads and writes.
#[derive(Debug)]
pub enum ConfigError {
    /// The supplied config path was blank or could not be made absolute.
    InvalidPath(String),
    /// An I/O operation failed (read, lock open, temp write, replace...).
    Io(io::Error),
    /// The config file is not valid JSON or its shape does not match.
    Json(serde_json::Error),
    /// The exclusive lock could not be acquired within the timeout.
    Busy(Option<io::Error>),
    /// The config changed on disk after it was loaded.
    Conflict,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::InvalidPath(path) => write!(f, "invalid configuration path: {path}"),
            ConfigError::Io(error) => write!(f, "config I/O error: {error}"),
            ConfigError::Json(error) => write!(f, "invalid config JSON: {error}"),
            ConfigError::Busy(_) => {
                write!(
                    f,
                    "configuration is busy; retry after the other writer finishes"
                )
            }
            ConfigError::Conflict => write!(
                f,
                "configuration changed after it was loaded; reload and retry the mutation"
            ),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Io(error) => Some(error),
            ConfigError::Json(error) => Some(error),
            ConfigError::Busy(Some(error)) => Some(error),
            _ => None,
        }
    }
}

/// User configuration with atomic, cross-process-safe mutation.
///
/// The three public fields define the persisted application configuration. Serialization
/// emits the established PascalCase keys; deserialization is
/// ASCII-case-insensitive for every property name. The private fields only
/// track where this instance was loaded from so [`AppConfig::save`] can detect
/// stale-object conflicts; they never participate in equality or JSON output.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AppConfig {
    /// Custom tool sources (`sessionatlas config add-tool`).
    pub custom_tools: Vec<ToolSource>,
    /// Explicitly enabled adapter ids. `None` preserves the first-run default:
    /// bundled official adapters are enabled and locally installed adapters
    /// remain disabled until the user selects them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled_adapters: Option<Vec<String>>,
    /// Active version overrides for independently installed adapters. Missing
    /// entries use the bundled version for official adapters and the newest
    /// installed version for user adapters.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub active_adapter_versions: BTreeMap<String, String>,
    /// Preferred tool per absolute project path.
    pub preferred_tools_by_path: BTreeMap<String, String>,
    /// Terminal used by `open`; `"auto"` picks the platform default.
    pub default_terminal: String,

    #[serde(skip)]
    source_path: Option<PathBuf>,
    #[serde(skip)]
    source_fingerprint: Option<String>,
}

impl AppConfig {
    /// Persists this config under an exclusive lock. When this instance was
    /// previously loaded from `path`, the on-disk fingerprint must still match
    /// the one captured at load time, otherwise [`ConfigError::Conflict`] is
    /// returned and nothing is written. On success the instance's source
    /// tracking is refreshed so repeated saves of the same instance work.
    pub fn save(
        &mut self,
        path: impl AsRef<Path>,
        lock_timeout: Option<Duration>,
    ) -> Result<(), ConfigError> {
        let path = normalize_config_path(path.as_ref())?;
        create_config_directory(&path)?;
        let _guard = acquire_lock(&path, lock_timeout.unwrap_or(DEFAULT_LOCK_TIMEOUT))?;
        cleanup_stale_temps(&path, Utc::now())?;

        let on_disk = fingerprint(read_config_bytes(&path)?.as_deref());
        let tracked_from_here = self
            .source_path
            .as_deref()
            .is_some_and(|source| same_path(source, &path));
        if tracked_from_here && self.source_fingerprint.as_deref() != on_disk.as_deref() {
            return Err(ConfigError::Conflict);
        }

        let serialized = serialize(self)?;
        atomic_write(&path, &serialized)?;
        track_source(self, &path, fingerprint(Some(&serialized)));
        Ok(())
    }

    /// Saves to the path this instance was loaded from, or to the default
    /// path when it was created fresh.
    pub fn save_default(&mut self, lock_timeout: Option<Duration>) -> Result<(), ConfigError> {
        let path = self.source_path.clone().unwrap_or_else(default_config_path);
        self.save(path, lock_timeout)
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            custom_tools: Vec::new(),
            enabled_adapters: None,
            active_adapter_versions: BTreeMap::new(),
            preferred_tools_by_path: BTreeMap::new(),
            default_terminal: DEFAULT_TERMINAL.to_string(),
            source_path: None,
            source_fingerprint: None,
        }
    }
}

impl PartialEq for AppConfig {
    fn eq(&self, other: &Self) -> bool {
        self.custom_tools == other.custom_tools
            && self.enabled_adapters == other.enabled_adapters
            && self.active_adapter_versions == other.active_adapter_versions
            && self.preferred_tools_by_path == other.preferred_tools_by_path
            && self.default_terminal == other.default_terminal
    }
}

impl Eq for AppConfig {}

impl<'de> Deserialize<'de> for AppConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| de::Error::custom("config root must be a JSON object"))?;

        let mut config = AppConfig::default();

        if let Some(entry) = field_ci(object, "CustomTools") {
            let list = entry
                .as_array()
                .ok_or_else(|| de::Error::custom("`CustomTools` must be an array"))?;
            for (index, element) in list.iter().enumerate() {
                let tool = tool_source_ci(element)
                    .map_err(|error| de::Error::custom(format!("CustomTools[{index}]: {error}")))?;
                config.custom_tools.push(tool);
            }
        }

        if let Some(entry) = field_ci(object, "EnabledAdapters") {
            if entry.is_null() {
                config.enabled_adapters = None;
            } else {
                config.enabled_adapters =
                    Some(Vec::<String>::deserialize(entry).map_err(|error| {
                        de::Error::custom(format!("`EnabledAdapters`: {error}"))
                    })?);
            }
        }

        if let Some(entry) = field_ci(object, "ActiveAdapterVersions") {
            config.active_adapter_versions = BTreeMap::<String, String>::deserialize(entry)
                .map_err(|error| de::Error::custom(format!("`ActiveAdapterVersions`: {error}")))?;
        }

        if let Some(entry) = field_ci(object, "PreferredToolsByPath") {
            config.preferred_tools_by_path = BTreeMap::<String, String>::deserialize(entry)
                .map_err(|error| de::Error::custom(format!("`PreferredToolsByPath`: {error}")))?;
        }

        if let Some(entry) = field_ci(object, "DefaultTerminal") {
            config.default_terminal = entry
                .as_str()
                .ok_or_else(|| de::Error::custom("`DefaultTerminal` must be a string"))?
                .to_string();
        }

        Ok(config)
    }
}

/// Effective sessionatlas home directory.
///
/// `SESSIONATLAS_HOME` (when set to a non-blank value) wins; otherwise the
/// user profile on Windows and `$HOME` on POSIX, with `/` as the final POSIX
/// fallback. A blank override (empty or whitespace-only) is treated as unset.
/// An override is converted to an absolute path without requiring it to exist.
pub fn home_directory() -> PathBuf {
    if let Some(override_home) = std::env::var_os("SESSIONATLAS_HOME") {
        if !override_home.to_string_lossy().trim().is_empty() {
            return std::path::absolute(&override_home)
                .unwrap_or_else(|_| PathBuf::from(override_home));
        }
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// `~/.sessionatlas/config.json` under the given home.
pub fn config_path_for_home(home: impl AsRef<Path>) -> PathBuf {
    home.as_ref().join(".sessionatlas").join("config.json")
}

/// Default config path under [`home_directory`].
pub fn default_config_path() -> PathBuf {
    config_path_for_home(home_directory())
}

/// Loads the config, returning a default when the file is missing or blank.
/// Errors only when the file exists but cannot be read or parsed.
pub fn load(path: impl AsRef<Path>) -> Result<AppConfig, ConfigError> {
    let path = normalize_config_path(path.as_ref())?;
    let bytes = read_config_bytes(&path)?;
    let mut config = deserialize_config(bytes.as_deref())?;
    track_source(&mut config, &path, fingerprint(bytes.as_deref()));
    Ok(config)
}

/// [`load`] that maps every failure to `None`; a missing or invalid config
/// yields `None` so callers can fall back to built-in behavior.
pub fn try_load(path: impl AsRef<Path>) -> Option<AppConfig> {
    load(path).ok()
}

/// Applies `mutation` under an exclusive cross-process lock and persists the
/// result atomically. Mirrors `AppConfig.Update(path, mutation, lockTimeout)`.
pub fn update(
    path: impl AsRef<Path>,
    lock_timeout: Option<Duration>,
    mutation: impl FnOnce(&mut AppConfig),
) -> Result<AppConfig, ConfigError> {
    let path = normalize_config_path(path.as_ref())?;
    create_config_directory(&path)?;
    let _guard = acquire_lock(&path, lock_timeout.unwrap_or(DEFAULT_LOCK_TIMEOUT))?;
    cleanup_stale_temps(&path, Utc::now())?;

    let bytes = read_config_bytes(&path)?;
    let mut config = deserialize_config(bytes.as_deref())?;
    mutation(&mut config);

    let serialized = serialize(&config)?;
    atomic_write(&path, &serialized)?;
    track_source(&mut config, &path, fingerprint(Some(&serialized)));
    Ok(config)
}

thread_local! {
    static FORCE_NEXT_REPLACE_FAILURE: Cell<bool> = const { Cell::new(false) };
}

/// Test-only hook mirroring `AppConfig.BeforeAtomicReplaceForTests`: the next
/// atomic replace on this thread fails after the temp file has been written
/// and flushed. Never call from production code.
#[doc(hidden)]
pub fn force_next_atomic_replace_failure() {
    FORCE_NEXT_REPLACE_FAILURE.with(|flag| flag.set(true));
}

fn track_source(config: &mut AppConfig, path: &Path, config_fingerprint: Option<String>) {
    config.source_path = Some(path.to_path_buf());
    config.source_fingerprint = config_fingerprint;
}

fn same_path(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

fn serialize(config: &AppConfig) -> Result<Vec<u8>, ConfigError> {
    serde_json::to_vec_pretty(config).map_err(ConfigError::Json)
}

fn deserialize_config(bytes: Option<&[u8]>) -> Result<AppConfig, ConfigError> {
    match bytes {
        None => Ok(AppConfig::default()),
        Some(bytes) => serde_json::from_slice(strip_utf8_bom(bytes)).map_err(ConfigError::Json),
    }
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes)
}

fn fingerprint(bytes: Option<&[u8]>) -> Option<String> {
    bytes.map(sha256_hex)
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Reads the config bytes with a bounded retry for transient I/O failures such
/// as Windows share violations. A missing file is
/// reported as `Ok(None)`.
fn read_config_bytes(path: &Path) -> Result<Option<Vec<u8>>, ConfigError> {
    #[cfg(unix)]
    {
        match fs::symlink_metadata(path) {
            Ok(_) => create_config_directory(path)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(ConfigError::Io(error)),
        }
        private_fs::harden_existing_private_file(path).map_err(ConfigError::Io)?;
    }
    for attempt in 0..5u32 {
        match fs::read(path) {
            Ok(bytes) => return Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) if attempt == 4 => return Err(ConfigError::Io(error)),
            Err(_) => thread::sleep(Duration::from_millis(5)),
        }
    }
    unreachable!("bounded retry loop always returns")
}

fn normalize_config_path(path: &Path) -> Result<PathBuf, ConfigError> {
    let text = path.to_string_lossy();
    if text.trim().is_empty() {
        return Err(ConfigError::InvalidPath(text.into_owned()));
    }
    std::path::absolute(path).map_err(|_| ConfigError::InvalidPath(text.into_owned()))
}

fn create_config_directory(path: &Path) -> Result<(), ConfigError> {
    let directory = path
        .parent()
        .ok_or_else(|| ConfigError::InvalidPath(path.display().to_string()))?;
    if directory
        .file_name()
        .is_some_and(|name| name == ".sessionatlas")
    {
        private_fs::ensure_private_directory(directory).map_err(ConfigError::Io)
    } else {
        fs::create_dir_all(directory).map_err(ConfigError::Io)
    }
}

fn lock_path_for(config_path: &Path) -> PathBuf {
    let mut lock = config_path.as_os_str().to_os_string();
    lock.push(".lock");
    PathBuf::from(lock)
}

/// Acquires the exclusive cross-process lock on `config.json.lock`, retrying
/// until `timeout` elapses. Holding the returned handle keeps the lock; the
/// lock is released when the handle is dropped.
fn acquire_lock(config_path: &Path, timeout: Duration) -> Result<File, ConfigError> {
    let lock_path = lock_path_for(config_path);
    let deadline = Instant::now() + timeout;
    loop {
        match private_fs::open_private_read_write(&lock_path)
            .and_then(|file| file.try_lock_exclusive().map(|()| file))
        {
            Ok(file) => return Ok(file),
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err(ConfigError::Busy(Some(error)));
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

/// Writes `bytes` to a sibling temp file, flushes them to disk, then replaces
/// `path` atomically. Any failure (including the test hook) leaves the
/// previous file intact and removes the temp file.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ConfigError> {
    let directory = path
        .parent()
        .ok_or_else(|| ConfigError::InvalidPath(path.display().to_string()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| ConfigError::InvalidPath(path.display().to_string()))?;
    let temp_path = directory.join(format!(
        "{}.tmp.{}.{}",
        file_name.to_string_lossy(),
        std::process::id(),
        Uuid::new_v4().simple()
    ));

    let outcome = (|| -> Result<(), ConfigError> {
        write_temp_and_sync(&temp_path, bytes)?;
        if FORCE_NEXT_REPLACE_FAILURE.with(|flag| flag.replace(false)) {
            return Err(ConfigError::Io(io::Error::other(
                "forced atomic replace failure (test hook)",
            )));
        }
        atomic_replace_file(&temp_path, path).map_err(ConfigError::Io)
    })();

    if outcome.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    outcome
}

fn write_temp_and_sync(temp_path: &Path, bytes: &[u8]) -> Result<(), ConfigError> {
    let mut file = private_fs::open_private_create_new(temp_path).map_err(ConfigError::Io)?;
    file.write_all(bytes).map_err(ConfigError::Io)?;
    file.sync_all().map_err(ConfigError::Io)?;
    Ok(())
}

/// Replaces `target_path` with the sibling temporary file atomically.
///
/// Windows uses `ReplaceFileW` when the target already exists and `MoveFileExW`
/// for the first write; elsewhere `rename` replaces atomically. Scanner and
/// Tauri caches use this helper too,
/// so their second write has the same overwrite semantics as config writes.
pub fn atomic_replace_file(temp_path: &Path, target_path: &Path) -> io::Result<()> {
    replace_file(temp_path, target_path)
}

/// Crash-safe atomic replacement implementation.
#[cfg(windows)]
fn replace_file(temp_path: &Path, target_path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_WRITE_THROUGH, REPLACEFILE_IGNORE_MERGE_ERRORS,
        REPLACEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = temp_path.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = target_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();

    // SAFETY: both buffers are NUL-terminated wide strings that stay alive for
    // the call. The temp and target are siblings on the same volume.
    let ok = unsafe {
        if target_path.exists() {
            ReplaceFileW(
                destination.as_ptr(),
                source.as_ptr(),
                ptr::null(),
                REPLACEFILE_IGNORE_MERGE_ERRORS | REPLACEFILE_WRITE_THROUGH,
                ptr::null(),
                ptr::null(),
            )
        } else {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        }
    };
    if ok == 0 {
        Err(io::Error::from_raw_os_error(
            unsafe { GetLastError() } as i32
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(temp_path: &Path, target_path: &Path) -> io::Result<()> {
    fs::rename(temp_path, target_path)
}

/// Removes stale generated temp files older than 24 h from the config's
/// directory. A candidate is reclaimed only when it has a strictly generated
/// name, is not a reparse/symlink entry, and its last-write time is strictly
/// older than 24 h. Individual deletion failures are ignored.
fn cleanup_stale_temps(config_path: &Path, now: DateTime<Utc>) -> Result<(), ConfigError> {
    let directory = config_path
        .parent()
        .ok_or_else(|| ConfigError::InvalidPath(config_path.display().to_string()))?;
    let file_name = config_path
        .file_name()
        .ok_or_else(|| ConfigError::InvalidPath(config_path.display().to_string()))?;
    let prefix = format!("{}.tmp.", file_name.to_string_lossy());

    for entry in fs::read_dir(directory).map_err(ConfigError::Io)? {
        let entry = entry.map_err(ConfigError::Io)?;
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if !name.starts_with(&prefix) || !is_generated_temp_name(&name[prefix.len()..]) {
            continue;
        }
        let entry_path = entry.path();
        if is_reparse_or_symlink(&entry_path) {
            continue;
        }
        let modified = fs::metadata(&entry_path)
            .map_err(ConfigError::Io)?
            .modified()
            .map_err(ConfigError::Io)?;
        let modified = DateTime::<Utc>::from(modified);
        if now.signed_duration_since(modified) > chrono::Duration::hours(STALE_TEMP_HOURS) {
            let _ = fs::remove_file(&entry_path);
        }
    }
    Ok(())
}

/// True only for `<positive pid>.<32 hex chars>` — the exact shape the temp
/// writer generates.
fn is_generated_temp_name(suffix: &str) -> bool {
    let mut parts = suffix.split('.');
    let pid_part = match parts.next() {
        Some(part) => part,
        None => return false,
    };
    let guid_part = match parts.next() {
        Some(part) => part,
        None => return false,
    };
    if parts.next().is_some() {
        return false;
    }
    match pid_part.parse::<u32>() {
        Ok(pid) if pid > 0 => {}
        _ => return false,
    }
    guid_part.len() == 32 && guid_part.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Detects a reparse point on Windows and a symlink on POSIX. Used to keep
/// cleanup from ever following or deleting link entries that merely look like
/// generated temps.
fn is_reparse_or_symlink(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileAttributesW, FILE_ATTRIBUTE_REPARSE_POINT, INVALID_FILE_ATTRIBUTES,
        };

        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: `wide` is a NUL-terminated buffer valid for the call duration.
        let attributes = unsafe { GetFileAttributesW(wide.as_ptr()) };
        attributes != INVALID_FILE_ATTRIBUTES && (attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0
    }
    #[cfg(not(windows))]
    {
        fs::symlink_metadata(path)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
    }
}

/// ASCII-case-insensitive lookup of a single JSON member.
fn field_ci<'a>(object: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    object
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value)
}

fn ci_string(object: &Map<String, Value>, name: &str) -> Result<Option<String>, serde_json::Error> {
    match field_ci(object, name) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(|text| Some(text.to_string()))
            .ok_or_else(|| serde::de::Error::custom(format!("`{name}` must be a string"))),
    }
}

fn ci_bool(object: &Map<String, Value>, name: &str) -> Result<Option<bool>, serde_json::Error> {
    match field_ci(object, name) {
        None => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom(format!("`{name}` must be a boolean"))),
    }
}

/// ASCII-case-insensitive `ToolSource` deserialization for custom-tool objects.
/// Unknown members are ignored and omitted members use the domain defaults.
fn tool_source_ci(value: &Value) -> Result<ToolSource, serde_json::Error> {
    let object = value
        .as_object()
        .ok_or_else(|| serde::de::Error::custom("tool source entry must be a JSON object"))?;

    Ok(ToolSource {
        key: ci_string(object, "Key")?.unwrap_or_default(),
        name: ci_string(object, "Name")?.unwrap_or_default(),
        cli_command: ci_string(object, "CliCommand")?.unwrap_or_default(),
        data_directory: ci_string(object, "DataDirectory")?.unwrap_or_default(),
        scanner_type: ci_string(object, "ScannerType")?.unwrap_or_default(),
        is_installed: ci_bool(object, "IsInstalled")?.unwrap_or_default(),
        is_enabled: ci_bool(object, "IsEnabled")?.unwrap_or(true),
        open_command_template: ci_string(object, "OpenCommandTemplate")?
            .unwrap_or_else(|| DEFAULT_OPEN_COMMAND_TEMPLATE.to_string()),
    })
}
