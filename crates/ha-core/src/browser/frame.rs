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
//! Frames are JPEG quality≈70 to keep the per-frame payload to ~50-200KB.
//! Session-scoped captures prefer the extension backend so the panel mirrors
//! the user's claimed Chrome tab; if they fall back to the cached CDP backend,
//! the frame stays tagged with the requesting session so other chat panels can
//! ignore it. Only legacy/global captures have no session id.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use std::sync::Arc;

use super::backend::{BrowserBackend, ImageFormat, ScreenshotParams};
use super::backend_select::peek_active;
use super::extension::{BrowserBackendContext, BrowserExtensionBroker, ExtensionBackend};
use super::BrowserBackendPreference;

/// Event name emitted to the EventBus when a fresh browser frame is captured.
/// Subscribed to by `src/components/chat/BrowserPanel.tsx` via the Transport
/// listener API (Tauri `listen` / HTTP WebSocket).
pub const EVENT_BROWSER_FRAME: &str = "browser:frame";

/// Payload emitted alongside [`EVENT_BROWSER_FRAME`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserFramePayload {
    /// Owning Hope session when the frame was captured for a session-scoped
    /// extension backend. `None` for legacy/global CDP captures.
    pub session_id: Option<String>,
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
    /// Backend identifier (`"extension"` or `"cdp"`), kept on the wire for
    /// front-end badge code and mixed-backend diagnostics.
    pub backend: String,
    /// Action-event foreign key (`tool_actions::ToolActionEvent.action_id`)
    /// when this frame was triggered by a recorded tool step. `None` for the
    /// 1s fallback poll and legacy captures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BrowserFrameInfo {
    pub session_id: Option<String>,
    pub target_id: Option<String>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub backend: String,
}

/// Capture a JPEG frame from the active backend. Returns `Ok(None)` when no
/// backend is active (we don't want to force-launch Chrome just to take a
/// frame — the panel will show its empty state instead).
pub async fn capture_frame(session_id: Option<&str>) -> Result<Option<BrowserFramePayload>> {
    let Some((backend, frame_session_id)) = capture_backend(session_id).await else {
        return Ok(None);
    };
    if !backend.is_connected().await {
        return Ok(None);
    }

    let info = frame_info_from_backend(&*backend, frame_session_id).await;

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
        session_id: info.session_id,
        target_id: info.target_id,
        url: info.url,
        title: info.title,
        jpeg_base64,
        captured_at: chrono::Utc::now().timestamp_millis(),
        backend: info.backend,
        action_id: None,
    }))
}

/// Return the current browser tab identity for UI side-output without taking a
/// screenshot. Like [`capture_frame`], this never force-launches a CDP browser.
pub async fn current_frame_info(session_id: Option<&str>) -> Result<Option<BrowserFrameInfo>> {
    let Some((backend, frame_session_id)) = capture_backend(session_id).await else {
        return Ok(None);
    };
    if !backend.is_connected().await {
        return Ok(None);
    }
    Ok(Some(
        frame_info_from_backend(&*backend, frame_session_id).await,
    ))
}

async fn frame_info_from_backend(
    backend: &dyn BrowserBackend,
    session_id: Option<String>,
) -> BrowserFrameInfo {
    // Fast-path metadata fetch — avoids the per-tab `evaluate("document.title")`
    // round-trip that `status()` does for every tab.
    let (target_id, url, title) = match backend.active_tab_info().await.ok().flatten() {
        Some(t) => (Some(t.target_id), Some(t.url), Some(t.title)),
        None => (None, None, None),
    };
    BrowserFrameInfo {
        session_id,
        target_id,
        url,
        title,
        backend: backend.backend_name().to_string(),
    }
}

/// Same backend selection the frame mirror uses — exposed for the panel
/// quick-bar (`browser_ui::panel_navigate`) so navigation drives the exact
/// tab the panel is mirroring.
pub(crate) async fn panel_backend(
    session_id: Option<&str>,
) -> Option<(Arc<dyn BrowserBackend>, Option<String>)> {
    capture_backend(session_id).await
}

async fn capture_backend(
    session_id: Option<&str>,
) -> Option<(Arc<dyn BrowserBackend>, Option<String>)> {
    let sid = session_id.filter(|s| !s.is_empty());
    if let Some(session_id) = sid {
        if let Some(backend) = extension_capture_backend(session_id) {
            return Some((backend, Some(session_id.to_string())));
        }
    }
    let frame_session_id = sid.map(str::to_string);
    peek_active()
        .await
        .map(|backend| (backend, frame_session_id))
}

fn extension_capture_backend(session_id: &str) -> Option<Arc<dyn BrowserBackend>> {
    let cfg = crate::config::cached_config();
    let browser_cfg = cfg.browser.as_ref();
    let preference = browser_cfg
        .and_then(|b| b.backend_preference)
        .unwrap_or_default();
    if preference == BrowserBackendPreference::CdpOnly {
        return None;
    }
    let extension_enabled = browser_cfg
        .and_then(|b| b.extension.as_ref())
        .is_none_or(|ext| ext.enabled());
    if !extension_enabled || !super::extension::current_status().backend_available {
        return None;
    }
    let broker = BrowserExtensionBroker::global()?;
    Some(Arc::new(ExtensionBackend::new(
        broker,
        BrowserBackendContext {
            session_id: Some(session_id.to_string()),
            source: Some("browser.frame".to_string()),
            ..BrowserBackendContext::default()
        },
    )))
}

/// Fire-and-forget: capture a frame in a background task and emit it on the
/// EventBus. Safe to call from synchronous-feeling 8-action handlers — never
/// blocks the action's return path.
///
/// Errors (no backend / capture failed) are logged at warn level but never
/// surface to the caller: the panel will pick up the next opportunity via
/// the 1s fallback poll.
pub fn emit_frame_async(session_id: Option<String>, action_id: Option<String>) {
    crate::browser_state::browser_runtime().spawn(async move {
        match capture_frame(session_id.as_deref()).await {
            Ok(Some(mut payload)) => {
                payload.action_id = action_id.clone();
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
                // Backfill the timeline thumbnail (best-effort, memory only).
                // The CPU-bound JPEG decode + re-encode goes to the blocking
                // pool — this runtime has only 2 workers shared with the CDP
                // event loop / heartbeat, which must not stall behind image
                // work on rapid act sequences.
                if let Some(action_id) = action_id {
                    let jpeg_base64 = payload.jpeg_base64.clone();
                    tokio::task::spawn_blocking(move || {
                        if let Ok(bytes) = base64::Engine::decode(
                            &base64::engine::general_purpose::STANDARD,
                            &jpeg_base64,
                        ) {
                            if let Some(thumb) =
                                crate::tool_actions::encode_thumbnail_from_jpeg(&bytes)
                            {
                                crate::tool_actions::attach_thumbnail(
                                    session_id.as_deref(),
                                    &action_id,
                                    thumb,
                                );
                            }
                        }
                    });
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
