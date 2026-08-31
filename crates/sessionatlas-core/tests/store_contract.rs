//! Contract tests for `sessionatlas_core::store` (task R07).
//!
//! These tests cover snapshot replacement and platform-specific path semantics.
//! They use `tempfile` for isolated database
//! directories and `rusqlite` only to seed legacy schemas or attach triggers —
//! never the real `~/.sessionatlas`.

use chrono::{DateTime, NaiveDate, Utc};
use sessionatlas_core::content_index::ContentIndexOptions;
use sessionatlas_core::model::{Project, Session, ToolUsage};
use sessionatlas_core::path;
use sessionatlas_core::store::{
    content_index_is_safe, content_index_sanitizer_is_current, IndexReader, SqliteStore,
    StoreError, CONTENT_INDEX_SANITIZER_VERSION, INDEX_SCHEMA_VERSION,
};

fn utc(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc()
}

/// A deterministic absolute path for the current host's native flavor.
fn abs_path(segments: &[&str]) -> String {
    if cfg!(windows) {
        format!("C:\\{}", segments.join("\\"))
    } else {
        format!("/{}", segments.join("/"))
    }
}

fn usage(key: &str, last_used: DateTime<Utc>, count: i32, session_id: Option<&str>) -> ToolUsage {
    ToolUsage {
        tool_name: key.to_string(),
        tool_key: key.to_string(),
        last_used_at: last_used,
        session_count: count,
        last_session_id: session_id.map(str::to_string),
    }
}

fn project_at(
    path_text: &str,
    id: &str,
    first_seen: DateTime<Utc>,
    usages: &[ToolUsage],
) -> Project {
    Project {
        id: id.to_string(),
        path: path_text.to_string(),
        last_accessed_at: usages.iter().map(|usage| usage.last_used_at).max().unwrap(),
        first_seen_at: first_seen,
        git_branch: None,
        git_remote_url: None,
        path_missing: false,
        tool_usages: usages.to_vec(),
    }
}

fn session(id: &str, path_text: &str, started: DateTime<Utc>) -> Session {
    Session {
        id: id.to_string(),
        project_path: path_text.to_string(),
        tool_key: "codex".to_string(),
        tool_name: "Codex".to_string(),
        started_at: started,
        ended_at: None,
        session_id_from_tool: None,
    }
}

fn db_path(root: &tempfile::TempDir) -> std::path::PathBuf {
    root.path().join("index.db")
}

fn database_fingerprint(
    path: &std::path::Path,
) -> (Vec<u8>, u64, std::time::SystemTime, Vec<String>) {
    let metadata = std::fs::metadata(path).unwrap();
    let mut entries = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let metadata = entry.metadata().unwrap();
            format!(
                "{}:{}:{}",
                entry.file_name().to_string_lossy(),
                metadata.len(),
                metadata.is_dir()
            )
        })
        .collect::<Vec<_>>();
    entries.sort();
    (
        std::fs::read(path).unwrap(),
        metadata.len(),
        metadata.modified().unwrap(),
        entries,
    )
}

fn seed_legacy_base_schema(connection: &rusqlite::Connection, project_path: &str) {
    connection
        .execute_batch(
            "CREATE TABLE projects (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                last_accessed_at TEXT,
                first_seen_at TEXT,
                git_branch TEXT,
                git_remote_url TEXT
            );
            CREATE VIRTUAL TABLE projects_fts USING fts5(name, path);
            CREATE TABLE tool_usages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                tool_key TEXT NOT NULL,
                last_used_at TEXT,
                session_count INTEGER DEFAULT 1,
                last_session_id TEXT
            );
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                project_path TEXT NOT NULL,
                tool_key TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                started_at TEXT,
                ended_at TEXT,
                session_id_from_tool TEXT
            );",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO projects (id, path, last_accessed_at, first_seen_at)
             VALUES ('legacy-project', ?1, '2026-07-02T12:00:00Z', '2026-07-01T12:00:00Z')",
            [project_path],
        )
        .unwrap();
    connection
        .execute_batch(
            "INSERT INTO projects_fts (rowid, name, path)
             SELECT rowid, 'legacy-project', path FROM projects;
             INSERT INTO tool_usages
                 (project_id, tool_name, tool_key, last_used_at, session_count)
             VALUES
                 ('legacy-project', 'Codex', 'codex', '2026-07-01T12:00:00Z', 1),
                 ('legacy-project', 'Codex', 'CODEX', '2026-07-02T12:00:00Z', 4);",
        )
        .unwrap();
}

#[test]
fn fresh_store_sets_explicit_index_schema_version() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    drop(SqliteStore::new(&db).unwrap());
    let connection = rusqlite::Connection::open(&db).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, INDEX_SCHEMA_VERSION);
}

#[test]
fn legacy_store_migrates_atomically_and_preserves_data_and_fts() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let connection = rusqlite::Connection::open(&db).unwrap();
    seed_legacy_base_schema(&connection, &abs_path(&["legacy-project"]));
    drop(connection);

    drop(SqliteStore::new(&db).unwrap());
    let connection = rusqlite::Connection::open(&db).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, INDEX_SCHEMA_VERSION);
    let usage_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM tool_usages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(usage_count, 1);
    let usage_count: i64 = connection
        .query_row("SELECT session_count FROM tool_usages", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(usage_count, 4);
    let fts_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM projects_fts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(fts_count, 1);
}

#[test]
fn v1_store_open_does_not_change_database_fingerprint() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let mut store = SqliteStore::new(&db).unwrap();
    store
        .upsert_project(&project_at(
            &abs_path(&["v1-fingerprint"]),
            "v1-fingerprint",
            utc(2026, 7, 1),
            &[usage("codex", utc(2026, 7, 2), 1, None)],
        ))
        .unwrap();
    drop(store);
    let before = database_fingerprint(&db);
    drop(SqliteStore::new(&db).unwrap());
    assert_eq!(database_fingerprint(&db), before);
}

