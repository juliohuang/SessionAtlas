use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use rusqlite::{params, Connection, OpenFlags, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sessionatlas_core::config as core_config;
use sessionatlas_core::indexer::{build_index, IndexedToolScan};
use sessionatlas_core::scanner::aider::AiderScanner;
use sessionatlas_core::scanner::claude::ClaudeScanner;
use sessionatlas_core::scanner::codex::CodexScanner;
use sessionatlas_core::scanner::custom::CustomToolScanner;
use sessionatlas_core::scanner::kimi::KimiScanner;
use sessionatlas_core::scanner::opencode::OpenCodeScanner;
use sessionatlas_core::scanner::{
    ScanDiagnostic, ScanDiagnosticSeverity, Scanner, CONFIG_READ_FAILED, UNEXPECTED_SCANNER_FAILURE,
};
use sessionatlas_core::store::SqliteStore;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use tauri::menu::{IsMenuItem, MenuBuilder, MenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent, Wry};
use tauri_plugin_notification::NotificationExt;

mod process;
mod pty;
mod security;

use process::{git_read_spec, ProcessOutput, ProcessRunner, SystemProcessRunner};
use pty::{normalize_pty_size, take_once, validate_pty_input, SessionStore, Utf8StreamDecoder};
use security::{
    build_argv_launch_input, is_shell_program, parse_command_template, quote_remote_path,
    render_shell_command, ssh_destination, tool_launch_argv, validate_cli_argv,
    validate_display_label, validate_external_url, validate_session_id, validate_ssh_host,
    validate_ssh_user, validate_tool_key,
};

const HOME_OVERRIDE_ENV: &str = "SESSIONATLAS_HOME";
const DATA_DIRECTORY: &str = ".sessionatlas";

fn resolve_home_directory(
    override_home: Option<&str>,
    os_home: Option<PathBuf>,
    current_directory: &Path,
) -> PathBuf {
    let selected = override_home
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or(os_home)
        .unwrap_or_else(|| current_directory.to_path_buf());
    if selected.is_absolute() {
        selected
    } else {
        current_directory.join(selected)
    }
}

fn app_home_directory() -> PathBuf {
    let current_directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let override_home = std::env::var(HOME_OVERRIDE_ENV).ok();
    resolve_home_directory(
        override_home.as_deref(),
        dirs::home_dir(),
        &current_directory,
    )
}

fn data_path_for_home(home: &Path, file_name: &str) -> PathBuf {
    home.join(DATA_DIRECTORY).join(file_name)
}

/// Locate the shared SQLite index created by the `sessionatlas` CLI.
fn db_path() -> PathBuf {
    data_path_for_home(&app_home_directory(), "index.db")
}

fn cli_config_path() -> PathBuf {
    data_path_for_home(&app_home_directory(), "config.json")
}

#[derive(Deserialize, Default, Debug)]
struct CliConfigFile {
    #[serde(rename = "CustomTools", alias = "customTools", default)]
    custom_tools: Vec<CliToolConfig>,
}

#[derive(Deserialize, Debug)]
struct CliToolConfig {
    #[serde(rename = "Key", alias = "key")]
    key: String,
    #[serde(rename = "CliCommand", alias = "cliCommand")]
    cli_command: String,
    #[serde(rename = "IsEnabled", alias = "isEnabled", default = "default_true")]
    is_enabled: bool,
}

fn default_true() -> bool {
    true
}

fn load_cli_config() -> Result<CliConfigFile, String> {
    let path = cli_config_path();
    if !path.exists() {
        return Ok(CliConfigFile::default());
    }
    let json = std::fs::read_to_string(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    serde_json::from_str(&json)
        .map_err(|error| format!("invalid CLI config at {}: {error}", path.display()))
}

fn builtin_tool_key(tool_key: &str) -> Option<&'static str> {
    ["claude", "codex", "kimi", "opencode", "aider"]
        .into_iter()
        .find(|builtin| builtin.eq_ignore_ascii_case(tool_key))
}

fn resolve_tool_launch_argv_from_config(
    tool_key: &str,
    session_id: Option<&str>,
    config: &CliConfigFile,
) -> Result<Vec<String>, String> {
    let tool_key = validate_tool_key(tool_key)?;
    if let Some(builtin) = builtin_tool_key(tool_key) {
        return tool_launch_argv(builtin, session_id);
    }

    let configured = config
        .custom_tools
        .iter()
        .find(|tool| tool.is_enabled && tool.key.eq_ignore_ascii_case(tool_key))
        .ok_or_else(|| format!("tool '{tool_key}' is not enabled in config.json"))?;
    let mut args = parse_command_template(&configured.cli_command)?;
    if let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) {
        args.push("--resume".to_string());
        args.push(validate_session_id(session_id)?.to_string());
    }
    validate_cli_argv(&args)?;
    Ok(args)
}

fn resolve_configured_tool_launch_argv(
    tool_key: &str,
    session_id: Option<&str>,
) -> Result<Vec<String>, String> {
    if let Some(builtin) = builtin_tool_key(tool_key) {
        return tool_launch_argv(builtin, session_id);
    }
    let config = load_cli_config()?;
    resolve_tool_launch_argv_from_config(tool_key, session_id, &config)
}

#[derive(Serialize, Clone)]
pub struct ToolUsage {
    #[serde(rename = "toolKey")]
    tool_key: String,
    #[serde(rename = "toolName")]
    tool_name: String,
    #[serde(rename = "lastUsedAt")]
    last_used_at: String,
    #[serde(rename = "sessionCount")]
    session_count: i64,
    #[serde(rename = "lastSessionId")]
    last_session_id: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct Project {
    id: String,
    path: String,
    name: String,
    #[serde(rename = "lastAccessedAt")]
    last_accessed_at: String,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
    #[serde(rename = "toolUsages")]
    tool_usages: Vec<ToolUsage>,
}

/// A user-added SSH server. Frontend gets this verbatim and renders the
/// drawer "Remote servers" section from it.
#[derive(Serialize, Clone)]
pub struct RemoteServer {
    #[serde(rename = "id")]
    id: i64,
    #[serde(rename = "label")]
    label: String,
    #[serde(rename = "user")]
    user: String,
    #[serde(rename = "host")]
    host: String,
    #[serde(rename = "port")]
    port: i64,
    #[serde(rename = "identityFile")]
    identity_file: Option<String>,
    #[serde(rename = "scanRoots")]
    scan_roots: String,
    #[serde(rename = "createdAt")]
    created_at: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct RemoteConnectionProbe {
    home: String,
    #[serde(rename = "tmuxAvailable")]
    tmux_available: bool,
    #[serde(rename = "tmuxVersion")]
    tmux_version: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
struct RemoteScanServerResult {
    #[serde(rename = "serverId")]
    server_id: i64,
    count: i64,
    success: bool,
    #[serde(rename = "errorKind")]
    error_kind: Option<String>,
    message: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
struct RemoteScanBatchResult {
    #[serde(rename = "totalCount")]
    total_count: i64,
    #[serde(rename = "successCount")]
    success_count: i64,
    #[serde(rename = "failureCount")]
    failure_count: i64,
    partial: bool,
    servers: Vec<RemoteScanServerResult>,
}

fn summarize_remote_scan_outcomes(
    outcomes: Vec<(i64, Result<i64, String>)>,
) -> RemoteScanBatchResult {
    let mut total_count = 0;
    let mut success_count = 0;
    let mut failure_count = 0;
    let mut servers = Vec::with_capacity(outcomes.len());
    for (server_id, outcome) in outcomes {
        match outcome {
            Ok(count) => {
                total_count += count;
                success_count += 1;
                servers.push(RemoteScanServerResult {
                    server_id,
                    count,
                    success: true,
                    error_kind: None,
                    message: None,
                });
            }
            Err(message) => {
                failure_count += 1;
                let error_kind = if message.contains("ssh") || message.contains("SSH") {
                    "ssh"
                } else {
                    "remote_scan"
                };
                servers.push(RemoteScanServerResult {
                    server_id,
                    count: 0,
                    success: false,
                    error_kind: Some(error_kind.to_string()),
                    message: Some(message),
                });
            }
        }
    }
    RemoteScanBatchResult {
        total_count,
        success_count,
        failure_count,
        partial: failure_count > 0,
        servers,
    }
}

/// A project discovered on a remote server via `scan_remote_server`.
/// Same camelCase shape as `Project` (so the frontend can merge into
/// `state.all` without conditional rendering), plus `source` and
/// `remoteServerId` so the ledger can show the remote dot and look up
/// the parent server for the tooltip.
#[derive(Serialize, Clone)]
pub struct RemoteProject {
    #[serde(rename = "id")]
    id: String,
    #[serde(rename = "source")]
    source: String, // always "remote"
    #[serde(rename = "remoteServerId")]
    remote_server_id: i64,
    #[serde(rename = "path")]
    path: String,
    #[serde(rename = "name")]
    name: String,
    #[serde(rename = "lastAccessedAt")]
    last_accessed_at: String,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
    #[serde(rename = "toolUsages")]
    tool_usages: Vec<ToolUsage>,
}

/// Cached read-write handles for the two SQLite files. Lazy-initialized
/// on first use so the index DB can be missing at startup (the app shows
/// an error card and waits for `sessionatlas scan` to populate it). WAL mode
/// is enabled so concurrent readers never block each other.
static INDEX_DB: OnceLock<Mutex<Connection>> = OnceLock::new();
static PREFS_DB: OnceLock<Mutex<Connection>> = OnceLock::new();

fn configure_prefs_connection(c: &Connection) -> Result<(), String> {
    // WAL: readers don't block writers and vice versa. NORMAL synchronous
    // trades a sliver of crash-safety for big write speedups — acceptable
    // since both DBs are derived caches (index.db) or user prefs (prefs.db).
    c.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| e.to_string())?;
    c.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| e.to_string())?;
    // Enforce FKs so `delete_group` cascades to project_group_assignments
    // (and any future ON DELETE CASCADE). Off by default in SQLite.
    c.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn open_index_db() -> Result<Connection, String> {
    let path = db_path();
    open_index_reader(&path)
}

/// Open the index strictly for reads. The console queries through this
/// read-only handle; the index itself is written in-process by `scan_projects`
/// (or by the `sessionatlas` CLI) through the core `SqliteStore`.
fn open_index_reader(path: &std::path::Path) -> Result<Connection, String> {
    if !path.exists() {
        return Err(format!(
            "index not found at {} — run `sessionatlas scan` first",
            path.display()
        ));
    }
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("could not open read-only index {}: {e}", path.display()))?;
    c.pragma_update(None, "query_only", "ON")
        .map_err(|e| format!("could not configure read-only index: {e}"))?;
    c.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("could not configure index timeout: {e}"))?;
    Ok(c)
}

fn conn() -> Result<&'static Mutex<Connection>, String> {
    if let Some(m) = INDEX_DB.get() {
        return Ok(m);
    }
    let c = open_index_db()?;
    // Race-safe: if another thread won, drop ours and return theirs.
    Ok(INDEX_DB.get_or_init(|| Mutex::new(c)))
}

/// Lock the cached index DB and run `f`. `f` must NOT call `with_index`
/// recursively (would deadlock). Errors from init or the closure propagate.
fn with_index<T, F>(f: F) -> Result<T, String>
where
    F: FnOnce(&Connection) -> Result<T, String>,
{
    let m = conn()?;
    let g = m.lock().map_err(|e| e.to_string())?;
    f(&g)
}

/// A user-configurable external opener — built-in defaults seeded into the
/// prefs DB on first launch, plus user-defined entries the settings drawer
/// can add/delete.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenerPref {
    #[serde(rename = "id")]
    id: i64,
    #[serde(rename = "type")]
    r#type: String, // "builtin" | "custom"
    #[serde(rename = "builtinKey")]
    builtin_key: Option<String>,
    #[serde(rename = "label")]
    label: String,
    #[serde(rename = "command")]
    command: String,
    #[serde(rename = "enabled")]
    enabled: bool,
    #[serde(rename = "sortOrder")]
    sort_order: i64,
}

/// Path of the side SQLite file we own — separate from `index.db` (which
/// `sessionatlas scan` owns) so user preferences persist across rescans.
fn prefs_db_path() -> PathBuf {
    data_path_for_home(&app_home_directory(), "prefs.db")
}

fn open_prefs_db() -> Result<Connection, String> {
    let path = prefs_db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let c = Connection::open(&path).map_err(|e| e.to_string())?;
    configure_prefs_connection(&c)?;
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS opener_prefs (
             id              INTEGER PRIMARY KEY AUTOINCREMENT,
             type            TEXT    NOT NULL CHECK (type IN ('builtin','custom')),
             builtin_key     TEXT,
             label           TEXT    NOT NULL,
             command_template TEXT   NOT NULL,
             enabled         INTEGER NOT NULL DEFAULT 1,
             sort_order      INTEGER NOT NULL DEFAULT 0,
             created_at      TEXT    NOT NULL DEFAULT (datetime('now')),
             UNIQUE(builtin_key)
         );
         CREATE INDEX IF NOT EXISTS idx_opener_prefs_sort ON opener_prefs(sort_order);
         INSERT OR IGNORE INTO opener_prefs (type, builtin_key, label, command_template, enabled, sort_order)
         VALUES ('builtin','vscode',  'VSCode',  'code {path}',       1, 10),
                ('builtin','finder',  'Explorer','explorer {path}',   1, 20),
                ('builtin','terminal','Terminal','wt -d {path}',      0, 30);

         CREATE TABLE IF NOT EXISTS project_groups (
             id          INTEGER PRIMARY KEY AUTOINCREMENT,
             name        TEXT    NOT NULL UNIQUE,
             sort_order  INTEGER NOT NULL DEFAULT 0,
             created_at  TEXT    NOT NULL DEFAULT (datetime('now'))
         );
         CREATE INDEX IF NOT EXISTS idx_project_groups_sort ON project_groups(sort_order);
         CREATE TABLE IF NOT EXISTS project_group_assignments (
             project_id  TEXT    PRIMARY KEY,
             group_id    INTEGER NOT NULL,
             FOREIGN KEY (group_id) REFERENCES project_groups(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_group_assignments_group ON project_group_assignments(group_id);

         -- Manual per-group sort order. Lives in prefs.db (not index.db,
         -- which `sessionatlas scan` updates) so a user's drag order survives
         -- rescans. group_key is 'ungrouped' or str(group_id). A group is
         -- manual iff >=1 of its members has a row here; projects with no
         -- row sort to the end (Infinity fallback) so newly scanned entries
         -- in a manual group auto-append with no rescan-sync step.
         CREATE TABLE IF NOT EXISTS project_sort (
             project_id  TEXT    PRIMARY KEY,
             group_key   TEXT    NOT NULL,
             sort_order  INTEGER NOT NULL
          );
          CREATE INDEX IF NOT EXISTS idx_project_sort_group ON project_sort(group_key, sort_order);

          CREATE TABLE IF NOT EXISTS prefs_revisions (
              scope     TEXT PRIMARY KEY,
              revision  INTEGER NOT NULL
          );
          INSERT OR IGNORE INTO prefs_revisions (scope, revision) VALUES ('groups', 0);

          -- Remote SSH servers the user has added. We own this data
         -- (unlike index.db which `sessionatlas scan` owns), so the user
         -- can add / delete servers without losing the local index.
         -- identity_file is optional (omit when the user relies on
         -- ssh-agent). scan_roots is a space-separated list of paths
         -- (with ~ expanded by the remote shell) to recursively scan
         -- for git projects; default covers the common layouts.
         CREATE TABLE IF NOT EXISTS remote_servers (
             id              INTEGER PRIMARY KEY AUTOINCREMENT,
             label           TEXT    NOT NULL,
             user            TEXT    NOT NULL,
             host            TEXT    NOT NULL,
             port            INTEGER NOT NULL DEFAULT 22,
             identity_file   TEXT,
             scan_roots      TEXT    NOT NULL DEFAULT '~ ~/projects ~/code',
             created_at      TEXT    NOT NULL DEFAULT (datetime('now'))
         );

         -- One row per git project discovered on a remote server. We
         -- write these ourselves via `scan_remote_server` (no `sessionatlas`
         -- required on the remote box). project_id is synthetic and
         -- stable across rescans so the frontend can use it as a key.
         CREATE TABLE IF NOT EXISTS remote_projects (
             project_id       TEXT    PRIMARY KEY,
             server_id        INTEGER NOT NULL,
             path             TEXT    NOT NULL,
             name             TEXT    NOT NULL,
             last_accessed_at TEXT    NOT NULL,
             git_branch       TEXT,
             FOREIGN KEY (server_id) REFERENCES remote_servers(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_remote_projects_server ON remote_projects(server_id);

         -- Per-tool session usage for remote projects. Mirrors the local
         -- `tool_usages` table on index.db (which is owned by the
         -- `sessionatlas` CLI). The frontend calls record_remote_tool_usage
         -- right after a remote PTY auto-launches a tool, so the next
         -- list_remote_projects reflects the touched session.
         CREATE TABLE IF NOT EXISTS remote_tool_usages (
             server_id       INTEGER NOT NULL,
             project_id      TEXT    NOT NULL,
             tool_key        TEXT    NOT NULL,
             tool_name       TEXT    NOT NULL,
             last_used_at    TEXT    NOT NULL,
             session_count   INTEGER NOT NULL DEFAULT 0,
             last_session_id TEXT,
             PRIMARY KEY (server_id, project_id, tool_key),
             FOREIGN KEY (server_id) REFERENCES remote_servers(id) ON DELETE CASCADE
         );"
    ).map_err(|e| e.to_string())?;
    Ok(c)
}

fn prefs_conn() -> Result<&'static Mutex<Connection>, String> {
    if let Some(m) = PREFS_DB.get() {
        return Ok(m);
    }
    let c = open_prefs_db()?;
    Ok(PREFS_DB.get_or_init(|| Mutex::new(c)))
}

/// Lock the cached prefs DB and run `f`. The prefs DB always inits
/// successfully (creates its file), so this only fails on closure errors.
fn with_prefs<T, F>(f: F) -> Result<T, String>
where
    F: FnOnce(&Connection) -> Result<T, String>,
{
    let m = prefs_conn()?;
    let g = m.lock().map_err(|e| e.to_string())?;
    f(&g)
}

fn with_prefs_transaction<T, F>(f: F) -> Result<T, String>
where
    F: FnOnce(&Transaction<'_>) -> Result<T, String>,
{
    let m = prefs_conn()?;
    let mut connection = m.lock().map_err(|e| e.to_string())?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| format!("could not begin prefs transaction: {e}"))?;
    let result = f(&transaction);
    match result {
        Ok(value) => {
            transaction
                .commit()
                .map_err(|e| format!("could not commit prefs transaction: {e}"))?;
            Ok(value)
        }
        Err(error) => Err(error),
    }
}

fn row_to_opener(r: &rusqlite::Row) -> rusqlite::Result<OpenerPref> {
    Ok(OpenerPref {
        id: r.get(0)?,
        r#type: r.get(1)?,
        builtin_key: r.get(2)?,
        label: r.get(3)?,
        command: r.get(4)?,
        enabled: r.get::<_, i64>(5)? != 0,
        sort_order: r.get(6)?,
    })
}

/// Build a Project from row fields. Tool usages are attached separately
/// by the caller so we can fetch them in a single batched query instead
/// of one per project (the N+1 the original implementation had).
fn row_to_project(
    id: &str,
    path: &str,
    last: &str,
    branch: Option<String>,
    usages: Vec<ToolUsage>,
) -> Project {
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    Project {
        id: id.to_string(),
        path: path.to_string(),
        name,
        last_accessed_at: last.to_string(),
        git_branch: branch,
        tool_usages: usages,
    }
}

/// Fetch all tool_usages for the given project ids in a single query,
/// returning a map keyed by project_id. Saves N prepares per list query.
fn fetch_usages_by_project(
    c: &Connection,
    ids: &[String],
) -> Result<HashMap<String, Vec<ToolUsage>>, String> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT project_id, tool_name, tool_key, last_used_at, session_count, last_session_id
         FROM tool_usages
         WHERE project_id IN ({placeholders})
         ORDER BY last_used_at DESC"
    );
    let mut stmt = c.prepare(&sql).map_err(|e| e.to_string())?;
    let params_vec: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let rows = stmt
        .query_map(params_vec.as_slice(), |r| {
            Ok((
                r.get::<_, String>(0)?,
                ToolUsage {
                    tool_key: r.get::<_, String>(2)?,
                    tool_name: r.get::<_, String>(1)?,
                    last_used_at: r.get::<_, String>(3)?,
                    session_count: r.get::<_, i64>(4)?,
                    last_session_id: r.get::<_, Option<String>>(5)?,
                },
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut map: HashMap<String, Vec<ToolUsage>> = HashMap::new();
    for row in collect_query_rows(rows, "fetch_usages_by_project")? {
        map.entry(row.0).or_default().push(row.1);
    }
    Ok(map)
}

type ProjectRow = (String, String, String, Option<String>);
fn row_to_project_fields(r: &rusqlite::Row) -> rusqlite::Result<ProjectRow> {
    Ok((
        r.get::<_, String>(0)?,
        r.get::<_, String>(1)?,
        r.get::<_, String>(2)?,
        r.get::<_, Option<String>>(3)?,
    ))
}

/// Materialize rusqlite mapped rows without silently dropping a decode error.
/// `Iterator::flatten()` is intentionally not used for database rows because
/// one malformed row must fail the whole command, not disappear from it.
fn collect_query_rows<T, I>(rows: I, context: &str) -> Result<Vec<T>, String>
where
    I: IntoIterator<Item = rusqlite::Result<T>>,
{
    rows.into_iter()
        .map(|row| row.map_err(|error| format!("{context}: {error}")))
        .collect()
}

/// Take the materialized `(id, path, last_accessed_at, git_branch)` rows from
/// a projects-list query and attach tool_usages in one batched fetch. Used by
/// both `list_projects` and `search_projects`.
fn projects_from_rows(c: &Connection, rows: Vec<ProjectRow>) -> Result<Vec<Project>, String> {
    let ids: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
    let usages = fetch_usages_by_project(c, &ids)?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let u = usages.get(&r.0).cloned().unwrap_or_default();
            row_to_project(&r.0, &r.1, &r.2, r.3, u)
        })
        .collect())
}

/// AI-CLI home subdirectories whose contents are tool internals (session
/// stores, configs, caches) rather than real projects. Projects living under
/// any of these are excluded from the listing so they don't clutter the
/// ledger. Component-based prefix match via `Path::starts_with` is safe
/// across path separators and won't false-match `.claudefoo`-style names.
fn is_excluded_project_path(path: &str) -> bool {
    let home = app_home_directory();
    let p = std::path::Path::new(path);
    [".claude", ".codex", ".kimi", ".opencode", ".aider"]
        .iter()
        .any(|sub| p.starts_with(home.join(sub)))
}

#[tauri::command]
fn list_projects(limit: Option<i64>) -> Result<Vec<Project>, String> {
    let limit = validate_project_limit(limit)?;
    with_index(|c| {
        // No SQL LIMIT: we filter excluded paths in Rust first, then cap,
        // so excluded projects don't shrink the visible window.
        let mut stmt = c
            .prepare("SELECT id, path, last_accessed_at, git_branch FROM projects ORDER BY last_accessed_at DESC")
            .map_err(|e| e.to_string())?;
        let decoded_rows: Vec<ProjectRow> = collect_query_rows(
            stmt.query_map([], row_to_project_fields)
                .map_err(|e| e.to_string())?,
            "list_projects",
        )?;
        let rows: Vec<ProjectRow> = decoded_rows
            .into_iter()
            .filter(|r| !is_excluded_project_path(&r.1))
            .take(limit)
            .collect();
        projects_from_rows(c, rows)
    })
}

fn validate_project_limit(limit: Option<i64>) -> Result<usize, String> {
    let value = limit.unwrap_or(500);
    if !(1..=10_000).contains(&value) {
        return Err("project limit must be between 1 and 10000".to_string());
    }
    usize::try_from(value).map_err(|_| "project limit is outside the supported range".to_string())
}

#[tauri::command]
fn search_projects(query: String) -> Result<Vec<Project>, String> {
    let Some(pattern) = build_fts_prefix_query(&query) else {
        return Ok(Vec::new());
    };
    with_index(|c| {
        let mut stmt = c
            .prepare(
                "SELECT p.id, p.path, p.last_accessed_at, p.git_branch
                 FROM projects p
                 WHERE p.rowid IN (SELECT rowid FROM projects_fts WHERE projects_fts MATCH ?1)
                 ORDER BY p.last_accessed_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let decoded_rows: Vec<ProjectRow> = collect_query_rows(
            stmt.query_map(params![pattern], row_to_project_fields)
                .map_err(|e| e.to_string())?,
            "search_projects",
        )?;
        let rows: Vec<ProjectRow> = decoded_rows
            .into_iter()
            .filter(|r| !is_excluded_project_path(&r.1))
            .take(200)
            .collect();
        projects_from_rows(c, rows)
    })
}

fn build_fts_prefix_query(query: &str) -> Option<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    for character in query.chars() {
        if character.is_alphanumeric() || character == '_' {
            current.push(character);
            continue;
        }
        if !current.is_empty() {
            terms.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        terms.push(current);
    }
    if terms.is_empty() {
        None
    } else {
        Some(
            terms
                .into_iter()
                .map(|term| format!("\"{term}\"*"))
                .collect::<Vec<_>>()
                .join(" AND "),
        )
    }
}

#[derive(Serialize, Clone)]
pub struct Tool {
    #[serde(rename = "toolKey")]
    tool_key: String,
    #[serde(rename = "toolName")]
    tool_name: String,
}

/// Distinct tools that appear in the index, for building filter chips dynamically.
#[tauri::command]
fn list_tools() -> Result<Vec<Tool>, String> {
    with_index(|c| {
        let mut stmt = c
            .prepare("SELECT tool_key, tool_name FROM tool_usages GROUP BY tool_key ORDER BY MAX(last_used_at) DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Tool {
                    tool_key: r.get::<_, String>(0)?,
                    tool_name: r.get::<_, String>(1)?,
                })
            })
            .map_err(|e| e.to_string())?;
        collect_query_rows(rows, "list_tools")
    })
}

