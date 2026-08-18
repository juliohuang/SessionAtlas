//! Contract tests for `sessionatlas_core::config`.
//!
//! Covers the R03 pass conditions:
//! empty config, invalid JSON, concurrent updates, busy-lock timeout, stale
//! object conflict, replace failure keeping the old file, and strict cleanup
//! of stale generated temps. Every test uses `tempfile` — never the real user
//! home. The only tests touching the environment hold a global mutex and
//! restore `SESSIONATLAS_HOME` afterwards.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fs2::FileExt;
use sessionatlas_core::config::{
    self, config_path_for_home, default_config_path, home_directory, update, AppConfig, ConfigError,
};

/// Serializes the environment-dependent tests so the process-global
/// `SESSIONATLAS_HOME` cannot leak across them.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_path_for(config_path: &Path) -> PathBuf {
    let mut lock = config_path.as_os_str().to_os_string();
    lock.push(".lock");
    PathBuf::from(lock)
}

fn set_mtime_hours_ago(path: &Path, hours: i64) {
    let modified =
        std::time::SystemTime::now() - std::time::Duration::from_secs(hours.unsigned_abs() * 3600);
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
}

fn guid_hex(seed: u32) -> String {
    format!("{seed:032x}")
}

fn restore_sessionatlas_home(previous: Option<std::ffi::OsString>) {
    match previous {
        Some(value) => std::env::set_var("SESSIONATLAS_HOME", value),
        None => std::env::remove_var("SESSIONATLAS_HOME"),
    }
}

#[test]
fn config_contract_config_path_for_home_joins_dot_sessionatlas() {
    let temp = tempfile::tempdir().unwrap();
    assert_eq!(
        config_path_for_home(temp.path()),
        temp.path().join(".sessionatlas").join("config.json")
    );
}

#[test]
fn config_contract_default_config_path_follows_sessionatlas_home() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let previous = std::env::var_os("SESSIONATLAS_HOME");
    std::env::set_var("SESSIONATLAS_HOME", temp.path());

    let result = default_config_path();
    let resolved_home = home_directory();

    restore_sessionatlas_home(previous);

    assert_eq!(result, config_path_for_home(temp.path()));
    assert_eq!(resolved_home, temp.path());
}

#[test]
fn config_contract_home_falls_back_to_user_home() {
    let _guard = ENV_LOCK.lock().unwrap();
    let previous = std::env::var_os("SESSIONATLAS_HOME");
    std::env::remove_var("SESSIONATLAS_HOME");

    let home = home_directory();

    restore_sessionatlas_home(previous);

    let expected = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    assert_eq!(home, expected);
    assert!(!home.as_os_str().is_empty());
}

#[test]
fn config_contract_whitespace_only_sessionatlas_home_is_ignored() {
    let _guard = ENV_LOCK.lock().unwrap();
    let previous = std::env::var_os("SESSIONATLAS_HOME");
    std::env::set_var("SESSIONATLAS_HOME", " \t\n");

    let home = home_directory();

    restore_sessionatlas_home(previous);

    let expected = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    assert_eq!(home, expected);
    assert!(!home.as_os_str().is_empty());
}

#[test]
fn config_contract_missing_file_loads_default() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("absent").join("config.json");
    let config = config::load(&missing).unwrap();
    assert_eq!(config.default_terminal, "auto");
    assert!(config.custom_tools.is_empty());
    assert!(config.preferred_tools_by_path.is_empty());
    let via_try = config::try_load(&missing).unwrap();
    assert_eq!(via_try.default_terminal, "auto");
}

#[test]
fn config_contract_empty_and_blank_files_are_invalid_json() {
    let temp = tempfile::tempdir().unwrap();
    for (name, content) in [("empty.json", ""), ("blank.json", "   \r\n\t ")] {
        let path = temp.path().join(name);
        fs::write(&path, content).unwrap();
        assert!(matches!(config::load(&path), Err(ConfigError::Json(_))));
        assert!(config::try_load(&path).is_none());
    }
}

#[test]
fn config_contract_empty_object_loads_default() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    fs::write(&path, "{}").unwrap();
    let config = config::load(&path).unwrap();
    assert_eq!(config.default_terminal, "auto");
    assert!(config.custom_tools.is_empty());
}

