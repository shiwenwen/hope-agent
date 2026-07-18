//! Step-level action event stream for the browser / mac control side panels.
//!
//! The frame events (`browser:frame` / `mac_control:frame`) only carry a
//! screenshot snapshot — they say nothing about *what* the agent just did.
//! This module records one [`ToolActionEvent`] per meaningful tool step
//! (click / type / navigate / window focus / …) so the panels can render an
//! execution timeline, and keeps a bounded in-memory history per session for
//! panel re-open / reload.
//!
//! Contracts:
//! - **Memory only, never persisted** — incognito sessions record like any
//!   other (nothing touches disk); [`purge_for_session`] wipes the buffer on
//!   session delete/purge via the session cleanup watcher.
//! - **Redaction** — typed/filled/pasted text never enters a payload; callers
//!   summarize it via [`redacted_text_summary`] (length only, no prefix).
//! - Thumbnails are attached asynchronously after the follow-up frame capture
//!   and are capped to the newest [`MAX_THUMBNAILS_PER_SESSION`] records so a
//!   full ring stays well under ~1MB per session.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

/// Emitted after each recordable browser tool step.
pub const EVENT_BROWSER_ACTION: &str = "browser:action";
/// Emitted after each recordable mac control tool step.
pub const EVENT_MAC_CONTROL_ACTION: &str = "mac_control:action";

const MAX_RECORDS_PER_SESSION: usize = 200;
const MAX_THUMBNAILS_PER_SESSION: usize = 50;
const MAX_SESSION_KEYS: usize = 64;
const MAX_ERROR_BYTES: usize = 256;
/// Bucket for actions that carry no session id (e.g. mac control outside a chat turn).
const GLOBAL_KEY: &str = "__global__";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolActionSource {
    Browser,
    MacControl,
}

impl ToolActionSource {
    pub fn event_name(self) -> &'static str {
        match self {
            ToolActionSource::Browser => EVENT_BROWSER_ACTION,
            ToolActionSource::MacControl => EVENT_MAC_CONTROL_ACTION,
        }
    }

    /// Lenient wire-format parser for the owner query surface.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "browser" => Some(ToolActionSource::Browser),
            "mac_control" | "mac-control" | "macControl" => Some(ToolActionSource::MacControl),
            _ => None,
        }
    }
}

/// One recorded tool step. Small by contract (<2KB) — frames/thumbnails are
/// never inlined in the event; the follow-up frame references `action_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolActionEvent {
    /// `act_<uuid>` — foreign key stamped onto the follow-up frame payload.
    pub action_id: String,
    pub source: ToolActionSource,
    pub session_id: Option<String>,
    /// Top-level tool action (`navigate` / `act` / `tabs` / `windows` / …).
    pub action: String,
    /// Sub-operation (`click` / `fill` / `go` / `focus` / …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    /// Human-oriented target description (element label / coordinates / target id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Redacted parameter summary (`"text(12 chars)"` / `"key=cmd+c"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Browser only: navigation / current page URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Mac control only: frontmost app name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub duration_ms: u64,
    /// Unix millis at step start.
    pub started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Whether this step triggered a follow-up frame capture.
    pub has_frame: bool,
}

/// Ring-buffer record: the event plus a lazily backfilled thumbnail.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolActionRecord {
    #[serde(flatten)]
    pub event: ToolActionEvent,
    /// ≤240px-wide JPEG (quality 60), backfilled by the frame capture task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb_jpeg_base64: Option<String>,
}

pub fn new_action_id() -> String {
    format!("act_{}", uuid::Uuid::new_v4().simple())
}

/// Redaction contract for typed text: length only — a prefix preview would
/// leak password prefixes just the same.
pub fn redacted_text_summary(text: &str) -> String {
    format!("text({} chars)", text.chars().count())
}

/// Truncate an error message for the wire (UTF-8 safe).
pub fn clamp_error(error: &str) -> String {
    crate::truncate_utf8(error, MAX_ERROR_BYTES).to_string()
}

struct SessionRing {
    records: VecDeque<ToolActionRecord>,
    /// Monotonic touch counter for LRU eviction.
    last_used: u64,
}

struct Store {
    rings: HashMap<String, SessionRing>,
    clock: u64,
}

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();

fn store() -> &'static Mutex<Store> {
    STORE.get_or_init(|| {
        Mutex::new(Store {
            rings: HashMap::new(),
            clock: 0,
        })
    })
}

fn session_key(session_id: Option<&str>) -> String {
    match session_id {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => GLOBAL_KEY.to_string(),
    }
}