/// Builds the canonical scanner set for the config file at `config_path`: the
/// five built-in scanners in C# registration order (Claude, Kimi, Codex,
/// OpenCode, Aider), then each enabled custom tool whose key does not collide
/// with a built-in (case-insensitive). When the config cannot be read or
/// parsed, built-ins remain available and a `config_read_failed` warning is
/// returned, mirroring `ScannerRegistry` and `commands::scan::build_default_scanners`.
fn build_scan_scanners(config_path: &Path) -> (Vec<Box<dyn Scanner>>, Vec<ScanDiagnostic>) {
    let mut scanners: Vec<Box<dyn Scanner>> = vec![
        Box::new(ClaudeScanner::new()),
        Box::new(KimiScanner::new()),
        Box::new(CodexScanner::new()),
        Box::new(OpenCodeScanner::new()),
        Box::new(AiderScanner::new()),
    ];
    let mut diagnostics = Vec::new();
    match core_config::load(config_path) {
        Ok(config) => {
            for tool in config.custom_tools.iter().filter(|tool| tool.is_enabled) {
                if scanners
                    .iter()
                    .any(|scanner| scanner.tool_key().eq_ignore_ascii_case(&tool.key))
                {
                    continue;
                }
                scanners.push(Box::new(CustomToolScanner::new(tool.clone())));
            }
        }
        Err(_) => diagnostics.push(ScanDiagnostic::new(
            "config",
            ScanDiagnosticSeverity::Warning,
            CONFIG_READ_FAILED,
            "The custom-tool configuration could not be read; built-in scanners remain available.",
        )),
    }
    (scanners, diagnostics)
}

/// Runs the configured local scan in-process, mirroring the CLI's
/// `commands::scan::run_scan`: only successful outcomes feed
/// `replace_tool_snapshots`; `Failed`/`Unavailable` outcomes and scanner panics
/// preserve the previous snapshot. Zero successful tools returns a sanitized
/// error and performs no snapshot writes (the index is neither created nor
/// touched). On success the merged index is written through the core
/// `SqliteStore` — schema, migrations and FTS are rebuilt — and the total
/// project count is returned.
fn run_local_scan(db_path: &Path, config_path: &Path) -> Result<i64, String> {
    let (scanners, initial_diagnostics) = build_scan_scanners(config_path);
    run_scan_with_scanners(db_path, &scanners, &initial_diagnostics)
}

fn run_scan_with_scanners(
    db_path: &Path,
    scanners: &[Box<dyn Scanner>],
    initial_diagnostics: &[ScanDiagnostic],
) -> Result<i64, String> {
    let mut diagnostics: Vec<ScanDiagnostic> = initial_diagnostics.to_vec();
    let mut successful: Vec<IndexedToolScan> = Vec::new();

    for scanner in scanners {
        let outcome =
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scanner.scan())) {
                Ok(outcome) => outcome,
                Err(_) => {
                    diagnostics.push(ScanDiagnostic::new(
                        scanner.tool_key(),
                        ScanDiagnosticSeverity::Error,
                        UNEXPECTED_SCANNER_FAILURE,
                        "The scanner stopped unexpectedly; its previous index is preserved.",
                    ));
                    continue;
                }
            };
        diagnostics.extend(outcome.diagnostics().iter().cloned());
        if outcome.is_successful() {
            successful.push(IndexedToolScan {
                tool_key: scanner.tool_key().to_string(),
                tool_name: scanner.tool_name().to_string(),
                projects: outcome.into_projects(),
            });
        }
    }

    if successful.is_empty() {
        return Err(sanitize_scan_error(scanners.len()));
    }

    let projects = build_index(&successful);
    let scanned_keys: Vec<&str> = successful
        .iter()
        .map(|scan| scan.tool_key.as_str())
        .collect();
    let mut store = SqliteStore::new(db_path)
        .map_err(|error| format!("could not open index {}: {error}", db_path.display()))?;
    store
        .replace_tool_snapshots(&projects, &scanned_keys)
        .map_err(|error| format!("could not update index: {error}"))?;
    count_index_projects(db_path)
}

/// Counts the project rows of the freshly written index. A new read-only
/// connection observes the committed snapshot written by the scan worker's own
/// writer connection, so the count never depends on the app's cached reader.
fn count_index_projects(db_path: &Path) -> Result<i64, String> {
    let connection =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| {
                format!(
                    "could not open index {} for counting: {error}",
                    db_path.display()
                )
            })?;
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
        .map_err(|error| format!("could not count indexed projects: {error}"))?;
    Ok(count)
}

/// Builds the sanitized error returned when no tool produced a trustworthy
/// snapshot. Only constant text and the tool count are included, then control
/// characters (apart from newline/tab) are replaced so the message can never
/// smuggle terminal escapes to the frontend.
fn sanitize_scan_error(tool_count: usize) -> String {
    let message = format!(
        "No tool produced a trustworthy snapshot; the index was left unchanged \
         ({tool_count} tool(s) scanned, all preserved previous data).\n\
         没有工具产生可信快照，索引未发生变化（扫描 {tool_count} 个工具，全部保留了上一份索引）。"
    );
    message
        .chars()
        .map(|character| {
            if character == '\n' || character == '\r' || character == '\t' {
                character
            } else if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

/// Rescan every configured tool in-process and atomically replace the
/// snapshots of the tools that succeeded. All scanning, filesystem and SQLite
/// work runs on a blocking worker so the Tauri async executor is never blocked.
#[tauri::command]
async fn scan_projects() -> Result<i64, String> {
    let db_path = db_path();
    let config_path = cli_config_path();
    tauri::async_runtime::spawn_blocking(move || run_local_scan(&db_path, &config_path))
        .await
        .map_err(|error| format!("local scan worker panicked: {error}"))?
}

/// Launch an AI CLI in the project directory via an external terminal.
/// When `session_id` is given, appends `--resume <id>` so the CLI reopens
/// the recorded session.
#[tauri::command]
fn launch_project(
    path: String,
    tool_key: String,
    session_id: Option<String>,
) -> Result<(), String> {
    if !std::path::Path::new(&path).is_dir() {
        return Err(format!("directory not found: {path}"));
    }
    let tool_args = resolve_configured_tool_launch_argv(&tool_key, session_id.as_deref())?;
    #[cfg(target_os = "windows")]
    {
        let wt = dirs::cache_dir()
            .map(|d| d.join(r"..\Local\Microsoft\WindowsApps\wt.exe"))
            .unwrap_or_else(|| std::path::PathBuf::from("wt.exe"));
        let have_wt = wt.exists();
        let command = render_shell_command(&tool_args)?;
        if have_wt {
            Command::new(wt)
                .arg("-d")
                .arg(&path)
                .arg("cmd.exe")
                .arg("/D")
                .arg("/K")
                .arg(command)
                .spawn()
                .map_err(|e| e.to_string())?;
        } else {
            Command::new("cmd.exe")
                .current_dir(&path)
                .arg("/D")
                .arg("/K")
                .arg(command)
                .spawn()
                .map_err(|e| e.to_string())?;
        }
    }
    #[cfg(target_os = "macos")]
    {
        let path = security::posix_shell_quote(&path)?;
        let command = tool_args
            .iter()
            .map(|arg| security::posix_shell_quote(arg))
            .collect::<Result<Vec<_>, _>>()?
            .join(" ");
        let shell_command = format!("cd {path} && exec {command}");
        let script = "on run argv\n\
                      tell application \"Terminal\"\n\
                        activate\n\
                        do script (item 1 of argv)\n\
                      end tell\n\
                      end run";
        Command::new("osascript")
            .args(["-e", script, "--", &shell_command])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let term = std::env::var("TERMINAL").ok().filter(|t| !t.is_empty());
        let chosen = term
            .or_else(which("gnome-terminal"))
            .or_else(which("konsole"))
            .or_else(which("xterm"));
        match chosen {
            Some(t) => {
                // gnome-terminal needs `-- `, konsole/xterm use `-e`.
                let dash = if t.ends_with("gnome-terminal") {
                    "--"
                } else {
                    "-e"
                };
                let mut command = Command::new(t);
                command
                    .current_dir(&path)
                    .arg(dash)
                    .args(&tool_args)
                    .spawn()
                    .map_err(|e| e.to_string())?;
            }
            None => {
                // No known terminal: fall back to running the tool directly.
                let mut c = Command::new(&tool_args[0]);
                c.current_dir(&path);
                c.args(&tool_args[1..]);
                c.spawn().map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// Look up the absolute path of an executable on PATH, if present.
#[cfg(all(unix, not(target_os = "macos")))]
fn which(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let cand = dir.join(name);
            if cand.is_file() {
                Some(cand)
            } else {
                None
            }
        })
    })
}

/* ============================================================
External openers (VSCode, Explorer/Finder, custom commands).
Preferences live in a separate `prefs.db` (not `index.db`),
because `sessionatlas scan` updates the index and would otherwise
wipe user prefs. Frontend reads/writes via the commands below
and renders the enabled set as `--ember` pills under the
AI-tool launch row in the ledger.
============================================================ */

#[tauri::command]
fn list_opener_prefs() -> Result<Vec<OpenerPref>, String> {
    with_prefs(|c| {
        let mut stmt = c
            .prepare(
                "SELECT id, type, builtin_key, label, command_template, enabled, sort_order
             FROM opener_prefs ORDER BY sort_order ASC, id ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], row_to_opener)
            .map_err(|e| e.to_string())?;
        collect_query_rows(rows, "list_opener_prefs")
    })
}

#[tauri::command]
fn set_opener_enabled(pref_id: i64, enabled: bool) -> Result<(), String> {
    with_prefs(|c| set_opener_enabled_db(c, pref_id, enabled))
}

fn set_opener_enabled_db(c: &Connection, pref_id: i64, enabled: bool) -> Result<(), String> {
    let changed = c
        .execute(
            "UPDATE opener_prefs SET enabled = ?1 WHERE id = ?2",
            params![enabled as i64, pref_id],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err(format!("opener preference not found: {pref_id}"));
    }
    Ok(())
}

#[tauri::command]
fn set_opener_command(pref_id: i64, command: String) -> Result<(), String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("command template cannot be empty".into());
    }
    validate_custom_opener_template(trimmed)?;
    with_prefs(|c| set_opener_command_db(c, pref_id, trimmed))
}

fn set_opener_command_db(c: &Connection, pref_id: i64, command: &str) -> Result<(), String> {
    let changed = c
        .execute(
            "UPDATE opener_prefs SET command_template = ?1 WHERE id = ?2",
            params![command, pref_id],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err(format!("opener preference not found: {pref_id}"));
    }
    Ok(())
}

