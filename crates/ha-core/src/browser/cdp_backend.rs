//! Direct CDP backend backed by `chromiumoxide`.
//!
//! All page driving goes through the legacy [`crate::browser_state`] global
//! singleton — this struct is intentionally stateless and simply forwards trait
//! calls to the helpers used to live in `tools/browser/*.rs`. Stale-ref
//! recovery for `act` is implemented here once (no per-action duplication).
//!
//! This is the only backend implementation; the trait is kept as a future
//! extension point.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex as StdMutex;
use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType,
    MouseButton,
};
use chromiumoxide::Page;
use futures_util::StreamExt;
use serde_json::Value;

use super::backend::{
    ActKind, ActParams, BackendStatus, BrowserBackend, DialogAction, ElementRef, ImageFormat,
    ObserveEntry, ObserveKind, PdfParams, ScreenshotParams, ScrollDirection, ScrollParams,
    Snapshot, SnapshotFormat, TabInfo, WaitParams,
};
use super::observe_buffer;
use crate::browser_state::{get_browser_state, BrowserReadyMode, ElementRef as StateRef};

/// JavaScript injected into the page to extract an accessibility-like element
/// tree. Lifted verbatim from the previous `tools/browser/snapshot.rs` —
/// only the wrapper changed.
const SNAPSHOT_JS: &str = r#"(() => {
  const MAX_ELEMENTS = 300;
  const MAX_TEXT_LEN = 100;
  const refs = [];
  let refId = 0;

  const INTERACTIVE_SELECTORS = [
    'a[href]', 'button', 'input', 'select', 'textarea',
    '[role="button"]', '[role="link"]', '[role="textbox"]',
    '[role="checkbox"]', '[role="radio"]', '[role="tab"]',
    '[role="menuitem"]', '[role="option"]', '[role="switch"]',
    '[contenteditable="true"]', '[tabindex]'
  ];

  const SEMANTIC_TAGS = new Set([
    'h1','h2','h3','h4','h5','h6','p','li','td','th',
    'label','img','nav','main','header','footer','section',
    'article','aside','form','table','caption','figcaption'
  ]);

  function isVisible(el) {
    if (!el.getBoundingClientRect) return false;
    const rect = el.getBoundingClientRect();
    if (rect.width === 0 && rect.height === 0) return false;
    const style = window.getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') return false;
    return true;
  }

  function isInteractive(el) {
    return INTERACTIVE_SELECTORS.some(sel => {
      try { return el.matches(sel); } catch(e) { return false; }
    });
  }

  function getRole(el) {
    const role = el.getAttribute('role');
    if (role) return role;
    const tag = el.tagName.toLowerCase();
    const typeAttr = el.getAttribute('type');
    if (tag === 'a' && el.hasAttribute('href')) return 'link';
    if (tag === 'button') return 'button';
    if (tag === 'input') {
      if (typeAttr === 'checkbox') return 'checkbox';
      if (typeAttr === 'radio') return 'radio';
      if (typeAttr === 'submit' || typeAttr === 'button') return 'button';
      return 'textbox';
    }
    if (tag === 'textarea') return 'textbox';
    if (tag === 'select') return 'combobox';
    if (tag === 'img') return 'img';
    if (/^h[1-6]$/.test(tag)) return 'heading';
    return tag;
  }

  function getText(el) {
    const ariaLabel = el.getAttribute('aria-label');
    if (ariaLabel) return ariaLabel.trim().substring(0, MAX_TEXT_LEN);
    const alt = el.getAttribute('alt');
    if (alt) return alt.trim().substring(0, MAX_TEXT_LEN);
    const title = el.getAttribute('title');
    if (title && !el.children.length) return title.trim().substring(0, MAX_TEXT_LEN);
    const text = el.innerText || el.textContent || '';
    return text.trim().substring(0, MAX_TEXT_LEN);
  }

  function buildUniqueSelector(el) {
    if (el.id) return '#' + CSS.escape(el.id);
    const path = [];
    let current = el;
    while (current && current !== document.body && path.length < 5) {
      let selector = current.tagName.toLowerCase();
      if (current.id) {
        path.unshift('#' + CSS.escape(current.id) + ' > ' + selector);
        break;
      }
      if (current.className && typeof current.className === 'string') {
        const classes = current.className.trim().split(/\s+/).slice(0, 2);
        if (classes.length && classes[0]) {
          selector += '.' + classes.map(c => CSS.escape(c)).join('.');
        }
      }
      const parent = current.parentElement;
      if (parent) {
        const siblings = Array.from(parent.children).filter(c => c.tagName === current.tagName);
        if (siblings.length > 1) {
          const idx = siblings.indexOf(current) + 1;
          selector += ':nth-of-type(' + idx + ')';
        }
      }
      path.unshift(selector);
      current = current.parentElement;
    }
    return path.join(' > ');
  }

  function walk(el, depth) {
    if (refId >= MAX_ELEMENTS) return;
    if (!el || !el.tagName) return;
    if (!isVisible(el)) return;

    const tag = el.tagName.toLowerCase();
    const interactive = isInteractive(el);
    const semantic = SEMANTIC_TAGS.has(tag);

    if (interactive || semantic) {
      refId++;
      const rect = el.getBoundingClientRect();
      const info = {
        ref: refId,
        depth: depth,
        role: getRole(el),
        text: getText(el),
        selector: buildUniqueSelector(el),
        cx: Math.round(rect.x + rect.width / 2),
        cy: Math.round(rect.y + rect.height / 2),
        attrs: {}
      };
      if (el.href) info.attrs.url = el.href;
      if (el.value !== undefined && el.value !== '') info.attrs.value = String(el.value);
      if (el.placeholder) info.attrs.placeholder = el.placeholder;
      if (el.name) info.attrs.name = el.name;
      if (el.type) info.attrs.type = el.type;
      if (el.checked !== undefined) info.attrs.checked = el.checked;
      if (el.disabled) info.attrs.disabled = true;
      if (el.readOnly) info.attrs.readonly = true;
      if (tag.match(/^h[1-6]$/)) info.attrs.level = parseInt(tag[1]);
      refs.push(info);
    }

    for (const child of el.children) {
      walk(child, depth + (interactive || semantic ? 1 : 0));
    }
  }

  walk(document.body, 0);

  return JSON.stringify({
    url: location.href,
    title: document.title,
    viewport: { w: window.innerWidth, h: window.innerHeight },
    elements: refs,
    truncated: refId >= MAX_ELEMENTS
  });
})()"#;

