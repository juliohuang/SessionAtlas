//! Content policy and lifecycle live beside their data, under the same writer
//! lock. No second config file can get out of sync with the search gate.

use super::*;
use serde::{Deserialize, Serialize};

const ENABLED_KEY: &str = "content_index_enabled";
const PHASE_KEY: &str = "content_index_phase";

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContentIndexPhase {
    Ready,
    CleanupPending,
    RebuildPending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentIndexAction {
    Enable,
    Disable,
    Clear,
    Rebuild,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentIndexStatus {
    pub index_exists: bool,
    pub enabled: bool,
    pub phase: ContentIndexPhase,
    pub searchable: bool,
    pub indexed_files: i64,
    pub indexed_bytes: i64,
}

/// Read-only, including when the index is missing. Does not create an empty
/// index that would suppress the desktop's first-run scan.
pub fn content_index_status_at(path: impl AsRef<Path>) -> Result<ContentIndexStatus> {
    let path = path.as_ref();
    if !path.try_exists()? {
        return Ok(ContentIndexStatus {
            index_exists: false,
            enabled: true,
            phase: ContentIndexPhase::Ready,
            searchable: false,
            indexed_files: 0,
            indexed_bytes: 0,
        });
    }
    IndexReader::open_existing(path)?.content_index_status()
}

/// Explicit mutations require an existing project index. All actions serialize
/// with scans; callers must query status again on errors because cleanup may
/// have committed its fail-closed marker before physical cleanup failed.
pub fn change_content_index_at(
    path: impl AsRef<Path>,
    action: ContentIndexAction,
) -> Result<ContentIndexStatus> {
    let path = path.as_ref();
    if !path.try_exists()? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No project index. Run sessionatlas scan first.",
        )
        .into());
    }
    let mut store = SqliteStore::new(path)?;
    store.change_content_index(action)?;
    store.content_index_status()
}

impl IndexReader {
    pub fn content_index_status(&self) -> Result<ContentIndexStatus> {
        read_status(&self.connection)
    }
}

impl SqliteStore {
    pub fn content_index_status(&self) -> Result<ContentIndexStatus> {
        read_status(&self.connection)
    }

    pub fn change_content_index(&mut self, action: ContentIndexAction) -> Result<()> {
        match action {
            ContentIndexAction::Disable => self.clear_content_index(Some(false)),
            ContentIndexAction::Clear => self.clear_content_index(None),
            ContentIndexAction::Enable => {
                // Enabling alone never resurrects partial or old content.
                if read_phase(&self.connection)? != ContentIndexPhase::Ready {
                    self.clear_content_index(None)?;
                }
                set_meta(&self.connection, ENABLED_KEY, "true")
            }
            ContentIndexAction::Rebuild => {
                if !read_enabled(&self.connection)? {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Content indexing is disabled. Enable it before rebuilding.",
                    )
                    .into());
                }
                self.clear_content_index(None)?;
                set_meta(&self.connection, PHASE_KEY, "rebuildPending")?;
                self.index_project_content_with(ContentIndexOptions::default())?;
                self.finish_content_refresh()?;
                Ok(())
            }
        }
    }

    fn clear_content_index(&mut self, enabled: Option<bool>) -> Result<()> {
        // Commit the gate BEFORE deletion so even a failed DELETE cannot leave
        // content searchable. A rejected gate transaction reports failure and
        // leaves the previous setting intact, never falsely claiming disabled.
        let tx = self.connection.transaction()?;
        if let Some(enabled) = enabled {
            set_meta(&tx, ENABLED_KEY, if enabled { "true" } else { "false" })?;
        }
        set_meta(&tx, PHASE_KEY, "cleanupPending")?;
        tx.commit()?;
        self.connection.execute_batch("PRAGMA secure_delete = ON")?;
        let tx = self.connection.transaction()?;
        // Reset FTS segments, not just posting-list tombstones: VACUUM alone
        // cannot erase terms still retained inside live FTS shadow blobs.
        tx.execute(
            "INSERT INTO project_content_fts(project_content_fts) VALUES('delete-all')",
            [],
        )?;
        tx.execute("DELETE FROM project_content_files", [])?;
        tx.execute("DELETE FROM project_content_status", [])?;
        tx.commit()?;
        // Logical removal is not physical cleanup. A pinned WAL reader or
        // failed VACUUM keeps cleanupPending, and a retry repeats these steps.
        self.connection.execute_batch("VACUUM")?;
        let busy: i64 =
            self.connection
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
        if busy != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "Content rows removed, but WAL cleanup is busy. Close active readers and retry clear.",
            )
            .into());
        }
        set_meta(&self.connection, PHASE_KEY, "ready")
    }

    pub(super) fn prepare_content_refresh(&mut self) -> Result<bool> {
        let enabled = read_enabled(&self.connection)?;
        let phase = read_phase(&self.connection)?;
        if phase != ContentIndexPhase::Ready {
            // Discard partial files from an interrupted rebuild, including
            // fingerprints that would otherwise let unchanged files be reused.
            self.clear_content_index(None)?;
        }
        if enabled && phase != ContentIndexPhase::Ready {
            set_meta(&self.connection, PHASE_KEY, "rebuildPending")?;
        }
        Ok(enabled)
    }

    pub(super) fn finish_content_refresh(&self) -> Result<()> {
        if read_phase(&self.connection)? == ContentIndexPhase::RebuildPending {
            set_meta(&self.connection, PHASE_KEY, "ready")?;
        }
        Ok(())
    }
}

