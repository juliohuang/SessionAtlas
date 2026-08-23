//! Bounded, incremental project-content discovery for the secondary FTS5
//! index. The index is intentionally conservative: it reads source/docs only,
//! skips dependency/build/AI-session trees and credential-shaped files, and
//! never persists the raw file body. FTS keeps terms while a small LZ4
//! preview supports result subtitles.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use walkdir::{DirEntry, WalkDir};

/// File metadata used to skip unchanged documents without reading them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentFingerprint {
    pub modified_ns: i64,
    pub file_size: i64,
}

/// One changed text document ready to be written to SQLite.
#[derive(Debug)]
pub struct ContentDocument {
    pub relative_path: String,
    pub fingerprint: ContentFingerprint,
    pub indexed_bytes: usize,
    pub compressed_preview: Vec<u8>,
    pub body: String,
}

/// Bounded discovery result for one project.
#[derive(Debug, Default)]
pub struct ContentCollection {
    pub documents: Vec<ContentDocument>,
    pub retained_paths: HashSet<String>,
    pub reused_files: usize,
    pub skipped_files: usize,
    pub indexed_bytes: usize,
    pub truncated: bool,
}

/// Hard limits that keep rescans and index size predictable.
#[derive(Clone, Copy, Debug)]
pub struct ContentIndexOptions {
    pub max_walk_entries: usize,
    pub max_files_per_project: usize,
    pub max_file_bytes: usize,
    pub max_project_bytes: usize,
    pub max_preview_bytes: usize,
}

impl Default for ContentIndexOptions {
    fn default() -> Self {
        Self {
            max_walk_entries: 50_000,
            max_files_per_project: 2_000,
            max_file_bytes: 256 * 1024,
            max_project_bytes: 8 * 1024 * 1024,
            max_preview_bytes: 32 * 1024,
        }
    }
}

#[derive(Debug)]
struct Candidate {
    path: PathBuf,
    relative_path: String,
    fingerprint: ContentFingerprint,
}

/// Collect changed documents under `root`. Unchanged files are represented in
/// `retained_paths` but never opened. Results are deterministic by relative
/// path so the same project always receives the same bounded slice.
pub fn collect_project_content(
    root: &Path,
    known: &HashMap<String, ContentFingerprint>,
    options: ContentIndexOptions,
) -> io::Result<ContentCollection> {
    let mut result = ContentCollection::default();
    if !root.is_dir() {
        return Ok(result);
    }

    let mut candidates = Vec::new();
    for (walked_entries, entry_result) in WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(should_descend)
        .enumerate()
    {
        if walked_entries >= options.max_walk_entries {
            result.truncated = true;
            break;
        }
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(_) => {
                result.skipped_files += 1;
                continue;
            }
        };
        if !entry.file_type().is_file() || !is_indexable_file(entry.path()) {
            continue;
        }
        let relative = match entry.path().strip_prefix(root) {
            Ok(relative) => relative,
            Err(_) => continue,
        };
        let Some(relative) = relative.to_str() else {
            result.skipped_files += 1;
            continue;
        };
        let relative_path = relative.replace('\\', "/");
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => {
                result.skipped_files += 1;
                continue;
            }
        };
        let file_size = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
        if file_size <= 0 || file_size as usize > options.max_file_bytes {
            result.skipped_files += 1;
            continue;
        }
        candidates.push(Candidate {
            path: entry.into_path(),
            relative_path,
            fingerprint: ContentFingerprint {
                modified_ns: modified_ns(&metadata),
                file_size,
            },
        });
    }
    candidates.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    let mut selected_files = 0usize;
    let mut selected_bytes = 0usize;
    for candidate in candidates {
        let bytes = candidate.fingerprint.file_size as usize;
        if selected_files >= options.max_files_per_project
            || selected_bytes.saturating_add(bytes) > options.max_project_bytes
        {
            result.truncated = true;
            result.skipped_files += 1;
            continue;
        }
        selected_files += 1;
        selected_bytes += bytes;

        if known.get(&candidate.relative_path) == Some(&candidate.fingerprint) {
            result.retained_paths.insert(candidate.relative_path);
            result.reused_files += 1;
            continue;
        }

        let body = match read_utf8_document(&candidate.path, options.max_file_bytes) {
            Ok(Some(body)) => body,
            Ok(None) | Err(_) => {
                result.skipped_files += 1;
                continue;
            }
        };
        let body = redact_sensitive_lines(&body);
        let preview = utf8_prefix(&body, options.max_preview_bytes);
        let compressed_preview = lz4_flex::compress_prepend_size(preview.as_bytes());
        result.indexed_bytes += body.len();
        result
            .retained_paths
            .insert(candidate.relative_path.clone());
        result.documents.push(ContentDocument {
            relative_path: candidate.relative_path,
            fingerprint: candidate.fingerprint,
            indexed_bytes: body.len(),
            compressed_preview,
            body,
        });
    }
    Ok(result)
}

