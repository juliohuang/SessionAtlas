use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

const RECENT_PROTECTION_DAYS: i64 = 14;
const TRIVIAL_AGE_DAYS: i64 = 30;
const LIKELY_TRIVIAL_AGE_DAYS: i64 = 90;
const OLD_CODEX_MINOR: u32 = 140;
const SESSION_CLEANUP_CACHE_VERSION: u32 = 1;
const SESSION_CLEANUP_PARSER_VERSION: u32 = 2;
const SESSION_CLEANUP_CACHE_FILE: &str = "session-cleanup-cache-v1.json";

static SESSION_CLEANUP_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionCleanupCandidate {
    pub(crate) key: String,
    pub(crate) tool_key: String,
    pub(crate) session_id: String,
    pub(crate) parent_session_id: Option<String>,
    pub(crate) project_path: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) cli_version: Option<String>,
    pub(crate) agent_kind: String,
    pub(crate) storage_state: String,
    pub(crate) classification: String,
    pub(crate) reasons: Vec<String>,
    pub(crate) protections: Vec<String>,
    pub(crate) age_days: i64,
    pub(crate) size_bytes: u64,
    pub(crate) user_turns: usize,
    pub(crate) tool_calls: usize,
    pub(crate) can_clean: bool,
    #[serde(skip)]
    completed: bool,
    #[serde(skip)]
    source_modified_ns: String,
    #[serde(skip)]
    source_path: PathBuf,
    #[serde(skip)]
    observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionCleanupAnalysis {
    pub(crate) candidates: Vec<SessionCleanupCandidate>,
    pub(crate) supported_tools: Vec<String>,
    pub(crate) scanned_at: String,
    pub(crate) snapshot_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedParsedSession {
    id: String,
    parent_id: Option<String>,
    project_path: Option<String>,
    title: Option<String>,
    cli_version: Option<String>,
    agent_kind: String,
    latest_at: Option<String>,
    user_turns: usize,
    tool_calls: usize,
    completed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SessionCacheEntry {
    parser_version: u32,
    source_path: String,
    tool_key: String,
    storage_state: String,
    size_bytes: u64,
    modified_ns: String,
    parsed: CachedParsedSession,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedCandidate {
    key: String,
    tool_key: String,
    session_id: String,
    parent_session_id: Option<String>,
    project_path: Option<String>,
    title: Option<String>,
    cli_version: Option<String>,
    agent_kind: String,
    storage_state: String,
    classification: String,
    reasons: Vec<String>,
    protections: Vec<String>,
    age_days: i64,
    size_bytes: u64,
    user_turns: usize,
    tool_calls: usize,
    can_clean: bool,
    completed: bool,
    source_path: String,
    observed_at: String,
    modified_ns: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct SessionCleanupCache {
    version: u32,
    parser_version: u32,
    inventory_fingerprint: String,
    snapshot_id: String,
    entries: BTreeMap<String, SessionCacheEntry>,
    candidates: Vec<CachedCandidate>,
}

#[derive(Clone, Debug)]
struct FileFingerprint {
    normalized_path: String,
    size_bytes: u64,
    modified_ns: String,
}

impl CachedParsedSession {
    fn from_parsed(parsed: &ParsedSession) -> Self {
        Self {
            id: parsed.id.clone(),
            parent_id: parsed.parent_id.clone(),
            project_path: parsed.project_path.clone(),
            title: parsed.title.clone(),
            cli_version: parsed.cli_version.clone(),
            agent_kind: parsed.agent_kind.clone(),
            latest_at: parsed.latest_at.map(|value| value.to_rfc3339()),
            user_turns: parsed.user_turns,
            tool_calls: parsed.tool_calls,
            completed: parsed.completed,
        }
    }
}

impl CachedCandidate {
    fn from_candidate(candidate: &SessionCleanupCandidate) -> Self {
        Self {
            key: candidate.key.clone(),
            tool_key: candidate.tool_key.clone(),
            session_id: candidate.session_id.clone(),
            parent_session_id: candidate.parent_session_id.clone(),
            project_path: candidate.project_path.clone(),
            title: candidate.title.clone(),
            cli_version: candidate.cli_version.clone(),
            agent_kind: candidate.agent_kind.clone(),
            storage_state: candidate.storage_state.clone(),
            classification: candidate.classification.clone(),
            reasons: candidate.reasons.clone(),
            protections: candidate.protections.clone(),
            age_days: candidate.age_days,
            size_bytes: candidate.size_bytes,
            user_turns: candidate.user_turns,
            tool_calls: candidate.tool_calls,
            can_clean: candidate.can_clean,
            completed: candidate.completed,
            source_path: candidate.source_path.to_string_lossy().into_owned(),
            observed_at: candidate.observed_at.to_rfc3339(),
            modified_ns: candidate.source_modified_ns.clone(),
        }
    }

    fn into_candidate(self) -> Option<SessionCleanupCandidate> {
        Some(SessionCleanupCandidate {
            key: self.key,
            tool_key: self.tool_key,
            session_id: self.session_id,
            parent_session_id: self.parent_session_id,
            project_path: self.project_path,
            title: self.title,
            cli_version: self.cli_version,
            agent_kind: self.agent_kind,
            storage_state: self.storage_state,
            classification: self.classification,
            reasons: self.reasons,
            protections: self.protections,
            age_days: self.age_days,
            size_bytes: self.size_bytes,
            user_turns: self.user_turns,
            tool_calls: self.tool_calls,
            can_clean: self.can_clean,
            completed: self.completed,
            source_modified_ns: self.modified_ns,
            source_path: PathBuf::from(self.source_path),
            observed_at: DateTime::parse_from_rfc3339(&self.observed_at)
                .ok()?
                .with_timezone(&Utc),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionTrashBatch {
    pub(crate) batch_id: String,
    pub(crate) created_at: String,
    pub(crate) session_count: usize,
    pub(crate) size_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrashManifestEntry {
    original_path: String,
    trashed_path: String,
    key: String,
    size_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrashManifest {
    batch_id: String,
    created_at: String,
    entries: Vec<TrashManifestEntry>,
}

#[derive(Default)]
struct ParsedSession {
    id: String,
    parent_id: Option<String>,
    project_path: Option<String>,
    title: Option<String>,
    cli_version: Option<String>,
    agent_kind: String,
    latest_at: Option<DateTime<Utc>>,
    user_turns: usize,
    tool_calls: usize,
    completed: bool,
}

pub(crate) fn analyze_sessions(home: &Path) -> Result<SessionCleanupAnalysis, String> {
    let lock = SESSION_CLEANUP_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock
        .lock()
        .map_err(|_| "session cleanup lock poisoned".to_string())?;
    analyze_sessions_at(home, Utc::now())
}

fn analyze_sessions_at(home: &Path, now: DateTime<Utc>) -> Result<SessionCleanupAnalysis, String> {
    let mut cache = load_cache(home);
    let mut raw = Vec::new();
    let mut inventory = Vec::new();
    collect_codex_sessions_at(home, &mut raw, &mut inventory, &mut cache, now)?;
    collect_claude_sessions_at(home, &mut raw, &mut inventory, &mut cache, now)?;
    let inventory_fingerprint = fingerprint_inventory(&inventory);
    let current_paths: HashSet<String> = inventory
        .iter()
        .map(|fingerprint| fingerprint.normalized_path.clone())
        .collect();
    cache.entries.retain(|path, _| current_paths.contains(path));

    apply_candidate_safety(&mut raw);

    raw.sort_by(|left, right| {
        classification_rank(&left.classification)
            .cmp(&classification_rank(&right.classification))
            .then(right.age_days.cmp(&left.age_days))
            .then(right.size_bytes.cmp(&left.size_bytes))
    });
    let snapshot_id = fingerprint_candidates(&raw, &inventory_fingerprint);
    cache.version = SESSION_CLEANUP_CACHE_VERSION;
    cache.parser_version = SESSION_CLEANUP_PARSER_VERSION;
    cache.inventory_fingerprint = inventory_fingerprint;
    cache.snapshot_id = snapshot_id.clone();
    cache.candidates = raw.iter().map(CachedCandidate::from_candidate).collect();
    save_cache(home, &cache);

    Ok(SessionCleanupAnalysis {
        candidates: raw,
        supported_tools: vec!["codex".to_string(), "claude".to_string()],
        scanned_at: now.to_rfc3339(),
        snapshot_id,
    })
}

fn classification_rank(value: &str) -> u8 {
    match value {
        "likely" => 0,
        "possible" => 1,
        _ => 2,
    }
}

fn apply_candidate_safety(raw: &mut [SessionCleanupCandidate]) {
    let ids: HashMap<String, bool> = raw
        .iter()
        .filter(|candidate| !candidate.session_id.is_empty())
        .map(|candidate| (candidate.session_id.clone(), candidate.completed))
        .collect();
    let parent_ids: HashSet<String> = raw
        .iter()
        .filter_map(|candidate| candidate.parent_session_id.clone())
        .collect();
    let current_codex_thread = std::env::var("CODEX_THREAD_ID").ok();
    let newest_by_project: HashMap<(String, String), DateTime<Utc>> = raw
        .iter()
        .filter(|candidate| candidate.agent_kind == "root")
        .filter_map(|candidate| {
            Some((
                (candidate.tool_key.clone(), candidate.project_path.clone()?),
                candidate.observed_at,
            ))
        })
        .fold(HashMap::new(), |mut newest, (key, value)| {
            newest
                .entry(key)
                .and_modify(|current| *current = (*current).max(value))
                .or_insert(value);
            newest
        });

    for candidate in raw {
        candidate.classification = "keep".to_string();
        candidate.reasons.clear();
        candidate.protections.clear();
        if candidate.completed {
            candidate.reasons.push("completed".to_string());
        }
        let parent_present = candidate
            .parent_session_id
            .as_ref()
            .is_some_and(|parent| ids.contains_key(parent));
        let parent_completed = candidate
            .parent_session_id
            .as_ref()
            .and_then(|parent| ids.get(parent))
            .copied()
            .unwrap_or(false);
        classify_candidate(candidate, parent_present, parent_completed);

        if candidate.age_days < RECENT_PROTECTION_DAYS {
            candidate.protections.push("recent".to_string());
        }
        if candidate.parent_session_id.is_some() && !parent_present {
            candidate.protections.push("parentMissing".to_string());
        }
        if parent_ids.contains(&candidate.session_id) {
            candidate.protections.push("hasChildren".to_string());
        }
        if candidate.tool_key == "codex"
            && current_codex_thread.as_deref() == Some(candidate.session_id.as_str())
        {
            candidate.protections.push("currentSession".to_string());
        }
        if candidate.agent_kind == "root" {
            if let Some(project) = &candidate.project_path {
                let key = (candidate.tool_key.clone(), project.clone());
                if newest_by_project
                    .get(&key)
                    .is_some_and(|latest| *latest == candidate.observed_at)
                {
                    candidate.protections.push("latestForProject".to_string());
                }
            }
        }
        candidate.reasons.retain(|reason| reason != "completed");
        candidate.can_clean =
            candidate.classification != "keep" && candidate.protections.is_empty();
    }
}

fn collect_codex_sessions_at(
    home: &Path,
    candidates: &mut Vec<SessionCleanupCandidate>,
    inventory: &mut Vec<FileFingerprint>,
    cache: &mut SessionCleanupCache,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let roots = [
        (home.join(".codex/sessions"), "active"),
        (home.join(".codex/archived_sessions"), "archived"),
    ];
    for (root, state) in roots {
        if !root.is_dir() {
            continue;
        }
        visit_files(&root, &mut |path| {
            if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
            {
                if let Some(parsed) = parse_or_cached_session(
                    path,
                    "codex",
                    state,
                    inventory,
                    cache,
                    parse_codex_session,
                ) {
                    candidates.push(to_candidate("codex", state, path, parsed, now));
                }
            }
        })?;
    }
    Ok(())
}

fn collect_claude_sessions_at(
    home: &Path,
    candidates: &mut Vec<SessionCleanupCandidate>,
    inventory: &mut Vec<FileFingerprint>,
    cache: &mut SessionCleanupCache,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let root = home.join(".claude/projects");
    if !root.is_dir() {
        return Ok(());
    }
    visit_files(&root, &mut |path| {
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
        {
            return;
        }
        if let Some(mut parsed) = parse_or_cached_session(
            path,
            "claude",
            "active",
            inventory,
            cache,
            parse_claude_session,
        ) {
            if parsed.id.is_empty() {
                parsed.id = path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_string();
            }
            candidates.push(to_candidate("claude", "active", path, parsed, now));
        }
    })?;
    Ok(())
}

fn parse_or_cached_session(
    path: &Path,
    tool_key: &str,
    storage_state: &str,
    inventory: &mut Vec<FileFingerprint>,
    cache: &mut SessionCleanupCache,
    parser: fn(&Path) -> Result<ParsedSession, String>,
) -> Option<ParsedSession> {
    let metadata = fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let modified_ns = modified
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos()
        .to_string();
    let normalized_path = normalize_source_path(path);
    let fingerprint = FileFingerprint {
        normalized_path: normalized_path.clone(),
        size_bytes: metadata.len(),
        modified_ns: modified_ns.clone(),
    };
    inventory.push(fingerprint);

    if let Some(entry) = cache.entries.get(&normalized_path) {
        if entry.parser_version == SESSION_CLEANUP_PARSER_VERSION
            && entry.tool_key == tool_key
            && entry.storage_state == storage_state
            && entry.size_bytes == metadata.len()
            && entry.modified_ns == modified_ns
        {
            return cached_parsed_session(&entry.parsed);
        }
    }

    let parsed = parser(path).ok()?;
    cache.entries.insert(
        normalized_path.clone(),
        SessionCacheEntry {
            parser_version: SESSION_CLEANUP_PARSER_VERSION,
            source_path: path.to_string_lossy().into_owned(),
            tool_key: tool_key.to_string(),
            storage_state: storage_state.to_string(),
            size_bytes: metadata.len(),
            modified_ns,
            parsed: CachedParsedSession::from_parsed(&parsed),
        },
    );
    Some(parsed)
}

fn normalize_source_path(path: &Path) -> String {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    {
        canonical
            .to_string_lossy()
            .replace('/', "\\")
            .to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        canonical.to_string_lossy().into_owned()
    }
}

fn file_modified_ns(path: &Path) -> Option<String> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos().to_string())
}

fn cache_path(home: &Path) -> PathBuf {
    home.join(".sessionatlas").join(SESSION_CLEANUP_CACHE_FILE)
}

fn load_cache(home: &Path) -> SessionCleanupCache {
    let path = cache_path(home);
    let Ok(bytes) = fs::read(path) else {
        return SessionCleanupCache::default();
    };
    let Ok(cache) = serde_json::from_slice::<SessionCleanupCache>(&bytes) else {
        return SessionCleanupCache::default();
    };
    if cache.version != SESSION_CLEANUP_CACHE_VERSION
        || cache.parser_version != SESSION_CLEANUP_PARSER_VERSION
    {
        return SessionCleanupCache::default();
    }
    cache
}

fn save_cache(home: &Path, cache: &SessionCleanupCache) {
    let path = cache_path(home);
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(bytes) = serde_json::to_vec(cache) else {
        return;
    };
    if fs::read(&path).is_ok_and(|current| current == bytes) {
        return;
    }
    let temporary = parent.join(format!(
        ".{SESSION_CLEANUP_CACHE_FILE}.{}.{}.tmp",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));
    let write_result = (|| -> std::io::Result<()> {
        {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options.open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        sessionatlas_core::config::atomic_replace_file(&temporary, &path)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
}

fn cached_parsed_session(cached: &CachedParsedSession) -> Option<ParsedSession> {
    Some(ParsedSession {
        id: cached.id.clone(),
        parent_id: cached.parent_id.clone(),
        project_path: cached.project_path.clone(),
        title: cached.title.clone(),
        cli_version: cached.cli_version.clone(),
        agent_kind: cached.agent_kind.clone(),
        latest_at: cached
            .latest_at
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc)),
        user_turns: cached.user_turns,
        tool_calls: cached.tool_calls,
        completed: cached.completed,
    })
}

fn fingerprint_inventory(inventory: &[FileFingerprint]) -> String {
    let mut entries: Vec<String> = inventory
        .iter()
        .map(|fingerprint| {
            format!(
                "{}\0{}\0{}",
                fingerprint.normalized_path, fingerprint.size_bytes, fingerprint.modified_ns
            )
        })
        .collect();
    entries.sort();
    stable_fingerprint(entries.iter().map(String::as_str))
}

fn fingerprint_candidates(
    candidates: &[SessionCleanupCandidate],
    inventory_fingerprint: &str,
) -> String {
    let mut entries: Vec<String> = candidates
        .iter()
        .map(|candidate| {
            format!(
                "{}\0{}\0{}\0{}\0{}\0{}",
                candidate.key,
                normalize_source_path(&candidate.source_path),
                candidate.size_bytes,
                candidate.observed_at.to_rfc3339(),
                candidate.classification,
                candidate.protections.join(",")
            )
        })
        .collect();
    entries.sort();
    stable_fingerprint(
        std::iter::once(inventory_fingerprint).chain(entries.iter().map(String::as_str)),
    )
}

fn stable_fingerprint<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("v{SESSION_CLEANUP_CACHE_VERSION}-{hash:016x}")
}

fn visit_files(root: &Path, visit: &mut impl FnMut(&Path)) -> Result<(), String> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("could not inspect {}: {error}", directory.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let file_type = entry.file_type().map_err(|error| error.to_string())?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                visit(&entry.path());
            }
        }
    }
    Ok(())
}

fn parse_codex_session(path: &Path) -> Result<ParsedSession, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut parsed = ParsedSession {
        agent_kind: "root".to_string(),
        ..ParsedSession::default()
    };
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|error| error.to_string())?;
        if line.contains("\"type\":\"session_meta\"") {
            if let Ok(record) = serde_json::from_str::<Value>(&line) {
                let payload = &record["payload"];
                parsed.id = json_string(payload, "id").unwrap_or_default();
                parsed.parent_id = json_string(payload, "parent_thread_id");
                parsed.project_path = json_string(payload, "cwd");
                parsed.cli_version = json_string(payload, "cli_version");
                let source = payload.get("source").cloned().unwrap_or(Value::Null);
                let source_text = source.to_string();
                parsed.agent_kind = if source_text.contains("guardian") {
                    "guardian".to_string()
                } else if parsed.parent_id.is_some() || source_text.contains("subagent") {
                    "delegated".to_string()
                } else {
                    "root".to_string()
                };
                update_latest(&mut parsed.latest_at, record.get("timestamp"));
                update_latest(&mut parsed.latest_at, payload.get("timestamp"));
            }
        } else if line.contains("\"type\":\"user_message\"") {
            parsed.user_turns += 1;
            if parsed.title.is_none() {
                if let Ok(record) = serde_json::from_str::<Value>(&line) {
                    parsed.title = json_string(&record["payload"], "message").map(short_title);
                }
            }
        } else if line.contains("\"type\":\"task_complete\"") {
            parsed.completed = true;
        } else if line.contains("\"type\":\"function_call\"")
            || line.contains("\"type\":\"custom_tool_call\"")
        {
            parsed.tool_calls += 1;
        }
        if let Ok(record) = serde_json::from_str::<Value>(&line) {
            update_latest(&mut parsed.latest_at, record.get("timestamp"));
        }
    }
    if parsed.id.is_empty() {
        return Err("missing Codex session id".to_string());
    }
    Ok(parsed)
}

fn parse_claude_session(path: &Path) -> Result<ParsedSession, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let is_subagent = path
        .components()
        .any(|component| component.as_os_str() == "subagents");
    let mut parsed = ParsedSession {
        agent_kind: if is_subagent { "delegated" } else { "root" }.to_string(),
        ..ParsedSession::default()
    };
    if is_subagent {
        parsed.parent_id = path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .map(str::to_string);
    }
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|error| error.to_string())?;
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if parsed.id.is_empty() {
            parsed.id = json_string(&record, "sessionId").unwrap_or_default();
        }
        if parsed.project_path.is_none() {
            parsed.project_path = json_string(&record, "cwd");
        }
        if parsed.cli_version.is_none() {
            parsed.cli_version = json_string(&record, "version");
        }
        update_latest(&mut parsed.latest_at, record.get("timestamp"));
        let role = record
            .get("message")
            .and_then(|message| message.get("role"))
            .and_then(Value::as_str)
            .or_else(|| record.get("type").and_then(Value::as_str));
        if role == Some("user") {
            parsed.user_turns += 1;
            if parsed.title.is_none() {
                parsed.title = extract_claude_text(&record).map(short_title);
            }
        } else if role == Some("assistant") {
            parsed.completed = true;
            parsed.tool_calls += count_claude_tool_calls(&record);
        }
    }
    Ok(parsed)
}

