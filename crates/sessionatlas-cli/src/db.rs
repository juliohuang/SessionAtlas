//! Index database path resolution.
//!
//! The canonical data root is `~/.sessionatlas/`. The default database is
//! `$SESSIONATLAS_HOME/.sessionatlas/index.db` when `SESSIONATLAS_HOME` is set
//! to a non-blank value, otherwise `<user home>/.sessionatlas/index.db`,
//! using the same home-resolution contract as the shared core.

use std::path::{Path, PathBuf};

use sessionatlas_core::store::SqliteStore;

/// `~/.sessionatlas/index.db` under an explicit home directory.
pub fn db_path_for_home(home: impl AsRef<Path>) -> PathBuf {
    home.as_ref().join(".sessionatlas").join("index.db")
}

/// Default index database path for this process (follows `SESSIONATLAS_HOME`).
pub fn default_db_path() -> PathBuf {
    db_path_for_home(sessionatlas_core::config::home_directory())
}

/// Opens the index database for the read-only commands.
///
/// A missing database is a hard error: the read-only commands never create the
/// index or touch user data — `scan` (R09) owns creation. An existing database
/// is opened through the core store, which applies the same idempotent schema
/// maintenance required whenever an existing index is opened.
pub fn open_index_store(db_path: &Path) -> Result<SqliteStore, String> {
    if !db_path.is_file() {
        return Err(format!(
            "未找到索引数据库 {}，请先运行 `sessionatlas scan`。",
            db_path.display()
        ));
    }
    SqliteStore::new(db_path).map_err(|error| format!("打开索引数据库失败: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_path_for_home_joins_dot_sessionatlas_index_db() {
        let home = Path::new("/tmp/fake-home");
        assert_eq!(
            db_path_for_home(home),
            PathBuf::from("/tmp/fake-home")
                .join(".sessionatlas")
                .join("index.db")
        );
    }
}
