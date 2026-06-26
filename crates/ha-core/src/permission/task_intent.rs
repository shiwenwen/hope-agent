//! Per-session "task intent" store for the Smart-mode judge.
//!
//! A scheduled (cron) run is **pre-authorized** by the user: the prompt they
//! wrote when creating the task *is* the authorization for whatever the run
//! does (e.g. "delete the temp dir", "email me a summary"). When such a run is
//! in Smart mode, the judge ([`super::judge`]) gets this intent so it can allow
//! actions that are consistent with it and deny out-of-scope / injection-driven
//! ones — while strict gates (protected paths, dangerous commands, raw CDP)
//! stay blocked regardless.
//!
//! This is a process-global session-keyed map (mirroring
//! [`super::session_edits`]) rather than threading a field through every
//! `ChatEngineParams` / `ToolExecContext`: only the cron executor populates it,
//! and only the engine reads it. Non-cron sessions never set an intent, so the
//! judge sees `None` and behaves exactly as before.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Cap the stored intent so an unusually long cron prompt can't bloat the judge
/// prompt / blow its token budget. The intent is the user-authored task prompt
/// (trusted), but still bounded defensively.
const MAX_INTENT_BYTES: usize = 2048;

fn store() -> &'static Mutex<HashMap<String, String>> {
    static STORE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record the pre-authorized task intent for a session. Truncated to
/// [`MAX_INTENT_BYTES`] on a UTF-8 boundary. Blank intents are ignored.
pub fn set(session_id: &str, intent: &str) {
    let trimmed = intent.trim();
    if trimmed.is_empty() {
        return;
    }
    let bounded = crate::truncate_utf8(trimmed, MAX_INTENT_BYTES).to_string();
    if let Ok(mut map) = store().lock() {
        map.insert(session_id.to_string(), bounded);
    }
}

/// Look up the pre-authorized task intent for a session, if any.
pub fn get(session_id: &str) -> Option<String> {
    store().lock().ok()?.get(session_id).cloned()
}

/// Drop a session's recorded intent (called when its run finishes).
pub fn clear(session_id: &str) {
    if let Ok(mut map) = store().lock() {
        map.remove(session_id);
    }
}

/// RAII guard: records the intent on construction and clears it on drop, so the
/// entry never leaks if the run panics / times out / is cancelled.
pub struct TaskIntentGuard {
    session_id: String,
}

impl TaskIntentGuard {
    pub fn new(session_id: &str, intent: &str) -> Self {
        set(session_id, intent);
        Self {
            session_id: session_id.to_string(),
        }
    }
}

impl Drop for TaskIntentGuard {
    fn drop(&mut self) {
        clear(&self.session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_clear_round_trip() {
        let sid = "task-intent-test-session-1";
        assert_eq!(get(sid), None);
        set(sid, "  delete the temp dir  ");
        assert_eq!(get(sid).as_deref(), Some("delete the temp dir"));
        clear(sid);
        assert_eq!(get(sid), None);
    }

    #[test]
    fn blank_intent_ignored() {
        let sid = "task-intent-test-session-2";
        set(sid, "   ");
        assert_eq!(get(sid), None);
    }

    #[test]
    fn guard_clears_on_drop() {
        let sid = "task-intent-test-session-3";
        {
            let _g = TaskIntentGuard::new(sid, "send me a summary");
            assert_eq!(get(sid).as_deref(), Some("send me a summary"));
        }
        assert_eq!(get(sid), None);
    }

    #[test]
    fn long_intent_truncated_on_utf8_boundary() {
        let sid = "task-intent-test-session-4";
        let long = "é".repeat(2000); // 4000 bytes
        set(sid, &long);
        let got = get(sid).unwrap();
        assert!(got.len() <= MAX_INTENT_BYTES);
        assert!(got.chars().all(|c| c == 'é')); // no broken char at the cut
        clear(sid);
    }
}