#[test]
fn config_contract_utf8_bom_is_accepted() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    fs::write(&path, b"\xef\xbb\xbf{\"DefaultTerminal\":\"kitty\"}").unwrap();

    let config = config::load(&path).unwrap();

    assert_eq!(config.default_terminal, "kitty");
}

#[test]
fn config_contract_invalid_json_is_a_typed_error() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    fs::write(&path, "{not-valid-json").unwrap();

    let error = config::load(&path).unwrap_err();
    assert!(matches!(error, ConfigError::Json(_)), "got {error:?}");
    assert!(config::try_load(&path).is_none());
}

#[test]
fn config_contract_case_insensitive_property_names_at_appconfig_level() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    fs::write(
        &path,
        r#"{
            "dEfAuLtTeRmInAl": "kitty",
            "CUSTOMTOOLS": [],
            "enabledadapters": ["codex", "myagent"],
            "ACTIVEADAPTERVERSIONS": { "myagent": "1.2.0" },
            "preferredtoolsbypath": { "C:\\repo": "codex" }
        }"#,
    )
    .unwrap();

    let config = config::load(&path).unwrap();
    assert_eq!(config.default_terminal, "kitty");
    assert_eq!(
        config.enabled_adapters.as_deref(),
        Some(["codex".to_string(), "myagent".to_string()].as_slice())
    );
    assert_eq!(
        config
            .active_adapter_versions
            .get("myagent")
            .map(String::as_str),
        Some("1.2.0")
    );
    assert_eq!(
        config
            .preferred_tools_by_path
            .get(r"C:\repo")
            .map(String::as_str),
        Some("codex")
    );
}

#[test]
fn config_contract_case_insensitive_property_names_at_tool_source_level() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    fs::write(
        &path,
        r#"{
            "customTools": [{
                "key": "mytool",
                "NAME": "My Tool",
                "cliCommand": "mytool",
                "DataDirectory": "/data/mytool",
                "scannerType": "custom",
                "ISINSTALLED": true,
                "isenabled": true,
                "OpenCommandTemplate": "cd \"{projectPath}\" && mytool {sessionId}"
            }]
        }"#,
    )
    .unwrap();

    let config = config::load(&path).unwrap();
    assert_eq!(config.custom_tools.len(), 1);
    let tool = &config.custom_tools[0];
    assert_eq!(tool.key, "mytool");
    assert_eq!(tool.name, "My Tool");
    assert_eq!(tool.cli_command, "mytool");
    assert_eq!(tool.data_directory, "/data/mytool");
    assert_eq!(tool.scanner_type, "custom");
    assert!(tool.is_installed);
    assert!(tool.is_enabled);
    assert_eq!(
        tool.open_command_template,
        "cd \"{projectPath}\" && mytool {sessionId}"
    );
}

#[test]
fn config_contract_tool_source_missing_fields_use_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    fs::write(&path, r#"{ "customtools": [ { "key": "kimi" } ] }"#).unwrap();

    let config = config::load(&path).unwrap();
    let tool = &config.custom_tools[0];
    assert_eq!(tool.key, "kimi");
    assert!(tool.is_enabled);
    assert!(!tool.is_installed);
    assert_eq!(
        tool.open_command_template,
        "cd \"{projectPath}\" && {cliCommand}"
    );
}

#[test]
fn config_contract_wrong_field_types_are_errors() {
    let temp = tempfile::tempdir().unwrap();
    let cases = [
        r#"{ "DefaultTerminal": 42 }"#,
        r#"{ "CustomTools": { "not": "an array" } }"#,
        r#"{ "CustomTools": [ 42 ] }"#,
        r#"{ "customtools": [ { "IsEnabled": "yes" } ] }"#,
        r#"{ "EnabledAdapters": "codex" }"#,
        r#"{ "ActiveAdapterVersions": ["codex"] }"#,
    ];
    for (index, json) in cases.iter().enumerate() {
        let path = temp.path().join(format!("config-{index}.json"));
        fs::write(&path, json).unwrap();
        let error = config::load(&path).unwrap_err();
        assert!(matches!(error, ConfigError::Json(_)), "got {error:?}");
    }
}

