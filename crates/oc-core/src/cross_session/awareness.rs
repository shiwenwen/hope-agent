//! `SessionAwareness` — per-session holder for the dynamic cross-session
//! suffix. Owned by `AssistantAgent`, refreshed on every user turn.
//!
//! Three-layer refresh trigger:
//!   L1: dirty bit — another session pinged us
//!   L2: time window — cheap tick-based refresh (min_refresh_secs)
//!   L3: semantic hint — the user's current message mentions "other session",
//!       "last time", etc.
//!
//! Rebuild short-circuits when the rendered markdown is byte-identical to the
//! last one — we reuse the same `Arc<String>` so cache-friendly providers see
//! an unchanged suffix.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use once_cell::sync::Lazy;
use regex::Regex;

use super::config::{CrossSessionConfig, CrossSessionMode};
use super::types::{CrossSessionSnapshot, RefreshReason};
use crate::session::SessionDB;

/// Dynamic cross-session suffix holder attached to `AssistantAgent`.
pub struct SessionAwareness {
    pub session_id: String,
    pub cfg: Mutex<CrossSessionConfig>,
    last_suffix: Mutex<Option<Arc<String>>>,
    last_suffix_hash: AtomicU64,
    last_refresh_at: Mutex<Option<Instant>>,
    forced_next: AtomicBool,
    // ── LLM digest state ─────
    last_digest: Mutex<Option<Arc<String>>>,
    last_digest_at: Mutex<Option<Instant>>,
    digest_inflight: AtomicBool,
    digest_candidate_hash: AtomicU64,
    // ── Last snapshot used to render the suffix (also exposed to peek_tool) ──
    last_snapshot: Mutex<Option<CrossSessionSnapshot>>,
}

impl SessionAwareness {
    /// Create and register a new awareness instance.
    pub fn new(session_id: impl Into<String>, cfg: CrossSessionConfig) -> Arc<Self> {
        let session_id = session_id.into();
        super::dirty::register_observer(&session_id);
        Arc::new(Self {
            session_id,
            cfg: Mutex::new(cfg),
            last_suffix: Mutex::new(None),
            last_suffix_hash: AtomicU64::new(0),
            last_refresh_at: Mutex::new(None),
            forced_next: AtomicBool::new(false),
            last_digest: Mutex::new(None),
            last_digest_at: Mutex::new(None),
            digest_inflight: AtomicBool::new(false),
            digest_candidate_hash: AtomicU64::new(0),
            last_snapshot: Mutex::new(None),
        })
    }

    /// Hot-swap the resolved config (e.g., after user edits the session settings).
    pub fn update_config(&self, new_cfg: CrossSessionConfig) {
        let mut guard = self.cfg.lock().unwrap_or_else(|e| e.into_inner());
        *guard = new_cfg;
        // Force next turn to rebuild regardless of throttle.
        self.forced_next.store(true, Ordering::Release);
    }

    /// Force rebuild on the next turn. Called from compaction to piggyback
    /// on the already-invalidated cache.
    pub fn mark_force_refresh(&self) {
        self.forced_next.store(true, Ordering::Release);
    }

