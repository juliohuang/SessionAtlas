use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

pub(crate) const MAX_PTY_COLS: u16 = 1_000;
pub(crate) const MAX_PTY_ROWS: u16 = 1_000;
pub(crate) const MAX_PTY_INPUT_BYTES: usize = 1024 * 1024;

pub(crate) fn normalize_pty_size(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(2, MAX_PTY_COLS), rows.clamp(1, MAX_PTY_ROWS))
}

pub(crate) fn validate_pty_input(data: &str) -> Result<(), String> {
    if data.len() > MAX_PTY_INPUT_BYTES {
        return Err(format!(
            "PTY input exceeds the {MAX_PTY_INPUT_BYTES}-byte limit"
        ));
    }
    Ok(())
}

pub(crate) fn take_once<T>(slot: &Mutex<Option<T>>) -> Result<Option<T>, String> {
    let mut value = slot
        .lock()
        .map_err(|_| "one-shot PTY state lock poisoned".to_string())?;
    Ok(value.take())
}

/// Concurrent session ownership with a short global map lock. Each stored
/// value owns its operation-specific synchronization, so the registry never
/// stays locked during PTY I/O or child-process waits.
pub(crate) struct SessionStore<T> {
    inner: Arc<SessionStoreInner<T>>,
}

struct SessionStoreInner<T> {
    next: AtomicU32,
    sessions: Mutex<HashMap<u32, Arc<T>>>,
}

impl<T> Default for SessionStore<T> {
    fn default() -> Self {
        Self {
            inner: Arc::new(SessionStoreInner {
                next: AtomicU32::new(0),
                sessions: Mutex::new(HashMap::new()),
            }),
        }
    }
}

impl<T> Clone for SessionStore<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> SessionStore<T> {
    pub(crate) fn insert(&self, value: T) -> Result<u32, (String, T)> {
        let mut sessions = self
            .inner
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let mut value = Some(value);
        for _ in 0..u32::MAX {
            let id = self
                .inner
                .next
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_add(1);
            if id == 0 || sessions.contains_key(&id) {
                continue;
            }

            sessions.insert(
                id,
                Arc::new(value.take().expect("session value inserted once")),
            );
            return Ok(id);
        }

        Err((
            "PTY session id space exhausted".to_string(),
            value.expect("session value remains when insertion fails"),
        ))
    }

    pub(crate) fn get(&self, id: u32) -> Result<Option<Arc<T>>, String> {
        let sessions = self
            .inner
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(sessions.get(&id).cloned())
    }

    pub(crate) fn remove(&self, id: u32) -> Result<Option<Arc<T>>, String> {
        let mut sessions = self
            .inner
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(sessions.remove(&id))
    }

    pub(crate) fn drain(&self) -> Result<Vec<Arc<T>>, String> {
        let mut sessions = self
            .inner
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(sessions.drain().map(|(_, session)| session).collect())
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner.sessions.lock().unwrap().len()
    }

    #[cfg(test)]
    fn set_next(&self, next: u32) {
        self.inner.next.store(next, Ordering::Relaxed);
    }
}

/// Incremental UTF-8 decoder for arbitrary PTY read boundaries. A multi-byte
/// character split between reads is retained until complete instead of being
/// replaced twice by U+FFFD.
pub(crate) struct Utf8StreamDecoder {
    pending: Vec<u8>,
}

impl Utf8StreamDecoder {
    pub(crate) fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut output = String::new();
        let mut consumed = 0;

        while consumed < self.pending.len() {
            match std::str::from_utf8(&self.pending[consumed..]) {
                Ok(valid) => {
                    output.push_str(valid);
                    consumed = self.pending.len();
                }
                Err(error) => {
                    let valid_end = consumed + error.valid_up_to();
                    if valid_end > consumed {
                        output.push_str(
                            std::str::from_utf8(&self.pending[consumed..valid_end])
                                .expect("valid_up_to bytes are UTF-8"),
                        );
                    }
                    consumed = valid_end;
                    match error.error_len() {
                        Some(invalid_len) => {
                            output.push('\u{fffd}');
                            consumed += invalid_len;
                        }
                        None => break,
                    }
                }
            }
        }

        if consumed > 0 {
            self.pending.drain(..consumed);
        }
        output
    }

    pub(crate) fn finish(&mut self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        self.pending.clear();
        "\u{fffd}".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pty_size_is_never_zero_or_unbounded() {
        assert_eq!(normalize_pty_size(0, 0), (2, 1));
        assert_eq!(normalize_pty_size(80, 24), (80, 24));
        assert_eq!(
            normalize_pty_size(u16::MAX, u16::MAX),
            (MAX_PTY_COLS, MAX_PTY_ROWS)
        );
    }

    #[test]
    fn pty_input_has_a_bounded_write_size() {
        validate_pty_input(&"x".repeat(MAX_PTY_INPUT_BYTES)).unwrap();
        assert!(validate_pty_input(&"x".repeat(MAX_PTY_INPUT_BYTES + 1)).is_err());
    }

    #[test]
    fn one_shot_state_is_taken_exactly_once() {
        let slot = Mutex::new(Some("reader"));
        assert_eq!(take_once(&slot).unwrap(), Some("reader"));
        assert_eq!(take_once(&slot).unwrap(), None);
    }

    #[test]
    fn session_store_remove_and_drain_are_idempotent() {
        let store = SessionStore::default();
        let first = store.insert("first").unwrap();
        let second = store.insert("second").unwrap();
        assert_eq!(store.len(), 2);

        assert!(store.remove(first).unwrap().is_some());
        assert!(store.remove(first).unwrap().is_none());
        assert_eq!(store.drain().unwrap().len(), 1);
        assert_eq!(store.len(), 0);
        assert!(store.remove(second).unwrap().is_none());
    }

    #[test]
    fn session_ids_wrap_without_zero_or_collision() {
        let store = SessionStore::default();
        assert_eq!(store.insert("one").unwrap(), 1);
        store.set_next(u32::MAX - 1);
        assert_eq!(store.insert("max").unwrap(), u32::MAX);
        assert_eq!(store.insert("after-wrap").unwrap(), 2);
    }

    #[test]
    fn decoder_preserves_utf8_split_across_reads() {
        let mut decoder = Utf8StreamDecoder::new();
        assert_eq!(decoder.push(&[0xf0, 0x9f]), "");
        assert_eq!(decoder.push(&[0x98, 0x80]), "😀");
        assert_eq!(decoder.finish(), "");
    }

    #[test]
    fn decoder_replaces_invalid_and_incomplete_sequences_once() {
        let mut decoder = Utf8StreamDecoder::new();
        assert_eq!(decoder.push(&[0xff, b'a']), "\u{fffd}a");
        assert_eq!(decoder.push(&[0xe2]), "");
        assert_eq!(decoder.finish(), "\u{fffd}");
    }
}
