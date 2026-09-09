//! Coordination primitives for local index scans.
//!
//! The index is shared by the CLI and the desktop application.  The file lock
//! in this module is therefore deliberately independent of SQLite's locking
//! and is held for the complete lifetime of a writer.  [`ScanCoordinator`]
//! coalesces concurrent in-process requests into at most one follow-up scan.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::File;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use fs2::FileExt;

use crate::private_fs;

/// Default bound for acquiring the shared index writer lock.
pub const DEFAULT_INDEX_WRITE_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Errors acquiring the cross-process index writer lock.
#[derive(Debug)]
pub enum IndexWriteLockError {
    /// The lock remained held until the bounded timeout elapsed.
    Busy { path: PathBuf, timeout: Duration },
    /// The lock file could not be created or locked.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// A caller tried to use a guard acquired for another database.
    WrongDatabase {
        lock_path: PathBuf,
        database: PathBuf,
    },
}

impl std::fmt::Display for IndexWriteLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy { path, timeout } => write!(
                f,
                "index write lock is busy: {} (timed out after {} ms; retry later)",
                path.display(),
                timeout.as_millis()
            ),
            Self::Io { path, source } => {
                write!(
                    f,
                    "could not open index write lock {}: {source}",
                    path.display()
                )
            }
            Self::WrongDatabase {
                lock_path,
                database,
            } => write!(
                f,
                "index write guard {} does not protect {}",
                lock_path.display(),
                database.display()
            ),
        }
    }
}

impl std::error::Error for IndexWriteLockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Busy { .. } | Self::WrongDatabase { .. } => None,
        }
    }
}

/// A held exclusive lock for an index database.
///
/// The lock file is always the sibling `<index.db>.lock`.  It is intentionally
/// retained after release (like a normal lock file); only its advisory lock is
/// released when this value is dropped.
#[derive(Debug)]
pub struct IndexWriteGuard {
    file: File,
    path: PathBuf,
}

impl IndexWriteGuard {
    /// Returns the fixed lock-file path for an index database.
    pub fn lock_path(database_path: impl AsRef<Path>) -> PathBuf {
        let mut path = database_path.as_ref().as_os_str().to_os_string();
        path.push(".lock");
        PathBuf::from(path)
    }

    /// Acquires the index lock using [`DEFAULT_INDEX_WRITE_LOCK_TIMEOUT`].
    pub fn acquire(database_path: impl AsRef<Path>) -> Result<Self, IndexWriteLockError> {
        Self::acquire_with_timeout(database_path, DEFAULT_INDEX_WRITE_LOCK_TIMEOUT)
    }

    /// Acquires the index lock, retrying until `timeout` elapses.
    pub fn acquire_with_timeout(
        database_path: impl AsRef<Path>,
        timeout: Duration,
    ) -> Result<Self, IndexWriteLockError> {
        let database_path = database_path.as_ref();
        let fallback_path = Self::lock_path(database_path);
        let absolute_database =
            std::path::absolute(database_path).map_err(|source| IndexWriteLockError::Io {
                path: fallback_path,
                source,
            })?;
        let lock_path = Self::lock_path(&absolute_database);
        let started = Instant::now();
        let deadline = started
            .checked_add(timeout)
            .ok_or_else(|| IndexWriteLockError::Io {
                path: lock_path.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "index write lock timeout is too large",
                ),
            })?;
        if let Some(parent) = lock_path.parent() {
            let result = if parent
                .file_name()
                .is_some_and(|name| name == ".sessionatlas")
            {
                private_fs::ensure_private_directory(parent)
            } else {
                std::fs::create_dir_all(parent)
            };
            result.map_err(|source| IndexWriteLockError::Io {
                path: lock_path.clone(),
                source,
            })?;
        }
        let file = private_fs::open_private_read_write(&lock_path).map_err(|source| {
            IndexWriteLockError::Io {
                path: lock_path.clone(),
                source,
            }
        })?;
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => {
                    return Ok(Self {
                        file,
                        path: lock_path,
                    })
                }
                Err(source) if is_lock_contention(&source) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(IndexWriteLockError::Busy {
                            path: lock_path,
                            timeout,
                        });
                    }
                    thread::sleep(Duration::from_millis(25).min(deadline.duration_since(now)));
                }
                Err(source) => {
                    return Err(IndexWriteLockError::Io {
                        path: lock_path,
                        source,
                    })
                }
            }
        }
    }

    /// Returns the lock path represented by this guard.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether this guard was acquired for `database_path`.
    pub fn protects(&self, database_path: impl AsRef<Path>) -> bool {
        std::path::absolute(database_path.as_ref())
            .map(|database| Self::lock_path(database) == self.path)
            .unwrap_or(false)
    }
}

