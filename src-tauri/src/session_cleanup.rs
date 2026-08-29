use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

use sessionatlas_core::private_fs;

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
        let Some((safe_root, canonical_root)) = safe_source_root(home, &root) else {
            continue;
        };
        visit_files(&root, &mut |path| {
            if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
                && safe_candidate_path(path, &safe_root, &canonical_root)
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
    let Some((safe_root, canonical_root)) = safe_source_root(home, &root) else {
        return Ok(());
    };
    visit_files(&root, &mut |path| {
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
            || !safe_candidate_path(path, &safe_root, &canonical_root)
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
    let metadata = fs::symlink_metadata(path).ok()?;
    if !is_regular_file(&metadata) {
        return None;
    }
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

fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    if metadata.file_attributes() & 0x400 != 0 {
        return true;
    }
    false
}

fn is_regular_file(metadata: &fs::Metadata) -> bool {
    metadata.is_file() && !metadata_is_link_or_reparse(metadata)
}

fn is_safe_directory(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_dir() && !metadata_is_link_or_reparse(&metadata))
        .unwrap_or(false)
}

fn canonical_home(home: &Path) -> Option<PathBuf> {
    if !is_safe_directory(home) {
        return None;
    }
    let canonical = fs::canonicalize(home).ok()?;
    is_safe_directory(&canonical).then_some(canonical)
}

fn comparison_path(path: &Path) -> Option<String> {
    let mut value = path.to_str()?.to_string();
    #[cfg(windows)]
    {
        value = value.replace('/', "\\");
        if let Some(stripped) = value.strip_prefix(r"\\?\") {
            value = stripped.to_string();
        }
        value.make_ascii_lowercase();
    }
    Some(value)
}

fn is_same_or_child_path(candidate: &Path, parent: &Path) -> bool {
    let Some(candidate) = comparison_path(candidate) else {
        return false;
    };
    let Some(parent) = comparison_path(parent) else {
        return false;
    };
    sessionatlas_core::path::is_same_or_child_native(&candidate, &parent)
}

fn same_path(left: &Path, right: &Path) -> bool {
    comparison_path(left) == comparison_path(right)
}

fn safe_source_root(home: &Path, root: &Path) -> Option<(PathBuf, PathBuf)> {
    let canonical_home = canonical_home(home)?;
    if !is_safe_directory(root) {
        return None;
    }
    let canonical_root = fs::canonicalize(root).ok()?;
    if !is_same_or_child_path(&canonical_root, &canonical_home) {
        return None;
    }
    Some((root.to_path_buf(), canonical_root))
}

fn safe_parent_chain(path: &Path, root: &Path) -> bool {
    let mut current = path;
    loop {
        let Ok(metadata) = fs::symlink_metadata(current) else {
            return false;
        };
        if metadata_is_link_or_reparse(&metadata) {
            return false;
        }
        if same_path(current, root) {
            return true;
        }
        let Some(parent) = current.parent() else {
            return false;
        };
        if same_path(current, parent) {
            return false;
        }
        current = parent;
    }
}

fn safe_candidate_path(path: &Path, root: &Path, canonical_root: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !is_regular_file(&metadata) || !safe_parent_chain(path, root) {
        return false;
    }
    let Ok(canonical_path) = fs::canonicalize(path) else {
        return false;
    };
    is_same_or_child_path(&canonical_path, canonical_root)
}

fn approved_source_roots(home: &Path) -> Vec<(PathBuf, PathBuf)> {
    [
        home.join(".codex/sessions"),
        home.join(".codex/archived_sessions"),
        home.join(".claude/projects"),
    ]
    .into_iter()
    .filter_map(|root| safe_source_root(home, &root))
    .collect()
}

fn validate_source_file(home: &Path, path: &Path) -> Option<PathBuf> {
    for (root, canonical_root) in approved_source_roots(home) {
        if safe_candidate_path(path, &root, &canonical_root) {
            return Some(canonical_root);
        }
    }
    None
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
    if private_fs::harden_existing_private_file(&path).is_err() {
        return SessionCleanupCache::default();
    }
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
    if private_fs::ensure_private_directory(parent).is_err() {
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
            let mut file = private_fs::open_private_create_new(&temporary)?;
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
    if !is_safe_directory(root) {
        return Ok(());
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("could not inspect {}: {error}", directory.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
            if metadata_is_link_or_reparse(&metadata) {
                continue;
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                visit(&path);
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

fn safe_trash_root(home: &Path, create: bool) -> Result<Option<PathBuf>, String> {
    let canonical_home = canonical_home(home)
        .ok_or_else(|| "home directory is not a safe regular directory".to_string())?;
    let root = canonical_home.join(".sessionatlas").join("session-trash");
    let mut current = canonical_home.clone();
    for component in [".sessionatlas", "session-trash"] {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
                    return Err("recovery area contains a link or reparse point".to_string());
                }
                private_fs::ensure_private_directory(&current)
                    .map_err(|error| error.to_string())?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
                private_fs::ensure_private_directory(&current)
                    .map_err(|error| error.to_string())?;
                let metadata = fs::symlink_metadata(&current).map_err(|error| error.to_string())?;
                if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
                    return Err("recovery area contains a link or reparse point".to_string());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        }
    }
    let canonical_root = fs::canonicalize(&root).map_err(|error| error.to_string())?;
    if !is_same_or_child_path(&canonical_root, &canonical_home) {
        return Err("recovery area escapes the home directory".to_string());
    }
    Ok(Some(canonical_root))
}

fn safe_batch_directory(root: &Path, batch: &Path) -> Result<(), String> {
    if !is_safe_directory(root) || !safe_parent_chain(batch, root) {
        return Err("recovery batch is not inside a safe recovery area".to_string());
    }
    let canonical_root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    let canonical_batch = fs::canonicalize(batch).map_err(|error| error.to_string())?;
    if same_path(&canonical_batch, &canonical_root)
        || !is_same_or_child_path(&canonical_batch, &canonical_root)
    {
        return Err("recovery batch escapes the recovery area".to_string());
    }
    Ok(())
}

fn safe_trash_file(root: &Path, batch: &Path, path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !is_regular_file(&metadata) || safe_batch_directory(root, batch).is_err() {
        return false;
    }
    if !safe_parent_chain(path, batch) {
        return false;
    }
    let Ok(canonical_path) = fs::canonicalize(path) else {
        return false;
    };
    let Ok(canonical_batch) = fs::canonicalize(batch) else {
        return false;
    };
    is_same_or_child_path(&canonical_path, &canonical_batch)
}

fn safe_trash_destination(root: &Path, batch: &Path, path: &Path) -> bool {
    if fs::symlink_metadata(path).is_ok() || safe_batch_directory(root, batch).is_err() {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    if !safe_parent_chain(parent, batch) {
        return false;
    }
    let Ok(canonical_parent) = fs::canonicalize(parent) else {
        return false;
    };
    let Ok(canonical_batch) = fs::canonicalize(batch) else {
        return false;
    };
    is_same_or_child_path(&canonical_parent, &canonical_batch)
}

fn create_batch_directory(root: &Path) -> Result<(String, PathBuf), String> {
    let base = Utc::now().format("%Y%m%dT%H%M%S%.3fZ").to_string();
    for attempt in 0..100u32 {
        let batch_id = if attempt == 0 {
            base.clone()
        } else {
            format!("{base}-{attempt}")
        };
        let batch = root.join(&batch_id);
        match fs::create_dir(&batch) {
            Ok(()) => {
                private_fs::ensure_private_directory(&batch).map_err(|error| error.to_string())?;
                safe_batch_directory(root, &batch)?;
                return Ok((batch_id, batch));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("could not allocate a recovery batch".to_string())
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
        if !inventory_paths.contains(&normalize_source_path(&candidate.source_path))
            || validate_source_file(home, &candidate.source_path).is_none()
        {
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

    let trash =
        safe_trash_root(home, true)?.ok_or_else(|| "could not create recovery area".to_string())?;
    let (batch_id, batch_dir) = create_batch_directory(&trash)?;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut entries = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let file_name = candidate
            .source_path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "session file name is not valid UTF-8".to_string())?;
        let destination = batch_dir.join(format!("{index:04}-{file_name}"));
        // Recheck the source and both sides of the move immediately before rename.
        if validate_source_file(home, &candidate.source_path).is_none()
            || file_fingerprint(&candidate.source_path).is_none_or(|fingerprint| {
                fingerprint.size_bytes != candidate.size_bytes
                    || fingerprint.modified_ns != candidate.source_modified_ns
            })
            || safe_batch_directory(&trash, &batch_dir).is_err()
            || fs::symlink_metadata(&destination).is_ok()
        {
            rollback_moves(home, &trash, &batch_dir, &moved);
            return Err("session path or recovery area changed; analyze again".to_string());
        }
        if let Err(error) = fs::rename(&candidate.source_path, &destination) {
            rollback_moves(home, &trash, &batch_dir, &moved);
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
    let write_result = (|| -> std::io::Result<()> {
        let mut file = private_fs::open_private_create_new(&manifest_path)?;
        file.write_all(&json)?;
        file.sync_all()
    })();
    if let Err(error) = write_result {
        rollback_moves(home, &trash, &batch_dir, &moved);
        return Err(format!("could not write recovery manifest: {error}"));
    }
    Ok(batch_summary(&manifest))
}

fn file_fingerprint(path: &Path) -> Option<FileFingerprint> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !is_regular_file(&metadata) {
        return None;
    }
    Some(FileFingerprint {
        normalized_path: normalize_source_path(path),
        size_bytes: metadata.len(),
        modified_ns: file_modified_ns(path)?,
    })
}

fn collect_inventory(home: &Path) -> Result<Vec<FileFingerprint>, String> {
    let mut inventory = Vec::new();
    for (root, canonical_root) in approved_source_roots(home) {
        visit_files(&root, &mut |path| {
            if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
                && safe_candidate_path(path, &root, &canonical_root)
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

fn rollback_moves(home: &Path, root: &Path, batch: &Path, moved: &[(PathBuf, PathBuf)]) {
    for (original, destination) in moved.iter().rev() {
        if safe_trash_file(root, batch, destination)
            && validate_original_path(home, original).is_some()
            && fs::symlink_metadata(original).is_err()
        {
            let _ = fs::rename(destination, original);
        }
    }
}

pub(crate) fn list_session_trash(home: &Path) -> Result<Vec<SessionTrashBatch>, String> {
    let Some(root) = safe_trash_root(home, false)? else {
        return Ok(Vec::new());
    };
    let mut batches = Vec::new();
    for entry in fs::read_dir(&root).map_err(|error| error.to_string())? {
        let batch = entry.map_err(|error| error.to_string())?.path();
        if !is_safe_directory(&batch) {
            return Err("recovery area contains an unsafe batch".to_string());
        }
        safe_batch_directory(&root, &batch)?;
        let path = batch.join("manifest.json");
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        if !is_regular_file(&metadata) {
            return Err("recovery manifest is not a regular file".to_string());
        }
        if !safe_parent_chain(&path, &batch) {
            return Err("recovery manifest is outside its batch".to_string());
        }
        if !safe_trash_file(&root, &batch, &path) {
            return Err("recovery manifest is not in a safe batch".to_string());
        }
        let json = fs::read(&path).map_err(|error| error.to_string())?;
        if let Ok(manifest) = serde_json::from_slice::<TrashManifest>(&json) {
            batches.push(batch_summary(&manifest));
        }
    }
    batches.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(batches)
}

fn valid_batch_id(batch_id: &str) -> bool {
    !batch_id.is_empty()
        && batch_id != "."
        && batch_id != ".."
        && !batch_id.chars().all(|character| character == '.')
        && batch_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '.' | 'T' | 'Z')
        })
        && Path::new(batch_id)
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn validate_original_path(home: &Path, original: &Path) -> Option<(PathBuf, PathBuf)> {
    if !original.is_absolute()
        || original
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    let parent = original.parent()?;
    let mut existing = parent.to_path_buf();
    let mut missing = Vec::new();
    while fs::symlink_metadata(&existing).is_err() {
        missing.push(existing.clone());
        existing = existing.parent()?.to_path_buf();
    }
    let existing_metadata = fs::symlink_metadata(&existing).ok()?;
    if !existing_metadata.is_dir() || metadata_is_link_or_reparse(&existing_metadata) {
        return None;
    }
    let canonical_existing = fs::canonicalize(&existing).ok()?;
    for (root, canonical_root) in approved_source_roots(home) {
        if !safe_parent_chain(&existing, &root)
            || !is_same_or_child_path(&canonical_existing, &canonical_root)
        {
            continue;
        }
        let mut projected = canonical_existing.clone();
        for missing_path in missing.iter().rev() {
            projected.push(missing_path.file_name()?);
        }
        projected.push(original.file_name()?);
        if is_same_or_child_path(&projected, &canonical_root) {
            return Some((root, canonical_root));
        }
    }
    None
}

fn ensure_original_parent(home: &Path, original: &Path) -> Result<(), String> {
    let parent = original
        .parent()
        .ok_or_else(|| "original session path has no parent".to_string())?;
    let mut existing = parent.to_path_buf();
    let mut missing = Vec::new();
    while fs::symlink_metadata(&existing).is_err() {
        missing.push(existing.clone());
        existing = existing
            .parent()
            .ok_or_else(|| "original session path has no safe parent".to_string())?
            .to_path_buf();
    }
    validate_original_path(home, original)
        .ok_or_else(|| "original session path is outside approved roots".to_string())?;
    for directory in missing.iter().rev() {
        match fs::create_dir(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
        if !is_safe_directory(directory) {
            return Err("original session parent contains a link or reparse point".to_string());
        }
    }
    Ok(())
}

pub(crate) fn restore_session_trash(home: &Path, batch_id: &str) -> Result<usize, String> {
    if !valid_batch_id(batch_id) {
        return Err("invalid recovery batch id".to_string());
    }
    let lock = SESSION_CLEANUP_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock
        .lock()
        .map_err(|_| "session cleanup lock poisoned".to_string())?;
    let Some(root) = safe_trash_root(home, false)? else {
        return Err("recovery batch was not found".to_string());
    };
    let batch_dir = root.join(batch_id);
    safe_batch_directory(&root, &batch_dir)?;
    let manifest_path = batch_dir.join("manifest.json");
    let manifest_metadata =
        fs::symlink_metadata(&manifest_path).map_err(|error| error.to_string())?;
    if !is_regular_file(&manifest_metadata) || !safe_trash_file(&root, &batch_dir, &manifest_path) {
        return Err("recovery manifest is not a safe regular file".to_string());
    }
    let json = fs::read(&manifest_path).map_err(|error| error.to_string())?;
    let manifest: TrashManifest =
        serde_json::from_slice(&json).map_err(|error| error.to_string())?;
    if manifest.batch_id != batch_id {
        return Err("recovery manifest does not match its directory".to_string());
    }

    // Validate every manifest entry before moving any file. This makes a
    // modified manifest fail closed without a partial restore.
    let manifest_key = comparison_path(&manifest_path)
        .ok_or_else(|| "recovery manifest path is not valid".to_string())?;
    let mut endpoints = HashSet::new();
    for entry in &manifest.entries {
        let original = PathBuf::from(&entry.original_path);
        let trashed = PathBuf::from(&entry.trashed_path);
        let original_key = comparison_path(&original)
            .ok_or_else(|| "recovery manifest contains an invalid original path".to_string())?;
        let trashed_key = comparison_path(&trashed)
            .ok_or_else(|| "recovery manifest contains an invalid trashed path".to_string())?;
        if trashed_key == manifest_key
            || !endpoints.insert(original_key)
            || !endpoints.insert(trashed_key)
        {
            return Err("recovery manifest contains duplicate or conflicting paths".to_string());
        }
        if validate_original_path(home, &original).is_none() {
            return Err("recovery manifest contains an unsafe original path".to_string());
        }
        if fs::symlink_metadata(&original).is_ok() {
            return Err(format!(
                "original session path already exists: {}",
                original.display()
            ));
        }
        if !safe_trash_file(&root, &batch_dir, &trashed) {
            return Err("recovery manifest contains an unsafe trashed path".to_string());
        }
    }

    let mut restored = Vec::new();
    for entry in &manifest.entries {
        let original = PathBuf::from(&entry.original_path);
        let trashed = PathBuf::from(&entry.trashed_path);
        if let Err(error) = ensure_original_parent(home, &original) {
            rollback_restores(home, &root, &batch_dir, &restored);
            return Err(error);
        }
        // Revalidate both endpoints immediately before every rename.
        if validate_original_path(home, &original).is_none() {
            rollback_restores(home, &root, &batch_dir, &restored);
            return Err("original session path changed; refusing restore".to_string());
        }
        if fs::symlink_metadata(&original).is_ok()
            || !safe_trash_file(&root, &batch_dir, &trashed)
            || safe_batch_directory(&root, &batch_dir).is_err()
            || safe_trash_root(home, false)
                .ok()
                .flatten()
                .is_none_or(|current| !same_path(&current, &root))
        {
            rollback_restores(home, &root, &batch_dir, &restored);
            return Err("recovery paths changed; refusing restore".to_string());
        }
        if let Err(error) = fs::rename(&trashed, &original) {
            rollback_restores(home, &root, &batch_dir, &restored);
            return Err(format!("could not restore session: {error}"));
        }
        restored.push((original, trashed));
    }
    if !safe_trash_file(&root, &batch_dir, &manifest_path) {
        rollback_restores(home, &root, &batch_dir, &restored);
        return Err("recovery manifest changed; refusing restore".to_string());
    }
    if let Err(error) = safe_batch_directory(&root, &batch_dir) {
        rollback_restores(home, &root, &batch_dir, &restored);
        return Err(error);
    }
    fs::remove_file(&manifest_path).map_err(|error| {
        rollback_restores(home, &root, &batch_dir, &restored);
        error.to_string()
    })?;
    let _ = fs::remove_dir(&batch_dir);
    Ok(restored.len())
}

fn rollback_restores(home: &Path, root: &Path, batch: &Path, restored: &[(PathBuf, PathBuf)]) {
    for (original, trashed) in restored.iter().rev() {
        if validate_source_file(home, original).is_some()
            && safe_trash_destination(root, batch, trashed)
        {
            let _ = fs::rename(original, trashed);
        }
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

    fn make_directory_link(link: &Path, target: &Path) -> bool {
        #[cfg(unix)]
        {
            let result = std::os::unix::fs::symlink(target, link);
            assert!(
                result.is_ok(),
                "symlink fixture creation failed ({} -> {}): {}",
                link.display(),
                target.display(),
                result.unwrap_err()
            );
            true
        }
        #[cfg(windows)]
        {
            let link_text = link.to_string_lossy().into_owned();
            let link_text = link_text
                .strip_prefix(r"\\?\")
                .unwrap_or(&link_text)
                .replace('/', "\\");
            let target_text = target.to_string_lossy().into_owned();
            let target_text = target_text
                .strip_prefix(r"\\?\")
                .unwrap_or(&target_text)
                .replace('/', "\\");
            let command = format!("mklink /J {link_text} {target_text}");
            let output = std::process::Command::new("cmd")
                .args(["/c", &command])
                .output()
                .expect("mklink command must start");
            assert!(
                output.status.success(),
                "junction fixture creation failed ({} -> {}): {}",
                link_text,
                target_text,
                String::from_utf8_lossy(&output.stderr)
            );
            true
        }
    }

    fn remove_directory_link(path: &Path) {
        #[cfg(unix)]
        {
            let _ = fs::remove_file(path);
        }
        #[cfg(windows)]
        {
            let _ = fs::remove_dir(path);
        }
    }

    fn replace_directory_with_link(root: &Path, target: &Path) -> Option<PathBuf> {
        let backup =
            root.with_file_name(format!("{}-original", root.file_name()?.to_string_lossy()));
        fs::rename(root, &backup).ok()?;
        if !make_directory_link(root, target) {
            let _ = fs::rename(&backup, root);
            return None;
        }
        Some(backup)
    }

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
            let data_mode = fs::metadata(home.path().join(".sessionatlas"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            let mode = fs::metadata(&cache).unwrap().permissions().mode() & 0o777;
            assert_eq!(data_mode, 0o700);
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

    #[cfg(any(unix, windows))]
    #[test]
    fn source_root_and_nested_directory_links_never_become_candidates() {
        let home = tempfile::tempdir().unwrap();
        let now = DateTime::parse_from_rfc3339("2026-08-17T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let regular = write_codex_session(
            home.path(),
            "regular",
            None,
            serde_json::json!({"subagent": {"other": "guardian"}}),
            "regular",
            "2026-01-01T00:00:00Z",
        );
        let external = tempfile::tempdir().unwrap();
        let external_session = write_codex_session(
            external.path(),
            "external",
            None,
            serde_json::json!({"subagent": {"other": "guardian"}}),
            "external",
            "2026-01-01T00:00:00Z",
        );
        let nested_link = home.path().join(".codex/sessions/nested-link");
        fs::create_dir_all(nested_link.parent().unwrap()).unwrap();
        if !make_directory_link(&nested_link, external.path()) {
            eprintln!("directory-link fixture unavailable; link test not verified");
            return;
        }
        let analysis = analyze_sessions_at(home.path(), now).unwrap();
        assert!(analysis
            .candidates
            .iter()
            .all(|candidate| candidate.source_path != external_session));
        assert!(analysis.candidates.iter().any(|candidate| {
            candidate.source_path == fs::canonicalize(&regular).unwrap()
                || candidate.source_path == regular
        }));
        remove_directory_link(&nested_link);

        let root = home.path().join(".codex/sessions");
        let outside_root = tempfile::tempdir().unwrap();
        let backup = replace_directory_with_link(&root, outside_root.path());
        if backup.is_none() {
            eprintln!("top-level directory-link fixture unavailable; nested link was verified");
            return;
        }
        let replaced = analyze_sessions_at(home.path(), now).unwrap();
        assert!(replaced
            .candidates
            .iter()
            .all(|candidate| { !candidate.source_path.starts_with(outside_root.path()) }));
        remove_directory_link(&root);
        fs::rename(backup.unwrap(), &root).unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn quarantine_rejects_source_root_replacement_after_analysis() {
        let home = tempfile::tempdir().unwrap();
        let now = DateTime::parse_from_rfc3339("2026-08-17T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let source = write_codex_session(
            home.path(),
            "replace-root",
            None,
            serde_json::json!({"subagent": {"other": "guardian"}}),
            "replace root",
            "2026-01-01T00:00:00Z",
        );
        let analysis = analyze_sessions_at(home.path(), now).unwrap();
        let candidate = analysis
            .candidates
            .iter()
            .find(|candidate| candidate.source_path == source)
            .unwrap();
        let external = tempfile::tempdir().unwrap();
        let external_file = external.path().join("outside.jsonl");
        fs::write(&external_file, b"outside").unwrap();
        let root = home.path().join(".codex/sessions");
        let backup = replace_directory_with_link(&root, external.path());
        if backup.is_none() {
            eprintln!("source-root replacement fixture unavailable; not verified");
            return;
        }
        let result = quarantine_sessions(
            home.path(),
            &analysis.snapshot_id,
            std::slice::from_ref(&candidate.key),
        );
        assert!(result.is_err());
        assert!(external_file.is_file());
        assert!(!home.path().join(".sessionatlas/session-trash").exists());
        remove_directory_link(&root);
        fs::rename(backup.unwrap(), &root).unwrap();
    }

    #[test]
    fn restore_rejects_tampered_manifest_paths_and_unsafe_batch_ids() {
        let home = tempfile::tempdir().unwrap();
        let now = DateTime::parse_from_rfc3339("2026-08-17T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let source = write_codex_session(
            home.path(),
            "tamper",
            None,
            serde_json::json!({"subagent": {"other": "guardian"}}),
            "tamper",
            "2026-01-01T00:00:00Z",
        );
        let analysis = analyze_sessions_at(home.path(), now).unwrap();
        let key = analysis
            .candidates
            .iter()
            .find(|candidate| candidate.source_path == source)
            .map(|candidate| candidate.key.clone())
            .unwrap();
        let batch = quarantine_sessions(home.path(), &analysis.snapshot_id, &[key]).unwrap();
        assert!(restore_session_trash(home.path(), ".").is_err());
        assert!(restore_session_trash(home.path(), "..").is_err());
        assert!(restore_session_trash(home.path(), "...").is_err());

        let root = safe_trash_root(home.path(), false).unwrap().unwrap();
        let batch_dir = root.join(&batch.batch_id);
        let manifest_path = batch_dir.join("manifest.json");
        let mut manifest: TrashManifest =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest.entries[0].trashed_path = manifest_path.to_string_lossy().into_owned();
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(restore_session_trash(home.path(), &batch.batch_id).is_err());
        assert!(manifest_path.is_file());
        assert!(!source.exists());

        manifest = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        let original = manifest.entries[0].original_path.clone();
        manifest.entries[0].original_path = tempfile::tempdir()
            .unwrap()
            .path()
            .join("outside.jsonl")
            .to_string_lossy()
            .into_owned();
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(restore_session_trash(home.path(), &batch.batch_id).is_err());
        assert!(!PathBuf::from(original).exists());

        manifest.entries[0].original_path = source.to_string_lossy().into_owned();
        manifest.entries[0].trashed_path = home
            .path()
            .join("outside-trashed.jsonl")
            .to_string_lossy()
            .into_owned();
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(restore_session_trash(home.path(), &batch.batch_id).is_err());
        assert!(!source.exists());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn list_and_restore_reject_linked_trash_root_and_batch() {
        let home = tempfile::tempdir().unwrap();
        let now = DateTime::parse_from_rfc3339("2026-08-17T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let source = write_codex_session(
            home.path(),
            "linked-trash",
            None,
            serde_json::json!({"subagent": {"other": "guardian"}}),
            "linked trash",
            "2026-01-01T00:00:00Z",
        );
        let analysis = analyze_sessions_at(home.path(), now).unwrap();
        let key = analysis
            .candidates
            .iter()
            .find(|candidate| candidate.source_path == source)
            .map(|candidate| candidate.key.clone())
            .unwrap();
        let batch = quarantine_sessions(home.path(), &analysis.snapshot_id, &[key]).unwrap();
        let root = safe_trash_root(home.path(), false).unwrap().unwrap();
        let external_root = tempfile::tempdir().unwrap();
        let root_backup = root.with_file_name("session-trash-original");
        fs::rename(&root, &root_backup).unwrap();
        if !make_directory_link(&root, external_root.path()) {
            eprintln!("trash-root link fixture unavailable; not verified");
            fs::rename(&root_backup, &root).unwrap();
            return;
        }
        assert!(list_session_trash(home.path()).is_err());
        assert!(restore_session_trash(home.path(), &batch.batch_id).is_err());
        remove_directory_link(&root);
        fs::rename(&root_backup, &root).unwrap();

        let batch_dir = root.join(&batch.batch_id);
        let batch_backup = root.join(format!("{}-original", batch.batch_id));
        let external_batch = tempfile::tempdir().unwrap();
        fs::rename(&batch_dir, &batch_backup).unwrap();
        if !make_directory_link(&batch_dir, external_batch.path()) {
            eprintln!("trash-batch link fixture unavailable; root link was verified");
            fs::rename(&batch_backup, &batch_dir).unwrap();
            return;
        }
        assert!(list_session_trash(home.path()).is_err());
        assert!(restore_session_trash(home.path(), &batch.batch_id).is_err());
        remove_directory_link(&batch_dir);
        fs::rename(batch_backup, batch_dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unix_normal_paths_are_contained_through_analysis_quarantine_and_restore() {
        let home = tempfile::tempdir().unwrap();
        let now = DateTime::parse_from_rfc3339("2026-08-17T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let source = write_codex_session(
            home.path(),
            "unix-contained",
            Some("parent"),
            serde_json::json!({"subagent": {"other": "guardian"}}),
            "unix contained",
            "2026-01-01T00:00:00Z",
        );
        let root = home.path().join(".codex/sessions");
        let (safe_root, canonical_root) = safe_source_root(home.path(), &root).unwrap();
        assert!(safe_candidate_path(&source, &safe_root, &canonical_root));
        let analysis = analyze_sessions_at(home.path(), now).unwrap();
        let candidate = analysis
            .candidates
            .iter()
            .find(|candidate| candidate.source_path == source)
            .unwrap();
        // Make the child independently cleanable without relying on a
        // platform-specific link fixture.
        let mut candidate = candidate.clone();
        candidate.can_clean = true;
        let batch = quarantine_sessions(
            home.path(),
            &analysis.snapshot_id,
            std::slice::from_ref(&candidate.key),
        );
        assert!(
            batch.is_err(),
            "the stale cache candidate must be authoritative"
        );

        let parent = write_codex_session(
            home.path(),
            "unix-parent",
            None,
            serde_json::json!("vscode"),
            "unix parent",
            "2026-01-01T00:00:00Z",
        );
        let child = write_codex_session(
            home.path(),
            "unix-child",
            Some("unix-parent"),
            serde_json::json!({"subagent": {"other": "guardian"}}),
            "unix child",
            "2026-01-01T00:00:00Z",
        );
        let analysis = analyze_sessions_at(home.path(), now).unwrap();
        let child_candidate = analysis
            .candidates
            .iter()
            .find(|candidate| candidate.source_path == child)
            .unwrap();
        assert!(safe_candidate_path(&child, &safe_root, &canonical_root));
        let batch = quarantine_sessions(
            home.path(),
            &analysis.snapshot_id,
            std::slice::from_ref(&child_candidate.key),
        )
        .unwrap();
        let trash = safe_trash_root(home.path(), false).unwrap().unwrap();
        let batch_dir = trash.join(&batch.batch_id);
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&trash).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&batch_dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(batch_dir.join("manifest.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert!(is_same_or_child_path(&batch_dir, &trash));
        assert!(!same_path(&batch_dir, &trash));
        assert!(!child.exists());
        assert!(parent.exists());
        assert_eq!(
            restore_session_trash(home.path(), &batch.batch_id).unwrap(),
            1
        );
        assert!(child.is_file());
    }

    #[test]
    fn rollback_restores_all_previously_restored_files_and_fails_closed() {
        let home = tempfile::tempdir().unwrap();
        let now = DateTime::parse_from_rfc3339("2026-08-17T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let parent = write_codex_session(
            home.path(),
            "rollback-parent",
            None,
            serde_json::json!("vscode"),
            "rollback parent",
            "2026-01-01T00:00:00Z",
        );
        let child_one = write_codex_session(
            home.path(),
            "rollback-one",
            Some("rollback-parent"),
            serde_json::json!({"subagent": {"other": "guardian"}}),
            "rollback one",
            "2026-01-01T00:00:00Z",
        );
        let child_two = write_codex_session(
            home.path(),
            "rollback-two",
            Some("rollback-parent"),
            serde_json::json!({"subagent": {"other": "guardian"}}),
            "rollback two",
            "2026-01-01T00:00:00Z",
        );
        let analysis = analyze_sessions_at(home.path(), now).unwrap();
        let keys = analysis
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.source_path == child_one || candidate.source_path == child_two
            })
            .map(|candidate| candidate.key.clone())
            .collect::<Vec<_>>();
        assert_eq!(keys.len(), 2);
        let batch = quarantine_sessions(home.path(), &analysis.snapshot_id, &keys).unwrap();
        let root = safe_trash_root(home.path(), false).unwrap().unwrap();
        let batch_dir = root.join(&batch.batch_id);
        let manifest: TrashManifest =
            serde_json::from_slice(&fs::read(batch_dir.join("manifest.json")).unwrap()).unwrap();
        let first = (
            PathBuf::from(&manifest.entries[0].original_path),
            PathBuf::from(&manifest.entries[0].trashed_path),
        );
        let second = (
            PathBuf::from(&manifest.entries[1].original_path),
            PathBuf::from(&manifest.entries[1].trashed_path),
        );
        fs::rename(&first.1, &first.0).unwrap();
        fs::rename(&second.1, &second.0).unwrap();
        rollback_restores(
            home.path(),
            &root,
            &batch_dir,
            &[first.clone(), second.clone()],
        );
        assert!(!child_one.exists());
        assert!(!child_two.exists());
        assert!(fs::symlink_metadata(&first.1).is_ok());
        assert!(fs::symlink_metadata(&second.1).is_ok());

        fs::rename(&first.1, &first.0).unwrap();
        fs::rename(&second.1, &second.0).unwrap();
        let unsafe_trashed = home.path().join("unsafe-rollback-target.jsonl");
        fs::write(&unsafe_trashed, b"occupied").unwrap();
        let mut rollback_entries = vec![first, second];
        rollback_entries[1].1 = unsafe_trashed.clone();
        rollback_restores(home.path(), &root, &batch_dir, &rollback_entries);
        assert!(!child_one.exists());
        assert!(child_two.exists());
        assert!(fs::symlink_metadata(&manifest.entries[0].trashed_path).is_ok());
        assert!(fs::symlink_metadata(&unsafe_trashed).is_ok());
        assert!(parent.exists());
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_source_paths_keep_backslashes_as_literal_bytes() {
        let left = Path::new("/tmp/session-cleanup-a\\b");
        let right = Path::new("/tmp/session-cleanup-a/b");
        assert_ne!(normalize_source_path(left), normalize_source_path(right));
    }
}
