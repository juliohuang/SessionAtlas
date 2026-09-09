use super::*;
use crate::model::{Session, ToolUsage};

const BODY: &str = "alphaquartznebula source documentation for a harmless synthetic fixture\n";

#[test]
fn search_keeps_policy_and_content_in_one_snapshot() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    let (_root, db, mut store) = fixture();
    store
        .connection
        .execute_batch("PRAGMA journal_mode=WAL; DELETE FROM project_content_fts;")
        .unwrap();
    let reader = IndexReader::open_existing(&db).unwrap();
    let fired = Arc::new(AtomicBool::new(false));
    let triggered = fired.clone();
    // Commit a partial rebuild exactly when the search prepares its content
    // query, after it has checked the ready policy. WAL lets the writer commit
    // while a correct reader retains its earlier snapshot.
    reader.connection.authorizer(Some(move |context: AuthContext<'_>| {
        if matches!(context.action, AuthAction::Read { table_name: "project_content_files", column_name: "project_id" })
            && !triggered.swap(true, Ordering::SeqCst)
        {
            let tx = store.connection.transaction().unwrap();
            set_meta(&tx, PHASE_KEY, "rebuildPending").unwrap();
            tx.execute("INSERT INTO project_content_fts(rowid,relative_path,body) SELECT id,relative_path,?1 FROM project_content_files", [BODY]).unwrap();
            tx.commit().unwrap();
        }
        Authorization::Allow
    })).unwrap();
    assert_eq!(
        content_hits(&reader),
        0,
        "must not expose the partial rebuild"
    );
    assert!(fired.load(Ordering::SeqCst), "writer interleaving must run");
    assert_eq!(
        reader.content_index_status().unwrap().phase,
        ContentIndexPhase::RebuildPending
    );
    assert_eq!(
        content_hits(&reader),
        0,
        "later searches must observe the closed gate"
    );
}

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, SqliteStore) {
    let root = tempfile::tempdir().unwrap();
    let project_dir = root.path().join("fixture-project");
    std::fs::create_dir(&project_dir).unwrap();
    std::fs::write(project_dir.join("README.md"), BODY).unwrap();
    let db = root.path().join("index.db");
    let project = Project {
        id: "fixture".into(),
        path: project_dir.to_string_lossy().into_owned(),
        last_accessed_at: Utc::now(),
        tool_usages: vec![ToolUsage {
            tool_key: "codex".into(),
            tool_name: "Codex".into(),
            last_used_at: Utc::now(),
            ..ToolUsage::default()
        }],
        ..Project::default()
    };
    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(std::slice::from_ref(&project), &["codex"])
        .unwrap();
    store
        .record_session(&Session {
            project_path: project.path,
            tool_key: "codex".into(),
            tool_name: "Codex".into(),
            started_at: Utc::now(),
            ..Session::default()
        })
        .unwrap();
    store.refresh_project_content_index().unwrap();
    (root, db, store)
}

fn content_hits(reader: &IndexReader) -> usize {
    reader
        .list_projects(Some("alphaquartznebula"), None, 10)
        .unwrap()
        .len()
}

fn assert_empty(store: &SqliteStore) {
    for table in [
        "project_content_files",
        "project_content_fts",
        "project_content_status",
    ] {
        let count: i64 = store
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
}

#[test]
fn missing_status_and_mutations_never_create_first_run_artifacts() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("missing/index.db");
    let status = content_index_status_at(&db).unwrap();
    assert!(!status.index_exists);
    assert!(!status.searchable);
    for action in [
        ContentIndexAction::Enable,
        ContentIndexAction::Disable,
        ContentIndexAction::Clear,
        ContentIndexAction::Rebuild,
    ] {
        assert!(change_content_index_at(&db, action).is_err());
    }
    assert!(!root.path().join("missing").exists());
    assert!(content_index_status_at(root.path()).is_err());
}