fn is_lock_contention(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

impl Drop for IndexWriteGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

type Operation<R> = Box<dyn FnOnce() -> R + Send + 'static>;

thread_local! {
    static ACTIVE_COORDINATORS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

struct ActiveCoordinatorGuard {
    id: usize,
}

impl ActiveCoordinatorGuard {
    fn assert_not_reentrant(id: usize) {
        ACTIVE_COORDINATORS.with(|active| {
            assert!(
                !active.borrow().contains(&id),
                "a scan coordinator operation cannot recursively call the same coordinator"
            );
        });
    }

    fn enter(id: usize) -> Self {
        ACTIVE_COORDINATORS.with(|active| active.borrow_mut().push(id));
        Self { id }
    }
}

impl Drop for ActiveCoordinatorGuard {
    fn drop(&mut self) {
        ACTIVE_COORDINATORS.with(|active| {
            let removed = active.borrow_mut().pop();
            debug_assert_eq!(removed, Some(self.id));
        });
    }
}

struct CoordinatorState<R> {
    running: bool,
    following_up: bool,
    pending: Option<Operation<R>>,
    generation: u64,
    waiters: BTreeMap<u64, usize>,
    results: BTreeMap<u64, SharedResult<R>>,
}

#[derive(Clone)]
enum SharedResult<R> {
    Completed(R),
    Panicked,
}

/// Observable coordinator state, primarily useful for diagnostics and
/// deterministic concurrency tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanCoordinatorStatus {
    pub running: bool,
    pub pending: bool,
}

/// Thread-safe in-process scan coalescing.
///
/// The first caller runs immediately.  While it is running, one follow-up
/// operation is retained; all other callers wait for that follow-up (or the
/// first operation when no follow-up was needed) and receive its cloned result.
/// Requests arriving during the follow-up share that result without scheduling
/// another pass, so a continuous request stream cannot indefinitely extend a batch.
/// Panics release the state before being resumed, so a later request can retry.
/// Recursively invoking the same coordinator from its owner operation panics
/// immediately instead of waiting on itself.
pub struct ScanCoordinator<R: Clone> {
    state: Arc<(Mutex<CoordinatorState<R>>, Condvar)>,
}

impl<R: Clone> Default for ScanCoordinator<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: Clone> Clone for ScanCoordinator<R> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
        }
    }
}

impl<R: Clone> ScanCoordinator<R> {
    /// Creates an idle coordinator.
    pub fn new() -> Self {
        Self {
            state: Arc::new((
                Mutex::new(CoordinatorState {
                    running: false,
                    following_up: false,
                    pending: None,
                    generation: 0,
                    waiters: BTreeMap::new(),
                    results: BTreeMap::new(),
                }),
                Condvar::new(),
            )),
        }
    }

    /// Runs or coalesces one operation.
    pub fn run<F>(&self, operation: F) -> R
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let coordinator_id = Arc::as_ptr(&self.state) as usize;
        ActiveCoordinatorGuard::assert_not_reentrant(coordinator_id);
        let mut operation: Option<Operation<R>> = Some(Box::new(operation));
        let (lock, wake) = &*self.state;
        {
            let mut state = lock.lock().unwrap_or_else(|error| error.into_inner());
            if !state.running {
                state.running = true;
                state.following_up = false;
            } else {
                let target_generation = state
                    .generation
                    .checked_add(1)
                    .expect("scan coordinator generation overflow");
                *state.waiters.entry(target_generation).or_default() += 1;
                if !state.following_up && state.pending.is_none() {
                    state.pending = operation.take();
                }
                state = wake
                    .wait_while(state, |state| state.generation < target_generation)
                    .unwrap_or_else(|error| error.into_inner());
                let outcome = state
                    .results
                    .get(&target_generation)
                    .expect("completed coordinator generation has a result")
                    .clone();
                let remaining = state
                    .waiters
                    .get_mut(&target_generation)
                    .expect("waiting coordinator generation is tracked");
                *remaining -= 1;
                if *remaining == 0 {
                    state.waiters.remove(&target_generation);
                    state.results.remove(&target_generation);
                }
                drop(state);
                return match outcome {
                    SharedResult::Completed(result) => result,
                    SharedResult::Panicked => panic!("coordinated scan operation panicked"),
                };
            }
        }

        let _active = ActiveCoordinatorGuard::enter(coordinator_id);

