//! Per-session guard for user-facing chat turns.
//!
//! This sits one layer above `stream_seq`: callers acquire it before they
//! persist the user message, so reloads or duplicate "continue" clicks cannot
//! create a second main turn for the same session.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, OnceLock};

use super::stream_seq::{ChatSource, ACTIVE_STREAM_ERROR_CODE};

#[derive(Debug, Clone)]
pub struct ActiveTurnError {
    pub session_id: String,
    pub existing_source: ChatSource,
}

impl fmt::Display for ActiveTurnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{ACTIVE_STREAM_ERROR_CODE}: session {} already has an active {} chat turn",
            self.session_id, self.existing_source
        )
    }
}

impl std::error::Error for ActiveTurnError {}

#[derive(Debug, Clone)]
struct Entry {
    token: String,
    source: ChatSource,
}

static ACTIVE_TURNS: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, Entry>> {
    ACTIVE_TURNS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug)]
pub struct ActiveTurnGuard {
    session_id: String,
    token: String,
    released: bool,
}

impl ActiveTurnGuard {
    pub fn release(&mut self) {
        if self.released {
            return;
        }
        let mut map = registry()
            .lock()
            .expect("active chat turn registry poisoned");
        if map
            .get(&self.session_id)
            .map(|entry| entry.token.as_str() == self.token)
            .unwrap_or(false)
        {
            map.remove(&self.session_id);
        }
        self.released = true;
    }
}

impl Drop for ActiveTurnGuard {
    fn drop(&mut self) {
        self.release();
    }
}

pub fn try_acquire(
    session_id: &str,
    source: ChatSource,
) -> Result<ActiveTurnGuard, ActiveTurnError> {
    let token = uuid::Uuid::new_v4().to_string();
    let mut map = registry()
        .lock()
        .expect("active chat turn registry poisoned");
    if let Some(existing) = map.get(session_id) {
        return Err(ActiveTurnError {
            session_id: session_id.to_string(),
            existing_source: existing.source,
        });
    }
    map.insert(
        session_id.to_string(),
        Entry {
            token: token.clone(),
            source,
        },
    );
    Ok(ActiveTurnGuard {
        session_id: session_id.to_string(),
        token,
        released: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_second_turn_until_guard_drops() {
        let sid = "test-active-turn-rejects-second";
        {
            let _guard = try_acquire(sid, ChatSource::Desktop).unwrap();
            let err = try_acquire(sid, ChatSource::Http).unwrap_err();
            assert_eq!(err.session_id, sid);
            assert_eq!(err.existing_source, ChatSource::Desktop);
        }

        let _guard = try_acquire(sid, ChatSource::Http).unwrap();
    }
}
