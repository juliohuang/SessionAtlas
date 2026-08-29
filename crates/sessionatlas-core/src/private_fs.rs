//! Private filesystem primitives for SessionAtlas-owned local state.
//!
//! On Unix, the application data boundary is an owner-only directory and
//! owner-only files. Existing state is repaired on open so upgrades do not
//! retain permissions inherited from a permissive process umask.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

/// Creates `path` when needed and enforces an owner-only directory on Unix.
/// Only the exact directory is changed; existing ancestor permissions are not.
pub fn ensure_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err(io::Error::other(
                "private data directory is not a regular directory",
            ));
        }
        ensure_current_owner(metadata.uid())?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Opens or creates an owner-only read/write file.
pub fn open_private_read_write(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    configure_private_options(&mut options);
    let file = options.open(path)?;
    harden_open_file(&file)?;
    Ok(file)
}

/// Creates a new owner-only file and fails when the path already exists.
pub fn open_private_create_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    configure_private_options(&mut options);
    let file = options.open(path)?;
    harden_open_file(&file)?;
    Ok(file)
}

/// Repairs an existing sensitive file. Missing files are reported as `false`.
pub fn harden_existing_private_file(path: &Path) -> io::Result<bool> {
    let mut options = OpenOptions::new();
    options.read(true);
    configure_no_follow(&mut options);
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    harden_open_file(&file)?;
    Ok(true)
}

/// Prepares a SQLite database before rusqlite opens it.
pub fn prepare_private_database(path: &Path) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        if parent
            .file_name()
            .is_some_and(|name| name == ".sessionatlas")
        {
            ensure_private_directory(parent)?;
        } else {
            fs::create_dir_all(parent)?;
        }
    }
    drop(open_private_read_write(path)?);
    Ok(())
}

/// Repairs SQLite sidecars that exist while the owning connection is active.
/// SQLite derives new Unix sidecar modes from the main database; the private
/// parent directory remains the authoritative boundary during creation races.
pub fn harden_sqlite_sidecars(path: &Path) -> io::Result<()> {
    for suffix in ["-journal", "-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        harden_existing_private_file(Path::new(&sidecar))?;
    }
    Ok(())
}

fn configure_private_options(_options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        _options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
}

fn configure_no_follow(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(not(unix))]
    let _ = options;
}

fn harden_open_file(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other("private data path is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure_current_owner(metadata.uid())?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_current_owner(owner: u32) -> io::Result<()> {
    // SAFETY: geteuid has no preconditions and returns the effective process UID.
    let current = unsafe { libc::geteuid() };
    if owner != current {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private data path is not owned by the current user",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::*;

    #[cfg(unix)]
    #[test]
    fn repairs_existing_permissive_directory_and_file_modes() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let root = tempfile::tempdir().unwrap();
        let data = root.path().join(".sessionatlas");
        fs::create_dir(&data).unwrap();
        fs::set_permissions(&data, fs::Permissions::from_mode(0o755)).unwrap();
        let file = data.join("index.db");
        fs::write(&file, b"fixture").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();

        ensure_private_directory(&data).unwrap();
        assert!(harden_existing_private_file(&file).unwrap());

        assert_eq!(fs::metadata(&data).unwrap().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(&file).unwrap().mode() & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn sqlite_wal_sidecars_remain_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let data = root.path().join(".sessionatlas");
        fs::create_dir(&data).unwrap();
        fs::set_permissions(&data, fs::Permissions::from_mode(0o755)).unwrap();
        let database = data.join("prefs.db");

        prepare_private_database(&database).unwrap();
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .unwrap();
        connection
            .execute_batch("CREATE TABLE fixture(value TEXT); INSERT INTO fixture VALUES ('x');")
            .unwrap();
        harden_sqlite_sidecars(&database).unwrap();

        assert_eq!(
            fs::metadata(&data).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&database).unwrap().permissions().mode() & 0o777,
            0o600
        );
        for suffix in ["-journal", "-wal", "-shm"] {
            let mut sidecar = database.as_os_str().to_os_string();
            sidecar.push(suffix);
            let sidecar = std::path::PathBuf::from(sidecar);
            if sidecar.exists() {
                assert_eq!(
                    fs::metadata(sidecar).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
    }
}