fn extract_claude_text(record: &Value) -> Option<String> {
    let content = record.get("message")?.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    let text = content
        .as_array()?
        .iter()
        .find_map(|item| item.get("text").and_then(Value::as_str))?;
    Some(text.to_string())
}

fn count_claude_tool_calls(record: &Value) -> usize {
    record
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| item.get("type").and_then(Value::as_str) == Some("tool_use"))
                .count()
        })
        .unwrap_or(0)
}

fn to_candidate(
    tool: &str,
    state: &str,
    path: &Path,
    parsed: ParsedSession,
    now: DateTime<Utc>,
) -> SessionCleanupCandidate {
    let modified = parsed.latest_at.unwrap_or_else(|| {
        path.metadata()
            .and_then(|metadata| metadata.modified())
            .map(DateTime::<Utc>::from)
            .unwrap_or(now)
    });
    let age_days = (now - modified).num_days().max(0);
    SessionCleanupCandidate {
        key: format!("{tool}:{state}:{}", parsed.id),
        tool_key: tool.to_string(),
        session_id: parsed.id,
        parent_session_id: parsed.parent_id,
        project_path: parsed.project_path,
        title: parsed.title,
        cli_version: parsed.cli_version,
        agent_kind: parsed.agent_kind,
        storage_state: state.to_string(),
        classification: "keep".to_string(),
        reasons: if parsed.completed {
            vec!["completed".to_string()]
        } else {
            Vec::new()
        },
        protections: Vec::new(),
        age_days,
        size_bytes: path.metadata().map(|metadata| metadata.len()).unwrap_or(0),
        user_turns: parsed.user_turns,
        tool_calls: parsed.tool_calls,
        can_clean: false,
        completed: parsed.completed,
        source_modified_ns: file_modified_ns(path).unwrap_or_default(),
        source_path: path.to_path_buf(),
        observed_at: modified,
    }
}

