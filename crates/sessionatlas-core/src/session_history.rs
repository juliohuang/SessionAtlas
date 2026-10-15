//! Bounded, read-only per-project catalog of native AI session metadata.
//!
//! This module deliberately does not call the project scanners: those scanners
//! may update their metadata cache. History text is held only while this call
//! runs and is reduced to a short, credential-filtered user preview.

use std::cmp::Reverse;
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::Value;

const MAX_DEPTH: usize = 8;
const MAX_ENTRIES: usize = 8_000;
const MAX_FILES: usize = 1_200;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 64 * 1024;
const MAX_RECORDS_PER_FILE: usize = 5_000;
const MAX_SESSIONS: usize = 200;
const MAX_FILE_DURATION: Duration = Duration::from_secs(2);
const MAX_CALL_DURATION: Duration = Duration::from_secs(8);

#[derive(Clone, Debug)]
pub struct HistorySource {
    pub tool_key: String,
    pub root: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistory {
    pub sessions: Vec<HistorySession>,
    pub issues: Vec<HistoryIssue>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySession {
    pub tool_key: String,
    pub session_id: Option<String>,
    pub title: String,
    pub preview: String,
    pub last_used_at: String,
    pub can_resume: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryIssue {
    pub tool_key: String,
    pub code: String,
}

#[derive(Default)]
struct Draft {
    id: Option<String>,
    title: Option<String>,
    first_user_text: Option<String>,
    last_used: Option<DateTime<Utc>>,
    auxiliary: bool,
    project_matches: bool,
    project_seen: bool,
    malformed_records: bool,
    line_limit_hit: bool,
}

#[derive(Default)]
struct Budget {
    entries: usize,
    files: usize,
    bytes: u64,
    started: Option<Instant>,
}

impl Budget {
    fn begin(&mut self) {
        self.started.get_or_insert_with(Instant::now);
    }

    fn check_call(&self) -> bool {
        self.started
            .is_some_and(|started| started.elapsed() <= MAX_CALL_DURATION)
    }
}

/// Read a small project-specific session catalog without writing caches or
/// persisting conversation content.
pub fn read_project_history(project_path: &str, sources: &[HistorySource]) -> SessionHistory {
    let Some(project_key) = project_key(project_path) else {
        return SessionHistory {
            sessions: Vec::new(),
            issues: sources
                .iter()
                .map(|source| issue(&source.tool_key, "invalid_project_path"))
                .collect(),
        };
    };
    let mut history = SessionHistory {
        sessions: Vec::new(),
        issues: Vec::new(),
    };
    let mut budget = Budget::default();
    let mut seen = HashSet::new();

    for source in sources {
        budget.begin();
        if !budget.check_call() {
            history.issues.push(issue(&source.tool_key, "time_limit"));
            continue;
        }
        let key = source.tool_key.to_ascii_lowercase();
        if !matches!(
            key.as_str(),
            "claude" | "codex" | "kimi" | "pi" | "opencode" | "aider"
        ) {
            history
                .issues
                .push(issue(&source.tool_key, "unsupported_format"));
            continue;
        }
        if key == "aider" {
            read_aider(source, &project_key, &mut history);
            continue;
        }
        if key == "opencode" {
            read_opencode(source, &project_key, &mut budget, &mut history, &mut seen);
            continue;
        }

        let files = match enumerate(source, &key, &mut budget, &mut history) {
            Some(files) => files,
            None => continue,
        };
        for file in files {
            if !budget.check_call() {
                history.issues.push(issue(&source.tool_key, "time_limit"));
                break;
            }
            let (contents, truncated) = match read_bounded(&file, &mut budget) {
                Ok(contents) => contents,
                Err(code) => {
                    history.issues.push(issue(&source.tool_key, code));
                    continue;
                }
            };
            if truncated {
                history.issues.push(issue(&source.tool_key, "file_limit"));
                if contents.last().is_some_and(|byte| *byte != b'\n') {
                    history.issues.push(issue(&source.tool_key, "line_limit"));
                }
            }
            match parse_file(&key, &file, &contents, &project_key, truncated) {
                Ok(Some(draft)) if draft.project_matches && !draft.auxiliary => {
                    if draft.malformed_records {
                        history
                            .issues
                            .push(issue(&source.tool_key, "malformed_data"));
                    }
                    if draft.line_limit_hit {
                        history.issues.push(issue(&source.tool_key, "line_limit"));
                    }
                    let Some(session) = finish_draft(&key, draft, &file) else {
                        history
                            .issues
                            .push(issue(&source.tool_key, "malformed_data"));
                        continue;
                    };
                    add_session(session, &mut history, &mut seen);
                }
                Ok(Some(draft)) if draft.project_matches && draft.auxiliary => {
                    history
                        .issues
                        .push(issue(&source.tool_key, "auxiliary_filtered"));
                    if draft.malformed_records {
                        history
                            .issues
                            .push(issue(&source.tool_key, "malformed_data"));
                    }
                    if draft.line_limit_hit {
                        history.issues.push(issue(&source.tool_key, "line_limit"));
                    }
                }
                Ok(_) => {}
                Err(code) => history.issues.push(issue(&source.tool_key, code)),
            }
        }
    }

    history
        .sessions
        .sort_by(|a, b| b.last_used_at.cmp(&a.last_used_at));
    if history.sessions.len() > MAX_SESSIONS {
        history.sessions.truncate(MAX_SESSIONS);
        history.issues.push(issue("", "session_limit"));
    }
    history
}

fn project_key(path: &str) -> Option<String> {
    let normalized = crate::path::normalize_native(path)?;
    Some(if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    })
}

fn same_project(candidate: &str, expected: &str) -> bool {
    project_key(candidate).is_some_and(|candidate| candidate == expected)
}

fn issue(tool_key: &str, code: &'static str) -> HistoryIssue {
    HistoryIssue {
        tool_key: tool_key.to_owned(),
        code: code.to_owned(),
    }
}

fn enumerate(
    source: &HistorySource,
    key: &str,
    budget: &mut Budget,
    result: &mut SessionHistory,
) -> Option<Vec<PathBuf>> {
    match no_follow_metadata(&source.root) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => {
            result
                .issues
                .push(issue(&source.tool_key, "unreadable_source"));
            return None;
        }
        Err(_)
            if fs::symlink_metadata(&source.root)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            result
                .issues
                .push(issue(&source.tool_key, "source_unavailable"));
            return None;
        }
        Err(_)
            if fs::symlink_metadata(&source.root)
                .is_ok_and(|metadata| is_link_or_reparse(&metadata)) =>
        {
            result
                .issues
                .push(issue(&source.tool_key, "symlink_skipped"));
            return None;
        }
        Err(_) => {
            result
                .issues
                .push(issue(&source.tool_key, "unreadable_source"));
            return None;
        }
    }
    let mut pending = vec![(source.root.clone(), 0usize)];
    let mut files = Vec::new();
    let started = Instant::now();
    while let Some((dir, depth)) = pending.pop() {
        if depth > MAX_DEPTH
            || budget.entries >= MAX_ENTRIES
            || started.elapsed() > MAX_FILE_DURATION
        {
            result
                .issues
                .push(issue(&source.tool_key, "traversal_limit"));
            return Some(files);
        }
        if !verify_ancestors(&dir) {
            result
                .issues
                .push(issue(&source.tool_key, "symlink_skipped"));
            continue;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => {
                result.issues.push(issue(&source.tool_key, "read_failed"));
                continue;
            }
        };
        for entry in entries {
            budget.entries += 1;
            if budget.entries > MAX_ENTRIES {
                result.issues.push(issue(&source.tool_key, "entry_limit"));
                return Some(files);
            }
            if started.elapsed() > MAX_FILE_DURATION {
                result.issues.push(issue(&source.tool_key, "time_limit"));
                return Some(files);
            }
            let Ok(entry) = entry else {
                result.issues.push(issue(&source.tool_key, "read_failed"));
                continue;
            };
            let path = entry.path();
            let Ok(meta) = no_follow_metadata(&path) else {
                result
                    .issues
                    .push(issue(&source.tool_key, "symlink_skipped"));
                continue;
            };
            if meta.is_dir() {
                pending.push((path, depth + 1));
            } else if meta.is_file() && accepted_file(key, &path) {
                if files.len() >= MAX_FILES {
                    result.issues.push(issue(&source.tool_key, "file_limit"));
                    return Some(files);
                }
                files.push(path);
            }
        }
    }
    files.sort_by_cached_key(|path| {
        (
            Reverse(
                no_follow_metadata(path)
                    .ok()
                    .and_then(|metadata| metadata.modified().ok()),
            ),
            path.clone(),
        )
    });
    Some(files)
}