#[test]
fn unsupported_schema_version_is_rejected_without_sanitizer_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    drop(SqliteStore::new(&db).unwrap());
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute(
            "INSERT INTO sessionatlas_meta (key, value) VALUES ('sentinel', 'keep')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO projects (id, path) VALUES ('v2-content', ?1)",
            [abs_path(&["v2-content"])],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO project_content_files
                (project_id, relative_path, modified_ns, file_size, indexed_bytes, compressed_preview)
             VALUES ('v2-content', 'secret.txt', 1, 1, 1, X'01')",
            [],
        )
        .unwrap();
    let content_rowid = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO project_content_fts (rowid, relative_path, body)
             VALUES (?1, 'secret.txt', 'sentinel-content')",
            [content_rowid],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE sessionatlas_meta SET value = '1'
             WHERE key = 'content_index_sanitizer_version'",
            [],
        )
        .unwrap();
    connection
        .pragma_update(None, "user_version", 2_i64)
        .unwrap();
    drop(connection);
    let before = database_fingerprint(&db);
    let error = match SqliteStore::new(&db) {
        Ok(_) => panic!("unsupported schema unexpectedly opened"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        StoreError::UnsupportedSchemaVersion { found: 2, .. }
    ));
    assert_eq!(database_fingerprint(&db), before);
    let connection = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT value FROM sessionatlas_meta WHERE key = 'sentinel'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "keep"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT value FROM sessionatlas_meta WHERE key = 'content_index_sanitizer_version'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "1"
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM project_content_files", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        1
    );
    connection
        .pragma_update(None, "user_version", -1_i64)
        .unwrap();
    drop(connection);
    let error = match SqliteStore::new(&db) {
        Ok(_) => panic!("negative schema unexpectedly opened"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        StoreError::UnsupportedSchemaVersion { found: -1, .. }
    ));
}

#[cfg(unix)]
#[test]
fn unsupported_writer_version_does_not_harden_existing_modes() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let root = tempfile::tempdir().unwrap();
    let data = root.path().join(".sessionatlas");
    std::fs::create_dir(&data).unwrap();
    let db = data.join("index.db");
    drop(SqliteStore::new(&db).unwrap());
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .pragma_update(None, "user_version", 2_i64)
        .unwrap();
    drop(connection);
    let before_dir = std::fs::metadata(&data).unwrap().mode() & 0o777;
    let before_db = std::fs::metadata(&db).unwrap().mode() & 0o777;
    assert!(matches!(
        SqliteStore::new(&db),
        Err(StoreError::UnsupportedSchemaVersion { found: 2, .. })
    ));
    assert_eq!(std::fs::metadata(&data).unwrap().mode() & 0o777, before_dir);
    assert_eq!(std::fs::metadata(&db).unwrap().mode() & 0o777, before_db);
}

#[test]
fn reader_rejects_unsupported_schema_version_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    drop(SqliteStore::new(&db).unwrap());
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .pragma_update(None, "user_version", 2_i64)
        .unwrap();
    drop(connection);
    let before = database_fingerprint(&db);
    let error = match IndexReader::open_existing(&db) {
        Ok(_) => panic!("unsupported schema unexpectedly opened"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        StoreError::UnsupportedSchemaVersion { found: 2, .. }
    ));
    assert_eq!(database_fingerprint(&db), before);
}

#[test]
fn reader_rejects_a_plain_table_that_masquerades_as_projects_fts() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    drop(SqliteStore::new(&db).unwrap());
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "DROP TABLE projects_fts;
             CREATE TABLE projects_fts (name TEXT, path TEXT);",
        )
        .unwrap();
    drop(connection);
    let error = match IndexReader::open_existing(&db) {
        Ok(_) => panic!("plain projects_fts table unexpectedly opened"),
        Err(error) => error,
    };
    assert!(matches!(error, StoreError::SchemaInvariant(_)), "{error}");
}

#[test]
fn reader_rejects_a_plain_table_with_fts_markers_in_its_sql() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    drop(SqliteStore::new(&db).unwrap());
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "DROP TABLE projects_fts;
             CREATE TABLE projects_fts (
                name TEXT DEFAULT 'virtual table',
                path TEXT DEFAULT 'using fts5'
             );",
        )
        .unwrap();
    drop(connection);

    let error = match IndexReader::open_existing(&db) {
        Ok(_) => panic!("plain projects_fts table with marker text unexpectedly opened"),
        Err(error) => error,
    };
    assert!(matches!(error, StoreError::SchemaInvariant(_)), "{error}");
}

#[test]
fn content_index_is_fail_closed_for_a_plain_content_fts_table() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    drop(SqliteStore::new(&db).unwrap());
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "DROP TABLE project_content_fts;
             CREATE TABLE project_content_fts (relative_path TEXT, body TEXT);",
        )
        .unwrap();
    assert!(!content_index_is_safe(&connection).unwrap());
}

#[test]
fn content_index_is_fail_closed_for_a_contentful_fts5_table() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    drop(SqliteStore::new(&db).unwrap());
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "DROP TABLE project_content_fts;
             CREATE VIRTUAL TABLE project_content_fts USING fts5(relative_path, body);",
        )
        .unwrap();

    assert!(!content_index_is_safe(&connection).unwrap());
}

#[test]
fn content_index_propagates_malformed_metadata_errors() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    drop(SqliteStore::new(&db).unwrap());
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "DROP TABLE sessionatlas_meta;
             CREATE TABLE sessionatlas_meta (key TEXT PRIMARY KEY);",
        )
        .unwrap();

    assert!(matches!(
        content_index_is_safe(&connection),
        Err(StoreError::Sql(_))
    ));
}

#[test]
fn reader_ignores_content_search_for_a_plain_content_table_but_keeps_name_search() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let path_text = abs_path(&["content-ordinary-table"]);
    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(
            &[project_at(
                &path_text,
                "content-ordinary-table",
                utc(2026, 7, 1),
                &[usage("codex", utc(2026, 7, 2), 1, None)],
            )],
            &["codex"],
        )
        .unwrap();
    drop(store);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "DROP TABLE project_content_fts;
             CREATE TABLE project_content_fts (relative_path TEXT, body TEXT);",
        )
        .unwrap();
    drop(connection);

    let reader = IndexReader::open_existing(&db).unwrap();
    assert!(reader
        .list_projects(Some("ordinary-content-secret"), None, 10)
        .unwrap()
        .is_empty());
    assert_eq!(
        reader
            .list_projects(Some("content-ordinary-table"), None, 10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn v1_writer_rejects_a_missing_unique_usage_index_without_repairing() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    drop(SqliteStore::new(&db).unwrap());
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute("DROP INDEX idx_usages_project_tool", [])
        .unwrap();
    drop(connection);
    let before = database_fingerprint(&db);
    let error = match SqliteStore::new(&db) {
        Ok(_) => panic!("writer repaired a missing unique index"),
        Err(error) => error,
    };
    assert!(matches!(error, StoreError::SchemaInvariant(_)), "{error}");
    assert_eq!(database_fingerprint(&db), before);
}

#[test]
fn v1_writer_rejects_a_misleading_unique_usage_index() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    drop(SqliteStore::new(&db).unwrap());
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "DROP INDEX idx_usages_project_tool;
             CREATE UNIQUE INDEX idx_usages_project_tool ON tool_usages(id);",
        )
        .unwrap();
    drop(connection);
    let before = database_fingerprint(&db);

    let error = match SqliteStore::new(&db) {
        Ok(_) => panic!("writer accepted a unique index with the wrong identity"),
        Err(error) => error,
    };
    assert!(matches!(error, StoreError::SchemaInvariant(_)), "{error}");
    assert_eq!(database_fingerprint(&db), before);
}