#[test]
fn existing_status_is_read_only_and_legacy_policy_defaults_to_enabled() {
    let (_root, db, store) = fixture();
    drop(store);
    let before = std::fs::read(&db).unwrap();
    let modified = std::fs::metadata(&db).unwrap().modified().unwrap();
    let lock = db.with_extension("db.lock");
    let lock_modified = std::fs::metadata(&lock).unwrap().modified().unwrap();
    let status = content_index_status_at(&db).unwrap();
    assert!(status.enabled && status.searchable);
    assert_eq!(status.indexed_files, 1);
    assert_eq!(std::fs::read(&db).unwrap(), before);
    assert_eq!(
        std::fs::metadata(&db).unwrap().modified().unwrap(),
        modified
    );
    assert_eq!(
        std::fs::metadata(lock).unwrap().modified().unwrap(),
        lock_modified
    );
}

#[test]
fn disable_survives_reopen_and_scan_preserving_source_sessions_and_generation() {
    let (root, db, mut store) = fixture();
    let reader = IndexReader::open_existing(&db).unwrap();
    let generation = reader.scan_generation().unwrap();
    let sessions = reader.get_recent_sessions(10).unwrap();
    assert_eq!(content_hits(&reader), 1);
    store
        .change_content_index(ContentIndexAction::Disable)
        .unwrap();
    assert_empty(&store);
    assert_eq!(content_hits(&reader), 0);
    assert_eq!(
        reader
            .list_projects(Some("fixture-project"), None, 10)
            .unwrap()
            .len(),
        1
    );
    drop(store);
    let mut store = SqliteStore::new(&db).unwrap();
    assert!(!store.content_index_status().unwrap().enabled);
    assert_eq!(
        store.refresh_project_content_index().unwrap(),
        ContentIndexStats::default()
    );
    assert_empty(&store);
    assert_eq!(reader.get_recent_sessions(10).unwrap(), sessions);
    assert_eq!(reader.scan_generation().unwrap(), generation);
    assert_eq!(
        std::fs::read_to_string(root.path().join("fixture-project/README.md")).unwrap(),
        BODY
    );
    store
        .change_content_index(ContentIndexAction::Enable)
        .unwrap();
    assert_empty(&store); // enabling does not implicitly read sources
    store
        .change_content_index(ContentIndexAction::Rebuild)
        .unwrap();
    assert_eq!(content_hits(&reader), 1);
}

#[test]
fn clear_keeps_enabled_and_next_scan_reopens_unchanged_files() {
    let (_root, _db, mut store) = fixture();
    store
        .change_content_index(ContentIndexAction::Clear)
        .unwrap();
    assert!(store.content_index_status().unwrap().enabled);
    assert_empty(&store);
    let stats = store.refresh_project_content_index().unwrap();
    assert_eq!(stats.files_indexed, 1);
    assert_eq!(stats.files_reused, 0);
}

#[test]
fn disabled_rebuild_is_rejected_without_reenabling_or_changing_data() {
    let (_root, db, mut store) = fixture();
    store
        .change_content_index(ContentIndexAction::Disable)
        .unwrap();
    let before = std::fs::read(&db).unwrap();
    assert!(store
        .change_content_index(ContentIndexAction::Rebuild)
        .is_err());
    assert_eq!(std::fs::read(&db).unwrap(), before);
    assert!(!store.content_index_status().unwrap().enabled);
}