fn accepted_file(key: &str, path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or_default();
    match key {
        "kimi" => name.eq_ignore_ascii_case("state.json"),
        "claude" | "codex" | "pi" => path.extension().and_then(|v| v.to_str()) == Some("jsonl"),
        _ => false,
    }
}

fn read_bounded(path: &Path, budget: &mut Budget) -> Result<(Vec<u8>, bool), &'static str> {
    budget.files += 1;
    if budget.files > MAX_FILES {
        return Err("file_limit");
    }
    if !budget.check_call() {
        return Err("time_limit");
    }
    if !verify_ancestors(path) {
        return Err("symlink_skipped");
    }
    let meta = no_follow_metadata(path).map_err(|_| "read_failed")?;
    if !meta.is_file() {
        return Err("read_failed");
    }
    if budget.bytes >= MAX_TOTAL_BYTES {
        return Err("byte_limit");
    }
    let available = MAX_TOTAL_BYTES.saturating_sub(budget.bytes);
    let read_limit = MAX_FILE_BYTES.min(available);
    let truncated = meta.len() > read_limit;
    let started = Instant::now();
    let mut contents = Vec::with_capacity(meta.len().min(read_limit) as usize);
    open_readonly_nofollow(path)
        .map_err(|_| "read_failed")?
        .take(read_limit)
        .read_to_end(&mut contents)
        .map_err(|_| "read_failed")?;
    if started.elapsed() > MAX_FILE_DURATION {
        return Err("time_limit");
    }
    budget.bytes = budget.bytes.saturating_add(contents.len() as u64);
    Ok((contents, truncated))
}

fn open_readonly_nofollow(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(
            windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT
                | windows_sys::Win32::Storage::FileSystem::FILE_FLAG_SEQUENTIAL_SCAN,
        );
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if is_link_or_reparse(&metadata) || !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "not a regular file",
        ));
    }
    Ok(file)
}

