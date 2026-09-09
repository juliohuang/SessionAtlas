use super::*;
use sessionatlas_core::model::{Project, ToolUsage};
use sessionatlas_core::scanner::{ScanOutcome, ScannedProject, Scanner};
use sessionatlas_core::store::SqliteStore;
use std::time::SystemTime;

struct FixtureScanner(String);

#[test]
fn desktop_search_keeps_policy_and_content_in_one_snapshot() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    let root = tempfile::Builder::new()
        .prefix("sessionatlas-content-race-")
        .tempdir()
        .unwrap();
    let project = root.path().join("desktop-fixture");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("README.md"),
        "quartznebula synthetic documentation",
    )
    .unwrap();
    let db = root.path().join("index.db");
    let mut store = SqliteStore::new(&db).unwrap();
    store
        .upsert_project(&Project {
            path: project.to_string_lossy().into_owned(),
            first_seen_at: SystemTime::now().into(),
            last_accessed_at: SystemTime::now().into(),
            ..Project::default()
        })
        .unwrap();
    store.refresh_project_content_index().unwrap();
    drop(store);
    let _guard = sessionatlas_core::scan_coordinator::IndexWriteGuard::acquire(&db).unwrap();
    let mut writer = rusqlite::Connection::open(&db).unwrap();
    writer
        .execute_batch("PRAGMA journal_mode=WAL; DELETE FROM project_content_fts;")
        .unwrap();
    let reader =
        rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let fired = Arc::new(AtomicBool::new(false));
    let triggered = fired.clone();
    reader.authorizer(Some(move |context: AuthContext<'_>| {
        if matches!(context.action, AuthAction::Read { table_name: "project_content_files", column_name: "project_id" })
            && !triggered.swap(true, Ordering::SeqCst)
        {
            let tx = writer.transaction().unwrap();
            tx.execute("INSERT INTO sessionatlas_meta(key,value) VALUES('content_index_phase','rebuildPending') ON CONFLICT(key) DO UPDATE SET value=excluded.value", []).unwrap();
            tx.execute("INSERT INTO project_content_fts(rowid,relative_path,body) SELECT id,relative_path,'quartznebula synthetic documentation' FROM project_content_files", []).unwrap();
            tx.commit().unwrap();
        }
        Authorization::Allow
    })).unwrap();
    assert!(
        crate::search_projects_in_connection(&reader, "quartznebula", &[])
            .unwrap()
            .is_empty(),
        "must not expose the partial rebuild"
    );
    assert!(fired.load(Ordering::SeqCst), "writer interleaving must run");
    assert!(!content_index_status_at(&db).unwrap().searchable);
    assert!(
        crate::search_projects_in_connection(&reader, "quartznebula", &[])
            .unwrap()
            .is_empty()
    );
}
impl Scanner for FixtureScanner {
    fn tool_key(&self) -> &str {
        "codex"
    }
    fn tool_name(&self) -> &str {
        "Codex"
    }
    fn is_available(&self) -> bool {
        true
    }
    fn scan(&self) -> ScanOutcome {
        ScanOutcome::succeeded(
            vec![ScannedProject {
                path: self.0.clone(),
                last_accessed_at: SystemTime::now().into(),
                session_id: None,
                git_branch: None,
            }],
            [],
        )
    }
}

#[test]
fn desktop_search_and_scan_observe_shared_content_controls() {
    // Desktop intentionally hides paths containing dot-prefixed components;
    // tempfile's default .tmp prefix is not a visible project fixture.
    let root = tempfile::Builder::new()
        .prefix("sessionatlas-content-controls-")
        .tempdir()
        .unwrap();
    let project = root.path().join("desktop-fixture");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("README.md"),
        "quartznebula synthetic documentation",
    )
    .unwrap();
    let db = root.path().join("index.db");
    let path = project.to_string_lossy().into_owned();
    let mut store = SqliteStore::new(&db).unwrap();
    store
        .replace_tool_snapshots(
            &[Project {
                path: path.clone(),
                last_accessed_at: SystemTime::now().into(),
                tool_usages: vec![ToolUsage {
                    tool_key: "codex".into(),
                    tool_name: "Codex".into(),
                    last_used_at: SystemTime::now().into(),
                    ..ToolUsage::default()
                }],
                ..Project::default()
            }],
            &["codex"],
        )
        .unwrap();
    store.refresh_project_content_index().unwrap();
    drop(store);
    let reader =
        rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    assert_eq!(
        crate::search_projects_in_connection(&reader, "quartznebula", &[])
            .unwrap()
            .len(),
        1
    );
    change_content_index_at(&db, ContentIndexAction::Disable).unwrap();
    let scanners: Vec<Box<dyn Scanner>> = vec![Box::new(FixtureScanner(path))];
    assert_eq!(
        crate::run_scan_with_scanners(&db, &scanners, &[]).unwrap(),
        1
    );
    assert_eq!(content_index_status_at(&db).unwrap().indexed_files, 0);
    assert!(
        crate::search_projects_in_connection(&reader, "quartznebula", &[])
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        crate::search_projects_in_connection(&reader, "desktop-fixture", &[])
            .unwrap()
            .len(),
        1
    );
    change_content_index_at(&db, ContentIndexAction::Enable).unwrap();
    change_content_index_at(&db, ContentIndexAction::Rebuild).unwrap();
    let hits = crate::search_projects_in_connection(&reader, "quartznebula", &[]).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0]
        .content_match
        .as_ref()
        .unwrap()
        .snippet
        .as_ref()
        .unwrap()
        .contains("quartznebula"));
}

#[test]
fn ipc_action_is_strict_and_status_fields_are_camel_case() {
    for action in ["enable", "disable", "clear", "rebuild"] {
        assert!(serde_json::from_value::<ContentIndexAction>(serde_json::json!(action)).is_ok());
    }
    for action in ["delete", "../../index.db", "DISABLE", ""] {
        assert!(serde_json::from_value::<ContentIndexAction>(serde_json::json!(action)).is_err());
    }
    let root = tempfile::tempdir().unwrap();
    let value =
        serde_json::to_value(content_index_status_at(root.path().join("missing.db")).unwrap())
            .unwrap();
    assert_eq!(value["indexExists"], false);
    assert_eq!(value["indexedFiles"], 0);
    assert!(value.get("index_exists").is_none());
}