#[test]
fn v1_writer_rejects_tool_usages_without_a_writer_id_column() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    drop(SqliteStore::new(&db).unwrap());
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "DROP INDEX idx_usages_project_tool;
             DROP INDEX idx_usages_project;
             DROP INDEX idx_usages_tool;
             DROP INDEX idx_usages_tool_nocase;
             ALTER TABLE tool_usages RENAME TO old_tool_usages;
             CREATE TABLE tool_usages (
                project_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                tool_key TEXT NOT NULL,
                last_used_at TEXT,
                session_count INTEGER DEFAULT 1,
                last_session_id TEXT
             );
             CREATE UNIQUE INDEX idx_usages_project_tool
             ON tool_usages(project_id, tool_key COLLATE NOCASE);
             DROP TABLE old_tool_usages;",
        )
        .unwrap();
    drop(connection);
    let before = database_fingerprint(&db);

    let error = match SqliteStore::new(&db) {
        Ok(_) => panic!("writer accepted tool_usages without its row identity"),
        Err(error) => error,
    };
    assert!(matches!(error, StoreError::SchemaInvariant(_)), "{error}");
    assert_eq!(database_fingerprint(&db), before);
}

#[test]
fn v1_writer_rejects_a_non_integer_tool_usage_primary_key() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    drop(SqliteStore::new(&db).unwrap());
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "DROP INDEX idx_usages_project_tool;
             DROP INDEX idx_usages_project;
             DROP INDEX idx_usages_tool;
             DROP INDEX idx_usages_tool_nocase;
             ALTER TABLE tool_usages RENAME TO old_tool_usages;
             CREATE TABLE tool_usages (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                tool_key TEXT NOT NULL,
                last_used_at TEXT,
                session_count INTEGER DEFAULT 1,
                last_session_id TEXT
             );
             CREATE UNIQUE INDEX idx_usages_project_tool
             ON tool_usages(project_id, tool_key COLLATE NOCASE);
             DROP TABLE old_tool_usages;",
        )
        .unwrap();
    drop(connection);
    let before = database_fingerprint(&db);

    let error = match SqliteStore::new(&db) {
        Ok(_) => panic!("writer accepted a non-integer tool usage row identity"),
        Err(error) => error,
    };
    assert!(matches!(error, StoreError::SchemaInvariant(_)), "{error}");
    assert_eq!(database_fingerprint(&db), before);
}

#[test]
fn failed_v0_migration_rolls_back_all_schema_and_fts_changes() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE projects (
                id TEXT PRIMARY KEY, path TEXT NOT NULL UNIQUE,
                last_accessed_at TEXT, first_seen_at TEXT,
                git_branch TEXT, git_remote_url TEXT
             );
             CREATE VIRTUAL TABLE projects_fts USING fts5(name, path);
             CREATE TABLE tool_usages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id TEXT NOT NULL, tool_name TEXT NOT NULL,
                tool_key TEXT NOT NULL, last_used_at TEXT,
                session_count INTEGER DEFAULT 1, last_session_id TEXT
             );
             CREATE TABLE sessions (
                id TEXT PRIMARY KEY, project_path TEXT NOT NULL,
                tool_key TEXT NOT NULL, tool_name TEXT NOT NULL,
                started_at TEXT, ended_at TEXT, session_id_from_tool TEXT
             );
             CREATE TABLE sessionatlas_meta (key TEXT PRIMARY KEY);
             INSERT INTO projects_fts (rowid, name, path)
             VALUES (1, 'fts-sentinel', 'fts-sentinel');",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO projects (id, path) VALUES ('rollback', ?1)",
            [abs_path(&["rollback"])],
        )
        .unwrap();
    connection
        .execute_batch(
            "INSERT INTO tool_usages
                (project_id, tool_name, tool_key, last_used_at, session_count)
             VALUES
                ('rollback', 'Codex', 'codex', '2026-07-01T12:00:00Z', 1),
                ('rollback', 'Codex', 'CODEX', '2026-07-02T12:00:00Z', 4);",
        )
        .unwrap();
    drop(connection);
    let before = database_fingerprint(&db);
    assert!(SqliteStore::new(&db).is_err());
    assert_eq!(database_fingerprint(&db), before);
    let connection = rusqlite::Connection::open(&db).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 0);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM projects_fts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row("SELECT name FROM projects_fts LIMIT 1", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "fts-sentinel"
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM tool_usages", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert!(
        connection
            .query_row::<i64, _, _>(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'sessionatlas_meta'",
                [],
                |row| row.get(0),
            )
            .unwrap()
            == 1
    );
    for table in [
        "project_content_files",
        "project_content_fts",
        "project_content_status",
    ] {
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
                    [table],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0,
            "migration must not leave {table}"
        );
    }
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('sessionatlas_meta')
                 WHERE name = 'value'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn store_schema_matches_expected_tables_and_indexes() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let _store = SqliteStore::new(&db).unwrap();
    drop(_store);

    let connection = rusqlite::Connection::open(&db).unwrap();
    let names: Vec<String> = connection
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type IN ('table', 'index') AND name NOT LIKE 'sqlite_%'",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for expected in [
        "sessionatlas_meta",
        "projects",
        "tool_usages",
        "sessions",
        "projects_fts",
        "project_content_files",
        "project_content_fts",
        "project_content_status",
        "idx_usages_project_tool",
        "idx_sessions_started",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "schema missing {expected}: {names:?}"
        );
    }
    let fts_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM projects_fts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(fts_count, 0);
}