fn parse_file(
    key: &str,
    path: &Path,
    bytes: &[u8],
    expected: &str,
    truncated: bool,
) -> Result<Option<Draft>, &'static str> {
    if key == "kimi" {
        let root: Value = serde_json::from_slice(bytes).map_err(|_| "malformed_data")?;
        let Some(work_dir) = root.get("workDir").and_then(Value::as_str) else {
            return Ok(None);
        };
        if !same_project(work_dir, expected) {
            return Ok(None);
        }
        let id = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|v| v.to_str())
            .map(str::to_owned);
        let mut draft = Draft {
            id,
            project_matches: true,
            ..Draft::default()
        };
        draft.title = safe_title(root.get("title").and_then(Value::as_str));
        draft.last_used = ["updatedAt", "lastUpdatedAt", "timestamp"]
            .into_iter()
            .find_map(|field| root.get(field).and_then(parse_timestamp));
        return Ok(Some(draft));
    }
    let bytes = if truncated {
        match bytes.iter().rposition(|byte| *byte == b'\n') {
            Some(end) => &bytes[..=end],
            None => return Err("line_limit"),
        }
    } else {
        bytes
    };
    let text = std::str::from_utf8(bytes).map_err(|_| "malformed_data")?;
    let mut draft = Draft::default();
    if key == "claude"
        && (path
            .components()
            .any(|component| component.as_os_str() == "subagents")
            || path
                .file_stem()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("agent-")))
    {
        draft.auxiliary = true;
    }
    let mut records = 0usize;
    let mut malformed = false;
    let mut line_limit_hit = false;
    let mut first = true;
    for line in BufReader::new(text.as_bytes()).split(b'\n') {
        let line = line.map_err(|_| "read_failed")?;
        if line.len() > MAX_LINE_BYTES {
            line_limit_hit = true;
            continue;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        records += 1;
        if records > MAX_RECORDS_PER_FILE {
            return Err("record_limit");
        }
        let line = if first {
            first = false;
            line.strip_prefix(&[0xef, 0xbb, 0xbf])
                .unwrap_or(&line)
                .to_vec()
        } else {
            line
        };
        let Ok(record) = serde_json::from_slice::<Value>(&line) else {
            malformed = true;
            continue;
        };
        process_record(key, &record, &mut draft, expected);
    }
    if malformed && !draft.project_matches {
        return Err("malformed_data");
    }
    draft.malformed_records = malformed;
    draft.line_limit_hit = line_limit_hit;
    if !draft.project_matches {
        return Ok(None);
    }
    if draft.id.is_none() && key == "claude" {
        draft.id = path.file_stem().and_then(|v| v.to_str()).map(str::to_owned);
    }
    Ok(Some(draft))
}

fn process_record(key: &str, record: &Value, draft: &mut Draft, expected: &str) {
    let (kind, payload) = if key == "codex" {
        (
            record
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            record.get("payload").unwrap_or(record),
        )
    } else {
        (
            record
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            record,
        )
    };
    match key {
        "claude" => {
            if let Some(cwd) = record.get("cwd").and_then(Value::as_str) {
                set_project(draft, cwd, expected);
            }
            if draft.id.is_none() {
                draft.id = record
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            if draft.title.is_none() {
                draft.title = safe_title(
                    record
                        .get("customTitle")
                        .or_else(|| record.get("title"))
                        .and_then(Value::as_str),
                );
            }
            if kind == "custom-title" {
                draft.title = safe_title(
                    record
                        .get("customTitle")
                        .or_else(|| record.get("title"))
                        .and_then(Value::as_str),
                );
            }
            if record.get("isSidechain").and_then(Value::as_bool) == Some(true) {
                draft.auxiliary = true;
            }
            if kind == "user" && record.get("isMeta").and_then(Value::as_bool) != Some(true) {
                set_first_user(draft, record.get("message").and_then(|m| m.get("content")));
            }
        }
        "codex" => {
            if kind == "session_meta" {
                if let Some(cwd) = payload.get("cwd").and_then(Value::as_str) {
                    set_project(draft, cwd, expected);
                }
                if draft.id.is_none() {
                    draft.id = payload.get("id").and_then(Value::as_str).map(str::to_owned);
                }
                if draft.title.is_none() {
                    draft.title = safe_title(
                        payload
                            .get("custom_title")
                            .or_else(|| payload.get("title"))
                            .and_then(Value::as_str),
                    );
                }
                draft.auxiliary |= codex_metadata_is_auxiliary(payload);
            } else if (kind == "event_msg"
                && payload.get("type").and_then(Value::as_str) == Some("user_message"))
                || kind == "user_message"
            {
                set_first_user(draft, payload.get("message"));
            } else if kind == "response_item"
                && payload.get("type").and_then(Value::as_str) == Some("message")
                && payload.get("role").and_then(Value::as_str) == Some("user")
            {
                set_first_user(draft, payload.get("content"));
            }
        }
        "pi" => {
            if kind == "session" {
                if let Some(cwd) = record.get("cwd").and_then(Value::as_str) {
                    set_project(draft, cwd, expected);
                }
                if draft.id.is_none() {
                    draft.id = record.get("id").and_then(Value::as_str).map(str::to_owned);
                }
                if draft.title.is_none() {
                    draft.title = safe_title(
                        record
                            .get("customTitle")
                            .or_else(|| record.get("title"))
                            .and_then(Value::as_str),
                    );
                }
            }
            if kind == "message" {
                let message = record.get("message").unwrap_or(record);
                if message.get("role").and_then(Value::as_str) == Some("user") {
                    set_first_user(draft, message.get("content"));
                }
            }
        }
        _ => {}
    }
    if let Some(ts) = record
        .get("timestamp")
        .and_then(parse_timestamp)
        .or_else(|| payload.get("timestamp").and_then(parse_timestamp))
        .or_else(|| {
            record
                .get("message")
                .and_then(|message| message.get("timestamp"))
                .and_then(parse_timestamp)
        })
    {
        draft.last_used = Some(draft.last_used.map_or(ts, |prev| prev.max(ts)));
    }
}

fn codex_metadata_is_auxiliary(payload: &Value) -> bool {
    let thread_source = payload.get("thread_source").and_then(Value::as_str);
    let source = payload.get("source");
    let source_name = source.and_then(Value::as_str).or_else(|| {
        source
            .and_then(|value| value.get("type"))
            .and_then(Value::as_str)
    });
    let subagent = source.and_then(|value| value.get("subagent"));
    let guardian_source = subagent
        .and_then(|value| value.get("other"))
        .and_then(Value::as_str)
        == Some("guardian");
    let guardian_fields = ["guardian", "guardian_review", "is_guardian"];
    let guardian_marker = guardian_fields
        .into_iter()
        .any(|field| has_metadata_marker(payload.get(field)))
        || source.is_some_and(|value| {
            guardian_fields
                .into_iter()
                .any(|field| has_metadata_marker(value.get(field)))
        });
    let parent_marker = ["forked_from", "parent_thread_id"]
        .into_iter()
        .any(|field| has_metadata_marker(payload.get(field)))
        || source.is_some_and(|value| {
            ["forked_from", "parent_thread_id"]
                .into_iter()
                .any(|field| has_metadata_marker(value.get(field)))
        });
    guardian_source
        || guardian_marker
        || parent_marker
        || matches!(
            thread_source,
            Some("child" | "subagent" | "guardian" | "guardian_review" | "exec")
        )
        || matches!(
            source_name,
            Some("child" | "subagent" | "guardian" | "guardian_review" | "exec")
        )
        || subagent.is_some()
}

fn has_metadata_marker(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => !value.trim().is_empty(),
        Some(Value::Object(value)) => !value.is_empty(),
        Some(Value::Array(value)) => !value.is_empty(),
        _ => false,
    }
}