#[tauri::command]
fn upsert_custom_opener(
    label: String,
    command: String,
    enabled: bool,
) -> Result<OpenerPref, String> {
    let l = label.trim();
    let cmd = command.trim();
    if l.is_empty() || cmd.is_empty() {
        return Err("label and command are required".into());
    }
    validate_display_label(l)?;
    validate_custom_opener_template(cmd)?;
    with_prefs(|c| {
        let next: i64 = c
            .query_row(
                "SELECT COALESCE(MAX(sort_order),0)+10 FROM opener_prefs",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        c.execute(
            "INSERT INTO opener_prefs (type, builtin_key, label, command_template, enabled, sort_order)
             VALUES ('custom', NULL, ?1, ?2, ?3, ?4)",
            params![l, cmd, enabled as i64, next]
        ).map_err(|e| e.to_string())?;
        let id = c.last_insert_rowid();
        let pref = c
            .query_row(
                "SELECT id, type, builtin_key, label, command_template, enabled, sort_order
             FROM opener_prefs WHERE id = ?1",
                params![id],
                row_to_opener,
            )
            .map_err(|e| e.to_string())?;
        Ok(pref)
    })
}

#[tauri::command]
fn delete_custom_opener(pref_id: i64) -> Result<(), String> {
    with_prefs(|c| {
        let typ: String = c
            .query_row(
                "SELECT type FROM opener_prefs WHERE id = ?1",
                params![pref_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if typ != "custom" {
            return Err("only custom openers can be deleted".into());
        }
        c.execute("DELETE FROM opener_prefs WHERE id = ?1", params![pref_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// Resolve a builtin opener by key — VSCode / file manager / terminal —
/// platform-specific argv so paths with spaces are safe. The persisted
/// `command_template` is shown in the settings drawer for transparency
/// but is NOT what we spawn from; we always use these argv forms so the
/// platform quirks (explorer backslash, terminal flags, etc.) stay
/// correct even if the user edits the template.
#[tauri::command]
fn open_with_opener(opener_id: i64, path: String) -> Result<(), String> {
    let path_buf = std::path::PathBuf::from(&path);
    if !path_buf.is_dir() {
        return Err(format!("directory not found: {path}"));
    }
    let pref = with_prefs(|c| {
        c.query_row(
            "SELECT id, type, builtin_key, label, command_template, enabled, sort_order
             FROM opener_prefs WHERE id = ?1",
            params![opener_id],
            row_to_opener,
        )
        .map_err(|e| format!("opener not found: {e}"))
    })?;
    if !pref.enabled {
        return Err(format!("opener '{}' is disabled", pref.label));
    }
    match pref.builtin_key.as_deref() {
        Some("vscode") => open_vscode(&path_buf),
        Some("finder") => open_filemanager(&path_buf),
        Some("terminal") => open_terminal_native(&path_buf),
        _ => open_generic(&pref.command, &path_buf),
    }
}

fn open_vscode(path: &std::path::Path) -> Result<(), String> {
    let mut candidates = vec![std::path::PathBuf::from(if cfg!(windows) {
        "code.exe"
    } else {
        "code"
    })];
    #[cfg(windows)]
    {
        if let Some(local) = dirs::data_local_dir() {
            candidates.push(local.join(r"Programs\Microsoft VS Code\Code.exe"));
        }
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = std::env::var_os(variable) {
                candidates.push(std::path::PathBuf::from(root).join(r"Microsoft VS Code\Code.exe"));
            }
        }
    }

    for bin in candidates {
        match Command::new(&bin).arg(path).spawn() {
            Ok(_) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("VSCode CLI 'code' not found on PATH — install via VSCode Command Palette → 'Shell Command: Install code in PATH'".into())
}

fn open_filemanager(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        // explorer treats a trailing backslash as "open the parent dir" — strip it.
        let p = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let s = p.to_string_lossy().trim_end_matches('\\').to_string();
        Command::new("explorer")
            .arg(&s)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn open_terminal_native(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let wt = dirs::cache_dir()
            .map(|d| d.join(r"..\Local\Microsoft\WindowsApps\wt.exe"))
            .unwrap_or_else(|| std::path::PathBuf::from("wt.exe"));
        if wt.exists() {
            Command::new(wt)
                .arg("-d")
                .arg(path)
                .spawn()
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        // Fallback: inherit the requested working directory. No `cd` command
        // is needed, so the project path never enters cmd.exe syntax.
        Command::new("cmd.exe")
            .current_dir(path)
            .arg("/D")
            .arg("/K")
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let path = security::posix_shell_quote(&path.to_string_lossy())?;
        let shell_command = format!("cd {path}");
        let script = "on run argv\n\
                      tell application \"Terminal\"\n\
                        activate\n\
                        do script (item 1 of argv)\n\
                      end tell\n\
                      end run";
        Command::new("osascript")
            .args(["-e", script, "--", &shell_command])
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let term = std::env::var("TERMINAL")
            .ok()
            .filter(|t| !t.is_empty())
            .or_else(which("gnome-terminal"))
            .or_else(which("konsole"))
            .or_else(which("xterm"));
        match term {
            Some(t) => {
                Command::new(t)
                    .current_dir(path)
                    .spawn()
                    .map_err(|e| e.to_string())?;
            }
            None => {
                // Last resort: open the file manager so the user can navigate.
                Command::new("xdg-open")
                    .arg(path)
                    .spawn()
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}

fn open_generic(template: &str, path: &std::path::Path) -> Result<(), String> {
    let spec = build_generic_opener_spec(template, path)?;
    SystemProcessRunner.spawn(&spec)
}

fn validate_custom_opener_template(template: &str) -> Result<Vec<String>, String> {
    let argv = parse_command_template(template)?;
    if argv[0] == "{path}" {
        return Err("the {path} placeholder cannot be the opener program".into());
    }
    if is_shell_program(&argv[0]) {
        return Err("shell interpreters and script shims are not allowed as custom openers".into());
    }
    let mut path_tokens = 0;
    for token in &argv {
        if token == "{path}" {
            path_tokens += 1;
        } else if token.contains("{path}") {
            return Err("the {path} placeholder must be a complete argument".into());
        }
    }
    if path_tokens == 0 {
        return Err("custom opener template must contain a {path} argument".into());
    }
    Ok(argv)
}

fn build_generic_opener_spec(
    template: &str,
    path: &std::path::Path,
) -> Result<process::ProcessSpec, String> {
    let argv = validate_custom_opener_template(template)?;
    let mut spec = process::ProcessSpec::new(&argv[0]);
    for argument in &argv[1..] {
        spec = if argument == "{path}" {
            spec.arg(path.as_os_str())
        } else {
            spec.arg(argument)
        };
    }
    Ok(spec)
}

/* ============================================================
In-app terminal (PTY) sessions.
Each spawn opens a real pseudo-terminal running the user's
shell in the project directory; output is streamed to the
frontend via `pty-data` events, input comes back via
`pty_write`. One tab = one session.
============================================================ */

struct PtySession {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn std::io::Write + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    reader: Mutex<Option<Box<dyn std::io::Read + Send>>>,
    remote_server_id: Option<i64>,
}

#[derive(Default)]
struct PtyRegistry {
    sessions: SessionStore<PtySession>,
}

#[derive(Serialize, Clone)]
struct PtyData {
    id: u32,
    data: String,
}

#[derive(Serialize, Clone)]
struct PtyExit {
    id: u32,
    #[serde(rename = "exitCode")]
    exit_code: Option<u32>,
    #[serde(rename = "readError")]
    read_error: bool,
}

fn local_shell_command(path: &str) -> CommandBuilder {
    #[cfg(windows)]
    let program = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let program = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());

    let mut command = CommandBuilder::new(program);
    command.cwd(path);
    command
}

fn claude_print_argv(prompt: &str) -> Vec<String> {
    vec!["claude".to_string(), "-p".to_string(), prompt.to_string()]
}

fn finish_pty_session(
    sessions: &SessionStore<PtySession>,
    id: u32,
    kill_first: bool,
) -> Option<u32> {
    let session = sessions.remove(id).ok().flatten()?;
    let mut child = session
        .child
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if kill_first {
        let _ = child.kill();
    }
    child.wait().ok().map(|status| status.exit_code())
}

fn shutdown_pty_sessions(registry: &PtyRegistry) {
    let Ok(sessions) = registry.sessions.drain() else {
        return;
    };
    for session in sessions {
        let mut child = session
            .child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = child.kill();
        let _ = child.wait();
    }
}

const REMOTE_TMUX_MISSING_MESSAGE: &str = "SessionAtlas requires tmux for persistent remote terminals.\nInstall tmux and reconnect (Ubuntu/Debian: sudo apt install tmux; Fedora/RHEL: sudo dnf install tmux; macOS: brew install tmux).\nSessionAtlas 的持久化远程终端需要 tmux。\n请安装 tmux 后重新连接（Ubuntu/Debian：sudo apt install tmux；Fedora/RHEL：sudo dnf install tmux；macOS：brew install tmux）。";
const DEFAULT_REMOTE_SCAN_ROOTS: &str = "~ ~/projects ~/code";
const REMOTE_TERMINAL_TYPE: &str = "xterm-256color";
const REMOTE_TMUX_SOCKET: &str = "sessionatlas-v1";

fn configure_remote_pty_environment(command: &mut CommandBuilder) {
    // Windows commonly has no TERM (or inherits "dumb"). OpenSSH forwards
    // this value in its PTY request; tmux otherwise rejects attach with
    // "terminal does not support clear" before the remote TUI can start.
    command.env("TERM", REMOTE_TERMINAL_TYPE);
}

fn remote_tmux_session_name(path: &str, tool_key: Option<&str>) -> Result<String, String> {
    let tool_key = tool_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("shell");
    let tool_key = validate_tool_key(tool_key)?.to_ascii_lowercase();
    let display_key: String = tool_key
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .take(24)
        .collect();

    // FNV-1a keeps names deterministic across app restarts without adding a
    // cryptographic dependency. The hash is an identifier, not a trust token.
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in path
        .as_bytes()
        .iter()
        .copied()
        .chain(std::iter::once(0))
        .chain(tool_key.as_bytes().iter().copied())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    Ok(format!("sessionatlas-{display_key}-{hash:016x}"))
}

fn resolve_remote_tool_launch(
    tool_key: Option<&str>,
    session_id: Option<&str>,
) -> Result<(Option<String>, Option<Vec<String>>), String> {
    let requested_tool = tool_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let launch_argv = match requested_tool.as_deref() {
        Some(value) if !value.eq_ignore_ascii_case("shell") => {
            Some(resolve_configured_tool_launch_argv(value, session_id)?)
        }
        _ => {
            if session_id.is_some_and(|value| !value.trim().is_empty()) {
                return Err("session id requires a tool key".to_string());
            }
            None
        }
    };
    Ok((requested_tool, launch_argv))
}

fn remote_tmux_startup_command(launch_argv: Option<&[String]>) -> Result<String, String> {
    match launch_argv {
        Some(arguments) => {
            let launch_command = render_shell_command(arguments)?;
            let login_script = format!("{launch_command}; exec \"$SHELL\" -l");
            Ok(format!(
                "exec \"$SHELL\" -lc {}",
                shell_quote(&login_script)
            ))
        }
        None => Ok("exec \"$SHELL\" -l".to_string()),
    }
}

fn escape_tmux_formats(value: &str) -> String {
    value.replace('#', "##")
}

/// Quote one argument for tmux's command parser. These strings are typed into
/// the tmux command prompt, so POSIX shell quoting is not the correct grammar.
fn tmux_quote_argument(value: &str) -> Result<String, String> {
    if value.chars().any(char::is_control) {
        return Err("tmux argument contains control characters".to_string());
    }
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\\' | '"' | '$' => {
                quoted.push('\\');
                quoted.push(character);
            }
            // tmux formats treat #{...}, #(...), and short #X forms as
            // expansion directives. Doubling keeps a caller-controlled #
            // literal through the later format-expansion phase.
            '#' => quoted.push_str("##"),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    Ok(quoted)
}

fn build_remote_tmux_prompt_commands(
    path: &str,
    tool_key: Option<&str>,
    launch_argv: Option<&[String]>,
) -> Result<(String, String), String> {
    // Validate paths with the same boundary as initial SSH launch before
    // quoting for tmux's own command language.
    let normalized_path = path.trim();
    quote_remote_path(normalized_path)?;
    let session_name = remote_tmux_session_name(normalized_path, tool_key)?;
    let safe_path = tmux_quote_argument(normalized_path)?;
    let startup_command = remote_tmux_startup_command(launch_argv)?;
    let safe_startup_command = tmux_quote_argument(&startup_command)?;
    Ok((
        format!("new-session -d -s {session_name} -c {safe_path} {safe_startup_command}"),
        format!("switch-client -t {session_name}"),
    ))
}

fn write_tmux_prompt_command(
    writer: &mut (dyn std::io::Write + Send),
    command: &str,
) -> Result<(), String> {
    validate_pty_input(command)?;
    if command.chars().any(char::is_control) {
        return Err("tmux command contains control characters".to_string());
    }
    // Use the isolated socket's fixed C-b prefix, open the command prompt, and
    // type slowly enough that remote PTYs do not classify the control sequence
    // as pasted text. Tests skip delays but still assert the exact byte stream.
    writer.write_all(b"\x02:").map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())?;
    #[cfg(not(test))]
    std::thread::sleep(std::time::Duration::from_millis(25));
    for byte in command.bytes() {
        writer.write_all(&[byte]).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
        #[cfg(not(test))]
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
    writer.write_all(b"\r").map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())?;
    #[cfg(not(test))]
    std::thread::sleep(std::time::Duration::from_millis(75));
    Ok(())
}

fn ensure_remote_server_matches(
    actual_server_id: Option<i64>,
    requested_server_id: i64,
) -> Result<(), String> {
    if requested_server_id <= 0 {
        return Err("remote server id must be positive".to_string());
    }
    match actual_server_id {
        Some(actual) if actual == requested_server_id => Ok(()),
        Some(_) => Err("PTY belongs to a different remote server".to_string()),
        None => Err("PTY is not a remote server connection".to_string()),
    }
}

fn build_remote_tmux_command(
    path: &str,
    tool_key: Option<&str>,
    launch_argv: Option<&[String]>,
) -> Result<String, String> {
    let normalized_path = path.trim();
    quote_remote_path(normalized_path)?;
    let safe_path = quote_remote_path(&escape_tmux_formats(normalized_path))?;
    let session_name = remote_tmux_session_name(normalized_path, tool_key)?;
    let safe_session_name = shell_quote(&session_name);
    let startup_command = remote_tmux_startup_command(launch_argv)?;
    let safe_startup_command = shell_quote(&escape_tmux_formats(&startup_command));
    let safe_missing_message = shell_quote(REMOTE_TMUX_MISSING_MESSAGE);
    let safe_socket = shell_quote(REMOTE_TMUX_SOCKET);

    Ok(format!(
        "if ! command -v tmux >/dev/null 2>&1; then \
         printf '%s\\n' {safe_missing_message} >&2; exit 127; fi; \
         if ! tmux -L {safe_socket} has-session -t {safe_session_name} 2>/dev/null; then \
         tmux -L {safe_socket} -f /dev/null new-session -d -s {safe_session_name} -c {safe_path} {safe_startup_command} 2>/dev/null || true; \
         fi; \
         tmux -L {safe_socket} set-option -g prefix C-b; \
         tmux -L {safe_socket} set-option -g prefix2 None; \
         tmux -L {safe_socket} set-option -g assume-paste-time 0; \
         exec tmux -L {safe_socket} -f /dev/null attach-session -t {safe_session_name}"
    ))
}

/// Spawn an interactive shell in `path` and stream its output.
/// Returns the session id the frontend uses to address the tab.
///
/// `source` is "local" (default) or "remote". When "remote", `path` is
/// a directory path on the remote machine and `remote` carries the
/// connection and tool details. We shell out to `ssh` and create or attach a
/// persistent tmux session in that directory. `BatchMode=yes` ensures the SSH
/// call fails fast (no interactive password prompt) when keys/auth are wrong.
#[tauri::command]
fn pty_spawn(
    path: String,
    cols: u16,
    rows: u16,
    state: tauri::State<PtyRegistry>,
    source: Option<String>,
    remote: Option<RemotePtyOpts>,
    claude_print: Option<String>,
) -> Result<u32, String> {
    let is_remote = source.as_deref() == Some("remote");
    if is_remote && claude_print.is_some() {
        return Err("remote queued prompts are not supported".to_string());
    }
    if !is_remote && !std::path::Path::new(&path).is_dir() {
        return Err(format!("directory not found: {path}"));
    }
    let pty_system = native_pty_system();
    let (cols, rows) = normalize_pty_size(cols, rows);
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;

    let mut remote_server_id = None;
    let cmd = if let Some(prompt) = claude_print {
        // Queue/headless mode: run a single Claude Code prompt to
        // completion, then exit. `-p`/`--print` is non-interactive so
        // the process terminates on its own — the frontend watches
        // `pty-exit` to know the task is done and advances the queue.
        // Keep Claude's normal permission policy. Queue mode is unattended,
        // so prompts that require approval must fail or stop instead of
        // silently receiving unrestricted tool access.
        let tool_args = claude_print_argv(&prompt);
        let mut cmd = CommandBuilder::new(&tool_args[0]);
        for arg in &tool_args[1..] {
            cmd.arg(arg);
        }
        if is_remote {
            // Wrap the local command line for remote execution. We keep
            // it simple: ssh -tt ... -- <shelled command>. This is only
            // reachable if the frontend explicitly asks for a remote
            // queue run, which today it doesn't.
            let r = remote.ok_or_else(|| "remote pty opts missing".to_string())?;
            let port = r.port.unwrap_or(22);
            let safe_path = quote_remote_path(&path)?;
            let command = format!("claude -p {}", shell_quote(&prompt));
            let shelled = format!("cd {safe_path} && {command}");
            let mut ssh = CommandBuilder::new("ssh");
            ssh.arg("-tt");
            ssh.arg("-o");
            ssh.arg("BatchMode=yes");
            ssh.arg("-o");
            ssh.arg("ConnectTimeout=5");
            ssh.arg("-o");
            ssh.arg("StrictHostKeyChecking=accept-new");
            if port != 22 {
                ssh.arg("-p");
                ssh.arg(port.to_string());
            }
            if let Some(idfile) = r.identity_file.as_deref() {
                if !idfile.is_empty() {
                    ssh.arg("-i");
                    ssh.arg(idfile);
                }
            }
            ssh.arg(format!("{}@{}", r.user, r.host));
            ssh.arg("--");
            ssh.arg(shelled);
            ssh
        } else {
            cmd.cwd(&path);
            cmd
        }
    } else if is_remote {
        // Remote: attach to a stable per-project/per-tool tmux session. A new
        // session starts the selected tool once; reconnects attach without
        // injecting a second launch command into the running TUI.
        let r = remote.ok_or_else(|| "remote pty opts missing".to_string())?;
        if r.server_id <= 0 {
            return Err("remote server id must be positive".to_string());
        }
        remote_server_id = Some(r.server_id);
        let port = r.port.unwrap_or(22);
        if port == 0 {
            return Err("SSH port must be between 1 and 65535".to_string());
        }
        let destination = ssh_destination(&r.user, &r.host)?;
        let identity_file = normalize_identity_file(r.identity_file.as_deref())?;
        let (requested_tool, launch_argv) =
            resolve_remote_tool_launch(r.tool_key.as_deref(), r.session_id.as_deref())?;
        let shell_cmd =
            build_remote_tmux_command(&path, requested_tool.as_deref(), launch_argv.as_deref())?;
        let mut ssh = CommandBuilder::new("ssh");
        configure_remote_pty_environment(&mut ssh);
        ssh.arg("-tt");
        ssh.arg("-o");
        ssh.arg("BatchMode=yes");
        ssh.arg("-o");
        ssh.arg("ConnectTimeout=5");
        ssh.arg("-o");
        ssh.arg("StrictHostKeyChecking=accept-new");
        if port != 22 {
            ssh.arg("-p");
            ssh.arg(port.to_string());
        }
        if let Some(idfile) = identity_file {
            ssh.arg("-i");
            ssh.arg(idfile);
        }
        ssh.arg("--");
        ssh.arg(destination);
        ssh.arg(shell_cmd);
        ssh
    } else {
        local_shell_command(&path)
    };

    // Acquire all fallible master handles before the child exists. If either
    // operation fails, no spawned process is left behind.
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    // Drop the slave so EOF propagates when the child exits.
    drop(pair.slave);

    let session = PtySession {
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        child: Mutex::new(child),
        reader: Mutex::new(Some(reader)),
        remote_server_id,
    };
    match state.sessions.insert(session) {
        Ok(id) => Ok(id),
        Err((error, session)) => {
            let mut child = session
                .child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = child.kill();
            let _ = child.wait();
            Err(error)
        }
    }
}

/// Attach the frontend after its tab and event listeners exist. PTY output is
/// intentionally not consumed before this handshake, so the shell's first
/// prompt cannot race ahead of the tab registration. The optional initial
/// input launches the selected AI tool exactly once.
#[tauri::command]
fn pty_attach(
    id: u32,
    tool_key: Option<String>,
    session_id: Option<String>,
    state: tauri::State<PtyRegistry>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let initial_input = match tool_key.as_deref() {
        Some(tool_key) => Some(build_argv_launch_input(
            &resolve_configured_tool_launch_argv(tool_key, session_id.as_deref())?,
        )?),
        None if session_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()) =>
        {
            return Err("session id requires a tool key".to_string());
        }
        None => None,
    };
    if let Some(input) = initial_input.as_deref() {
        validate_pty_input(input)?;
    }

    let session = state
        .sessions
        .get(id)?
        .ok_or_else(|| "session not found".to_string())?;

    let reader_result = take_once(&session.reader);
    let mut reader = match reader_result {
        Ok(Some(reader)) => reader,
        // A repeated bridge call after a successful attach must not launch
        // the tool twice or tear down the already-running session.
        Ok(None) => return Ok(()),
        Err(error) => {
            finish_pty_session(&state.sessions, id, true);
            return Err(error);
        }
    };

    let attach_result = (|| {
        if let Some(input) = initial_input.as_deref() {
            let mut writer = session
                .writer
                .lock()
                .map_err(|_| "PTY writer lock poisoned".to_string())?;
            writer
                .write_all(input.as_bytes())
                .map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())?;
        }
        Ok::<_, String>(())
    })();

    if let Err(error) = attach_result {
        finish_pty_session(&state.sessions, id, true);
        return Err(error);
    }
    let sessions = state.sessions.clone();
    let reader_thread = std::thread::Builder::new()
        .name(format!("pty-reader-{id}"))
        .spawn(move || {
            let mut buf = [0u8; 8192];
            let mut decoder = Utf8StreamDecoder::new();
            let mut read_error = false;

            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = decoder.push(&buf[..n]);
                        if !chunk.is_empty() {
                            let _ = app.emit("pty-data", PtyData { id, data: chunk });
                        }
                    }
                    Err(_) => {
                        read_error = true;
                        break;
                    }
                }
            }

            let final_chunk = decoder.finish();
            if !final_chunk.is_empty() {
                let _ = app.emit(
                    "pty-data",
                    PtyData {
                        id,
                        data: final_chunk,
                    },
                );
            }
            drop(reader);

            let exit_code = finish_pty_session(&sessions, id, read_error);
            let _ = app.emit(
                "pty-exit",
                PtyExit {
                    id,
                    exit_code,
                    read_error,
                },
            );
        });

    if let Err(error) = reader_thread {
        finish_pty_session(&state.sessions, id, true);
        return Err(format!("failed to start PTY reader: {error}"));
    }

    Ok(())
}

#[tauri::command]
fn pty_write(id: u32, data: String, state: tauri::State<PtyRegistry>) -> Result<(), String> {
    validate_pty_input(&data)?;
    let session = state
        .sessions
        .get(id)?
        .ok_or_else(|| "session not found".to_string())?;
    let mut writer = session
        .writer
        .lock()
        .map_err(|_| "PTY writer lock poisoned".to_string())?;
    writer
        .write_all(data.as_bytes())
        .map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())?;
    Ok(())
}

/// Reuse an existing SSH PTY and switch its attached tmux client to another
/// deterministic project/tool session. This never starts another SSH process.
#[tauri::command]
fn pty_remote_switch(
    id: u32,
    path: String,
    server_id: i64,
    tool_key: String,
    session_id: Option<String>,
    state: tauri::State<PtyRegistry>,
) -> Result<(), String> {
    let session = state
        .sessions
        .get(id)?
        .ok_or_else(|| "session not found".to_string())?;
    ensure_remote_server_matches(session.remote_server_id, server_id)?;

    let (requested_tool, launch_argv) =
        resolve_remote_tool_launch(Some(&tool_key), session_id.as_deref())?;
    let (create_command, switch_command) = build_remote_tmux_prompt_commands(
        &path,
        requested_tool.as_deref(),
        launch_argv.as_deref(),
    )?;

    let mut writer = session
        .writer
        .lock()
        .map_err(|_| "PTY writer lock poisoned".to_string())?;
    // `new-session` is expected to report "duplicate session" when the target
    // already exists. The independent second prompt must still switch to it.
    write_tmux_prompt_command(writer.as_mut(), &create_command)?;
    write_tmux_prompt_command(writer.as_mut(), &switch_command)?;
    Ok(())
}

#[tauri::command]
fn pty_resize(
    id: u32,
    cols: u16,
    rows: u16,
    state: tauri::State<PtyRegistry>,
) -> Result<(), String> {
    let session = state
        .sessions
        .get(id)?
        .ok_or_else(|| "session not found".to_string())?;
    let master = session
        .master
        .lock()
        .map_err(|_| "PTY master lock poisoned".to_string())?;
    let (cols, rows) = normalize_pty_size(cols, rows);
    master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn pty_kill(id: u32, state: tauri::State<PtyRegistry>) -> Result<(), String> {
    finish_pty_session(&state.sessions, id, true);
    Ok(())
}

/// Fire a native OS notification. Used by the Claude task queue to
/// alert the user when a prompt finishes (and again when the whole
/// queue drains). Wraps the notification plugin's Rust API so the
/// frontend doesn't need the plugin's JS bundle — a plain `invoke`
/// call is enough.
#[tauri::command]
fn notify(app: tauri::AppHandle, title: String, body: Option<String>) -> Result<(), String> {
    let mut b = app.notification().builder().title(title);
    if let Some(body) = body {
        b = b.body(body);
    }
    b.show().map_err(|e| e.to_string())
}

/* ============================================================
Project groups. `project_groups` lists user-defined buckets;
`project_group_assignments` maps each project id to at most
one group. A project with no row in the assignments table
is implicitly in the special "未分组" (Ungrouped) section,
rendered last in the ledger. Groups are owned by the user
(no `sessionatlas` involvement) and live in `prefs.db` alongside
opener preferences.
============================================================ */

#[derive(Serialize, Clone, Debug)]
pub struct ProjectGroup {
    #[serde(rename = "id")]
    id: i64,
    #[serde(rename = "name")]
    name: String,
    #[serde(rename = "sortOrder")]
    sort_order: i64,
    #[serde(rename = "memberCount")]
    member_count: i64,
}

/// A single manual-sort row: which project, which group key, what order.
#[derive(Serialize, Clone, Debug)]
pub struct SortOrder {
    #[serde(rename = "projectId")]
    project_id: String,
    #[serde(rename = "groupKey")]
    group_key: String,
    #[serde(rename = "sortOrder")]
    sort_order: i64,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct GroupMoveResult {
    revision: i64,
    #[serde(rename = "orderedIds")]
    ordered_ids: Vec<String>,
}

fn row_to_group(r: &rusqlite::Row) -> rusqlite::Result<ProjectGroup> {
    Ok(ProjectGroup {
        id: r.get(0)?,
        name: r.get::<_, String>(1)?,
        sort_order: r.get(2)?,
        member_count: r.get(3)?,
    })
}

#[tauri::command]
fn list_groups() -> Result<Vec<ProjectGroup>, String> {
    with_prefs(|c| {
        let mut stmt = c.prepare(
            "SELECT g.id, g.name, g.sort_order,
                    (SELECT COUNT(*) FROM project_group_assignments a WHERE a.group_id = g.id) AS member_count
             FROM project_groups g
             ORDER BY g.sort_order ASC, g.id ASC"
        ).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], row_to_group)
            .map_err(|e| e.to_string())?;
        collect_query_rows(rows, "list_groups")
    })
}

fn find_existing_group(c: &Transaction<'_>, name: &str) -> Result<Option<ProjectGroup>, String> {
    match c.query_row(
        "SELECT id, name, sort_order,
                (SELECT COUNT(*) FROM project_group_assignments a WHERE a.group_id = project_groups.id) AS member_count
         FROM project_groups WHERE name = ?1",
        params![name],
        row_to_group,
    ) {
        Ok(group) => Ok(Some(group)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(format!("create_group lookup failed: {error}")),
    }
}

fn bump_group_revision(tx: &Transaction<'_>) -> Result<(), String> {
    tx.execute(
        "UPDATE prefs_revisions SET revision = revision + 1 WHERE scope = 'groups'",
        [],
    )
    .map_err(|e| format!("bump groups revision: {e}"))?;
    Ok(())
}

fn current_group_revision(tx: &Transaction<'_>) -> Result<i64, String> {
    tx.query_row(
        "SELECT revision FROM prefs_revisions WHERE scope = 'groups'",
        [],
        |row| row.get(0),
    )
    .map_err(|e| format!("read groups revision: {e}"))
}

fn move_group_project_tx(
    tx: &Transaction<'_>,
    project_id: &str,
    target_group_key: &str,
    anchor_project_id: &str,
    placement: &str,
    catalog_ids: &[String],
    expected_revision: i64,
) -> Result<GroupMoveResult, String> {
    let revision = current_group_revision(tx)?;
    if revision != expected_revision {
        return Err(format!(
            "group revision conflict: expected {expected_revision}, current {revision}"
        ));
    }
    let target_group_id = validate_group_key(tx, target_group_key)?;
    if project_id.is_empty() || anchor_project_id.is_empty() || project_id == anchor_project_id {
        return Err("project and anchor must be distinct non-empty ids".to_string());
    }
    if placement != "before" && placement != "after" {
        return Err(format!("invalid placement: {placement}"));
    }
    let mut catalog_seen = std::collections::HashSet::with_capacity(catalog_ids.len());
    if catalog_ids
        .iter()
        .any(|id| id.is_empty() || !catalog_seen.insert(id.as_str()))
    {
        return Err("catalog_ids must contain unique non-empty project ids".to_string());
    }
    if !catalog_seen.contains(project_id) || !catalog_seen.contains(anchor_project_id) {
        return Err("project and anchor must both exist in the complete catalog".to_string());
    }

    let mut assignment_stmt = tx
        .prepare("SELECT project_id, group_id FROM project_group_assignments")
        .map_err(|e| format!("load group assignments: {e}"))?;
    let assignment_rows = assignment_stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|e| format!("load group assignments: {e}"))?;
    let assignments: std::collections::HashMap<String, i64> =
        collect_query_rows(assignment_rows, "load group assignments")?
            .into_iter()
            .collect();

    let is_target = |id: &str| match target_group_id {
        Some(group_id) => assignments.get(id).copied() == Some(group_id),
        None => !assignments.contains_key(id),
    };
    if !is_target(anchor_project_id) {
        return Err(format!(
            "anchor project is not in target group: {anchor_project_id}"
        ));
    }

    let mut sort_stmt = tx
        .prepare("SELECT project_id, sort_order FROM project_sort WHERE group_key = ?1")
        .map_err(|e| format!("load target sort order: {e}"))?;
    let sort_rows = sort_stmt
        .query_map(params![target_group_key], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|e| format!("load target sort order: {e}"))?;
    let sort_orders: std::collections::HashMap<String, i64> =
        collect_query_rows(sort_rows, "load target sort order")?
            .into_iter()
            .collect();
    let catalog_positions: std::collections::HashMap<&str, usize> = catalog_ids
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect();
    let mut ordered_ids: Vec<String> = catalog_ids
        .iter()
        .filter(|id| id.as_str() != project_id && is_target(id))
        .cloned()
        .collect();
    ordered_ids.sort_by_key(|id| {
        (
            sort_orders.get(id).copied().unwrap_or(i64::MAX),
            catalog_positions
                .get(id.as_str())
                .copied()
                .unwrap_or(usize::MAX),
        )
    });
    let anchor_index = ordered_ids
        .iter()
        .position(|id| id == anchor_project_id)
        .ok_or_else(|| format!("anchor project is not active: {anchor_project_id}"))?;
    let insert_index = if placement == "after" {
        anchor_index + 1
    } else {
        anchor_index
    };
    ordered_ids.insert(insert_index, project_id.to_string());

    assign_project_to_group_tx(tx, project_id, target_group_id)?;
    for (index, id) in ordered_ids.iter().enumerate() {
        tx.execute(
            "INSERT INTO project_sort (project_id, group_key, sort_order)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(project_id) DO UPDATE SET group_key = excluded.group_key, sort_order = excluded.sort_order",
            params![id, target_group_key, (index as i64 + 1) * 10],
        )
        .map_err(|e| format!("write target sort order: {e}"))?;
    }
    bump_group_revision(tx)?;
    Ok(GroupMoveResult {
        revision: revision + 1,
        ordered_ids,
    })
}

fn validate_group_key(tx: &Transaction<'_>, group_key: &str) -> Result<Option<i64>, String> {
    if group_key == "ungrouped" {
        return Ok(None);
    }
    if group_key.is_empty()
        || group_key.starts_with('0')
        || !group_key
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return Err(format!("invalid group_key: {group_key}"));
    }
    let group_id: i64 = group_key
        .parse()
        .map_err(|_| format!("invalid group_key: {group_key}"))?;
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM project_groups WHERE id = ?1)",
            params![group_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|e| format!("validate group_key: {e}"))?
        != 0;
    if !exists {
        return Err(format!("group not found: {group_id}"));
    }
    Ok(Some(group_id))
}

fn rename_group_tx(tx: &Transaction<'_>, group_id: i64, name: &str) -> Result<bool, String> {
    let changed = tx
        .execute(
            "UPDATE project_groups SET name = ?1 WHERE id = ?2",
            params![name, group_id],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err(format!("group not found: {group_id}"));
    }
    Ok(true)
}

fn delete_group_tx(tx: &Transaction<'_>, group_id: i64) -> Result<(), String> {
    let changed = tx
        .execute(
            "DELETE FROM project_groups WHERE id = ?1",
            params![group_id],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err(format!("group not found: {group_id}"));
    }
    tx.execute(
        "DELETE FROM project_sort WHERE group_key = ?1",
        params![group_id.to_string()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn assign_project_to_group_tx(
    tx: &Transaction<'_>,
    project_id: &str,
    group_id: Option<i64>,
) -> Result<(), String> {
    if let Some(group_id) = group_id {
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM project_groups WHERE id = ?1)",
                params![group_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())?
            != 0;
        if !exists {
            return Err(format!("group not found: {group_id}"));
        }
        tx.execute(
            "INSERT INTO project_group_assignments (project_id, group_id)
             VALUES (?1, ?2)
             ON CONFLICT(project_id) DO UPDATE SET group_id = excluded.group_id",
            params![project_id, group_id],
        )
        .map_err(|e| e.to_string())?;
        let key = group_id.to_string();
        let manual: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM project_sort WHERE group_key = ?1)",
                params![key],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())?
            != 0;
        if manual {
            let next: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(sort_order),0)+10 FROM project_sort WHERE group_key = ?1",
                    params![key],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO project_sort (project_id, group_key, sort_order)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(project_id) DO UPDATE SET group_key = excluded.group_key, sort_order = excluded.sort_order",
                params![project_id, key, next],
            )
            .map_err(|e| e.to_string())?;
        } else {
            tx.execute(
                "DELETE FROM project_sort WHERE project_id = ?1",
                params![project_id],
            )
            .map_err(|e| e.to_string())?;
        }
    } else {
        tx.execute(
            "DELETE FROM project_group_assignments WHERE project_id = ?1",
            params![project_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM project_sort WHERE project_id = ?1",
            params![project_id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn set_group_order_tx(
    tx: &Transaction<'_>,
    group_key: &str,
    ordered_ids: &[String],
) -> Result<(), String> {
    let group_id = validate_group_key(tx, group_key)?;
    let mut seen = std::collections::HashSet::with_capacity(ordered_ids.len());
    if ordered_ids
        .iter()
        .any(|id| id.is_empty() || !seen.insert(id))
    {
        return Err("ordered_ids must contain unique non-empty project ids".to_string());
    }
    if ordered_ids.is_empty() {
        return Ok(());
    }
    // The caller must submit a complete sequence for every member that is
    // already in the target bucket. New ids are allowed so a cross-group
    // positional move can append the dragged project atomically.
    let existing_sql = if group_id.is_some() {
        "SELECT project_id FROM project_group_assignments WHERE group_id = ?1 ORDER BY project_id"
    } else {
        "SELECT project_id FROM project_sort WHERE group_key = ?1 ORDER BY project_id"
    };
    let existing_param = group_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| group_key.to_string());
    let mut existing_stmt = tx
        .prepare(existing_sql)
        .map_err(|e| format!("load existing group order: {e}"))?;
    let rows = existing_stmt
        .query_map(params![existing_param], |row| row.get::<_, String>(0))
        .map_err(|e| format!("load existing group order: {e}"))?;
    let existing = collect_query_rows(rows, "load existing group order")?;
    let submitted: std::collections::HashSet<&str> =
        ordered_ids.iter().map(String::as_str).collect();
    if let Some(missing) = existing.iter().find(|id| !submitted.contains(id.as_str())) {
        return Err(format!(
            "ordered_ids is incomplete; missing existing project {missing}"
        ));
    }
    let placeholders = std::iter::repeat_n("?", ordered_ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let del_sql = format!("DELETE FROM project_sort WHERE project_id IN ({placeholders})");
    let del_params: Vec<&dyn rusqlite::ToSql> = ordered_ids
        .iter()
        .map(|id| id as &dyn rusqlite::ToSql)
        .collect();
    tx.execute(&del_sql, del_params.as_slice())
        .map_err(|e| e.to_string())?;
    for (index, id) in ordered_ids.iter().enumerate() {
        tx.execute(
            "INSERT INTO project_sort (project_id, group_key, sort_order) VALUES (?1, ?2, ?3)",
            params![id, group_key, (index as i64 + 1) * 10],
        )
        .map_err(|e| e.to_string())?;
    }
    if group_id.is_some() {
        for id in ordered_ids {
            tx.execute(
                "INSERT INTO project_group_assignments (project_id, group_id)
                 VALUES (?1, ?2)
                 ON CONFLICT(project_id) DO UPDATE SET group_id = excluded.group_id",
                params![id, group_id],
            )
            .map_err(|e| e.to_string())?;
        }
    } else {
        let sql =
            format!("DELETE FROM project_group_assignments WHERE project_id IN ({placeholders})");
        tx.execute(&sql, del_params.as_slice())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn create_group(name: String) -> Result<ProjectGroup, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("group name cannot be empty".into());
    }
    with_prefs_transaction(|tx| {
        // If the name already exists, return the existing group instead of erroring.
        if let Some(existing) = find_existing_group(tx, trimmed)? {
            return Ok(existing);
        }
        let next: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(sort_order),0)+10 FROM project_groups",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO project_groups (name, sort_order) VALUES (?1, ?2)",
            params![trimmed, next],
        )
        .map_err(|e| e.to_string())?;
        let id = tx.last_insert_rowid();
        let group = tx
            .query_row(
            "SELECT id, name, sort_order,
                    (SELECT COUNT(*) FROM project_group_assignments a WHERE a.group_id = ?1) AS member_count
             FROM project_groups WHERE id = ?1",
            params![id],
            row_to_group,
        )
        .map_err(|e| e.to_string())?;
        bump_group_revision(tx)?;
        Ok(group)
    })
}

#[tauri::command]
fn rename_group(group_id: i64, name: String) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("group name cannot be empty".into());
    }
    with_prefs_transaction(|tx| {
        rename_group_tx(tx, group_id, trimmed)?;
        bump_group_revision(tx)?;
        Ok(())
    })
}

#[tauri::command]
fn delete_group(group_id: i64) -> Result<(), String> {
    with_prefs_transaction(|tx| {
        delete_group_tx(tx, group_id)?;
        bump_group_revision(tx)?;
        Ok(())
    })
}

/// Assign a project to a group. `group_id = None` clears the
/// assignment (project returns to the implicit "未分组" bucket).
///
/// Sort reconciliation: dropping a project onto a group (via the dropdown
/// or a header-drop) must stay consistent with drag reorder. If the target
/// group is already manual, the project is appended with a sort_order
/// (max+10) so it lands at the end in its new home. If the target is not
/// manual, any stale sort row is removed so the project rejoins recency
/// order and we don't accidentally lock the target group. Moving to
/// "未分组" always clears the sort row (ungrouped defaults to recency;
/// positional drops into ungrouped go through `set_group_order` instead).
#[tauri::command]
fn assign_project_to_group(project_id: String, group_id: Option<i64>) -> Result<(), String> {
    with_prefs_transaction(|tx| {
        assign_project_to_group_tx(tx, &project_id, group_id)?;
        bump_group_revision(tx)?;
        Ok(())
    })
}

/// All manual sort rows, flattened. Frontend builds a {projectId: sortOrder}
/// map and derives which groups are "manual" (>=1 member has a row).
#[tauri::command]
fn list_sort_orders() -> Result<Vec<SortOrder>, String> {
    with_prefs(|c| {
        let mut stmt = c
            .prepare("SELECT project_id, group_key, sort_order FROM project_sort")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SortOrder {
                    project_id: r.get(0)?,
                    group_key: r.get(1)?,
                    sort_order: r.get(2)?,
                })
            })
            .map_err(|e| e.to_string())?;
        collect_query_rows(rows, "list_sort_orders")
    })
}