/// Decompress a stored preview and build a compact one-line subtitle around
/// the first query term. Corrupt previews fail closed.
pub fn content_match_snippet(compressed: &[u8], query: &str, max_chars: usize) -> Option<String> {
    let bytes = lz4_flex::decompress_size_prepended(compressed).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flattened.is_empty() {
        return None;
    }
    let lower = flattened.to_ascii_lowercase();
    let hit = query_terms(query)
        .into_iter()
        .filter_map(|term| lower.find(&term.to_ascii_lowercase()))
        .min()
        .unwrap_or(0);
    Some(char_window(&flattened, hit, max_chars))
}

fn should_descend(entry: &DirEntry) -> bool {
    entry.depth() == 0
        || !entry.file_type().is_dir()
        || !is_ignored_directory(&entry.file_name().to_string_lossy())
}

fn is_ignored_directory(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        ".git"
            | ".sessionatlas"
            | ".claude"
            | ".codex"
            | ".kimi"
            | ".kimi-code"
            | ".opencode"
            | ".aider"
            | ".pi"
            | "node_modules"
            | "vendor"
            | "target"
            | "dist"
            | "build"
            | "out"
            | "bin"
            | "obj"
            | "coverage"
            | ".next"
            | ".nuxt"
            | ".cache"
            | ".gradle"
            | ".m2"
            | ".venv"
            | "venv"
            | "__pycache__"
            | ".idea"
            | ".vscode"
    )
}