#[test]
fn store_content_index_is_incremental_compressed_searchable_and_deletes_stale_files() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let project_dir = root.path().join("content-project");
    std::fs::create_dir_all(project_dir.join("src")).unwrap();
    let source = project_dir.join("src/main.rs");
    std::fs::write(&source, "pub fn searchable_component() {}\n".repeat(200)).unwrap();
    std::fs::write(
        project_dir.join("README.md"),
        "bounded full text architecture\n",
    )
    .unwrap();
    std::fs::write(project_dir.join(".env"), "PASSWORD=never_index_secret\n").unwrap();
    let project_path = path::normalize_native(&project_dir.to_string_lossy()).unwrap();

    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(
            &[project_at(
                &project_path,
                "content-project",
                utc(2026, 8, 15),
                &[usage("codex", utc(2026, 8, 16), 1, Some("session"))],
            )],
            &["codex"],
        )
        .unwrap();
    let options = ContentIndexOptions {
        max_walk_entries: 100,
        max_files_per_project: 20,
        max_file_bytes: 32 * 1024,
        max_project_bytes: 64 * 1024,
        max_preview_bytes: 8 * 1024,
    };
    let first = store.refresh_project_content_index_with(options).unwrap();

    assert_eq!(first.projects_scanned, 1);
    assert_eq!(first.files_indexed, 2);
    assert_eq!(
        store
            .list_projects(Some("searchable_component"), None, 10)
            .unwrap()[0]
            .id,
        "content-project"
    );
    assert!(store
        .list_projects(Some("never_index_secret"), None, 10)
        .unwrap()
        .is_empty());

    let second = store.refresh_project_content_index_with(options).unwrap();
    assert_eq!(second.files_indexed, 0);
    assert_eq!(second.files_reused, 2);
    drop(store);

    let connection = rusqlite::Connection::open(&db).unwrap();
    let (compressed_bytes, indexed_bytes): (i64, i64) = connection
        .query_row(
            "SELECT LENGTH(compressed_preview), indexed_bytes
             FROM project_content_files WHERE relative_path = 'src/main.rs'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let raw_body: Option<String> = connection
        .query_row("SELECT body FROM project_content_fts LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(compressed_bytes < indexed_bytes);
    assert!(
        raw_body.is_none(),
        "contentless FTS must not retain raw source"
    );
    drop(connection);

    std::fs::write(&source, "pub fn replacement_symbol() {}\n").unwrap();
    let mut store = SqliteStore::new(&db).unwrap();
    let changed = store.refresh_project_content_index_with(options).unwrap();
    assert_eq!(changed.files_indexed, 1);
    assert!(store
        .list_projects(Some("searchable_component"), None, 10)
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .list_projects(Some("replacement_symbol"), None, 10)
            .unwrap()
            .len(),
        1
    );

    std::fs::remove_file(&source).unwrap();
    let removed = store.refresh_project_content_index_with(options).unwrap();
    assert_eq!(removed.files_removed, 1);
    assert!(store
        .list_projects(Some("replacement_symbol"), None, 10)
        .unwrap()
        .is_empty());
}

#[cfg(unix)]
#[test]
fn store_repairs_default_data_root_database_and_sqlite_sidecar_modes() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let data = root.path().join(".sessionatlas");
    std::fs::create_dir(&data).unwrap();
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
    let db = data.join("index.db");
    std::fs::write(&db, []).unwrap();
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();

    drop(SqliteStore::new(&db).unwrap());

    assert_eq!(
        std::fs::metadata(&data).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&db).unwrap().permissions().mode() & 0o777,
        0o600
    );
    for suffix in ["-journal", "-wal", "-shm"] {
        let sidecar = std::path::PathBuf::from(format!("{}{suffix}", db.display()));
        if sidecar.exists() {
            assert_eq!(
                std::fs::metadata(sidecar).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

#[test]
fn stale_content_sanitizer_purges_old_rows_and_forces_a_safe_rebuild() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let project_dir = root.path().join("privacy-migration-project");
    std::fs::create_dir_all(&project_dir).unwrap();
    let source = project_dir.join("main.rs");
    let secret = "legacy_leaked_token_value_123456789";
    std::fs::write(
        &source,
        format!("GITHUB_TOKEN={secret}\npub fn safe_rebuild_symbol() {{}}\n"),
    )
    .unwrap();
    let project_path = path::normalize_native(&project_dir.to_string_lossy()).unwrap();

    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(
            &[project_at(
                &project_path,
                "privacy-migration",
                utc(2026, 8, 27),
                &[usage("codex", utc(2026, 8, 27), 1, Some("session"))],
            )],
            &["codex"],
        )
        .unwrap();
    store.refresh_project_content_index().unwrap();
    drop(store);

    let connection = rusqlite::Connection::open(&db).unwrap();
    let rowid: i64 = connection
        .query_row("SELECT id FROM project_content_files LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    let legacy_preview = lz4_flex::compress_prepend_size(secret.as_bytes());
    connection
        .execute(
            "UPDATE project_content_files SET compressed_preview = ?1 WHERE id = ?2",
            rusqlite::params![legacy_preview, rowid],
        )
        .unwrap();
    connection
        .execute("DELETE FROM project_content_fts WHERE rowid = ?1", [rowid])
        .unwrap();
    connection
        .execute(
            "INSERT INTO project_content_fts (rowid, relative_path, body)
             VALUES (?1, 'main.rs', ?2)",
            rusqlite::params![rowid, secret],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE sessionatlas_meta SET value = '1'
             WHERE key = 'content_index_sanitizer_version'",
            [],
        )
        .unwrap();
    drop(connection);

    let mut store = SqliteStore::new(&db).unwrap();
    let connection = rusqlite::Connection::open(&db).unwrap();
    assert!(content_index_sanitizer_is_current(&connection).unwrap());
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM project_content_files", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    assert_eq!(CONTENT_INDEX_SANITIZER_VERSION, 2);
    drop(connection);
    assert!(
        !std::fs::read(&db)
            .unwrap()
            .windows(secret.len())
            .any(|window| window == secret.as_bytes()),
        "VACUUM must remove the legacy plaintext from database pages"
    );

    let rebuilt = store.refresh_project_content_index().unwrap();
    assert_eq!(rebuilt.files_indexed, 1);
    assert!(store
        .list_projects(Some(secret), None, 10)
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .list_projects(Some("safe_rebuild_symbol"), None, 10)
            .unwrap()
            .len(),
        1
    );
    drop(store);

    let connection = rusqlite::Connection::open(&db).unwrap();
    let preview: Vec<u8> = connection
        .query_row(
            "SELECT compressed_preview FROM project_content_files LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let preview = lz4_flex::decompress_size_prepended(&preview).unwrap();
    assert!(!preview
        .windows(secret.len())
        .any(|window| window == secret.as_bytes()));
}

#[test]
fn store_recomputes_missing_directory_state_without_rescanning() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let project_dir = root.path().join("project-that-will-be-removed");
    std::fs::create_dir_all(&project_dir).unwrap();
    let project_path = path::normalize_native(&project_dir.to_string_lossy()).unwrap();

    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(
            &[project_at(
                &project_path,
                "live-path",
                utc(2026, 8, 15),
                &[usage("claude", utc(2026, 8, 16), 1, Some("session"))],
            )],
            &["claude"],
        )
        .unwrap();
    assert!(!store.list_projects(None, None, 100).unwrap()[0].path_missing);
    assert!(
        !store
            .get_project_by_path(&project_path)
            .unwrap()
            .unwrap()
            .path_missing
    );

    std::fs::remove_dir(&project_dir).unwrap();
    assert!(store.list_projects(None, None, 100).unwrap()[0].path_missing);
    assert!(
        store
            .get_project_by_path(&project_path)
            .unwrap()
            .unwrap()
            .path_missing
    );
}

#[test]
fn store_repeating_identical_snapshot_keeps_identity_and_single_usage() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let project_path = abs_path(&["project"]);
    let first_seen = utc(2026, 7, 1);
    let last_used = utc(2026, 7, 30);

    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(
            &[project_at(
                &project_path,
                "incoming-one",
                first_seen,
                &[usage("codex", last_used, 2, Some("session-2"))],
            )],
            &["codex"],
        )
        .unwrap();

    let first_list = store.list_projects(None, None, 100).unwrap();
    let first_id = first_list[0].id.clone();
    let first_seen_stored = first_list[0].first_seen_at;

    store
        .replace_tool_snapshots(
            &[project_at(
                &project_path,
                "incoming-two",
                utc(2026, 7, 31),
                &[usage("codex", last_used, 2, Some("session-2"))],
            )],
            &["codex"],
        )
        .unwrap();

    let second = &store.list_projects(None, None, 100).unwrap()[0];
    assert_eq!(second.id, first_id);
    assert_eq!(second.first_seen_at, first_seen_stored);
    assert_eq!(second.tool_usages.len(), 1);
    assert_eq!(second.tool_usages[0].session_count, 2);
    assert_eq!(
        second.tool_usages[0].last_session_id.as_deref(),
        Some("session-2")
    );
}

#[test]
fn store_partial_and_empty_snapshots_replace_only_scanned_tools_and_remove_orphans() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let project_path = abs_path(&["project"]);
    let first_seen = utc(2026, 7, 1);

    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(
            &[project_at(
                &project_path,
                "project-one",
                first_seen,
                &[
                    usage("claude", utc(2026, 7, 20), 3, Some("claude-3")),
                    usage("codex", utc(2026, 7, 21), 2, Some("codex-2")),
                ],
            )],
            &["claude", "codex"],
        )
        .unwrap();

    store
        .replace_tool_snapshots(
            &[project_at(
                &project_path,
                "ignored-new-id",
                utc(2026, 7, 31),
                &[usage("codex", utc(2026, 7, 25), 4, Some("codex-4"))],
            )],
            &["codex"],
        )
        .unwrap();

    let after_partial = &store.list_projects(None, None, 100).unwrap()[0];
    assert_eq!(after_partial.first_seen_at, first_seen);
    assert_eq!(after_partial.last_accessed_at, utc(2026, 7, 25));
    assert_eq!(after_partial.tool_usages.len(), 2);
    let claude = after_partial
        .tool_usages
        .iter()
        .find(|usage| usage.tool_key == "claude")
        .unwrap();
    let codex = after_partial
        .tool_usages
        .iter()
        .find(|usage| usage.tool_key == "codex")
        .unwrap();
    assert_eq!(claude.session_count, 3);
    assert_eq!(codex.session_count, 4);

    store.replace_tool_snapshots(&[], &["codex"]).unwrap();
    let after_codex_empty = &store.list_projects(None, None, 100).unwrap()[0];
    assert_eq!(after_codex_empty.last_accessed_at, utc(2026, 7, 20));
    assert_eq!(after_codex_empty.tool_usages.len(), 1);
    assert_eq!(after_codex_empty.tool_usages[0].tool_key, "claude");

    store.replace_tool_snapshots(&[], &["claude"]).unwrap();
    assert!(store.list_projects(None, None, 100).unwrap().is_empty());
    assert!(store
        .list_projects(Some("project"), None, 100)
        .unwrap()
        .is_empty());
}

#[test]
fn store_snapshot_failure_rolls_back_every_project_usage_and_fts_change() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let baseline_path = abs_path(&["baseline"]);
    let rejected_path = abs_path(&["must-not-survive"]);

    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(
            &[project_at(
                &baseline_path,
                "baseline-id",
                utc(2026, 7, 1),
                &[usage("codex", utc(2026, 7, 20), 1, Some("base"))],
            )],
            &["codex"],
        )
        .unwrap();

    {
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER reject_blocked_usage
                 BEFORE INSERT ON tool_usages
                 WHEN NEW.tool_key = 'blocked'
                 BEGIN
                     SELECT RAISE(ABORT, 'forced snapshot failure');
                 END;",
            )
            .unwrap();
    }

    let result = store.replace_tool_snapshots(
        &[project_at(
            &rejected_path,
            "rejected-id",
            utc(2026, 7, 2),
            &[
                usage("codex", utc(2026, 7, 30), 1, Some("new")),
                usage("blocked", utc(2026, 7, 30), 1, Some("blocked")),
            ],
        )],
        &["codex", "blocked"],
    );
    assert!(matches!(result, Err(StoreError::Sql(_))), "{result:?}");

    let remaining = &store.list_projects(None, None, 100).unwrap()[0];
    assert_eq!(
        remaining.path,
        path::normalize_native(&baseline_path).unwrap()
    );
    assert!(store
        .list_projects(Some("must"), None, 100)
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .list_projects(Some("baseline"), None, 100)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn store_opening_legacy_database_collapses_duplicate_tool_usages() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("legacy.db");
    let project_path = abs_path(&["demo", "project"]);
    {
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE projects (
                    id TEXT PRIMARY KEY,
                    path TEXT NOT NULL UNIQUE,
                    last_accessed_at TEXT,
                    first_seen_at TEXT,
                    git_branch TEXT,
                    git_remote_url TEXT
                );
                CREATE TABLE tool_usages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    project_id TEXT NOT NULL,
                    tool_name TEXT NOT NULL,
                    tool_key TEXT NOT NULL,
                    last_used_at TEXT,
                    session_count INTEGER DEFAULT 1,
                    last_session_id TEXT
                );",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO projects (id, path, last_accessed_at, first_seen_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    "project-id",
                    project_path,
                    "2026-07-30T12:00:00.0000000Z",
                    "2026-07-01T12:00:00.0000000Z"
                ],
            )
            .unwrap();
        connection
            .execute_batch(
                "INSERT INTO tool_usages
                    (project_id, tool_name, tool_key, last_used_at, session_count, last_session_id)
                 VALUES
                    ('project-id', 'Codex', 'codex', '2026-07-20T12:00:00.0000000Z', 5, 'older'),
                    ('project-id', 'Codex', 'codex', '2026-07-30T12:00:00.0000000Z', 2, 'newest');",
            )
            .unwrap();
    }

    let store = SqliteStore::new(&db).unwrap();
    let listed = store.list_projects(None, None, 100).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].tool_usages.len(), 1);
    let usage = &listed[0].tool_usages[0];
    assert_eq!(usage.session_count, 5);
    assert_eq!(usage.last_used_at, utc(2026, 7, 30));
    assert_eq!(usage.last_session_id.as_deref(), Some("newest"));
}