fn classify_candidate(
    candidate: &mut SessionCleanupCandidate,
    parent_present: bool,
    parent_completed: bool,
) {
    let completed = candidate.completed;
    if candidate.agent_kind == "guardian"
        && parent_present
        && parent_completed
        && completed
        && candidate.age_days >= RECENT_PROTECTION_DAYS
    {
        candidate.classification = "likely".to_string();
        candidate.reasons.push("guardianDelivered".to_string());
        return;
    }
    if candidate.agent_kind == "root"
        && candidate.age_days >= LIKELY_TRIVIAL_AGE_DAYS
        && candidate.user_turns <= 1
        && candidate.tool_calls == 0
        && candidate.size_bytes < 64 * 1024
    {
        candidate.classification = "likely".to_string();
        candidate.reasons.push("oldTrivial".to_string());
        return;
    }
    let completed_child = candidate.agent_kind != "root"
        && parent_present
        && completed
        && candidate.age_days >= TRIVIAL_AGE_DAYS;
    let short_single_turn = candidate.agent_kind == "root"
        && candidate.age_days >= TRIVIAL_AGE_DAYS
        && candidate.user_turns <= 1
        && candidate.tool_calls == 0
        && candidate.size_bytes < 200 * 1024;
    let old_codex = candidate.tool_key == "codex"
        && candidate.age_days >= TRIVIAL_AGE_DAYS
        && candidate
            .cli_version
            .as_deref()
            .and_then(codex_minor)
            .is_some_and(|minor| minor < OLD_CODEX_MINOR);
    if completed_child || short_single_turn || old_codex {
        candidate.classification = "possible".to_string();
        if completed_child {
            candidate.reasons.push("completedChild".to_string());
        }
        if short_single_turn {
            candidate.reasons.push("shortSingleTurn".to_string());
        }
        if old_codex {
            candidate.reasons.push("oldVersion".to_string());
        }
    }
}