/// Tracks pages we've already attached observe-event subscribers to so we
/// don't open duplicate `event_listener` streams when the same `Page` handle
/// is encountered multiple times across `new_page` / `list_pages` /
/// `active_tab_info` etc.
fn subscribed_pages() -> &'static StdMutex<HashSet<String>> {
    static SUBSCRIBED: OnceLock<StdMutex<HashSet<String>>> = OnceLock::new();
    SUBSCRIBED.get_or_init(|| StdMutex::new(HashSet::new()))
}

/// Forget all subscribed page IDs — called from [`super::backend_select::reset_backend`]
/// so a relaunch of Chrome starts with a fresh subscriber registry instead of
/// hanging onto dead target IDs.
///
/// Walk every currently-known page in `browser_state` and install observe
/// subscribers (Console / Network / Exception). Called from launch and
/// reconnect paths to make sure new sessions immediately start capturing
/// observability events into [`super::observe_buffer`].
pub async fn activate_observe_subscribers_for_all_pages() -> anyhow::Result<()> {
    let state = crate::browser_state::get_browser_state().lock().await;
    if !state.is_connected() {
        return Ok(());
    }
    for (target_id, page) in state.pages.iter() {
        ensure_observe_subscribers(page, target_id).await;
    }
    Ok(())
}

/// Like [`activate_observe_subscribers_for_all_pages`] but scoped to a
/// single target id — called when opening a new tab so we don't re-walk
/// the first N. `subscribed_pages` HashSet is idempotent so re-entry is
/// also safe.
pub async fn activate_observe_subscribers_for_target(target_id: &str) -> anyhow::Result<()> {
    let state = crate::browser_state::get_browser_state().lock().await;
    if !state.is_connected() {
        return Ok(());
    }
    if let Some(page) = state.pages.get(target_id) {
        ensure_observe_subscribers(page, target_id).await;
    } else {
        // New target ids surface in `state.pages` only after `refresh_pages()`;
        // if we missed it here, the next `list_pages` will pick it up via
        // the `for_all_pages` path or a future `_for_target` call.
    }
    Ok(())
}

pub(super) fn clear_subscribed_pages() {
    if let Ok(mut set) = subscribed_pages().lock() {
        set.clear();
    }
}

