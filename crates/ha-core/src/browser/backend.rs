//! Browser automation backend abstraction.
//!
//! Defines the [`BrowserBackend`] trait that hides "how we drive Chrome" from
//! the 8-action LLM tool surface. Two implementations live next to this file:
//!
//! - [`super::cdp_backend::CdpBackend`] — direct CDP via `chromiumoxide` (zero
//!   runtime dependencies, always available).
//! - [`super::mcp_backend::ChromeMcpBackend`] — `chrome-devtools-mcp` over
//!   stdio (Google official MCP server; requires Node.js >= 18).
//!
//! Selection happens at backend acquisition time via
//! [`super::backend_select::select_backend`]; the result is cached so a given
//! browser session sticks with one backend. LLM tool calls never see which
//! backend is active — the [`backend_name`] hint exists only for telemetry,
//! the [`BrowserPanel`](../../components/chat/BrowserPanel.tsx) badge, and
//! diagnostics.

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Shared data types ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TabInfo {
    pub target_id: String,
    pub url: String,
    pub title: String,
    pub is_active: bool,
}

/// Element reference inside a snapshot. Surface-stable across backends:
/// CDP backend assigns sequential `ref_id`s, MCP backend maps
/// `chrome-devtools-mcp`'s opaque `uid` → local `ref_id` via a per-snapshot
/// table so LLMs always see `[ref=12]` style references.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementRef {
    pub ref_id: u32,
    pub role: String,
    pub text: String,
    /// Backend-specific opaque locator (CSS selector for CDP; chrome-devtools-mcp
    /// `uid` for MCP). The 8-action layer never inspects this; the backend
    /// uses it internally to actually drive Chrome.
    pub locator: String,
    #[serde(default)]
    pub depth: u32,
    #[serde(default)]
    pub attrs: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub url: String,
    pub title: String,
    pub viewport: (u32, u32),
    pub elements: Vec<ElementRef>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum SnapshotFormat {
    /// ARIA / role-based text tree for the LLM.
    Role,
}

#[derive(Debug, Clone)]
pub struct ScreenshotParams {
    pub format: ImageFormat,
    pub full_page: bool,
    pub quality: Option<u8>,
    /// Optional crop to a specific element (by ref_id).
    pub ref_id: Option<u32>,
}