fn codex_minor(version: &str) -> Option<u32> {
    let mut parts = version.split('.');
    if parts.next()? != "0" {
        return None;
    }
    parts.next()?.parse().ok()
}

fn update_latest(current: &mut Option<DateTime<Utc>>, value: Option<&Value>) {
    let Some(text) = value.and_then(Value::as_str) else {
        return;
    };
    let Ok(parsed) = DateTime::parse_from_rfc3339(text) else {
        return;
    };
    let parsed = parsed.with_timezone(&Utc);
    if current.is_none_or(|existing| parsed > existing) {
        *current = Some(parsed);
    }
}

fn json_string(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

fn short_title(value: String) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(120).collect()
}

fn trash_root(home: &Path) -> PathBuf {
    home.join(".sessionatlas/session-trash")
}

pub(crate) fn quarantine_sessions(
    home: &Path,
    snapshot_id: &str,
    keys: &[String],
) -> Result<SessionTrashBatch, String> {
    if keys.is_empty() || keys.len() > 500 {
        return Err("select between 1 and 500 session candidates".to_string());
    }
    if snapshot_id.trim().is_empty() {
        return Err("session analysis is stale; analyze again".to_string());
    }
    let selected: HashSet<&str> = keys.iter().map(String::as_str).collect();
    let lock = SESSION_CLEANUP_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock
        .lock()
        .map_err(|_| "session cleanup lock poisoned".to_string())?;
    let cache = load_cache(home);
    if cache.snapshot_id != snapshot_id {
        return Err("one or more session candidates are stale; analyze again".to_string());
    }
    let inventory = collect_inventory(home)?;
    if fingerprint_inventory(&inventory) != cache.inventory_fingerprint {
        return Err("session inventory changed; analyze again".to_string());
    }
    let inventory_paths: HashSet<String> = inventory
        .iter()
        .map(|fingerprint| fingerprint.normalized_path.clone())
        .collect();
    let mut current = cache
        .candidates
        .into_iter()
        .filter_map(CachedCandidate::into_candidate)
        .collect::<Vec<_>>();
    apply_candidate_safety(&mut current);
    let candidates: Vec<_> = current
        .into_iter()
        .filter(|candidate| selected.contains(candidate.key.as_str()))
        .collect();
    if candidates.len() != selected.len() {
        return Err("one or more session candidates are stale; analyze again".to_string());
    }
    if candidates.iter().any(|candidate| !candidate.can_clean) {
        return Err("one or more selected sessions are protected".to_string());
    }
    for candidate in &candidates {
        if !inventory_paths.contains(&normalize_source_path(&candidate.source_path)) {
            return Err("one or more session candidates are stale; analyze again".to_string());
        }
        let fingerprint = file_fingerprint(&candidate.source_path)
            .ok_or_else(|| "one or more session candidates are stale; analyze again".to_string())?;
        if fingerprint.size_bytes != candidate.size_bytes
            || fingerprint.modified_ns != candidate.source_modified_ns
        {
            return Err("one or more session candidates are stale; analyze again".to_string());
        }
    }

    let batch_id = Utc::now().format("%Y%m%dT%H%M%S%.3fZ").to_string();
    let batch_dir = trash_root(home).join(&batch_id);
    fs::create_dir_all(&batch_dir).map_err(|error| error.to_string())?;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut entries = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let file_name = candidate
            .source_path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "session file name is not valid UTF-8".to_string())?;
        let destination = batch_dir.join(format!("{index:04}-{file_name}"));
        if let Err(error) = fs::rename(&candidate.source_path, &destination) {
            rollback_moves(&moved);
            return Err(format!(
                "could not move session into recovery area: {error}"
            ));
        }
        moved.push((candidate.source_path.clone(), destination.clone()));
        entries.push(TrashManifestEntry {
            original_path: candidate.source_path.to_string_lossy().into_owned(),
            trashed_path: destination.to_string_lossy().into_owned(),
            key: candidate.key.clone(),
            size_bytes: candidate.size_bytes,
        });
    }
    let manifest = TrashManifest {
        batch_id: batch_id.clone(),
        created_at: Utc::now().to_rfc3339(),
        entries,
    };
    let manifest_path = batch_dir.join("manifest.json");
    let json = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    if let Err(error) = fs::write(&manifest_path, json) {
        rollback_moves(&moved);
        return Err(format!("could not write recovery manifest: {error}"));
    }
    Ok(batch_summary(&manifest))
}