fn meta(connection: &Connection, key: &str) -> Result<Option<String>> {
    let has_meta: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='sessionatlas_meta')",
        [],
        |row| row.get(0),
    )?;
    if !has_meta {
        return Ok(None);
    }
    Ok(connection
        .query_row(
            "SELECT value FROM sessionatlas_meta WHERE key=?1",
            [key],
            |row| row.get(0),
        )
        .optional()?)
}

fn set_meta(connection: &Connection, key: &str, value: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO sessionatlas_meta (key,value) VALUES (?1,?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn read_enabled(connection: &Connection) -> Result<bool> {
    match meta(connection, ENABLED_KEY)?.as_deref() {
        None | Some("true") => Ok(true),
        Some("false") => Ok(false),
        _ => Err(StoreError::CorruptRow(
            "Invalid content index policy".into(),
        )),
    }
}

fn read_phase(connection: &Connection) -> Result<ContentIndexPhase> {
    match meta(connection, PHASE_KEY)?.as_deref() {
        None | Some("ready") => Ok(ContentIndexPhase::Ready),
        Some("cleanupPending") => Ok(ContentIndexPhase::CleanupPending),
        Some("rebuildPending") => Ok(ContentIndexPhase::RebuildPending),
        _ => Err(StoreError::CorruptRow("Invalid content index phase".into())),
    }
}

pub(super) fn content_policy_allows_search(connection: &Connection) -> Result<bool> {
    Ok(matches!(
        meta(connection, ENABLED_KEY)?.as_deref(),
        None | Some("true")
    ) && matches!(
        meta(connection, PHASE_KEY)?.as_deref(),
        None | Some("ready")
    ))
}

fn read_status(connection: &Connection) -> Result<ContentIndexStatus> {
    // Keep flags and counts from one committed snapshot, even if a writer is
    // clearing/rebuilding concurrently.
    let tx = connection.unchecked_transaction()?;
    let enabled = read_enabled(&tx)?;
    let phase = read_phase(&tx)?;
    let searchable = content_index_is_safe(&tx)?;
    let has_files: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='project_content_files')",
        [], |row| row.get(0),
    )?;
    let (indexed_files, indexed_bytes) = if has_files {
        tx.query_row(
            "SELECT COUNT(*), COALESCE(SUM(indexed_bytes), 0) FROM project_content_files",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?
    } else {
        (0, 0)
    };
    tx.commit()?;
    Ok(ContentIndexStatus {
        index_exists: true,
        enabled,
        phase,
        searchable,
        indexed_files,
        indexed_bytes,
    })
}
