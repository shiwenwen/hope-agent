//! Sprite / inspiration mode — a proactive, transient writing companion for the
//! knowledge-space chat panel.
//!
//! Unlike `memory::dreaming` / `knowledge::maintenance` (background app-idle
//! loops), the sprite reacts to the note the user is **currently editing**,
//! which only the frontend knows. So the flow is **frontend edit-idle trigger →
//! `kb_sprite_observe_cmd` → [`observe_and_maybe_speak`] (throttle + side_query +
//! emit) → `sprite:suggestion` event → transient bubble**. No cron loop / idle
//! ticker. We reuse dreaming/maintenance's *serial-lock + side_query* idioms,
//! not their schedulers.

pub mod config;
pub mod context;
pub mod types;

pub use config::{SpriteConfig, SpriteSenses, SpriteTriggers};
pub use types::{SpriteCategory, SpriteObserveParams, SpriteOutcome, SpriteSuggestion};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use crate::ttl_cache::TtlCache;

// ── Config service (owner plane: GUI + ha-settings) ─────────────────────

/// Current sprite config (clamped).
pub fn get_config() -> SpriteConfig {
    crate::config::cached_config().sprite.clamped()
}

/// Persist the sprite config; returns the clamped value saved.
pub fn set_config(cfg: SpriteConfig, source: &str) -> anyhow::Result<SpriteConfig> {
    let clamped = cfg.clamped();
    let to_save = clamped.clone();
    crate::config::mutate_config(("sprite", source), move |store| {
        store.sprite = to_save.clone();
        Ok(())
    })?;
    Ok(clamped)
}

// ── Serial lock (only one observation at a time) ────────────────────────

static SPRITE_RUNNING: AtomicBool = AtomicBool::new(false);

struct RunningGuard;
impl Drop for RunningGuard {
    fn drop(&mut self) {
        SPRITE_RUNNING.store(false, Ordering::Release);
    }
}
fn try_claim() -> Option<RunningGuard> {
    SPRITE_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .ok()
        .map(|_| RunningGuard)
}

// ── Per-key throttle state (cooldown / hourly cap / doc-hash dedup) ──────

#[derive(Default, Clone)]
struct SpriteThrottle {
    last_call_secs: i64,
    last_doc_hash: u64,
    hour_window_start: i64,
    hour_count: u32,
}

/// Per-key (session id, else note path) throttle state. A [`TtlCache`] (capacity
/// 1024, time-evicted) bounds memory so editing many notes / opening many
/// knowledge sessions on a long-running instance can't grow the map without end.
fn throttle() -> &'static TtlCache<String, SpriteThrottle> {
    static THROTTLE: OnceLock<TtlCache<String, SpriteThrottle>> = OnceLock::new();
    THROTTLE.get_or_init(|| TtlCache::new(1024))
}

/// Entries older than this read back as fresh (and become evictable) — well past
/// the 1h window + max cooldown, so it never drops state that's still gating.
const THROTTLE_TTL: Duration = Duration::from_secs(2 * 3600);

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn hash_doc(doc: &str, edit: Option<&str>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    doc.hash(&mut h);
    // Fold the actual edit in: a tail edit beyond the doc-send cap leaves the
    // truncated `doc` prefix identical, so without this, appending past the cap
    // (the common "keep writing a long note" pattern) would be wrongly deduped
    // as "unchanged" and the sprite would go permanently silent on that note.
    if let Some(e) = edit {
        e.hash(&mut h);
    }
    h.finish()
}