fn set_project(draft: &mut Draft, candidate: &str, expected: &str) {
    if !draft.project_seen {
        draft.project_seen = true;
        draft.project_matches = same_project(candidate, expected);
    }
}

fn set_first_user(draft: &mut Draft, content: Option<&Value>) {
    if draft.first_user_text.is_none() {
        draft.first_user_text = content.and_then(extract_text).and_then(safe_text);
    }
}

fn extract_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let mut result = String::new();
            for item in items {
                if item
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind != "text" && kind != "input_text")
                {
                    continue;
                }
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    if !result.is_empty() {
                        result.push(' ');
                    }
                    result.push_str(text);
                }
            }
            (!result.is_empty()).then_some(result)
        }
        _ => None,
    }
}

fn safe_text(text: String) -> Option<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("agents.md") {
        return None;
    }
    let suspicious = [
        "api_key",
        "api-key",
        "apikey",
        "access_token",
        "refresh_token",
        "password=",
        "passwd=",
        "secret=",
        "private key",
        "-----begin",
        "authorization: bearer",
        "ghp_",
        "sk-",
        "xoxb-",
        "<environment_context",
        "</environment_context",
        "<external_codex_apps",
        "external_codex",
        "external codex",
        "codex_apps",
        "codex apps",
        "<developer>",
        "<developer",
        "</developer>",
        "<system>",
        "<system",
        "agents.md instructions",
        "# agents.md",
        "you are codex",
    ];
    if suspicious.iter().any(|needle| lower.contains(needle)) {
        return None;
    }
    let text: String = text.chars().take(240).collect();
    (!text.trim().is_empty()).then_some(text)
}

fn safe_title(value: Option<&str>) -> Option<String> {
    value
        .and_then(|s| safe_text(s.to_owned()))
        .map(|s| s.chars().take(100).collect())
}

fn finish_draft(key: &str, draft: Draft, file: &Path) -> Option<HistorySession> {
    let title = draft
        .title
        .or_else(|| {
            draft
                .first_user_text
                .as_deref()
                .and_then(|text| safe_title(Some(text)))
        })
        .unwrap_or_else(|| "Untitled session".to_owned());
    let preview = draft.first_user_text.unwrap_or_default();
    let file_time = no_follow_metadata(file)
        .ok()?
        .modified()
        .ok()
        .map(DateTime::<Utc>::from);
    let time = draft.last_used.into_iter().chain(file_time).max()?;
    let session_id = draft
        .id
        .as_deref()
        .and_then(|id| crate::security::validate_session_id(id).ok());
    let can_resume = session_id.is_some();
    Some(HistorySession {
        tool_key: key.to_owned(),
        session_id,
        title,
        preview,
        last_used_at: time.to_rfc3339_opts(SecondsFormat::Secs, true),
        can_resume,
        reason: (!can_resume).then(|| "native_resume_id_unavailable".to_owned()),
    })
}

fn parse_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    value
        .as_str()
        .and_then(|s| {
            DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        })
        .or_else(|| {
            value
                .as_i64()
                .and_then(crate::scanner::try_read_unix_timestamp)
        })
}