#[tauri::command]
fn get_group_revision() -> Result<i64, String> {
    with_prefs(|connection| {
        connection
            .query_row(
                "SELECT revision FROM prefs_revisions WHERE scope = 'groups'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("read groups revision: {e}"))
    })
}

#[tauri::command]
fn move_group_project(
    project_id: String,
    target_group_key: String,
    anchor_project_id: String,
    placement: String,
    catalog_ids: Vec<String>,
    expected_revision: i64,
) -> Result<GroupMoveResult, String> {
    with_prefs_transaction(|tx| {
        move_group_project_tx(
            tx,
            &project_id,
            &target_group_key,
            &anchor_project_id,
            &placement,
            &catalog_ids,
            expected_revision,
        )
    })
}

/// The single drag command: rewrite the manual order for `group_key` to
/// `ordered_ids` (index*10 spacing) and reconcile group assignments so a
/// cross-group positional move updates assignment in the same transaction.
/// Locks the target group (every listed member gets a row). Used for both
/// in-group reorder and cross-group positional drops.
#[tauri::command]
fn set_group_order(group_key: String, ordered_ids: Vec<String>) -> Result<(), String> {
    with_prefs_transaction(|tx| {
        set_group_order_tx(tx, &group_key, &ordered_ids)?;
        if !ordered_ids.is_empty() {
            bump_group_revision(tx)?;
        }
        Ok(())
    })
}

/// Return all `project_id → group_id` assignments as a flat map.
#[tauri::command]
fn list_group_assignments() -> Result<std::collections::HashMap<String, i64>, String> {
    with_prefs(|c| {
        let mut stmt = c
            .prepare("SELECT project_id, group_id FROM project_group_assignments")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(|e| e.to_string())?;
        let mut map = std::collections::HashMap::new();
        for row in collect_query_rows(rows, "list_group_assignments")? {
            map.insert(row.0, row.1);
        }
        Ok(map)
    })
}

/* ============================================================
Project documentation.
The frontend can list a project's markdown files and read their
contents so users can preview README/CHANGELOG/docs/ inline without
leaving the app. We deliberately do NOT walk recursively — only the
project root and the conventional doc subdirs (docs/, doc/,
documentation/) — and we cap the read size to keep the webview
responsive on accidentally-huge files.
============================================================ */

/// Max bytes the frontend will fetch for a single doc read.
const MAX_DOC_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Serialize, Clone, Debug)]
pub struct DocEntry {
    #[serde(rename = "name")]
    name: String,
    #[serde(rename = "relPath")]
    rel_path: String,
    #[serde(rename = "size")]
    size: u64,
}

// ============================================================
// Remote SSH servers (settings drawer + PTY path)
// ============================================================
// The frontend's settings drawer lets the user register a remote
// machine; we shell out to `ssh` from Rust to discover its git
// projects and to spawn PTY sessions. No new crate dependency —
// `ssh` is assumed to be present on both the local and remote side
// (the local side is whatever machine runs this Tauri app; the
// remote side is the box the user just configured). All remote
// project metadata and tool-usage rows live in `prefs.db` so the
// local `index.db` (which `sessionatlas scan` owns) stays untouched.

/// `pty_spawn` opts for a remote session. Mirrors what the frontend
/// keeps in `state.remoteServerById[project.remoteServerId]`.
#[derive(Deserialize, Debug)]
struct RemotePtyOpts {
    #[serde(rename = "serverId")]
    server_id: i64,
    user: String,
    host: String,
    port: Option<u16>,
    #[serde(rename = "identityFile")]
    identity_file: Option<String>,
    #[serde(rename = "toolKey")]
    tool_key: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
}

#[derive(Serialize)]
struct RemoteServerRow {
    id: i64,
    label: String,
    user: String,
    host: String,
    port: i64,
    identity_file: Option<String>,
    scan_roots: String,
    created_at: String,
}

fn row_to_remote_server(r: &rusqlite::Row<'_>) -> rusqlite::Result<RemoteServerRow> {
    Ok(RemoteServerRow {
        id: r.get(0)?,
        label: r.get(1)?,
        user: r.get(2)?,
        host: r.get(3)?,
        port: r.get(4)?,
        identity_file: r.get(5)?,
        scan_roots: r.get(6)?,
        created_at: r.get(7)?,
    })
}

/// Render a `RemoteServerRow` into the JSON shape the frontend
/// consumes (camelCase, mirrors the `RemoteServer` Rust struct).
fn remote_server_to_json(row: RemoteServerRow) -> RemoteServer {
    RemoteServer {
        id: row.id,
        label: row.label,
        user: row.user,
        host: row.host,
        port: row.port,
        identity_file: row.identity_file,
        scan_roots: row.scan_roots,
        created_at: row.created_at,
    }
}

#[tauri::command]
fn list_remote_servers() -> Result<Vec<RemoteServer>, String> {
    with_prefs(|c| {
        let mut stmt = c
            .prepare(
                "SELECT id, label, user, host, port, identity_file, scan_roots, created_at
                 FROM remote_servers ORDER BY id ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows: Vec<RemoteServerRow> = collect_query_rows(
            stmt.query_map([], row_to_remote_server)
                .map_err(|e| e.to_string())?,
            "list_remote_servers",
        )?;
        Ok(rows.into_iter().map(remote_server_to_json).collect())
    })
}

fn normalize_identity_file(value: Option<&str>) -> Result<Option<std::path::PathBuf>, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.chars().any(char::is_control) {
        return Err("identity file contains a control character".to_string());
    }

    let path = if value == "~" {
        dirs::home_dir().ok_or_else(|| "cannot resolve the local home directory".to_string())?
    } else if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        dirs::home_dir()
            .ok_or_else(|| "cannot resolve the local home directory".to_string())?
            .join(rest)
    } else {
        std::path::PathBuf::from(value)
    };
    if !path.is_absolute() {
        return Err("identity file must be an absolute path or start with ~/".to_string());
    }
    if !path.is_file() {
        return Err("identity file does not exist or is not a regular file".to_string());
    }
    path.canonicalize()
        .map(Some)
        .map_err(|error| format!("cannot resolve identity file: {error}"))
}

/// Insert a validated remote server and return the persisted row.
#[tauri::command]
fn add_remote_server(
    label: String,
    user: String,
    host: String,
    port: Option<u16>,
    identity_file: Option<String>,
    scan_roots: Option<String>,
) -> Result<RemoteServer, String> {
    let label = validate_display_label(&label)?;
    let user = validate_ssh_user(&user)?;
    let host = validate_ssh_host(&host)?;
    let port = port.unwrap_or(22) as i64;
    if !(1..=65535).contains(&port) {
        return Err(format!("port out of range: {port}"));
    }
    let identity_file = normalize_identity_file(identity_file.as_deref())?
        .map(|path| path.to_string_lossy().into_owned());
    let scan_roots = match scan_roots {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => DEFAULT_REMOTE_SCAN_ROOTS.to_string(),
    };
    shell_quote_roots(&scan_roots)?;
    with_prefs(|c| {
        c.execute(
            "INSERT INTO remote_servers (label, user, host, port, identity_file, scan_roots)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![label, user, host, port, identity_file, scan_roots],
        )
        .map_err(|e| e.to_string())?;
        let id = c.last_insert_rowid();
        let mut stmt = c
            .prepare(
                "SELECT id, label, user, host, port, identity_file, scan_roots, created_at
                 FROM remote_servers WHERE id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let row = stmt
            .query_row(params![id], row_to_remote_server)
            .map_err(|e| e.to_string())?;
        Ok(remote_server_to_json(row))
    })
}

#[tauri::command]
fn delete_remote_server(server_id: i64) -> Result<(), String> {
    with_prefs(|c| {
        // ON DELETE CASCADE on remote_projects / remote_tool_usages
        // handles the children.
        let n = c
            .execute(
                "DELETE FROM remote_servers WHERE id = ?1",
                params![server_id],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err(format!("remote server {server_id} not found"));
        }
        Ok(())
    })
}

fn parse_remote_connection_probe(stdout: &str) -> RemoteConnectionProbe {
    let mut home = String::new();
    let mut tmux_available = false;
    let mut tmux_version = None;
    for line in stdout.lines() {
        if let Some(index) = line.find("SESSIONATLAS_SSH_OK:") {
            let value = &line[index + "SESSIONATLAS_SSH_OK:".len()..];
            home = value.trim().to_string();
        } else if let Some(index) = line.find("SESSIONATLAS_TMUX_OK:") {
            let value = &line[index + "SESSIONATLAS_TMUX_OK:".len()..];
            tmux_available = true;
            let value = value.trim();
            if !value.is_empty() {
                tmux_version = Some(value.to_string());
            }
        }
    }
    RemoteConnectionProbe {
        home,
        tmux_available,
        tmux_version,
    }
}

/// Connection pre-check for a remote SSH server: confirms passwordless SSH,
/// resolves `$HOME`, and reports whether tmux is available for persistent
/// remote terminals. SSH failures remain actionable bilingual messages.
///
/// Called by the frontend BEFORE `add_remote_server` so a server that can't
/// be reached / lacks passwordless auth never lands in the prefs DB as a
/// zombie record. A missing tmux does not block project indexing; the returned
/// capability lets the frontend warn before the first terminal is opened.
#[tauri::command]
fn test_remote_connection(
    user: String,
    host: String,
    port: Option<u16>,
    identity_file: Option<String>,
) -> Result<RemoteConnectionProbe, String> {
    let user = validate_ssh_user(&user)?.to_string();
    let host = validate_ssh_host(&host)?.to_string();
    let p = port.unwrap_or(22);
    if p == 0 {
        return Err("port out of range: 0".to_string());
    }
    let idfile = normalize_identity_file(identity_file.as_deref())?;

    // Keep the command successful when tmux is absent: SSH and indexing still
    // work, while the structured result tells the UI to show an install hint.
    let probe = "printf 'SESSIONATLAS_SSH_OK:%s\\n' \"$HOME\"; \
                 if command -v tmux >/dev/null 2>&1; then \
                 printf 'SESSIONATLAS_TMUX_OK:'; tmux -V 2>/dev/null; \
                 else printf 'SESSIONATLAS_TMUX_MISSING\\n'; fi";
    let cmd = build_ssh_command(
        &user,
        &host,
        p,
        idfile.as_deref().and_then(|path| path.to_str()),
        probe,
    )?;
    let out = SystemProcessRunner.output(&cmd).map_err(|e| {
        // Failed to even spawn ssh — usually means ssh isn't installed.
        format!(
            "Could not start ssh.\n\
             Is OpenSSH installed and on PATH? ({e})\n\
             无法启动 ssh。\n\
             请确认已安装 OpenSSH 并在 PATH 中。({e})"
        )
    })?;

    if !out.success {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(classify_ssh_failure(&user, &host, p, &stderr));
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(parse_remote_connection_probe(&stdout))
}

/// Mirror of the local `is_excluded_project_path` (which lives below).
/// Same home dirs, applied to the remote side so we don't surface
/// `~/.claude` / `~/.codex` etc. as project entries on the remote.
fn is_remote_path_excluded(path: &str, remote_home: &str) -> bool {
    let normalized = path.trim_end_matches('/');
    for suffix in [".claude", ".codex", ".kimi", ".opencode", ".aider"] {
        // match "<home>/.claude" or "<home>/.claude/..." (any depth).
        let exact = format!("{remote_home}/{suffix}");
        let nested = format!("{exact}/");
        if normalized == exact || normalized.starts_with(&nested) {
            return true;
        }
    }
    false
}

/// Build the ssh command for either a remote project scan or a remote
/// PTY session. Centralized so both call sites share the same flag
/// recipe (`BatchMode=yes` so a bad key fails fast without a hanging
/// password prompt; `ConnectTimeout` so unreachable hosts error
/// instead of stalling).
fn build_ssh_command(
    user: &str,
    host: &str,
    port: u16,
    identity_file: Option<&str>,
    remote_cmd: &str,
) -> Result<process::ProcessSpec, String> {
    if port == 0 {
        return Err("SSH port must be between 1 and 65535".to_string());
    }
    let destination = ssh_destination(user, host)?;
    let identity_file = normalize_identity_file(identity_file)?;
    let mut cmd = process::ProcessSpec::new("ssh")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=5")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new");
    if port != 22 {
        cmd = cmd.arg("-p").arg(port.to_string());
    }
    if let Some(idfile) = identity_file {
        cmd = cmd.arg("-i").arg(idfile.as_os_str());
    }
    Ok(cmd.arg("--").arg(destination).arg(remote_cmd))
}

/// Turn a raw ssh failure (exit code + stderr) into a friendly, bilingual,
/// actionable message. Used by the connection pre-check AND by
/// `scan_remote_server` so the user always gets guidance ("add your public
/// key to authorized_keys", "run ssh-keygen -R", "check the network") rather
/// than a bare `Permission denied (publickey).`.
///
/// Returns a single multi-line string: an EN line then a ZH line, separated
/// by a newline. The frontend `showError` renders it as-is (its body keeps
/// newlines via white-space handling), so we avoid a per-diagnosis i18n key.
fn classify_ssh_failure(user: &str, host: &str, port: u16, stderr: &str) -> String {
    let s = stderr.to_lowercase();
    let target = if port == 22 {
        format!("{user}@{host}")
    } else {
        format!("{user}@{host}:{port}")
    };

    // Host key changed / verification failed.
    if s.contains("remote host identification has changed")
        || s.contains("host key verification failed")
        || s.contains("man-in-the-middle")
    {
        return format!(
            "Host key changed for {target}.\n\
             Run `ssh-keygen -R {host}` (or :{port}) to clear the old entry, then retry.\n\
             主机密钥已变更。\n\
             请运行 `ssh-keygen -R {host}`（或加 :{port}）清除旧记录后重试。"
        );
    }

    // Auth failed — the common "passwordless not set up" case.
    if s.contains("permission denied")
        || s.contains("publickey")
        || s.contains("no supported authentication methods")
        || s.contains("authentications that can continue")
    {
        return format!(
            "Passwordless login is not configured for {target}.\n\
             Add your public key to the remote `~/.ssh/authorized_keys`, or start ssh-agent and add your key with `ssh-add`.\n\
             未配置免密登录。\n\
             请把本机公钥加入远程 `~/.ssh/authorized_keys`，或启动 ssh-agent 并用 `ssh-add` 添加密钥。"
        );
    }

    // Network / unreachable.
    if s.contains("connection refused")
        || s.contains("connection timed out")
        || s.contains("timed out")
        || s.contains("could not resolve")
        || s.contains("no route")
        || s.contains("network is unreachable")
        || s.contains("port 22")
    {
        return format!(
            "Could not reach {target}.\n\
             Check the address, port ({port}), and that the host is running sshd.\n\
             无法连接到 {target}。\n\
             请检查地址、端口（{port}）以及目标主机是否运行了 sshd。"
        );
    }

    // Fallback: include the raw stderr so it's still debuggable.
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        format!(
            "ssh to {target} failed.\n\
             无法通过 ssh 连接到 {target}。"
        )
    } else {
        format!(
            "ssh to {target} failed:\n{trimmed}\n\
             无法通过 ssh 连接到 {target}。"
        )
    }
}