fn file_fingerprint(path: &Path) -> Option<FileFingerprint> {
    let metadata = fs::metadata(path).ok()?;
    Some(FileFingerprint {
        normalized_path: normalize_source_path(path),
        size_bytes: metadata.len(),
        modified_ns: file_modified_ns(path)?,
    })
}

fn collect_inventory(home: &Path) -> Result<Vec<FileFingerprint>, String> {
    let mut inventory = Vec::new();
    let roots = [
        home.join(".codex/sessions"),
        home.join(".codex/archived_sessions"),
        home.join(".claude/projects"),
    ];
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        visit_files(&root, &mut |path| {
            if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
            {
                if let Some(fingerprint) = file_fingerprint(path) {
                    inventory.push(fingerprint);
                }
            }
        })?;
    }
    inventory.sort_by(|left, right| left.normalized_path.cmp(&right.normalized_path));
    Ok(inventory)
}

fn rollback_moves(moved: &[(PathBuf, PathBuf)]) {
    for (original, destination) in moved.iter().rev() {
        let _ = fs::rename(destination, original);
    }
}

pub(crate) fn list_session_trash(home: &Path) -> Result<Vec<SessionTrashBatch>, String> {
    let root = trash_root(home);
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut batches = Vec::new();
    for entry in fs::read_dir(&root).map_err(|error| error.to_string())? {
        let path = entry
            .map_err(|error| error.to_string())?
            .path()
            .join("manifest.json");
        if !path.is_file() {
            continue;
        }
        let json = fs::read(&path).map_err(|error| error.to_string())?;
        if let Ok(manifest) = serde_json::from_slice::<TrashManifest>(&json) {
            batches.push(batch_summary(&manifest));
        }
    }
    batches.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(batches)
}

