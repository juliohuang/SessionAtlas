//! Small, versioned cache for successful per-file scanner results.
//!
//! The cache is an optimization only: a missing, malformed, or unwritable
//! cache always falls back to the scanner's normal source-of-truth path. Empty
//! or failed parses are deliberately never stored as successful entries.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::base::ScannedProject;
use super::base::{no_follow_metadata, open_regular_file};

const CACHE_VERSION: u32 = 1;
const CACHE_FILE: &str = "scanner-cache-v1.json";
#[cfg(test)]
const MAX_CACHE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedProject {
    path: String,
    last_accessed_at: String,
    session_id: Option<String>,
    git_branch: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheEntry {
    parser_version: u32,
    size_bytes: u64,
    modified_ns: String,
    projects: Vec<CachedProject>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct CacheDocument {
    version: u32,
    entries: BTreeMap<String, CacheEntry>,
}

#[derive(Debug)]
pub(crate) struct FileCache {
    path: PathBuf,
    parser_version: u32,
    document: CacheDocument,
    dirty: bool,
    max_bytes: u64,
}

impl FileCache {
    #[cfg(test)]
    pub(crate) fn load(home: &Path, parser_version: u32) -> Self {
        Self::load_with_limit(home, parser_version, MAX_CACHE_BYTES)
    }

    pub(crate) fn load_with_limit(home: &Path, parser_version: u32, max_bytes: u64) -> Self {
        let path = home.join(".sessionatlas").join(CACHE_FILE);
        let document = no_follow_metadata(&path)
            .ok()
            .filter(|metadata| metadata.is_file() && metadata.len() <= max_bytes)
            .and_then(|_| read_bounded_bytes(&path, max_bytes))
            .and_then(|bytes| serde_json::from_slice::<CacheDocument>(&bytes).ok())
            .filter(|document| document.version == CACHE_VERSION)
            .unwrap_or_else(|| CacheDocument {
                version: CACHE_VERSION,
                entries: BTreeMap::new(),
            });
        Self {
            path,
            parser_version,
            document,
            dirty: false,
            max_bytes,
        }
    }

    pub(crate) fn get(&self, tool_key: &str, path: &Path) -> Option<Vec<ScannedProject>> {
        let fingerprint = file_fingerprint(path)?;
        let entry = self.document.entries.get(&cache_key(tool_key, path))?;
        if entry.parser_version != self.parser_version
            || entry.size_bytes != fingerprint.0
            || entry.modified_ns != fingerprint.1
        {
            return None;
        }
        entry
            .projects
            .iter()
            .map(|project| {
                Ok(ScannedProject {
                    path: project.path.clone(),
                    last_accessed_at: project.last_accessed_at.parse().map_err(|_| ())?,
                    session_id: project.session_id.clone(),
                    git_branch: project.git_branch.clone(),
                })
            })
            .collect::<Result<Vec<_>, ()>>()
            .ok()
    }

    pub(crate) fn record(&mut self, tool_key: &str, path: &Path, projects: &[ScannedProject]) {
        if projects.is_empty() {
            return;
        }
        let Some((size_bytes, modified_ns)) = file_fingerprint(path) else {
            return;
        };
        self.document.entries.insert(
            cache_key(tool_key, path),
            CacheEntry {
                parser_version: self.parser_version,
                size_bytes,
                modified_ns,
                projects: projects
                    .iter()
                    .map(|project| CachedProject {
                        path: project.path.clone(),
                        last_accessed_at: project.last_accessed_at.to_rfc3339(),
                        session_id: project.session_id.clone(),
                        git_branch: project.git_branch.clone(),
                    })
                    .collect(),
            },
        );
        self.dirty = true;
    }

    /// Removes cache entries for files no longer present in this scan. Entries
    /// from other tools remain untouched because all scanners share one file.
    /// A file that was seen but failed to parse is retained, but its changed
    /// fingerprint prevents it from being treated as a cache hit next time.
    pub(crate) fn retain_paths(&mut self, tool_key: &str, paths: &[PathBuf]) {
        let prefix = format!("{tool_key}:");
        let seen: HashSet<String> = paths.iter().map(|path| cache_key(tool_key, path)).collect();
        let before = self.document.entries.len();
        self.document
            .entries
            .retain(|key, _| !key.starts_with(&prefix) || seen.contains(key));
        if self.document.entries.len() != before {
            self.dirty = true;
        }
    }

    pub(crate) fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(parent) = self.path.parent() else {
            return;
        };
        if fs::create_dir_all(parent).is_err() {
            return;
        }
        let temporary = parent.join(format!(
            ".{CACHE_FILE}.{}.{}.tmp",
            std::process::id(),
            Uuid::new_v4().simple()
        ));
        let write_result = (|| -> std::io::Result<()> {
            {
                let file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)?;
                let mut limited = LimitedWriter::new(file, self.max_bytes);
                if serde_json::to_writer(&mut limited, &self.document).is_err() {
                    let _ = fs::remove_file(&temporary);
                    self.dirty = false;
                    return Ok(());
                }
                let file = limited.into_inner();
                file.sync_all()?;
            }
            crate::config::atomic_replace_file(&temporary, &self.path)
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        self.dirty = false;
    }
}

fn read_bounded_bytes(path: &Path, max_bytes: u64) -> Option<Vec<u8>> {
    let file = open_regular_file(path).ok()?;
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= max_bytes).then_some(bytes)
}

struct LimitedWriter<W> {
    inner: W,
    written: u64,
    max: u64,
}
impl<W: Write> LimitedWriter<W> {
    fn new(inner: W, max: u64) -> Self {
        Self {
            inner,
            written: 0,
            max,
        }
    }
    fn into_inner(self) -> W {
        self.inner
    }
}
impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.written.saturating_add(bytes.len() as u64) > self.max {
            return Err(std::io::Error::other("cache size limit"));
        }
        let written = self.inner.write(bytes)?;
        self.written = self.written.saturating_add(written as u64);
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn cache_key(tool_key: &str, path: &Path) -> String {
    // Scanner paths have already been validated as no-follow regular files;
    // avoid a second canonicalization that could follow a raced symlink.
    let normalized = path.to_path_buf();
    #[cfg(windows)]
    let text = normalized
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    #[cfg(not(windows))]
    let text = normalized.to_string_lossy().into_owned();
    format!("{tool_key}:{text}")
}

fn file_fingerprint(path: &Path) -> Option<(u64, String)> {
    let metadata = no_follow_metadata(path).ok()?;
    let modified_ns = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos()
        .to_string();
    Some((metadata.len(), modified_ns))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn project(path: &str) -> ScannedProject {
        ScannedProject {
            path: path.to_string(),
            last_accessed_at: chrono::Utc::now(),
            session_id: Some("session".to_string()),
            git_branch: None,
        }
    }

    #[test]
    fn cache_round_trip_and_fingerprint_invalidation() {
        let home = tempfile::tempdir().unwrap();
        let source = home.path().join("session.jsonl");
        fs::write(&source, b"one").unwrap();
        let mut cache = FileCache::load(home.path(), 7);
        cache.record("codex", &source, &[project("C:\\repo")]);
        cache.save();

        let cache = FileCache::load(home.path(), 7);
        assert_eq!(cache.get("codex", &source).unwrap().len(), 1);
        assert!(FileCache::load(home.path(), 8)
            .get("codex", &source)
            .is_none());
        std::thread::sleep(Duration::from_millis(2));
        fs::write(&source, b"changed").unwrap();
        assert!(cache.get("codex", &source).is_none());
    }

    #[test]
    fn corrupt_cache_falls_back_to_empty() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".sessionatlas/scanner-cache-v1.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"broken").unwrap();
        let cache = FileCache::load(home.path(), 1);
        assert!(cache.document.entries.is_empty());
    }

    #[test]
    fn scanner_cache_load_with_limit_reads_at_most_max_plus_one() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".sessionatlas/scanner-cache-v1.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"0123456789").unwrap();

        let cache = FileCache::load_with_limit(home.path(), 1, 4);
        assert!(cache.document.entries.is_empty());
        assert!(read_bounded_bytes(&path, 4).is_none());
    }

    #[test]
    fn scanner_cache_limited_writer_allows_exact_boundary_and_rejects_one_more() {
        use std::io::Write as _;

        let mut exact = LimitedWriter::new(Vec::new(), 3);
        exact.write_all(b"abc").unwrap();
        assert_eq!(exact.into_inner(), b"abc");

        let mut over = LimitedWriter::new(Vec::new(), 3);
        over.write_all(b"abc").unwrap();
        assert!(over.write_all(b"d").is_err());
    }

    #[test]
    fn scanner_cache_oversized_save_keeps_existing_cache_file() {
        let home = tempfile::tempdir().unwrap();
        let cache_path = home.path().join(".sessionatlas/scanner-cache-v1.json");
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, b"existing-cache").unwrap();
        let source = home.path().join("session.jsonl");
        fs::write(&source, b"one").unwrap();

        let mut cache = FileCache::load_with_limit(home.path(), 1, 1);
        cache.record("codex", &source, &[project("a-very-long-project-path")]);
        cache.save();

        assert_eq!(fs::read(&cache_path).unwrap(), b"existing-cache");
    }

    #[test]
    fn second_save_replaces_the_first_cache_atomically() {
        let home = tempfile::tempdir().unwrap();
        let source = home.path().join("session.jsonl");
        fs::write(&source, b"one").unwrap();
        let mut cache = FileCache::load(home.path(), 1);
        cache.record("codex", &source, &[project("first")]);
        cache.save();

        std::thread::sleep(Duration::from_millis(2));
        fs::write(&source, b"two").unwrap();
        let mut cache = FileCache::load(home.path(), 1);
        cache.record("codex", &source, &[project("second")]);
        cache.save();

        let loaded = FileCache::load(home.path(), 1);
        assert_eq!(loaded.get("codex", &source).unwrap()[0].path, "second");
    }

    #[test]
    fn deleted_files_are_pruned_but_other_tool_entries_remain() {
        let home = tempfile::tempdir().unwrap();
        let first = home.path().join("first.jsonl");
        let deleted = home.path().join("deleted.jsonl");
        fs::write(&first, b"one").unwrap();
        fs::write(&deleted, b"two").unwrap();
        let mut cache = FileCache::load(home.path(), 1);
        cache.record("codex", &first, &[project("first")]);
        cache.record("codex", &deleted, &[project("deleted")]);
        cache.record("claude", &deleted, &[project("claude")]);
        cache.save();

        let mut cache = FileCache::load(home.path(), 1);
        cache.retain_paths("codex", std::slice::from_ref(&first));
        cache.save();
        let loaded = FileCache::load(home.path(), 1);
        assert!(loaded.get("codex", &deleted).is_none());
        assert_eq!(loaded.get("codex", &first).unwrap()[0].path, "first");
        assert_eq!(loaded.get("claude", &deleted).unwrap()[0].path, "claude");
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_cache_keys_keep_backslashes_as_literal_path_bytes() {
        let left = Path::new("/tmp/cache-key-a\\b");
        let right = Path::new("/tmp/cache-key-a/b");
        assert_ne!(cache_key("codex", left), cache_key("codex", right));
    }
}