#[test]
fn config_contract_round_trip_preserves_config_and_pascal_case_keys() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");

    let mut config = AppConfig::default();
    config.default_terminal = "windows-terminal".to_string();
    config.enabled_adapters = Some(vec!["codex".to_string(), "mytool".to_string()]);
    config
        .active_adapter_versions
        .insert("mytool".to_string(), "2.1.0".to_string());
    config
        .preferred_tools_by_path
        .insert(r"C:\repo".to_string(), "codex".to_string());
    config
        .custom_tools
        .push(sessionatlas_core::model::ToolSource {
            key: "mytool".to_string(),
            name: "My Tool".to_string(),
            cli_command: "mytool".to_string(),
            data_directory: "/data/mytool".to_string(),
            scanner_type: "custom".to_string(),
            is_installed: true,
            is_enabled: true,
            open_command_template: "cd \"{projectPath}\" && mytool {sessionId}".to_string(),
        });

    config.save(&path, None).unwrap();

    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"CustomTools\""));
    assert!(text.contains("\"EnabledAdapters\""));
    assert!(text.contains("\"ActiveAdapterVersions\""));
    assert!(text.contains("\"PreferredToolsByPath\""));
    assert!(text.contains("\"DefaultTerminal\""));
    assert!(text.contains("\"Key\""));
    assert!(text.contains("\"CliCommand\""));
    assert!(text.contains("\"OpenCommandTemplate\""));
    assert!(!text.contains("source_path"));
    assert!(!text.contains("source_fingerprint"));

    assert_eq!(config::load(&path).unwrap(), config);
}

#[test]
fn config_contract_update_creates_missing_directories() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("nested").join("dir").join("config.json");
    let config = update(&path, None, |config| {
        config.default_terminal = "kitty".to_string();
    })
    .unwrap();
    assert_eq!(config.default_terminal, "kitty");
    assert!(path.exists());
    assert_eq!(config::load(&path).unwrap().default_terminal, "kitty");
}

#[test]
fn config_contract_stale_instance_cannot_overwrite_a_newer_save() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");

    let mut first = config::load(&path).unwrap();
    first.default_terminal = "initial".to_string();
    first.save(&path, None).unwrap();

    let mut first = config::load(&path).unwrap();
    let mut stale = config::load(&path).unwrap();
    first.default_terminal = "first".to_string();
    first.save(&path, None).unwrap();
    stale.default_terminal = "stale".to_string();

    let error = stale.save(&path, None).unwrap_err();
    assert!(matches!(error, ConfigError::Conflict), "got {error:?}");
    assert_eq!(config::load(&path).unwrap().default_terminal, "first");
}

#[test]
fn config_contract_concurrent_updates_do_not_lose_mutations_or_expose_partial_json() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    update(&path, None, |_| {}).unwrap();

    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let read_errors = std::sync::Arc::new(std::sync::Mutex::new(0usize));

    let reader_stop = std::sync::Arc::clone(&stop);
    let reader_errors = std::sync::Arc::clone(&read_errors);
    let reader_path = path.clone();
    let reader = std::thread::spawn(move || {
        while !reader_stop.load(std::sync::atomic::Ordering::Relaxed) {
            if config::try_load(&reader_path).is_none() {
                *reader_errors.lock().unwrap() += 1;
            }
        }
    });

    let mut writers = Vec::new();
    for writer in 0..2u32 {
        let writer_path = path.clone();
        writers.push(std::thread::spawn(move || {
            for iteration in 0..50u32 {
                let key = format!("writer-{writer}-{iteration}");
                let writer_path = writer_path.clone();
                update(&writer_path, None, move |config| {
                    config
                        .preferred_tools_by_path
                        .insert(key, "codex".to_string());
                })
                .unwrap();
            }
        }));
    }
    for writer in writers {
        writer.join().unwrap();
    }

    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    reader.join().unwrap();

    assert_eq!(*read_errors.lock().unwrap(), 0);
    let final_config = config::load(&path).unwrap();
    assert_eq!(final_config.preferred_tools_by_path.len(), 100);
}

