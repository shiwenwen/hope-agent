//! `chrome-devtools-mcp` stdio backend.
//!
//! The Chrome instance itself is launched by hope-agent through the legacy
//! [`crate::browser_state`] singleton (the same path the CDP backend uses).
//! chrome-devtools-mcp connects to that running Chrome via `--browserUrl
//! http://127.0.0.1:9222`. This double-control-plane is intentional:
//!
//! - **Main control flow** (`act` / `snapshot` / `navigate` / `evaluate` …)
//!   goes through chrome-devtools-mcp, which gives us Google's vetted
//!   handling of ARIA snapshotting, stale-ref retry, and so on.
//! - **Observability** (`observe.console` / `observe.network` /
//!   `observe.page_errors`) stays on hope-agent's own chromiumoxide
//!   connection to the same 9222 port. `try_new` explicitly arms
//!   [`super::cdp_backend::activate_observe_subscribers_for_all_pages`] so
//!   the existing CDP event listeners feed [`super::observe_buffer`]; the
//!   MCP backend's `observe()` is just a buffer read.
//!
//! Lifecycle: the rmcp `RunningService` is held inside an `Arc<dyn
//! BrowserBackend>` in [`super::backend_select`]; dropping that Arc closes
//! stdio and reaps the chrome-devtools-mcp subprocess. Chrome itself stays
//! owned by `browser_state`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use rmcp::model::{CallToolRequestParams, CallToolResult, RawContent};
use rmcp::service::RunningService;
use rmcp::RoleClient;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use super::backend::{
    ActKind, ActParams, BackendStatus, BrowserBackend, DialogAction, ElementRef, ImageFormat,
    ObserveEntry, ObserveKind, PdfParams, ScreenshotParams, ScrollDirection, ScrollParams,
    Snapshot, SnapshotFormat, TabInfo, WaitParams,
};
use super::{
    activate_observe_subscribers_for_all_pages, activate_observe_subscribers_for_target,
    mcp_client, observe_buffer,
};

/// Exact version of chrome-devtools-mcp we ship with. Deliberately pinned
/// rather than `@latest` so:
/// - upstream releases never silently change behaviour on existing installs,
/// - we don't run arbitrary npm `latest` code with user privileges,
/// - upgrades are an audited single-line change here.
///
/// **Note**: `npx -y` still resolves this from the live npm registry on
/// first run, so true supply-chain protection requires bundling the
/// tarball with the app (tracked separately). Pinning the version is the
/// minimum bar.
pub const CHROME_DEVTOOLS_MCP_VERSION_SPEC: &str = "chrome-devtools-mcp@0.26.0";

/// Feature flags forwarded to the chrome-devtools-mcp invocation. The
/// structured-content flag is what makes [`Self::call_tool_json`] able to
/// pull JSON out of `CallToolResult.structured_content`; the page-id
/// routing flag is required for `select_page` / `close_page` to take the
/// `pageId` we hand them.
pub const CHROME_DEVTOOLS_MCP_FEATURE_ARGS: &[&str] = &[
    "--experimentalStructuredContent",
    "--experimental-page-id-routing",
];

pub struct ChromeMcpBackend {
    /// chrome-devtools-mcp stdio client. tokio `Mutex` rather than
    /// `std::sync::Mutex` because we hold the guard briefly across an
    /// `await` to clone the `Peer` handle. The outer `Arc<dyn
    /// BrowserBackend>` in [`super::backend_select`] is the lifecycle owner.
    client: Mutex<RunningService<RoleClient, ()>>,
    /// Snapshot-scoped mapping from our sequential `ref_id` (LLM-visible) to
    /// chrome-devtools-mcp's opaque `uid`. Reset on every `take_snapshot`.
    uid_index: Mutex<BTreeMap<u32, String>>,
    /// Monotonically reassigned per snapshot to keep `ref_id`s small.
    next_ref_id: AtomicU32,
}