impl Default for ScreenshotParams {
    fn default() -> Self {
        Self {
            format: ImageFormat::Png,
            full_page: false,
            quality: None,
            ref_id: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl ImageFormat {
    pub fn mime(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PdfParams {
    /// `a3` / `a4` / `a5` / `letter` / `legal` / `tabloid`. Defaults to a4 when None.
    pub paper_format: Option<String>,
    pub landscape: Option<bool>,
    pub print_background: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActKind {
    Click,
    DoubleClick,
    Type,
    Hover,
    Drag,
    Select,
    Fill,
    Press,
    Upload,
}

impl ActKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "click" => ActKind::Click,
            "double_click" | "dblclick" => ActKind::DoubleClick,
            "type" => ActKind::Type,
            "hover" => ActKind::Hover,
            "drag" => ActKind::Drag,
            "select" => ActKind::Select,
            "fill" => ActKind::Fill,
            "press" => ActKind::Press,
            "upload" => ActKind::Upload,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActParams {
    pub ref_id: Option<u32>,
    pub target_ref: Option<u32>,
    pub text: Option<String>,
    pub key: Option<String>,
    pub file_path: Option<String>,
    pub modifiers: Option<Vec<String>>,
    pub values: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct WaitParams {
    pub text: Option<String>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone)]
pub struct ScrollParams {
    pub direction: ScrollDirection,
    pub amount: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogAction {
    Accept,
    Dismiss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserveKind {
    Console,
    Network,
    PageErrors,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserveEntry {
    /// Unix millis when the event was captured.
    pub at: i64,
    /// `log` / `info` / `warn` / `error` / `request` / `response` / `exception`.
    pub level: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendStatus {
    pub connected: bool,
    pub backend: String,
    pub active_target_id: Option<String>,
    pub tabs: Vec<TabInfo>,
}

// ── The trait ────────────────────────────────────────────────────────────

/// Browser automation backend. Implementations must be `Send + Sync` and
/// support concurrent use from multiple async tasks (tool handlers, BrowserPanel
/// frame capture, observe-buffer drain).
#[async_trait]
pub trait BrowserBackend: Send + Sync {
    /// Stable identifier for telemetry / BrowserPanel badge. `"cdp"` or `"mcp"`.
    fn backend_name(&self) -> &'static str;

    /// Best-effort connection check. Used by `status` action and for deciding
    /// whether to auto-launch.
    async fn is_connected(&self) -> bool;

    /// Return current status snapshot (connected? active tab? tab list?).
    async fn status(&self) -> Result<BackendStatus>;

    // ── Tabs ────────────────────────────────────────────────────────────
    async fn list_pages(&self) -> Result<Vec<TabInfo>>;
    /// Cheap fast-path that fetches metadata only for the currently active
    /// tab. Used by [`super::frame::capture_frame`] which would otherwise
    /// pay the per-tab evaluate cost of [`Self::status`] for a single
    /// screenshot. Default impl falls back to scanning [`Self::status`].
    async fn active_tab_info(&self) -> Result<Option<TabInfo>> {
        let s = self.status().await?;
        Ok(s.tabs.into_iter().find(|t| t.is_active).or_else(|| {
            Some(TabInfo {
                target_id: s.active_target_id.clone().unwrap_or_default(),
                url: String::new(),
                title: String::new(),
                is_active: true,
            })
            .filter(|t| !t.target_id.is_empty())
        }))
    }
    /// Create a new tab. `url` is optional — `None` opens `about:blank`.
    /// Implementations must validate `url` through SSRF policy when set.
    async fn new_page(&self, url: Option<&str>) -> Result<TabInfo>;
    async fn select_page(&self, target_id: &str) -> Result<()>;
    async fn close_page(&self, target_id: &str) -> Result<()>;

    // ── Navigation ──────────────────────────────────────────────────────
    /// Caller has already validated URL through SSRF before calling.
    async fn navigate(&self, url: &str) -> Result<String>;
    async fn go_back(&self) -> Result<String>;
    async fn go_forward(&self) -> Result<String>;
    async fn reload(&self) -> Result<String>;

    // ── Snapshot / Capture ──────────────────────────────────────────────
    async fn take_snapshot(&self, format: SnapshotFormat) -> Result<Snapshot>;
    /// Returns raw image bytes. The 8-action layer formats them for the LLM
    /// (attachment or inline base64) and also forwards a JPEG copy to the
    /// chat BrowserPanel via the `browser:frame` event.
    async fn take_screenshot(&self, params: ScreenshotParams) -> Result<Vec<u8>>;
    async fn save_pdf(&self, params: PdfParams) -> Result<Vec<u8>>;

    // ── Interaction ─────────────────────────────────────────────────────
    /// Perform an interaction. Implementations should attempt one-shot
    /// stale-ref recovery on failure (re-snapshot + role+text fuzzy match,
    /// see [`super::cdp_backend::CdpBackend::act_with_recovery`]) and append
    /// `(ref auto-recovered)` to the success string when it kicks in.
    async fn act(&self, kind: ActKind, params: ActParams) -> Result<String>;

    // ── Control ─────────────────────────────────────────────────────────
    async fn evaluate(&self, script: &str) -> Result<Value>;
    async fn wait_for(&self, params: WaitParams) -> Result<String>;
    async fn handle_dialog(&self, action: DialogAction, prompt: Option<&str>) -> Result<String>;
    async fn resize(&self, width: u32, height: u32) -> Result<String>;
    async fn scroll(&self, params: ScrollParams) -> Result<String>;

    // ── Observe ─────────────────────────────────────────────────────────
    /// Drain or peek the observe ring buffer. `since` is a unix-millis cursor;
    /// when `None` returns the entire ring buffer (newest last).
    async fn observe(&self, kind: ObserveKind, since: Option<i64>) -> Result<Vec<ObserveEntry>>;
}