        // The caller that transitioned idle -> running owns execution.  A
        // completed first pass starts the single pending pass before notifying
        // waiters, so every coalesced caller observes the same final result.
        let mut next = operation.take();
        loop {
            let current = next
                .take()
                .expect("coordinator owner must have an operation");
            let outcome = catch_unwind(AssertUnwindSafe(current));
            match outcome {
                Ok(result) => {
                    let mut state = lock.lock().unwrap_or_else(|error| error.into_inner());
                    if let Some(pending) = state.pending.take() {
                        state.following_up = true;
                        next = Some(pending);
                        drop(state);
                        continue;
                    }
                    let completed_generation = state
                        .generation
                        .checked_add(1)
                        .expect("scan coordinator generation overflow");
                    state.running = false;
                    state.following_up = false;
                    state.generation = completed_generation;
                    if state.waiters.contains_key(&completed_generation) {
                        state.results.insert(
                            completed_generation,
                            SharedResult::Completed(result.clone()),
                        );
                    }
                    wake.notify_all();
                    return result;
                }
                Err(payload) => {
                    let mut state = lock.lock().unwrap_or_else(|error| error.into_inner());
                    let completed_generation = state
                        .generation
                        .checked_add(1)
                        .expect("scan coordinator generation overflow");
                    state.running = false;
                    state.following_up = false;
                    // Waiting requests are allowed to retry after the panic;
                    // retaining a closure here could make a panic unexpectedly
                    // execute a stale request after recovery.
                    state.pending = None;
                    state.generation = completed_generation;
                    if state.waiters.contains_key(&completed_generation) {
                        state
                            .results
                            .insert(completed_generation, SharedResult::Panicked);
                    }
                    wake.notify_all();
                    drop(state);
                    resume_unwind(payload);
                }
            }
        }
    }

    /// Returns whether a scan is running and whether one follow-up is queued.
    pub fn status(&self) -> ScanCoordinatorStatus {
        let state = self
            .state
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        ScanCoordinatorStatus {
            running: state.running,
            pending: state.pending.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;

    fn wait_for_waiters(coordinator: &ScanCoordinator<usize>, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let count = coordinator
                .state
                .0
                .lock()
                .unwrap()
                .waiters
                .values()
                .sum::<usize>();
            if count == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "expected {expected} waiters, got {count}"
            );
            thread::yield_now();
        }
    }

    // Called only by the isolated parent regression below. Running the full
    // unit suite without its private fixture variable makes this a no-op.
    #[test]
    fn writer_lock_child_process_fixture() {
        use std::io::{Read, Write};
        let Some(database) = std::env::var_os("SESSIONATLAS_TEST_LOCK_DATABASE") else {
            return;
        };
        let _guard = IndexWriteGuard::acquire(PathBuf::from(database)).unwrap();
        println!("SESSIONATLAS_TEST_LOCK_READY");
        std::io::stdout().flush().unwrap();
        let _ = std::io::stdin().read_exact(&mut [0_u8]);
    }

    #[test]
    fn another_process_holds_the_lock_until_it_exits() {
        use std::io::{BufRead, BufReader};
        use std::process::{Child, Command, Stdio};
        use std::sync::mpsc;

        struct ChildGuard(Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("index.db");
        let mut child = ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "scan_coordinator::tests::writer_lock_child_process_fixture",
                    "--nocapture",
                ])
                .env("SESSIONATLAS_TEST_LOCK_DATABASE", &database)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let stdout = child.0.stdout.take().unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let output_reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if line.unwrap().contains("SESSIONATLAS_TEST_LOCK_READY") {
                    let _ = ready_tx.send(());
                    return;
                }
            }
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let error = IndexWriteGuard::acquire_with_timeout(&database, Duration::ZERO).unwrap_err();
        assert!(matches!(error, IndexWriteLockError::Busy { .. }), "{error}");
        // Simulate an abrupt exit; no Rust Drop implementation runs in the
        // child. The OS must still release its advisory lock.
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        output_reader.join().unwrap();
        let guard = IndexWriteGuard::acquire_with_timeout(&database, Duration::ZERO).unwrap();
        assert!(
            guard.path().is_file(),
            "coordination file must remain after exit"
        );
    }

    #[test]
    fn lock_contention_is_bounded_and_release_allows_retry() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        let first = IndexWriteGuard::acquire(&db).unwrap();
        let error = IndexWriteGuard::acquire_with_timeout(&db, Duration::from_millis(40))
            .expect_err("a held guard must report contention");
        assert!(
            matches!(error, IndexWriteLockError::Busy { .. }),
            "{error:?}"
        );
        drop(first);
        IndexWriteGuard::acquire_with_timeout(&db, Duration::from_millis(40)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn lock_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        let guard = IndexWriteGuard::acquire(&db).unwrap();
        assert_eq!(
            std::fs::metadata(guard.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn concurrent_requests_coalesce_to_one_pending_operation() {
        let coordinator = ScanCoordinator::<usize>::new();
        let runs = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let first = {
            let coordinator = coordinator.clone();
            let runs = Arc::clone(&runs);
            let started = Arc::clone(&started);
            let release = Arc::clone(&release);
            std::thread::spawn(move || {
                coordinator.run(move || {
                    runs.fetch_add(1, Ordering::SeqCst);
                    started.wait();
                    release.wait();
                    7
                })
            })
        };
        started.wait();
        let second = {
            let coordinator = coordinator.clone();
            let runs = Arc::clone(&runs);
            std::thread::spawn(move || {
                coordinator.run(move || {
                    runs.fetch_add(1, Ordering::SeqCst);
                    8
                })
            })
        };
        let deadline = Instant::now() + Duration::from_secs(1);
        while !coordinator.status().pending && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(coordinator.status().pending, "follow-up was not queued");
        release.wait();
        assert_eq!(first.join().unwrap(), 8);
        assert_eq!(second.join().unwrap(), 8);
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn requests_during_follow_up_do_not_extend_the_batch() {
        use std::sync::mpsc;

        let coordinator = ScanCoordinator::<usize>::new();
        let runs = Arc::new(AtomicUsize::new(0));
        let (first_started_tx, first_started_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        let owner = {
            let coordinator = coordinator.clone();
            let runs = Arc::clone(&runs);
            thread::spawn(move || {
                coordinator.run(move || {
                    runs.fetch_add(1, Ordering::SeqCst);
                    first_started_tx.send(()).unwrap();
                    release_first_rx.recv().unwrap();
                    1
                })
            })
        };
        first_started_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();

        let (follow_up_started_tx, follow_up_started_rx) = mpsc::channel();
        let (release_follow_up_tx, release_follow_up_rx) = mpsc::channel();
        let follower = {
            let coordinator = coordinator.clone();
            let runs = Arc::clone(&runs);
            thread::spawn(move || {
                coordinator.run(move || {
                    runs.fetch_add(1, Ordering::SeqCst);
                    follow_up_started_tx.send(()).unwrap();
                    release_follow_up_rx.recv().unwrap();
                    2
                })
            })
        };
        wait_for_waiters(&coordinator, 1);
        release_first_tx.send(()).unwrap();
        follow_up_started_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();

        let late_callers: Vec<_> = (0..6)
            .map(|_| {
                let coordinator = coordinator.clone();
                let runs = Arc::clone(&runs);
                thread::spawn(move || {
                    coordinator.run(move || {
                        runs.fetch_add(1, Ordering::SeqCst);
                        3
                    })
                })
            })
            .collect();
        wait_for_waiters(&coordinator, 7);
        assert!(!coordinator.status().pending);
        release_follow_up_tx.send(()).unwrap();
        assert_eq!(owner.join().unwrap(), 2);
        assert_eq!(follower.join().unwrap(), 2);
        for caller in late_callers {
            assert_eq!(caller.join().unwrap(), 2);
        }
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        assert_eq!(coordinator.run(|| 4), 4);
    }

    #[test]
    fn panic_releases_coordinator_for_retry() {
        let coordinator = ScanCoordinator::<usize>::new();
        let result = std::panic::catch_unwind({
            let coordinator = coordinator.clone();
            move || coordinator.run(|| panic!("injected"))
        });
        assert!(result.is_err());
        assert_eq!(coordinator.run(|| 11), 11);
    }

    #[test]
    fn recursive_call_panics_instead_of_waiting_for_itself() {
        let coordinator = ScanCoordinator::<usize>::new();
        let nested = coordinator.clone();
        let result = std::panic::catch_unwind({
            let coordinator = coordinator.clone();
            move || coordinator.run(move || nested.run(|| 1))
        });
        assert!(result.is_err());
        assert_eq!(coordinator.run(|| 2), 2);
    }

    #[test]
    fn panic_notifies_waiters_without_returning_an_older_result() {
        let coordinator = ScanCoordinator::<usize>::new();
        assert_eq!(coordinator.run(|| 3), 3);
        let started = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let owner = {
            let coordinator = coordinator.clone();
            let started = Arc::clone(&started);
            let release = Arc::clone(&release);
            std::thread::spawn(move || {
                coordinator.run(move || {
                    started.wait();
                    release.wait();
                    panic!("injected owner panic")
                })
            })
        };
        started.wait();
        let waiter = {
            let coordinator = coordinator.clone();
            std::thread::spawn(move || coordinator.run(|| 99))
        };
        let deadline = Instant::now() + Duration::from_secs(1);
        while !coordinator.status().pending && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(coordinator.status().pending, "waiter was not queued");
        release.wait();
        assert!(owner.join().is_err());
        assert!(waiter.join().is_err());
        assert_eq!(coordinator.run(|| 11), 11);
    }
}
