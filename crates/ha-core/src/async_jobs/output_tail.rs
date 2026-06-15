//! Bounded in-memory output tail for running background-job tools (R3 ①).
//!
//! Mirrors Claude Code's `BashOutput`: while a backgrounded `exec` job runs, its
//! child stdout/stderr is teed into a small per-job ring buffer (last ~8KB) so
//! the agent can `job_status(action:status)` a *running* job and see the latest
//! output — enough to judge "still making progress" vs "stuck" without waiting
//! for completion. On completion the full result goes to `result_path`; the ring
//! is dropped. The tail is process-local (the producing exec runs in this
//! process) and is NEVER created for incognito jobs (close-and-burn parity with
//! the spool).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// Default tail size in bytes (keep the last ~8KB). Configurable knob deferred
/// to R9; a const keeps R3 self-contained.
pub const DEFAULT_TAIL_BYTES: usize = 8 * 1024;

/// Per-job ring of the most recent output bytes (capped at `DEFAULT_TAIL_BYTES`).
static TAILS: LazyLock<Mutex<HashMap<String, Vec<u8>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Register an empty tail buffer for a job about to run. Idempotent.
pub fn register(job_id: &str) {
    TAILS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .entry(job_id.to_string())
        .or_default();
}

/// Append output bytes to a job's tail, trimming the front so only the last
/// `DEFAULT_TAIL_BYTES` are retained. No-op if the job has no registered buffer
/// (e.g. incognito jobs never register one).
pub fn append(job_id: &str, bytes: &[u8]) {
    let mut map = TAILS.lock().unwrap_or_else(|p| p.into_inner());
    let Some(buf) = map.get_mut(job_id) else {
        return;
    };
    buf.extend_from_slice(bytes);
    if buf.len() > DEFAULT_TAIL_BYTES {
        let overflow = buf.len() - DEFAULT_TAIL_BYTES;
        buf.drain(..overflow);
    }
}

/// Read the current tail as a lossy-UTF8 string. `None` if the job has no
/// buffer; `Some("")` if it has one but no output yet.
pub fn read(job_id: &str) -> Option<String> {
    TAILS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(job_id)
        .map(|buf| String::from_utf8_lossy(buf).into_owned())
}

/// Drop a job's tail buffer (on completion / cancel / cleanup).
pub fn remove(job_id: &str) {
    TAILS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(job_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unregistered_append_is_noop_and_read_is_none() {
        let id = "tail-unregistered";
        append(id, b"hello");
        assert_eq!(read(id), None);
    }

    #[test]
    fn append_then_read_roundtrips() {
        let id = "tail-roundtrip";
        register(id);
        assert_eq!(read(id).as_deref(), Some(""));
        append(id, b"hello ");
        append(id, b"world");
        assert_eq!(read(id).as_deref(), Some("hello world"));
        remove(id);
        assert_eq!(read(id), None);
    }

    #[test]
    fn ring_keeps_only_the_last_n_bytes() {
        let id = "tail-ring";
        register(id);
        // Write more than the cap; only the last DEFAULT_TAIL_BYTES survive.
        let big = vec![b'a'; DEFAULT_TAIL_BYTES + 100];
        append(id, &big);
        append(id, b"TAILEND");
        let got = read(id).unwrap();
        assert_eq!(got.len(), DEFAULT_TAIL_BYTES);
        assert!(got.ends_with("TAILEND"));
        remove(id);
    }
}
