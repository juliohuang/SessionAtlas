//! SQLite index reader/writer: schema validation, creation and migration, path
//! identity, snapshot replacement, FTS5, list/search/exact lookup, and session
//! recording.
//!
//! The store owns `index.db`
//! and writes exactly the schema the Tauri console queries read-only:
//! `projects`, `tool_usages`, `sessions` plus the FTS5 `projects_fts` table.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Transaction};

use crate::content_index::{collect_project_content, ContentFingerprint, ContentIndexOptions};
use crate::model::{project_path_missing, Project, Session, ToolUsage};
use crate::path;
use crate::private_fs;

/// Bump whenever content sanitization changes in a way that requires every
/// unchanged source file to be reopened and all stored previews/FTS terms to
/// be rebuilt.
pub const CONTENT_INDEX_SANITIZER_VERSION: i64 = 2;

/// Version of the on-disk `index.db` schema.
pub const INDEX_SCHEMA_VERSION: i64 = 1;

/// Result alias for store operations.
pub type Result<T> = std::result::Result<T, StoreError>;

/// Errors surfaced by the SQLite store.
#[derive(Debug)]
pub enum StoreError {
    /// Underlying SQLite failure.
    Sql(rusqlite::Error),
    /// Filesystem failure while preparing the database directory.
    Io(std::io::Error),
    /// `replace_tool_snapshots` requires at least one scanned tool key.
    ScannedToolKeysEmpty,
    /// A scanned tool key was blank/whitespace-only.
    EmptyScannedToolKey,
    /// A snapshot project path was blank.
    EmptyProjectPath,
    /// Two snapshot projects normalized to the same native path.
    DuplicateProjectPath(String),
    /// A snapshot project carried no tool usages.
    NoToolUsages(String),
    /// A usage references a tool not declared as successfully scanned.
    UndeclaredUsageTool { project: String, tool: String },
    /// A project declared the same tool more than once (case-insensitive).
    DuplicateUsageTool { project: String, tool: String },
    /// A usage reported a negative session count.
    NegativeSessionCount { project: String, tool: String },
    /// A project/session path was not a valid absolute native path.
    InvalidProjectPath(String),
    /// A session path was not a valid absolute native path.
    InvalidSessionPath(String),
    /// A limit argument fell outside its documented range.
    InvalidLimit,
    /// A stored value could not be converted to the model type.
    CorruptRow(String),
    /// A stored timestamp could not be parsed as RFC 3339.
    BadTimestamp(String),
    /// The database schema version is not supported by this build.
    UnsupportedSchemaVersion { found: i64, supported: i64 },
    /// The database schema metadata is malformed.
    InvalidSchema(String),
    /// A required table or column is missing or has an invalid shape.
    SchemaInvariant(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Sql(error) => write!(f, "SQLite error: {error}"),
            StoreError::Io(error) => write!(f, "I/O error: {error}"),
            StoreError::ScannedToolKeysEmpty => {
                write!(f, "At least one successfully scanned tool key is required.")
            }
            StoreError::EmptyScannedToolKey => {
                write!(f, "Scanned tool keys cannot be empty.")
            }
            StoreError::EmptyProjectPath => {
                write!(f, "Snapshot project paths cannot be empty.")
            }
            StoreError::DuplicateProjectPath(full_path) => {
                write!(f, "Snapshot contains duplicate project path: {full_path}")
            }
            StoreError::NoToolUsages(full_path) => {
                write!(f, "Snapshot project has no tool usages: {full_path}")
            }
            StoreError::UndeclaredUsageTool { project, tool } => write!(
                f,
                "Usage tool '{tool}' is not declared as successfully scanned: {project}"
            ),
            StoreError::DuplicateUsageTool { project, tool } => write!(
                f,
                "Project contains duplicate usage for tool '{tool}': {project}"
            ),
            StoreError::NegativeSessionCount { project, tool } => write!(
                f,
                "Session count cannot be negative for tool '{tool}': {project}"
            ),
            StoreError::InvalidProjectPath(candidate) => {
                write!(f, "Project path must be a valid absolute path: {candidate}")
            }
            StoreError::InvalidSessionPath(candidate) => {
                write!(f, "Session path must be a valid absolute path: {candidate}")
            }
            StoreError::InvalidLimit => write!(f, "Limit is outside its documented range."),
            StoreError::CorruptRow(detail) => {
                write!(f, "Stored row is corrupt: {detail}")
            }
            StoreError::BadTimestamp(text) => {
                write!(f, "Stored timestamp is not valid RFC 3339: {text}")
            }
            StoreError::UnsupportedSchemaVersion { found, supported } => write!(
                f,
                "Unsupported index schema version {found}; this build supports versions 0 through {supported}."
            ),
            StoreError::InvalidSchema(detail) => write!(f, "Invalid index schema: {detail}"),
            StoreError::SchemaInvariant(detail) => {
                write!(f, "Index schema invariant failed: {detail}")
            }
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Sql(error) => Some(error),
            StoreError::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        StoreError::Sql(error)
    }
}

impl From<std::io::Error> for StoreError {
    fn from(error: std::io::Error) -> Self {
        StoreError::Io(error)
    }
}