/// Push a record into the per-session ring and emit the matching EventBus
/// event. The emitted payload is the bare event (no thumbnail).
pub fn record_action(event: ToolActionEvent) {
    let key = session_key(event.session_id.as_deref());
    {
        let Ok(mut store) = store().lock() else {
            return;
        };
        store.clock += 1;
        let clock = store.clock;
        if !store.rings.contains_key(&key) && store.rings.len() >= MAX_SESSION_KEYS {
            evict_lru(&mut store.rings);
        }
        let ring = store.rings.entry(key).or_insert_with(|| SessionRing {
            records: VecDeque::with_capacity(32),
            last_used: clock,
        });
        ring.last_used = clock;
        if ring.records.len() >= MAX_RECORDS_PER_SESSION {
            ring.records.pop_front();
        }
        ring.records.push_back(ToolActionRecord {
            event: event.clone(),
            thumb_jpeg_base64: None,
        });
        trim_thumbnails(&mut ring.records);
    }

    if let Some(bus) = crate::globals::get_event_bus() {
        match serde_json::to_value(&event) {
            Ok(value) => bus.emit(event.source.event_name(), value),
            Err(e) => app_warn!(
                "tool_actions",
                "record",
                "Failed to serialize ToolActionEvent: {}",
                e
            ),
        }
    }
}

fn evict_lru(rings: &mut HashMap<String, SessionRing>) {
    if let Some(key) = rings
        .iter()
        .min_by_key(|(_, ring)| ring.last_used)
        .map(|(k, _)| k.clone())
    {
        rings.remove(&key);
    }
}

fn trim_thumbnails(records: &mut VecDeque<ToolActionRecord>) {
    let len = records.len();
    if len <= MAX_THUMBNAILS_PER_SESSION {
        return;
    }
    let cutoff = len - MAX_THUMBNAILS_PER_SESSION;
    for record in records.iter_mut().take(cutoff) {
        record.thumb_jpeg_base64 = None;
    }
}

/// Backfill the thumbnail for a recorded action (frame capture task calls this
/// after emitting the full frame). Marks `has_frame` since a frame did land.
pub fn attach_thumbnail(session_id: Option<&str>, action_id: &str, thumb_jpeg_base64: String) {
    let key = session_key(session_id);
    let Ok(mut store) = store().lock() else {
        return;
    };
    let Some(ring) = store.rings.get_mut(&key) else {
        return;
    };
    if let Some(record) = ring
        .records
        .iter_mut()
        .rev()
        .find(|r| r.event.action_id == action_id)
    {
        record.event.has_frame = true;
        record.thumb_jpeg_base64 = Some(thumb_jpeg_base64);
        trim_thumbnails(&mut ring.records);
    }
}

/// Snapshot recent actions, oldest first. `session_id = Some` reads that
/// session's ring merged with the global bucket; `None` merges every ring
/// (mac control panel is not session-scoped).
pub fn recent(
    source: Option<ToolActionSource>,
    session_id: Option<&str>,
    limit: usize,
) -> Vec<ToolActionRecord> {
    let limit = limit.clamp(1, MAX_RECORDS_PER_SESSION);
    let Ok(store) = store().lock() else {
        return Vec::new();
    };
    let mut out: Vec<ToolActionRecord> = Vec::new();
    match session_id.filter(|s| !s.is_empty()) {
        Some(sid) => {
            for key in [sid, GLOBAL_KEY] {
                if let Some(ring) = store.rings.get(key) {
                    out.extend(ring.records.iter().cloned());
                }
            }
        }
        None => {
            for ring in store.rings.values() {
                out.extend(ring.records.iter().cloned());
            }
        }
    }
    if let Some(source) = source {
        out.retain(|r| r.event.source == source);
    }
    out.sort_by_key(|r| (r.event.started_at, r.event.action_id.clone()));
    if out.len() > limit {
        out.drain(..out.len() - limit);
    }
    out
}

/// Drop a session's action history — called from the session cleanup watcher
/// for both delete and purge (incognito burn) paths.
pub fn purge_for_session(session_id: &str) {
    if session_id.is_empty() {
        return;
    }
    if let Ok(mut store) = store().lock() {
        store.rings.remove(session_id);
    }
}

