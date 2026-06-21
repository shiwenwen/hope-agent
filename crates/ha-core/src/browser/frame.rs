//! BrowserPanel live mirror — frame capture + `browser:frame` event.
//!
//! The chat-side BrowserPanel ([`src/components/chat/BrowserPanel.tsx`]) keeps
//! the user visually in the loop by mirroring the active tab. It uses two
//! signals:
//!
//! 1. **Event-driven** — the 8-action handlers call [`emit_frame_async`] after
//!    every `act` / `navigate` / `tabs.new|select` so the panel updates within
//!    one tick of an LLM-driven action.
//! 2. **1s fallback polling** — when the panel is open it calls the Tauri /
//!    HTTP `browser_capture_frame` command on a 1-second interval, catching
//!    user-initiated changes (e.g. they clicked something in the Chrome
//!    window themselves) without paying steady CPU cost when the panel is
//!    closed.
//!
//! Frames are JPEG quality≈70 to keep the per-frame payload to ~50–200KB.
//! Capture goes through the active browser backend trait, which is currently
//! the direct CDP backend.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::backend::{ImageFormat, ScreenshotParams};
use super::backend_select::peek_active;

/// Event name emitted to the EventBus when a fresh browser frame is captured.
/// Subscribed to by `src/components/chat/BrowserPanel.tsx` via the Transport
/// listener API (Tauri `listen` / HTTP WebSocket).
pub const EVENT_BROWSER_FRAME: &str = "browser:frame";

/// Payload emitted alongside [`EVENT_BROWSER_FRAME`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserFramePayload {
    /// Active tab `target_id`.
    pub target_id: Option<String>,
    /// Active tab URL at capture time.
    pub url: Option<String>,
    /// Active tab title at capture time.
    pub title: Option<String>,
    /// JPEG bytes base64-encoded.
    pub jpeg_base64: String,
    /// Unix-millis capture timestamp.
    pub captured_at: i64,
    /// Backend identifier (always `"cdp"` since the MCP backend was removed;
    /// kept on the wire for forward compatibility / front-end badge code
    /// that switches on this string).
    pub backend: String,
}

/// Capture a JPEG frame from the active backend. Returns `Ok(None)` when no
/// backend is active (we don't want to force-launch Chrome just to take a
/// frame — the panel will show its empty state instead).
pub async fn capture_frame() -> Result<Option<BrowserFramePayload>> {
    // Uses peek_active (the cached backend), not status_backend: a live UI frame
    // needs the session's active tab, but the per-session ExtensionBackend is
    // never cached. So in extension-only mode this returns None (no panel frame)
    // — a known limitation; `status` still reports tabs via status_backend.
    let Some(backend) = peek_active().await else {
        return Ok(None);
    };
    if !backend.is_connected().await {
        return Ok(None);
    }

    // Fast-path metadata fetch — avoids the per-tab `evaluate("document.title")`
    // round-trip that `status()` does for every tab.
    let (target_id, url, title) = match backend.active_tab_info().await.ok().flatten() {
        Some(t) => (Some(t.target_id), Some(t.url), Some(t.title)),
        None => (None, None, None),
    };

    let bytes = backend
        .take_screenshot(ScreenshotParams {
            format: ImageFormat::Jpeg,
            full_page: false,
            quality: Some(70),
            ref_id: None,
        })
        .await?;
    let jpeg_base64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);

    Ok(Some(BrowserFramePayload {
        target_id,
        url,
        title,
        jpeg_base64,
        captured_at: chrono::Utc::now().timestamp_millis(),
        backend: backend.backend_name().to_string(),
    }))
}

/// Fire-and-forget: capture a frame in a background task and emit it on the
/// EventBus. Safe to call from synchronous-feeling 8-action handlers — never
/// blocks the action's return path.
///
/// Errors (no backend / capture failed) are logged at warn level but never
/// surface to the caller: the panel will pick up the next opportunity via
/// the 1s fallback poll.
pub fn emit_frame_async() {
    crate::browser_state::browser_runtime().spawn(async move {
        match capture_frame().await {
            Ok(Some(payload)) => {
                if let Some(bus) = crate::globals::get_event_bus() {
                    match serde_json::to_value(&payload) {
                        Ok(value) => bus.emit(EVENT_BROWSER_FRAME, value),
                        Err(e) => app_warn!(
                            "browser",
                            "frame",
                            "Failed to serialize BrowserFramePayload: {}",
                            e
                        ),
                    }
                }
            }
            Ok(None) => {
                // No active backend — silently skip (this is normal).
            }
            Err(e) => {
                app_warn!("browser", "frame", "capture_frame failed: {}", e);
            }
        }
    });
}
