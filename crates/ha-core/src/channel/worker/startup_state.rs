//! Sentinel state for `startup_watcher` — a tiny JSON file at
//! `~/.hope-agent/startup_state.json` recording the last time we
//! delivered a "back online" notice into each IM chat.
//!
//! Independent from `runtime.lock` on purpose: that file is the
//! advisory-lock body and must not carry business state. Writes go
//! through `platform::write_secure_file` (tmp + fsync + 0600 + rename)
//! so a crash mid-save leaves the previous state intact (cost: at worst
//! one duplicate notice on the next boot).

use std::collections::HashMap;
use std::io::Result as IoResult;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

const STATE_FILE: &str = "startup_state.json";
const SCHEMA_VERSION: u32 = 1;

/// Build the stable `last_notified` key for a (channel, account, chat,
/// thread) tuple. `thread_id == None` becomes an empty segment so a
/// thread-less chat and a thread on the same chat never collide.
pub fn conv_key(
    channel_id: &str,
    account_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
) -> String {
    format!(
        "{}|{}|{}|{}",
        channel_id,
        account_id,
        chat_id,
        thread_id.unwrap_or("")
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupState {
    #[serde(default = "default_schema")]
    pub schema_version: u32,
    /// Map `conv_key()` → timestamp of the last delivered notice.
    /// Cooldown checks compare `now - last` against
    /// `AppConfig.startup_notification.cooldown_secs`.
    #[serde(default)]
    pub last_notified: HashMap<String, DateTime<Utc>>,
}

fn default_schema() -> u32 {
    SCHEMA_VERSION
}

impl Default for StartupState {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            last_notified: HashMap::new(),
        }
    }
}

fn state_path() -> IoResult<PathBuf> {
    crate::paths::root_dir()
        .map(|d| d.join(STATE_FILE))
        .map_err(|e| std::io::Error::other(e.to_string()))
}

impl StartupState {
    /// Read the sentinel from disk; corrupt / missing files degrade to
    /// `Default` (the cost is at worst one redundant notice).
    pub fn load() -> Self {
        let Ok(path) = state_path() else {
            return Self::default();
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return Self::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    /// Atomic save via the shared `platform::write_secure_file` helper
    /// (tmp + fsync + 0600 + rename, all on the same filesystem).
    pub fn save(&self) -> IoResult<()> {
        let path = state_path()?;
        let body =
            serde_json::to_vec_pretty(self).map_err(|e| std::io::Error::other(e.to_string()))?;
        crate::platform::write_secure_file(&path, &body)
    }

    /// `true` when this chat was notified less than `cooldown_secs` ago
    /// and the call site should skip the current boot. Clock skew that
    /// puts `last` in the future also counts as active — better to skip
    /// than over-deliver.
    pub fn cooldown_active(&self, key: &str, now: DateTime<Utc>, cooldown_secs: u64) -> bool {
        let Some(last) = self.last_notified.get(key) else {
            return false;
        };
        let elapsed = now.signed_duration_since(*last);
        if elapsed.num_seconds() < 0 {
            return true;
        }
        (elapsed.num_seconds() as u64) < cooldown_secs
    }

    pub fn mark_notified(&mut self, key: String, now: DateTime<Utc>) {
        self.last_notified.insert(key, now);
    }

    /// Drop entries older than `retention_secs`. Called once per save
    /// so the file does not grow unbounded across years of operation.
    pub fn prune(&mut self, now: DateTime<Utc>, retention_secs: u64) {
        let cutoff = retention_secs as i64;
        self.last_notified.retain(|_, ts| {
            let age = now.signed_duration_since(*ts).num_seconds();
            age < cutoff
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn conv_key_distinguishes_thread_from_threadless() {
        let bare = conv_key("tg", "acc", "chat", None);
        let topic = conv_key("tg", "acc", "chat", Some("1"));
        assert_ne!(bare, topic);
    }

    #[test]
    fn cooldown_active_inside_window() {
        let mut s = StartupState::default();
        let now = Utc::now();
        s.mark_notified("k".to_string(), now);
        assert!(s.cooldown_active("k", now + Duration::seconds(60), 1800));
    }

    #[test]
    fn cooldown_inactive_beyond_window() {
        let mut s = StartupState::default();
        let now = Utc::now();
        s.mark_notified("k".to_string(), now);
        assert!(!s.cooldown_active("k", now + Duration::seconds(2000), 1800));
    }

    #[test]
    fn cooldown_inactive_for_unseen_key() {
        let s = StartupState::default();
        assert!(!s.cooldown_active("missing", Utc::now(), 1800));
    }

    #[test]
    fn cooldown_active_on_negative_skew() {
        // last > now — clock went backwards; treat as still cooling
        let mut s = StartupState::default();
        let now = Utc::now();
        s.mark_notified("k".to_string(), now + Duration::seconds(60));
        assert!(s.cooldown_active("k", now, 1800));
    }

    #[test]
    fn prune_drops_stale_entries() {
        let mut s = StartupState::default();
        let now = Utc::now();
        s.mark_notified("recent".to_string(), now - Duration::seconds(60));
        s.mark_notified("stale".to_string(), now - Duration::days(30));
        s.prune(now, 7 * 24 * 3600);
        assert!(s.last_notified.contains_key("recent"));
        assert!(!s.last_notified.contains_key("stale"));
    }

    #[test]
    fn roundtrip_preserves_entries() {
        let mut s = StartupState::default();
        let now = Utc::now();
        s.mark_notified(conv_key("tg", "acc1", "chat1", None), now);
        s.mark_notified(conv_key("lark", "acc2", "chat2", Some("thr")), now);
        let json = serde_json::to_string(&s).unwrap();
        let back: StartupState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.schema_version, SCHEMA_VERSION);
        assert_eq!(back.last_notified.len(), 2);
    }

    #[test]
    fn missing_fields_degrade_to_default() {
        let s: StartupState = serde_json::from_str("{}").unwrap();
        assert_eq!(s.schema_version, SCHEMA_VERSION);
        assert!(s.last_notified.is_empty());
    }
}