#[test]
fn store_rejects_snapshot_usage_outside_declared_successful_tools_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let project_path = abs_path(&["project"]);

    let mut store = SqliteStore::new(&db).unwrap();
    let result = store.replace_tool_snapshots(
        &[project_at(
            &project_path,
            "id",
            utc(2026, 7, 1),
            &[usage("claude", utc(2026, 7, 30), 1, Some("s1"))],
        )],
        &["codex"],
    );
    assert!(matches!(
        result,
        Err(StoreError::UndeclaredUsageTool { .. })
    ));
    assert!(store.list_projects(None, None, 100).unwrap().is_empty());
}

#[test]
fn store_snapshot_validation_rejects_empty_and_malformed_input() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let project_path = abs_path(&["project"]);

    let mut store = SqliteStore::new(&db).unwrap();
    let ok_usage = usage("codex", utc(2026, 7, 30), 1, None);

    assert!(matches!(
        store.replace_tool_snapshots(
            &[project_at(
                &project_path,
                "id",
                utc(2026, 7, 1),
                std::slice::from_ref(&ok_usage)
            )],
            &[]
        ),
        Err(StoreError::ScannedToolKeysEmpty)
    ));
    assert!(matches!(
        store.replace_tool_snapshots(&[], &["  "]),
        Err(StoreError::EmptyScannedToolKey)
    ));

    let mut no_usages = project_at(
        &project_path,
        "id",
        utc(2026, 7, 1),
        std::slice::from_ref(&ok_usage),
    );
    no_usages.tool_usages.clear();
    assert!(matches!(
        store.replace_tool_snapshots(&[no_usages], &["codex"]),
        Err(StoreError::NoToolUsages(_))
    ));

    let mut duplicate = project_at(
        &project_path,
        "id",
        utc(2026, 7, 1),
        &[ok_usage.clone(), ok_usage],
    );
    duplicate.tool_usages[0].tool_key = "CODEX".to_string();
    assert!(matches!(
        store.replace_tool_snapshots(&[duplicate], &["codex"]),
        Err(StoreError::DuplicateUsageTool { .. })
    ));

    assert!(matches!(
        store.replace_tool_snapshots(
            &[project_at(
                &project_path,
                "id",
                utc(2026, 7, 1),
                &[usage("codex", utc(2026, 7, 30), -1, None)]
            )],
            &["codex"]
        ),
        Err(StoreError::NegativeSessionCount { .. })
    ));

    let empty_path = project_at(
        "",
        "id",
        utc(2026, 7, 1),
        &[usage("codex", utc(2026, 7, 30), 1, None)],
    );
    assert!(matches!(
        store.replace_tool_snapshots(&[empty_path], &["codex"]),
        Err(StoreError::EmptyProjectPath)
    ));

    let relative = project_at(
        "relative/path",
        "id",
        utc(2026, 7, 1),
        &[usage("codex", utc(2026, 7, 30), 1, None)],
    );
    assert!(matches!(
        store.replace_tool_snapshots(&[relative], &["codex"]),
        Err(StoreError::InvalidProjectPath(_))
    ));

    assert!(store.list_projects(None, None, 100).unwrap().is_empty());
}