fn add_session(
    session: HistorySession,
    history: &mut SessionHistory,
    seen: &mut HashSet<(String, String)>,
) {
    if let Some(id) = session.session_id.as_deref() {
        if !seen.insert((session.tool_key.clone(), id.to_owned())) {
            return;
        }
    }
    history.sessions.push(session);
}

fn read_aider(source: &HistorySource, expected: &str, history: &mut SessionHistory) {
    let marker = if source.root.file_name().and_then(|n| n.to_str()) == Some(".aider.chat.history")
    {
        source.root.clone()
    } else {
        source.root.join(".aider.chat.history")
    };
    let Some(project) = marker
        .parent()
        .and_then(|p| project_key(&p.to_string_lossy()))
    else {
        history
            .issues
            .push(issue(&source.tool_key, "invalid_source"));
        return;
    };
    if project != expected {
        return;
    }
    let Ok(meta) = no_follow_metadata(&marker) else {
        history
            .issues
            .push(issue(&source.tool_key, "source_unavailable"));
        return;
    };
    if !meta.is_file() {
        history
            .issues
            .push(issue(&source.tool_key, "unreadable_source"));
        return;
    }
    let Some(time) = meta.modified().ok().map(DateTime::<Utc>::from) else {
        history.issues.push(issue(&source.tool_key, "read_failed"));
        return;
    };
    history.sessions.push(HistorySession {
        tool_key: "aider".into(),
        session_id: None,
        title: "Aider chat history".into(),
        preview: String::new(),
        last_used_at: time.to_rfc3339_opts(SecondsFormat::Secs, true),
        can_resume: false,
        reason: Some("native_resume_id_unavailable".into()),
    });
}

fn read_opencode(
    source: &HistorySource,
    expected: &str,
    budget: &mut Budget,
    history: &mut SessionHistory,
    seen: &mut HashSet<(String, String)>,
) {
    let database = opencode_database_path(&source.root);
    if !verify_ancestors(&database) {
        history
            .issues
            .push(issue(&source.tool_key, "symlink_skipped"));
        return;
    }
    let Ok(meta) = no_follow_metadata(&database) else {
        history
            .issues
            .push(issue(&source.tool_key, "source_unavailable"));
        return;
    };
    if !meta.is_file() {
        history
            .issues
            .push(issue(&source.tool_key, "unreadable_source"));
        return;
    }
    if meta.len() > MAX_TOTAL_BYTES {
        history
            .issues
            .push(issue(&source.tool_key, "database_limit"));
        return;
    }
    if budget.bytes.saturating_add(meta.len()) > MAX_TOTAL_BYTES {
        history.issues.push(issue(&source.tool_key, "byte_limit"));
        return;
    }
    if !budget.check_call() {
        history.issues.push(issue(&source.tool_key, "time_limit"));
        return;
    }
    let Ok(connection) = Connection::open_with_flags(
        &database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        history
            .issues
            .push(issue(&source.tool_key, "unreadable_source"));
        return;
    };
    budget.bytes = budget.bytes.saturating_add(meta.len());
    let _ = connection.busy_timeout(Duration::from_millis(250));
    let query_started = budget.started.unwrap_or_else(Instant::now);
    if connection
        .progress_handler(
            1_000,
            Some(move || query_started.elapsed() > MAX_CALL_DURATION),
        )
        .is_err()
    {
        history
            .issues
            .push(issue(&source.tool_key, "unreadable_source"));
        return;
    }
    let columns = match table_columns(&connection) {
        Ok(c) => c,
        Err(_) => {
            history
                .issues
                .push(issue(&source.tool_key, "malformed_data"));
            return;
        }
    };
    for required in ["id", "directory", "time_updated", "project_id"] {
        if !columns.contains(required) {
            history
                .issues
                .push(issue(&source.tool_key, "unsupported_format"));
            return;
        }
    }
    let has_parent = columns.contains("parent_id");
    let has_title = columns.contains("title");
    let has_permission = columns.contains("permission");
    let parent_projection = if has_parent {
        "session.parent_id"
    } else {
        "NULL"
    };
    let title_projection = if has_title { "session.title" } else { "NULL" };
    let permission_projection = if has_permission {
        "session.permission"
    } else {
        "NULL"
    };
    let collate = if cfg!(windows) { "COLLATE NOCASE" } else { "" };
    let sql = format!(
        "SELECT session.id, CASE WHEN TRIM(session.directory) <> '' THEN session.directory ELSE project.worktree END, session.time_updated, {parent_projection}, {title_projection}, {permission_projection} FROM session JOIN project ON project.id=session.project_id WHERE (CASE WHEN TRIM(session.directory) <> '' THEN session.directory ELSE project.worktree END) {collate} = ?1 ORDER BY session.time_updated DESC LIMIT 201"
    );
    let mut stmt = match connection.prepare(&sql) {
        Ok(s) => s,
        Err(_) => {
            history
                .issues
                .push(issue(&source.tool_key, "unsupported_format"));
            return;
        }
    };
    let rows = match stmt.query_map([expected], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4).ok().flatten(),
            row.get::<_, Option<String>>(5).ok().flatten(),
        ))
    }) {
        Ok(r) => r,
        Err(_) => {
            history
                .issues
                .push(issue(&source.tool_key, "malformed_data"));
            return;
        }
    };
    let mut count = 0;
    for row in rows {
        count += 1;
        if count > 200 {
            history.issues.push(issue(&source.tool_key, "record_limit"));
            break;
        }
        if !budget.check_call() {
            history.issues.push(issue(&source.tool_key, "time_limit"));
            break;
        }
        let Ok((id, directory, timestamp, parent, title, permission)) = row else {
            history
                .issues
                .push(issue(&source.tool_key, "malformed_data"));
            continue;
        };
        if !same_project(&directory, expected) {
            continue;
        }
        if parent.as_deref().is_some_and(|p| !p.trim().is_empty()) {
            history
                .issues
                .push(issue(&source.tool_key, "auxiliary_filtered"));
            continue;
        }
        if permission
            .as_deref()
            .is_some_and(is_non_interactive_permission)
        {
            history
                .issues
                .push(issue(&source.tool_key, "auxiliary_filtered"));
            continue;
        }
        let Some(time) = timestamp.and_then(crate::scanner::try_read_unix_timestamp) else {
            history
                .issues
                .push(issue(&source.tool_key, "malformed_data"));
            continue;
        };
        let Some(title) =
            safe_title(title.as_deref()).or_else(|| safe_title(Some("OpenCode session")))
        else {
            continue;
        };
        let session_id = crate::security::validate_session_id(&id).ok();
        let can_resume = session_id.is_some();
        add_session(
            HistorySession {
                tool_key: "opencode".into(),
                session_id,
                title,
                preview: String::new(),
                last_used_at: time.to_rfc3339_opts(SecondsFormat::Secs, true),
                can_resume,
                reason: None,
            },
            history,
            seen,
        );
    }
}