#[test]
fn failed_deletion_commits_closed_gate_and_retries_after_reopen_without_source_reads() {
    let (_root, db, mut store) = fixture();
    store.connection.execute_batch("CREATE TRIGGER reject_clear BEFORE DELETE ON project_content_files BEGIN SELECT RAISE(ABORT,'injected delete failure'); END;").unwrap();
    assert!(store
        .change_content_index(ContentIndexAction::Disable)
        .is_err());
    let status = store.content_index_status().unwrap();
    assert_eq!(status.phase, ContentIndexPhase::CleanupPending);
    assert!(!status.enabled && !status.searchable);
    assert_eq!(status.indexed_files, 1); // deletion transaction rolled back
    let reader = IndexReader::open_existing(&db).unwrap();
    assert_eq!(content_hits(&reader), 0);
    assert_eq!(
        reader
            .list_projects(Some("fixture-project"), None, 10)
            .unwrap()
            .len(),
        1
    );
    store
        .connection
        .execute_batch("DROP TRIGGER reject_clear")
        .unwrap();
    drop(store);
    let mut store = SqliteStore::new(&db).unwrap();
    assert_eq!(
        store
            .refresh_project_content_index()
            .unwrap()
            .projects_scanned,
        0
    );
    assert_empty(&store);
    assert_eq!(
        store.content_index_status().unwrap().phase,
        ContentIndexPhase::Ready
    );
}

#[test]
fn gate_transaction_failure_does_not_claim_disable_or_delete_any_data() {
    let (_root, db, mut store) = fixture();
    store.connection.execute_batch("CREATE TRIGGER reject_gate BEFORE INSERT ON sessionatlas_meta WHEN NEW.key='content_index_phase' BEGIN SELECT RAISE(ABORT,'injected gate failure'); END;").unwrap();
    let before = std::fs::read(&db).unwrap();
    assert!(store
        .change_content_index(ContentIndexAction::Disable)
        .is_err());
    assert_eq!(std::fs::read(&db).unwrap(), before);
    let status = store.content_index_status().unwrap();
    assert!(status.enabled && status.searchable);
    assert_eq!(status.phase, ContentIndexPhase::Ready);
}