#[cfg(windows)]
#[test]
fn store_windows_path_identity_remains_stable_when_path_casing_changes() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let project_path = abs_path(&["MixedCaseProject"]);

    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(
            &[project_at(
                &project_path,
                "stable-id",
                utc(2026, 7, 1),
                &[usage("codex", utc(2026, 7, 20), 1, Some("old"))],
            )],
            &["codex"],
        )
        .unwrap();
    store
        .replace_tool_snapshots(
            &[project_at(
                &project_path.to_uppercase(),
                "new-id",
                utc(2026, 7, 2),
                &[usage("codex", utc(2026, 7, 30), 2, Some("new"))],
            )],
            &["codex"],
        )
        .unwrap();

    let project = &store.list_projects(None, None, 100).unwrap()[0];
    assert_eq!(project.id, "stable-id");
    assert_eq!(project.tool_usages[0].session_count, 2);
}

#[test]
fn store_snapshot_accepts_usage_with_no_known_native_session_id() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let project_path = abs_path(&["aider-project"]);

    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(
            &[project_at(
                &project_path,
                "aider-id",
                utc(2026, 7, 1),
                &[usage("aider", utc(2026, 7, 30), 0, None)],
            )],
            &["aider"],
        )
        .unwrap();

    let listed = store.list_projects(None, None, 100).unwrap();
    let usage = &listed[0].tool_usages[0];
    assert_eq!(usage.session_count, 0);
    assert_eq!(usage.last_session_id, None);
}

#[test]
fn store_exact_path_lookup_is_not_limited_by_recency_or_list_window() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let oldest_path = abs_path(&["oldest-project"]);
    let base_timestamp = 1_782_892_800_i64;

    let mut projects = Vec::new();
    for index in 0..125 {
        let (path_text, key, session_id) = if index == 0 {
            (
                oldest_path.clone(),
                "fixture".to_string(),
                "session-000".to_string(),
            )
        } else {
            (
                abs_path(&[&format!("project-{index:03}")]),
                "codex".to_string(),
                format!("session-{index:03}"),
            )
        };
        let at = DateTime::from_timestamp(base_timestamp + index * 60, 0).unwrap();
        projects.push(project_at(
            &path_text,
            &format!("project-{index:03}"),
            utc(2026, 7, 1),
            &[usage(&key, at, 1, Some(&session_id))],
        ));
    }

    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(&projects, &["fixture", "codex"])
        .unwrap();

    let separator = if cfg!(windows) { "\\" } else { "/" };
    let mut lookup = format!("{oldest_path}{separator}");
    if cfg!(windows) {
        lookup = lookup.to_uppercase();
    }

    let project = store.get_project_by_path(&lookup).unwrap().unwrap();
    assert_eq!(project.path, path::normalize_native(&oldest_path).unwrap());
    assert_eq!(project.tool_usages.len(), 1);
    assert_eq!(project.tool_usages[0].tool_key, "fixture");
    assert_eq!(
        project.tool_usages[0].last_session_id.as_deref(),
        Some("session-000")
    );
}

#[test]
fn store_search_treats_fts_operators_and_punctuation_as_literal_separators() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let project_path = abs_path(&["alpha-beta"]);

    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(
            &[project_at(
                &project_path,
                "alpha-beta",
                utc(2026, 7, 1),
                &[usage("codex", utc(2026, 7, 30), 1, Some("session"))],
            )],
            &["codex"],
        )
        .unwrap();

    assert_eq!(
        store
            .list_projects(Some("alpha-beta"), None, 100)
            .unwrap()
            .len(),
        1
    );
    assert!(store
        .list_projects(Some("\" OR * -"), None, 100)
        .unwrap()
        .is_empty());
}