pub(crate) fn restore_session_trash(home: &Path, batch_id: &str) -> Result<usize, String> {
    if batch_id.is_empty()
        || !batch_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '.' | 'T' | 'Z')
        })
    {
        return Err("invalid recovery batch id".to_string());
    }
    let batch_dir = trash_root(home).join(batch_id);
    let manifest_path = batch_dir.join("manifest.json");
    let json = fs::read(&manifest_path).map_err(|error| error.to_string())?;
    let manifest: TrashManifest =
        serde_json::from_slice(&json).map_err(|error| error.to_string())?;
    if manifest.batch_id != batch_id {
        return Err("recovery manifest does not match its directory".to_string());
    }
    let mut restored = Vec::new();
    for entry in &manifest.entries {
        let original = PathBuf::from(&entry.original_path);
        let trashed = PathBuf::from(&entry.trashed_path);
        if original.exists() {
            rollback_restores(&restored);
            return Err(format!(
                "original session path already exists: {}",
                original.display()
            ));
        }
        if let Some(parent) = original.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        if let Err(error) = fs::rename(&trashed, &original) {
            rollback_restores(&restored);
            return Err(format!("could not restore session: {error}"));
        }
        restored.push((original, trashed));
    }
    fs::remove_file(&manifest_path).map_err(|error| error.to_string())?;
    let _ = fs::remove_dir(&batch_dir);
    Ok(restored.len())
}