/// Single-quote `s` for safe interpolation into a POSIX shell command
/// line, using the standard `'\''` escape for embedded quotes. Rejects
/// control characters outright — a legit prompt never has them, and they
/// are a classic injection vector. Used when wrapping a Claude Code
/// queue prompt into a remote `ssh ... -- <cmd>` string.
fn shell_quote(s: &str) -> String {
    // Replace each embedded single quote with the `'\''` close-reopen
    // sequence; wrap the whole thing in single quotes.
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// Shell-quote a single value for safe interpolation into a remote POSIX
/// shell command. Strips single quotes (they cannot legally appear in a
/// path and would otherwise break out of the surrounding quotes) and wraps
/// the remainder in single quotes. `~` is preserved as the first character
/// so `~/projects` keeps its home-relative meaning after the remote shell
/// expands it inside the quotes — POSIX shells still expand `~` when it
/// appears at the start of a quoted word that follows an unquoted `=`, but
/// NOT inside single quotes, so we special-case a leading `~`/`~/...` by
/// emitting it unquoted and quoting the rest. This mirrors how `pty_spawn`
/// handles `safe_path`.
fn shell_quote_path(value: &str) -> Result<String, String> {
    quote_remote_path(value)
}

/// Parse a whitespace-separated `scan_roots` string into a shell-safe
/// `for`-list body, e.g. `~'/projects' '~'/code`. Returns an error if any
/// entry is empty or contains control characters. Used by the remote scan
/// so a crafted scan_roots value can't inject commands into the remote shell.
fn shell_quote_roots(scan_roots: &str) -> Result<String, String> {
    let quoted: Vec<String> = scan_roots
        .split_whitespace()
        .map(shell_quote_path)
        .collect::<Result<_, _>>()?;
    if quoted.is_empty() {
        return Err("scan_roots has no entries".to_string());
    }
    Ok(quoted.join(" "))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RemoteScanRow {
    path: String,
    branch: Option<String>,
    last_activity_epoch: Option<i64>,
}

/// Build the POSIX command used to discover remote Git working trees.
///
/// `find` deliberately matches both `.git` directories (regular clones)
/// and `.git` files (linked worktrees). Git then supplies the canonical
/// working-tree root, so the persisted path is never the metadata marker.
/// Records are NUL-delimited so whitespace and newlines in valid paths do
/// not corrupt the protocol.
fn build_remote_scan_command(scan_roots: &str) -> Result<String, String> {
    let roots = shell_quote_roots(scan_roots)?;
    // The two nested built-in roots are optional accelerators: they extend
    // the depth available below common project directories, but many valid
    // hosts do not have one or both of them. Preserve fail-closed behavior
    // for every custom root list while skipping only absent built-in roots.
    let skip_absent_builtin_roots = if scan_roots.trim() == DEFAULT_REMOTE_SCAN_ROOTS {
        "if [ ! -e \"$r\" ]; then continue; fi; "
    } else {
        ""
    };
    Ok(format!(
        "scan_failed=0; \
         for r in {roots}; do \
             {skip_absent_builtin_roots}\
             if ! \
            find \"$r\" -maxdepth 6 -name .git -prune -exec sh -c ' \
                for marker do \
                    candidate=${{marker%/.git}}; \
                    repo=$(git -C \"$candidate\" rev-parse --show-toplevel 2>/dev/null) || continue; \
                    branch=$(git -C \"$repo\" branch --show-current 2>/dev/null); \
                    activity=$(git -C \"$repo\" log -1 --all --format=%ct 2>/dev/null); \
                    printf \"%s\\000%s\\000%s\\000\" \"$repo\" \"$branch\" \"$activity\"; \
                done \
            ' sh {{}} +; then \
                scan_failed=1; \
            fi; \
         done; \
         exit \"$scan_failed\""
    ))
}

/// Parse and de-duplicate the NUL-delimited scan protocol. Overlapping
/// roots such as `~` and `~/projects` can discover the same repository
/// repeatedly; a path-keyed BTreeMap gives one stable, deterministic row.
fn parse_remote_scan_output(
    stdout: &[u8],
    remote_home: &str,
) -> Result<Vec<RemoteScanRow>, String> {
    let mut fields: Vec<&[u8]> = stdout.split(|byte| *byte == 0).collect();
    if fields.last().is_some_and(|field| field.is_empty()) {
        fields.pop();
    }
    if !fields.len().is_multiple_of(3) {
        return Err(format!(
            "remote scan returned an incomplete record ({} fields)",
            fields.len()
        ));
    }

    let mut deduplicated: BTreeMap<String, RemoteScanRow> = BTreeMap::new();
    for record in fields.chunks_exact(3) {
        let path = std::str::from_utf8(record[0])
            .map_err(|_| "remote scan returned a non-UTF-8 project path".to_string())?
            .to_string();
        if path.is_empty() {
            return Err("remote scan returned an empty project path".to_string());
        }
        if is_remote_path_excluded(&path, remote_home) {
            continue;
        }

        let branch = std::str::from_utf8(record[1])
            .map_err(|_| format!("remote scan returned a non-UTF-8 branch for {path}"))?
            .trim()
            .to_string();
        let branch = (!branch.is_empty()).then_some(branch);
        let activity = std::str::from_utf8(record[2])
            .map_err(|_| format!("remote scan returned an invalid activity time for {path}"))?
            .trim();
        let last_activity_epoch =
            if activity.is_empty() {
                None
            } else {
                Some(activity.parse::<i64>().map_err(|_| {
                    format!("remote scan returned an invalid activity time for {path}")
                })?)
            };

        deduplicated
            .entry(path.clone())
            .and_modify(|existing| {
                if existing.branch.is_none() {
                    existing.branch.clone_from(&branch);
                }
                existing.last_activity_epoch =
                    match (existing.last_activity_epoch, last_activity_epoch) {
                        (Some(left), Some(right)) => Some(left.max(right)),
                        (left, right) => left.or(right),
                    };
            })
            .or_insert(RemoteScanRow {
                path,
                branch,
                last_activity_epoch,
            });
    }

    Ok(deduplicated.into_values().collect())
}

const MAX_REMOTE_SCAN_DIAGNOSTIC_BYTES: usize = 4096;

fn sanitize_remote_scan_diagnostic(
    stderr: &[u8],
    remote_home: &str,
    identity_file: Option<&str>,
) -> String {
    let mut diagnostic = String::from_utf8_lossy(stderr).to_string();
    if !remote_home.is_empty() {
        diagnostic = diagnostic.replace(remote_home, "$HOME");
    }
    if let Some(identity) = identity_file {
        if !identity.is_empty() {
            diagnostic = diagnostic.replace(identity, "<identity>");
        }
    }
    let diagnostic: String = diagnostic
        .chars()
        .map(|character| {
            if character == '\n' || character == '\r' || character == '\t' {
                character
            } else if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    if diagnostic.len() <= MAX_REMOTE_SCAN_DIAGNOSTIC_BYTES {
        return diagnostic;
    }
    let mut end = MAX_REMOTE_SCAN_DIAGNOSTIC_BYTES;
    while end > 0 && !diagnostic.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}[truncated]", &diagnostic[..end])
}

fn validate_remote_scan_output(
    output: &ProcessOutput,
    remote_home: &str,
    user: &str,
    host: &str,
    port: u16,
    identity_file: Option<&str>,
) -> Result<Vec<RemoteScanRow>, String> {
    if !output.success {
        let diagnostic =
            sanitize_remote_scan_diagnostic(&output.stderr, remote_home, identity_file);
        if output.status_code == Some(255) {
            return Err(classify_ssh_failure(user, host, port, &diagnostic));
        }
        let status = output
            .status_code
            .map_or_else(|| "unknown".to_string(), |code| code.to_string());
        return Err(format!(
            "remote scan command failed (status {status}); previous snapshot was preserved: {diagnostic}"
        ));
    }
    parse_remote_scan_output(&output.stdout, remote_home)
}

fn legacy_remote_project_path(path: &str) -> String {
    format!("{}/.git", path.trim_end_matches('/'))
}

/// The previous scanner persisted `<worktree>/.git`, so correcting the path
/// changes the synthetic project id. Move any recorded usage to the new id
/// and merge defensively if both forms already exist.
fn migrate_legacy_remote_tool_usages(
    c: &Connection,
    server_id: i64,
    legacy_project_id: &str,
    project_id: &str,
) -> Result<(), String> {
    if legacy_project_id == project_id {
        return Ok(());
    }
    c.execute(
        "INSERT INTO remote_tool_usages
            (server_id, project_id, tool_key, tool_name, last_used_at, session_count, last_session_id)
         SELECT server_id, ?1, tool_key, tool_name, last_used_at, session_count, last_session_id
         FROM remote_tool_usages
         WHERE server_id = ?2 AND project_id = ?3
         ON CONFLICT(server_id, project_id, tool_key) DO UPDATE SET
            tool_name = CASE
                WHEN excluded.last_used_at >= remote_tool_usages.last_used_at
                THEN excluded.tool_name
                ELSE remote_tool_usages.tool_name
            END,
            last_used_at = MAX(remote_tool_usages.last_used_at, excluded.last_used_at),
            session_count = remote_tool_usages.session_count + excluded.session_count,
            last_session_id = CASE
                WHEN excluded.last_used_at >= remote_tool_usages.last_used_at
                THEN COALESCE(excluded.last_session_id, remote_tool_usages.last_session_id)
                ELSE remote_tool_usages.last_session_id
            END",
        params![project_id, server_id, legacy_project_id],
    )
    .map_err(|e| e.to_string())?;
    c.execute(
        "DELETE FROM remote_tool_usages
         WHERE server_id = ?1 AND project_id = ?2",
        params![server_id, legacy_project_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Discover git projects on a remote server. We don't require `sessionatlas`
/// on the remote; instead we shell out to `ssh` and run a small POSIX
/// pipeline that walks the configured scan roots and emits the canonical
/// working-tree root, current branch, and latest commit timestamp.
#[tauri::command]
fn scan_remote_server(server_id: i64) -> Result<i64, String> {
    // Fetch server config.
    let server = with_prefs(|c| {
        let mut stmt = c
            .prepare(
                "SELECT id, label, user, host, port, identity_file, scan_roots, created_at
                 FROM remote_servers WHERE id = ?1",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_row(params![server_id], row_to_remote_server)
            .map_err(|e| format!("remote server {server_id} not found: {e}"))
    })?;
    if !(1..=65535).contains(&server.port) {
        return Err(format!("stored SSH port is invalid: {}", server.port));
    }
    let port = server.port as u16;
    let idfile = server.identity_file.clone();

    // Two-step ssh:
    //   1. resolve $HOME on the remote so we can apply the
    //      exclusion list against the user's actual home directory.
    //   2. run the find pipeline and emit NUL-delimited
    //      `<path><branch><latest-commit-epoch>` records.
    let home_cmd = "printf '%s' \"$HOME\"".to_string();
    let home = {
        let cmd = build_ssh_command(
            &server.user,
            &server.host,
            port,
            idfile.as_deref(),
            &home_cmd,
        )?;
        let out = SystemProcessRunner.output(&cmd).map_err(|e| {
            format!(
                "Could not start ssh.\n\
                 Is OpenSSH installed and on PATH? ({e})\n\
                 无法启动 ssh。\n\
                 请确认已安装 OpenSSH 并在 PATH 中。({e})"
            )
        })?;
        if !out.success {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(classify_ssh_failure(
                &server.user,
                &server.host,
                port,
                &stderr,
            ));
        }
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    if home.is_empty() {
        return Err("could not resolve remote $HOME".to_string());
    }

    // Quote each scan root before interpolation so a crafted scan_roots
    // value can't inject shell metacharacters into the remote command.
    let find_cmd = build_remote_scan_command(&server.scan_roots)?;
    let out = {
        let cmd = build_ssh_command(
            &server.user,
            &server.host,
            port,
            idfile.as_deref(),
            &find_cmd,
        )?;
        SystemProcessRunner.output(&cmd).map_err(|e| {
            format!(
                "Could not start ssh.\n\
                 Is OpenSSH installed and on PATH? ({e})\n\
                 无法启动 ssh。\n\
                 请确认已安装 OpenSSH 并在 PATH 中。({e})"
            )
        })?
    };
    let rows = validate_remote_scan_output(
        &out,
        &home,
        &server.user,
        &server.host,
        port,
        idfile.as_deref(),
    )?;

    // Upsert into remote_projects. Use a transaction so a partial scan
    // never leaves the table half-written. We replace the entire set
    // for this server (matches the local `scan_projects` model — each
    // scan is a snapshot, deletions happen because the dir is gone).
    with_prefs(|c| {
        let tx = c.unchecked_transaction().map_err(|e| e.to_string())?;
        let previous_access: HashMap<String, String> = {
            let mut stmt = tx
                .prepare(
                    "SELECT path, last_accessed_at
                     FROM remote_projects
                     WHERE server_id = ?1",
                )
                .map_err(|e| e.to_string())?;
            let values = stmt
                .query_map(params![server_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?;
            values
        };
        for row in &rows {
            let project_id = remote_project_id(server_id, &row.path);
            let legacy_project_id =
                remote_project_id(server_id, &legacy_remote_project_path(&row.path));
            migrate_legacy_remote_tool_usages(&tx, server_id, &legacy_project_id, &project_id)?;
        }
        let latest_tool_access: HashMap<String, String> = {
            let mut stmt = tx
                .prepare(
                    "SELECT project_id, MAX(last_used_at)
                     FROM remote_tool_usages
                     WHERE server_id = ?1
                     GROUP BY project_id",
                )
                .map_err(|e| e.to_string())?;
            let values = stmt
                .query_map(params![server_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?;
            values
        };
        tx.execute(
            "DELETE FROM remote_projects WHERE server_id = ?1",
            params![server_id],
        )
        .map_err(|e| e.to_string())?;
        let now = chrono_like_now_iso();
        let mut total: i64 = 0;
        for row in &rows {
            let path = &row.path;
            let legacy_path = legacy_remote_project_path(path);
            // project_id is synthetic: 'r<server_id>:<path>'. We hash the
            // path into a short hex suffix so the id stays well-bounded
            // for the frontend (avoids giant DOM ids in queries).
            let project_id = remote_project_id(server_id, path);
            let last_accessed_at = remote_last_accessed_at(
                row.last_activity_epoch,
                latest_tool_access.get(&project_id).map(String::as_str),
                previous_access
                    .get(path)
                    .or_else(|| previous_access.get(&legacy_path))
                    .map(String::as_str),
                &now,
            );
            tx.execute(
                "INSERT INTO remote_projects
                   (project_id, server_id, path, name, last_accessed_at, git_branch)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    project_id,
                    server_id,
                    path,
                    project_name_from_path(path),
                    last_accessed_at,
                    row.branch.as_deref()
                ],
            )
            .map_err(|e| e.to_string())?;
            total += 1;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(total)
    })
}

/// Scan every registered remote server and return per-server outcomes. A
/// failed server keeps its previous snapshot while successful servers commit
/// independently; callers must inspect `partial` instead of treating a count
/// as proof that every server succeeded.
#[tauri::command]
fn scan_all_remote_servers() -> Result<RemoteScanBatchResult, String> {
    let ids = with_prefs(|c| -> Result<Vec<i64>, String> {
        let mut stmt = c
            .prepare("SELECT id FROM remote_servers ORDER BY id ASC")
            .map_err(|e| e.to_string())?;
        let v: Vec<i64> = collect_query_rows(
            stmt.query_map([], |r| r.get::<_, i64>(0))
                .map_err(|e| e.to_string())?,
            "scan_all_remote_servers",
        )?;
        Ok(v)
    })?;
    let outcomes = ids
        .into_iter()
        .map(|id| (id, scan_remote_server(id)))
        .collect();
    Ok(summarize_remote_scan_outcomes(outcomes))
}

/// Cheap ISO-8601 `now()` for `last_accessed_at` stamps. Avoids pulling in
/// `chrono` by formatting from `SystemTime` via the civil-date helper below.
/// The frontend's `relTime()` parses this with `new Date(...)`, and both this
/// and the local SQLite `datetime('now')` shape round-trip fine.
fn chrono_like_now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let total = dur.as_secs() as i64;
    epoch_to_iso(total).expect("the current system time must fit a four-digit ISO year")
}

/// Format a Unix timestamp as the UTC ISO shape consumed by the frontend.
/// Restrict the range to four-digit years so lexical ordering remains valid.
fn epoch_to_iso(secs: i64) -> Option<String> {
    // 0000-01-01T00:00:00Z through 9999-12-31T23:59:59Z.
    if !(-62_167_219_200..=253_402_300_799).contains(&secs) {
        return None;
    }
    let (yr, mo, day, h, mi, s) = epoch_to_ymdhms(secs);
    Some(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        yr, mo, day, h, mi, s
    ))
}

fn latest_iso<'a>(left: Option<&'a str>, right: Option<&'a str>) -> Option<&'a str> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    }
}

/// A remote project's recency is its latest known real activity: Git commit
/// time or AI-tool launch time. For an unborn repository, retain its prior
/// discovery time instead of making every rescan look like fresh activity.
fn remote_last_accessed_at(
    git_activity_epoch: Option<i64>,
    latest_tool_access: Option<&str>,
    previous_access: Option<&str>,
    first_seen_at: &str,
) -> String {
    if let Some(git_activity) = git_activity_epoch.and_then(epoch_to_iso) {
        latest_iso(Some(&git_activity), latest_tool_access)
            .unwrap_or(&git_activity)
            .to_string()
    } else {
        latest_iso(latest_tool_access, previous_access)
            .unwrap_or(first_seen_at)
            .to_string()
    }
}

/// Minimal epoch → civil-date conversion (UTC) so we don't need a
/// `chrono` dependency. Good enough for `last_accessed_at` stamps.
fn epoch_to_ymdhms(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let h = (rem / 3600) as u32;
    let mi = ((rem % 3600) / 60) as u32;
    let s = (rem % 60) as u32;
    // Civil-from-days: Howard Hinnant's algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 {
        (mp + 3) as u32
    } else {
        (mp - 9) as u32
    };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, h, mi, s)
}

/// Pick a human-friendly project name from its path: the last
/// non-empty path segment. Mirrors the local convention.
fn project_name_from_path(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn remote_project_id(server_id: i64, path: &str) -> String {
    let mut hasher: u64 = 1469598103934665603; // FNV offset basis
    for &byte in path.as_bytes() {
        hasher ^= byte as u64;
        hasher = hasher.wrapping_mul(1099511628211);
    }
    format!("r{server_id}:{hasher:x}")
}

/// Aggregate remote_tool_usages rows for the given remote project ids
/// into a HashMap<project_id, Vec<ToolUsage>>. Mirrors the local
/// `fetch_usages_by_project` but reads from prefs.db.
fn fetch_remote_usages_by_project(
    c: &Connection,
    ids: &[String],
) -> Result<HashMap<String, Vec<ToolUsage>>, String> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT project_id, tool_name, tool_key, last_used_at, session_count, last_session_id
         FROM remote_tool_usages
         WHERE project_id IN ({placeholders})
         ORDER BY last_used_at DESC"
    );
    let mut stmt = c.prepare(&sql).map_err(|e| e.to_string())?;
    let params_vec: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let rows = stmt
        .query_map(params_vec.as_slice(), |r| {
            Ok((
                r.get::<_, String>(0)?,
                ToolUsage {
                    tool_key: r.get(2)?,
                    tool_name: r.get(1)?,
                    last_used_at: r.get(3)?,
                    session_count: r.get(4)?,
                    last_session_id: r.get(5)?,
                },
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out: HashMap<String, Vec<ToolUsage>> = HashMap::new();
    for r in collect_query_rows(rows, "fetch_remote_usages_by_project")? {
        out.entry(r.0).or_default().push(r.1);
    }
    Ok(out)
}

/// Returns every remote project across every server, with its
/// recorded tool usages merged in. Same shape as `list_projects`
/// (plus `source` and `remoteServerId`) so the frontend can append
/// directly to `state.all`.
#[tauri::command]
fn list_remote_projects() -> Result<Vec<RemoteProject>, String> {
    with_prefs(|c| {
        let mut stmt = c
            .prepare(
                "SELECT project_id, server_id, path, name, last_accessed_at, git_branch
                 FROM remote_projects
                 ORDER BY last_accessed_at DESC, project_id ASC",
            )
            .map_err(|e| e.to_string())?;
        let mut rows: Vec<(String, i64, String, String, String, Option<String>)> =
            collect_query_rows(
                stmt.query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                })
                .map_err(|e| e.to_string())?,
                "list_remote_projects",
            )?;

        // Bulk-fetch usages (avoids N+1).
        let ids: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
        let usages = fetch_remote_usages_by_project(c, &ids)?;

        let mut out = Vec::with_capacity(rows.len());
        for (id, server_id, path, name, last_accessed_at, git_branch) in rows.drain(..) {
            let tool_usages = usages.get(&id).cloned().unwrap_or_default();
            out.push(RemoteProject {
                id,
                source: "remote".to_string(),
                remote_server_id: server_id,
                path,
                name,
                last_accessed_at,
                git_branch,
                tool_usages,
            });
        }
        Ok(out)
    })
}

/// LIKE-based search across remote projects (name + path). The dataset
/// per server is small enough that LIKE is fine; FTS5 would be overkill
/// for a side DB we own.
#[tauri::command]
fn search_remote_projects(query: String) -> Result<Vec<RemoteProject>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return list_remote_projects();
    }
    let needle = format!("%{}%", trimmed);
    with_prefs(|c| {
        let mut stmt = c
            .prepare(
                "SELECT project_id, server_id, path, name, last_accessed_at, git_branch
                 FROM remote_projects
                 WHERE name LIKE ?1 OR path LIKE ?1
                 ORDER BY last_accessed_at DESC
                 LIMIT 200",
            )
            .map_err(|e| e.to_string())?;
        let rows: Vec<(String, i64, String, String, String, Option<String>)> = collect_query_rows(
            stmt.query_map(params![needle], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })
            .map_err(|e| e.to_string())?,
            "search_remote_projects",
        )?;
        let ids: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
        let usages = fetch_remote_usages_by_project(c, &ids)?;
        let mut out = Vec::with_capacity(rows.len());
        for (id, server_id, path, name, last_accessed_at, git_branch) in rows {
            let tool_usages = usages.get(&id).cloned().unwrap_or_default();
            out.push(RemoteProject {
                id,
                source: "remote".to_string(),
                remote_server_id: server_id,
                path,
                name,
                last_accessed_at,
                git_branch,
                tool_usages,
            });
        }
        Ok(out)
    })
}

/// Bump (or insert) a remote tool-usage row. Called from the frontend
/// right after a remote PTY auto-launches a tool.
fn record_remote_tool_usage_at(
    c: &Connection,
    server_id: i64,
    project_id: &str,
    tool_key: &str,
    tool_name: &str,
    session_id: Option<&str>,
    accessed_at: &str,
) -> Result<(), String> {
    let tx = c.unchecked_transaction().map_err(|e| e.to_string())?;
    let updated = tx
        .execute(
            "UPDATE remote_projects
             SET last_accessed_at = ?1
             WHERE server_id = ?2 AND project_id = ?3",
            params![accessed_at, server_id, project_id],
        )
        .map_err(|e| e.to_string())?;
    if updated == 0 {
        return Err(format!(
            "remote project {project_id} was not found on server {server_id}"
        ));
    }
    tx.execute(
        "INSERT INTO remote_tool_usages
            (server_id, project_id, tool_key, tool_name, last_used_at, session_count, last_session_id)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)
         ON CONFLICT(server_id, project_id, tool_key) DO UPDATE SET
            last_used_at = excluded.last_used_at,
            session_count = session_count + 1,
            last_session_id = COALESCE(excluded.last_session_id, last_session_id)",
        params![
            server_id,
            project_id,
            tool_key,
            tool_name,
            accessed_at,
            session_id
        ],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

#[tauri::command]
fn record_remote_tool_usage(
    server_id: i64,
    project_id: String,
    tool_key: String,
    tool_name: String,
    session_id: Option<String>,
) -> Result<(), String> {
    if project_id.is_empty() {
        return Err("project_id and tool_key required".to_string());
    }
    let tool_key = validate_tool_key(&tool_key)?.to_string();
    let tool_name = validate_display_label(&tool_name)?.to_string();
    let session_id = session_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(validate_session_id)
        .transpose()?
        .map(str::to_string);
    let now = chrono_like_now_iso();
    with_prefs(|c| {
        record_remote_tool_usage_at(
            c,
            server_id,
            &project_id,
            &tool_key,
            &tool_name,
            session_id.as_deref(),
            &now,
        )
    })
}

#[tauri::command]
fn list_project_docs(path: String) -> Result<Vec<DocEntry>, String> {
    let dir = std::path::PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("directory not found: {path}"));
    }
    // Scan the project root plus a few conventional doc subdirs.
    let roots = [
        dir.clone(),
        dir.join("docs"),
        dir.join("doc"),
        dir.join("documentation"),
    ];
    let mut out: Vec<DocEntry> = Vec::new();
    for root in &roots {
        let entries = match std::fs::read_dir(root) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let ext = std::path::Path::new(&name)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext != "md" && ext != "markdown" {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let rel = p
                .strip_prefix(&dir)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            out.push(DocEntry {
                name,
                rel_path: rel,
                size,
            });
        }
    }
    // README first, then alphabetical; deduplicate by rel_path (defensive).
    out.sort_by(|a, b| {
        let a_is_readme = a.name.to_lowercase().starts_with("readme");
        let b_is_readme = b.name.to_lowercase().starts_with("readme");
        match (b_is_readme, a_is_readme) {
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            _ => a.rel_path.cmp(&b.rel_path),
        }
    });
    out.dedup_by(|a, b| a.rel_path == b.rel_path);
    Ok(out)
}

#[tauri::command]
fn read_project_doc(path: String, rel_path: String) -> Result<String, String> {
    let dir = std::path::PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("directory not found: {path}"));
    }
    // Resolve to absolute paths so we can guarantee the requested doc stays
    // inside the project root (rel_path could contain `..` components).
    let canonical_dir = dir.canonicalize().map_err(|e| e.to_string())?;
    let full = canonical_dir.join(&rel_path);
    let canonical_full = full
        .canonicalize()
        .map_err(|e| format!("doc not found: {rel_path}: {e}"))?;
    if !canonical_full.starts_with(&canonical_dir) {
        return Err("doc path escapes project root".into());
    }
    let meta = std::fs::metadata(&canonical_full).map_err(|e| e.to_string())?;
    if meta.len() > MAX_DOC_BYTES {
        return Err(format!(
            "doc too large ({} bytes; max {} bytes)",
            meta.len(),
            MAX_DOC_BYTES
        ));
    }
    std::fs::read_to_string(&canonical_full).map_err(|e| e.to_string())
}

/// Read a single text file (under the project root) for the frontend
/// file tree view. Reuses the read_project_doc security path — same
/// canonical-path + 2 MB cap + UTF-8 read — but lives under a name that
/// describes the use case, so docs and tree files can evolve separately.
#[tauri::command]
fn read_text_file(path: String, rel_path: String) -> Result<String, String> {
    let dir = std::path::PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("directory not found: {path}"));
    }
    let canonical_dir = dir.canonicalize().map_err(|e| e.to_string())?;
    let full = canonical_dir.join(&rel_path);
    let canonical_full = full
        .canonicalize()
        .map_err(|e| format!("file not found: {rel_path}: {e}"))?;
    if !canonical_full.starts_with(&canonical_dir) {
        return Err("file path escapes project root".into());
    }
    let meta = std::fs::metadata(&canonical_full).map_err(|e| e.to_string())?;
    if meta.len() > MAX_DOC_BYTES {
        return Err(format!(
            "file too large ({} bytes; max {} bytes)",
            meta.len(),
            MAX_DOC_BYTES
        ));
    }
    std::fs::read_to_string(&canonical_full).map_err(|e| e.to_string())
}