fn is_non_interactive_permission(permission: &str) -> bool {
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(permission) else {
        return false;
    };
    ["question", "plan_enter", "plan_exit"]
        .into_iter()
        .all(|name| {
            entries.iter().any(|entry| {
                entry.get("permission").and_then(Value::as_str) == Some(name)
                    && entry.get("action").and_then(Value::as_str) == Some("deny")
            })
        })
}

fn opencode_database_path(root: &Path) -> PathBuf {
    if no_follow_metadata(root).is_ok_and(|metadata| metadata.is_file()) {
        return root.to_path_buf();
    }
    for candidate in [
        root.join("opencode.db"),
        root.join("opencode").join("opencode.db"),
    ] {
        if no_follow_metadata(&candidate).is_ok_and(|metadata| metadata.is_file()) {
            return candidate;
        }
    }
    root.join("opencode.db")
}

fn table_columns(connection: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut statement = connection.prepare("PRAGMA table_info(session)")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    rows.collect()
}

/// lstat-style metadata for every component, rejecting symlink/reparse-point
/// ancestors so traversal never intentionally descends through one.
fn no_follow_metadata(path: &Path) -> std::io::Result<fs::Metadata> {
    verify_ancestors(path).then_some(()).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, "symlink ancestor")
    })?;
    fs::symlink_metadata(path)
}

fn verify_ancestors(path: &Path) -> bool {
    let absolute = match std::path::absolute(path) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(component.as_os_str()),
            Component::CurDir => continue,
            Component::ParentDir => current.push(component.as_os_str()),
            Component::Normal(part) => current.push(part),
        }
        match fs::symlink_metadata(&current) {
            Ok(meta) if !is_link_or_reparse(&meta) => {}
            _ => return false,
        }
    }
    true
}

#[cfg(windows)]
fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_type().is_symlink()
        || metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
}