/// Downscale a full JPEG frame to a ≤240px-wide JPEG (quality 60) thumbnail,
/// base64-encoded. Returns `None` on decode/encode failure — thumbnails are
/// best-effort.
pub fn encode_thumbnail_from_jpeg(jpeg_bytes: &[u8]) -> Option<String> {
    const THUMB_MAX_WIDTH: u32 = 240;
    const THUMB_JPEG_QUALITY: u8 = 60;
    let img = image::load_from_memory_with_format(jpeg_bytes, image::ImageFormat::Jpeg).ok()?;
    let thumb = if img.width() > THUMB_MAX_WIDTH {
        let height =
            ((THUMB_MAX_WIDTH as u64 * img.height() as u64) / img.width() as u64).max(1) as u32;
        img.thumbnail(THUMB_MAX_WIDTH, height)
    } else {
        img
    };
    let mut buf = std::io::Cursor::new(Vec::new());
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, THUMB_JPEG_QUALITY);
    thumb.write_with_encoder(encoder).ok()?;
    Some(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        buf.into_inner(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests share the process-global store — serialize them.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn make_event(source: ToolActionSource, session: Option<&str>, at: i64) -> ToolActionEvent {
        ToolActionEvent {
            action_id: new_action_id(),
            source,
            session_id: session.map(str::to_string),
            action: "act".to_string(),
            op: Some("click".to_string()),
            target: None,
            detail: None,
            url: None,
            app: None,
            ok: true,
            error: None,
            duration_ms: 5,
            started_at: at,
            tool_call_id: None,
            has_frame: false,
        }
    }

    fn reset_store() {
        if let Ok(mut store) = store().lock() {
            store.rings.clear();
            store.clock = 0;
        }
    }

    #[test]
    fn ring_capacity_and_thumbnail_trim() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_store();
        let session = "cap-session";
        let mut ids = Vec::new();
        for i in 0..(MAX_RECORDS_PER_SESSION + 20) {
            let ev = make_event(ToolActionSource::Browser, Some(session), i as i64);
            ids.push(ev.action_id.clone());
            record_action(ev);
        }
        // Attach thumbnails to everything still in the ring.
        for id in &ids {
            attach_thumbnail(Some(session), id, "dGh1bWI=".to_string());
        }
        let records = recent(None, Some(session), MAX_RECORDS_PER_SESSION);
        assert_eq!(records.len(), MAX_RECORDS_PER_SESSION);
        // Oldest 20 were evicted entirely.
        assert!(!records.iter().any(|r| r.event.action_id == ids[0]));
        let with_thumbs = records
            .iter()
            .filter(|r| r.thumb_jpeg_base64.is_some())
            .count();
        assert_eq!(with_thumbs, MAX_THUMBNAILS_PER_SESSION);
        // The trimmed ones are the oldest.
        assert!(records[0].thumb_jpeg_base64.is_none());
        assert!(records.last().unwrap().thumb_jpeg_base64.is_some());
        reset_store();
    }

    #[test]
    fn lru_eviction_caps_session_keys() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_store();
        for i in 0..(MAX_SESSION_KEYS + 5) {
            let session = format!("lru-{i}");
            record_action(make_event(
                ToolActionSource::Browser,
                Some(&session),
                i as i64,
            ));
        }
        // Oldest sessions evicted; newest retained.
        assert!(recent(None, Some("lru-0"), 10).is_empty());
        assert_eq!(
            recent(None, Some(&format!("lru-{}", MAX_SESSION_KEYS + 4)), 10).len(),
            1
        );
        let store_len = store().lock().unwrap().rings.len();
        assert!(store_len <= MAX_SESSION_KEYS);
        reset_store();
    }

    #[test]
    fn purge_drops_session_history() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_store();
        record_action(make_event(
            ToolActionSource::MacControl,
            Some("purge-me"),
            1,
        ));
        assert_eq!(recent(None, Some("purge-me"), 10).len(), 1);
        purge_for_session("purge-me");
        assert!(recent(None, Some("purge-me"), 10).is_empty());
        reset_store();
    }

    #[test]
    fn recent_filters_by_source_and_merges_global() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_store();
        record_action(make_event(ToolActionSource::Browser, Some("mix"), 1));
        record_action(make_event(ToolActionSource::MacControl, None, 2));
        let browser_only = recent(Some(ToolActionSource::Browser), Some("mix"), 10);
        assert_eq!(browser_only.len(), 1);
        // Session query merges the global bucket.
        let all = recent(None, Some("mix"), 10);
        assert_eq!(all.len(), 2);
        assert!(all[0].event.started_at <= all[1].event.started_at);
        reset_store();
    }

    #[test]
    fn redaction_and_error_clamp() {
        assert_eq!(redacted_text_summary("hunter2!"), "text(8 chars)");
        assert_eq!(redacted_text_summary("密码123"), "text(5 chars)");
        let long = "e".repeat(1000);
        assert!(clamp_error(&long).len() <= MAX_ERROR_BYTES);
    }
}
