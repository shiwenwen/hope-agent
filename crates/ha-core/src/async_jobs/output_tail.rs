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

/// A job's bounded output ring plus the byte cap captured when it was registered.
/// The cap is snapshotted per-job so a mid-run config change can't shrink/grow an
/// in-flight ring (and so a tiny configured value can't truncate a job that
/// started under a larger one).
struct Tail {
    buf: Vec<u8>,
    cap: usize,
}

/// Per-job ring of the most recent output bytes (each capped at its own
/// registered `cap`).
static TAILS: LazyLock<Mutex<HashMap<String, Tail>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Safe band for the configured running-output tail size (R9): a pathological
/// config can neither make the ring useless (0 → floored) nor blow up RAM (the
/// ring count is already bounded by the concurrent-job cap).
const TAIL_BYTES_FLOOR: usize = 256;
const TAIL_BYTES_CEILING: usize = 1024 * 1024;

/// The configured running-output tail size (R9), clamped to `[256, 1MB]`. The
/// canonical default (8KB) lives in `AsyncToolsConfig`.
pub fn configured_bytes() -> usize {
    crate::config::cached_config()
        .async_tools
        .output_tail_bytes
        .clamp(TAIL_BYTES_FLOOR, TAIL_BYTES_CEILING)
}

/// Register an empty tail buffer for a job about to run, capped at `cap` bytes.
/// Idempotent — a second register keeps the first cap (the running ring is the
/// source of truth for its own lifetime).
pub fn register(job_id: &str, cap: usize) {
    TAILS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .entry(job_id.to_string())
        .or_insert_with(|| Tail {
            buf: Vec::new(),
            cap: cap.max(1),
        });
}

/// Append output bytes to a job's tail, trimming the front so only the last
/// `cap` bytes are retained. No-op if the job has no registered buffer
/// (e.g. incognito jobs never register one).
pub fn append(job_id: &str, bytes: &[u8]) {
    let mut map = TAILS.lock().unwrap_or_else(|p| p.into_inner());
    let Some(tail) = map.get_mut(job_id) else {
        return;
    };
    tail.buf.extend_from_slice(bytes);
    if tail.buf.len() > tail.cap {
        let overflow = tail.buf.len() - tail.cap;
        tail.buf.drain(..overflow);
    }
}

/// Read the current tail as a lossy-UTF8 string. `None` if the job has no
/// buffer; `Some("")` if it has one but no output yet.
pub fn read(job_id: &str) -> Option<String> {
    TAILS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(job_id)
        .map(|tail| String::from_utf8_lossy(&tail.buf).into_owned())
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

    // Representative ring size for the mechanics tests (the production default
    // lives in `AsyncToolsConfig::output_tail_bytes`).
    const TEST_CAP: usize = 8 * 1024;

    #[test]
    fn append_then_read_roundtrips() {
        let id = "tail-roundtrip";
        register(id, TEST_CAP);
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
        register(id, TEST_CAP);
        // Write more than the cap; only the last TEST_CAP bytes survive.
        let big = vec![b'a'; TEST_CAP + 100];
        append(id, &big);
        append(id, b"TAILEND");
        let got = read(id).unwrap();
        assert_eq!(got.len(), TEST_CAP);
        assert!(got.ends_with("TAILEND"));
        remove(id);
    }

    #[test]
    fn ring_honors_a_per_job_cap_smaller_than_default() {
        // R9: the cap is per-job (snapshotted at register), not the global const.
        let id = "tail-small-cap";
        register(id, 16);
        append(id, b"0123456789ABCDEF" /* exactly 16 */);
        append(id, b"GHIJ");
        let got = read(id).unwrap();
        assert_eq!(got.len(), 16, "trimmed to the per-job cap, not DEFAULT");
        assert!(got.ends_with("GHIJ"));
        remove(id);
    }

    #[test]
    fn first_register_cap_wins_over_a_later_one() {
        // Idempotent register must not resize a live ring.
        let id = "tail-cap-stable";
        register(id, 32);
        register(id, 4); // ignored — ring already exists with cap 32
        let big = vec![b'x'; 100];
        append(id, &big);
        assert_eq!(read(id).unwrap().len(), 32);
        remove(id);
    }
}