/// Idempotently install console / network / runtime-exception subscribers on
/// `page` so that the `observe` action has data to return. The subscribers
/// run for the lifetime of the page (or until the EventStream closes when
/// the page is torn down). Failures are logged but never propagated — observe
/// is a best-effort visibility feature, not a correctness gate.
async fn ensure_observe_subscribers(page: &Page, target_id: &str) {
    {
        let mut set = match subscribed_pages().lock() {
            Ok(s) => s,
            Err(p) => p.into_inner(),
        };
        if !set.insert(target_id.to_string()) {
            return;
        }
    }
    use chromiumoxide::cdp::browser_protocol::network::EnableParams as NetworkEnable;
    use chromiumoxide::cdp::js_protocol::runtime::{
        EnableParams as RuntimeEnable, EventConsoleApiCalled, EventExceptionThrown,
    };

    if let Err(e) = page.execute(RuntimeEnable::default()).await {
        app_warn!(
            "browser",
            "observe",
            "Runtime.enable failed for {}: {}",
            target_id,
            e
        );
        return;
    }
    if let Err(e) = page.execute(NetworkEnable::builder().build()).await {
        app_warn!(
            "browser",
            "observe",
            "Network.enable failed for {}: {}",
            target_id,
            e
        );
    }

    if let Ok(mut stream) = page.event_listener::<EventConsoleApiCalled>().await {
        let tid = target_id.to_string();
        crate::browser_state::browser_runtime().spawn(async move {
            while let Some(evt) = stream.next().await {
                let level = format!("{:?}", evt.r#type).to_ascii_lowercase();
                let text = evt
                    .args
                    .iter()
                    .filter_map(|a| {
                        a.value
                            .as_ref()
                            .map(|v| v.to_string())
                            .or_else(|| a.description.clone())
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                observe_buffer::push(
                    ObserveKind::Console,
                    ObserveEntry {
                        at: chrono::Utc::now().timestamp_millis(),
                        level,
                        text,
                        url: None,
                    },
                );
            }
            app_debug!("browser", "observe", "console stream ended for {}", tid);
        });
    }

    if let Ok(mut stream) = page.event_listener::<EventExceptionThrown>().await {
        let tid = target_id.to_string();
        crate::browser_state::browser_runtime().spawn(async move {
            while let Some(evt) = stream.next().await {
                let text = evt.exception_details.text.clone();
                let detail_msg = evt
                    .exception_details
                    .exception
                    .as_ref()
                    .and_then(|e| e.description.clone())
                    .unwrap_or_default();
                let combined = if detail_msg.is_empty() {
                    text
                } else {
                    format!("{} — {}", text, detail_msg)
                };
                observe_buffer::push(
                    ObserveKind::PageErrors,
                    ObserveEntry {
                        at: chrono::Utc::now().timestamp_millis(),
                        level: "exception".to_string(),
                        text: combined,
                        url: None,
                    },
                );
            }
            app_debug!("browser", "observe", "exception stream ended for {}", tid);
        });
    }

    if let Ok(mut stream) = page
        .event_listener::<chromiumoxide::cdp::browser_protocol::network::EventResponseReceived>()
        .await
    {
        let tid = target_id.to_string();
        crate::browser_state::browser_runtime().spawn(async move {
            while let Some(evt) = stream.next().await {
                let url = evt.response.url.clone();
                let status = evt.response.status;
                let mime = evt.response.mime_type.clone();
                observe_buffer::push(
                    ObserveKind::Network,
                    ObserveEntry {
                        at: chrono::Utc::now().timestamp_millis(),
                        level: format!("{}", status),
                        text: format!("{} ({})", url, mime),
                        url: Some(url),
                    },
                );
            }
            app_debug!("browser", "observe", "network stream ended for {}", tid);
        });
    }
}

pub struct CdpBackend;

impl CdpBackend {
    pub fn new() -> Self {
        Self
    }

    /// Re-snapshot the active page and find an element whose `(role, text)`
    /// closely matches the stale ref. Returns the rebuilt selector when a
    /// match is found.
    async fn try_recover_stale_ref(role: &str, text: &str) -> Result<(u32, String)> {
        // Re-run snapshot to refresh refs/selectors.
        let _ = Self::take_snapshot_inner().await?;
        let state = get_browser_state().lock().await;
        let needle = text.trim();
        let best = state
            .element_refs
            .iter()
            .find(|r| r.role == role && r.text.trim() == needle)
            .or_else(|| {
                state.element_refs.iter().find(|r| {
                    r.role == role
                        && !needle.is_empty()
                        && (r.text.contains(needle) || needle.contains(r.text.trim()))
                })
            });
        match best {
            Some(r) => Ok((r.ref_id, r.selector.clone())),
            None => Err(anyhow!(
                "stale ref recovery failed: no element with role='{}' text~='{}' after resnapshot",
                role,
                text
            )),
        }
    }

    async fn take_snapshot_inner() -> Result<Snapshot> {
        crate::browser_state::ensure_connected_or_launch_managed().await?;
        let page = {
            let state = get_browser_state().lock().await;
            state.get_active_page()?.clone()
        };
        let json_str: String = page
            .evaluate(SNAPSHOT_JS)
            .await
            .map_err(|e| anyhow!("Failed to take snapshot: {}", e))?
            .into_value()
            .map_err(|e| anyhow!("Snapshot returned invalid data: {}", e))?;
        let data: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| anyhow!("Failed to parse snapshot: {}", e))?;

        let url = data
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let title = data
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("untitled")
            .to_string();
        let viewport_w = data
            .get("viewport")
            .and_then(|v| v.get("w"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let viewport_h = data
            .get("viewport")
            .and_then(|v| v.get("h"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let truncated = data
            .get("truncated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut elements = Vec::new();
        let mut new_state_refs: Vec<StateRef> = Vec::new();
        if let Some(arr) = data.get("elements").and_then(|v| v.as_array()) {
            for el in arr {
                let ref_id = el.get("ref").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let depth = el.get("depth").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let role = el.get("role").and_then(|v| v.as_str()).unwrap_or("unknown");
                let text = el.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let selector = el.get("selector").and_then(|v| v.as_str()).unwrap_or("");
                let cx = el.get("cx").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let cy = el.get("cy").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let attrs_raw = el.get("attrs").cloned().unwrap_or(serde_json::json!({}));
                let mut attr_map: HashMap<String, String> = HashMap::new();
                if let Some(obj) = attrs_raw.as_object() {
                    for (k, v) in obj {
                        if let Some(s) = v.as_str() {
                            attr_map.insert(k.clone(), s.to_string());
                        } else {
                            attr_map.insert(k.clone(), v.to_string());
                        }
                    }
                }
                elements.push(ElementRef {
                    ref_id,
                    role: role.to_string(),
                    text: text.to_string(),
                    locator: selector.to_string(),
                    depth,
                    attrs: attr_map.clone(),
                });
                new_state_refs.push(StateRef {
                    ref_id,
                    role: role.to_string(),
                    text: text.to_string(),
                    selector: selector.to_string(),
                    center_x: cx,
                    center_y: cy,
                    attrs: attr_map,
                });
            }
        }

        // Persist into the legacy global so subsequent act() calls can look up
        // the underlying selector by ref_id.
        let mut state = get_browser_state().lock().await;
        state.element_refs = new_state_refs;
        state.snapshot_url = Some(url.clone());

        Ok(Snapshot {
            url,
            title,
            viewport: (viewport_w, viewport_h),
            elements,
            truncated,
        })
    }
}

impl Default for CdpBackend {
    fn default() -> Self {
        Self::new()
    }
}

fn is_stale_ref_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("not found")
        || msg.contains("no such element")
        || msg.contains("ref=")
        || msg.contains("stale")
        || msg.contains("detached")
}

#[async_trait]
impl BrowserBackend for CdpBackend {
    fn backend_name(&self) -> &'static str {
        "cdp"
    }

    async fn is_connected(&self) -> bool {
        let state = get_browser_state().lock().await;
        state.is_connected()
    }

    async fn status(&self) -> Result<BackendStatus> {
        // Snapshot phase: hold the lock only long enough to refresh the page
        // table and clone the page handles we want to introspect. The
        // subsequent CDP round-trips (`page.url()`, `page.evaluate(...)`)
        // are awaited with the global mutex released so a concurrent tool
        // call (or BrowserPanel 1Hz capture) isn't blocked behind this
        // status report.
        let (connected, active_target_id, page_handles) = {
            let mut state = get_browser_state().lock().await;
            let connected = state.is_connected();
            let active_target_id = state.active_page_id.clone();
            let handles: Vec<(String, Page)> = if connected {
                let _ = state.refresh_pages().await;
                state
                    .pages
                    .iter()
                    .map(|(id, page)| (id.clone(), page.clone()))
                    .collect()
            } else {
                Vec::new()
            };
            (connected, active_target_id, handles)
        };
        let active_id_str = active_target_id.clone().unwrap_or_default();
        let mut tabs: Vec<TabInfo> = Vec::with_capacity(page_handles.len());
        for (id, page) in &page_handles {
            let url = page
                .url()
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| "about:blank".to_string());
            let title = page
                .evaluate("document.title")
                .await
                .ok()
                .and_then(|v| v.into_value().ok())
                .unwrap_or_else(|| "untitled".to_string());
            tabs.push(TabInfo {
                target_id: id.clone(),
                url,
                title,
                is_active: *id == active_id_str,
            });
        }
        for (id, page) in &page_handles {
            ensure_observe_subscribers(page, id).await;
        }
        Ok(BackendStatus {
            connected,
            backend: self.backend_name().to_string(),
            active_target_id,
            tabs,
        })
    }

    async fn active_tab_info(&self) -> Result<Option<TabInfo>> {
        crate::browser_state::ensure_connected().await?;
        let state = get_browser_state().lock().await;
        let Some(active_id) = state.active_page_id.clone() else {
            return Ok(None);
        };
        let Some(page) = state.pages.get(&active_id).cloned() else {
            return Ok(None);
        };
        drop(state);
        ensure_observe_subscribers(&page, &active_id).await;
        let url = page
            .url()
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "about:blank".to_string());
        let title = page
            .evaluate("document.title")
            .await
            .ok()
            .and_then(|v| v.into_value().ok())
            .unwrap_or_else(|| "untitled".to_string());
        Ok(Some(TabInfo {
            target_id: active_id,
            url,
            title,
            is_active: true,
        }))
    }

    async fn list_pages(&self) -> Result<Vec<TabInfo>> {
        crate::browser_state::ensure_connected_or_launch_managed().await?;
        // Snapshot phase: hold the lock only long enough to refresh and
        // clone Page handles. The per-tab `url()` / `evaluate("document.title")`
        // CDP round-trips run after the lock is dropped so a concurrent
        // tool call doesn't queue behind this listing.
        let (active_id, page_handles) = {
            let mut state = get_browser_state().lock().await;
            state.refresh_pages().await?;
            let active_id = state.active_page_id.clone().unwrap_or_default();
            let handles: Vec<(String, Page)> = state
                .pages
                .iter()
                .map(|(id, page)| (id.clone(), page.clone()))
                .collect();
            (active_id, handles)
        };
        let mut out = Vec::with_capacity(page_handles.len());
        for (id, page) in &page_handles {
            let url = page
                .url()
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| "about:blank".to_string());
            let title = page
                .evaluate("document.title")
                .await
                .ok()
                .and_then(|v| v.into_value().ok())
                .unwrap_or_else(|| "untitled".to_string());
            out.push(TabInfo {
                target_id: id.clone(),
                url,
                title,
                is_active: *id == active_id,
            });
        }
        for (id, page) in &page_handles {
            ensure_observe_subscribers(page, id).await;
        }
        Ok(out)
    }

    async fn new_page(&self, url: Option<&str>) -> Result<TabInfo> {
        let target_url = url.unwrap_or("about:blank");
        let _ready: BrowserReadyMode =
            crate::browser_state::ensure_connected_or_launch_managed().await?;
        let mut state = get_browser_state().lock().await;
        let browser = state
            .browser
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;
        let page = browser
            .new_page(target_url)
            .await
            .map_err(|e| anyhow!("Failed to create new page: {}", e))?;
        let target_id = page.target_id().as_ref().to_string();
        state.active_page_id = Some(target_id.clone());
        let page_clone = page.clone();
        state.pages.insert(target_id.clone(), page);
        state.element_refs.clear();
        state.snapshot_url = None;
        drop(state);
        ensure_observe_subscribers(&page_clone, &target_id).await;
        Ok(TabInfo {
            target_id,
            url: target_url.to_string(),
            title: String::new(),
            is_active: true,
        })
    }

    async fn select_page(&self, target_id: &str) -> Result<()> {
        crate::browser_state::ensure_connected().await?;
        let mut state = get_browser_state().lock().await;
        if !state.pages.contains_key(target_id) {
            let available: Vec<&String> = state.pages.keys().collect();
            return Err(anyhow!(
                "Page '{}' not found. Available: {:?}",
                target_id,
                available
            ));
        }
        state.active_page_id = Some(target_id.to_string());
        state.element_refs.clear();
        state.snapshot_url = None;
        Ok(())
    }

    async fn close_page(&self, target_id: &str) -> Result<()> {
        crate::browser_state::ensure_connected().await?;
        let mut state = get_browser_state().lock().await;
        let page = state
            .pages
            .remove(target_id)
            .ok_or_else(|| anyhow!("Page '{}' not found", target_id))?;
        let _ = page.close().await;
        if state.active_page_id.as_deref() == Some(target_id) {
            state.active_page_id = state.pages.keys().next().cloned();
            state.element_refs.clear();
            state.snapshot_url = None;
        }
        Ok(())
    }

    async fn navigate(&self, url: &str) -> Result<String> {
        crate::browser_state::ensure_connected_or_launch_managed().await?;
        let page = {
            let state = get_browser_state().lock().await;
            state.get_active_page()?.clone()
        };
        page.goto(url)
            .await
            .map_err(|e| anyhow!("Navigation failed: {}", e))?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let title: String = page
            .evaluate("document.title")
            .await
            .ok()
            .and_then(|r| r.into_value().ok())
            .unwrap_or_else(|| "untitled".to_string());
        let current_url = page
            .url()
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| url.to_string());
        let mut state = get_browser_state().lock().await;
        state.element_refs.clear();
        state.snapshot_url = None;
        Ok(format!("Navigated to: {} - \"{}\"", current_url, title))
    }

    async fn go_back(&self) -> Result<String> {
        crate::browser_state::ensure_connected().await?;
        let page = {
            let state = get_browser_state().lock().await;
            state.get_active_page()?.clone()
        };
        page.evaluate("history.back()")
            .await
            .map_err(|e| anyhow!("Go back failed: {}", e))?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let url = page
            .url()
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "unknown".to_string());
        let mut state = get_browser_state().lock().await;
        state.element_refs.clear();
        state.snapshot_url = None;
        Ok(format!("Navigated back to: {}", url))
    }

    async fn go_forward(&self) -> Result<String> {
        crate::browser_state::ensure_connected().await?;
        let page = {
            let state = get_browser_state().lock().await;
            state.get_active_page()?.clone()
        };
        page.evaluate("history.forward()")
            .await
            .map_err(|e| anyhow!("Go forward failed: {}", e))?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let url = page
            .url()
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "unknown".to_string());
        let mut state = get_browser_state().lock().await;
        state.element_refs.clear();
        state.snapshot_url = None;
        Ok(format!("Navigated forward to: {}", url))
    }

    async fn reload(&self) -> Result<String> {
        crate::browser_state::ensure_connected().await?;
        let page = {
            let state = get_browser_state().lock().await;
            state.get_active_page()?.clone()
        };
        page.reload()
            .await
            .map_err(|e| anyhow!("Reload failed: {}", e))?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let url = page
            .url()
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "unknown".to_string());
        let mut state = get_browser_state().lock().await;
        state.element_refs.clear();
        state.snapshot_url = None;
        Ok(format!("Reloaded: {}", url))
    }

    async fn take_snapshot(&self, _format: SnapshotFormat) -> Result<Snapshot> {
        Self::take_snapshot_inner().await
    }

    async fn take_screenshot(&self, params: ScreenshotParams) -> Result<Vec<u8>> {
        use base64::Engine;
        use chromiumoxide::cdp::browser_protocol::page::{
            CaptureScreenshotFormat, CaptureScreenshotParams, GetLayoutMetricsParams, Viewport,
        };
        crate::browser_state::ensure_connected().await?;
        let page = {
            let state = get_browser_state().lock().await;
            state.get_active_page()?.clone()
        };
        let format = match params.format {
            ImageFormat::Jpeg => CaptureScreenshotFormat::Jpeg,
            ImageFormat::Png => CaptureScreenshotFormat::Png,
        };
        // Bypass `Page::screenshot` because chromiumoxide 0.9.1 internally
        // calls `Page.bringToFront` on every screenshot (handler/page.rs:397
        // `self.activate().await?` before the actual CDP call). That makes
        // Chrome steal focus from Hope Agent every time. The BrowserPanel UI
        // polls a screenshot at 1 Hz, so the user would see Chrome jump to
        // the foreground every second. `Page.captureScreenshot` itself does
        // not require the tab to be foregrounded — Chrome headless mode
        // captures fine — so we emit the CDP command directly via
        // `page.execute(...)` and skip the activate step.
        let mut cdp = CaptureScreenshotParams {
            format: Some(format),
            quality: params.quality.map(|q| q as i64),
            ..Default::default()
        };
        if params.full_page {
            let metrics = page
                .execute(GetLayoutMetricsParams::default())
                .await
                .map_err(|e| anyhow!("getLayoutMetrics failed: {}", e))?;
            let css = &metrics.result.css_content_size;
            cdp.clip = Some(Viewport {
                x: 0.0,
                y: 0.0,
                width: css.width,
                height: css.height,
                scale: 1.0,
            });
            cdp.capture_beyond_viewport = Some(true);
        }
        let resp = page
            .execute(cdp)
            .await
            .map_err(|e| anyhow!("Screenshot failed: {}", e))?;
        let b64: &str = resp.result.data.as_ref();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| anyhow!("Screenshot base64 decode failed: {}", e))?;
        Ok(bytes)
    }

    async fn save_pdf(&self, params: PdfParams) -> Result<Vec<u8>> {
        use chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams;
        crate::browser_state::ensure_connected().await?;
        let page = {
            let state = get_browser_state().lock().await;
            state.get_active_page()?.clone()
        };
        let mut p = PrintToPdfParams::default();
        if let Some(paper) = params.paper_format.as_deref() {
            let (w, h) = match paper.to_ascii_lowercase().as_str() {
                "a3" => (11.69, 16.54),
                "a4" => (8.27, 11.69),
                "a5" => (5.83, 8.27),
                "letter" => (8.5, 11.0),
                "legal" => (8.5, 14.0),
                "tabloid" => (11.0, 17.0),
                other => {
                    return Err(anyhow!(
                        "Unknown paper_format: '{}'. Options: a3, a4, a5, letter, legal, tabloid",
                        other
                    ))
                }
            };
            p.paper_width = Some(w);
            p.paper_height = Some(h);
        }
        if let Some(l) = params.landscape {
            p.landscape = Some(l);
        }
        if let Some(b) = params.print_background {
            p.print_background = Some(b);
        }
        page.pdf(p)
            .await
            .map_err(|e| anyhow!("PDF export failed: {}", e))
    }

    async fn act(&self, kind: ActKind, params: ActParams) -> Result<String> {
        match self.act_inner(kind, &params, false).await {
            Ok(s) => Ok(s),
            Err(e) if is_stale_ref_error(&e) => {
                // Stale-ref recovery: try once with a freshly resnapshotted ref.
                if let Some(ref_id) = params.ref_id {
                    let role_text = {
                        let state = get_browser_state().lock().await;
                        state
                            .element_refs
                            .iter()
                            .find(|r| r.ref_id == ref_id)
                            .map(|r| (r.role.clone(), r.text.clone()))
                    };
                    if let Some((role, text)) = role_text {
                        match Self::try_recover_stale_ref(&role, &text).await {
                            Ok((new_ref, _selector)) => {
                                let recovered_params = ActParams {
                                    ref_id: Some(new_ref),
                                    ..params.clone()
                                };
                                let retry = self.act_inner(kind, &recovered_params, true).await?;
                                Ok(format!(
                                    "{} (ref auto-recovered: {} -> {})",
                                    retry, ref_id, new_ref
                                ))
                            }
                            Err(rec_err) => {
                                app_warn!("browser", "stale-ref", "recovery failed: {}", rec_err);
                                Err(e)
                            }
                        }
                    } else {
                        Err(e)
                    }
                } else {
                    Err(e)
                }
            }
            Err(e) => Err(e),
        }
    }

    async fn evaluate(&self, script: &str) -> Result<Value> {
        crate::browser_state::ensure_connected().await?;
        let page = {
            let state = get_browser_state().lock().await;
            state.get_active_page()?.clone()
        };
        let result = page
            .evaluate(script)
            .await
            .map_err(|e| anyhow!("Script evaluation failed: {}", e))?;
        Ok(result.into_value().unwrap_or(Value::Null))
    }

    async fn wait_for(&self, params: WaitParams) -> Result<String> {
        crate::browser_state::ensure_connected().await?;
        let needle = params
            .text
            .clone()
            .ok_or_else(|| anyhow!("wait_for requires 'text' parameter"))?;
        let timeout_ms = params.timeout_ms;
        let check_js = format!(
            "document.body.innerText.includes('{}')",
            needle
                .replace('\\', "\\\\")
                .replace('\'', "\\'")
                .replace('\n', "\\n")
        );
        // Clone the active Page handle ONCE up front. The previous version
        // re-acquired the global state mutex inside every poll iteration AND
        // held it across `page.evaluate(...).await`, blocking concurrent
        // tool calls / BrowserPanel frame captures for the whole timeout
        // window. Page is a cheap chromiumoxide handle (Arc internally), so
        // cloning is fine; tab navigations during the wait stay observable
        // because the handle tracks the underlying target.
        let page = {
            let state = get_browser_state().lock().await;
            state.get_active_page()?.clone()
        };
        let start = std::time::Instant::now();
        let poll = std::time::Duration::from_millis(500);
        loop {
            let found: bool = page
                .evaluate(check_js.as_str())
                .await
                .ok()
                .and_then(|r| r.into_value().ok())
                .unwrap_or(false);
            if found {
                return Ok(format!("Text \"{}\" found on page.", needle));
            }
            if start.elapsed().as_millis() as u64 >= timeout_ms {
                return Err(anyhow!(
                    "Timeout after {}ms waiting for text \"{}\"",
                    timeout_ms,
                    needle
                ));
            }
            tokio::time::sleep(poll).await;
        }
    }

    async fn handle_dialog(&self, action: DialogAction, prompt: Option<&str>) -> Result<String> {
        use chromiumoxide::cdp::browser_protocol::page::HandleJavaScriptDialogParams;
        crate::browser_state::ensure_connected().await?;
        let accept = matches!(action, DialogAction::Accept);
        let page = {
            let state = get_browser_state().lock().await;
            state.get_active_page()?.clone()
        };
        let mut params = HandleJavaScriptDialogParams::new(accept);
        if let Some(p) = prompt {
            params.prompt_text = Some(p.to_string());
        }
        page.execute(params)
            .await
            .map_err(|e| anyhow!("Handle dialog failed: {}. Is there an open dialog?", e))?;
        Ok(format!(
            "Dialog {}.{}",
            if accept { "accepted" } else { "dismissed" },
            prompt
                .map(|t| format!(" Prompt text: \"{}\"", t))
                .unwrap_or_default()
        ))
    }

    async fn resize(&self, width: u32, height: u32) -> Result<String> {
        use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;
        crate::browser_state::ensure_connected().await?;
        let page = {
            let state = get_browser_state().lock().await;
            state.get_active_page()?.clone()
        };
        let params = SetDeviceMetricsOverrideParams::new(width as i64, height as i64, 1.0, false);
        page.execute(params)
            .await
            .map_err(|e| anyhow!("Resize failed: {}", e))?;
        Ok(format!("Viewport resized to {}x{}", width, height))
    }

    async fn scroll(&self, params: ScrollParams) -> Result<String> {
        crate::browser_state::ensure_connected().await?;
        let page = {
            let state = get_browser_state().lock().await;
            state.get_active_page()?.clone()
        };
        let (dx, dy) = match params.direction {
            ScrollDirection::Up => (0, -params.amount),
            ScrollDirection::Down => (0, params.amount),
            ScrollDirection::Left => (-params.amount, 0),
            ScrollDirection::Right => (params.amount, 0),
        };
        let js = format!("window.scrollBy({}, {})", dx, dy);
        page.evaluate(js)
            .await
            .map_err(|e| anyhow!("Scroll failed: {}", e))?;
        Ok(format!(
            "Scrolled {:?} by {} pixels",
            params.direction,
            params.amount.abs()
        ))
    }

    async fn observe(&self, kind: ObserveKind, since: Option<i64>) -> Result<Vec<ObserveEntry>> {
        Ok(super::observe_buffer::snapshot(kind, since))
    }
}