impl ChromeMcpBackend {
    /// Attempt to initialise the MCP backend.
    ///
    /// Pre-condition: Chrome must already be running and reachable via the
    /// connection URL stored in `browser_state` (i.e. `profile.launch` or
    /// `profile.connect` has been called). If not, returns `Err` so
    /// [`super::backend_select::acquire_backend`] cleanly falls back to the
    /// CDP backend, which will then bring Chrome up itself before another
    /// `acquire_backend` cycle gets the chance to swap MCP back in.
    pub async fn try_new() -> Result<Self> {
        let browser_url = crate::browser_state::current_connection_url()
            .await
            .ok_or_else(|| {
                anyhow!(
                    "Chrome not yet connected — MCP backend defers to CDP for the initial launch \
                     (profile.launch / profile.connect must run first)"
                )
            })?;

        // Two independent operations: arm observe subscribers (cheap, just
        // CDP `Runtime.enable` / `Network.enable` on each page) and spawn
        // `npx -y chrome-devtools-mcp` (potentially 10–30s on a cold npm
        // cache). Run them in parallel. Observe activation is best-effort —
        // a failure there shouldn't gate the main control plane.
        let (obs_res, spawn_res) = tokio::join!(
            activate_observe_subscribers_for_all_pages(),
            mcp_client::spawn(&browser_url),
        );
        if let Err(e) = obs_res {
            app_warn!(
                "browser",
                "mcp_backend",
                "activate_observe_subscribers_for_all_pages failed: {}",
                e
            );
        }
        let connected = spawn_res?;
        app_info!(
            "browser",
            "mcp_backend",
            "ChromeMcpBackend ready, browser_url={}",
            browser_url
        );

        Ok(Self {
            client: Mutex::new(connected.running),
            uid_index: Mutex::new(BTreeMap::new()),
            next_ref_id: AtomicU32::new(1),
        })
    }

    /// Single entry point for chrome-devtools-mcp `tools/call`. Wraps the
    /// rmcp call, classifies `is_error=true` returns as `Err`, and pulls
    /// out the structured-content JSON when the server provides one
    /// (chrome-devtools-mcp does because we pass `--experimentalStructuredContent`).
    async fn call_tool(&self, name: &str, args: Value) -> Result<CallToolResult> {
        let arguments = match args {
            Value::Object(m) => Some(m),
            Value::Null => None,
            other => {
                let mut m = serde_json::Map::new();
                m.insert("value".into(), other);
                Some(m)
            }
        };
        let mut params = CallToolRequestParams::new(name.to_string());
        params.arguments = arguments;
        let guard = self.client.lock().await;
        // `Peer` is an `Arc`-backed handle; cloning is cheap and lets us
        // release the lock so concurrent calls don't serialise on it.
        let peer = guard.peer().clone();
        drop(guard);

        let result = peer
            .call_tool(params)
            .await
            .map_err(|e| anyhow!("chrome-devtools-mcp call_tool({}) failed: {e}", name))?;

        if result.is_error.unwrap_or(false) {
            let body = collect_text(&result);
            // Server-side errors (e.g. STALE_SELECTED_PAGE_ERROR) bubble up
            // verbatim so the LLM can decide whether to resnapshot.
            return Err(anyhow!(body));
        }
        Ok(result)
    }

    /// Convenience wrapper that extracts the structured-content JSON object
    /// from a tool result. Falls back to wrapping the joined text payload
    /// when chrome-devtools-mcp returns plain text only.
    async fn call_tool_json(&self, name: &str, args: Value) -> Result<Value> {
        let result = self.call_tool(name, args).await?;
        Ok(extract_value(&result))
    }
}