#[test]
fn config_contract_busy_lock_is_bounded_and_does_not_modify_config() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    update(&path, None, |config| {
        config.default_terminal = "old".to_string();
    })
    .unwrap();

    let held = fs::File::options()
        .read(true)
        .write(true)
        .open(lock_path_for(&path))
        .unwrap();
    held.try_lock_exclusive().unwrap();

    let error = update(&path, Some(Duration::from_millis(75)), |config| {
        config.default_terminal = "new".to_string()
    })
    .unwrap_err();
    assert!(matches!(error, ConfigError::Busy(_)), "got {error:?}");

    assert_eq!(config::load(&path).unwrap().default_terminal, "old");
}

#[test]
fn config_contract_replace_failure_keeps_old_json_and_cleans_only_the_current_temp() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");

    let mut config = config::load(&path).unwrap();
    config.default_terminal = "old".to_string();
    config.save(&path, None).unwrap();

    let mut config = config::load(&path).unwrap();
    config.default_terminal = "sensitive-placeholder".to_string();
    config::force_next_atomic_replace_failure();

    let error = config.save(&path, None).unwrap_err();
    assert!(matches!(error, ConfigError::Io(_)), "got {error:?}");

    assert_eq!(config::load(&path).unwrap().default_terminal, "old");
    let temps: Vec<PathBuf> = fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|candidate| {
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("config.json.tmp."))
        })
        .collect();
    assert!(temps.is_empty(), "leftover temp files: {temps:?}");
    assert!(!fs::read_to_string(&path)
        .unwrap()
        .contains("sensitive-placeholder"));
}

#[test]
fn config_contract_cleanup_deletes_only_strict_old_generated_temps() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    update(&path, None, |_| {}).unwrap();

    let old_generated = temp
        .path()
        .join(format!("config.json.tmp.123.{}", guid_hex(1)));
    let new_generated = temp
        .path()
        .join(format!("config.json.tmp.124.{}", guid_hex(2)));
    let similar = temp.path().join("config.json.tmp.not-a-generated-name");
    fs::write(&old_generated, "old").unwrap();
    fs::write(&new_generated, "new").unwrap();
    fs::write(&similar, "similar").unwrap();
    set_mtime_hours_ago(&old_generated, 25);
    set_mtime_hours_ago(&similar, 25);

    update(&path, None, |config| {
        config.default_terminal = "updated".to_string();
    })
    .unwrap();

    assert!(
        !old_generated.exists(),
        "stale generated temp must be deleted"
    );
    assert!(new_generated.exists(), "recent temp must be kept");
    assert!(similar.exists(), "non-generated name must be kept");
}

#[test]
fn config_contract_cleanup_skips_symlinked_generated_names() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    update(&path, None, |_| {}).unwrap();

    let victim = temp.path().join("victim.txt");
    fs::write(&victim, "keep me").unwrap();
    let link = temp
        .path()
        .join(format!("config.json.tmp.999.{}", guid_hex(9)));

    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_file;
        if symlink_file(&victim, &link).is_err() {
            eprintln!("skipping: creating symlinks requires developer mode/admin");
            return;
        }
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&victim, &link).unwrap();

    set_mtime_hours_ago(&victim, 25);

    update(&path, None, |config| {
        config.default_terminal = "updated".to_string();
    })
    .unwrap();

    assert!(
        link.symlink_metadata().is_ok(),
        "symlinked temp name must not be deleted"
    );
    assert_eq!(fs::read_to_string(&victim).unwrap(), "keep me");
}

#[test]
fn config_contract_save_default_writes_under_isolated_sessionatlas_home() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let previous = std::env::var_os("SESSIONATLAS_HOME");
    std::env::set_var("SESSIONATLAS_HOME", temp.path());

    let mut config = AppConfig::default();
    config.default_terminal = "kitty".to_string();
    let result = config.save_default(None);

    restore_sessionatlas_home(previous);
    result.unwrap();

    let stored = config::load(config_path_for_home(temp.path())).unwrap();
    assert_eq!(stored.default_terminal, "kitty");
}