#[cfg(not(windows))]
fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn source(tool: &str, root: &Path) -> HistorySource {
        HistorySource {
            tool_key: tool.into(),
            root: root.into(),
        }
    }

    #[test]
    fn codex_catalog_keeps_primary_only_and_extracts_user_text() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("sessions");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("primary.jsonl"), format!("{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"abc-1\",\"cwd\":{}}}}}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"Fix the parser\"}}}}\n{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"Later user message\"}}]}}}}\n{{\"type\":\"turn_context\",\"timestamp\":\"2026-01-01T00:00:00Z\"}}", serde_json::to_string(&temp.path().to_string_lossy()).unwrap())).unwrap();
        fs::write(root.join("child.jsonl"), format!("{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"child-2\",\"cwd\":{},\"thread_source\":\"child\"}}}}\n{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"child-2\",\"cwd\":{},\"thread_source\":\"user\"}}}}", serde_json::to_string(&temp.path().to_string_lossy()).unwrap(), serde_json::to_string(&temp.path().to_string_lossy()).unwrap())).unwrap();
        fs::write(root.join("missing-native-id.jsonl"), format!("{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":{}}}}}\n{{\"type\":\"user_message\",\"payload\":{{\"message\":\"No fabricated resume target\"}}}}", serde_json::to_string(&temp.path().to_string_lossy()).unwrap())).unwrap();
        let result =
            read_project_history(&temp.path().to_string_lossy(), &[source("codex", &root)]);
        assert_eq!(result.sessions.len(), 2);
        let primary = result
            .sessions
            .iter()
            .find(|session| session.session_id.as_deref() == Some("abc-1"))
            .unwrap();
        assert_eq!(primary.preview, "Fix the parser");
        let missing_id = result
            .sessions
            .iter()
            .find(|session| session.preview == "No fabricated resume target")
            .unwrap();
        assert_eq!(missing_id.session_id, None);
        assert!(!missing_id.can_resume);
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.code == "auxiliary_filtered"));
        for auxiliary in [
            serde_json::json!({"thread_source":"exec"}),
            serde_json::json!({"thread_source":"guardian_review"}),
            serde_json::json!({"source":"exec"}),
            serde_json::json!({"source":"guardian"}),
            serde_json::json!({"forked_from":"parent"}),
            serde_json::json!({"parent_thread_id":"parent"}),
            serde_json::json!({"guardian":{"review_id":"review"}}),
        ] {
            assert!(codex_metadata_is_auxiliary(&auxiliary));
        }
    }

    #[test]
    fn exact_project_match_and_credential_shaped_user_text_are_enforced() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("projects");
        fs::create_dir_all(&root).unwrap();
        let wanted = temp.path().join("wanted");
        let other = temp.path().join("other");
        let content = format!("{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"id\",\"cwd\":{}}}}}\n{{\"type\":\"user_message\",\"payload\":{{\"message\":\"api_key=secret\"}}}}", serde_json::to_string(&wanted.to_string_lossy()).unwrap());
        fs::write(root.join("wanted.jsonl"), content).unwrap();
        fs::write(
            root.join("other.jsonl"),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"other\",\"cwd\":{}}}}}",
                serde_json::to_string(&other.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();
        let result = read_project_history(&wanted.to_string_lossy(), &[source("codex", &root)]);
        assert_eq!(result.sessions.len(), 1);
        assert_eq!(result.sessions[0].preview, "");
        assert_eq!(result.sessions[0].title, "Untitled session");
    }

    #[cfg(unix)]
    #[test]
    fn traversal_skips_symlinked_session_file() {
        use std::os::unix::fs::symlink;
        let temp = tempdir().unwrap();
        let root = temp.path().join("sessions");
        fs::create_dir_all(&root).unwrap();
        let outside = temp.path().join("outside.jsonl");
        fs::write(&outside, "{}").unwrap();
        symlink(outside, root.join("linked.jsonl")).unwrap();
        let result =
            read_project_history(&temp.path().to_string_lossy(), &[source("codex", &root)]);
        assert!(result.sessions.is_empty());
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.code == "symlink_skipped"));
    }

    #[test]
    fn aider_activity_has_no_fabricated_resume_id() {
        let temp = tempdir().unwrap();
        fs::create_dir_all(temp.path()).unwrap();
        fs::write(temp.path().join(".aider.chat.history"), "ignored body").unwrap();
        let result = read_project_history(
            &temp.path().to_string_lossy(),
            &[source("aider", temp.path())],
        );
        assert_eq!(result.sessions.len(), 1);
        assert_eq!(result.sessions[0].session_id, None);
        assert!(!result.sessions[0].can_resume);
    }

    #[test]
    fn claude_kimi_and_pi_formats_use_native_metadata_and_user_turns() {
        let temp = tempdir().unwrap();
        let project = temp.path().join("work");
        fs::create_dir_all(&project).unwrap();
        let claude = temp.path().join("claude");
        fs::create_dir_all(&claude).unwrap();
        let cwd = serde_json::to_string(&project.to_string_lossy()).unwrap();
        fs::write(
            claude.join("claude.jsonl"),
            format!(
                "{{\"type\":\"user\",\"sessionId\":\"claude-1\",\"cwd\":{cwd},\"timestamp\":\"2026-02-03T04:05:06Z\",\"message\":{{\"content\":\"Review this change\"}}}}\n{{\"type\":\"custom-title\",\"customTitle\":\"Safe title\"}}"
            ),
        )
        .unwrap();
        fs::create_dir_all(claude.join("subagents")).unwrap();
        fs::write(
            claude.join("subagents/agent-child.jsonl"),
            format!("{{\"type\":\"user\",\"sessionId\":\"child\",\"cwd\":{cwd}}}"),
        )
        .unwrap();

        let kimi = temp.path().join("kimi").join("session-k");
        fs::create_dir_all(&kimi).unwrap();
        fs::write(
            kimi.join("state.json"),
            serde_json::json!({
                "workDir": project,
                "updatedAt": "2026-02-03T04:05:06Z",
                "title": "Kimi session"
            })
            .to_string(),
        )
        .unwrap();

        let pi = temp.path().join("pi");
        fs::create_dir_all(&pi).unwrap();
        fs::write(
            pi.join("pi.jsonl"),
            format!(
                "{{\"type\":\"session\",\"id\":\"pi-1\",\"cwd\":{cwd},\"timestamp\":\"2026-02-03T04:05:06Z\"}}\n{{\"type\":\"message\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"Inspect the tests\"}}]}}}}"
            ),
        )
        .unwrap();

        let result = read_project_history(
            &project.to_string_lossy(),
            &[
                source("claude", &claude),
                source("kimi", kimi.parent().unwrap()),
                source("pi", &pi),
            ],
        );
        assert_eq!(result.sessions.len(), 3);
        assert!(result.sessions.iter().any(|session| {
            session.tool_key == "claude"
                && session.session_id.as_deref() == Some("claude-1")
                && session.title == "Safe title"
                && session.preview == "Review this change"
        }));
        assert!(result.sessions.iter().any(|session| {
            session.tool_key == "kimi"
                && session.session_id.as_deref() == Some("session-k")
                && session.title == "Kimi session"
        }));
        assert!(result.sessions.iter().any(|session| {
            session.tool_key == "pi"
                && session.session_id.as_deref() == Some("pi-1")
                && session.preview == "Inspect the tests"
        }));
    }

    #[test]
    fn opencode_reads_bounded_title_metadata_and_filters_child_sessions() {
        let temp = tempdir().unwrap();
        let project_path = temp.path().join("project");
        fs::create_dir_all(&project_path).unwrap();
        let database = temp.path().join("opencode.db");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE project(id TEXT PRIMARY KEY, worktree TEXT, time_updated INTEGER);\
                 CREATE TABLE session(id TEXT PRIMARY KEY, project_id TEXT, directory TEXT, time_updated INTEGER, parent_id TEXT, title TEXT, permission TEXT);\
                 INSERT INTO project VALUES('p', '', 1700000000);\
                 INSERT INTO session VALUES('root-1', 'p', '', 1700000001, NULL, 'OpenCode title', NULL);\
                 INSERT INTO session VALUES('child-1', 'p', '', 1700000002, 'root-1', 'Child', NULL);\
                 INSERT INTO session VALUES('exec-1', 'p', '', 1700000003, NULL, 'One-shot run', '[{\"permission\":\"question\",\"action\":\"deny\"},{\"permission\":\"plan_enter\",\"action\":\"deny\"},{\"permission\":\"plan_exit\",\"action\":\"deny\"}]');",
            )
            .unwrap();
        connection
            .execute(
                "UPDATE project SET worktree=?1 WHERE id='p'",
                [project_path.to_string_lossy().as_ref()],
            )
            .unwrap();
        drop(connection);
        let result = read_project_history(
            &project_path.to_string_lossy(),
            &[source("opencode", &database)],
        );
        assert_eq!(result.sessions.len(), 1);
        assert_eq!(result.sessions[0].session_id.as_deref(), Some("root-1"));
        assert_eq!(result.sessions[0].title, "OpenCode title");
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.code == "auxiliary_filtered"));
    }

    #[test]
    fn large_jsonl_keeps_complete_metadata_prefix_and_reports_limit() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("sessions");
        fs::create_dir_all(&root).unwrap();
        let cwd = serde_json::to_string(&temp.path().to_string_lossy()).unwrap();
        let mut content = format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"large-1\",\"cwd\":{cwd}}}}}\n{{\"type\":\"user_message\",\"payload\":{{\"message\":\"Keep the prefix\"}}}}\n"
        );
        let filler = "x".repeat(60_000);
        for _ in 0..20 {
            content.push_str(
                &serde_json::json!({"type":"event_msg", "payload":{"text":filler}}).to_string(),
            );
            content.push('\n');
        }
        assert!(content.len() > MAX_FILE_BYTES as usize);
        fs::write(root.join("large.jsonl"), content).unwrap();
        let result =
            read_project_history(&temp.path().to_string_lossy(), &[source("codex", &root)]);
        assert_eq!(result.sessions.len(), 1);
        assert_eq!(result.sessions[0].session_id.as_deref(), Some("large-1"));
        assert_eq!(result.sessions[0].preview, "Keep the prefix");
        assert!(result.issues.iter().any(|issue| issue.code == "file_limit"));
    }

    #[test]
    fn oversized_instruction_line_is_skipped_without_losing_real_user_request() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("sessions");
        fs::create_dir_all(&root).unwrap();
        let cwd = serde_json::to_string(&temp.path().to_string_lossy()).unwrap();
        let mut content = format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"parent-1\",\"cwd\":{cwd}}}}}\n"
        );
        content.push_str(&format!(
            "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":{:?}}}}}\n",
            format!("<environment_context>AGENTS.md {}", "x".repeat(MAX_LINE_BYTES + 500))
        ));
        content.push_str("{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Fix the issue\"}]}}\n");
        fs::write(root.join("parent.jsonl"), content).unwrap();
        let result =
            read_project_history(&temp.path().to_string_lossy(), &[source("codex", &root)]);
        assert_eq!(result.sessions.len(), 1);
        assert_eq!(result.sessions[0].session_id.as_deref(), Some("parent-1"));
        assert_eq!(result.sessions[0].preview, "Fix the issue");
        assert!(result.issues.iter().any(|issue| issue.code == "line_limit"));
    }

    #[test]
    fn recent_sessions_are_read_before_old_files_when_byte_budget_is_reached() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("sessions");
        fs::create_dir_all(&root).unwrap();
        let cwd = serde_json::to_string(&temp.path().to_string_lossy()).unwrap();
        let filler = "x".repeat(60_000);
        for index in 0..17 {
            let mut content = format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"session-{index}\",\"cwd\":{cwd}}}}}\n"
            );
            content.push_str("{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"recent fixture\"}}\n");
            for _ in 0..18 {
                content.push_str(
                    &serde_json::json!({"type":"noise", "payload":{"value":filler}}).to_string(),
                );
                content.push('\n');
            }
            fs::write(root.join(format!("{index:02}.jsonl")), content).unwrap();
            std::thread::sleep(Duration::from_millis(15));
        }
        let result =
            read_project_history(&temp.path().to_string_lossy(), &[source("codex", &root)]);
        assert!(result
            .sessions
            .iter()
            .any(|session| session.session_id.as_deref() == Some("session-16")));
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.code == "byte_limit" || issue.code == "file_limit"));
    }

    #[test]
    fn external_wrapper_text_is_not_exposed_as_a_preview() {
        assert_eq!(safe_text("<environment_context>AGENTS.md".to_owned()), None);
        assert_eq!(safe_text("Open external_codex app tool".to_owned()), None);
        assert_eq!(safe_text("<developer>Only metadata".to_owned()), None);
    }
}