#[test]
fn store_native_root_round_trips_through_snapshot_lookup_and_fts_rebuild() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let drive_root = if cfg!(windows) {
        "C:\\".to_string()
    } else {
        "/".to_string()
    };

    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(
            &[project_at(
                &drive_root,
                "root-id",
                utc(2026, 8, 1),
                &[usage("codex", utc(2026, 8, 2), 1, Some("fixture"))],
            )],
            &["codex"],
        )
        .unwrap();
    store.rebuild_search_index().unwrap();
    store.rebuild_search_index().unwrap();

    let listed = store.list_projects(None, None, 100).unwrap();
    assert_eq!(listed.len(), 1);
    let normalized_root = path::normalize_native(&drive_root).unwrap();
    assert_eq!(listed[0].path, normalized_root);
    let name = path::display_name_native(&normalized_root).unwrap();
    assert!(!name.is_empty());
    assert_eq!(
        store.get_project_by_path(&drive_root).unwrap().unwrap().id,
        "root-id"
    );
    assert_eq!(
        store.list_projects(Some(&name), None, 100).unwrap().len(),
        1
    );
}

#[test]
fn store_snapshot_duplicate_validation_uses_final_native_path_before_mutation() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let path_text = abs_path(&["duplicate"]);
    let separator = if cfg!(windows) { "\\" } else { "/" };

    let mut store = SqliteStore::new(&db).unwrap();
    let result = store.replace_tool_snapshots(
        &[
            project_at(
                &path_text,
                "one",
                utc(2026, 8, 1),
                &[usage("codex", utc(2026, 8, 2), 1, Some("a"))],
            ),
            project_at(
                &format!("{path_text}{separator}"),
                "two",
                utc(2026, 8, 1),
                &[usage("codex", utc(2026, 8, 2), 1, Some("b"))],
            ),
        ],
        &["codex"],
    );
    assert!(matches!(result, Err(StoreError::DuplicateProjectPath(_))));
    assert!(store.list_projects(None, None, 100).unwrap().is_empty());
}

#[test]
fn store_legacy_upsert_and_session_recording_use_canonical_native_paths() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let path_text = abs_path(&["MixedCase"]);
    let separator = if cfg!(windows) { "\\" } else { "/" };

    let mut store = SqliteStore::new(&db).unwrap();
    store
        .upsert_project(&project_at(
            &format!("{path_text}{separator}"),
            "stable",
            utc(2026, 8, 1),
            &[usage("codex", utc(2026, 8, 2), 1, Some("s"))],
        ))
        .unwrap();
    let variant = if cfg!(windows) {
        path_text.to_uppercase()
    } else {
        path_text.clone()
    };
    store
        .upsert_project(&project_at(
            &variant,
            "replacement",
            utc(2026, 8, 1),
            &[usage("codex", utc(2026, 8, 2), 1, Some("s"))],
        ))
        .unwrap();

    let listed = store.list_projects(None, None, 100).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "stable");
    assert_eq!(listed[0].path, path::normalize_native(&path_text).unwrap());

    store
        .record_session(&session(
            "session",
            &format!("{path_text}{separator}"),
            utc(2026, 8, 3),
        ))
        .unwrap();
    let recent = store.get_recent_sessions(10).unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(
        recent[0].project_path,
        path::normalize_native(&path_text).unwrap()
    );
}

#[cfg(windows)]
#[test]
fn store_legacy_anomaly_inspection_is_read_only_and_reports_invalid_and_colliding_rows() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    {
        let _store = SqliteStore::new(&db).unwrap();
    }
    {
        let connection = rusqlite::Connection::open(&db).unwrap();
        for (id, stored_path) in [
            ("invalid", "C:"),
            ("upper", r"C:\Repo"),
            ("lower", r"c:\repo\"),
        ] {
            connection
                .execute(
                    "INSERT INTO projects (id, path) VALUES (?1, ?2)",
                    rusqlite::params![id, stored_path],
                )
                .unwrap();
        }
    }

    let store = SqliteStore::new(&db).unwrap();
    let anomalies = store.inspect_project_path_anomalies().unwrap();
    assert!(
        anomalies
            .iter()
            .any(|item| item.contains("invalid legacy path")),
        "{anomalies:?}"
    );
    assert!(
        anomalies.iter().any(|item| item.contains("collide")),
        "{anomalies:?}"
    );
    drop(store);

    let connection = rusqlite::Connection::open(&db).unwrap();
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 3);
}

#[test]
fn store_isolated_database_path_creates_no_other_business_files() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let project_path = abs_path(&["project"]);
    {
        let mut store = SqliteStore::new(&db).unwrap();
        store
            .replace_tool_snapshots(
                &[project_at(
                    &project_path,
                    "id",
                    utc(2026, 8, 1),
                    &[usage("codex", utc(2026, 8, 2), 1, Some("s"))],
                )],
                &["codex"],
            )
            .unwrap();
        store
            .record_session(&session("s", &project_path, utc(2026, 8, 3)))
            .unwrap();
    }

    let mut files: Vec<String> = std::fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, vec!["index.db".to_string()]);
}

#[test]
fn store_list_filters_by_tool_and_validates_limit() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let p1 = abs_path(&["one"]);
    let p2 = abs_path(&["two"]);

    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(
            &[
                project_at(
                    &p1,
                    "one-id",
                    utc(2026, 8, 1),
                    &[usage("codex", utc(2026, 8, 2), 1, Some("a"))],
                ),
                project_at(
                    &p2,
                    "two-id",
                    utc(2026, 8, 1),
                    &[usage("claude", utc(2026, 8, 3), 1, Some("b"))],
                ),
            ],
            &["codex", "claude"],
        )
        .unwrap();

    let codex = store.list_projects(None, Some("codex"), 100).unwrap();
    assert_eq!(codex.len(), 1);
    assert_eq!(codex[0].id, "one-id");
    assert_eq!(
        store.list_projects(None, Some("CODEX"), 100).unwrap().len(),
        1,
        "tool key filter is case-insensitive"
    );

    assert!(matches!(
        store.list_projects(None, None, 0),
        Err(StoreError::InvalidLimit)
    ));
    assert!(matches!(
        store.list_projects(None, None, 10_001),
        Err(StoreError::InvalidLimit)
    ));
}