/// The `index.db` schema, mirroring `SqliteStore.InitializeSchema`.
const SCHEMA_SQL: &str = r#"
    CREATE TABLE IF NOT EXISTS sessionatlas_meta (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS projects (
        id TEXT PRIMARY KEY,
        path TEXT NOT NULL UNIQUE,
        last_accessed_at TEXT,
        first_seen_at TEXT,
        git_branch TEXT,
        git_remote_url TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_projects_path ON projects(path);
    CREATE INDEX IF NOT EXISTS idx_projects_last_accessed ON projects(last_accessed_at);

    CREATE VIRTUAL TABLE IF NOT EXISTS projects_fts USING fts5(name, path);

    CREATE TABLE IF NOT EXISTS project_content_files (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        project_id TEXT NOT NULL,
        relative_path TEXT NOT NULL,
        modified_ns INTEGER NOT NULL,
        file_size INTEGER NOT NULL,
        indexed_bytes INTEGER NOT NULL,
        compressed_preview BLOB NOT NULL,
        UNIQUE(project_id, relative_path),
        FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
    );
    CREATE INDEX IF NOT EXISTS idx_content_files_project
        ON project_content_files(project_id);
    CREATE VIRTUAL TABLE IF NOT EXISTS project_content_fts USING fts5(
        relative_path,
        body,
        content='',
        contentless_delete=1,
        tokenize='unicode61 remove_diacritics 2'
    );
    CREATE TABLE IF NOT EXISTS project_content_status (
        project_id TEXT PRIMARY KEY,
        indexed_files INTEGER NOT NULL,
        indexed_bytes INTEGER NOT NULL,
        skipped_files INTEGER NOT NULL,
        truncated INTEGER NOT NULL,
        FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
    );

    CREATE TABLE IF NOT EXISTS tool_usages (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        project_id TEXT NOT NULL,
        tool_name TEXT NOT NULL,
        tool_key TEXT NOT NULL,
        last_used_at TEXT,
        session_count INTEGER DEFAULT 1,
        last_session_id TEXT,
        FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
    );
    CREATE INDEX IF NOT EXISTS idx_usages_project ON tool_usages(project_id);
    CREATE INDEX IF NOT EXISTS idx_usages_tool ON tool_usages(tool_key);

    CREATE TABLE IF NOT EXISTS sessions (
        id TEXT PRIMARY KEY,
        project_path TEXT NOT NULL,
        tool_key TEXT NOT NULL,
        tool_name TEXT NOT NULL,
        started_at TEXT,
        ended_at TEXT,
        session_id_from_tool TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at);
"#;

/// Case-insensitive tool-key index kept in sync by both write paths.
const TOOL_KEY_NOCASE_INDEX_SQL: &str = r#"
    CREATE INDEX IF NOT EXISTS idx_usages_tool_nocase
    ON tool_usages(tool_key COLLATE NOCASE, project_id)
"#;

/// Collapses legacy duplicate usages (one row per `(project, tool)`) and then
/// enforces that identity with a unique index. Mirrors
/// `SqliteStore.MigrateToolUsageIdentity`.
const MIGRATE_TOOL_USAGE_IDENTITY_SQL: &str = r#"
    DELETE FROM tool_usages
    WHERE NOT EXISTS (
        SELECT 1 FROM projects p WHERE p.id = tool_usages.project_id
    );

    WITH ranked AS (
        SELECT
            id,
            ROW_NUMBER() OVER (
                PARTITION BY project_id, tool_key COLLATE NOCASE
                ORDER BY last_used_at DESC, id DESC
            ) AS rank,
            MAX(session_count) OVER (
                PARTITION BY project_id, tool_key COLLATE NOCASE
            ) AS max_session_count
        FROM tool_usages
    )
    UPDATE tool_usages
    SET session_count = (
        SELECT max_session_count
        FROM ranked
        WHERE ranked.id = tool_usages.id
    )
    WHERE id IN (SELECT id FROM ranked WHERE rank = 1);

    WITH ranked AS (
        SELECT
            id,
            ROW_NUMBER() OVER (
                PARTITION BY project_id, tool_key COLLATE NOCASE
                ORDER BY last_used_at DESC, id DESC
            ) AS rank
        FROM tool_usages
    )
    DELETE FROM tool_usages
    WHERE id IN (SELECT id FROM ranked WHERE rank > 1);

    CREATE UNIQUE INDEX IF NOT EXISTS idx_usages_project_tool
    ON tool_usages(project_id, tool_key COLLATE NOCASE);
"#;

/// Per-snapshot temp tables and their reset.
const PREPARE_SNAPSHOT_TABLES_SQL: &str = r#"
    CREATE TEMP TABLE IF NOT EXISTS scanned_tool_keys (
        tool_key TEXT PRIMARY KEY COLLATE NOCASE
    ) WITHOUT ROWID;
    CREATE TEMP TABLE IF NOT EXISTS snapshot_tool_usages (
        project_id TEXT NOT NULL,
        tool_key TEXT NOT NULL COLLATE NOCASE,
        PRIMARY KEY (project_id, tool_key)
    ) WITHOUT ROWID;
    DELETE FROM scanned_tool_keys;
    DELETE FROM snapshot_tool_usages;
"#;

/// Removes snapshot rows that vanished for a scanned tool, then drops projects
/// left without any usage. Mirrors `SqliteStore.DeleteStaleSnapshotRows`.
const DELETE_STALE_SNAPSHOT_ROWS_SQL: &str = r#"
    DELETE FROM tool_usages
    WHERE EXISTS (
        SELECT 1
        FROM scanned_tool_keys scanned
        WHERE scanned.tool_key = tool_usages.tool_key COLLATE NOCASE
    )
    AND NOT EXISTS (
        SELECT 1
        FROM snapshot_tool_usages snapshot
        WHERE snapshot.project_id = tool_usages.project_id
          AND snapshot.tool_key = tool_usages.tool_key COLLATE NOCASE
    );

    DELETE FROM projects
    WHERE NOT EXISTS (
        SELECT 1 FROM tool_usages usage WHERE usage.project_id = projects.id
    );
"#;

/// Re-derives project activity from the merged usage rows.
const RECOMPUTE_PROJECT_ACTIVITY_SQL: &str = r#"
    UPDATE projects
    SET last_accessed_at = (
        SELECT MAX(usage.last_used_at)
        FROM tool_usages usage
        WHERE usage.project_id = projects.id
    )
"#;

/// Upsert for a single snapshot usage; the versioned v0 -> v1 migration creates
/// the unique identity index when needed.
const USAGE_UPSERT_SQL: &str = r#"
    INSERT INTO tool_usages
        (project_id, tool_name, tool_key, last_used_at, session_count, last_session_id)
    VALUES
        (?1, ?2, ?3, ?4, ?5, ?6)
    ON CONFLICT DO UPDATE SET
        tool_name = excluded.tool_name,
        tool_key = excluded.tool_key,
        last_used_at = excluded.last_used_at,
        session_count = excluded.session_count,
        last_session_id = excluded.last_session_id
"#;

/// Column list shared by every project read path.
const PROJECT_COLUMNS: &str =
    "id, path, last_accessed_at, first_seen_at, git_branch, git_remote_url";

/// The SQLite store. `new` creates the schema and runs migrations; the
/// connection is single-threaded and owned by this struct.
pub struct SqliteStore {
    connection: Connection,
}

/// Read-only view of an existing index database.
///
/// Unlike [`SqliteStore`], opening this type never creates directories/files,
/// initializes or migrates schema, repairs permissions, or rebuilds FTS. The
/// database must already exist and be complete enough for the requested query;
/// missing or malformed required structures are returned as schema-invariant
/// errors instead of being repaired.
pub struct IndexReader {
    connection: Connection,
}

impl IndexReader {
    /// Opens an existing index with SQLite's read-only flag and query-only
    /// protection. This is intentionally a separate facade from the writer so
    /// callers cannot accidentally obtain any mutation methods.
    pub fn open_existing(database_path: impl AsRef<Path>) -> Result<Self> {
        let connection =
            Connection::open_with_flags(database_path.as_ref(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA query_only = ON")?;
        validate_index_schema(&connection)?;
        Ok(Self { connection })
    }

    /// Lists projects using the same query and rendering model as the writer.
    pub fn list_projects(
        &self,
        search: Option<&str>,
        tool_key: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Project>> {
        list_projects_from_connection(&self.connection, search, tool_key, limit)
    }

    /// Returns the most recent sessions ordered by start time descending.
    pub fn get_recent_sessions(&self, limit: usize) -> Result<Vec<Session>> {
        get_recent_sessions_from_connection(&self.connection, limit)
    }
}

/// Summary of one incremental content-index refresh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContentIndexStats {
    pub projects_scanned: usize,
    pub files_indexed: usize,
    pub files_reused: usize,
    pub files_removed: usize,
    pub files_skipped: usize,
    pub indexed_bytes: usize,
    pub truncated_projects: usize,
}

impl SqliteStore {
    /// Opens (creating when missing) the database at `database_path`, creates
    /// parent directories, validates the current schema, and runs any required
    /// versioned migration. FTS is rebuilt only by a migration that requires
    /// it. No other business file is created or touched.
    pub fn new(database_path: impl AsRef<Path>) -> Result<Self> {
        let path = database_path.as_ref();
        if path.exists() {
            let read_only = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let schema_version = read_index_schema_version(&read_only)?;
            ensure_supported_schema_version(schema_version)?;
        }
        private_fs::prepare_private_database(path)?;
        let mut connection = Connection::open(path)?;
        connection.execute_batch("PRAGMA foreign_keys = ON")?;
        initialize_schema(&mut connection)?;
        private_fs::harden_existing_private_file(path)?;
        private_fs::harden_sqlite_sidecars(path)?;
        Ok(Self { connection })
    }

    /// Atomically inserts or updates one project (legacy upsert path) together
    /// with its FTS row and tool usages.
    pub fn upsert_project(&mut self, project: &Project) -> Result<()> {
        let normalized_path = normalize_project_path(&project.path)?;
        let tx = self.connection.transaction()?;
        let path_where = path_equality_clause("p.");

        let existing: Option<(String, i64)> = tx
            .query_row(
                &format!("SELECT p.id, p.rowid FROM projects p WHERE {path_where} LIMIT 1"),
                params![normalized_path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let (actual_id, rowid) = match existing {
            Some((id, rowid)) => {
                tx.execute(
                    "UPDATE projects SET
                        last_accessed_at = ?1,
                        git_branch = COALESCE(?2, git_branch),
                        git_remote_url = COALESCE(?3, git_remote_url)
                     WHERE id = ?4",
                    params![
                        timestamp(project.last_accessed_at),
                        project.git_branch,
                        project.git_remote_url,
                        id
                    ],
                )?;
                (id, rowid)
            }
            None => {
                let id = if project.id.trim().is_empty() {
                    uuid::Uuid::new_v4().simple().to_string()
                } else {
                    project.id.clone()
                };
                tx.execute(
                    "INSERT INTO projects
                        (id, path, last_accessed_at, first_seen_at, git_branch, git_remote_url)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        id,
                        normalized_path,
                        timestamp(project.last_accessed_at),
                        timestamp(project.first_seen_at),
                        project.git_branch,
                        project.git_remote_url
                    ],
                )?;
                let rowid = tx.query_row(
                    "SELECT rowid FROM projects WHERE id = ?1",
                    params![id],
                    |row| row.get(0),
                )?;
                (id, rowid)
            }
        };

        sync_fts_row(&tx, rowid, &normalized_path)?;
        for usage in &project.tool_usages {
            tx.execute(
                USAGE_UPSERT_SQL,
                params![
                    actual_id,
                    usage.tool_name,
                    usage.tool_key,
                    timestamp(usage.last_used_at),
                    usage.session_count,
                    usage.last_session_id
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Atomically replaces the snapshots for the declared successfully scanned
    /// tools. Tools omitted from `scanned_tool_keys` are preserved, while a
    /// declared tool with no incoming rows is cleared; projects left without
    /// any usage are removed. Any validation or SQLite failure rolls the whole
    /// transaction back.
    pub fn replace_tool_snapshots(
        &mut self,
        projects: &[Project],
        scanned_tool_keys: &[&str],
    ) -> Result<()> {
        let tool_keys = validate_snapshot(projects, scanned_tool_keys)?;
        let tx = self.connection.transaction()?;

        tx.execute_batch(PREPARE_SNAPSHOT_TABLES_SQL)?;
        for tool_key in &tool_keys {
            tx.execute(
                "INSERT INTO scanned_tool_keys (tool_key) VALUES (?1)",
                params![tool_key],
            )?;
        }

        for project in projects {
            let actual_project_id = upsert_snapshot_project(&tx, project)?;
            for usage in &project.tool_usages {
                tx.execute(
                    USAGE_UPSERT_SQL,
                    params![
                        actual_project_id,
                        usage.tool_name,
                        usage.tool_key,
                        timestamp(usage.last_used_at),
                        usage.session_count,
                        usage.last_session_id
                    ],
                )?;
                tx.execute(
                    "INSERT INTO snapshot_tool_usages (project_id, tool_key)
                     VALUES (?1, ?2)",
                    params![actual_project_id, usage.tool_key],
                )?;
            }
        }

        tx.execute_batch(DELETE_STALE_SNAPSHOT_ROWS_SQL)?;
        tx.execute_batch(RECOMPUTE_PROJECT_ACTIVITY_SQL)?;
        rebuild_fts(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// Rebuilds the FTS5 index from the current `projects` rows.
    pub fn rebuild_search_index(&mut self) -> Result<()> {
        let tx = self.connection.transaction()?;
        rebuild_fts(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// Incrementally refreshes the bounded source/document index for every
    /// local project. Unchanged files are identified by `(mtime_ns, size)` and
    /// never opened. Raw bodies are inserted only into the contentless FTS5
    /// table; SQLite retains terms, while result subtitles use a small LZ4
    /// preview stored in `project_content_files`.
    pub fn refresh_project_content_index(&mut self) -> Result<ContentIndexStats> {
        self.refresh_project_content_index_with(ContentIndexOptions::default())
    }

    /// Same as [`Self::refresh_project_content_index`] with injectable limits
    /// for deterministic tests.
    pub fn refresh_project_content_index_with(
        &mut self,
        options: ContentIndexOptions,
    ) -> Result<ContentIndexStats> {
        let projects: Vec<(String, String)> = {
            let mut stmt = self
                .connection
                .prepare("SELECT id, path FROM projects ORDER BY rowid")?;
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        // Cascading deletes remove metadata rows when projects disappear, but
        // the contentless virtual table has no foreign key of its own.
        self.connection.execute(
            "DELETE FROM project_content_fts
             WHERE rowid NOT IN (SELECT id FROM project_content_files)",
            [],
        )?;

        let mut stats = ContentIndexStats::default();
        for (project_id, project_path) in projects {
            let known: HashMap<String, ContentFingerprint> = {
                let mut stmt = self.connection.prepare(
                    "SELECT relative_path, modified_ns, file_size
                     FROM project_content_files
                     WHERE project_id = ?1",
                )?;
                let rows = stmt
                    .query_map(params![project_id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            ContentFingerprint {
                                modified_ns: row.get(1)?,
                                file_size: row.get(2)?,
                            },
                        ))
                    })?
                    .collect::<std::result::Result<HashMap<_, _>, _>>()?;
                rows
            };
            let collection = collect_project_content(Path::new(&project_path), &known, options)?;
            let tx = self.connection.transaction()?;

            for document in &collection.documents {
                let existing_id = tx
                    .query_row(
                        "SELECT id FROM project_content_files
                         WHERE project_id = ?1 AND relative_path = ?2",
                        params![project_id, document.relative_path],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?;
                let rowid = if let Some(rowid) = existing_id {
                    tx.execute(
                        "DELETE FROM project_content_fts WHERE rowid = ?1",
                        params![rowid],
                    )?;
                    tx.execute(
                        "UPDATE project_content_files
                         SET modified_ns = ?1, file_size = ?2, indexed_bytes = ?3,
                             compressed_preview = ?4
                         WHERE id = ?5",
                        params![
                            document.fingerprint.modified_ns,
                            document.fingerprint.file_size,
                            document.indexed_bytes as i64,
                            document.compressed_preview,
                            rowid
                        ],
                    )?;
                    rowid
                } else {
                    tx.execute(
                        "INSERT INTO project_content_files
                            (project_id, relative_path, modified_ns, file_size,
                             indexed_bytes, compressed_preview)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            project_id,
                            document.relative_path,
                            document.fingerprint.modified_ns,
                            document.fingerprint.file_size,
                            document.indexed_bytes as i64,
                            document.compressed_preview
                        ],
                    )?;
                    tx.last_insert_rowid()
                };
                tx.execute(
                    "INSERT INTO project_content_fts (rowid, relative_path, body)
                     VALUES (?1, ?2, ?3)",
                    params![rowid, document.relative_path, document.body],
                )?;
            }

            let existing_rows: Vec<(i64, String)> = {
                let mut stmt = tx.prepare(
                    "SELECT id, relative_path FROM project_content_files
                     WHERE project_id = ?1",
                )?;
                let rows = stmt
                    .query_map(params![project_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                rows
            };
            let mut removed = 0usize;
            for (rowid, relative_path) in existing_rows {
                if collection.retained_paths.contains(&relative_path) {
                    continue;
                }
                tx.execute(
                    "DELETE FROM project_content_fts WHERE rowid = ?1",
                    params![rowid],
                )?;
                tx.execute(
                    "DELETE FROM project_content_files WHERE id = ?1",
                    params![rowid],
                )?;
                removed += 1;
            }
            let (indexed_files, total_bytes): (i64, i64) = tx.query_row(
                "SELECT COUNT(*), COALESCE(SUM(indexed_bytes), 0)
                 FROM project_content_files WHERE project_id = ?1",
                params![project_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            tx.execute(
                "INSERT INTO project_content_status
                    (project_id, indexed_files, indexed_bytes,
                     skipped_files, truncated)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(project_id) DO UPDATE SET
                    indexed_files = excluded.indexed_files,
                    indexed_bytes = excluded.indexed_bytes,
                    skipped_files = excluded.skipped_files,
                    truncated = excluded.truncated",
                params![
                    project_id,
                    indexed_files,
                    total_bytes,
                    collection.skipped_files as i64,
                    i64::from(collection.truncated)
                ],
            )?;
            tx.commit()?;

            stats.projects_scanned += 1;
            stats.files_indexed += collection.documents.len();
            stats.files_reused += collection.reused_files;
            stats.files_removed += removed;
            stats.files_skipped += collection.skipped_files;
            stats.indexed_bytes += collection.indexed_bytes;
            stats.truncated_projects += usize::from(collection.truncated);
        }
        Ok(stats)
    }

    /// Read-only detection of legacy rows that cannot be normalized safely or
    /// that collide after native normalization. No repair is attempted here.
    pub fn inspect_project_path_anomalies(&self) -> Result<Vec<String>> {
        let mut anomalies = Vec::new();
        let mut seen: HashMap<String, String> = HashMap::new();
        let mut stmt = self
            .connection
            .prepare("SELECT id, path FROM projects ORDER BY rowid")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (id, stored_path) in rows {
            match path::normalize_native(&stored_path) {
                None => anomalies.push(format!(
                    "Project '{id}' has an invalid legacy path: '{stored_path}'."
                )),
                Some(normalized) => {
                    let identity = path_identity_key(&normalized);
                    if let Some(existing_id) = seen.get(&identity) {
                        anomalies.push(format!(
                            "Projects '{existing_id}' and '{id}' collide after path normalization: '{normalized}'."
                        ));
                    } else {
                        seen.insert(identity, id);
                    }
                }
            }
        }
        Ok(anomalies)
    }

    /// Lists projects ordered by `last_accessed_at` descending. When `search`
    /// names an exact native path (its display name equals itself), the lookup
    /// is exact rather than FTS-based. `tool_key` filters by case-insensitive
    /// tool key.
    pub fn list_projects(
        &self,
        search: Option<&str>,
        tool_key: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Project>> {
        list_projects_from_connection(&self.connection, search, tool_key, limit)
    }

    /// Exact project lookup by native path identity; returns `None` when the
    /// path is not valid or not indexed.
    pub fn get_project_by_path(&self, candidate: &str) -> Result<Option<Project>> {
        let Some(normalized) = path::normalize_native(candidate) else {
            return Ok(None);
        };
        let path_where = path_equality_clause("");
        let sql = format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE {path_where} LIMIT 1");
        let row = self
            .connection
            .query_row(&sql, params![normalized], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .optional()?;
        let Some((id, project_path, last, first, branch, remote)) = row else {
            return Ok(None);
        };
        let path_missing = project_path_missing(&project_path);
        let mut project = Project {
            id,
            path: project_path,
            path_missing,
            last_accessed_at: parse_timestamp(&last)?,
            first_seen_at: parse_timestamp(&first)?,
            git_branch: branch,
            git_remote_url: remote,
            tool_usages: Vec::new(),
        };
        project.tool_usages = load_usages(&self.connection, &project.id)?;
        Ok(Some(project))
    }

    /// Records a session with its native-normalized project path.
    pub fn record_session(&self, session: &Session) -> Result<()> {
        let normalized = normalize_session_path(&session.project_path)?;
        self.connection.execute(
            "INSERT INTO sessions
                (id, project_path, tool_key, tool_name, started_at, ended_at, session_id_from_tool)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session.id,
                normalized,
                session.tool_key,
                session.tool_name,
                timestamp(session.started_at),
                session.ended_at.map(timestamp),
                session.session_id_from_tool
            ],
        )?;
        Ok(())
    }

    /// Returns the most recent sessions ordered by `started_at` descending.
    pub fn get_recent_sessions(&self, limit: usize) -> Result<Vec<Session>> {
        get_recent_sessions_from_connection(&self.connection, limit)
    }
}

type ProjectFields = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);

fn list_projects_from_connection(
    connection: &Connection,
    search: Option<&str>,
    tool_key: Option<&str>,
    limit: usize,
) -> Result<Vec<Project>> {
    if !(1..=10_000).contains(&limit) {
        return Err(StoreError::InvalidLimit);
    }
    let rows = if let Some(search) = search.filter(|value| !value.trim().is_empty()) {
        search_projects_rows(connection, search, limit)?
    } else if let Some(tool_key) = tool_key.filter(|value| !value.trim().is_empty()) {
        list_projects_by_tool_rows(connection, tool_key, limit)?
    } else {
        list_all_projects_rows(connection, limit)?
    };
    let mut projects = Vec::with_capacity(rows.len());
    for (id, project_path, last, first, branch, remote) in rows {
        let path_missing = project_path_missing(&project_path);
        projects.push(Project {
            id,
            path: project_path,
            path_missing,
            last_accessed_at: parse_timestamp(&last)?,
            first_seen_at: parse_timestamp(&first)?,
            git_branch: branch,
            git_remote_url: remote,
            tool_usages: Vec::new(),
        });
    }
    for project in &mut projects {
        project.tool_usages = load_usages(connection, &project.id)?;
    }
    Ok(projects)
}

fn get_recent_sessions_from_connection(
    connection: &Connection,
    limit: usize,
) -> Result<Vec<Session>> {
    if !(1..=1000).contains(&limit) {
        return Err(StoreError::InvalidLimit);
    }
    let mut stmt = connection.prepare(
        "SELECT id, project_path, tool_key, tool_name, started_at, ended_at, session_id_from_tool
         FROM sessions
         ORDER BY started_at DESC
         LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut sessions = Vec::with_capacity(rows.len());
    for (id, project_path, tool_key, tool_name, started, ended, session_id_from_tool) in rows {
        sessions.push(Session {
            id,
            project_path,
            tool_key,
            tool_name,
            started_at: parse_timestamp(&started)?,
            ended_at: match ended {
                Some(text) => Some(parse_timestamp(&text)?),
                None => None,
            },
            session_id_from_tool,
        });
    }
    Ok(sessions)
}

fn search_projects_rows(
    connection: &Connection,
    search: &str,
    limit: usize,
) -> Result<Vec<ProjectFields>> {
    let exact = path::normalize_native(search).filter(|candidate| {
        path::display_name_native(candidate).as_deref() == Some(candidate.as_str())
    });
    if let Some(root) = exact {
        let path_where = path_equality_clause("p.");
        let sql = format!(
            "SELECT p.{PROJECT_COLUMNS} FROM projects p
             WHERE {path_where}
             ORDER BY p.last_accessed_at DESC
             LIMIT ?2"
        );
        let mut stmt = connection.prepare(&sql)?;
        let rows = stmt
            .query_map(params![root, limit as i64], project_fields)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        return Ok(rows);
    }
    let Some(fts_query) = build_fts_prefix_query(search) else {
        return Ok(Vec::new());
    };
    let content_query = build_content_fts_prefix_query(search);
    let content_search_enabled = content_query.is_some() && content_index_is_safe(connection)?;
    let rows = if let Some(content_query) = content_query.filter(|_| content_search_enabled) {
        let sql = format!(
            "SELECT p.{PROJECT_COLUMNS} FROM projects p
             WHERE p.rowid IN (
                       SELECT rowid FROM projects_fts WHERE projects_fts MATCH ?1
                   )
                OR p.id IN (
                       SELECT content_file.project_id
                       FROM project_content_fts
                       JOIN project_content_files content_file
                         ON content_file.id = project_content_fts.rowid
                       WHERE project_content_fts MATCH ?2
                   )
             ORDER BY p.last_accessed_at DESC
             LIMIT ?3"
        );
        let mut stmt = connection.prepare(&sql)?;
        let rows = stmt
            .query_map(
                params![fts_query, content_query, limit as i64],
                project_fields,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows
    } else {
        let sql = format!(
            "SELECT p.{PROJECT_COLUMNS} FROM projects p
             WHERE p.rowid IN (
                 SELECT rowid FROM projects_fts WHERE projects_fts MATCH ?1
             )
             ORDER BY p.last_accessed_at DESC
             LIMIT ?2"
        );
        let mut stmt = connection.prepare(&sql)?;
        let rows = stmt
            .query_map(params![fts_query, limit as i64], project_fields)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows
    };
    Ok(rows)
}

/// Whether optional content search is safe to expose. Any missing, malformed,
/// or non-FTS5 content structure fails closed while name/path search remains
/// available.
pub fn content_index_is_safe(connection: &Connection) -> Result<bool> {
    if !content_index_sanitizer_is_current(connection)? {
        return Ok(false);
    }
    if !optional_schema_is_valid(validate_table_columns(
        connection,
        "project_content_files",
        &[
            "id",
            "project_id",
            "relative_path",
            "modified_ns",
            "file_size",
            "indexed_bytes",
            "compressed_preview",
        ],
    ))? {
        return Ok(false);
    }
    if !optional_schema_is_valid(validate_content_fts5_table(
        connection,
        "project_content_fts",
        &["relative_path", "body"],
    ))? {
        return Ok(false);
    }
    Ok(true)
}

fn optional_schema_is_valid(result: Result<()>) -> Result<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(StoreError::SchemaInvariant(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

fn list_projects_by_tool_rows(
    connection: &Connection,
    tool_key: &str,
    limit: usize,
) -> Result<Vec<ProjectFields>> {
    let sql = format!(
        "SELECT DISTINCT p.{PROJECT_COLUMNS} FROM projects p
         JOIN tool_usages u ON u.project_id = p.id
         WHERE u.tool_key = ?1 COLLATE NOCASE
         ORDER BY p.last_accessed_at DESC
         LIMIT ?2"
    );
    let mut stmt = connection.prepare(&sql)?;
    let rows = stmt
        .query_map(params![tool_key, limit as i64], project_fields)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn list_all_projects_rows(connection: &Connection, limit: usize) -> Result<Vec<ProjectFields>> {
    let sql = format!(
        "SELECT {PROJECT_COLUMNS} FROM projects
         ORDER BY last_accessed_at DESC
         LIMIT ?1"
    );
    let mut stmt = connection.prepare(&sql)?;
    let rows = stmt
        .query_map(params![limit as i64], project_fields)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn project_fields(row: &rusqlite::Row) -> rusqlite::Result<ProjectFields> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

fn initialize_schema(connection: &mut Connection) -> Result<()> {
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let schema_version = read_index_schema_version(&tx)?;
    ensure_supported_schema_version(schema_version)?;
    if schema_version == 0 {
        // All structural changes, legacy deduplication, FTS rebuilding and the
        // version marker are one transaction. A failed migration therefore
        // leaves the v0 database byte-for-byte usable for a retry.
        tx.execute_batch(SCHEMA_SQL)?;
        migrate_tool_usage_identity_tx(&tx)?;
        if cfg!(windows) {
            tx.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_projects_path_nocase ON projects(path COLLATE NOCASE)",
            )?;
        }
        tx.execute_batch(TOOL_KEY_NOCASE_INDEX_SQL)?;
        rebuild_fts(&tx)?;
        validate_writer_schema(&tx)?;
        tx.pragma_update(None, "user_version", INDEX_SCHEMA_VERSION)?;
    } else {
        validate_writer_schema(&tx)?;
    }

    tx.commit()?;

    // Content sanitization is an independent cache-generation maintenance
    // pass. It intentionally runs only after the schema transaction commits;
    // VACUUM/checkpoint must never be part of the schema migration.
    migrate_content_index_sanitizer(connection)?;
    Ok(())
}

fn read_index_schema_version(connection: &Connection) -> Result<i64> {
    connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| {
            StoreError::InvalidSchema(format!("cannot read PRAGMA user_version: {error}"))
        })
}

fn ensure_supported_schema_version(version: i64) -> Result<()> {
    if !(0..=INDEX_SCHEMA_VERSION).contains(&version) {
        return Err(StoreError::UnsupportedSchemaVersion {
            found: version,
            supported: INDEX_SCHEMA_VERSION,
        });
    }
    Ok(())
}

/// Validates the shared reader contract without creating, repairing or
/// otherwise mutating the database. Both v0 legacy indexes and v1 indexes use
/// the same required query schema.
pub fn validate_index_schema(connection: &Connection) -> Result<i64> {
    let version = read_index_schema_version(connection)?;
    if !(0..=INDEX_SCHEMA_VERSION).contains(&version) {
        return Err(StoreError::UnsupportedSchemaVersion {
            found: version,
            supported: INDEX_SCHEMA_VERSION,
        });
    }

    for (table, columns) in [
        (
            "projects",
            [
                "id",
                "path",
                "last_accessed_at",
                "first_seen_at",
                "git_branch",
                "git_remote_url",
            ]
            .as_slice(),
        ),
        (
            "tool_usages",
            [
                "project_id",
                "tool_name",
                "tool_key",
                "last_used_at",
                "session_count",
                "last_session_id",
            ]
            .as_slice(),
        ),
        (
            "sessions",
            [
                "id",
                "project_path",
                "tool_key",
                "tool_name",
                "started_at",
                "ended_at",
                "session_id_from_tool",
            ]
            .as_slice(),
        ),
        ("projects_fts", ["name", "path"].as_slice()),
    ] {
        validate_table_columns(connection, table, columns)?;
        if table == "projects_fts" {
            validate_fts5_table(connection, table, columns)?;
        }
    }
    Ok(version)
}

fn validate_writer_schema(connection: &Connection) -> Result<i64> {
    let version = validate_index_schema(connection)?;
    validate_table_columns(
        connection,
        "tool_usages",
        &[
            "id",
            "project_id",
            "tool_name",
            "tool_key",
            "last_used_at",
            "session_count",
            "last_session_id",
        ],
    )?;
    validate_integer_primary_key(connection, "tool_usages", "id")?;
    validate_table_columns(connection, "sessionatlas_meta", &["key", "value"])?;
    validate_table_columns(
        connection,
        "project_content_files",
        &[
            "id",
            "project_id",
            "relative_path",
            "modified_ns",
            "file_size",
            "indexed_bytes",
            "compressed_preview",
        ],
    )?;
    validate_integer_primary_key(connection, "project_content_files", "id")?;
    validate_content_fts5_table(
        connection,
        "project_content_fts",
        &["relative_path", "body"],
    )?;
    validate_table_columns(
        connection,
        "project_content_status",
        &[
            "project_id",
            "indexed_files",
            "indexed_bytes",
            "skipped_files",
            "truncated",
        ],
    )?;
    validate_unique_usage_index(connection)?;
    Ok(version)
}

fn validate_table_columns(connection: &Connection, table: &str, columns: &[&str]) -> Result<()> {
    let table_type: Option<String> = connection
        .query_row(
            "SELECT type FROM sqlite_master WHERE name = ?1",
            [table],
            |row| row.get(0),
        )
        .optional()?;
    if table_type.as_deref() != Some("table") {
        return Err(StoreError::SchemaInvariant(format!(
            "required table '{table}' is missing"
        )));
    }

    let pragma = format!("PRAGMA table_info('{}')", table.replace('\'', "''"));
    let actual = connection
        .prepare(&pragma)?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<HashSet<_>, _>>()?;
    let missing = columns
        .iter()
        .filter(|column| !actual.contains(**column))
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(StoreError::SchemaInvariant(format!(
            "required table '{table}' is missing columns: {}",
            missing.join(", ")
        )));
    }
    Ok(())
}

fn validate_integer_primary_key(connection: &Connection, table: &str, column: &str) -> Result<()> {
    let pragma = format!("PRAGMA table_info('{}')", table.replace('\'', "''"));
    let declared_columns = connection
        .prepare(&pragma)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(5)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let primary_key_columns = declared_columns
        .iter()
        .filter(|(_, _, primary_key_order)| *primary_key_order > 0)
        .count();
    let valid = declared_columns.iter().any(|(name, declared_type, order)| {
        name == column && declared_type.eq_ignore_ascii_case("INTEGER") && *order == 1
    }) && primary_key_columns == 1;
    if !valid {
        return Err(StoreError::SchemaInvariant(format!(
            "required column '{table}.{column}' must be the single INTEGER PRIMARY KEY"
        )));
    }
    Ok(())
}

fn validate_fts5_table(connection: &Connection, table: &str, columns: &[&str]) -> Result<()> {
    validate_fts5_definition(connection, table)?;
    validate_table_columns(connection, table, columns)
}

fn validate_fts5_definition(connection: &Connection, table: &str) -> Result<String> {
    let table_kind: Option<String> = connection
        .query_row(
            "SELECT type FROM pragma_table_list
             WHERE schema = 'main' AND name = ?1",
            [table],
            |row| row.get(0),
        )
        .optional()?;
    if table_kind.as_deref() != Some("virtual") {
        return Err(StoreError::SchemaInvariant(format!(
            "required table '{table}' is not a virtual table"
        )));
    }

    let sql: Option<String> = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        )
        .optional()?;
    let normalized = sql
        .as_deref()
        .map(|definition| {
            definition
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase()
        })
        .unwrap_or_default();
    let module = normalized
        .split_once(" using ")
        .map(|(_, remainder)| remainder)
        .and_then(|remainder| {
            remainder
                .split(|character: char| character == '(' || character.is_whitespace())
                .find(|part| !part.is_empty())
        });
    let is_fts5 = module == Some("fts5");
    if !is_fts5 {
        return Err(StoreError::SchemaInvariant(format!(
            "required table '{table}' is not an FTS5 virtual table"
        )));
    }
    Ok(normalized)
}

fn validate_content_fts5_table(
    connection: &Connection,
    table: &str,
    columns: &[&str],
) -> Result<()> {
    let definition = validate_fts5_definition(connection, table)?;
    validate_table_columns(connection, table, columns)?;
    let compact_definition = definition
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let content_shadow = format!("{table}_content");
    let has_content_shadow: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [content_shadow],
        |row| row.get(0),
    )?;
    let is_contentless = has_content_shadow == 0
        && compact_definition.contains("content=''")
        && compact_definition.contains("contentless_delete=1");
    if !is_contentless {
        return Err(StoreError::SchemaInvariant(format!(
            "required table '{table}' must use content='' and contentless_delete=1"
        )));
    }
    Ok(())
}

fn validate_unique_usage_index(connection: &Connection) -> Result<()> {
    let mut statement = connection.prepare("PRAGMA index_list('tool_usages')")?;
    let indexes = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if indexes
        .iter()
        .find(|(name, _, _)| name == "idx_usages_project_tool")
        .map(|(_, unique, partial)| (*unique, *partial))
        != Some((1, 0))
    {
        return Err(StoreError::SchemaInvariant(
            "idx_usages_project_tool must be a non-partial unique index".to_string(),
        ));
    }

    let mut statement = connection.prepare("PRAGMA index_xinfo('idx_usages_project_tool')")?;
    let index_columns = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|(_, _, is_key)| *is_key == 1)
        .map(|(name, collation, _)| (name, collation))
        .collect::<Vec<_>>();
    let has_expected_identity = index_columns.len() == 2
        && index_columns[0].0.as_deref() == Some("project_id")
        && index_columns[0].1.eq_ignore_ascii_case("BINARY")
        && index_columns[1].0.as_deref() == Some("tool_key")
        && index_columns[1].1.eq_ignore_ascii_case("NOCASE");
    if !has_expected_identity {
        return Err(StoreError::SchemaInvariant(
            "idx_usages_project_tool must index project_id and tool_key COLLATE NOCASE".to_string(),
        ));
    }
    Ok(())
}

fn migrate_content_index_sanitizer(connection: &Connection) -> Result<()> {
    let current = connection
        .query_row(
            "SELECT value FROM sessionatlas_meta
             WHERE key = 'content_index_sanitizer_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if current.as_deref() == Some(&CONTENT_INDEX_SANITIZER_VERSION.to_string()) {
        return Ok(());
    }

    connection.execute_batch("PRAGMA secure_delete = ON")?;
    let tx = connection.unchecked_transaction()?;
    tx.execute("DELETE FROM project_content_fts", [])?;
    tx.execute("DELETE FROM project_content_files", [])?;
    tx.execute("DELETE FROM project_content_status", [])?;
    tx.commit()?;

    // Reclaim pages that may still contain a preview from an older sanitizer.
    // Commit the version only after physical cleanup succeeds. An interrupted
    // or busy cleanup is therefore retried on the next open even when the
    // logical rows are already gone.
    connection.execute_batch("VACUUM")?;
    let checkpoint_busy: i64 =
        connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
    if checkpoint_busy != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "content index WAL cleanup is busy",
        )
        .into());
    }

    connection.execute(
        "INSERT INTO sessionatlas_meta (key, value)
         VALUES ('content_index_sanitizer_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![CONTENT_INDEX_SANITIZER_VERSION.to_string()],
    )?;
    Ok(())
}

/// Whether stored content was produced by the current sanitizer. Readers use
/// this as a fail-closed gate so an older database cannot surface stale
/// previews before the next scan rebuilds them.
pub fn content_index_sanitizer_is_current(connection: &Connection) -> Result<bool> {
    let has_meta: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'table' AND name = 'sessionatlas_meta'",
        [],
        |row| row.get(0),
    )?;
    if has_meta != 1 {
        return Ok(false);
    }
    let version = connection
        .query_row(
            "SELECT value FROM sessionatlas_meta
             WHERE key = 'content_index_sanitizer_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(version.as_deref() == Some(&CONTENT_INDEX_SANITIZER_VERSION.to_string()))
}

fn migrate_tool_usage_identity_tx(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(MIGRATE_TOOL_USAGE_IDENTITY_SQL)?;
    Ok(())
}

/// Path-equality predicate for SQL: case-insensitive on Windows and byte-exact
/// on Unix.
fn path_equality_clause(qualifier: &str) -> String {
    if cfg!(windows) {
        format!("{qualifier}path = ?1 COLLATE NOCASE")
    } else {
        format!("{qualifier}path = ?1")
    }
}

/// Upserts a snapshot project by path identity. Existing projects keep their
/// id and `first_seen_at`; only git metadata is refreshed.
fn upsert_snapshot_project(tx: &Transaction<'_>, project: &Project) -> Result<String> {
    let normalized_path = normalize_project_path(&project.path)?;
    let path_where = path_equality_clause("");
    let existing = tx
        .query_row(
            &format!("SELECT id FROM projects WHERE {path_where} LIMIT 1"),
            params![normalized_path],
            |row| row.get::<_, String>(0),
        )
        .optional()?;

    if let Some(existing_id) = existing {
        tx.execute(
            "UPDATE projects
             SET
                 git_branch = COALESCE(?1, git_branch),
                 git_remote_url = COALESCE(?2, git_remote_url)
             WHERE id = ?3",
            params![project.git_branch, project.git_remote_url, existing_id],
        )?;
        return Ok(existing_id);
    }

    let project_id = if project.id.trim().is_empty() {
        uuid::Uuid::new_v4().simple().to_string()
    } else {
        project.id.clone()
    };
    tx.execute(
        "INSERT INTO projects
            (id, path, last_accessed_at, first_seen_at, git_branch, git_remote_url)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            project_id,
            normalized_path,
            timestamp(project.last_accessed_at),
            timestamp(project.first_seen_at),
            project.git_branch,
            project.git_remote_url
        ],
    )?;
    Ok(project_id)
}

fn sync_fts_row(connection: &Connection, rowid: i64, normalized_path: &str) -> Result<()> {
    connection.execute("DELETE FROM projects_fts WHERE rowid = ?1", params![rowid])?;
    let name = path::display_name_native(normalized_path).unwrap_or_default();
    connection.execute(
        "INSERT INTO projects_fts (rowid, name, path) VALUES (?1, ?2, ?3)",
        params![rowid, name, normalized_path],
    )?;
    Ok(())
}

fn rebuild_fts(connection: &Connection) -> Result<()> {
    connection.execute("DELETE FROM projects_fts", [])?;
    let mut stmt = connection.prepare("SELECT rowid, path FROM projects ORDER BY rowid")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(stmt);
    for (rowid, project_path) in rows {
        sync_fts_row(connection, rowid, &project_path)?;
    }
    Ok(())
}

fn load_usages(connection: &Connection, project_id: &str) -> Result<Vec<ToolUsage>> {
    let mut stmt = connection.prepare(
        "SELECT tool_name, tool_key, last_used_at, session_count, last_session_id
         FROM tool_usages
         WHERE project_id = ?1",
    )?;
    let rows = stmt
        .query_map(params![project_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut usages = Vec::with_capacity(rows.len());
    for (tool_name, tool_key, last_used, session_count, last_session_id) in rows {
        usages.push(ToolUsage {
            tool_name,
            tool_key,
            last_used_at: parse_timestamp(&last_used)?,
            session_count: i32::try_from(session_count).map_err(|_| {
                StoreError::CorruptRow(format!("session_count out of range: {session_count}"))
            })?,
            last_session_id,
        });
    }
    Ok(usages)
}

/// Validates a snapshot before any mutation. Returns the trimmed scanned tool
/// keys (original case). Mirrors `SqliteStore.ValidateSnapshot`.
fn validate_snapshot(projects: &[Project], scanned_tool_keys: &[&str]) -> Result<HashSet<String>> {
    let mut tool_keys = HashSet::new();
    for key in scanned_tool_keys {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return Err(StoreError::EmptyScannedToolKey);
        }
        tool_keys.insert(trimmed.to_string());
    }
    if tool_keys.is_empty() {
        return Err(StoreError::ScannedToolKeysEmpty);
    }
    let folded_keys: HashSet<String> = tool_keys.iter().map(|key| fold_case(key)).collect();

    let mut seen_paths = HashSet::new();
    for project in projects {
        if project.path.trim().is_empty() {
            return Err(StoreError::EmptyProjectPath);
        }
        let normalized = normalize_project_path(&project.path)?;
        if !seen_paths.insert(path_identity_key(&normalized)) {
            return Err(StoreError::DuplicateProjectPath(normalized));
        }
        if project.tool_usages.is_empty() {
            return Err(StoreError::NoToolUsages(normalized.clone()));
        }
        let mut project_keys = HashSet::new();
        for usage in &project.tool_usages {
            let folded = fold_case(&usage.tool_key);
            if !folded_keys.contains(&folded) {
                return Err(StoreError::UndeclaredUsageTool {
                    project: normalized.clone(),
                    tool: usage.tool_key.clone(),
                });
            }
            if !project_keys.insert(folded) {
                return Err(StoreError::DuplicateUsageTool {
                    project: normalized.clone(),
                    tool: usage.tool_key.clone(),
                });
            }
            if usage.session_count < 0 {
                return Err(StoreError::NegativeSessionCount {
                    project: normalized.clone(),
                    tool: usage.tool_key.clone(),
                });
            }
        }
    }
    Ok(tool_keys)
}

fn normalize_project_path(candidate: &str) -> Result<String> {
    path::normalize_native(candidate)
        .ok_or_else(|| StoreError::InvalidProjectPath(candidate.to_string()))
}

fn normalize_session_path(candidate: &str) -> Result<String> {
    path::normalize_native(candidate)
        .ok_or_else(|| StoreError::InvalidSessionPath(candidate.to_string()))
}

/// Map key for native path identity: case-folded on Windows, byte-exact on
/// Unix, matching `path::paths_equal`.
fn path_identity_key(value: &str) -> String {
    if cfg!(windows) {
        fold_case(value)
    } else {
        value.to_string()
    }
}

/// Unicode case folding used for case-insensitive identity matching.
fn fold_case(value: &str) -> String {
    value.chars().flat_map(char::to_uppercase).collect()
}

/// Formats a UTC timestamp as RFC 3339 with microsecond precision, parseable by
/// `DateTime::parse_from_rfc3339`.
fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

fn parse_timestamp(text: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| StoreError::BadTimestamp(text.to_string()))
}

/// Splits a search term on non-alphanumeric characters (except `_`) and emits
/// an FTS5 prefix query joining every term with `AND`. `None` means no usable
/// term, so the caller returns an empty result — FTS operators and punctuation
/// become literal separators.
fn build_fts_prefix_query(query: &str) -> Option<String> {
    build_fts_prefix_query_with_minimum(query, 1)
}

fn build_content_fts_prefix_query(query: &str) -> Option<String> {
    build_fts_prefix_query_with_minimum(query, 2)
}

fn build_fts_prefix_query_with_minimum(query: &str, minimum_chars: usize) -> Option<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    for character in query.chars().take(256) {
        if character.is_alphanumeric() || character == '_' {
            current.push(character);
            continue;
        }
        if !current.is_empty() {
            if current.chars().count() >= minimum_chars {
                terms.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
            if terms.len() == 12 {
                break;
            }
        }
    }
    if terms.len() < 12 && current.chars().count() >= minimum_chars {
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
