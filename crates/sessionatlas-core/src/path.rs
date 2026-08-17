//! Path normalization and semantics (Windows/POSIX flavors, root safety,
//! display names, parent/child relations).
//!
//! Explicit flavors are lexical so both platform contracts can be exercised on every host; the
//! native flavor applies the current platform's rules without touching the
//! filesystem or the current working directory.

use std::cmp::Ordering;

/// Explicit path flavor. Both variants are always available so that each
/// platform contract can be tested on any host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathFlavor {
    Windows,
    Unix,
}

impl PathFlavor {
    /// The flavor of the host this process runs on.
    pub fn native() -> Self {
        if cfg!(windows) {
            PathFlavor::Windows
        } else {
            PathFlavor::Unix
        }
    }

    /// Whether path equality/ordering is case-sensitive for this flavor.
    pub fn case_sensitive(self) -> bool {
        matches!(self, PathFlavor::Unix)
    }

    /// Directory separator used by this flavor.
    pub fn separator(self) -> char {
        match self {
            PathFlavor::Windows => '\\',
            PathFlavor::Unix => '/',
        }
    }
}

/// Rejects empty/all-whitespace input and any string containing a null byte.
fn valid_input(candidate: &str) -> bool {
    !candidate.trim().is_empty() && !candidate.contains('\0')
}

/// Lexical normalization for an explicit flavor. Returns `None` when the
/// candidate is not a valid absolute path for that flavor.
pub fn normalize(flavor: PathFlavor, candidate: &str) -> Option<String> {
    if !valid_input(candidate) {
        return None;
    }
    match flavor {
        PathFlavor::Windows => normalize_windows(candidate),
        PathFlavor::Unix => normalize_unix(candidate),
    }
}

/// Native-flavor normalization: rejects relative/blank input, resolves the
/// platform-rooted path with the standard library, then applies lexical rules.
/// The target does not need to exist.
pub fn normalize_native(candidate: &str) -> Option<String> {
    if !valid_input(candidate) {
        return None;
    }
    if cfg!(windows) && is_drive_relative(candidate) {
        return None;
    }
    let candidate_path = std::path::Path::new(candidate);
    if !candidate_path.has_root() {
        return None;
    }
    let absolute = std::path::absolute(candidate_path).ok()?;
    normalize(PathFlavor::native(), absolute.to_str()?)
}

/// Windows drive-relative form (`C:foo`) — rooted-looking but not absolute.
fn is_drive_relative(candidate: &str) -> bool {
    let bytes = candidate.as_bytes();
    bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes.len() < 3 || !matches!(bytes[2], b'\\' | b'/'))
}

fn normalize_unix(candidate: &str) -> Option<String> {
    if !candidate.starts_with('/') {
        return None;
    }
    let segments = reduce_segments(candidate.split('/'));
    if segments.is_empty() {
        Some("/".to_string())
    } else {
        let joined = segments.join("/");
        Some(format!("/{joined}"))
    }
}

fn normalize_windows(candidate: &str) -> Option<String> {
    let value = candidate.replace('/', "\\");
    let (root, remainder): (String, Vec<&str>) = {
        let bytes = value.as_bytes();
        if value.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\'
        {
            let drive = (bytes[0] as char).to_ascii_uppercase();
            let root = format!("{drive}:\\");
            let rest = value[3..].split('\\').filter(|s| !s.is_empty()).collect();
            (root, rest)
        } else {
            let stripped = value.strip_prefix("\\\\")?;
            let parts: Vec<&str> = stripped.split('\\').filter(|s| !s.is_empty()).collect();
            if parts.len() < 2
                || parts[0] == "."
                || parts[0] == ".."
                || parts[1] == "."
                || parts[1] == ".."
            {
                return None;
            }
            let server = parts[0];
            let share = parts[1];
            let root = format!("\\\\{server}\\{share}");
            (root, parts[2..].to_vec())
        }
    };

    let segments = reduce_segments(remainder);
    let normalized = if segments.is_empty() {
        root
    } else if root.ends_with('\\') {
        let joined = segments.join("\\");
        format!("{root}{joined}")
    } else {
        let joined = segments.join("\\");
        format!("{root}\\{joined}")
    };
    Some(normalized)
}

/// Drops empty and `.` segments; pops one segment per `..` (ignored above the
/// root).
fn reduce_segments<'a, I>(source: I) -> Vec<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut segments = Vec::new();
    for segment in source {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            segments.pop();
            continue;
        }
        segments.push(segment);
    }
    segments
}

fn windows_case_key(value: &str) -> String {
    value.chars().flat_map(char::to_uppercase).collect()
}

/// Case rule for path comparison on a flavor. Windows uses invariant Unicode
/// case folding;
/// Unix compares byte-exactly.
pub fn paths_equal(flavor: PathFlavor, a: &str, b: &str) -> bool {
    match flavor {
        PathFlavor::Windows => windows_case_key(a) == windows_case_key(b),
        PathFlavor::Unix => a == b,
    }
}

/// Total ordering consistent with [`paths_equal`].
pub fn compare(flavor: PathFlavor, a: &str, b: &str) -> Ordering {
    match flavor {
        PathFlavor::Windows => windows_case_key(a).cmp(&windows_case_key(b)),
        PathFlavor::Unix => a.cmp(b),
    }
}

/// Display name for a path under an explicit flavor: a drive/UNC root (and the
/// POSIX root) renders as itself, any other path renders as its last segment.
/// Returns `None` when the path is not valid for the flavor.
pub fn display_name(flavor: PathFlavor, path: &str) -> Option<String> {
    let normalized = normalize(flavor, path)?;
    Some(match flavor {
        PathFlavor::Unix => {
            if normalized == "/" {
                "/".to_string()
            } else {
                normalized
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_string()
            }
        }
        PathFlavor::Windows => {
            let bytes = normalized.as_bytes();
            let is_drive_root = normalized.len() == 3 && bytes[1] == b':' && bytes[2] == b'\\';
            let is_share_root =
                normalized.starts_with("\\\\") && normalized.matches('\\').count() == 3;
            if is_drive_root || is_share_root {
                normalized
            } else {
                normalized
                    .rsplit('\\')
                    .next()
                    .unwrap_or_default()
                    .to_string()
            }
        }
    })
}

/// Native-flavor [`display_name`].
pub fn display_name_native(path: &str) -> Option<String> {
    display_name(PathFlavor::native(), path)
}

/// Whether `candidate` equals `parent` (case rule) or lies strictly under it,
/// bounded by a separator so sibling prefixes like `C:\repo2` are not treated
/// as children of `C:\repo`.
pub fn is_same_or_child(flavor: PathFlavor, candidate: &str, parent: &str) -> bool {
    if paths_equal(flavor, candidate, parent) {
        return true;
    }
    let separator = flavor.separator();
    let prefix = if parent.ends_with(separator) {
        parent.to_string()
    } else {
        let mut prefix = String::with_capacity(parent.len() + 1);
        prefix.push_str(parent);
        prefix.push(separator);
        prefix
    };
    match flavor {
        PathFlavor::Windows => windows_case_key(candidate).starts_with(&windows_case_key(&prefix)),
        PathFlavor::Unix => candidate.starts_with(&prefix),
    }
}

/// Native-flavor [`is_same_or_child`].
pub fn is_same_or_child_native(candidate: &str, parent: &str) -> bool {
    is_same_or_child(PathFlavor::native(), candidate, parent)
}