#[async_trait]
impl BrowserBackend for ChromeMcpBackend {
    fn backend_name(&self) -> &'static str {
        "mcp"
    }

    async fn is_connected(&self) -> bool {
        // The rmcp peer is always "up" once try_new succeeds; a torn-down
        // child would surface as a transport error on the next call_tool.
        true
    }

    async fn status(&self) -> Result<BackendStatus> {
        let tabs = self.list_pages().await.unwrap_or_default();
        let active = tabs
            .iter()
            .find(|t| t.is_active)
            .map(|t| t.target_id.clone());
        Ok(BackendStatus {
            connected: true,
            backend: "mcp".into(),
            active_target_id: active,
            tabs,
        })
    }

    async fn list_pages(&self) -> Result<Vec<TabInfo>> {
        let v = self.call_tool_json("list_pages", json!({})).await?;
        Ok(parse_tabs(&v))
    }

    async fn active_tab_info(&self) -> Result<Option<TabInfo>> {
        Ok(self.list_pages().await?.into_iter().find(|t| t.is_active))
    }

    async fn new_page(&self, url: Option<&str>) -> Result<TabInfo> {
        let v = self
            .call_tool_json("new_page", json!({ "url": url.unwrap_or("about:blank") }))
            .await?;
        // chrome-devtools-mcp returns the new page metadata; if missing,
        // re-query list_pages. Either way, arm observers just for the new
        // target so we don't re-scan every page in the session.
        let tab = parse_first_tab(&v).map_or_else(
            || {
                Err(anyhow!(
                    "new_page succeeded but no tab returned — retry via list_pages"
                ))
            },
            Ok,
        );
        let tab = match tab {
            Ok(t) => t,
            Err(_) => self
                .list_pages()
                .await?
                .into_iter()
                .find(|t| t.is_active)
                .ok_or_else(|| anyhow!("new_page succeeded but no tab returned"))?,
        };
        let _ = activate_observe_subscribers_for_target(&tab.target_id).await;
        Ok(tab)
    }

    async fn select_page(&self, target_id: &str) -> Result<()> {
        self.call_tool("select_page", json!({ "pageId": target_id }))
            .await?;
        Ok(())
    }

    async fn close_page(&self, target_id: &str) -> Result<()> {
        self.call_tool("close_page", json!({ "pageId": target_id }))
            .await?;
        Ok(())
    }

    async fn navigate(&self, url: &str) -> Result<String> {
        self.call_tool("navigate_page", json!({ "type": "url", "url": url }))
            .await?;
        Ok(format!("Navigated to {}", url))
    }

    async fn go_back(&self) -> Result<String> {
        self.call_tool("navigate_page", json!({ "type": "back" }))
            .await?;
        Ok("Navigated back".into())
    }

    async fn go_forward(&self) -> Result<String> {
        self.call_tool("navigate_page", json!({ "type": "forward" }))
            .await?;
        Ok("Navigated forward".into())
    }

    async fn reload(&self) -> Result<String> {
        self.call_tool("navigate_page", json!({ "type": "reload" }))
            .await?;
        Ok("Reloaded".into())
    }

    async fn take_snapshot(&self, format: SnapshotFormat) -> Result<Snapshot> {
        match format {
            SnapshotFormat::Role => {
                let raw = self.call_tool_json("take_snapshot", json!({})).await?;
                self.build_snapshot(&raw).await
            }
        }
    }

    async fn take_screenshot(&self, params: ScreenshotParams) -> Result<Vec<u8>> {
        let mut args = serde_json::Map::new();
        args.insert(
            "format".into(),
            Value::String(match params.format {
                ImageFormat::Jpeg => "jpeg".into(),
                ImageFormat::Png => "png".into(),
            }),
        );
        if let Some(q) = params.quality {
            args.insert("quality".into(), Value::Number(q.into()));
        }
        if params.full_page {
            args.insert("fullPage".into(), Value::Bool(true));
        }
        if let Some(ref_id) = params.ref_id {
            if let Some(uid) = self.uid_for_ref(ref_id).await {
                args.insert("uid".into(), Value::String(uid));
            }
        }
        let result = self
            .call_tool("take_screenshot", Value::Object(args))
            .await?;
        extract_base64_image(&result)
            .ok_or_else(|| anyhow!("take_screenshot returned no image data"))
    }

    async fn save_pdf(&self, _params: PdfParams) -> Result<Vec<u8>> {
        bail!(
            "PDF capture is not supported by the chrome-devtools-mcp backend. \
             Switch to the CDP backend (settings → Advanced → Force CDP) for PDF output."
        )
    }

    async fn act(&self, kind: ActKind, params: ActParams) -> Result<String> {
        match kind {
            ActKind::Click => {
                let uid = self.require_uid(params.ref_id).await?;
                self.call_tool("click", json!({ "uid": uid })).await?;
                Ok("Clicked".into())
            }
            ActKind::DoubleClick => {
                let uid = self.require_uid(params.ref_id).await?;
                self.call_tool("click", json!({ "uid": uid, "dblClick": true }))
                    .await?;
                Ok("Double-clicked".into())
            }
            ActKind::Type | ActKind::Fill => {
                let uid = self.require_uid(params.ref_id).await?;
                let text = params.text.unwrap_or_default();
                self.call_tool("fill", json!({ "uid": uid, "value": text }))
                    .await?;
                Ok(format!("Filled {} chars", count_chars(&text)))
            }
            ActKind::Hover => {
                let uid = self.require_uid(params.ref_id).await?;
                self.call_tool("hover", json!({ "uid": uid })).await?;
                Ok("Hovered".into())
            }
            ActKind::Drag => {
                let from = self.require_uid(params.ref_id).await?;
                let to = self
                    .require_uid(params.target_ref)
                    .await
                    .map_err(|_| anyhow!("drag requires target_ref"))?;
                self.call_tool("drag", json!({ "fromUid": from, "toUid": to }))
                    .await?;
                Ok("Dragged".into())
            }
            ActKind::Select => {
                // chrome-devtools-mcp exposes no native <select> driver, and
                // we can't reach into its `uid → element` resolver from
                // arbitrary JS. Fall through to `fill` — chrome-devtools-mcp
                // fills <select> by matching the visible option text.
                let uid = self.require_uid(params.ref_id).await?;
                let values = params.values.unwrap_or_default();
                let value = params.text.unwrap_or_else(|| values.join(","));
                self.call_tool("fill", json!({ "uid": uid, "value": value }))
                    .await?;
                Ok(format!("Selected {value}"))
            }
            ActKind::Press => {
                let key = params
                    .key
                    .ok_or_else(|| anyhow!("press requires a 'key' parameter"))?;
                self.call_tool("press_key", json!({ "key": key })).await?;
                Ok(format!("Pressed {}", key))
            }
            ActKind::Upload => {
                let uid = self.require_uid(params.ref_id).await?;
                let path = params
                    .file_path
                    .ok_or_else(|| anyhow!("upload requires a 'file_path' parameter"))?;
                let authorised = super::authorise_upload_path(&path)?;
                let path = authorised.to_string_lossy().into_owned();
                self.call_tool("upload_file", json!({ "uid": uid, "filePath": path }))
                    .await?;
                Ok(format!("Uploaded {path}"))
            }
        }
    }

    async fn evaluate(&self, script: &str) -> Result<Value> {
        let v = self
            .call_tool_json("evaluate_script", json!({ "script": script }))
            .await?;
        Ok(v)
    }

    async fn wait_for(&self, params: WaitParams) -> Result<String> {
        let mut args = serde_json::Map::new();
        args.insert("timeoutMs".into(), Value::Number(params.timeout_ms.into()));
        if let Some(text) = params.text {
            args.insert("text".into(), Value::String(text));
        }
        self.call_tool("wait_for", Value::Object(args)).await?;
        Ok("Wait condition met".into())
    }

    async fn handle_dialog(&self, action: DialogAction, prompt: Option<&str>) -> Result<String> {
        let act = match action {
            DialogAction::Accept => "accept",
            DialogAction::Dismiss => "dismiss",
        };
        let mut args = serde_json::Map::new();
        args.insert("action".into(), Value::String(act.into()));
        if let Some(p) = prompt {
            args.insert("promptText".into(), Value::String(p.into()));
        }
        self.call_tool("handle_dialog", Value::Object(args)).await?;
        Ok(format!("Dialog {}ed", act))
    }

    async fn resize(&self, width: u32, height: u32) -> Result<String> {
        self.call_tool("resize_page", json!({ "width": width, "height": height }))
            .await?;
        Ok(format!("Resized to {}x{}", width, height))
    }

    async fn scroll(&self, params: ScrollParams) -> Result<String> {
        // chrome-devtools-mcp has no native scroll tool; emit a small JS
        // window.scrollBy(...) shim instead.
        let (x, y) = match params.direction {
            ScrollDirection::Up => (0_i64, -params.amount),
            ScrollDirection::Down => (0, params.amount),
            ScrollDirection::Left => (-params.amount, 0),
            ScrollDirection::Right => (params.amount, 0),
        };
        let script = format!(
            "(() => {{ window.scrollBy({x}, {y}); return [window.scrollX, window.scrollY]; }})()"
        );
        self.call_tool("evaluate_script", json!({ "script": script }))
            .await?;
        Ok(format!(
            "Scrolled {:?} by {}",
            params.direction, params.amount
        ))
    }

    async fn observe(&self, kind: ObserveKind, since: Option<i64>) -> Result<Vec<ObserveEntry>> {
        Ok(observe_buffer::snapshot(kind, since))
    }
}