    /// Produce the suffix string to append to the system prompt. Returns
    /// `None` when the feature is disabled or there's nothing worth saying.
    ///
    /// This is the hot path — keep it cheap when the result is cached.
    pub fn prepare_dynamic_suffix(
        self: &Arc<Self>,
        user_text: &str,
        session_db: &SessionDB,
    ) -> Option<Arc<String>> {
        let cfg = self.cfg.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if !cfg.enabled || matches!(cfg.mode, CrossSessionMode::Off) || !cfg.dynamic_enabled {
            return self.reuse_or_none();
        }

        let reason = self.decide_refresh(&cfg, user_text);
        match reason {
            RefreshReason::Cached | RefreshReason::None => {
                // Attach existing digest (if any) to the cached suffix path.
                return self.reuse_or_none();
            }
            _ => {}
        }

        // Rebuild.
        let snap = match super::collect::collect_entries(session_db, &cfg, &self.session_id) {
            Ok(s) => s,
            Err(e) => {
                app_warn!(
                    "cross_session",
                    "awareness::prepare_dynamic_suffix",
                    "collect_entries failed for {}: {}",
                    self.session_id,
                    e
                );
                return self.reuse_or_none();
            }
        };

        let digest_text = self
            .last_digest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        let suffix_str = super::render::render_markdown(&snap, cfg.max_chars)
            .map(|mut s| {
                if let Some(digest) = digest_text.as_ref() {
                    s.push_str("\n\n## AI Digest\n");
                    s.push_str(digest);
                }
                s
            });

        // Persist snapshot for peek_tool.
        *self.last_snapshot.lock().unwrap_or_else(|e| e.into_inner()) = Some(snap.clone());

        // Hash comparison — byte-identical output → reuse old Arc.
        let suffix_hash = match &suffix_str {
            Some(s) => hash_str(s),
            None => 0,
        };
        let prev_hash = self.last_suffix_hash.load(Ordering::Acquire);

        *self
            .last_refresh_at
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());