fn is_indexable_file(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lower = file_name.to_ascii_lowercase();
    if is_sensitive_or_noisy_file(&lower) {
        return false;
    }
    if matches!(
        lower.as_str(),
        "dockerfile"
            | "makefile"
            | "cmakelists.txt"
            | "justfile"
            | ".gitignore"
            | ".dockerignore"
            | ".editorconfig"
    ) || lower.starts_with("readme")
        || lower.starts_with("license")
    {
        return true;
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    matches!(
        extension.as_str(),
        "rs" | "toml"
            | "md"
            | "txt"
            | "json"
            | "jsonc"
            | "yaml"
            | "yml"
            | "js"
            | "mjs"
            | "cjs"
            | "ts"
            | "tsx"
            | "jsx"
            | "py"
            | "go"
            | "java"
            | "kt"
            | "kts"
            | "c"
            | "cc"
            | "cpp"
            | "h"
            | "hpp"
            | "cs"
            | "fs"
            | "fsx"
            | "php"
            | "rb"
            | "swift"
            | "scala"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "ps1"
            | "sql"
            | "html"
            | "htm"
            | "css"
            | "scss"
            | "sass"
            | "less"
            | "vue"
            | "svelte"
            | "xml"
            | "proto"
            | "graphql"
            | "gql"
            | "gradle"
            | "properties"
            | "ini"
            | "conf"
            | "cfg"
    )
}

fn is_sensitive_or_noisy_file(lower: &str) -> bool {
    lower == ".npmrc"
        || lower == ".pypirc"
        || lower == ".netrc"
        || lower == "cargo.lock"
        || lower == "package-lock.json"
        || lower == "pnpm-lock.yaml"
        || lower == "yarn.lock"
        || lower == "poetry.lock"
        || lower == "uv.lock"
        || lower == "composer.lock"
        || lower == "gemfile.lock"
        || lower == "go.sum"
        || lower.starts_with(".env")
        || lower.starts_with("credentials.")
        || lower.starts_with("secrets.")
        || lower == "id_rsa"
        || lower == "id_ed25519"
        || [
            ".pem",
            ".key",
            ".p12",
            ".pfx",
            ".jks",
            ".keystore",
            ".map",
            ".min.js",
        ]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

fn modified_ns(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn read_utf8_document(path: &Path, max_bytes: usize) -> io::Result<Option<String>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(max_bytes as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes || bytes.contains(&0) {
        return Ok(None);
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        bytes.drain(..3);
    }
    Ok(String::from_utf8(bytes).ok())
}

fn redact_sensitive_lines(text: &str) -> String {
    let mut redacted = Vec::new();
    let mut inside_private_key = false;
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("begin private key") {
            inside_private_key = true;
            redacted.push("[redacted-sensitive-block]");
            continue;
        }
        if inside_private_key {
            if lower.contains("end private key") {
                inside_private_key = false;
            }
            continue;
        }
        let sensitive_marker = [
            "password",
            "passwd",
            "api_key",
            "apikey",
            "access_token",
            "refresh_token",
            "client_secret",
            "private_key",
            "authorization:",
            "bearer ",
        ]
        .iter()
        .any(|marker| lower.contains(marker));
        redacted.push(
            if sensitive_marker
                && (line.contains('=') || line.contains(':') || lower.contains("bearer "))
            {
                "[redacted-sensitive-line]"
            } else {
                line
            },
        );
    }
    redacted.join("\n")
}

fn utf8_prefix(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn query_terms(query: &str) -> Vec<String> {
    query
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|term| !term.is_empty())
        .map(str::to_string)
        .collect()
}

fn char_window(text: &str, hit_byte: usize, max_chars: usize) -> String {
    let positions: Vec<usize> = text.char_indices().map(|(index, _)| index).collect();
    if positions.len() <= max_chars {
        return text.to_string();
    }
    let hit_char = positions.partition_point(|position| *position < hit_byte);
    let start_char = hit_char.saturating_sub(max_chars / 3);
    let end_char = (start_char + max_chars).min(positions.len());
    let start = positions[start_char];
    let end = positions.get(end_char).copied().unwrap_or(text.len());
    format!(
        "{}{}{}",
        if start_char > 0 { "…" } else { "" },
        &text[start..end],
        if end_char < positions.len() {
            "…"
        } else {
            ""
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_collection_skips_dependencies_secrets_binary_and_large_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::create_dir_all(root.path().join("node_modules/pkg")).unwrap();
        std::fs::write(
            root.path().join("src/main.rs"),
            "fn searchable_symbol() {}\n",
        )
        .unwrap();
        std::fs::write(root.path().join("README.md"), "project architecture\n").unwrap();
        std::fs::write(root.path().join(".env"), "PASSWORD=never-index\n").unwrap();
        std::fs::write(
            root.path().join("node_modules/pkg/index.js"),
            "dependency_noise\n",
        )
        .unwrap();
        std::fs::write(root.path().join("binary.txt"), b"hello\0world").unwrap();
        std::fs::write(root.path().join("large.md"), "x".repeat(300)).unwrap();
        let options = ContentIndexOptions {
            max_walk_entries: 100,
            max_files_per_project: 10,
            max_file_bytes: 128,
            max_project_bytes: 1024,
            max_preview_bytes: 64,
        };

        let collection = collect_project_content(root.path(), &HashMap::new(), options).unwrap();
        let paths: Vec<_> = collection
            .documents
            .iter()
            .map(|document| document.relative_path.as_str())
            .collect();

        assert_eq!(paths, ["README.md", "src/main.rs"]);
        assert!(collection
            .documents
            .iter()
            .all(|document| !document.body.contains("never-index")));
        assert!(collection
            .documents
            .iter()
            .all(|document| !document.body.contains("dependency_noise")));
    }

    #[test]
    fn unchanged_files_are_reused_without_a_document_body() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("main.rs");
        std::fs::write(&path, "fn stable() {}\n").unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        let fingerprint = ContentFingerprint {
            modified_ns: modified_ns(&metadata),
            file_size: metadata.len() as i64,
        };
        let known = HashMap::from([("main.rs".to_string(), fingerprint)]);

        let collection =
            collect_project_content(root.path(), &known, ContentIndexOptions::default()).unwrap();

        assert!(collection.documents.is_empty());
        assert_eq!(collection.reused_files, 1);
        assert!(collection.retained_paths.contains("main.rs"));
    }

    #[test]
    fn deterministic_file_and_project_budgets_mark_the_index_truncated() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..6 {
            std::fs::write(
                root.path().join(format!("file-{index}.rs")),
                format!("fn symbol_{index}() {{}}\n"),
            )
            .unwrap();
        }
        let options = ContentIndexOptions {
            max_walk_entries: 100,
            max_files_per_project: 3,
            max_file_bytes: 128,
            max_project_bytes: 1024,
            max_preview_bytes: 64,
        };

        let collection = collect_project_content(root.path(), &HashMap::new(), options).unwrap();

        assert_eq!(collection.documents.len(), 3);
        assert!(collection.truncated);
        assert_eq!(
            collection
                .documents
                .iter()
                .map(|document| document.relative_path.as_str())
                .collect::<Vec<_>>(),
            ["file-0.rs", "file-1.rs", "file-2.rs"]
        );
    }

    #[test]
    fn directory_walk_budget_stops_discovery_deterministically() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..8 {
            std::fs::write(
                root.path().join(format!("file-{index}.rs")),
                format!("fn walked_{index}() {{}}\n"),
            )
            .unwrap();
        }
        let options = ContentIndexOptions {
            max_walk_entries: 4,
            max_files_per_project: 20,
            max_file_bytes: 128,
            max_project_bytes: 1024,
            max_preview_bytes: 64,
        };

        let collection = collect_project_content(root.path(), &HashMap::new(), options).unwrap();

        // The root itself consumes one walk entry, then the first three sorted files.
        assert_eq!(collection.documents.len(), 3);
        assert!(collection.truncated);
        assert_eq!(
            collection
                .documents
                .iter()
                .map(|document| document.relative_path.as_str())
                .collect::<Vec<_>>(),
            ["file-0.rs", "file-1.rs", "file-2.rs"]
        );
    }

    #[test]
    fn preview_is_compressed_redacted_and_produces_a_bounded_subtitle() {
        let body = "alpha architecture beta
PASSWORD=do-not-store
-----BEGIN PRIVATE KEY-----
private-key-material-must-not-survive
-----END PRIVATE KEY-----
gamma searchable_component delta";
        let redacted = redact_sensitive_lines(body);
        let compressed = lz4_flex::compress_prepend_size(redacted.as_bytes());

        let snippet = content_match_snippet(&compressed, "searchable", 32).unwrap();

        assert!(snippet.contains("searchable_component"));
        assert!(snippet.chars().count() <= 34);
        assert!(!redacted.contains("do-not-store"));
        assert!(!redacted.contains("private-key-material"));
        assert!(redacted.contains("[redacted-sensitive-block]"));
    }
}