/* ============================================================
Project file tree.
The frontend renders an expandable tree of the selected project's
directory. We expose a single `list_dir` command that returns the
immediate children of a path; the frontend drives the recursion by
re-calling it as the user expands nodes. We cap each call to 500
entries to avoid sending huge directories in one round trip.
============================================================ */

const MAX_DIR_ENTRIES: usize = 500;

#[derive(Serialize, Clone, Debug)]
pub struct DirEntry {
    #[serde(rename = "name")]
    name: String,
    #[serde(rename = "isDir")]
    is_dir: bool,
    #[serde(rename = "size")]
    size: u64,
}

#[tauri::command]
fn list_dir(path: String) -> Result<Vec<DirEntry>, String> {
    let dir = std::path::PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    let read = std::fs::read_dir(&dir).map_err(|e| format!("read_dir: {e}"))?;
    let mut out: Vec<DirEntry> = Vec::new();
    for e in read {
        let e = match e {
            Ok(e) => e,
            Err(_) => continue,
        };
        let meta = match e.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let name = e.file_name().to_string_lossy().to_string();
        out.push(DirEntry {
            name,
            is_dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
        });
        if out.len() > MAX_DIR_ENTRIES {
            return Err(format!(
                "too many entries (>{}); drill into a subdirectory",
                MAX_DIR_ENTRIES
            ));
        }
    }
    // Directories first, then alphabetical within each group. Stable.
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(out)
}

/* ============================================================
Project git info.
Surfaces branch / remotes / head summary / dirty flag for the
currently selected project so the footer can show a one-line
summary + clickable remotes. We shell out to `git` (which is a
hard requirement of the app's CLI workflow anyway) rather than
pulling in a Rust git library.
============================================================ */