impl ChromeMcpBackend {
    async fn require_uid(&self, ref_id: Option<u32>) -> Result<String> {
        let rid = ref_id.ok_or_else(|| anyhow!("action requires a 'ref' parameter"))?;
        self.uid_for_ref(rid).await.ok_or_else(|| {
            anyhow!(
                "stale ref {} (snapshot expired or DOM changed) — call snapshot first",
                rid
            )
        })
    }

    async fn uid_for_ref(&self, ref_id: u32) -> Option<String> {
        self.uid_index.lock().await.get(&ref_id).cloned()
    }

    /// Rebuild the `uid → ref_id` table and convert chrome-devtools-mcp's
    /// snapshot payload into our [`Snapshot`] shape. The exact JSON shape is
    /// the structured-content body returned by the server (enabled by the
    /// `--experimentalStructuredContent` flag).
    async fn build_snapshot(&self, raw: &Value) -> Result<Snapshot> {
        let mut uid_index = self.uid_index.lock().await;
        uid_index.clear();
        self.next_ref_id.store(1, Ordering::Relaxed);

        let url = raw
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let title = raw
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let viewport = raw
            .get("viewport")
            .and_then(|v| v.as_object())
            .map(|m| {
                let w = m.get("width").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
                let h = m.get("height").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
                (w, h)
            })
            .unwrap_or((0, 0));
        let truncated = raw
            .get("truncated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut elements = Vec::new();
        if let Some(nodes) = raw.get("elements").and_then(|v| v.as_array()) {
            for node in nodes {
                self.walk_snapshot_node(node, 0, &mut uid_index, &mut elements);
            }
        } else if let Some(root) = raw.get("root") {
            self.walk_snapshot_node(root, 0, &mut uid_index, &mut elements);
        }

        Ok(Snapshot {
            url,
            title,
            viewport,
            elements,
            truncated,
        })
    }

    fn walk_snapshot_node(
        &self,
        node: &Value,
        depth: u32,
        uid_index: &mut BTreeMap<u32, String>,
        elements: &mut Vec<ElementRef>,
    ) {
        let uid = node
            .get("uid")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let role = node
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let text = node
            .get("name")
            .or_else(|| node.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if let Some(uid_str) = uid {
            let ref_id = self.next_ref_id.fetch_add(1, Ordering::Relaxed);
            uid_index.insert(ref_id, uid_str.clone());
            elements.push(ElementRef {
                ref_id,
                role,
                text,
                locator: uid_str,
                depth,
                attrs: Default::default(),
            });
        }
        if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
            for child in children {
                self.walk_snapshot_node(child, depth + 1, uid_index, elements);
            }
        }
    }
}

// ── Helpers (free fns; pure JSON wrangling) ─────────────────────────────────

fn count_chars(s: &str) -> usize {
    s.chars().count()
}

fn collect_text(result: &CallToolResult) -> String {
    let mut buf = String::new();
    for c in &result.content {
        if let RawContent::Text(t) = &c.raw {
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(&t.text);
        }
    }
    buf
}

fn extract_value(result: &CallToolResult) -> Value {
    if let Some(sc) = result.structured_content.as_ref() {
        return sc.clone();
    }
    // Fall back to parsing the joined text body as JSON; otherwise wrap it.
    let text = collect_text(result);
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

fn extract_base64_image(result: &CallToolResult) -> Option<Vec<u8>> {
    use base64::Engine;
    for c in &result.content {
        if let RawContent::Image(img) = &c.raw {
            let engine = base64::engine::general_purpose::STANDARD;
            if let Ok(bytes) = engine.decode(img.data.as_bytes()) {
                return Some(bytes);
            }
        }
    }
    None
}

fn parse_tabs(v: &Value) -> Vec<TabInfo> {
    let arr = v
        .as_array()
        .or_else(|| v.get("pages").and_then(|x| x.as_array()))
        .or_else(|| v.get("tabs").and_then(|x| x.as_array()));
    let Some(arr) = arr else {
        return Vec::new();
    };
    arr.iter().filter_map(parse_tab_obj).collect()
}

fn parse_tab_obj(v: &Value) -> Option<TabInfo> {
    let obj = v.as_object()?;
    Some(TabInfo {
        target_id: obj
            .get("pageId")
            .or_else(|| obj.get("id"))
            .or_else(|| obj.get("targetId"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        url: obj
            .get("url")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        title: obj
            .get("title")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        is_active: obj
            .get("isActive")
            .or_else(|| obj.get("active"))
            .or_else(|| obj.get("selected"))
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
    })
}

fn parse_first_tab(v: &Value) -> Option<TabInfo> {
    parse_tab_obj(v)
        .or_else(|| v.get("page").and_then(parse_tab_obj))
        .or_else(|| parse_tabs(v).into_iter().next())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_spec_constant_is_pinned_to_explicit_version() {
        // Guard against accidental regressions back to `@latest` —
        // floating versions are a supply-chain risk for code we npx-spawn.
        assert!(CHROME_DEVTOOLS_MCP_VERSION_SPEC.starts_with("chrome-devtools-mcp@"));
        assert!(
            !CHROME_DEVTOOLS_MCP_VERSION_SPEC.ends_with("@latest"),
            "chrome-devtools-mcp must be pinned to an exact version, not @latest"
        );
    }

    #[test]
    fn feature_args_include_experimental_routing() {
        assert!(CHROME_DEVTOOLS_MCP_FEATURE_ARGS.contains(&"--experimental-page-id-routing"));
        assert!(CHROME_DEVTOOLS_MCP_FEATURE_ARGS.contains(&"--experimentalStructuredContent"));
    }

    #[test]
    fn parse_tabs_handles_pages_key() {
        let v = json!({
            "pages": [
                { "pageId": "p1", "url": "https://a", "title": "A", "active": true },
                { "pageId": "p2", "url": "https://b", "title": "B" },
            ]
        });
        let tabs = parse_tabs(&v);
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].target_id, "p1");
        assert!(tabs[0].is_active);
        assert!(!tabs[1].is_active);
    }

    #[test]
    fn parse_tabs_returns_empty_on_garbage() {
        assert!(parse_tabs(&json!({ "nope": 1 })).is_empty());
        assert!(parse_tabs(&Value::Null).is_empty());
    }
}