/// Record an attempt **up-front** (before the LLM call) so the hourly cap +
/// doc-hash dedup gate the next observation even when this call fails or times
/// out — otherwise a flaky/slow provider would be hit every idle cycle with no
/// backoff. The cooldown anchor is refreshed to completion time by
/// [`touch_cooldown`] once the call returns.
fn mark_called(key: &str, now: i64, doc_hash: u64) {
    let cache = throttle();
    let mut t = cache.get(key, THROTTLE_TTL).unwrap_or_default();
    if now - t.hour_window_start >= 3600 {
        t.hour_window_start = now;
        t.hour_count = 0;
    }
    t.hour_count = t.hour_count.saturating_add(1);
    t.last_doc_hash = doc_hash;
    t.last_call_secs = now;
    cache.put(key.to_string(), t);
}

/// Refresh the cooldown anchor to *now* (call completion) so a long side_query
/// doesn't let the cooldown window elapse during its own run.
fn touch_cooldown(key: &str) {
    let cache = throttle();
    let mut t = cache.get(key, THROTTLE_TTL).unwrap_or_default();
    t.last_call_secs = now_secs();
    cache.put(key.to_string(), t);
}

// ── Sense gathering (sync — called via spawn_blocking) ───────────────────

fn gather_memory(params: &SpriteObserveParams) -> Vec<String> {
    let query = params
        .recent_edit
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| crate::truncate_utf8(params.doc_content.trim(), 200).to_string());
    if query.is_empty() {
        return Vec::new();
    }
    let shared_global = crate::agent_loader::load_agent(&params.agent_id)
        .map(|d| d.config.memory.shared)
        .unwrap_or(true);
    let mut scopes = vec![crate::memory::MemoryScope::Agent {
        id: params.agent_id.clone(),
    }];
    if shared_global {
        scopes.push(crate::memory::MemoryScope::Global);
    }
    crate::agent::active_memory::shortlist_candidates(&query, &scopes, 5)
        .iter()
        .map(|m| {
            let content = crate::truncate_utf8(&m.content, 300);
            if m.tags.is_empty() {
                content.to_string()
            } else {
                format!("{} [tags: {}]", content, m.tags.join(","))
            }
        })
        .collect()
}

fn gather_awareness(session_id: Option<&str>, agent_id: &str) -> Vec<String> {
    let Some(db) = crate::get_session_db() else {
        return Vec::new();
    };
    let cfg = crate::config::cached_config().awareness.clone();
    let sid = session_id.unwrap_or("");
    let snap = match crate::awareness::collect::collect_entries(db, &cfg, sid, Some(agent_id)) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    snap.entries
        .iter()
        .take(6)
        .map(|e| {
            let mut line = format!("- {}", e.title);
            if let Some(sum) = e.brief_summary.as_deref().filter(|s| !s.is_empty()) {
                line.push_str(&format!("：{}", crate::truncate_utf8(sum, 160)));
            } else if let Some(goal) = e.underlying_goal.as_deref().filter(|s| !s.is_empty()) {
                line.push_str(&format!("（目标：{}）", crate::truncate_utf8(goal, 160)));
            }
            line
        })
        .collect()
}

fn parse_suggestion(raw: &str) -> Option<SpriteSuggestion> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end <= start {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&raw[start..=end]).ok()?;
    let cat = v
        .get("category")
        .and_then(|c| c.as_str())
        .unwrap_or("none")
        .trim();
    if cat.is_empty() || cat.eq_ignore_ascii_case("none") {
        return None;
    }
    let text = v
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return None;
    }
    Some(SpriteSuggestion {
        category: SpriteCategory::from_wire(cat),
        text,
    })
}

/// Emit the transient "casting" signal so the UI can show the sprite cat
/// actively working (a distinct glow) only while the LLM call is in flight —
/// fired after the throttle gates pass, cleared when the call returns. UI-only
/// (ids + bool, no KB content); the frontend filters by note/session.
fn emit_casting(params: &SpriteObserveParams, active: bool) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "sprite:casting",
            serde_json::json!({
                "sessionId": params.session_id,
                "kbId": params.kb_id,
                "notePath": params.note_path,
                "active": active,
            }),
        );
    }
}