#[derive(Serialize, Clone, Debug)]
pub struct GitRemote {
    #[serde(rename = "name")]
    name: String,
    #[serde(rename = "url")]
    url: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct BranchInfo {
    #[serde(rename = "name")]
    name: String,
    #[serde(rename = "isCurrent")]
    is_current: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct GitInfo {
    #[serde(rename = "isRepo")]
    is_repo: bool,
    #[serde(rename = "branch")]
    branch: Option<String>,
    #[serde(rename = "remotes")]
    remotes: Vec<GitRemote>,
    #[serde(rename = "localBranches")]
    local_branches: Vec<BranchInfo>,
    #[serde(rename = "headShort")]
    head_short: Option<String>,
    #[serde(rename = "headSummary")]
    head_summary: Option<String>,
    #[serde(rename = "dirty")]
    dirty: bool,
    #[serde(rename = "error")]
    error: Option<String>,
}

/// Run `git -C <path> <args...>`, returning trimmed stdout (None on non-zero
/// exit or spawn failure). All git queries in this section are best-effort
/// reads — the frontend treats failures as "no info" rather than errors.
fn run_git(path: &str, args: &[&str]) -> Option<String> {
    run_git_with(&SystemProcessRunner, path, args)
}

fn run_git_with(runner: &dyn ProcessRunner, path: &str, args: &[&str]) -> Option<String> {
    let spec = git_read_spec(std::path::Path::new(path), args);
    let out = runner.output(&spec).ok()?;
    if !out.success {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[tauri::command]
fn get_git_info(path: String) -> Result<GitInfo, String> {
    if !std::path::Path::new(&path).is_dir() {
        return Err(format!("directory not found: {path}"));
    }
    let is_repo = run_git(&path, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s == "true")
        .unwrap_or(false);
    if !is_repo {
        return Ok(GitInfo {
            is_repo: false,
            branch: None,
            remotes: vec![],
            local_branches: vec![],
            head_short: None,
            head_summary: None,
            dirty: false,
            error: Some("not a git repository".into()),
        });
    }
    let branch = run_git(&path, &["branch", "--show-current"]);
    let head_short = run_git(&path, &["rev-parse", "--short", "HEAD"]);
    let head_summary = run_git(&path, &["log", "-1", "--pretty=%s"]);
    let dirty = run_git(&path, &["status", "--porcelain"])
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    // Local branches via for-each-ref. Format: `* main` (current) or
    // `  feature` (non-current) — leading char is the HEAD marker.
    let mut local_branches: Vec<BranchInfo> = Vec::new();
    if let Some(out) = run_git(
        &path,
        &[
            "for-each-ref",
            "--format=%(HEAD) %(refname:short)",
            "refs/heads/",
        ],
    ) {
        for line in out.lines() {
            let mut parts = line.splitn(2, ' ');
            let marker = parts.next().unwrap_or("");
            let name = parts.next().unwrap_or("");
            if name.is_empty() {
                continue;
            }
            local_branches.push(BranchInfo {
                name: name.to_string(),
                is_current: marker == "*",
            });
        }
    }
    // `git remote -v` lines: "<name>\t<url> (fetch|push)". Two lines per
    // remote (fetch + push), we dedupe by name and keep the fetch URL.
    let mut remotes: Vec<GitRemote> = Vec::new();
    if let Some(out) = run_git(&path, &["remote", "-v"]) {
        for line in out.lines() {
            // split into [name, url "(fetch|push)"]
            let mut parts = line.split_whitespace();
            let name = match parts.next() {
                Some(n) => n,
                None => continue,
            };
            let url = match parts.next() {
                Some(u) => u,
                None => continue,
            };
            // skip duplicates (fetch + push emit the same name twice)
            if remotes.iter().any(|r| r.name == name) {
                continue;
            }
            remotes.push(GitRemote {
                name: name.to_string(),
                url: url.to_string(),
            });
        }
    }
    Ok(GitInfo {
        is_repo: true,
        branch,
        remotes,
        local_branches,
        head_short,
        head_summary,
        dirty,
        error: None,
    })
}

#[tauri::command]
fn add_git_remote(path: String, name: String, url: String) -> Result<(), String> {
    if !std::path::Path::new(&path).is_dir() {
        return Err(format!("directory not found: {path}"));
    }
    let name = name.trim();
    if name.is_empty() {
        return Err("remote name cannot be empty".into());
    }
    let url = url.trim();
    if url.is_empty() {
        return Err("remote url cannot be empty".into());
    }
    // If the directory isn't a git repo yet, initialize one first so the
    // remote can be attached. This makes "+ add remote" a one-click
    // "make this a git project and connect it" affordance — the user
    // can then `git add . && git commit` to push their first commit.
    let is_repo = run_git(&path, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s == "true")
        .unwrap_or(false);
    if !is_repo {
        let init = Command::new("git")
            .arg("-C")
            .arg(&path)
            .arg("init")
            .output()
            .map_err(|e| format!("failed to run git: {e}"))?;
        if !init.status.success() {
            return Err(format!(
                "git init failed: {}",
                String::from_utf8_lossy(&init.stderr).trim()
            ));
        }
    }
    let out = Command::new("git")
        .arg("-C")
        .arg(&path)
        .args(["remote", "add", name, url])
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git remote add failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Switch to a known local branch. We refuse to accept arbitrary names —
/// the branch must already exist locally (defense in depth: prevents a
/// typo from creating a new branch or running a git-injection). Git's
/// own "dirty worktree" check still applies, so a failed switch is
/// surfaced to the user without being silently destructive.
#[tauri::command]
fn checkout_branch(path: String, name: String) -> Result<(), String> {
    if !std::path::Path::new(&path).is_dir() {
        return Err(format!("directory not found: {path}"));
    }
    let name = name.trim();
    if name.is_empty() {
        return Err("branch name cannot be empty".into());
    }
    // Reject anything that isn't a clean local ref name. Allowed chars:
    // letters, digits, dot, underscore, slash, dash — no spaces, no
    // shell metacharacters, no leading dash (which git would parse as
    // a flag).
    if name.starts_with('-')
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '/' || c == '-')
    {
        return Err(format!("invalid branch name: {name}"));
    }
    // Confirm the branch actually exists in the local repo.
    let known = run_git(
        &path,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
    )
    .map(|s| s.lines().map(str::to_string).collect::<Vec<_>>())
    .unwrap_or_default();
    if !known.iter().any(|n| n == name) {
        return Err(format!("unknown local branch: {name}"));
    }
    // `git switch` (not `checkout`) is the modern, safer command —
    // refuses to clobber uncommitted changes unless --force.
    let out = Command::new("git")
        .arg("-C")
        .arg(&path)
        .args(["switch", name])
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git switch failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

// Note: `remove_git_remote` was deliberately not exposed. Deleting a
// remote is destructive and easy to do by accident, so the user must run
// `git remote remove <name>` (or `git remote set-url ...`) themselves in
// the project terminal.

/// Open an http(s) URL in the system default browser. We whitelist the
/// scheme so the command can't be misused to launch arbitrary binaries.
#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    open_external_url_with(&SystemProcessRunner, &url)
}

fn open_external_url_with(runner: &dyn ProcessRunner, url: &str) -> Result<(), String> {
    let url = validate_external_url(url)?;
    #[cfg(target_os = "windows")]
    {
        // explorer.exe delegates absolute http(s) URLs to the default browser
        // without routing attacker-controlled text through cmd.exe.
        let spec = process::ProcessSpec::new("explorer.exe").arg(&url);
        runner.spawn(&spec)?;
    }
    #[cfg(target_os = "macos")]
    {
        let spec = process::ProcessSpec::new("open").arg(&url);
        runner.spawn(&spec)?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let spec = process::ProcessSpec::new("xdg-open").arg(&url);
        runner.spawn(&spec)?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/* ============================================================
System tray.
- Tray icon lives in the OS notification area.
- Clicking the window's close button hides it to the tray (intercepted
  via on_window_event, not destroyed).
- Right-click the tray icon → dynamic menu of projects (grouped by
  user-defined group, plus an "未分组" bucket), a Show entry, and
  Quit. Picking a project emits `project:open` with the project id;
  the frontend listens, focuses the window, and opens the project
  with its most-recently-used tool.
- The frontend feeds the tray via `update_tray_projects` after each
  `reload()` so the menu tracks the live state without us having to
  query both DBs from Rust.
============================================================ */
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrayProject {
    #[serde(rename = "id")]
    id: String,
    #[serde(rename = "name")]
    name: String,
    #[serde(rename = "path")]
    path: String,
    /// Most-recently-used tool key (e.g. "claude"), or None if the
    /// project has never been opened with any tool.
    #[serde(rename = "topTool")]
    top_tool: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrayGroup {
    #[serde(rename = "id")]
    id: i64,
    #[serde(rename = "name")]
    name: String,
}

#[derive(Default, Clone)]
struct TrayData {
    projects: Vec<TrayProject>,
    groups: Vec<TrayGroup>,
    /// project_id -> group_id. Built from `list_group_assignments`.
    assignments: HashMap<String, i64>,
}

#[derive(Default)]
struct AppState {
    tray: Mutex<TrayData>,
    /// Current interface language ("en" | "zh"), pushed from the frontend
    /// via `set_tray_language` so the OS tray menu labels follow it.
    lang: Mutex<String>,
}

// ID we use when looking up the single tray icon we manage.
const TRAY_ID: &str = "agent-hub-main";

/// Resolve a tray menu label for the given key in the active language.
/// Tiny inline map — only a handful of strings live in the tray, so a
/// full i18n layer on the Rust side would be overkill. Defaults to English
/// for any unrecognized language code.
fn tray_label(key: &str, lang: &str) -> &'static str {
    let zh = matches!(lang, "zh" | "zh-CN" | "zh-CN.UTF-8");
    match key {
        "show" => {
            if zh {
                "显示"
            } else {
                "Show"
            }
        }
        "quit" => {
            if zh {
                "退出"
            } else {
                "Quit"
            }
        }
        "ungrouped" => {
            if zh {
                "未分组"
            } else {
                "Ungrouped"
            }
        }
        _ => "",
    }
}

fn rebuild_tray_menu(app: &AppHandle) -> tauri::Result<()> {
    let (data, lang) = {
        let state = app.state::<AppState>();
        let data = state.tray.lock().unwrap().clone();
        let lang = state.lang.lock().unwrap().clone();
        (data, lang)
    };
    let menu = build_tray_menu(app, &data, &lang)?;
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(menu))?;
        // Tauri 2 on Windows: set_menu updates the internal menu
        // model but the visible tray submenus don't always
        // repaint until the icon is redrawn. A short hide+show
        // forces the OS to rebuild the icon with the new submenus.
        // Tauri 2 on Windows: set_menu updates the internal menu
        // but the visible tray submenus don't always repaint. A
        // brief set_visible(false) / set_visible(true) forces a redraw.
        let _ = tray.set_visible(false);
        let _ = tray.set_visible(true);
    }
    Ok(())
}

fn build_tray_menu(
    app: &AppHandle,
    data: &TrayData,
    lang: &str,
) -> tauri::Result<tauri::menu::Menu<Wry>> {
    let app_for_items = app.clone();
    let mut builder = MenuBuilder::new(app);

    // Bucket projects by group, preserving the group's declared order.
    let mut by_group: BTreeMap<i64, Vec<&TrayProject>> = BTreeMap::new();
    let mut ungrouped: Vec<&TrayProject> = Vec::new();
    for p in &data.projects {
        match data.assignments.get(&p.id) {
            Some(&gid) => by_group.entry(gid).or_default().push(p),
            None => ungrouped.push(p),
        }
    }
    for g in &data.groups {
        if let Some(items) = by_group.get(&g.id) {
            let mut sub_items: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::new();
            for p in items {
                let item: Box<dyn IsMenuItem<Wry>> = Box::new(
                    MenuItem::with_id(
                        &app_for_items,
                        format!("project:{}", p.id),
                        &p.name,
                        true,
                        None::<&str>,
                    )
                    .expect("menu item"),
                );
                sub_items.push(item);
            }
            let sub_refs: Vec<&dyn IsMenuItem<Wry>> =
                sub_items.iter().map(|b| b.as_ref()).collect();
            let sub = Submenu::with_id_and_items(
                &app_for_items,
                format!("group:{}", g.id),
                &g.name,
                true,
                &sub_refs,
            )?;
            builder = builder.item(&sub);
        }
    }
    if !ungrouped.is_empty() {
        let mut sub_items: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::new();
        for p in &ungrouped {
            let item: Box<dyn IsMenuItem<Wry>> = Box::new(
                MenuItem::with_id(
                    &app_for_items,
                    format!("project:{}", p.id),
                    &p.name,
                    true,
                    None::<&str>,
                )
                .expect("menu item"),
            );
            sub_items.push(item);
        }
        let sub_refs: Vec<&dyn IsMenuItem<Wry>> = sub_items.iter().map(|b| b.as_ref()).collect();
        let sub = Submenu::with_id_and_items(
            &app_for_items,
            "group:ungrouped",
            tray_label("ungrouped", lang),
            true,
            &sub_refs,
        )?;
        builder = builder.item(&sub);
    }

    // Only show the separator + Show/Quit if we actually have projects;
    // otherwise the menu is just Show/Quit (empty case before the first
    // reload() completes).
    if !data.projects.is_empty() {
        builder = builder.separator();
    }

    builder = builder
        .item(
            &MenuItem::with_id(app, "show", tray_label("show", lang), true, None::<&str>)
                .expect("menu item"),
        )
        .item(
            &MenuItem::with_id(app, "quit", tray_label("quit", lang), true, None::<&str>)
                .expect("menu item"),
        );

    builder.build()
}

/// Frontend pushes its current project + group state so the tray menu
/// mirrors the ledger. Called after `reload()` and on group changes.
#[tauri::command]
fn update_tray_projects(
    projects: Vec<TrayProject>,
    groups: Vec<TrayGroup>,
    assignments: HashMap<String, i64>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    {
        let mut data = state.tray.lock().unwrap();
        data.projects = projects;
        data.groups = groups;
        data.assignments = assignments;
    }
    rebuild_tray_menu(&app).map_err(|e| e.to_string())
}

/// Set the interface language used by the OS tray menu labels, then
/// rebuild the menu so Show / Quit / the ungrouped submenu follow it.
/// The frontend calls this on boot and whenever the user changes the
/// language in Settings. Accepts "en" or "zh"; anything else falls back
/// to English inside `tray_label`.
#[tauri::command]
fn set_tray_language(
    lang: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    {
        let mut g = state.lang.lock().map_err(|e| e.to_string())?;
        *g = lang;
    }
    rebuild_tray_menu(&app).map_err(|e| e.to_string())
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .manage(PtyRegistry::default())
        .manage(AppState::default())
        .setup(|app| {
            // Build a minimal initial menu so the tray icon is usable
            // before the first `update_tray_projects` arrives (Show +
            // Quit are always present). Labels start English; the frontend
            // pushes the user's language via `set_tray_language` on boot.
            let initial = MenuBuilder::new(app)
                .item(&MenuItem::with_id(
                    app,
                    "show",
                    tray_label("show", "en"),
                    true,
                    None::<&str>,
                )?)
                .item(&MenuItem::with_id(
                    app,
                    "quit",
                    tray_label("quit", "en"),
                    true,
                    None::<&str>,
                )?)
                .build()?;
            let _tray = TrayIconBuilder::with_id(TRAY_ID)
                .icon(app.default_window_icon().cloned().unwrap_or_else(|| {
                    // Should never happen — the bundle always ships an
                    // icon — but fall back to no icon rather than crash.
                    tauri::image::Image::new_owned(vec![0], 1, 1)
                }))
                .tooltip("SessionAtlas")
                .menu(&initial)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    id if id.starts_with("project:") => {
                        // Show window first so the user sees the action,
                        // then emit the project id; frontend calls
                        // openProjectDefault() to honour the user's
                        // most-recently-used tool for that project.
                        let project_id = id.trim_start_matches("project:").to_string();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                        let _ = app.emit("project:open", project_id);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        // Left-click on the tray icon toggles the
                        // window: show + focus. (Right-click opens the
                        // menu by default.)
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;
            Ok(())
        })
        // Intercept window close → hide to tray instead of quitting.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_projects,
            search_projects,
            list_tools,
            scan_projects,
            launch_project,
            pty_spawn,
            pty_attach,
            pty_write,
            pty_remote_switch,
            pty_resize,
            pty_kill,
            notify,
            list_opener_prefs,
            set_opener_enabled,
            set_opener_command,
            upsert_custom_opener,
            delete_custom_opener,
            open_with_opener,
            list_groups,
            create_group,
            rename_group,
            delete_group,
            assign_project_to_group,
            list_group_assignments,
            list_sort_orders,
            get_group_revision,
            move_group_project,
            set_group_order,
            list_project_docs,
            read_project_doc,
            read_text_file,
            list_dir,
            get_git_info,
            add_git_remote,
            checkout_branch,
            open_external_url,
            update_tray_projects,
            set_tray_language,
            list_remote_servers,
            add_remote_server,
            delete_remote_server,
            test_remote_connection,
            scan_remote_server,
            scan_all_remote_servers,
            list_remote_projects,
            search_remote_projects,
            record_remote_tool_usage
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    app.run(|app, event| {
        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) {
            shutdown_pty_sessions(&app.state::<PtyRegistry>());
        }
    });
}

#[cfg(test)]
mod baseline_tests {
    use super::*;
    use crate::process::{ProcessOutput, ProcessSpec};
    use std::cell::RefCell;

    #[test]
    fn home_override_accepts_an_absolute_path() {
        let current = std::env::temp_dir().join("sessionatlas-current");
        let absolute = std::env::temp_dir().join("sessionatlas-acceptance-home");

        assert_eq!(
            resolve_home_directory(absolute.to_str(), None, &current),
            absolute
        );
    }

    #[test]
    fn home_override_resolves_a_relative_path_from_the_current_directory() {
        let current = std::env::temp_dir().join("sessionatlas-current");

        assert_eq!(
            resolve_home_directory(Some("acceptance-home"), None, &current),
            current.join("acceptance-home")
        );
    }

    #[test]
    fn blank_or_missing_home_override_uses_the_fallback() {
        let current = std::env::temp_dir().join("sessionatlas-current");
        let fallback = std::env::temp_dir().join("sessionatlas-os-home");

        assert_eq!(
            resolve_home_directory(Some("  \t"), Some(fallback.clone()), &current),
            fallback
        );
        assert_eq!(resolve_home_directory(None, None, &current), current);
    }

    #[test]
    fn all_application_files_share_one_sessionatlas_data_directory() {
        let home = std::env::temp_dir().join("sessionatlas-home");
        let data_directory = home.join(DATA_DIRECTORY);

        assert_eq!(
            data_path_for_home(&home, "index.db"),
            data_directory.join("index.db")
        );
        assert_eq!(
            data_path_for_home(&home, "config.json"),
            data_directory.join("config.json")
        );
        assert_eq!(
            data_path_for_home(&home, "prefs.db"),
            data_directory.join("prefs.db")
        );
    }

    #[test]
    fn claude_queue_preserves_normal_permission_checks() {
        let argv = claude_print_argv("summarize the repository");

        assert_eq!(argv, vec!["claude", "-p", "summarize the repository"]);
        assert!(!argv.iter().any(|arg| arg.contains("skip-permissions")));
    }

    #[test]
    fn project_limit_rejects_unbounded_and_negative_values_before_querying() {
        assert_eq!(validate_project_limit(None).unwrap(), 500);
        assert_eq!(validate_project_limit(Some(1)).unwrap(), 1);
        assert_eq!(validate_project_limit(Some(10_000)).unwrap(), 10_000);
        for invalid in [0, -1, 10_001] {
            assert!(validate_project_limit(Some(invalid)).is_err());
        }
    }

    #[test]
    fn index_reader_rejects_writes_without_creating_sidecars() {
        let root = std::env::temp_dir().join(format!(
            "sessionatlas-index-reader-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("index.db");
        {
            let writer = Connection::open(&path).unwrap();
            writer
                .execute_batch("CREATE TABLE projects (id INTEGER PRIMARY KEY)")
                .unwrap();
        }
        let before: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();

        let reader = open_index_reader(&path).unwrap();
        let query_only: i64 = reader
            .pragma_query_value(None, "query_only", |row| row.get(0))
            .unwrap();
        assert_eq!(query_only, 1);
        let write_error = reader
            .execute_batch("CREATE TABLE should_not_exist (id INTEGER)")
            .unwrap_err();
        assert!(write_error.to_string().contains("readonly"));
        drop(reader);

        let after: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(before, after);
        assert!(!root.join("index.db-wal").exists());
        assert!(!root.join("index.db-shm").exists());
        assert!(!root.join("index.db-journal").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_sqlite_row_is_reported_instead_of_omitted() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE projects (id TEXT, path BLOB, last_accessed_at TEXT, git_branch TEXT);
                 INSERT INTO projects VALUES ('ok', 'C:/ok', '2026-01-01', NULL);
                 INSERT INTO projects VALUES ('bad', X'00', '2026-01-02', NULL);",
            )
            .unwrap();
        let mut statement = connection
            .prepare("SELECT id, path, last_accessed_at, git_branch FROM projects ORDER BY id")
            .unwrap();
        let rows = statement.query_map([], row_to_project_fields).unwrap();
        let result = collect_query_rows(rows, "test projects");
        let error = result.unwrap_err();
        assert!(error.contains("test projects"));
        assert!(error.contains("Invalid column type") || error.contains("type"));
    }

    #[test]
    fn malformed_existing_group_is_not_treated_as_missing() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE project_groups (id INTEGER PRIMARY KEY, name TEXT, sort_order INTEGER);
                 CREATE TABLE project_group_assignments (project_id TEXT, group_id INTEGER);
                 INSERT INTO project_groups VALUES (1, 'broken', X'00');",
            )
            .unwrap();

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let result = find_existing_group(&transaction, "broken");
        assert!(result.is_err());
    }

    #[test]
    fn custom_remote_scan_command_accumulates_find_failures() {
        let command = build_remote_scan_command("/missing /projects").unwrap();
        assert!(command.contains("scan_failed=0"));
        assert!(command.contains("scan_failed=1"));
        assert!(command.contains("exit \"$scan_failed\""));
        assert!(!command.contains("if [ ! -e \"$r\" ]; then continue; fi"));
        assert!(!command.contains("+ 2>/dev/null"));
    }

    #[test]
    fn built_in_remote_scan_roots_skip_only_absent_optional_directories() {
        let command = build_remote_scan_command(DEFAULT_REMOTE_SCAN_ROOTS).unwrap();
        assert!(command.contains("if [ ! -e \"$r\" ]; then continue; fi"));
        assert!(command.contains("scan_failed=1"));
    }

    #[test]
    fn remote_scan_rejects_partial_stdout_when_process_fails() {
        let output = ProcessOutput {
            success: false,
            status_code: Some(1),
            stdout: b"/home/demo/repo\0main\x00123\0".to_vec(),
            stderr: b"find: /missing: Permission denied\n".to_vec(),
        };
        let error =
            validate_remote_scan_output(&output, "/home/demo", "demo", "example.test", 22, None)
                .unwrap_err();
        assert!(error.contains("remote scan command failed"));
        assert!(error.contains("Permission denied"));
    }

    #[test]
    fn remote_scan_batch_reports_partial_server_failures() {
        let summary = summarize_remote_scan_outcomes(vec![
            (1, Ok(3)),
            (2, Err("remote scan command failed".to_string())),
            (3, Ok(0)),
        ]);
        assert_eq!(summary.total_count, 3);
        assert_eq!(summary.success_count, 2);
        assert_eq!(summary.failure_count, 1);
        assert!(summary.partial);
        assert_eq!(summary.servers[0].count, 3);
        assert_eq!(
            summary.servers[1].error_kind.as_deref(),
            Some("remote_scan")
        );
        assert_eq!(summary.servers[2].count, 0);
    }

    #[test]
    fn remote_scan_diagnostic_is_redacted_and_bounded() {
        let mut stderr = b"/home/demo failed using /keys/id_ed25519: ".to_vec();
        stderr.extend(std::iter::repeat_n(b'x', 5000));
        let error = validate_remote_scan_output(
            &ProcessOutput {
                success: false,
                status_code: Some(1),
                stdout: Vec::new(),
                stderr,
            },
            "/home/demo",
            "demo",
            "example.test",
            22,
            Some("/keys/id_ed25519"),
        )
        .unwrap_err();
        assert!(!error.contains("/home/demo"));
        assert!(!error.contains("/keys/id_ed25519"));
        assert!(error.contains("[truncated]"));
        assert!(error.len() <= MAX_REMOTE_SCAN_DIAGNOSTIC_BYTES + 128);
    }

    #[cfg(unix)]
    #[test]
    fn custom_remote_scan_shell_fails_closed_for_missing_root() {
        let root =
            std::env::temp_dir().join(format!("sessionatlas-remote-shell-{}", std::process::id()));
        std::fs::create_dir_all(root.join("good")).unwrap();
        let missing = root.join("missing");
        let roots = format!("{} {}", missing.display(), root.join("good").display());
        let command = build_remote_scan_command(&roots).unwrap();
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .unwrap();
        assert!(!output.status.success());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn group_fixture() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE project_groups (id INTEGER PRIMARY KEY, name TEXT UNIQUE, sort_order INTEGER);
                 CREATE TABLE project_group_assignments (project_id TEXT PRIMARY KEY, group_id INTEGER NOT NULL REFERENCES project_groups(id) ON DELETE CASCADE);
                 CREATE TABLE project_sort (project_id TEXT PRIMARY KEY, group_key TEXT NOT NULL, sort_order INTEGER NOT NULL);
                 CREATE TABLE prefs_revisions (scope TEXT PRIMARY KEY, revision INTEGER NOT NULL);
                 INSERT INTO prefs_revisions VALUES ('groups', 0);
                 INSERT INTO project_groups VALUES (1, 'one', 10), (2, 'two', 20);
                 INSERT INTO project_group_assignments VALUES ('p1', 1), ('p2', 1);
                 INSERT INTO project_sort VALUES ('p1', '1', 10), ('p2', '1', 20);
                 INSERT INTO project_sort VALUES ('p3', '2', 10);",
            )
            .unwrap();
        connection
    }

    #[test]
    fn opener_updates_report_not_found_instead_of_false_success() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE opener_prefs (
                    id INTEGER PRIMARY KEY,
                    command_template TEXT NOT NULL,
                    enabled INTEGER NOT NULL
                 );
                 INSERT INTO opener_prefs VALUES (1, 'code {path}', 1);",
            )
            .unwrap();

        assert!(set_opener_enabled_db(&connection, 99, false).is_err());
        assert!(set_opener_command_db(&connection, 99, "cmd {path}").is_err());
        set_opener_enabled_db(&connection, 1, false).unwrap();
        set_opener_command_db(&connection, 1, "cmd {path}").unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT command_template, enabled FROM opener_prefs WHERE id = 1",
                    [],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                )
                .unwrap(),
            ("cmd {path}".to_string(), 0)
        );
    }

    fn group_snapshot(connection: &Connection) -> String {
        let groups: Vec<(i64, String, i64)> = connection
            .prepare("SELECT id, name, sort_order FROM project_groups ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let assignments: Vec<(String, i64)> = connection
            .prepare(
                "SELECT project_id, group_id FROM project_group_assignments ORDER BY project_id",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let sort: Vec<(String, String, i64)> = connection
            .prepare(
                "SELECT project_id, group_key, sort_order FROM project_sort ORDER BY project_id",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let revision: i64 = connection
            .query_row(
                "SELECT revision FROM prefs_revisions WHERE scope = 'groups'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        format!("{groups:?}|{assignments:?}|{sort:?}|{revision}")
    }

    #[test]
    fn group_delete_rolls_back_when_cleanup_fails() {
        let mut connection = group_fixture();
        let before = group_snapshot(&connection);
        connection
            .execute_batch(
                "CREATE TRIGGER fail_sort_delete BEFORE DELETE ON project_sort
                 BEGIN SELECT RAISE(ABORT, 'sort cleanup failed'); END;",
            )
            .unwrap();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(delete_group_tx(&transaction, 1).is_err());
        drop(transaction);
        assert_eq!(group_snapshot(&connection), before);
        assert_eq!(
            connection
                .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
    }

    #[test]
    fn group_assign_rolls_back_when_sort_write_fails() {
        let mut connection = group_fixture();
        let before = group_snapshot(&connection);
        connection
            .execute_batch(
                "CREATE TRIGGER fail_sort_insert BEFORE INSERT ON project_sort
                 BEGIN SELECT RAISE(ABORT, 'sort insert failed'); END;",
            )
            .unwrap();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(assign_project_to_group_tx(&transaction, "new", Some(1)).is_err());
        drop(transaction);
        assert_eq!(group_snapshot(&connection), before);
    }

    #[test]
    fn group_order_rolls_back_when_assignment_write_fails() {
        let mut connection = group_fixture();
        let before = group_snapshot(&connection);
        connection
            .execute_batch(
                "CREATE TRIGGER fail_assignment_insert BEFORE INSERT ON project_group_assignments
                 BEGIN SELECT RAISE(ABORT, 'assignment insert failed'); END;",
            )
            .unwrap();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(
            set_group_order_tx(&transaction, "2", &["p3".to_string(), "p1".to_string()]).is_err()
        );
        drop(transaction);
        assert_eq!(group_snapshot(&connection), before);
    }

    #[test]
    fn group_order_rejects_invalid_key_and_duplicate_ids_before_writes() {
        let mut connection = group_fixture();
        let before = group_snapshot(&connection);
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(set_group_order_tx(&transaction, "999", &["p1".to_string()]).is_err());
        drop(transaction);
        assert_eq!(group_snapshot(&connection), before);

        let mut connection = group_fixture();
        let before = group_snapshot(&connection);
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(
            set_group_order_tx(&transaction, "1", &["p1".to_string(), "p1".to_string()]).is_err()
        );
        drop(transaction);
        assert_eq!(group_snapshot(&connection), before);
    }

    #[test]
    fn group_order_rejects_incomplete_existing_members_before_writes() {
        let mut connection = group_fixture();
        let before = group_snapshot(&connection);
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(set_group_order_tx(&transaction, "1", &["p1".to_string()]).is_err());
        drop(transaction);
        assert_eq!(group_snapshot(&connection), before);
    }

    #[test]
    fn group_move_uses_anchor_and_advances_revision() {
        let mut connection = group_fixture();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let result = move_group_project_tx(
            &transaction,
            "p3",
            "1",
            "p1",
            "after",
            &["p1".to_string(), "p2".to_string(), "p3".to_string()],
            0,
        )
        .unwrap();
        transaction.commit().unwrap();

        assert_eq!(result.revision, 1);
        assert_eq!(result.ordered_ids, ["p1", "p3", "p2"]);
        assert_eq!(
            connection
                .query_row(
                    "SELECT group_id FROM project_group_assignments WHERE project_id = 'p3'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT revision FROM prefs_revisions WHERE scope = 'groups'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn group_move_rejects_stale_revision_and_wrong_anchor_without_writes() {
        let mut connection = group_fixture();
        let before = group_snapshot(&connection);
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(move_group_project_tx(
            &transaction,
            "p3",
            "1",
            "p1",
            "after",
            &["p1".to_string(), "p2".to_string(), "p3".to_string()],
            9,
        )
        .is_err());
        drop(transaction);
        assert_eq!(group_snapshot(&connection), before);

        let mut connection = group_fixture();
        let before = group_snapshot(&connection);
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(move_group_project_tx(
            &transaction,
            "p2",
            "2",
            "p1",
            "before",
            &["p1".to_string(), "p2".to_string(), "p3".to_string()],
            0,
        )
        .is_err());
        drop(transaction);
        assert_eq!(group_snapshot(&connection), before);
    }

    #[test]
    fn group_move_rolls_back_assignment_when_sort_write_fails() {
        let mut connection = group_fixture();
        let before = group_snapshot(&connection);
        connection
            .execute_batch(
                "CREATE TRIGGER fail_move_sort BEFORE UPDATE ON project_sort
                 BEGIN SELECT RAISE(ABORT, 'move sort failed'); END;",
            )
            .unwrap();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(move_group_project_tx(
            &transaction,
            "p3",
            "1",
            "p1",
            "after",
            &["p1".to_string(), "p2".to_string(), "p3".to_string()],
            0,
        )
        .is_err());
        drop(transaction);
        assert_eq!(group_snapshot(&connection), before);
    }

    struct RecordingRunner {
        requests: RefCell<Vec<ProcessSpec>>,
        spawn_requests: RefCell<Vec<ProcessSpec>>,
        output: ProcessOutput,
    }

    impl ProcessRunner for RecordingRunner {
        fn output(&self, spec: &ProcessSpec) -> Result<ProcessOutput, String> {
            self.requests.borrow_mut().push(spec.clone());
            Ok(self.output.clone())
        }

        fn spawn(&self, spec: &ProcessSpec) -> Result<(), String> {
            self.spawn_requests.borrow_mut().push(spec.clone());
            Ok(())
        }
    }

    #[test]
    fn git_reads_are_testable_without_starting_git() {
        let runner = RecordingRunner {
            requests: RefCell::new(Vec::new()),
            spawn_requests: RefCell::new(Vec::new()),
            output: ProcessOutput {
                success: true,
                status_code: Some(0),
                stdout: b"main\n".to_vec(),
                stderr: Vec::new(),
            },
        };

        assert_eq!(
            run_git_with(
                &runner,
                r"C:\fixture workspace",
                &["branch", "--show-current"]
            ),
            Some("main".to_string())
        );
        let requests = runner.requests.borrow();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].program, std::ffi::OsString::from("git"));
    }

    #[test]
    fn ssh_probe_is_constructed_without_starting_ssh() {
        let runner = RecordingRunner {
            requests: RefCell::new(Vec::new()),
            spawn_requests: RefCell::new(Vec::new()),
            output: ProcessOutput {
                success: true,
                status_code: Some(0),
                stdout: b"fixture\n".to_vec(),
                stderr: Vec::new(),
            },
        };
        let spec = build_ssh_command("demo", "example.test", 2222, None, "printf ok").unwrap();

        let output = runner.output(&spec).unwrap();

        assert!(output.success);
        let requests = runner.requests.borrow();
        assert_eq!(requests.len(), 1);
        assert!(requests[0]
            .args
            .contains(&std::ffi::OsString::from("BatchMode=yes")));
        assert!(requests[0]
            .args
            .contains(&std::ffi::OsString::from("demo@example.test")));
        let delimiter = requests[0].args.iter().position(|arg| arg == "--").unwrap();
        let destination = requests[0]
            .args
            .iter()
            .position(|arg| arg == "demo@example.test")
            .unwrap();
        assert!(delimiter < destination);
        assert!(
            build_ssh_command("-oProxyCommand=calc", "example.test", 22, None, "printf ok")
                .is_err()
        );
    }

    #[test]
    fn remote_connection_probe_reports_tmux_capability() {
        assert_eq!(
            parse_remote_connection_probe(
                "banner:SESSIONATLAS_SSH_OK:/home/demo\nSESSIONATLAS_TMUX_OK:tmux 3.4\n"
            ),
            RemoteConnectionProbe {
                home: "/home/demo".to_string(),
                tmux_available: true,
                tmux_version: Some("tmux 3.4".to_string()),
            }
        );
        assert_eq!(
            parse_remote_connection_probe(
                "SESSIONATLAS_SSH_OK:/home/demo\nSESSIONATLAS_TMUX_MISSING\n"
            ),
            RemoteConnectionProbe {
                home: "/home/demo".to_string(),
                tmux_available: false,
                tmux_version: None,
            }
        );
    }

    #[test]
    fn remote_tmux_names_are_stable_safe_and_tool_scoped() {
        let first = remote_tmux_session_name("/srv/Project One", Some("custom.tool")).unwrap();
        let second = remote_tmux_session_name("/srv/Project One", Some("custom.tool")).unwrap();
        let other_tool = remote_tmux_session_name("/srv/Project One", Some("claude")).unwrap();

        assert_eq!(first, second);
        assert_ne!(first, other_tool);
        assert!(first.starts_with("sessionatlas-custom-tool-"));
        assert!(first
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_')));
        assert!(remote_tmux_session_name("/srv/project", Some("-unsafe")).is_err());
    }

    #[test]
    fn remote_pty_forwards_a_tmux_compatible_terminal_type() {
        let mut command = CommandBuilder::new("ssh");
        configure_remote_pty_environment(&mut command);

        assert_eq!(
            command.get_env("TERM"),
            Some(std::ffi::OsStr::new("xterm-256color"))
        );
    }

    #[test]
    fn remote_tmux_command_detects_creates_and_reconnects_without_relaunch() {
        let launch_argv = vec![
            "claude".to_string(),
            "--resume".to_string(),
            "session-123".to_string(),
        ];
        let command =
            build_remote_tmux_command("/srv/team's project", Some("claude"), Some(&launch_argv))
                .unwrap();

        assert!(command.contains("command -v tmux"));
        assert!(command.contains("tmux -L 'sessionatlas-v1' has-session"));
        assert!(command.contains("tmux -L 'sessionatlas-v1' -f /dev/null new-session -d"));
        assert!(command.contains("2>/dev/null || true"));
        assert!(command.contains("set-option -g prefix C-b"));
        assert!(command.contains("set-option -g prefix2 None"));
        assert!(command.contains("set-option -g assume-paste-time 0"));
        assert!(command.contains("exec tmux -L 'sessionatlas-v1' -f /dev/null attach-session"));
        assert!(command.contains("SessionAtlas requires tmux"));
        assert!(command.contains("/srv/team'\"'\"'s project"));
        assert_eq!(command.matches("claude --resume session-123").count(), 1);

        let shell_command =
            build_remote_tmux_command("~/projects/demo", Some("shell"), None).unwrap();
        assert!(shell_command.contains("exec \"$SHELL\" -l"));
        assert!(!shell_command.contains("--resume"));

        let format_input = vec!["custom-tool".to_string(), "#(touch marker)".to_string()];
        let escaped_formats = build_remote_tmux_command(
            "/srv/#(path probe)",
            Some("custom-tool"),
            Some(&format_input),
        )
        .unwrap();
        assert!(escaped_formats.contains("/srv/##(path probe)"));
        assert!(escaped_formats.contains("##(touch marker)"));
    }

    #[test]
    fn tmux_prompt_arguments_escape_tmux_expansion_and_reject_controls() {
        assert_eq!(
            tmux_quote_argument("/srv/#(touch marker)/team's $project/\"quoted\"\\path").unwrap(),
            "\"/srv/##(touch marker)/team's \\$project/\\\"quoted\\\"\\\\path\""
        );
        assert!(tmux_quote_argument("/srv/project\nnext").is_err());
    }

    #[test]
    fn remote_tmux_switch_builds_independent_create_and_switch_commands() {
        let launch_argv = vec![
            "codex".to_string(),
            "resume".to_string(),
            "session-456".to_string(),
        ];
        let (create, switch) = build_remote_tmux_prompt_commands(
            "/srv/team's $project",
            Some("codex"),
            Some(&launch_argv),
        )
        .unwrap();
        let expected_name =
            remote_tmux_session_name("/srv/team's $project", Some("codex")).unwrap();

        assert!(create.starts_with(&format!("new-session -d -s {expected_name}")));
        assert!(create.contains("-c \"/srv/team's \\$project\""));
        assert!(create.contains("codex resume session-456"));
        assert_eq!(switch, format!("switch-client -t {expected_name}"));
    }

    #[test]
    fn tmux_prompt_writer_emits_prefix_prompt_command_and_enter() {
        let mut bytes = Vec::<u8>::new();
        write_tmux_prompt_command(&mut bytes, "switch-client -t target").unwrap();
        assert_eq!(bytes, b"\x02:switch-client -t target\r");
        assert!(write_tmux_prompt_command(&mut bytes, "bad\ncommand").is_err());
    }

    #[test]
    fn remote_switch_rejects_local_and_different_server_ptys() {
        assert!(ensure_remote_server_matches(Some(7), 7).is_ok());
        assert!(ensure_remote_server_matches(Some(7), 8).is_err());
        assert!(ensure_remote_server_matches(None, 7).is_err());
        assert!(ensure_remote_server_matches(Some(7), 0).is_err());
    }

    #[test]
    fn browser_open_is_testable_without_starting_a_browser() {
        let runner = RecordingRunner {
            requests: RefCell::new(Vec::new()),
            spawn_requests: RefCell::new(Vec::new()),
            output: ProcessOutput {
                success: true,
                status_code: Some(0),
                stdout: Vec::new(),
                stderr: Vec::new(),
            },
        };

        open_external_url_with(&runner, "https://example.test/path").unwrap();

        let requests = runner.spawn_requests.borrow();
        assert_eq!(requests.len(), 1);
        assert!(requests[0]
            .args
            .contains(&std::ffi::OsString::from("https://example.test/path")));
        #[cfg(windows)]
        assert_eq!(
            requests[0].program,
            std::ffi::OsString::from("explorer.exe")
        );
        assert!(open_external_url_with(&runner, "file:///tmp/demo").is_err());
        assert!(
            open_external_url_with(&runner, "https://user:secret@example.test/private").is_err()
        );
    }

    #[test]
    fn custom_opener_keeps_the_project_path_as_one_process_argument() {
        let path = std::path::Path::new(r"C:\fixture workspace\repo & safe");
        let spec = build_generic_opener_spec("code --reuse-window \"{path}\"", path).unwrap();

        assert_eq!(spec.program, std::ffi::OsString::from("code"));
        assert_eq!(spec.args[0], std::ffi::OsString::from("--reuse-window"));
        assert_eq!(
            spec.args[1],
            std::ffi::OsString::from(r"C:\fixture workspace\repo & safe")
        );
        assert!(build_generic_opener_spec("cmd /c echo {path}", path).is_err());
        assert!(build_generic_opener_spec("code --folder={path}", path).is_err());
        assert!(build_generic_opener_spec("{path} --reuse-window", path).is_err());
    }

    #[test]
    fn remote_path_filter_excludes_only_tool_internal_directories() {
        assert!(is_remote_path_excluded(
            "/home/demo/.codex/sessions",
            "/home/demo"
        ));
        assert!(!is_remote_path_excluded(
            "/home/demo/projects/.codex-example",
            "/home/demo"
        ));
    }

    #[test]
    fn remote_scan_root_quoting_rejects_control_characters() {
        assert!(shell_quote_path("~/projects\nmalicious").is_err());
        assert_eq!(shell_quote_roots("~ ~/projects").unwrap(), "~ ~/'projects'");
        assert_eq!(
            shell_quote_path("/srv/alice's repo").unwrap(),
            "'/srv/alice'\"'\"'s repo'"
        );
    }

    #[test]
    fn remote_scan_command_resolves_worktree_roots_and_activity() {
        let command = build_remote_scan_command("~ ~/projects").unwrap();

        assert!(command.contains("-name .git -prune -exec"));
        assert!(!command.contains("-type d"));
        assert!(command.contains("candidate=${marker%/.git}"));
        assert!(command.contains("rev-parse --show-toplevel"));
        assert!(command.contains("log -1 --all --format=%ct"));
        assert!(command.contains("printf \"%s\\000%s\\000%s\\000\""));
    }

    #[test]
    fn remote_scan_output_deduplicates_overlapping_roots() {
        let stdout = b"/srv/projects/repo\x00main\x001720000000\x00\
/srv/projects/repo\x00\x001730000000\x00\
/srv/projects/worktree\nwith-newline\x00feature\x001710000000\x00\
/home/demo/.codex/internal\x00main\x001740000000\x00";

        let rows = parse_remote_scan_output(stdout, "/home/demo").unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, "/srv/projects/repo");
        assert_eq!(rows[0].branch.as_deref(), Some("main"));
        assert_eq!(rows[0].last_activity_epoch, Some(1_730_000_000));
        assert_eq!(rows[1].path, "/srv/projects/worktree\nwith-newline");
        assert_eq!(rows[1].branch.as_deref(), Some("feature"));
    }

    #[test]
    fn remote_scan_output_rejects_incomplete_records() {
        let error = parse_remote_scan_output(b"/srv/repo\0main\0", "/home/demo").unwrap_err();
        assert!(error.contains("incomplete record"));
    }

    #[test]
    fn remote_recency_uses_real_activity_without_rescan_churn() {
        let previous_scan_time = "2099-01-01T00:00:00Z";
        let tool_access = "2025-01-01T00:00:00Z";
        let first_seen = "2026-01-01T00:00:00Z";

        assert_eq!(
            remote_last_accessed_at(
                Some(1_704_067_200),
                Some(tool_access),
                Some(previous_scan_time),
                first_seen,
            ),
            tool_access
        );
        assert_eq!(
            remote_last_accessed_at(None, None, Some("2024-06-01T00:00:00Z"), first_seen),
            "2024-06-01T00:00:00Z"
        );
        assert_eq!(
            remote_last_accessed_at(None, None, None, first_seen),
            first_seen
        );
    }

    #[test]
    fn recording_remote_usage_updates_project_recency_atomically() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE remote_projects (
                    project_id TEXT PRIMARY KEY,
                    server_id INTEGER NOT NULL,
                    last_accessed_at TEXT NOT NULL
                 );
                 CREATE TABLE remote_tool_usages (
                    server_id INTEGER NOT NULL,
                    project_id TEXT NOT NULL,
                    tool_key TEXT NOT NULL,
                    tool_name TEXT NOT NULL,
                    last_used_at TEXT NOT NULL,
                    session_count INTEGER NOT NULL DEFAULT 0,
                    last_session_id TEXT,
                    PRIMARY KEY (server_id, project_id, tool_key)
                 );
                 INSERT INTO remote_projects
                    (project_id, server_id, last_accessed_at)
                 VALUES ('r1:fixture', 1, '2024-01-01T00:00:00Z');",
            )
            .unwrap();

        record_remote_tool_usage_at(
            &connection,
            1,
            "r1:fixture",
            "codex",
            "Codex",
            Some("session-1"),
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        record_remote_tool_usage_at(
            &connection,
            1,
            "r1:fixture",
            "codex",
            "Codex",
            Some("session-2"),
            "2025-02-01T00:00:00Z",
        )
        .unwrap();

        let accessed_at: String = connection
            .query_row(
                "SELECT last_accessed_at FROM remote_projects WHERE project_id = 'r1:fixture'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let usage: (String, i64, String) = connection
            .query_row(
                "SELECT last_used_at, session_count, last_session_id
                 FROM remote_tool_usages
                 WHERE project_id = 'r1:fixture' AND tool_key = 'codex'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();

        assert_eq!(accessed_at, "2025-02-01T00:00:00Z");
        assert_eq!(
            usage,
            (
                "2025-02-01T00:00:00Z".to_string(),
                2,
                "session-2".to_string()
            )
        );
        assert!(record_remote_tool_usage_at(
            &connection,
            1,
            "r1:missing",
            "codex",
            "Codex",
            None,
            "2025-03-01T00:00:00Z",
        )
        .is_err());
    }

    #[test]
    fn corrected_remote_project_ids_keep_legacy_tool_history() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE remote_tool_usages (
                    server_id INTEGER NOT NULL,
                    project_id TEXT NOT NULL,
                    tool_key TEXT NOT NULL,
                    tool_name TEXT NOT NULL,
                    last_used_at TEXT NOT NULL,
                    session_count INTEGER NOT NULL DEFAULT 0,
                    last_session_id TEXT,
                    PRIMARY KEY (server_id, project_id, tool_key)
                 );",
            )
            .unwrap();
        let project_id = remote_project_id(1, "/srv/repo");
        let legacy_project_id = remote_project_id(1, "/srv/repo/.git");
        connection
            .execute(
                "INSERT INTO remote_tool_usages VALUES
                    (1, ?1, 'codex', 'Codex', '2025-01-01T00:00:00Z', 1, 'new-session'),
                    (1, ?2, 'codex', 'Codex', '2025-02-01T00:00:00Z', 2, 'legacy-session')",
                params![project_id, legacy_project_id],
            )
            .unwrap();

        migrate_legacy_remote_tool_usages(&connection, 1, &legacy_project_id, &project_id).unwrap();

        let usage: (String, i64, String) = connection
            .query_row(
                "SELECT last_used_at, session_count, last_session_id
                 FROM remote_tool_usages
                 WHERE server_id = 1 AND project_id = ?1 AND tool_key = 'codex'",
                params![project_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        let legacy_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM remote_tool_usages
                 WHERE server_id = 1 AND project_id = ?1",
                params![legacy_project_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(
            usage,
            (
                "2025-02-01T00:00:00Z".to_string(),
                3,
                "legacy-session".to_string()
            )
        );
        assert_eq!(legacy_count, 0);
    }

    #[test]
    fn custom_tool_launch_uses_configured_command_instead_of_key() {
        let config: CliConfigFile = serde_json::from_str(
            r#"{
                "CustomTools": [{
                    "Key": "friendly-key",
                    "CliCommand": "fixture-cli --profile \"safe profile\"",
                    "IsEnabled": true
                }]
            }"#,
        )
        .unwrap();

        let args =
            resolve_tool_launch_argv_from_config("friendly-key", Some("session-123"), &config)
                .unwrap();
        assert_eq!(
            args,
            vec![
                "fixture-cli",
                "--profile",
                "safe profile",
                "--resume",
                "session-123"
            ]
        );
        assert!(!args.iter().any(|argument| argument == "friendly-key"));
    }

    #[test]
    fn custom_tool_launch_rejects_disabled_unknown_and_shell_commands() {
        let config = CliConfigFile {
            custom_tools: vec![
                CliToolConfig {
                    key: "disabled".to_string(),
                    cli_command: "fixture-cli".to_string(),
                    is_enabled: false,
                },
                CliToolConfig {
                    key: "unsafe".to_string(),
                    cli_command: "cmd.exe /C calc".to_string(),
                    is_enabled: true,
                },
            ],
        };

        assert!(resolve_tool_launch_argv_from_config("disabled", None, &config).is_err());
        assert!(resolve_tool_launch_argv_from_config("missing", None, &config).is_err());
        assert!(resolve_tool_launch_argv_from_config("unsafe", None, &config).is_err());
    }

    #[test]
    fn builtin_tool_keys_are_canonicalized_before_launch() {
        let args = resolve_tool_launch_argv_from_config(
            "CoDeX",
            Some("session-123"),
            &CliConfigFile::default(),
        )
        .unwrap();
        assert_eq!(args, vec!["codex", "--resume", "session-123"]);
    }

    #[test]
    fn fts_query_quotes_tokens_and_ignores_operator_only_input() {
        let pattern = build_fts_prefix_query("alpha-beta OR gamma").unwrap();
        assert_eq!(
            pattern,
            "\"alpha\"* AND \"beta\"* AND \"OR\"* AND \"gamma\"*"
        );
        assert_eq!(build_fts_prefix_query("\" * -"), None);

        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE VIRTUAL TABLE projects_fts USING fts5(name, path);
                 INSERT INTO projects_fts (name, path)
                 VALUES ('alpha-beta-or-gamma', '/work/alpha-beta-or-gamma');",
            )
            .unwrap();
        let matches: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM projects_fts WHERE projects_fts MATCH ?1",
                params![pattern],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(matches, 1);
    }
}

/// Deterministic isolated tests for the R11 in-process local scan. They drive
/// `run_scan_with_scanners` with fake scanners against a throwaway temporary
/// home so no test reads a real tool data directory, launches an external
/// command, spawns a subprocess, or mutates the real `~/.sessionatlas`.
/// `spawn_blocking` stays structural in the `scan_projects` wrapper — the pure
/// synchronous scan logic below is what these tests exercise.
#[cfg(test)]
mod local_scan_tests {
    use super::*;
    use sessionatlas_core::model::{Project as CoreProject, ToolUsage as CoreUsage};
    use sessionatlas_core::scanner::{ScanOutcome, ScannedProject};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime};

    static SCAN_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Disposable `<temp>/sessionatlas-tauri-scan-<pid>-<ns>-<n>` root. Removed
    /// on drop; never points at the real user home.
    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let nonce = SCAN_TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
            let name = format!(
                "sessionatlas-tauri-scan-{}-{}-{nonce}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(name);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        /// The production layout: `<root>/.sessionatlas/index.db`.
        fn db(&self) -> PathBuf {
            self.0.join(".sessionatlas").join("index.db")
        }

        /// Whether any `.sessionatlas` artifact exists yet.
        fn data_created(&self) -> bool {
            self.0.join(".sessionatlas").exists()
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn abs_path(segments: &[&str]) -> String {
        if cfg!(windows) {
            format!("C:\\{}", segments.join("\\"))
        } else {
            format!("/{}", segments.join("/"))
        }
    }

    fn now_minus(secs: u64) -> SystemTime {
        SystemTime::now() - Duration::from_secs(secs)
    }

    fn core_usage(key: &str, last_used: SystemTime, count: i32) -> CoreUsage {
        CoreUsage {
            tool_name: key.to_string(),
            tool_key: key.to_string(),
            last_used_at: last_used.into(),
            session_count: count,
            last_session_id: None,
        }
    }

    fn core_project(path_text: &str, id: &str, usages: &[CoreUsage]) -> CoreProject {
        let last_accessed = usages.iter().map(|usage| usage.last_used_at).max().unwrap();
        CoreProject {
            id: id.to_string(),
            path: path_text.to_string(),
            last_accessed_at: last_accessed,
            tool_usages: usages.to_vec(),
            ..CoreProject::default()
        }
    }

    fn scanned_project(
        path_text: &str,
        last_accessed: SystemTime,
        session_id: &str,
    ) -> ScannedProject {
        ScannedProject {
            path: path_text.to_string(),
            last_accessed_at: last_accessed.into(),
            session_id: Some(session_id.to_string()),
            git_branch: None,
        }
    }

    /// Injected fake scanner: returns a canned outcome (or panics) without ever
    /// touching a real tool data directory or launching an AI CLI.
    struct FakeScanner {
        key: &'static str,
        name: &'static str,
        outcome: ScanOutcome,
        panics: bool,
    }

    impl FakeScanner {
        fn succeeded(key: &'static str, projects: Vec<ScannedProject>) -> Self {
            Self {
                key,
                name: key,
                outcome: ScanOutcome::succeeded(projects, []),
                panics: false,
            }
        }

        fn failed(key: &'static str) -> Self {
            Self {
                key,
                name: key,
                outcome: ScanOutcome::failed([ScanDiagnostic::new(
                    key,
                    ScanDiagnosticSeverity::Error,
                    "source_read_failed",
                    "unreadable source",
                )]),
                panics: false,
            }
        }

        fn unavailable(key: &'static str) -> Self {
            Self {
                key,
                name: key,
                outcome: ScanOutcome::unavailable([ScanDiagnostic::new(
                    key,
                    ScanDiagnosticSeverity::Info,
                    "source_unavailable",
                    "no source",
                )]),
                panics: false,
            }
        }

        fn panics(key: &'static str) -> Self {
            Self {
                key,
                name: key,
                outcome: ScanOutcome::unavailable([]),
                panics: true,
            }
        }
    }

    impl Scanner for FakeScanner {
        fn tool_key(&self) -> &str {
            self.key
        }

        fn tool_name(&self) -> &str {
            self.name
        }

        fn is_available(&self) -> bool {
            true
        }

        fn scan(&self) -> ScanOutcome {
            if self.panics {
                panic!("injected fake scanner panic");
            }
            self.outcome.clone()
        }
    }

    fn boxed(scanner: FakeScanner) -> Box<dyn Scanner> {
        Box::new(scanner)
    }

    /// Seeds `index.db` with the given tool snapshots through the real store.
    fn seed(db_path: &Path, projects: &[CoreProject], keys: &[&str]) {
        let mut store = SqliteStore::new(db_path).unwrap();
        store.replace_tool_snapshots(projects, keys).unwrap();
    }

    fn read_projects(db_path: &Path) -> Vec<CoreProject> {
        let store = SqliteStore::new(db_path).unwrap();
        store.list_projects(None, None, 10_000).unwrap()
    }

    fn project_paths(db_path: &Path) -> Vec<String> {
        read_projects(db_path)
            .into_iter()
            .map(|project| project.path)
            .collect()
    }

    #[test]
    fn scan_success_only_replaces_declared_tool_snapshots() {
        let dir = TestDir::new();
        seed(
            &dir.db(),
            &[
                core_project(
                    &abs_path(&["work", "old-codex"]),
                    "old-codex-id",
                    &[core_usage("codex", now_minus(7200), 3)],
                ),
                core_project(
                    &abs_path(&["work", "old-claude"]),
                    "old-claude-id",
                    &[core_usage("claude", now_minus(7200), 2)],
                ),
            ],
            &["codex", "claude"],
        );

        let scanners: Vec<Box<dyn Scanner>> = vec![
            boxed(FakeScanner::succeeded(
                "codex",
                vec![scanned_project(
                    &abs_path(&["work", "new-codex"]),
                    now_minus(60),
                    "codex-s2",
                )],
            )),
            boxed(FakeScanner::unavailable("claude")),
        ];
        let count = run_scan_with_scanners(&dir.db(), &scanners, &[]).unwrap();
        assert_eq!(count, 2);

        let paths = project_paths(&dir.db());
        assert!(
            paths.iter().any(|p| p.ends_with("new-codex")),
            "codex's new snapshot must be written: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.ends_with("old-codex")),
            "codex's old snapshot must be cleared: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.ends_with("old-claude")),
            "claude's snapshot must be preserved: {paths:?}"
        );
    }

    #[test]
    fn scan_failed_and_unavailable_tools_preserve_existing_rows() {
        let dir = TestDir::new();
        seed(
            &dir.db(),
            &[
                core_project(
                    &abs_path(&["work", "codex"]),
                    "codex-id",
                    &[core_usage("codex", now_minus(3600), 3)],
                ),
                core_project(
                    &abs_path(&["work", "claude"]),
                    "claude-id",
                    &[core_usage("claude", now_minus(3600), 1)],
                ),
            ],
            &["codex", "claude"],
        );

        let scanners: Vec<Box<dyn Scanner>> = vec![
            boxed(FakeScanner::failed("codex")),
            boxed(FakeScanner::unavailable("claude")),
        ];
        let error = run_scan_with_scanners(&dir.db(), &scanners, &[]).unwrap_err();
        assert!(error.contains("trustworthy snapshot"), "{error}");

        let paths = project_paths(&dir.db());
        assert_eq!(paths.len(), 2, "failed tools must preserve old rows");
        assert!(paths.iter().any(|p| p.ends_with("codex")), "{paths:?}");
        assert!(paths.iter().any(|p| p.ends_with("claude")), "{paths:?}");
    }

    #[test]
    fn scan_zero_success_never_creates_the_index() {
        let dir = TestDir::new();
        let scanners: Vec<Box<dyn Scanner>> = vec![
            boxed(FakeScanner::failed("codex")),
            boxed(FakeScanner::unavailable("claude")),
        ];
        let error = run_scan_with_scanners(&dir.db(), &scanners, &[]).unwrap_err();
        assert!(error.contains("trustworthy snapshot"), "{error}");
        assert!(
            !dir.data_created(),
            "zero successful tools must not create the data directory"
        );
        assert!(!dir.db().exists());
    }

    #[test]
    fn scan_creates_a_missing_index_and_returns_project_count() {
        let dir = TestDir::new();
        let scanners: Vec<Box<dyn Scanner>> = vec![boxed(FakeScanner::succeeded(
            "codex",
            vec![
                scanned_project(&abs_path(&["work", "a"]), now_minus(300), "s1"),
                scanned_project(&abs_path(&["work", "b"]), now_minus(600), "s2"),
            ],
        ))];
        let count = run_scan_with_scanners(&dir.db(), &scanners, &[]).unwrap();
        assert_eq!(count, 2);
        assert!(dir.db().is_file(), "the missing index must be created");
        assert_eq!(read_projects(&dir.db()).len(), 2);
    }

    #[test]
    fn existing_index_stays_queryable_after_scan() {
        let dir = TestDir::new();
        seed(
            &dir.db(),
            &[core_project(
                &abs_path(&["work", "old"]),
                "old-id",
                &[core_usage("claude", now_minus(3600), 5)],
            )],
            &["claude"],
        );

        let scanners: Vec<Box<dyn Scanner>> = vec![boxed(FakeScanner::succeeded(
            "codex",
            vec![scanned_project(
                &abs_path(&["work", "new"]),
                now_minus(60),
                "s1",
            )],
        ))];
        let count = run_scan_with_scanners(&dir.db(), &scanners, &[]).unwrap();
        assert_eq!(count, 2);

        // A fresh read-only connection (like the console's reader) can still
        // query both the preserved and the newly written rows after the scan.
        let connection = rusqlite::Connection::open_with_flags(
            dir.db(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let rows: Vec<(String, i64)> = connection
            .prepare(
                "SELECT path,
                        (SELECT COUNT(*) FROM tool_usages u WHERE u.project_id = projects.id)
                 FROM projects ORDER BY path",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter().any(|(path, _)| path.ends_with("old")),
            "{rows:?}"
        );
        assert!(
            rows.iter().any(|(path, _)| path.ends_with("new")),
            "{rows:?}"
        );
    }

    #[test]
    fn scan_scanner_panic_is_failure_and_preserves_old_data() {
        let dir = TestDir::new();
        seed(
            &dir.db(),
            &[core_project(
                &abs_path(&["work", "keep"]),
                "keep-id",
                &[core_usage("codex", now_minus(3600), 2)],
            )],
            &["codex"],
        );

        let scanners: Vec<Box<dyn Scanner>> = vec![boxed(FakeScanner::panics("codex"))];
        let error = run_scan_with_scanners(&dir.db(), &scanners, &[]).unwrap_err();
        assert!(error.contains("trustworthy snapshot"), "{error}");
        let projects = read_projects(&dir.db());
        assert_eq!(projects.len(), 1, "panic must preserve the old snapshot");
        assert!(projects[0].path.ends_with("keep"));
    }

    #[test]
    fn sanitize_scan_error_strips_control_characters() {
        // The sanitizer never receives tool data (only constant text + counts),
        // but must still guarantee no terminal escapes can leak to the frontend.
        let message = sanitize_scan_error(3);
        assert!(message.contains("trustworthy snapshot"), "{message}");
        assert!(message.contains('3'), "{message}");
        // Newline/tab separators are intentional; any other control character
        // (ESC, BEL, ...) must have been replaced.
        assert!(
            message.chars().all(|character| {
                !character.is_control() || matches!(character, '\n' | '\r' | '\t')
            }),
            "{message:?}"
        );
    }
}