fn rollback_restores(restored: &[(PathBuf, PathBuf)]) {
    for (original, trashed) in restored.iter().rev() {
        let _ = fs::rename(original, trashed);
    }
}

fn batch_summary(manifest: &TrashManifest) -> SessionTrashBatch {
    SessionTrashBatch {
        batch_id: manifest.batch_id.clone(),
        created_at: manifest.created_at.clone(),
        session_count: manifest.entries.len(),
        size_bytes: manifest.entries.iter().map(|entry| entry.size_bytes).sum(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_codex_session(
        home: &Path,
        id: &str,
        parent: Option<&str>,
        source: Value,
        title: &str,
        timestamp: &str,
    ) -> PathBuf {
        let path = home
            .join(".codex/sessions/2026/01/01")
            .join(format!("rollout-{id}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let meta = serde_json::json!({
            "timestamp": timestamp,
            "type": "session_meta",
            "payload": {
                "id": id,
                "parent_thread_id": parent,
                "timestamp": timestamp,
                "cwd": home.join("repo").to_string_lossy(),
                "cli_version": "0.139.0",
                "source": source
            }
        });
        let user = serde_json::json!({
            "timestamp": timestamp,
            "type": "event_msg",
            "payload": {"type": "user_message", "message": title}
        });
        let complete = serde_json::json!({
            "timestamp": timestamp,
            "type": "event_msg",
            "payload": {"type": "task_complete"}
        });
        fs::write(path.clone(), format!("{meta}\n{user}\n{complete}\n")).unwrap();
        path
    }

    #[test]
    fn guardian_child_with_completed_parent_is_likely_and_recoverable() {
        let home = tempfile::tempdir().unwrap();
        let now = DateTime::parse_from_rfc3339("2026-08-17T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        write_codex_session(
            home.path(),
            "parent",
            None,
            Value::String("vscode".to_string()),
            "main task",
            "2026-07-01T00:00:00Z",
        );
        write_codex_session(
            home.path(),
            "child",
            Some("parent"),
            serde_json::json!({"subagent": {"other": "guardian"}}),
            "audit",
            "2026-07-01T00:00:00Z",
        );
        let analysis = analyze_sessions_at(home.path(), now).unwrap();
        let child = analysis
            .candidates
            .iter()
            .find(|candidate| candidate.session_id == "child")
            .unwrap();
        assert_eq!(child.classification, "likely");
        assert!(child.can_clean);

        let batch = quarantine_sessions(
            home.path(),
            &analysis.snapshot_id,
            std::slice::from_ref(&child.key),
        )
        .unwrap();
        assert_eq!(batch.session_count, 1);
        assert_eq!(list_session_trash(home.path()).unwrap().len(), 1);
        assert_eq!(
            restore_session_trash(home.path(), &batch.batch_id).unwrap(),
            1
        );
        assert!(analysis
            .candidates
            .iter()
            .find(|candidate| candidate.session_id == "child")
            .unwrap()
            .source_path
            .is_file());
    }

    #[test]
    fn missing_parent_and_latest_project_sessions_are_protected() {
        let home = tempfile::tempdir().unwrap();
        let now = DateTime::parse_from_rfc3339("2026-08-17T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        write_codex_session(
            home.path(),
            "orphan",
            Some("missing"),
            serde_json::json!({"subagent": {"other": "guardian"}}),
            "audit",
            "2026-06-01T00:00:00Z",
        );
        let analysis = analyze_sessions_at(home.path(), now).unwrap();
        let orphan = &analysis.candidates[0];
        assert!(orphan.protections.contains(&"parentMissing".to_string()));
        assert!(!orphan.can_clean);
    }

    #[test]
    fn cleanup_cache_is_reused_and_corrupt_cache_is_rebuilt() {
        let home = tempfile::tempdir().unwrap();
        let now = DateTime::parse_from_rfc3339("2026-08-17T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let source_path = write_codex_session(
            home.path(),
            "cached",
            None,
            Value::String("vscode".to_string()),
            "cached session",
            "2026-01-01T00:00:00Z",
        );
        let first = analyze_sessions_at(home.path(), now).unwrap();
        let cache = cache_path(home.path());
        assert!(cache.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&cache).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let first_bytes = fs::read(&cache).unwrap();
        let first_source_size = fs::metadata(&source_path).unwrap().len();
        let first_cache_modified = fs::metadata(&cache).unwrap().modified().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(20));
        let second = analyze_sessions_at(home.path(), now).unwrap();
        assert_eq!(first.snapshot_id, second.snapshot_id);
        assert_eq!(first.candidates.len(), second.candidates.len());
        assert_eq!(first_bytes, fs::read(&cache).unwrap());
        assert_eq!(
            first_cache_modified,
            fs::metadata(&cache).unwrap().modified().unwrap()
        );

        std::thread::sleep(std::time::Duration::from_millis(2));
        write_codex_session(
            home.path(),
            "cached",
            None,
            Value::String("vscode".to_string()),
            "updated cached session with a deliberately different payload",
            "2026-02-01T00:00:00Z",
        );
        assert_ne!(
            first_source_size,
            fs::metadata(&source_path).unwrap().len(),
            "the invalidation fixture must change the source fingerprint"
        );
        assert_eq!(
            parse_codex_session(&source_path).unwrap().title.as_deref(),
            Some("updated cached session with a deliberately different payload")
        );
        let updated = analyze_sessions_at(home.path(), now).unwrap();
        assert_eq!(
            updated.candidates[0].title.as_deref(),
            Some("updated cached session with a deliberately different payload")
        );
        assert_ne!(first_bytes, fs::read(&cache).unwrap());

        fs::write(&cache, b"not json").unwrap();
        let rebuilt = analyze_sessions_at(home.path(), now).unwrap();
        assert_eq!(updated.snapshot_id, rebuilt.snapshot_id);
        assert!(serde_json::from_slice::<SessionCleanupCache>(&fs::read(cache).unwrap()).is_ok());
    }

    #[test]
    fn quarantine_rejects_a_stale_inventory_without_moving_files() {
        let home = tempfile::tempdir().unwrap();
        let now = DateTime::parse_from_rfc3339("2026-08-17T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let path = write_codex_session(
            home.path(),
            "stale",
            None,
            serde_json::json!({"subagent": {"other": "guardian"}}),
            "old child",
            "2026-01-01T00:00:00Z",
        );
        let analysis = analyze_sessions_at(home.path(), now).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        let error = quarantine_sessions(
            home.path(),
            &analysis.snapshot_id,
            std::slice::from_ref(&analysis.candidates[0].key),
        )
        .unwrap_err();
        assert!(error.contains("inventory changed") || error.contains("stale"));
        assert!(path.is_file());
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_source_paths_keep_backslashes_as_literal_bytes() {
        let left = Path::new("/tmp/session-cleanup-a\\b");
        let right = Path::new("/tmp/session-cleanup-a/b");
        assert_ne!(normalize_source_path(left), normalize_source_path(right));
    }
}
