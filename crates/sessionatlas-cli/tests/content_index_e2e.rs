//! Real child-process CLI round trips, confined to synthetic temporary homes.
use sessionatlas_core::model::{Project, ToolUsage};
use sessionatlas_core::store::{IndexReader, SqliteStore};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::SystemTime;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "sessionatlas-content-cli-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn db(&self) -> PathBuf {
        self.0.join(".sessionatlas/index.db")
    }
    fn command(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_sessionatlas"))
            .env("SESSIONATLAS_HOME", &self.0)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap()
    }
    fn content(&self, action: &str) -> serde_json::Value {
        let output = self.command(&["content-index", action, "--json"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn seed(&self) {
        let project_dir = self.0.join("fixture-project");
        std::fs::create_dir(&project_dir).unwrap();
        std::fs::write(
            project_dir.join("README.md"),
            "galacticquartz synthetic documentation",
        )
        .unwrap();
        let mut store = SqliteStore::new(self.db()).unwrap();
        store
            .replace_tool_snapshots(
                &[Project {
                    path: project_dir.to_string_lossy().into_owned(),
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
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn hits(db: &Path) -> usize {
    IndexReader::open_existing(db)
        .unwrap()
        .list_projects(Some("galacticquartz"), None, 10)
        .unwrap()
        .len()
}

#[test]
fn cli_content_controls_round_trip_across_real_processes() {
    let fixture = Fixture::new();
    fixture.seed();
    let db = fixture.db();
    let bytes = std::fs::read(&db).unwrap();
    let modified = std::fs::metadata(&db).unwrap().modified().unwrap();
    assert_eq!(fixture.content("status")["indexedFiles"], 1);
    assert_eq!(std::fs::read(&db).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(&db).unwrap().modified().unwrap(),
        modified
    );
    assert_eq!(hits(&db), 1);
    let disabled = fixture.content("disable");
    assert_eq!(disabled["enabled"], false);
    assert_eq!(disabled["indexedFiles"], 0);
    assert_eq!(hits(&db), 0);
    assert!(!fixture
        .command(&["content-index", "rebuild", "--json"])
        .status
        .success());
    assert_eq!(fixture.content("status")["enabled"], false);
    assert_eq!(fixture.content("enable")["indexedFiles"], 0);
    assert_eq!(fixture.content("rebuild")["indexedFiles"], 1);
    assert_eq!(hits(&db), 1);
    assert_eq!(fixture.content("clear")["indexedFiles"], 0);
    assert_eq!(fixture.content("status")["enabled"], true);
    assert_eq!(hits(&db), 0);
    assert!(fixture.command(&["list"]).status.success());
    assert_eq!(
        std::fs::read_to_string(fixture.0.join("fixture-project/README.md")).unwrap(),
        "galacticquartz synthetic documentation"
    );
}

#[test]
fn cli_missing_index_status_and_invalid_actions_do_not_create_artifacts() {
    let fixture = Fixture::new();
    assert_eq!(fixture.content("status")["indexExists"], false);
    assert!(fixture
        .command(&["content-index", "--json"])
        .status
        .success());
    for action in ["disable", "enable", "clear", "rebuild", "invalid-action"] {
        let output = fixture.command(&["content-index", action, "--json"]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    assert!(!fixture.0.join(".sessionatlas").exists());
}

#[test]
fn cli_locked_writer_reports_failure_not_success_or_empty_index() {
    let fixture = Fixture::new();
    fixture.seed();
    let writer = SqliteStore::new(fixture.db()).unwrap();
    let output = fixture.command(&["content-index", "disable", "--json"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("status"));
    assert!(writer.content_index_status().unwrap().enabled);
}