/// Observe the current editing context and, if there's something worth saying
/// (and throttle gates allow), run a bounded side_query and emit a
/// `sprite:suggestion` event. Returns an outcome for logging only.
pub async fn observe_and_maybe_speak(params: SpriteObserveParams) -> SpriteOutcome {
    let cfg = crate::config::cached_config().sprite.clamped();
    if !cfg.enabled {
        crate::app_debug!("sprite", "observe", "skip: disabled");
        return SpriteOutcome::Skipped("disabled");
    }

    // incognito zero-sprite (defensive — knowledge sessions aren't incognito).
    if crate::session::is_session_incognito(params.session_id.as_deref()) {
        return SpriteOutcome::Silent;
    }

    // Serial lock — at most one observation in flight; held across the LLM call.
    let _guard = match try_claim() {
        Some(g) => g,
        None => return SpriteOutcome::Skipped("running"),
    };

    let key = params
        .session_id
        .clone()
        .unwrap_or_else(|| params.note_path.clone());
    let now = now_secs();
    let doc_hash = hash_doc(&params.doc_content, params.recent_edit.as_deref());

    // Cheap throttle gates (no LLM): hourly cap, cooldown, doc-hash dedup. Pure
    // read — the authoritative write happens in `mark_called` below. Safe to
    // read-then-write because the SPRITE_RUNNING serial lock means no other
    // observation can interleave.
    {
        let t = throttle().get(&key, THROTTLE_TTL).unwrap_or_default();
        let hour_count = if now - t.hour_window_start < 3600 {
            t.hour_count
        } else {
            0
        };
        if hour_count >= cfg.max_per_session_per_hour {
            crate::app_debug!(
                "sprite",
                "observe",
                "skip: hourly cap ({}) reached",
                hour_count
            );
            return SpriteOutcome::Skipped("rate");
        }
        if t.last_call_secs != 0 && now - t.last_call_secs < cfg.cooldown_secs as i64 {
            crate::app_debug!(
                "sprite",
                "observe",
                "skip: cooldown ({}s left)",
                cfg.cooldown_secs as i64 - (now - t.last_call_secs)
            );
            return SpriteOutcome::Skipped("cooldown");
        }
        if t.last_doc_hash != 0 && t.last_doc_hash == doc_hash {
            crate::app_debug!("sprite", "observe", "skip: doc unchanged since last");
            return SpriteOutcome::Skipped("unchanged");
        }
    }

    // Passed the gates and committed to (attempt) an LLM call — record it now so
    // the cap + dedup gate the next observation regardless of success/failure.
    mark_called(&key, now, doc_hash);

    // Gather senses (blocking SQLite / disk → spawn_blocking).
    let (memory_lines, awareness_lines) = {
        let p = params.clone();
        let want_mem = cfg.senses.memory;
        let want_aware = cfg.senses.awareness;
        tokio::task::spawn_blocking(move || {
            let mem = if want_mem {
                gather_memory(&p)
            } else {
                Vec::new()
            };
            let aware = if want_aware {
                gather_awareness(p.session_id.as_deref(), &p.agent_id)
            } else {
                Vec::new()
            };
            (mem, aware)
        })
        .await
        .unwrap_or_default()
    };

    let conversation: Vec<(String, String)> = params
        .recent_messages
        .as_ref()
        .map(|msgs| {
            msgs.iter()
                .map(|m| (m.role.clone(), m.text.clone()))
                .collect()
        })
        .unwrap_or_default();

    let instruction = context::build_instruction(
        &cfg,
        &params,
        &conversation,
        &memory_lines,
        &awareness_lines,
    );

    let app_cfg = crate::config::cached_config();
    let chain = crate::automation::effective_chain(&app_cfg, cfg.model_override.clone());

    // Cat "casting" glow brackets the WHOLE degradation loop (all candidates),
    // not one call — a failover attempt sequence should still read as one
    // continuous "the cat is thinking" glow, not one flicker per attempt.
    // `cfg.timeout_secs` now bounds the combined budget across every
    // candidate, not a single attempt — same reinterpretation Compile/
    // Maintenance already apply to their own timeout around the whole
    // `automation::run` call.
    emit_casting(&params, true);
    let outcome = tokio::time::timeout(
        Duration::from_secs(cfg.timeout_secs),
        crate::automation::run(crate::automation::ModelTaskSpec {
            purpose: "sprite.observe",
            chain,
            session_key: &key,
            instruction: &instruction,
            max_tokens: cfg.max_tokens,
        }),
    )
    .await;
    emit_casting(&params, false);

    // Anchor the cooldown to *completion* time so a long side_query doesn't let
    // the cooldown elapse during its own run (applies to success + failure).
    touch_cooldown(&key);

    let res = match outcome {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            crate::app_warn!("sprite", "observe", "side_query failed: {}", e);
            return SpriteOutcome::Skipped("llm");
        }
        Err(_) => {
            crate::app_warn!("sprite", "observe", "side_query timed out");
            return SpriteOutcome::Skipped("timeout");
        }
    };

    let Some(suggestion) = parse_suggestion(&res.text) else {
        crate::app_debug!("sprite", "observe", "silent: model returned none/empty");
        return SpriteOutcome::Silent;
    };

    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "sprite:suggestion",
            serde_json::json!({
                "sessionId": params.session_id,
                "kbId": params.kb_id,
                "notePath": params.note_path,
                "category": suggestion.category,
                "text": suggestion.text,
            }),
        );
    }
    crate::app_info!("sprite", "observe", "spoke ({:?})", suggestion.category);
    SpriteOutcome::Spoke(suggestion)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_clamps_out_of_range() {
        let c = SpriteConfig {
            idle_edit_secs: 1,
            min_change_chars: 5,
            cooldown_secs: 1,
            max_per_session_per_hour: 999,
            periodic_secs: 1,
            paste_min_chars: 1,
            max_tokens: 10,
            timeout_secs: 999,
            ..Default::default()
        }
        .clamped();
        assert_eq!(c.idle_edit_secs, 3);
        assert_eq!(c.min_change_chars, 20);
        assert_eq!(c.cooldown_secs, 10);
        assert_eq!(c.max_per_session_per_hour, 60);
        assert_eq!(c.periodic_secs, 15);
        assert_eq!(c.paste_min_chars, 40);
        assert_eq!(c.max_tokens, 64);
        assert_eq!(c.timeout_secs, 60);
    }

    #[test]
    fn hash_doc_folds_in_edit() {
        // Identical (truncated) doc prefix but a different tail edit must hash
        // differently, else a tail-append past the doc-send cap is wrongly
        // deduped as "unchanged" and the sprite goes silent on long notes.
        assert_ne!(
            hash_doc("same prefix", Some("tail one")),
            hash_doc("same prefix", Some("tail two"))
        );
        assert_ne!(hash_doc("doc", None), hash_doc("doc", Some("x")));
    }

    #[test]
    fn parse_none_and_empty_are_silent() {
        assert!(parse_suggestion("{\"category\":\"none\"}").is_none());
        assert!(parse_suggestion("garbage no json").is_none());
        assert!(parse_suggestion("{\"category\":\"writing\",\"text\":\"\"}").is_none());
    }

    #[test]
    fn parse_extracts_category_and_text() {
        let s =
            parse_suggestion("noise {\"category\":\"review\",\"text\":\"写得不错\"} tail").unwrap();
        assert_eq!(s.category, SpriteCategory::Review);
        assert_eq!(s.text, "写得不错");
    }

    #[test]
    fn unknown_category_defaults_to_writing() {
        let s = parse_suggestion("{\"category\":\"wat\",\"text\":\"hi\"}").unwrap();
        assert_eq!(s.category, SpriteCategory::Writing);
    }
}