#[test]
fn store_record_and_recent_sessions_preserve_ended_timestamps_and_ordering() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let path_text = abs_path(&["project"]);
    let store = SqliteStore::new(&db).unwrap();

    store
        .record_session(&Session {
            id: "older".to_string(),
            project_path: path_text.clone(),
            tool_key: "codex".to_string(),
            tool_name: "Codex".to_string(),
            started_at: utc(2026, 7, 1),
            ended_at: Some(utc(2026, 7, 1)),
            session_id_from_tool: None,
        })
        .unwrap();
    store
        .record_session(&Session {
            id: "newer".to_string(),
            project_path: path_text,
            tool_key: "claude".to_string(),
            tool_name: "Claude".to_string(),
            started_at: utc(2026, 7, 2),
            ended_at: None,
            session_id_from_tool: Some("resume-me".to_string()),
        })
        .unwrap();

    let recent = store.get_recent_sessions(10).unwrap();
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].id, "newer");
    assert_eq!(recent[0].ended_at, None);
    assert_eq!(recent[0].session_id_from_tool.as_deref(), Some("resume-me"));
    assert_eq!(recent[1].id, "older");
    assert_eq!(recent[1].ended_at, Some(utc(2026, 7, 1)));

    assert!(matches!(
        store.get_recent_sessions(0),
        Err(StoreError::InvalidLimit)
    ));
}

#[test]
fn index_reader_queries_without_mutating_database_or_creating_sidecars() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let path_text = abs_path(&["reader-project"]);
    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(
            &[project_at(
                &path_text,
                "reader-project-id",
                utc(2026, 7, 1),
                &[usage("codex", utc(2026, 7, 2), 1, None)],
            )],
            &["codex"],
        )
        .unwrap();
    store
        .record_session(&session("reader-session", &path_text, utc(2026, 7, 3)))
        .unwrap();
    drop(store);

    let before = database_fingerprint(&db);
    let reader = IndexReader::open_existing(&db).unwrap();
    assert_eq!(database_fingerprint(&db), before);
    assert_eq!(reader.list_projects(None, None, 10).unwrap().len(), 1);
    assert_eq!(
        reader
            .list_projects(Some("reader"), None, 10)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(reader.get_recent_sessions(10).unwrap().len(), 1);
    assert_eq!(database_fingerprint(&db), before);
    drop(reader);
    assert_eq!(database_fingerprint(&db), before);
}

#[test]
fn index_reader_rejects_incomplete_schema_without_repairing_it() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE projects (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                last_accessed_at TEXT,
                first_seen_at TEXT,
                git_branch TEXT,
                git_remote_url TEXT
            );",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO projects
                (id, path, last_accessed_at, first_seen_at, git_branch, git_remote_url)
             VALUES (?1, ?2, ?3, ?4, NULL, NULL)",
            rusqlite::params![
                "incomplete-project",
                abs_path(&["incomplete"]),
                utc(2026, 7, 1).to_rfc3339(),
                utc(2026, 7, 1).to_rfc3339()
            ],
        )
        .unwrap();
    drop(connection);

    let before = database_fingerprint(&db);
    let error = match IndexReader::open_existing(&db) {
        Ok(_) => panic!("incomplete schema unexpectedly opened"),
        Err(error) => error,
    };
    assert!(matches!(error, StoreError::SchemaInvariant(_)), "{error}");
    assert_eq!(database_fingerprint(&db), before);
}

#[test]
fn index_reader_rejects_incomplete_v1_schema_without_repairing_it() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE projects (id TEXT PRIMARY KEY, path TEXT NOT NULL);
             PRAGMA user_version = 1;",
        )
        .unwrap();
    drop(connection);
    let before = database_fingerprint(&db);
    let error = match IndexReader::open_existing(&db) {
        Ok(_) => panic!("incomplete v1 schema unexpectedly opened"),
        Err(error) => error,
    };
    assert!(matches!(error, StoreError::SchemaInvariant(_)), "{error}");
    assert_eq!(database_fingerprint(&db), before);
}

#[test]
fn index_reader_searches_legacy_name_index_without_optional_content_tables() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let path_text = abs_path(&["legacy-reader-name"]);
    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(
            &[project_at(
                &path_text,
                "legacy-reader-id",
                utc(2026, 7, 1),
                &[usage("codex", utc(2026, 7, 2), 1, None)],
            )],
            &["codex"],
        )
        .unwrap();
    drop(store);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "DROP TABLE project_content_fts;
             DROP TABLE project_content_files;
             DROP TABLE project_content_status;
             PRAGMA user_version = 0;",
        )
        .unwrap();
    drop(connection);

    let before = database_fingerprint(&db);
    let reader = IndexReader::open_existing(&db).unwrap();
    let projects = reader
        .list_projects(Some("legacy-reader"), None, 10)
        .unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].id, "legacy-reader-id");
    assert_eq!(database_fingerprint(&db), before);
    drop(reader);
    assert_eq!(database_fingerprint(&db), before);
}

#[test]
fn index_reader_hides_stale_content_terms_but_keeps_project_name_search() {
    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    let path_text = abs_path(&["visible-project-name"]);
    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(
            &[project_at(
                &path_text,
                "stale-content-id",
                utc(2026, 7, 1),
                &[usage("codex", utc(2026, 7, 2), 1, None)],
            )],
            &["codex"],
        )
        .unwrap();
    drop(store);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute(
            "INSERT INTO project_content_files
                (project_id, relative_path, modified_ns, file_size, indexed_bytes, compressed_preview)
             VALUES (?1, 'secret.txt', 1, 1, 1, X'00')",
            ["stale-content-id"],
        )
        .unwrap();
    let content_rowid = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO project_content_fts (rowid, relative_path, body)
             VALUES (?1, 'secret.txt', 'archivedcredentialword')",
            [content_rowid],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE sessionatlas_meta SET value = '1'
             WHERE key = 'content_index_sanitizer_version'",
            [],
        )
        .unwrap();
    drop(connection);

    let before = database_fingerprint(&db);
    let reader = IndexReader::open_existing(&db).unwrap();
    assert!(reader
        .list_projects(Some("archivedcredentialword"), None, 10)
        .unwrap()
        .is_empty());
    let projects = reader
        .list_projects(Some("visible-project"), None, 10)
        .unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(database_fingerprint(&db), before);
    drop(reader);
    assert_eq!(database_fingerprint(&db), before);
}

#[cfg(unix)]
#[test]
fn index_reader_does_not_change_unix_directory_or_database_modes() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let root = tempfile::tempdir().unwrap();
    let db = db_path(&root);
    drop(SqliteStore::new(&db).unwrap());
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o640)).unwrap();
    let before_dir_mode = std::fs::metadata(root.path()).unwrap().mode() & 0o777;
    let before_db_mode = std::fs::metadata(&db).unwrap().mode() & 0o777;

    let reader = IndexReader::open_existing(&db).unwrap();
    assert!(reader.list_projects(None, None, 10).unwrap().is_empty());
    assert_eq!(
        std::fs::metadata(root.path()).unwrap().mode() & 0o777,
        before_dir_mode
    );
    assert_eq!(
        std::fs::metadata(&db).unwrap().mode() & 0o777,
        before_db_mode
    );
}