impl CdpBackend {
    /// Inner act implementation (no recovery). `retry_attempt` flag is reserved
    /// for future fine-grained logging.
    async fn act_inner(
        &self,
        kind: ActKind,
        params: &ActParams,
        _retry_attempt: bool,
    ) -> Result<String> {
        crate::browser_state::ensure_connected().await?;
        match kind {
            ActKind::Click | ActKind::DoubleClick => {
                self.act_click(params, kind == ActKind::DoubleClick).await
            }
            ActKind::Fill => self.act_fill(params).await,
            ActKind::Hover => self.act_hover(params).await,
            ActKind::Drag => self.act_drag(params).await,
            ActKind::Press => self.act_press_key(params).await,
            ActKind::Upload => self.act_upload(params).await,
            ActKind::Select => self.act_select(params).await,
        }
    }

    async fn act_select(&self, params: &ActParams) -> Result<String> {
        let ref_id = params
            .ref_id
            .ok_or_else(|| anyhow!("act.select requires 'ref' parameter"))?;
        let values = params.values.clone().ok_or_else(|| {
            anyhow!("act.select requires 'values' parameter (array of option values)")
        })?;
        if values.is_empty() {
            return Err(anyhow!(
                "act.select 'values' must contain at least one entry"
            ));
        }
        let state = get_browser_state().lock().await;
        let info = state.find_ref(ref_id)?.clone();
        let page = state.get_active_page()?.clone();
        drop(state);

        let values_json = serde_json::to_string(&values)
            .map_err(|e| anyhow!("Failed to serialize select values: {}", e))?;
        let escaped_selector = info.selector.replace('\\', "\\\\").replace('\'', "\\'");
        // Sets the <select> value(s) and dispatches change+input events so
        // listeners (React etc.) see the update.
        let js = format!(
            r#"(() => {{
                const el = document.querySelector('{selector}');
                if (!el || el.tagName !== 'SELECT') return {{ ok: false, reason: 'not_a_select' }};
                const values = {values};
                if (el.multiple) {{
                    for (const opt of Array.from(el.options)) {{
                        opt.selected = values.includes(opt.value);
                    }}
                }} else {{
                    el.value = values[0];
                }}
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                return {{ ok: true }};
            }})()"#,
            selector = escaped_selector,
            values = values_json,
        );
        let raw: Value = page
            .evaluate(js)
            .await
            .map_err(|e| anyhow!("Select script evaluation failed: {}", e))?
            .into_value()
            .map_err(|e| anyhow!("Select script returned non-JSON: {}", e))?;
        let ok = raw.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if !ok {
            let reason = raw
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(anyhow!(
                "act.select failed: ref={} is not a <select> element ({})",
                ref_id,
                reason
            ));
        }
        Ok(format!(
            "Selected {} value(s) on [ref={}] {}: {:?}",
            values.len(),
            ref_id,
            info.role,
            values
        ))
    }

    async fn act_click(&self, params: &ActParams, double: bool) -> Result<String> {
        let ref_id = params
            .ref_id
            .ok_or_else(|| anyhow!("act.click requires 'ref' parameter"))?;
        let state = get_browser_state().lock().await;
        let info = state.find_ref(ref_id)?.clone();
        let page = state.get_active_page()?.clone();
        drop(state);
        let el = page.find_element(&info.selector).await.map_err(|e| {
            anyhow!(
                "Element ref={} (selector: {}) not found: {}",
                ref_id,
                info.selector,
                e
            )
        })?;
        el.scroll_into_view().await.ok();
        el.click()
            .await
            .map_err(|e| anyhow!("Click failed: {}", e))?;
        if double {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            el.click().await.ok();
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        Ok(format!(
            "Clicked{} [ref={}] {} \"{}\"",
            if double { " (double)" } else { "" },
            ref_id,
            info.role,
            info.text
        ))
    }

    async fn act_fill(&self, params: &ActParams) -> Result<String> {
        let ref_id = params
            .ref_id
            .ok_or_else(|| anyhow!("act.fill requires 'ref' parameter"))?;
        let value = params
            .text
            .as_deref()
            .ok_or_else(|| anyhow!("act.fill requires 'text' parameter"))?;
        let state = get_browser_state().lock().await;
        let info = state.find_ref(ref_id)?.clone();
        let page = state.get_active_page()?.clone();
        drop(state);
        let el = page
            .find_element(&info.selector)
            .await
            .map_err(|e| anyhow!("Element ref={} not found: {}", ref_id, e))?;
        el.scroll_into_view().await.ok();
        el.click().await.ok();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let clear_js = format!(
            "(() => {{ const el = document.querySelector('{}'); if (el) {{ el.value = ''; el.dispatchEvent(new Event('input', {{bubbles: true}})); }} }})()",
            info.selector.replace('\'', "\\'")
        );
        page.evaluate(clear_js).await.ok();
        el.type_str(value)
            .await
            .map_err(|e| anyhow!("Failed to type text: {}", e))?;
        Ok(format!(
            "Filled [ref={}] {} with \"{}\"",
            ref_id, info.role, value
        ))
    }

    async fn act_hover(&self, params: &ActParams) -> Result<String> {
        let ref_id = params
            .ref_id
            .ok_or_else(|| anyhow!("act.hover requires 'ref' parameter"))?;
        let state = get_browser_state().lock().await;
        let info = state.find_ref(ref_id)?.clone();
        let page = state.get_active_page()?.clone();
        drop(state);
        let el = page
            .find_element(&info.selector)
            .await
            .map_err(|e| anyhow!("Element ref={} not found: {}", ref_id, e))?;
        el.scroll_into_view().await.ok();
        let point = el
            .clickable_point()
            .await
            .map_err(|e| anyhow!("Cannot get element position: {}", e))?;
        page.execute(DispatchMouseEventParams::new(
            DispatchMouseEventType::MouseMoved,
            point.x,
            point.y,
        ))
        .await
        .map_err(|e| anyhow!("Hover failed: {}", e))?;
        Ok(format!(
            "Hovered [ref={}] {} \"{}\"",
            ref_id, info.role, info.text
        ))
    }

    async fn act_drag(&self, params: &ActParams) -> Result<String> {
        let from_ref = params
            .ref_id
            .ok_or_else(|| anyhow!("act.drag requires 'ref' parameter (source)"))?;
        let to_ref = params
            .target_ref
            .ok_or_else(|| anyhow!("act.drag requires 'target_ref' parameter (destination)"))?;
        let state = get_browser_state().lock().await;
        let from = state.find_ref(from_ref)?.clone();
        let to = state.find_ref(to_ref)?.clone();
        let page = state.get_active_page()?.clone();
        drop(state);
        let from_el = page
            .find_element(&from.selector)
            .await
            .map_err(|e| anyhow!("Source ref={} not found: {}", from_ref, e))?;
        let to_el = page
            .find_element(&to.selector)
            .await
            .map_err(|e| anyhow!("Target ref={} not found: {}", to_ref, e))?;
        let from_point = from_el.clickable_point().await?;
        let to_point = to_el.clickable_point().await?;
        let mut down = DispatchMouseEventParams::new(
            DispatchMouseEventType::MousePressed,
            from_point.x,
            from_point.y,
        );
        down.button = Some(MouseButton::Left);
        down.click_count = Some(1);
        page.execute(down).await?;
        let mut mv = DispatchMouseEventParams::new(
            DispatchMouseEventType::MouseMoved,
            to_point.x,
            to_point.y,
        );
        mv.button = Some(MouseButton::Left);
        page.execute(mv).await?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let mut up = DispatchMouseEventParams::new(
            DispatchMouseEventType::MouseReleased,
            to_point.x,
            to_point.y,
        );
        up.button = Some(MouseButton::Left);
        up.click_count = Some(1);
        page.execute(up).await?;
        Ok(format!(
            "Dragged [ref={}] \"{}\" -> [ref={}] \"{}\"",
            from_ref, from.text, to_ref, to.text
        ))
    }

    async fn act_press_key(&self, params: &ActParams) -> Result<String> {
        let key = params
            .key
            .as_deref()
            .ok_or_else(|| anyhow!("act.press requires 'key' parameter (e.g. 'Enter')"))?;
        let page = {
            let state = get_browser_state().lock().await;
            state.get_active_page()?.clone()
        };
        let mut down = DispatchKeyEventParams::new(DispatchKeyEventType::KeyDown);
        down.key = Some(key.to_string());
        page.execute(down)
            .await
            .map_err(|e| anyhow!("Key press failed: {}", e))?;
        let mut up = DispatchKeyEventParams::new(DispatchKeyEventType::KeyUp);
        up.key = Some(key.to_string());
        page.execute(up).await.ok();
        Ok(format!("Pressed key: {}", key))
    }

    async fn act_upload(&self, params: &ActParams) -> Result<String> {
        use chromiumoxide::cdp::browser_protocol::dom::{
            GetDocumentParams, QuerySelectorParams, SetFileInputFilesParams,
        };
        let ref_id = params
            .ref_id
            .ok_or_else(|| anyhow!("act.upload requires 'ref' parameter"))?;
        let file_path = params
            .file_path
            .as_deref()
            .ok_or_else(|| anyhow!("act.upload requires 'file_path' parameter"))?;
        let authorised = super::authorise_upload_path(file_path)?;
        let file_path = authorised.to_string_lossy().into_owned();
        let state = get_browser_state().lock().await;
        let info = state.find_ref(ref_id)?.clone();
        let page = state.get_active_page()?.clone();
        drop(state);
        let doc = page
            .execute(GetDocumentParams::default())
            .await
            .map_err(|e| anyhow!("Failed to get document: {}", e))?;
        let node_id = doc.result.root.node_id;
        let q = page
            .execute(QuerySelectorParams::new(node_id, &info.selector))
            .await
            .map_err(|e| anyhow!("Element ref={} not found: {}", ref_id, e))?;
        let mut set_files = SetFileInputFilesParams::new(vec![file_path.clone()]);
        set_files.node_id = Some(q.result.node_id);
        page.execute(set_files)
            .await
            .map_err(|e| anyhow!("Failed to set file: {}", e))?;
        Ok(format!("Uploaded file '{}' to [ref={}]", file_path, ref_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_ref_error_classifier_recognises_common_phrasings() {
        for msg in [
            "Element ref=12 not found on page",
            "No such element with selector .foo",
            "stale ref",
            "element is detached from DOM",
        ] {
            let e = anyhow!("{}", msg);
            assert!(
                is_stale_ref_error(&e),
                "expected stale-ref classify for: {msg}"
            );
        }
    }
}