#[test]
fn pinned_wal_reader_keeps_cleanup_pending_until_retry_and_terms_are_physically_removed() {
    let (_root, db, mut store) = fixture();
    store
        .connection
        .execute_batch("PRAGMA journal_mode=WAL")
        .unwrap();
    store
        .connection
        .busy_timeout(Duration::from_millis(20))
        .unwrap();
    let pinned = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    pinned
        .execute_batch("BEGIN; SELECT COUNT(*) FROM project_content_files;")
        .unwrap();
    assert!(store
        .change_content_index(ContentIndexAction::Disable)
        .is_err());
    let status = content_index_status_at(&db).unwrap();
    assert!(!status.searchable && !status.enabled);
    assert_eq!(status.phase, ContentIndexPhase::CleanupPending);
    assert_empty(&store);
    pinned.execute_batch("ROLLBACK").unwrap();
    drop(pinned);
    store
        .change_content_index(ContentIndexAction::Clear)
        .unwrap();
    assert_eq!(
        store.content_index_status().unwrap().phase,
        ContentIndexPhase::Ready
    );
    drop(store);
    for candidate in [
        db.clone(),
        db.with_extension("db-wal"),
        db.with_extension("db-journal"),
    ] {
        match std::fs::read(candidate) {
            Ok(bytes) => assert!(!bytes
                .windows(b"alphaquartznebula".len())
                .any(|chunk| chunk == b"alphaquartznebula")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("{error}"),
        }
    }
}

#[test]
fn failed_rebuild_hides_partial_content_and_next_scan_recovers() {
    let (root, db, mut store) = fixture();
    let second_dir = root.path().join("second-project");
    std::fs::create_dir(&second_dir).unwrap();
    std::fs::write(second_dir.join("README.md"), BODY).unwrap();
    store
        .upsert_project(&Project {
            id: "second".into(),
            path: second_dir.to_string_lossy().into_owned(),
            last_accessed_at: Utc::now(),
            tool_usages: vec![ToolUsage {
                tool_key: "codex".into(),
                tool_name: "Codex".into(),
                last_used_at: Utc::now(),
                ..ToolUsage::default()
            }],
            ..Project::default()
        })
        .unwrap();
    store.connection.execute_batch("CREATE TRIGGER reject_rebuild BEFORE INSERT ON project_content_status WHEN NEW.project_id='second' BEGIN SELECT RAISE(ABORT,'injected rebuild failure'); END;").unwrap();
    assert!(store
        .change_content_index(ContentIndexAction::Rebuild)
        .is_err());
    let status = store.content_index_status().unwrap();
    assert_eq!(status.phase, ContentIndexPhase::RebuildPending);
    assert!(status.enabled && !status.searchable);
    assert_eq!(status.indexed_files, 1); // first project's transaction committed
    assert_eq!(content_hits(&IndexReader::open_existing(&db).unwrap()), 0);
    store
        .connection
        .execute_batch("DROP TRIGGER reject_rebuild")
        .unwrap();
    drop(store);
    let mut store = SqliteStore::new(&db).unwrap();
    assert_eq!(
        store.refresh_project_content_index().unwrap().files_indexed,
        2
    );
    assert_eq!(
        store.content_index_status().unwrap().phase,
        ContentIndexPhase::Ready
    );
    assert_eq!(content_hits(&IndexReader::open_existing(&db).unwrap()), 2);
}

#[test]
fn explicit_rebuild_ignores_unchanged_mtime_and_size() {
    let (root, db, mut store) = fixture();
    let source = root.path().join("fixture-project/README.md");
    let modified = std::fs::metadata(&source).unwrap().modified().unwrap();
    let changed = BODY.replace("alphaquartznebula", "bravoquartznebula");
    assert_eq!(changed.len(), BODY.len());
    std::fs::write(&source, changed).unwrap();
    std::fs::File::options()
        .write(true)
        .open(source)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert_eq!(
        store.refresh_project_content_index().unwrap().files_reused,
        1
    );
    store
        .change_content_index(ContentIndexAction::Rebuild)
        .unwrap();
    let reader = IndexReader::open_existing(db).unwrap();
    assert_eq!(content_hits(&reader), 0);
    assert_eq!(
        reader
            .list_projects(Some("bravoquartznebula"), None, 10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn unknown_policy_and_phase_hide_content_but_preserve_name_search() {
    let (_root, db, store) = fixture();
    let reader = IndexReader::open_existing(db).unwrap();
    for (key, value) in [(ENABLED_KEY, "unexpected"), (PHASE_KEY, "futurePhase")] {
        set_meta(&store.connection, key, value).unwrap();
        assert_eq!(content_hits(&reader), 0);
        assert_eq!(
            reader
                .list_projects(Some("fixture-project"), None, 10)
                .unwrap()
                .len(),
            1
        );
        assert!(reader.content_index_status().is_err());
        store
            .connection
            .execute("DELETE FROM sessionatlas_meta WHERE key=?1", [key])
            .unwrap();
    }
}

#[test]
fn unsupported_schema_controls_are_non_mutating() {
    let (_root, db, store) = fixture();
    store
        .connection
        .pragma_update(None, "user_version", INDEX_SCHEMA_VERSION + 1)
        .unwrap();
    drop(store);
    let before = std::fs::read(&db).unwrap();
    assert!(content_index_status_at(&db).is_err());
    assert!(change_content_index_at(&db, ContentIndexAction::Disable).is_err());
    assert_eq!(std::fs::read(&db).unwrap(), before);
}

#[test]
fn disable_waits_for_the_scan_writer_then_prevents_later_refresh() {
    let (_root, db, mut store) = fixture();
    let path = db.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        change_content_index_at(path, ContentIndexAction::Disable)
    });
    started_rx.recv().unwrap();
    store.refresh_project_content_index().unwrap();
    drop(store);
    let status = handle.join().unwrap().unwrap();
    assert!(!status.enabled && !status.searchable);
    let mut store = SqliteStore::new(db).unwrap();
    assert_eq!(
        store
            .refresh_project_content_index()
            .unwrap()
            .projects_scanned,
        0
    );
}