        if suffix_hash == prev_hash && prev_hash != 0 {
            // Nothing new — reuse.
            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "debug",
                    "cross_session",
                    "awareness::prepare_dynamic_suffix",
                    &format!(
                        "suffix unchanged for {}, reason={}",
                        self.session_id,
                        reason.as_str()
                    ),
                    None,
                    None,
                    None,
                );
            }
            return self.reuse_or_none();
        }

        self.last_suffix_hash.store(suffix_hash, Ordering::Release);

        let arc_suffix = suffix_str.map(Arc::new);
        {
            let mut slot = self.last_suffix.lock().unwrap_or_else(|e| e.into_inner());
            *slot = arc_suffix.clone();
        }
        if matches!(reason, RefreshReason::Forced) {
            self.forced_next.store(false, Ordering::Release);
        }
        if let Some(logger) = crate::get_logger() {
            let len = arc_suffix.as_ref().map(|s| s.len()).unwrap_or(0);
            logger.log(
                "debug",
                "cross_session",
                "awareness::prepare_dynamic_suffix",
                &format!(
                    "rebuilt suffix for {}, reason={}, len={}",
                    self.session_id,
                    reason.as_str(),
                    len
                ),
                None,
                None,
                None,
            );
        }
        arc_suffix
    }

    /// Return the most recently-rendered snapshot (for peek_tool / debugging).
    pub fn last_snapshot(&self) -> Option<CrossSessionSnapshot> {
        self.last_snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Write a fresh LLM digest (called from llm_digest.rs background task).
    pub fn set_last_digest(&self, text: Arc<String>) {
        *self.last_digest.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
        *self.last_digest_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        self.digest_inflight.store(false, Ordering::Release);
        // Trigger a rebuild next turn so the new digest lands in the suffix.
        self.mark_force_refresh();
    }

    /// Record an LLM extraction failure and cool down.
    pub fn record_digest_failure(&self) {
        *self.last_digest_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        self.digest_inflight.store(false, Ordering::Release);
    }

    /// Whether the LLM digest path should run this turn.
    pub fn should_run_extraction(&self) -> bool {
        let cfg = self.cfg.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if !cfg.enabled || !matches!(cfg.mode, CrossSessionMode::LlmDigest) {
            return false;
        }
        if self.digest_inflight.load(Ordering::Acquire) {
            return false;
        }
        let last = self
            .last_digest_at
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(last) = last {
            if last.elapsed().as_secs() < cfg.llm_extraction.min_interval_secs {
                return false;
            }
        }
        true
    }

    /// Claim exclusive rights to run extraction now. Returns true on success.
    pub fn claim_extraction(&self) -> bool {
        self.digest_inflight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Compute a stable hash of a candidate session set so we can skip
    /// extraction when the same batch has already been digested.
    pub fn update_candidate_hash(&self, candidate_ids: &[String]) -> bool {
        let mut hasher = DefaultHasher::new();
        let mut sorted: Vec<&String> = candidate_ids.iter().collect();
        sorted.sort();
        sorted.hash(&mut hasher);
        let new_hash = hasher.finish();
        let prev = self.digest_candidate_hash.swap(new_hash, Ordering::AcqRel);
        prev != new_hash
    }

    /// Whether the current suffix already mentions an AI digest. Used by the
    /// extractor to decide whether to rerun even when candidates haven't changed.
    pub fn has_digest(&self) -> bool {
        self.last_digest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    /// Present the existing suffix as an Arc. Used when skipping rebuild.
    fn reuse_or_none(&self) -> Option<Arc<String>> {
        self.last_suffix
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn decide_refresh(&self, cfg: &CrossSessionConfig, user_text: &str) -> RefreshReason {
        if self.forced_next.load(Ordering::Acquire) {
            return RefreshReason::Forced;
        }
        // L3: semantic hint wins even within throttle window.
        if !user_text.is_empty() && matches_semantic_hint(&cfg.semantic_hint_regex, user_text) {
            return RefreshReason::SemanticHint;
        }
        // Throttle check.
        let within_throttle = match *self
            .last_refresh_at
            .lock()
            .unwrap_or_else(|e| e.into_inner())
        {
            Some(t) => t.elapsed().as_secs() < cfg.min_refresh_secs,
            None => false,
        };
        // L1: dirty bit. Only consume when we're actually going to refresh;
        // otherwise leave it set so the next turn can pick it up.
        if !within_throttle && super::dirty::take_dirty(&self.session_id) {
            return RefreshReason::DirtyBit;
        }
        // L2: time window — first turn or throttle elapsed.
        let last = *self
            .last_refresh_at
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        match last {
            None => RefreshReason::TimeWindow,
            Some(t) if t.elapsed().as_secs() >= cfg.min_refresh_secs => RefreshReason::TimeWindow,
            _ => RefreshReason::Cached,
        }
    }
}

impl Drop for SessionAwareness {
    fn drop(&mut self) {
        super::dirty::unregister_observer(&self.session_id);
    }
}

// ── Cached regex ────────────────────────────────────────────────

static SEMANTIC_HINT_CACHE: Lazy<Mutex<Option<(String, Regex)>>> = Lazy::new(|| Mutex::new(None));

fn matches_semantic_hint(pattern: &str, text: &str) -> bool {
    let mut guard = SEMANTIC_HINT_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let hit = match guard.as_ref() {
        Some((p, re)) if p == pattern => Some(re.clone()),
        _ => None,
    };
    let re = match hit {
        Some(re) => re,
        None => match Regex::new(pattern) {
            Ok(re) => {
                *guard = Some((pattern.to_string(), re.clone()));
                re
            }
            Err(e) => {
                app_warn!(
                    "cross_session",
                    "awareness::matches_semantic_hint",
                    "invalid semantic_hint_regex: {}",
                    e
                );
                return false;
            }
        },
    };
    drop(guard);
    re.is_match(text)
}

fn hash_str(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_hint_detects_chinese_phrases() {
        let re = super::super::config::CrossSessionConfig::default().semantic_hint_regex;
        assert!(matches_semantic_hint(&re, "我上次说的那个 bug"));
        assert!(matches_semantic_hint(&re, "last time we discussed this"));
        assert!(!matches_semantic_hint(&re, "hello world"));
    }

    #[test]
    fn candidate_hash_stable() {
        let a = SessionAwareness::new("sess-ct-1", CrossSessionConfig::default());
        assert!(a.update_candidate_hash(&["x".into(), "y".into()]));
        assert!(!a.update_candidate_hash(&["y".into(), "x".into()])); // sorted
        assert!(a.update_candidate_hash(&["x".into(), "z".into()]));
    }

    #[test]
    fn claim_extraction_is_one_shot() {
        let a = SessionAwareness::new("sess-ct-2", CrossSessionConfig::default());
        assert!(a.claim_extraction());
        assert!(!a.claim_extraction());
    }
}
