use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::browser::backend::{
    ActKind, ActParams, BackendStatus, BrowserBackend, DialogAction, ElementRef, ImageFormat,
    ObserveEntry, ObserveKind, PdfParams, RawCdpParams, ScreenshotParams, ScrollDirection,
    ScrollParams, Snapshot, SnapshotFormat, TabInfo, WaitParams,
};

use super::broker::BrowserExtensionBroker;
use super::events::{
    emit_control_stopped, emit_control_stopped_for_scope, scope_for_context,
    BrowserControlStoppedPayload, BrowserControlStoppedReason,
};
use super::registry::{self, ElementLocator, FinalizeTabAction, TabOwnerKind};
use super::BrowserBackendContext;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserExtensionStopResult {
    pub stopped_tabs: usize,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameSnapshot {
    frame_id: i64,
    #[serde(default)]
    document_id: Option<String>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    top_accessible: bool,
    #[serde(default)]
    viewport: FrameViewport,
    #[serde(default)]
    elements: Vec<FrameSnapshotElement>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameViewport {
    #[serde(default)]
    w: u32,
    #[serde(default)]
    h: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameSnapshotElement {
    #[serde(default)]
    depth: u32,
    #[serde(default)]
    role: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    selector: String,
    #[serde(default)]
    attrs: HashMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlatSessionPayload {
    #[serde(default)]
    sessions: Vec<FlatSessionInfo>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlatSessionInfo {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    target_info: FlatTargetInfo,
    #[serde(default)]
    matched_frame: Option<FlatMatchedFrame>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlatTargetInfo {
    #[serde(default)]
    target_id: String,
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlatMatchedFrame {
    #[serde(default)]
    status: String,
    #[serde(default)]
    frame_id: Option<i64>,
}

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
  function buildUniqueSelector(el, rootDoc) {
    if (el.id) return '#' + CSS.escape(el.id);
    const path = [];
    let current = el;
    while (current && current !== rootDoc.body && path.length < 5) {
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
        if (siblings.length > 1) selector += ':nth-of-type(' + (siblings.indexOf(current) + 1) + ')';
      }
      path.unshift(selector);
      current = current.parentElement;
    }
    return path.join(' > ');
  }
  function walk(el, depth, frameChain, rootDoc) {
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
        depth,
        role: getRole(el),
        text: getText(el),
        selector: frameChain.concat([buildUniqueSelector(el, rootDoc)]).join(' >>> '),
        attrs: {
          bounds: [rect.left, rect.top, rect.width, rect.height].map(n => Math.round(n)).join(',')
        }
      };
      if (frameChain.length) info.attrs.frame = 'iframe';
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
      const childTag = child.tagName ? child.tagName.toLowerCase() : '';
      if (childTag === 'iframe') {
        try {
          const frameDoc = child.contentDocument || (child.contentWindow && child.contentWindow.document);
          if (frameDoc && frameDoc.body) {
            const frameSelector = buildUniqueSelector(child, rootDoc);
            walk(frameDoc.body, depth + 1, frameChain.concat([frameSelector]), frameDoc);
          }
        } catch (e) {
          // Cross-origin / OOPIF frames are handled by the future flat-session path.
        }
      }
      walk(child, depth + (interactive || semantic ? 1 : 0), frameChain, rootDoc);
    }
  }
  walk(document.body, 0, [], document);
  return JSON.stringify({
    url: location.href,
    title: document.title,
    viewport: { w: window.innerWidth, h: window.innerHeight },
    elements: refs,
    truncated: refId >= MAX_ELEMENTS
  });
})()"#;

const SELECTOR_HELPER_JS: &str = r#"
function __hopeResolveSelector(selector) {
  const parts = String(selector).split(" >>> ").filter(Boolean);
  let doc = document;
  const frameRects = [];
  for (let i = 0; i < parts.length; i++) {
    const el = doc.querySelector(parts[i]);
    if (!el) return null;
    if (i === parts.length - 1) return { el, frameRects };
    const tag = el.tagName ? el.tagName.toLowerCase() : "";
    if (tag !== "iframe") throw new Error("Frame selector segment did not resolve to iframe");
    frameRects.push(el.getBoundingClientRect());
    doc = el.contentDocument || (el.contentWindow && el.contentWindow.document);
    if (!doc) throw new Error("Cannot access iframe document for selector");
  }
  return null;
}
function __hopeAbsoluteRect(el, frameRects) {
  const rect = el.getBoundingClientRect();
  let left = rect.left;
  let top = rect.top;
  for (const frameRect of frameRects) {
    left += frameRect.left;
    top += frameRect.top;
  }
  return { left, top, width: rect.width, height: rect.height };
}
"#;

const AX_ENRICH_LIMIT: usize = 80;
const AX_ONLY_LIMIT: usize = 120;
const FLAT_SESSION_AX_SESSION_LIMIT: usize = 8;
const FLAT_SESSION_AX_ONLY_LIMIT: usize = 80;
const AX_BACKEND_DOM_NODE_LOCATOR_PREFIX: &str = "ax_backend_dom_node_id:";
const FRAME_AX_BACKEND_DOM_NODE_LOCATOR_PREFIX: &str = "frame_ax_backend_dom_node_id:";
const FRAME_LOCATOR_PREFIX: &str = "frame:";

const ALLOWED_CDP_METHODS: &[&str] = &[
    "Accessibility.getFullAXTree",
    "Accessibility.getPartialAXTree",
    "DOM.resolveNode",
    "DOM.setFileInputFiles",
    "Emulation.setDeviceMetricsOverride",
    "Input.dispatchMouseEvent",
    "Network.enable",
    "Page.captureScreenshot",
    "Page.handleJavaScriptDialog",
    "Page.printToPDF",
    "Page.reload",
    "Runtime.enable",
    "Runtime.evaluate",
    "Runtime.callFunctionOn",
    "Runtime.releaseObjectGroup",
];

const BLOCKED_CDP_DOMAIN_PREFIXES: &[&str] = &[
    "Browser.",
    "CacheStorage.",
    "Database.",
    "Fetch.",
    "HeapProfiler.",
    "IndexedDB.",
    "IO.",
    "Profiler.",
    "Security.",
    "Storage.",
    "SystemInfo.",
    "Target.",
    "Tracing.",
];

/// Method-level blocklist for the `raw_cdp` escape hatch. `raw_cdp` deliberately
/// bypasses [`ALLOWED_CDP_METHODS`] — its whole purpose is to reach advanced
/// methods the curated path doesn't expose — but it must still honor the safety
/// blocklist. The `Network.` domain is intentionally NOT in
/// [`BLOCKED_CDP_DOMAIN_PREFIXES`] (because `Network.enable` is legitimate), so
/// the dangerous `Network.*` methods are enumerated here: cookie/credential
/// read-write, plus traffic tampering (header injection, request interception)
/// against the user's real logged-in tabs. (`Fetch.*`, the modern request
/// interception domain, is blocked wholesale via [`BLOCKED_CDP_DOMAIN_PREFIXES`].)
const BLOCKED_RAW_CDP_METHODS: &[&str] = &[
    // Cookie / credential read-write.
    "Network.getCookies",
    "Network.getAllCookies",
    "Network.setCookie",
    "Network.setCookies",
    "Network.deleteCookies",
    "Network.clearBrowserCookies",
    "Page.getCookies",
    // Traffic tampering / header & request forgery on real tabs.
    "Network.setExtraHTTPHeaders",
    "Network.setRequestInterception",
    "Network.continueInterceptedRequest",
    "Network.setBlockedURLs",
];

pub struct ExtensionBackend {
    broker: Arc<BrowserExtensionBroker>,
    ctx: BrowserBackendContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AxElementInfo {
    role: String,
    name: Option<String>,
    value: Option<String>,
}

#[derive(Debug, Clone)]
struct AxOnlySnapshotElement {
    element: ElementRef,
    locator: Option<ElementLocator>,
}

#[derive(Debug, Clone)]
struct FrameClip {
    clip: Value,
    url: String,
    title: String,
}

#[derive(Debug, Clone, PartialEq)]
struct PointerTarget {
    session_id: Option<String>,
    x: f64,
    y: f64,
}

impl ExtensionBackend {
    pub fn new(broker: Arc<BrowserExtensionBroker>, ctx: BrowserBackendContext) -> Self {
        Self { broker, ctx }
    }

    async fn tabs_query(&self, query: Value) -> Result<Vec<TabInfo>> {
        let result = self
            .broker
            .call("tabs.query", json!({ "query": query }))
            .await?;
        let tabs = result
            .as_array()
            .ok_or_else(|| anyhow!("Chrome Extension tabs.query returned non-array result"))?;
        Ok(tabs.iter().filter_map(tab_from_value).collect())
    }

    async fn tabs_query_all_reconciled(&self) -> Result<Vec<TabInfo>> {
        let tabs = self.tabs_query(json!({})).await?;
        self.reconcile_live_tabs(&tabs);
        Ok(tabs)
    }

    fn reconcile_live_tabs(&self, tabs: &[TabInfo]) {
        let live_tab_ids = tabs
            .iter()
            .filter_map(|tab| parse_tab_id(&tab.target_id).ok())
            .collect::<HashSet<_>>();
        let removed = registry::reconcile_live_tabs(&live_tab_ids);
        for lease in removed {
            emit_control_stopped_for_scope(
                lease.tab_id,
                lease.scope,
                BrowserControlStoppedReason::TabClosed,
                lease.owner_kind == TabOwnerKind::Agent,
            );
        }
    }

    async fn active_tab_id(&self) -> Result<i64> {
        if let Some(tab_id) = registry::active_tab_id(&self.ctx) {
            return Ok(tab_id);
        }
        Err(anyhow!(
            "No controlled Chrome tab is active for this Hope session. Use tabs.new or tabs.claim first."
        ))
    }

    fn record_agent_tab(&self, tab: &TabInfo) -> Result<()> {
        let tab_id = parse_tab_id(&tab.target_id)?;
        registry::record_agent_tab(
            &self.ctx,
            tab_id,
            Some(tab.url.clone()),
            Some(tab.title.clone()),
        )
    }

    async fn show_overlay(&self, tab_id: i64) {
        if !control_overlay_enabled() {
            return;
        }
        if let Err(e) = self
            .broker
            .call(
                // No label on purpose: the extension localizes the overlay text
                // to the user's Chrome UI language via chrome.i18n. Core can't
                // know the browser locale, so sending a fixed English string
                // here would override that and force English for everyone.
                "overlay.show",
                json!({ "tabId": tab_id }),
            )
            .await
        {
            app_debug!(
                "browser",
                "extension_backend",
                "overlay.show failed for tab {}: {}",
                tab_id,
                e
            );
        }
    }

    async fn hide_overlay(&self, tab_id: i64) {
        if let Err(e) = hide_overlay_with_broker(&self.broker, tab_id).await {
            app_debug!(
                "browser",
                "extension_backend",
                "overlay.hide failed for tab {}: {}",
                tab_id,
                e
            );
        }
    }

    async fn claim_user_tab(&self, tab: &TabInfo, steal: bool) -> Result<()> {
        let tab_id = parse_tab_id(&tab.target_id)?;
        let outcome = registry::claim_user_tab(
            &self.ctx,
            tab_id,
            Some(tab.url.clone()),
            Some(tab.title.clone()),
            steal,
        )?;
        if !outcome.stolen_from.is_empty() {
            let _ = self.detach_debugger(tab_id).await;
            let stopped_by_scope = scope_for_context(&self.ctx);
            for stolen_scope in &outcome.stolen_from {
                emit_control_stopped(
                    BrowserControlStoppedPayload::for_scope(
                        tab_id,
                        stolen_scope.clone(),
                        BrowserControlStoppedReason::LeaseStolen,
                        false,
                    )
                    .stopped_by(stopped_by_scope.clone()),
                );
            }
            app_info!(
                "browser",
                "extension_backend",
                "stole Chrome tab {} lease from {:?}",
                tab_id,
                outcome.stolen_from
            );
        }
        self.show_overlay(tab_id).await;
        Ok(())
    }

    async fn claim_page_inner(&self, target_id: &str, steal: bool) -> Result<()> {
        let tab_id = parse_tab_id(target_id)?;
        self.broker
            .call(
                "tabs.update",
                json!({
                    "tabId": tab_id,
                    "update": { "active": true }
                }),
            )
            .await?;
        let selected = self
            .tabs_query_all_reconciled()
            .await?
            .into_iter()
            .find(|tab| tab.target_id == target_id)
            .unwrap_or(TabInfo {
                target_id: target_id.to_string(),
                url: String::new(),
                title: String::new(),
                is_active: true,
            });
        self.claim_user_tab(&selected, steal).await
    }

    async fn detach_debugger(&self, tab_id: i64) -> Result<()> {
        detach_debugger_with_broker(&self.broker, tab_id).await
    }

    async fn attach_debugger(&self, tab_id: i64) -> Result<()> {
        match self
            .broker
            .call(
                "debugger.attach",
                json!({
                    "tabId": tab_id,
                    "version": "1.3"
                }),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(e)
                if e.to_string()
                    .to_ascii_lowercase()
                    .contains("already attached") =>
            {
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    async fn cdp_command(&self, tab_id: i64, command: &str, params: Value) -> Result<Value> {
        validate_cdp_method(command)?;
        self.send_cdp_command(tab_id, command, params).await
    }

    async fn cdp_command_for_session(
        &self,
        tab_id: i64,
        session_id: &str,
        command: &str,
        params: Value,
    ) -> Result<Value> {
        validate_cdp_method(command)?;
        self.attach_debugger(tab_id).await?;
        self.broker
            .call(
                "debugger.sendCommand",
                json!({
                    "tabId": tab_id,
                    "sessionId": session_id,
                    "command": command,
                    "params": params
                }),
            )
            .await
    }

    async fn raw_cdp_command(&self, tab_id: i64, command: &str, params: Value) -> Result<Value> {
        validate_raw_cdp_method(command)?;
        self.send_cdp_command(tab_id, command, params).await
    }

    async fn send_cdp_command(&self, tab_id: i64, command: &str, params: Value) -> Result<Value> {
        self.attach_debugger(tab_id).await?;
        self.broker
            .call(
                "debugger.sendCommand",
                json!({
                    "tabId": tab_id,
                    "command": command,
                    "params": params
                }),
            )
            .await
    }

    async fn flat_session_diagnostics(&self, tab_id: i64) -> Result<Value> {
        self.attach_debugger(tab_id).await?;
        self.broker
            .call("debugger.sessions", json!({ "tabId": tab_id }))
            .await
    }

    async fn flat_sessions(&self, tab_id: i64) -> Result<Vec<FlatSessionInfo>> {
        let value = self.flat_session_diagnostics(tab_id).await?;
        let payload: FlatSessionPayload = serde_json::from_value(value)
            .map_err(|e| anyhow!("Extension debugger.sessions returned invalid payload: {e}"))?;
        Ok(payload.sessions)
    }

    async fn evaluate_on_tab(&self, tab_id: i64, script: &str) -> Result<Value> {
        let result = self
            .cdp_command(
                tab_id,
                "Runtime.evaluate",
                json!({
                    "expression": script,
                    "returnByValue": true,
                    "awaitPromise": true
                }),
            )
            .await?;
        runtime_result_value(&result)
    }

    async fn frame_snapshots(&self, tab_id: i64) -> Result<Vec<FrameSnapshot>> {
        let value = self
            .broker
            .call(
                "frames.snapshot",
                json!({
                    "tabId": tab_id,
                    "maxElements": 160
                }),
            )
            .await?;
        serde_json::from_value(value)
            .map_err(|e| anyhow!("Extension frames.snapshot returned invalid payload: {e}"))
    }

    async fn cdp_binary_result_bytes(
        &self,
        result: &Value,
        label: &str,
        expected_purpose: &str,
        allowed_mimes: &[&str],
    ) -> Result<Vec<u8>> {
        use base64::Engine;

        if let Some(blob) = result.get("dataBlob").or_else(|| result.get("data_blob")) {
            return self
                .broker
                .take_blob_bytes(blob, expected_purpose, allowed_mimes)
                .await;
        }
        let data = result
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("{label} returned no data"))?;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| anyhow!("{label} base64 decode failed: {e}"))
    }

    async fn act_frame(
        &self,
        tab_id: i64,
        frame_id: i64,
        selector: &str,
        kind: ActKind,
        params: &ActParams,
        ref_id: u32,
    ) -> Result<String> {
        let mut frame_params = frame_action_params(params);
        match kind {
            ActKind::Click
            | ActKind::DoubleClick
            | ActKind::Hover
            | ActKind::Fill
            | ActKind::Select
            | ActKind::Press => {}
            ActKind::Drag => {
                let target_ref = params
                    .target_ref
                    .ok_or_else(|| anyhow!("act.drag requires 'target_ref' parameter"))?;
                let target = self.selector_for_ref(target_ref).await?;
                let target_selector = same_frame_drag_target_selector(frame_id, &target)?;
                if let Some(obj) = frame_params.as_object_mut() {
                    obj.insert(
                        "targetSelector".to_string(),
                        Value::String(target_selector.to_string()),
                    );
                }
            }
            ActKind::Upload => {
                bail!("act.upload is not supported for cross-origin iframe refs yet")
            }
        }
        let result = self
            .broker
            .call(
                "frames.act",
                json!({
                    "tabId": tab_id,
                    "frameId": frame_id,
                    "selector": selector,
                    "kind": act_kind_wire_name(kind),
                    "params": frame_params,
                }),
            )
            .await?;
        let message = result
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Frame action completed");
        Ok(format!("{message} [ref={ref_id}] in iframe"))
    }

    async fn frame_element_clip_on_tab(
        &self,
        tab_id: i64,
        frame_id: i64,
        selector: &str,
    ) -> Result<FrameClip> {
        let result = self
            .broker
            .call(
                "frames.act",
                json!({
                    "tabId": tab_id,
                    "frameId": frame_id,
                    "selector": selector,
                    "kind": "clip",
                    "params": {},
                }),
            )
            .await?;
        parse_frame_clip_result(&result)
    }

    async fn capture_frame_element_screenshot(
        &self,
        tab_id: i64,
        frame_id: i64,
        selector: &str,
        params: ScreenshotParams,
        format: &str,
    ) -> Result<Vec<u8>> {
        let frame_clip = self
            .frame_element_clip_on_tab(tab_id, frame_id, selector)
            .await?;
        let sessions = self.flat_sessions(tab_id).await?;
        let session_id = select_flat_session_for_frame_context(
            &sessions,
            frame_id,
            &frame_clip.url,
            &frame_clip.title,
        )?;
        let result = self
            .cdp_command_for_session(
                tab_id,
                session_id,
                "Page.captureScreenshot",
                json!({
                    "format": format,
                    "captureBeyondViewport": true,
                    "quality": params.quality,
                    "clip": frame_clip.clip,
                }),
            )
            .await?;
        self.cdp_binary_result_bytes(
            &result,
            "Page.captureScreenshot",
            "screenshot",
            &[params.format.mime()],
        )
        .await
    }

    async fn resolve_object_id_for_locator(
        &self,
        tab_id: i64,
        locator: &str,
        object_group: &str,
    ) -> Result<String> {
        if frame_locator_parts(locator).is_some() {
            bail!("Cross-origin iframe refs cannot be resolved through the root CDP session");
        }
        if let Some(backend_node_id) = ax_backend_dom_node_id_from_locator(locator) {
            let object = self
                .cdp_command(
                    tab_id,
                    "DOM.resolveNode",
                    json!({
                        "backendNodeId": backend_node_id,
                        "objectGroup": object_group
                    }),
                )
                .await?;
            return object
                .get("object")
                .and_then(|object| object.get("objectId"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .ok_or_else(|| anyhow!("AX backend DOM node could not be resolved"));
        }

        let selector = serde_json::to_string(locator)?;
        let object = self
            .cdp_command(
                tab_id,
                "Runtime.evaluate",
                json!({
                    "expression": format!(
                        r#"(() => {{
                          {selector_helper}
                          const resolved = __hopeResolveSelector({selector});
                          return resolved ? resolved.el : null;
                        }})()"#,
                        selector_helper = SELECTOR_HELPER_JS
                    ),
                    "returnByValue": false,
                    "objectGroup": object_group
                }),
            )
            .await?;
        object
            .get("result")
            .and_then(|r| r.get("objectId"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| anyhow!("Element not found for locator"))
    }

    async fn call_function_on_locator(
        &self,
        tab_id: i64,
        locator: &str,
        function_declaration: &str,
        arguments: Vec<Value>,
        object_group: &str,
    ) -> Result<Value> {
        let object_id = self
            .resolve_object_id_for_locator(tab_id, locator, object_group)
            .await?;
        let result = self
            .cdp_command(
                tab_id,
                "Runtime.callFunctionOn",
                json!({
                    "objectId": object_id,
                    "functionDeclaration": function_declaration,
                    "arguments": arguments,
                    "returnByValue": true,
                    "awaitPromise": true
                }),
            )
            .await;
        let _ = self
            .cdp_command(
                tab_id,
                "Runtime.releaseObjectGroup",
                json!({ "objectGroup": object_group }),
            )
            .await;
        runtime_result_value(&result?)
    }

    async fn resolve_object_id_for_frame_ax_locator(
        &self,
        tab_id: i64,
        locator: &str,
        object_group: &str,
    ) -> Result<(String, String)> {
        let Some((session_id, backend_node_id)) =
            frame_ax_backend_dom_node_id_from_locator(locator)
        else {
            bail!("Expected frame AX backend DOM node locator");
        };
        let object = self
            .cdp_command_for_session(
                tab_id,
                &session_id,
                "DOM.resolveNode",
                json!({
                    "backendNodeId": backend_node_id,
                    "objectGroup": object_group
                }),
            )
            .await?;
        let object_id = object
            .get("object")
            .and_then(|o| o.get("objectId"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("DOM.resolveNode returned no objectId for frame AX locator"))?
            .to_string();
        Ok((session_id, object_id))
    }

    async fn call_function_on_frame_ax_locator(
        &self,
        tab_id: i64,
        locator: &str,
        function_declaration: &str,
        arguments: Vec<Value>,
        object_group: &str,
    ) -> Result<Value> {
        let (session_id, object_id) = self
            .resolve_object_id_for_frame_ax_locator(tab_id, locator, object_group)
            .await?;
        let result = self
            .cdp_command_for_session(
                tab_id,
                &session_id,
                "Runtime.callFunctionOn",
                json!({
                    "objectId": object_id,
                    "functionDeclaration": function_declaration,
                    "arguments": arguments,
                    "returnByValue": true,
                    "awaitPromise": true
                }),
            )
            .await;
        let _ = self
            .cdp_command_for_session(
                tab_id,
                &session_id,
                "Runtime.releaseObjectGroup",
                json!({ "objectGroup": object_group }),
            )
            .await;
        runtime_result_value(&result?)
    }

    async fn full_accessibility_tree(&self, tab_id: i64) -> Result<Value> {
        self.cdp_command(tab_id, "Accessibility.getFullAXTree", json!({}))
            .await
    }

    async fn full_accessibility_tree_for_session(
        &self,
        tab_id: i64,
        session_id: &str,
    ) -> Result<Value> {
        self.cdp_command_for_session(tab_id, session_id, "Accessibility.getFullAXTree", json!({}))
            .await
    }

    async fn accessibility_for_selector(
        &self,
        tab_id: i64,
        selector: &str,
    ) -> Result<Option<AxElementInfo>> {
        if frame_locator_parts(selector).is_some() {
            return Ok(None);
        }
        if selector.trim().is_empty() {
            return Ok(None);
        }
        let selector = serde_json::to_string(selector)?;
        let object_group = "hope-agent-ax-snapshot";
        let object = self
            .cdp_command(
                tab_id,
                "Runtime.evaluate",
                json!({
                    "expression": format!(
                        r#"(() => {{
                          {selector_helper}
                          const resolved = __hopeResolveSelector({selector});
                          return resolved ? resolved.el : null;
                        }})()"#,
                        selector_helper = SELECTOR_HELPER_JS
                    ),
                    "returnByValue": false,
                    "objectGroup": object_group
                }),
            )
            .await?;
        let object_id = match object
            .get("result")
            .and_then(|r| r.get("objectId"))
            .and_then(Value::as_str)
        {
            Some(object_id) => object_id.to_string(),
            None => return Ok(None),
        };
        let ax_tree = self
            .cdp_command(
                tab_id,
                "Accessibility.getPartialAXTree",
                json!({
                    "objectId": object_id,
                    "fetchRelatives": false
                }),
            )
            .await;
        let _ = self
            .cdp_command(
                tab_id,
                "Runtime.releaseObjectGroup",
                json!({ "objectGroup": object_group }),
            )
            .await;
        ax_tree.map(|tree| parse_ax_element_info(&tree))
    }

    async fn selector_for_ref(&self, ref_id: u32) -> Result<ElementLocator> {
        registry::selector_for_ref(&self.ctx, ref_id)
    }

    async fn resolve_element_for_action(
        &self,
        ref_id: u32,
        expected: Option<&ElementLocator>,
    ) -> Result<(i64, ElementLocator)> {
        let tab_id = self.active_tab_id().await?;
        if let Some(expected) = expected {
            if let Some(recovered) =
                registry::find_ref_by_role_text(&self.ctx, &expected.role, &expected.text)
            {
                return Ok((tab_id, recovered));
            }
        }
        Ok((tab_id, self.selector_for_ref(ref_id).await?))
    }

    async fn act_once(
        &self,
        kind: ActKind,
        params: &ActParams,
        recovered_from: Option<&ElementLocator>,
    ) -> Result<String> {
        let ref_id = params
            .ref_id
            .ok_or_else(|| anyhow!("ExtensionBackend act requires 'ref'"))?;
        let (tab_id, element) = self
            .resolve_element_for_action(ref_id, recovered_from)
            .await?;
        if let Some((frame_id, frame_selector)) = frame_locator_parts(&element.selector) {
            if kind == ActKind::Drag {
                let target_ref = params
                    .target_ref
                    .ok_or_else(|| anyhow!("act.drag requires 'target_ref' parameter"))?;
                let target = self.selector_for_ref(target_ref).await?;
                if same_frame_drag_target_selector(frame_id, &target).is_err() {
                    return self
                        .act_drag_with_target(tab_id, ref_id, &element, target_ref, &target)
                        .await;
                }
            }
            return self
                .act_frame(tab_id, frame_id, frame_selector, kind, params, ref_id)
                .await;
        }
        let selector = serde_json::to_string(&element.selector)?;
        if kind == ActKind::Drag {
            return self.act_drag(tab_id, ref_id, params, &element).await;
        }
        if kind == ActKind::Upload {
            let file_path = params
                .file_path
                .as_deref()
                .ok_or_else(|| anyhow!("act.upload requires 'file_path' parameter"))?;
            if frame_ax_backend_dom_node_id_from_locator(&element.selector).is_some() {
                bail!("act.upload is not supported for cross-origin iframe AX refs yet");
            }
            let authorised = crate::browser::authorise_upload_path(file_path)?;
            let file_path = authorised.to_string_lossy().into_owned();
            let object_group = "hope-agent-upload";
            let object_id = self
                .resolve_object_id_for_locator(tab_id, &element.selector, object_group)
                .await?;
            let result = self
                .cdp_command(
                    tab_id,
                    "DOM.setFileInputFiles",
                    json!({
                        "files": [file_path],
                        "objectId": object_id
                    }),
                )
                .await;
            let _ = self
                .cdp_command(
                    tab_id,
                    "Runtime.releaseObjectGroup",
                    json!({ "objectGroup": object_group }),
                )
                .await;
            result?;
            return Ok(format!("Uploaded file to [ref={}]", ref_id));
        }
        if ax_backend_dom_node_id_from_locator(&element.selector).is_some() {
            let (function_declaration, arguments) = build_object_action_call(kind, params)?;
            let value = self
                .call_function_on_locator(
                    tab_id,
                    &element.selector,
                    &function_declaration,
                    arguments,
                    "hope-agent-ax-action",
                )
                .await?;
            return Ok(value.as_str().unwrap_or("Action completed").to_string());
        }
        if frame_ax_backend_dom_node_id_from_locator(&element.selector).is_some() {
            let (function_declaration, arguments) = build_object_action_call(kind, params)?;
            let value = self
                .call_function_on_frame_ax_locator(
                    tab_id,
                    &element.selector,
                    &function_declaration,
                    arguments,
                    "hope-agent-frame-ax-action",
                )
                .await?;
            return Ok(format!(
                "{} in iframe",
                value.as_str().unwrap_or("Action completed")
            ));
        }
        let script = build_action_script(kind, params, &selector)?;
        let value = self.evaluate_on_tab(tab_id, &script).await?;
        Ok(value.as_str().unwrap_or("Action completed").to_string())
    }

    async fn act_drag(
        &self,
        tab_id: i64,
        ref_id: u32,
        params: &ActParams,
        source: &ElementLocator,
    ) -> Result<String> {
        let target_ref = params
            .target_ref
            .ok_or_else(|| anyhow!("act.drag requires 'target_ref' parameter"))?;
        let target = self.selector_for_ref(target_ref).await?;
        self.act_drag_with_target(tab_id, ref_id, source, target_ref, &target)
            .await
    }

    async fn act_drag_with_target(
        &self,
        tab_id: i64,
        ref_id: u32,
        source: &ElementLocator,
        target_ref: u32,
        target: &ElementLocator,
    ) -> Result<String> {
        let source_pointer = self
            .element_pointer_target_on_tab(tab_id, &source.selector, "Source")
            .await?;

        self.dispatch_pointer_event(
            tab_id,
            &source_pointer,
            "mousePressed",
            Some("left"),
            Some(1),
        )
        .await?;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let target_pointer = self
            .element_pointer_target_on_tab(tab_id, &target.selector, "Target")
            .await;
        let target_pointer = match target_pointer {
            Ok(point) => point,
            Err(err) => {
                let _ = self
                    .dispatch_pointer_event(
                        tab_id,
                        &source_pointer,
                        "mouseReleased",
                        Some("left"),
                        Some(1),
                    )
                    .await;
                return Err(err);
            }
        };
        for point in drag_move_points(&source_pointer, &target_pointer) {
            self.dispatch_pointer_event(tab_id, &point, "mouseMoved", Some("left"), None)
                .await?;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        self.dispatch_pointer_event(
            tab_id,
            &target_pointer,
            "mouseReleased",
            Some("left"),
            Some(1),
        )
        .await?;
        Ok(format!(
            "Dragged [ref={}] \"{}\" -> [ref={}] \"{}\"",
            ref_id, source.text, target_ref, target.text
        ))
    }

    async fn dispatch_pointer_event(
        &self,
        tab_id: i64,
        target: &PointerTarget,
        event_type: &str,
        button: Option<&str>,
        click_count: Option<u32>,
    ) -> Result<Value> {
        let mut params = json!({
            "type": event_type,
            "x": target.x,
            "y": target.y,
        });
        if let Some(button) = button {
            params["button"] = json!(button);
        }
        if let Some(click_count) = click_count {
            params["clickCount"] = json!(click_count);
        }
        if let Some(session_id) = target.session_id.as_deref() {
            self.cdp_command_for_session(tab_id, session_id, "Input.dispatchMouseEvent", params)
                .await
        } else {
            self.cdp_command(tab_id, "Input.dispatchMouseEvent", params)
                .await
        }
    }

    async fn element_pointer_target_on_tab(
        &self,
        tab_id: i64,
        selector: &str,
        label: &str,
    ) -> Result<PointerTarget> {
        if let Some((frame_id, frame_selector)) = frame_locator_parts(selector) {
            let frame_clip = self
                .frame_element_clip_on_tab(tab_id, frame_id, frame_selector)
                .await?;
            let sessions = self.flat_sessions(tab_id).await?;
            let session_id = select_flat_session_for_frame_context(
                &sessions,
                frame_id,
                &frame_clip.url,
                &frame_clip.title,
            )?;
            return pointer_target_from_clip(Some(session_id.to_string()), &frame_clip.clip);
        }
        if frame_ax_backend_dom_node_id_from_locator(selector).is_some() {
            let Some((session_id, _)) = frame_ax_backend_dom_node_id_from_locator(selector) else {
                bail!("Expected frame AX backend DOM node locator");
            };
            let point = self
                .call_function_on_frame_ax_locator(
                    tab_id,
                    selector,
                    r#"function() {
                      const el = this;
                      if (!el || !el.getBoundingClientRect) throw new Error("Element has no bounds");
                      el.scrollIntoView({block: "center", inline: "center"});
                      const rect = el.getBoundingClientRect();
                      return {
                        x: Math.max(0, Math.min(window.innerWidth - 1, rect.left + rect.width / 2)),
                        y: Math.max(0, Math.min(window.innerHeight - 1, rect.top + rect.height / 2))
                      };
                    }"#,
                    Vec::new(),
                    "hope-agent-frame-ax-pointer",
                )
                .await?;
            return Ok(PointerTarget {
                session_id: Some(session_id),
                x: point_coord(&point, "x")?,
                y: point_coord(&point, "y")?,
            });
        }
        let (x, y) = self.element_center_on_tab(tab_id, selector, label).await?;
        Ok(PointerTarget {
            session_id: None,
            x,
            y,
        })
    }

    async fn element_center_on_tab(
        &self,
        tab_id: i64,
        selector: &str,
        label: &str,
    ) -> Result<(f64, f64)> {
        if frame_locator_parts(selector).is_some() {
            bail!("{label} element is inside a cross-origin iframe; coordinate actions are not supported for that ref yet");
        }
        if ax_backend_dom_node_id_from_locator(selector).is_some() {
            let point = self
                .call_function_on_locator(
                    tab_id,
                    selector,
                    r#"function() {
                      const el = this;
                      if (!el || !el.getBoundingClientRect) throw new Error("Element has no bounds");
                      el.scrollIntoView({block: "center", inline: "center"});
                      const rect = el.getBoundingClientRect();
                      return {
                        x: Math.max(0, Math.min(window.innerWidth - 1, rect.left + rect.width / 2)),
                        y: Math.max(0, Math.min(window.innerHeight - 1, rect.top + rect.height / 2))
                      };
                    }"#,
                    Vec::new(),
                    "hope-agent-ax-rect",
                )
                .await?;
            return Ok((point_coord(&point, "x")?, point_coord(&point, "y")?));
        }
        let selector = serde_json::to_string(selector)?;
        let label = serde_json::to_string(label)?;
        let point = self
            .evaluate_on_tab(
                tab_id,
                &format!(
                    r#"(() => {{
                      {selector_helper}
                      const selector = {selector};
                      const label = {label};
                      const resolved = __hopeResolveSelector(selector);
                      if (!resolved) throw new Error(label + " element not found for selector");
                      const el = resolved.el;
                      el.scrollIntoView({{block: "center", inline: "center"}});
                      const rect = __hopeAbsoluteRect(el, resolved.frameRects);
                      return {{
                        x: Math.max(0, Math.min(window.innerWidth - 1, rect.left + rect.width / 2)),
                        y: Math.max(0, Math.min(window.innerHeight - 1, rect.top + rect.height / 2))
                      }};
                    }})()"#,
                    selector_helper = SELECTOR_HELPER_JS
                ),
            )
            .await?;
        Ok((point_coord(&point, "x")?, point_coord(&point, "y")?))
    }

    async fn element_clip_on_tab(&self, tab_id: i64, selector: &str) -> Result<Value> {
        if frame_locator_parts(selector).is_some() {
            bail!("Element screenshot crop is not supported for cross-origin iframe refs yet");
        }
        if ax_backend_dom_node_id_from_locator(selector).is_some() {
            let clip = self
                .call_function_on_locator(
                    tab_id,
                    selector,
                    r#"function() {
                      const el = this;
                      if (!el || !el.getBoundingClientRect) throw new Error("Element has no bounds");
                      el.scrollIntoView({block: "center", inline: "center"});
                      const rect = el.getBoundingClientRect();
                      if (rect.width <= 0 || rect.height <= 0) throw new Error("Element has empty bounds");
                      return {
                        x: Math.max(0, rect.left + window.scrollX),
                        y: Math.max(0, rect.top + window.scrollY),
                        width: Math.max(1, rect.width),
                        height: Math.max(1, rect.height),
                        scale: 1
                      };
                    }"#,
                    Vec::new(),
                    "hope-agent-ax-clip",
                )
                .await?;
            validate_clip(&clip)?;
            return Ok(clip);
        }
        let selector = serde_json::to_string(selector)?;
        let clip = self
            .evaluate_on_tab(
                tab_id,
                &format!(
                    r#"(() => {{
                      {selector_helper}
                      const resolved = __hopeResolveSelector({selector});
                      if (!resolved) throw new Error("Element not found for selector");
                      const el = resolved.el;
                      el.scrollIntoView({{block: "center", inline: "center"}});
                      const rect = __hopeAbsoluteRect(el, resolved.frameRects);
                      if (rect.width <= 0 || rect.height <= 0) throw new Error("Element has empty bounds");
                      return {{
                        x: Math.max(0, rect.left + window.scrollX),
                        y: Math.max(0, rect.top + window.scrollY),
                        width: Math.max(1, rect.width),
                        height: Math.max(1, rect.height),
                        scale: 1
                      }};
                    }})()"#,
                    selector_helper = SELECTOR_HELPER_JS
                ),
            )
            .await?;
        validate_clip(&clip)?;
        Ok(clip)
    }
}

#[async_trait]
impl BrowserBackend for ExtensionBackend {
    fn backend_name(&self) -> &'static str {
        "extension"
    }

    async fn is_connected(&self) -> bool {
        self.broker.is_extension_connected().await
    }

    async fn status(&self) -> Result<BackendStatus> {
        let connected = self.is_connected().await;
        // Status is a read-only inspection: use a plain tabs query, NOT
        // `list_pages()` (which calls `reconcile_live_tabs` — pruning leases
        // across every scope and emitting BrowserControlStopped). Reconciliation
        // belongs on action paths, not on a status read, otherwise a `status`
        // call could detach another session's browser control as a side effect.
        let tabs = self.tabs_query(json!({})).await?;
        let active_override = registry::active_tab_id(&self.ctx);
        let active_target_id = tabs
            .iter()
            .find(|tab| {
                active_override
                    .map(|id| tab.target_id == id.to_string())
                    .unwrap_or(tab.is_active)
            })
            .map(|tab| tab.target_id.clone());
        // Diagnostics attach the debugger (flat_session_diagnostics →
        // attach_debugger), so only run them for a tab THIS session has already
        // claimed (active_override). Probing the user's plain active tab here
        // would turn a read-only status into an unapproved debugger attach —
        // Chrome's debugging banner + lingering observe state on a tab the user
        // never handed us. (Codex review P2.)
        let diagnostics = match active_override {
            Some(tab_id) => self.flat_session_diagnostics(tab_id).await.ok(),
            None => None,
        };
        Ok(BackendStatus {
            connected,
            backend: self.backend_name().to_string(),
            active_target_id,
            tabs,
            diagnostics,
        })
    }

    async fn list_pages(&self) -> Result<Vec<TabInfo>> {
        self.tabs_query_all_reconciled().await
    }

    async fn active_tab_info(&self) -> Result<Option<TabInfo>> {
        if let Some(active_id) = registry::active_tab_id(&self.ctx) {
            return Ok(self
                .tabs_query_all_reconciled()
                .await?
                .into_iter()
                .find(|tab| tab.target_id == active_id.to_string()));
        }
        Ok(self
            .tabs_query(json!({ "active": true, "currentWindow": true }))
            .await?
            .into_iter()
            .next())
    }

    async fn new_page(&self, url: Option<&str>) -> Result<TabInfo> {
        let result = self
            .broker
            .call(
                "tabs.create",
                json!({
                    "url": url.unwrap_or("about:blank")
                }),
            )
            .await?;
        let tab =
            tab_from_value(&result).ok_or_else(|| anyhow!("tabs.create returned invalid tab"))?;
        self.record_agent_tab(&tab)?;
        self.show_overlay(parse_tab_id(&tab.target_id)?).await;
        Ok(tab)
    }

    async fn select_page(&self, target_id: &str) -> Result<()> {
        self.claim_page_inner(target_id, false).await
    }

    async fn claim_page(&self, target_id: &str, steal: bool) -> Result<()> {
        self.claim_page_inner(target_id, steal).await
    }

    async fn close_page(&self, target_id: &str) -> Result<()> {
        let tab_id = parse_tab_id(target_id)?;
        match registry::controlled_kind(&self.ctx, tab_id) {
            Some(TabOwnerKind::Agent) => {}
            Some(TabOwnerKind::User) => {
                bail!(
                    "Refusing to close claimed user Chrome tab {}. Use tabs.release or tabs.finalize to keep it open.",
                    target_id
                );
            }
            None => {
                bail!(
                    "Chrome tab {} is not controlled by this Hope session. Use tabs.claim first.",
                    target_id
                );
            }
        }
        self.broker
            .call("tabs.remove", json!({ "tabId": tab_id }))
            .await?;
        registry::remove_closed_tab(&self.ctx, tab_id);
        Ok(())
    }

    async fn navigate(&self, url: &str) -> Result<String> {
        let Some(tab_id) = registry::active_tab_id(&self.ctx) else {
            let tab = self.new_page(Some(url)).await?;
            return Ok(format!(
                "Created controlled Chrome tab and navigated to: {} - \"{}\"",
                tab.url, tab.title
            ));
        };
        let result = self
            .broker
            .call(
                "tabs.update",
                json!({
                    "tabId": tab_id,
                    "update": { "url": url }
                }),
            )
            .await?;
        let tab = tab_from_value(&result).unwrap_or(TabInfo {
            target_id: tab_id.to_string(),
            url: url.to_string(),
            title: String::new(),
            is_active: true,
        });
        registry::activate_controlled_tab(&self.ctx, tab_id)?;
        Ok(format!("Navigated to: {} - \"{}\"", tab.url, tab.title))
    }

    async fn go_back(&self) -> Result<String> {
        self.evaluate("history.back()").await?;
        Ok("Navigated back.".to_string())
    }

    async fn go_forward(&self) -> Result<String> {
        self.evaluate("history.forward()").await?;
        Ok("Navigated forward.".to_string())
    }

    async fn reload(&self) -> Result<String> {
        let tab_id = self.active_tab_id().await?;
        self.cdp_command(tab_id, "Page.reload", json!({})).await?;
        registry::clear_refs(&self.ctx);
        Ok("Reloaded active tab.".to_string())
    }

    async fn take_snapshot(&self, _format: SnapshotFormat) -> Result<Snapshot> {
        let tab_id = self.active_tab_id().await?;
        let raw = self.evaluate_on_tab(tab_id, SNAPSHOT_JS).await?;
        let json_str = raw
            .as_str()
            .ok_or_else(|| anyhow!("Extension snapshot returned non-string data"))?;
        let data: Value = serde_json::from_str(json_str)?;
        let url = data
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let title = data
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("untitled")
            .to_string();
        let viewport_w = data
            .get("viewport")
            .and_then(|v| v.get("w"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let viewport_h = data
            .get("viewport")
            .and_then(|v| v.get("h"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let mut truncated = data
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let mut elements = Vec::new();
        let mut refs = Vec::new();
        if let Some(arr) = data.get("elements").and_then(Value::as_array) {
            for el in arr {
                let ref_id = el.get("ref").and_then(Value::as_u64).unwrap_or(0) as u32;
                let depth = el.get("depth").and_then(Value::as_u64).unwrap_or(0) as u32;
                let role = el.get("role").and_then(Value::as_str).unwrap_or("unknown");
                let text = el.get("text").and_then(Value::as_str).unwrap_or("");
                let selector = el.get("selector").and_then(Value::as_str).unwrap_or("");
                let mut attrs = HashMap::new();
                if let Some(obj) = el.get("attrs").and_then(Value::as_object) {
                    for (key, value) in obj {
                        attrs.insert(
                            key.clone(),
                            value
                                .as_str()
                                .map(ToString::to_string)
                                .unwrap_or_else(|| value.to_string()),
                        );
                    }
                }
                let mut role = role.to_string();
                let mut text = text.to_string();
                if elements.len() < AX_ENRICH_LIMIT {
                    if let Ok(Some(ax)) = self.accessibility_for_selector(tab_id, selector).await {
                        attrs.insert("source".to_string(), "ax+dom".to_string());
                        if !ax.role.is_empty() {
                            if ax.role != role {
                                attrs.insert("dom_role".to_string(), role.clone());
                            }
                            role = ax.role;
                        }
                        if let Some(name) = ax.name.filter(|name| !name.trim().is_empty()) {
                            if name != text {
                                attrs.insert("dom_text".to_string(), text.clone());
                            }
                            text = name;
                        }
                        if let Some(value) = ax.value.filter(|value| !value.trim().is_empty()) {
                            attrs.insert("ax_value".to_string(), value);
                        }
                    } else {
                        attrs
                            .entry("source".to_string())
                            .or_insert_with(|| "dom".to_string());
                    }
                } else {
                    attrs
                        .entry("source".to_string())
                        .or_insert_with(|| "dom".to_string());
                }
                elements.push(crate::browser::backend::ElementRef {
                    ref_id,
                    role: role.clone(),
                    text: text.clone(),
                    locator: selector.to_string(),
                    depth,
                    attrs: attrs.clone(),
                });
                refs.push(ElementLocator {
                    ref_id,
                    role,
                    text,
                    selector: selector.to_string(),
                });
            }
        }
        if let Ok(frame_snapshots) = self.frame_snapshots(tab_id).await {
            let mut next_ref_id = elements
                .iter()
                .map(|element| element.ref_id)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            for frame in frame_snapshots {
                if frame.frame_id == 0 || frame.top_accessible {
                    continue;
                }
                truncated = truncated || frame.truncated;
                for frame_element in frame.elements {
                    let FrameSnapshotElement {
                        depth,
                        role,
                        text,
                        selector,
                        attrs,
                    } = frame_element;
                    if selector.trim().is_empty() {
                        continue;
                    }
                    let role = if role.trim().is_empty() {
                        "unknown".to_string()
                    } else {
                        role
                    };
                    let locator = frame_locator(frame.frame_id, &selector);
                    let mut attrs = frame_element_attrs(attrs);
                    attrs.insert("source".to_string(), "dom_frame".to_string());
                    attrs.insert("frame".to_string(), "oopif".to_string());
                    attrs.insert("frame_id".to_string(), frame.frame_id.to_string());
                    if let Some(document_id) = frame.document_id.as_deref() {
                        attrs.insert("document_id".to_string(), document_id.to_string());
                    }
                    if !frame.url.is_empty() {
                        attrs.insert("frame_url".to_string(), frame.url.clone());
                    }
                    if !frame.title.is_empty() {
                        attrs.insert("frame_title".to_string(), frame.title.clone());
                    }
                    if frame.viewport.w > 0 && frame.viewport.h > 0 {
                        attrs.insert(
                            "frame_viewport".to_string(),
                            format!("{}x{}", frame.viewport.w, frame.viewport.h),
                        );
                    }
                    elements.push(crate::browser::backend::ElementRef {
                        ref_id: next_ref_id,
                        role: role.clone(),
                        text: text.clone(),
                        locator: locator.clone(),
                        depth: depth.saturating_add(1),
                        attrs,
                    });
                    refs.push(ElementLocator {
                        ref_id: next_ref_id,
                        role,
                        text,
                        selector: locator,
                    });
                    next_ref_id = next_ref_id.saturating_add(1);
                }
            }
        }
        let mut seen_ax = HashSet::new();
        for element in &elements {
            if let Some(signature) = ax_node_signature(&element.role, &element.text) {
                seen_ax.insert(signature);
            }
        }
        if let Ok(ax_tree) = self.full_accessibility_tree(tab_id).await {
            let mut next_ref_id = elements
                .iter()
                .map(|element| element.ref_id)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            for ax_element in
                parse_ax_only_elements(&ax_tree, &mut seen_ax, next_ref_id, AX_ONLY_LIMIT)
            {
                next_ref_id = next_ref_id.max(ax_element.element.ref_id.saturating_add(1));
                if let Some(locator) = ax_element.locator {
                    refs.push(locator);
                }
                elements.push(ax_element.element);
            }
            if let Ok(flat_sessions) = self.flat_sessions(tab_id).await {
                let mut remaining = FLAT_SESSION_AX_ONLY_LIMIT;
                for session in flat_sessions
                    .iter()
                    .filter(|session| is_flat_session_iframe_candidate(session))
                    .take(FLAT_SESSION_AX_SESSION_LIMIT)
                {
                    if remaining == 0 {
                        break;
                    }
                    let ax_tree = match self
                        .full_accessibility_tree_for_session(tab_id, &session.session_id)
                        .await
                    {
                        Ok(ax_tree) => ax_tree,
                        Err(e) => {
                            app_debug!(
                                "browser",
                                "extension_backend",
                                "flat-session AX snapshot failed for session {}: {}",
                                session.session_id,
                                e
                            );
                            continue;
                        }
                    };
                    let ax_elements = parse_flat_session_ax_only_elements(
                        &ax_tree,
                        &mut seen_ax,
                        next_ref_id,
                        remaining,
                        session,
                    );
                    for ax_element in ax_elements {
                        next_ref_id = next_ref_id.max(ax_element.element.ref_id.saturating_add(1));
                        if let Some(locator) = ax_element.locator {
                            refs.push(locator);
                        }
                        elements.push(ax_element.element);
                        remaining = remaining.saturating_sub(1);
                    }
                }
            }
        }
        registry::update_snapshot_refs(&self.ctx, tab_id, refs, url.clone())?;
        Ok(Snapshot {
            url,
            title,
            viewport: (viewport_w, viewport_h),
            elements,
            truncated,
        })
    }

    async fn take_screenshot(&self, params: ScreenshotParams) -> Result<Vec<u8>> {
        let tab_id = self.active_tab_id().await?;
        let format = match params.format {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpeg",
        };
        let mut cdp_params = json!({
            "format": format,
            "captureBeyondViewport": params.full_page,
            "quality": params.quality,
        });
        if let Some(ref_id) = params.ref_id {
            let element = self.selector_for_ref(ref_id).await?;
            if let Some((frame_id, frame_selector)) = frame_locator_parts(&element.selector) {
                return self
                    .capture_frame_element_screenshot(
                        tab_id,
                        frame_id,
                        frame_selector,
                        params,
                        format,
                    )
                    .await;
            }
            cdp_params["clip"] = self.element_clip_on_tab(tab_id, &element.selector).await?;
            cdp_params["captureBeyondViewport"] = json!(true);
        }
        let result = self
            .cdp_command(tab_id, "Page.captureScreenshot", cdp_params)
            .await?;
        self.cdp_binary_result_bytes(
            &result,
            "Page.captureScreenshot",
            "screenshot",
            &[params.format.mime()],
        )
        .await
    }

    async fn save_pdf(&self, params: PdfParams) -> Result<Vec<u8>> {
        let tab_id = self.active_tab_id().await?;
        let mut cdp_params = json!({});
        if let Some(landscape) = params.landscape {
            cdp_params["landscape"] = json!(landscape);
        }
        if let Some(print_background) = params.print_background {
            cdp_params["printBackground"] = json!(print_background);
        }
        if let Some(css_page) = params.prefer_css_page_size {
            cdp_params["preferCSSPageSize"] = json!(css_page);
        }
        if let Some(paper) = params.paper_format.as_deref() {
            let (w, h) = match paper.to_ascii_lowercase().as_str() {
                "a3" => (11.69, 16.54),
                "a4" => (8.27, 11.69),
                "a5" => (5.83, 8.27),
                "letter" => (8.5, 11.0),
                "legal" => (8.5, 14.0),
                "tabloid" => (11.0, 17.0),
                other => bail!(
                    "Unknown paper_format: '{}'. Options: a3, a4, a5, letter, legal, tabloid",
                    other
                ),
            };
            cdp_params["paperWidth"] = json!(w);
            cdp_params["paperHeight"] = json!(h);
        }
        let result = self
            .cdp_command(tab_id, "Page.printToPDF", cdp_params)
            .await?;
        self.cdp_binary_result_bytes(&result, "Page.printToPDF", "pdf", &["application/pdf"])
            .await
    }

    async fn act(&self, kind: ActKind, params: ActParams) -> Result<String> {
        let ref_id = params
            .ref_id
            .ok_or_else(|| anyhow!("ExtensionBackend act requires 'ref'"))?;
        let original = self.selector_for_ref(ref_id).await?;
        let original_target = if kind == ActKind::Drag {
            let target_ref = params
                .target_ref
                .ok_or_else(|| anyhow!("act.drag requires 'target_ref' parameter"))?;
            Some((target_ref, self.selector_for_ref(target_ref).await?))
        } else {
            None
        };
        match self.act_once(kind, &params, None).await {
            Ok(result) => Ok(result),
            Err(err) if is_stale_ref_error(&err) => {
                let _ = self.take_snapshot(SnapshotFormat::Role).await?;
                if kind == ActKind::Drag {
                    let (target_ref, original_target) = original_target
                        .ok_or_else(|| anyhow!("act.drag requires 'target_ref' parameter"))?;
                    let source =
                        registry::find_ref_by_role_text(&self.ctx, &original.role, &original.text)
                            .unwrap_or(original);
                    let target = registry::find_ref_by_role_text(
                        &self.ctx,
                        &original_target.role,
                        &original_target.text,
                    )
                    .unwrap_or(original_target);
                    let tab_id = self.active_tab_id().await?;
                    return self
                        .act_drag_with_target(tab_id, ref_id, &source, target_ref, &target)
                        .await
                        .map(|msg| format!("{msg} (ref auto-recovered)"));
                }
                self.act_once(kind, &params, Some(&original))
                    .await
                    .map(|msg| format!("{msg} (ref auto-recovered)"))
            }
            Err(err) => Err(err),
        }
    }

    async fn evaluate(&self, script: &str) -> Result<Value> {
        let tab_id = self.active_tab_id().await?;
        self.evaluate_on_tab(tab_id, script).await
    }

    async fn raw_cdp(&self, params: RawCdpParams) -> Result<Value> {
        let tab_id = self.active_tab_id().await?;
        self.raw_cdp_command(tab_id, &params.method, params.params)
            .await
    }

    async fn cancel_download(&self, download_id: i64) -> Result<String> {
        self.broker
            .call("downloads.cancel", json!({ "downloadId": download_id }))
            .await?;
        Ok(format!("Cancelled Chrome download {download_id}"))
    }

    async fn wait_for(&self, params: WaitParams) -> Result<String> {
        let needle = params
            .text
            .clone()
            .ok_or_else(|| anyhow!("wait_for requires 'text' parameter"))?;
        let escaped = serde_json::to_string(&needle)?;
        let script = format!("document.body && document.body.innerText.includes({escaped})");
        let start = std::time::Instant::now();
        let poll = std::time::Duration::from_millis(500);
        loop {
            let found = self.evaluate(&script).await?.as_bool().unwrap_or(false);
            if found {
                return Ok(format!("Text \"{}\" found on page.", needle));
            }
            if start.elapsed().as_millis() as u64 >= params.timeout_ms {
                bail!(
                    "Timeout after {}ms waiting for text \"{}\"",
                    params.timeout_ms,
                    needle
                );
            }
            tokio::time::sleep(poll).await;
        }
    }

    async fn handle_dialog(&self, action: DialogAction, prompt: Option<&str>) -> Result<String> {
        let tab_id = self.active_tab_id().await?;
        let accept = matches!(action, DialogAction::Accept);
        let mut params = json!({ "accept": accept });
        if let Some(prompt) = prompt {
            params["promptText"] = json!(prompt);
        }
        self.cdp_command(tab_id, "Page.handleJavaScriptDialog", params)
            .await
            .map_err(|e| anyhow!("Handle dialog failed: {e}. Is there an open dialog?"))?;
        Ok(format!(
            "Dialog {}.{}",
            if accept { "accepted" } else { "dismissed" },
            prompt
                .map(|t| format!(" Prompt text: \"{}\"", t))
                .unwrap_or_default()
        ))
    }

    async fn resize(&self, width: u32, height: u32) -> Result<String> {
        let tab_id = self.active_tab_id().await?;
        self.cdp_command(
            tab_id,
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": width,
                "height": height,
                "deviceScaleFactor": 1,
                "mobile": false
            }),
        )
        .await?;
        Ok(format!("Viewport resized to {}x{}", width, height))
    }

    async fn scroll(&self, params: ScrollParams) -> Result<String> {
        let (dx, dy) = match params.direction {
            ScrollDirection::Up => (0, -params.amount),
            ScrollDirection::Down => (0, params.amount),
            ScrollDirection::Left => (-params.amount, 0),
            ScrollDirection::Right => (params.amount, 0),
        };
        self.evaluate(&format!("window.scrollBy({}, {})", dx, dy))
            .await?;
        Ok(format!(
            "Scrolled {:?} by {}px",
            params.direction, params.amount
        ))
    }

    async fn observe(&self, kind: ObserveKind, since: Option<i64>) -> Result<Vec<ObserveEntry>> {
        let mut tab_id_filter = None;
        let kind = match kind {
            ObserveKind::Console => {
                let tab_id = self.active_tab_id().await?;
                let _ = self.cdp_command(tab_id, "Runtime.enable", json!({})).await;
                tab_id_filter = Some(tab_id);
                "console"
            }
            ObserveKind::Network => {
                let tab_id = self.active_tab_id().await?;
                let _ = self.cdp_command(tab_id, "Network.enable", json!({})).await;
                tab_id_filter = Some(tab_id);
                "network"
            }
            ObserveKind::PageErrors => {
                let tab_id = self.active_tab_id().await?;
                let _ = self.cdp_command(tab_id, "Runtime.enable", json!({})).await;
                tab_id_filter = Some(tab_id);
                "pageErrors"
            }
            ObserveKind::Downloads => "downloads",
        };
        let value = self
            .broker
            .call(
                "observe.read",
                json!({
                    "kind": kind,
                    "since": since,
                    "tabId": tab_id_filter
                }),
            )
            .await?;
        let entries: Vec<ObserveEntry> = serde_json::from_value(value)
            .map_err(|e| anyhow!("Extension observe.read returned invalid entries: {e}"))?;
        Ok(entries)
    }

    async fn release_page(&self, target_id: &str) -> Result<String> {
        let tab_id = parse_tab_id(target_id)?;
        let owner_kind = registry::release_tab(&self.ctx, tab_id)?;
        self.hide_overlay(tab_id).await;
        if owner_kind == TabOwnerKind::User {
            self.detach_debugger(tab_id).await?;
        }
        emit_control_stopped_for_scope(
            tab_id,
            scope_for_context(&self.ctx),
            BrowserControlStoppedReason::ManualRelease,
            false,
        );
        Ok(format!(
            "Released Chrome tab {}. The tab remains open in Chrome.",
            target_id
        ))
    }

    async fn finalize_pages(&self, keep: &[String]) -> Result<String> {
        let keep_ids: HashSet<i64> = keep
            .iter()
            .map(|id| parse_tab_id(id))
            .collect::<Result<HashSet<_>>>()?;
        let actions = registry::finalize_scope(&self.ctx, &keep_ids);
        emit_finalize_events(&self.ctx, &actions, BrowserControlStoppedReason::Finalize);
        Ok(apply_finalize_actions(&self.broker, actions).await)
    }
}

pub async fn cleanup_extension_session(session_id: &str) -> String {
    cleanup_extension_session_with_reason(session_id, BrowserControlStoppedReason::SessionCleanup)
        .await
}

pub async fn stop_all_extension_control() -> BrowserExtensionStopResult {
    let scoped_actions = registry::finalize_all_scopes();
    let stopped_tabs = scoped_actions.len();
    for scoped in &scoped_actions {
        emit_control_stopped_for_scope(
            scoped.action.tab_id,
            scoped.scope.clone(),
            BrowserControlStoppedReason::UserStop,
            scoped.action.close,
        );
    }

    let Some(broker) = BrowserExtensionBroker::global() else {
        let message = if scoped_actions.is_empty() {
            "No controlled Chrome tabs to stop.".to_string()
        } else {
            format!(
                "Released local browser control registry for {} tab(s); extension broker is unavailable.",
                stopped_tabs
            )
        };
        return BrowserExtensionStopResult {
            stopped_tabs,
            message,
        };
    };

    let actions = scoped_actions
        .into_iter()
        .map(|scoped| scoped.action)
        .collect();
    BrowserExtensionStopResult {
        stopped_tabs,
        message: apply_finalize_actions(&broker, actions).await,
    }
}

async fn cleanup_extension_session_with_reason(
    session_id: &str,
    reason: BrowserControlStoppedReason,
) -> String {
    let ctx = BrowserBackendContext {
        session_id: Some(session_id.to_string()),
        source: Some("session.cleanup".to_string()),
        ..BrowserBackendContext::default()
    };
    let actions = registry::finalize_scope(&ctx, &HashSet::new());
    emit_finalize_events(&ctx, &actions, reason);
    let Some(broker) = BrowserExtensionBroker::global() else {
        if actions.is_empty() {
            return "No controlled Chrome tabs to cleanup.".to_string();
        }
        return format!(
            "Released local browser control registry for {} tab(s); extension broker is unavailable.",
            actions.len()
        );
    };
    apply_finalize_actions(&broker, actions).await
}

pub fn schedule_extension_turn_finalize(session_id: &str) {
    if session_id.is_empty() {
        return;
    }
    let session_id = session_id.to_string();
    tokio::spawn(async move {
        let result = cleanup_extension_session_with_reason(
            &session_id,
            BrowserControlStoppedReason::TurnFinalize,
        )
        .await;
        app_debug!(
            "browser",
            "extension_backend",
            "turn-end browser finalize for {}: {}",
            session_id,
            result
        );
    });
}

fn emit_finalize_events(
    ctx: &BrowserBackendContext,
    actions: &[FinalizeTabAction],
    reason: BrowserControlStoppedReason,
) {
    let scope = scope_for_context(ctx);
    for action in actions {
        emit_control_stopped_for_scope(action.tab_id, scope.clone(), reason, action.close);
    }
}

async fn detach_debugger_with_broker(broker: &BrowserExtensionBroker, tab_id: i64) -> Result<()> {
    match broker
        .call("debugger.detach", json!({ "tabId": tab_id }))
        .await
    {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().to_ascii_lowercase().contains("not attached") => Ok(()),
        Err(e) => Err(e),
    }
}

async fn apply_finalize_actions(
    broker: &BrowserExtensionBroker,
    actions: Vec<FinalizeTabAction>,
) -> String {
    if actions.is_empty() {
        return "No controlled Chrome tabs to finalize.".to_string();
    }

    // Run the per-tab teardown concurrently: each tab's hide-overlay +
    // remove/detach is an independent broker round-trip, so serializing them
    // made finalize latency scale with tab count (up to CALL_TIMEOUT each).
    enum Outcome {
        Closed(i64),
        Released {
            id: i64,
            detach_error: Option<String>,
        },
        CloseFailed(String),
    }
    let outcomes = futures_util::future::join_all(actions.into_iter().map(|action| async move {
        let _ = hide_overlay_with_broker(broker, action.tab_id).await;
        if action.close {
            match broker
                .call("tabs.remove", json!({ "tabId": action.tab_id }))
                .await
            {
                Ok(_) => Outcome::Closed(action.tab_id),
                Err(e) => Outcome::CloseFailed(format!("close {}: {}", action.tab_id, e)),
            }
        } else {
            let detach_error = if action.owner_kind == TabOwnerKind::User {
                detach_debugger_with_broker(broker, action.tab_id)
                    .await
                    .err()
                    .map(|e| format!("detach {}: {}", action.tab_id, e))
            } else {
                None
            };
            Outcome::Released {
                id: action.tab_id,
                detach_error,
            }
        }
    }))
    .await;

    let mut closed = Vec::new();
    let mut released = Vec::new();
    let mut failed = Vec::new();
    for outcome in outcomes {
        match outcome {
            Outcome::Closed(id) => closed.push(id),
            Outcome::Released { id, detach_error } => {
                released.push(id);
                if let Some(e) = detach_error {
                    failed.push(e);
                }
            }
            Outcome::CloseFailed(msg) => failed.push(msg),
        }
    }

    let mut parts = Vec::new();
    if !closed.is_empty() {
        parts.push(format!("closed agent tabs: {:?}", closed));
    }
    if !released.is_empty() {
        parts.push(format!("released tabs: {:?}", released));
    }
    if !failed.is_empty() {
        parts.push(format!("failures: {}", failed.join("; ")));
    }
    format!("Finalized Chrome tabs: {}.", parts.join(", "))
}

async fn hide_overlay_with_broker(broker: &BrowserExtensionBroker, tab_id: i64) -> Result<()> {
    broker
        .call("overlay.hide", json!({ "tabId": tab_id }))
        .await
        .map(|_| ())
}

fn control_overlay_enabled() -> bool {
    crate::config::cached_config()
        .browser
        .as_ref()
        .and_then(|browser| browser.extension.as_ref())
        .and_then(|extension| extension.show_control_overlay)
        .unwrap_or(true)
}

fn validate_cdp_method(method: &str) -> Result<()> {
    if BLOCKED_CDP_DOMAIN_PREFIXES
        .iter()
        .any(|prefix| method.starts_with(prefix))
    {
        bail!("CDP method '{method}' is blocked by Hope Agent browser policy");
    }
    if ALLOWED_CDP_METHODS.contains(&method) {
        return Ok(());
    }
    bail!("CDP method '{method}' is not allowed by Hope Agent browser policy");
}

fn validate_raw_cdp_method(method: &str) -> Result<()> {
    validate_cdp_method_name(method)?;
    // raw_cdp bypasses the ALLOWED_CDP_METHODS whitelist on purpose, but never
    // the safety blocklist: it must not read/forge the user's real Chrome
    // credentials, wipe storage, intercept network traffic, or spawn targets.
    if BLOCKED_CDP_DOMAIN_PREFIXES
        .iter()
        .any(|prefix| method.starts_with(prefix))
    {
        bail!("CDP method '{method}' is blocked by Hope Agent browser policy and cannot be used via raw_cdp");
    }
    if BLOCKED_RAW_CDP_METHODS.contains(&method) {
        bail!(
            "CDP method '{method}' is blocked by Hope Agent browser policy \
             (cookie/credential access is not permitted via raw_cdp)"
        );
    }
    Ok(())
}

fn validate_cdp_method_name(method: &str) -> Result<()> {
    let Some((domain, name)) = method.split_once('.') else {
        bail!("CDP method must be in Domain.method form");
    };
    if domain.is_empty() || name.is_empty() {
        bail!("CDP method must be in Domain.method form");
    }
    if !method
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
    {
        bail!("CDP method contains unsupported characters");
    }
    Ok(())
}

fn parse_tab_id(target_id: &str) -> Result<i64> {
    target_id
        .parse::<i64>()
        .map_err(|_| anyhow!("Invalid extension tab id: {target_id}"))
}

fn tab_from_value(tab: &Value) -> Option<TabInfo> {
    let id = tab.get("id").and_then(Value::as_i64)?;
    Some(TabInfo {
        target_id: id.to_string(),
        url: tab
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("about:blank")
            .to_string(),
        title: tab
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        is_active: tab.get("active").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn build_action_script(kind: ActKind, params: &ActParams, selector: &str) -> Result<String> {
    Ok(match kind {
        ActKind::Click => format!(
            r#"(() => {{
              {selector_helper}
              const resolved = __hopeResolveSelector({selector});
              if (!resolved) throw new Error("Element not found for selector");
              const el = resolved.el;
              el.scrollIntoView({{block: "center", inline: "center"}});
              el.click();
              return "Clicked";
            }})()"#,
            selector_helper = SELECTOR_HELPER_JS
        ),
        ActKind::DoubleClick => format!(
            r#"(() => {{
              {selector_helper}
              const resolved = __hopeResolveSelector({selector});
              if (!resolved) throw new Error("Element not found for selector");
              const el = resolved.el;
              el.scrollIntoView({{block: "center", inline: "center"}});
              el.dispatchEvent(new MouseEvent("dblclick", {{bubbles: true, cancelable: true, view: window}}));
              return "Double clicked";
            }})()"#,
            selector_helper = SELECTOR_HELPER_JS
        ),
        ActKind::Hover => format!(
            r#"(() => {{
              {selector_helper}
              const resolved = __hopeResolveSelector({selector});
              if (!resolved) throw new Error("Element not found for selector");
              const el = resolved.el;
              el.scrollIntoView({{block: "center", inline: "center"}});
              el.dispatchEvent(new MouseEvent("mouseover", {{bubbles: true, cancelable: true, view: window}}));
              el.dispatchEvent(new MouseEvent("mouseenter", {{bubbles: true, cancelable: true, view: window}}));
              return "Hovered";
            }})()"#,
            selector_helper = SELECTOR_HELPER_JS
        ),
        ActKind::Fill => {
            let text = serde_json::to_string(&params.text.clone().unwrap_or_default())?;
            format!(
                r#"(() => {{
                  {selector_helper}
                  const resolved = __hopeResolveSelector({selector});
                  if (!resolved) throw new Error("Element not found for selector");
                  const el = resolved.el;
                  el.scrollIntoView({{block: "center", inline: "center"}});
                  el.focus();
                  const value = {text};
                  if ("value" in el) {{
                    const proto = Object.getPrototypeOf(el);
                    const desc = Object.getOwnPropertyDescriptor(proto, "value");
                    if (desc && desc.set) desc.set.call(el, value);
                    else el.value = value;
                  }} else {{
                    el.textContent = value;
                  }}
                  el.dispatchEvent(new InputEvent("input", {{bubbles: true, inputType: "insertText", data: value}}));
                  el.dispatchEvent(new Event("change", {{bubbles: true}}));
                  return "Filled";
                }})()"#,
                selector_helper = SELECTOR_HELPER_JS
            )
        }
        ActKind::Select => {
            let value = serde_json::to_string(
                &params
                    .values
                    .as_ref()
                    .and_then(|v| v.first())
                    .ok_or_else(|| anyhow!("act.select requires 'values'"))?,
            )?;
            format!(
                r#"(() => {{
                  {selector_helper}
                  const resolved = __hopeResolveSelector({selector});
                  if (!resolved) throw new Error("Element not found for selector");
                  const el = resolved.el;
                  el.value = {value};
                  el.dispatchEvent(new Event("input", {{bubbles: true}}));
                  el.dispatchEvent(new Event("change", {{bubbles: true}}));
                  return "Selected";
                }})()"#,
                selector_helper = SELECTOR_HELPER_JS
            )
        }
        ActKind::Press => {
            let key = serde_json::to_string(
                &params
                    .key
                    .as_ref()
                    .ok_or_else(|| anyhow!("act.press requires 'key'"))?,
            )?;
            format!(
                r#"(() => {{
                  {selector_helper}
                  const resolved = __hopeResolveSelector({selector});
                  if (!resolved) throw new Error("Element not found for selector");
                  const el = resolved.el;
                  el.focus();
                  const key = {key};
                  for (const type of ["keydown", "keypress", "keyup"]) {{
                    el.dispatchEvent(new KeyboardEvent(type, {{key, bubbles: true, cancelable: true}}));
                  }}
                  return "Pressed " + key;
                }})()"#,
                selector_helper = SELECTOR_HELPER_JS
            )
        }
        ActKind::Drag => bail!("ExtensionBackend drag must be handled before JS action build"),
        ActKind::Upload => bail!("ExtensionBackend upload must be handled before JS action build"),
    })
}

fn build_object_action_call(kind: ActKind, params: &ActParams) -> Result<(String, Vec<Value>)> {
    Ok(match kind {
        ActKind::Click => (
            r#"function() {
              const el = this;
              el.scrollIntoView({block: "center", inline: "center"});
              el.click();
              return "Clicked";
            }"#
            .to_string(),
            Vec::new(),
        ),
        ActKind::DoubleClick => (
            r#"function() {
              const el = this;
              el.scrollIntoView({block: "center", inline: "center"});
              el.dispatchEvent(new MouseEvent("dblclick", {bubbles: true, cancelable: true, view: window}));
              return "Double clicked";
            }"#
            .to_string(),
            Vec::new(),
        ),
        ActKind::Hover => (
            r#"function() {
              const el = this;
              el.scrollIntoView({block: "center", inline: "center"});
              el.dispatchEvent(new MouseEvent("mouseover", {bubbles: true, cancelable: true, view: window}));
              el.dispatchEvent(new MouseEvent("mouseenter", {bubbles: true, cancelable: true, view: window}));
              return "Hovered";
            }"#
            .to_string(),
            Vec::new(),
        ),
        ActKind::Fill => {
            let text = params.text.clone().unwrap_or_default();
            (
                r#"function(value) {
                  const el = this;
                  el.scrollIntoView({block: "center", inline: "center"});
                  el.focus();
                  if ("value" in el) {
                    const proto = Object.getPrototypeOf(el);
                    const desc = Object.getOwnPropertyDescriptor(proto, "value");
                    if (desc && desc.set) desc.set.call(el, value);
                    else el.value = value;
                  } else {
                    el.textContent = value;
                  }
                  el.dispatchEvent(new InputEvent("input", {bubbles: true, inputType: "insertText", data: value}));
                  el.dispatchEvent(new Event("change", {bubbles: true}));
                  return "Filled";
                }"#
                .to_string(),
                vec![json!({ "value": text })],
            )
        }
        ActKind::Select => {
            let value = params
                .values
                .as_ref()
                .and_then(|v| v.first())
                .ok_or_else(|| anyhow!("act.select requires 'values'"))?
                .clone();
            (
                r#"function(value) {
                  const el = this;
                  el.scrollIntoView({block: "center", inline: "center"});
                  el.value = value;
                  el.dispatchEvent(new Event("input", {bubbles: true}));
                  el.dispatchEvent(new Event("change", {bubbles: true}));
                  return "Selected";
                }"#
                .to_string(),
                vec![json!({ "value": value })],
            )
        }
        ActKind::Press => {
            let key = params
                .key
                .as_ref()
                .ok_or_else(|| anyhow!("act.press requires 'key'"))?
                .clone();
            (
                r#"function(key) {
                  const el = this;
                  el.focus();
                  for (const type of ["keydown", "keypress", "keyup"]) {
                    el.dispatchEvent(new KeyboardEvent(type, {key, bubbles: true, cancelable: true}));
                  }
                  return "Pressed " + key;
                }"#
                .to_string(),
                vec![json!({ "value": key })],
            )
        }
        ActKind::Drag => bail!("ExtensionBackend drag is handled with pointer events"),
        ActKind::Upload => bail!("ExtensionBackend upload must be handled before JS action build"),
    })
}

fn runtime_result_value(result: &Value) -> Result<Value> {
    if let Some(details) = result.get("exceptionDetails") {
        let text = details
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("JavaScript exception");
        let description = details
            .get("exception")
            .and_then(|e| e.get("description"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if description.is_empty() {
            bail!("{text}");
        }
        bail!("{text}: {description}");
    }
    Ok(result
        .get("result")
        .and_then(|r| r.get("value"))
        .cloned()
        .unwrap_or(Value::Null))
}

fn ax_backend_dom_node_locator(backend_dom_node_id: u64) -> String {
    format!("{AX_BACKEND_DOM_NODE_LOCATOR_PREFIX}{backend_dom_node_id}")
}

fn ax_backend_dom_node_id_from_locator(locator: &str) -> Option<u64> {
    locator
        .strip_prefix(AX_BACKEND_DOM_NODE_LOCATOR_PREFIX)?
        .parse()
        .ok()
}

fn frame_ax_backend_dom_node_locator(session_id: &str, backend_dom_node_id: u64) -> String {
    format!("{FRAME_AX_BACKEND_DOM_NODE_LOCATOR_PREFIX}{session_id}:{backend_dom_node_id}")
}

fn frame_ax_backend_dom_node_id_from_locator(locator: &str) -> Option<(String, u64)> {
    let rest = locator.strip_prefix(FRAME_AX_BACKEND_DOM_NODE_LOCATOR_PREFIX)?;
    let (session_id, backend_dom_node_id) = rest.rsplit_once(':')?;
    if session_id.is_empty() {
        return None;
    }
    Some((session_id.to_string(), backend_dom_node_id.parse().ok()?))
}

fn frame_locator(frame_id: i64, selector: &str) -> String {
    format!("{FRAME_LOCATOR_PREFIX}{frame_id} >>> {selector}")
}

fn frame_locator_parts(locator: &str) -> Option<(i64, &str)> {
    let rest = locator.strip_prefix(FRAME_LOCATOR_PREFIX)?;
    let (frame_id, selector) = rest.split_once(" >>> ")?;
    let frame_id = frame_id.parse().ok()?;
    if selector.trim().is_empty() {
        return None;
    }
    Some((frame_id, selector))
}

fn same_frame_drag_target_selector(source_frame_id: i64, target: &ElementLocator) -> Result<&str> {
    let Some((target_frame_id, target_selector)) = frame_locator_parts(&target.selector) else {
        bail!("act.drag from a cross-origin iframe ref to a root-frame ref is not supported yet");
    };
    if target_frame_id != source_frame_id {
        bail!("act.drag between different cross-origin iframe refs is not supported yet");
    }
    Ok(target_selector)
}

fn frame_element_attrs(attrs: HashMap<String, Value>) -> HashMap<String, String> {
    attrs
        .into_iter()
        .map(|(key, value)| {
            let value = value
                .as_str()
                .map(ToString::to_string)
                .unwrap_or_else(|| value.to_string());
            (key, value)
        })
        .collect()
}

fn act_kind_wire_name(kind: ActKind) -> &'static str {
    match kind {
        ActKind::Click => "click",
        ActKind::DoubleClick => "double_click",
        ActKind::Hover => "hover",
        ActKind::Fill => "fill",
        ActKind::Select => "select",
        ActKind::Press => "press",
        ActKind::Drag => "drag",
        ActKind::Upload => "upload",
    }
}

fn frame_action_params(params: &ActParams) -> Value {
    json!({
        "text": params.text.clone(),
        "key": params.key.clone(),
        "values": params.values.clone(),
    })
}

fn is_stale_ref_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("element not found")
        || msg.contains("not found for selector")
        || msg.contains("no frame")
        || msg.contains("frame action")
        || msg.contains("cannot find context with specified id")
        || msg.contains("execution context was destroyed")
}

fn point_coord(point: &Value, key: &str) -> Result<f64> {
    point
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("Drag point script returned invalid {key} coordinate"))
}

fn validate_clip(clip: &Value) -> Result<()> {
    let width = point_coord(clip, "width")?;
    let height = point_coord(clip, "height")?;
    let scale = point_coord(clip, "scale")?;
    if width <= 0.0 || height <= 0.0 || scale <= 0.0 {
        bail!("Screenshot clip must have positive width, height, and scale");
    }
    let _ = point_coord(clip, "x")?;
    let _ = point_coord(clip, "y")?;
    Ok(())
}

fn parse_frame_clip_result(result: &Value) -> Result<FrameClip> {
    let clip = result
        .get("clip")
        .cloned()
        .ok_or_else(|| anyhow!("Frame clip result did not include clip"))?;
    validate_clip(&clip)?;
    let url = result
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if url.trim().is_empty() {
        bail!("Frame clip result did not include frame URL");
    }
    let title = result
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(FrameClip { clip, url, title })
}

fn pointer_target_from_clip(session_id: Option<String>, clip: &Value) -> Result<PointerTarget> {
    validate_clip(clip)?;
    let x = point_coord(clip, "x")?;
    let y = point_coord(clip, "y")?;
    let width = point_coord(clip, "width")?;
    let height = point_coord(clip, "height")?;
    Ok(PointerTarget {
        session_id,
        x: x + width / 2.0,
        y: y + height / 2.0,
    })
}

fn drag_move_points(source: &PointerTarget, target: &PointerTarget) -> Vec<PointerTarget> {
    if source.session_id == target.session_id {
        return (1..=4)
            .map(|step| {
                let ratio = step as f64 / 4.0;
                PointerTarget {
                    session_id: source.session_id.clone(),
                    x: source.x + (target.x - source.x) * ratio,
                    y: source.y + (target.y - source.y) * ratio,
                }
            })
            .collect();
    }
    // Different CDP targets use different viewport coordinate spaces. Do not
    // interpolate between them; send a final move in the destination session.
    vec![source.clone(), target.clone()]
}

fn select_flat_session_for_frame_context<'a>(
    sessions: &'a [FlatSessionInfo],
    frame_id: i64,
    frame_url: &str,
    frame_title: &str,
) -> Result<&'a str> {
    let frame_matches = sessions
        .iter()
        .filter(|session| is_flat_session_iframe_candidate(session))
        .filter(|session| {
            session.matched_frame.as_ref().is_some_and(|matched| {
                matched.status == "matched" && matched.frame_id == Some(frame_id)
            })
        })
        .collect::<Vec<_>>();
    match frame_matches.as_slice() {
        [session] => return Ok(session.session_id.as_str()),
        [] => {}
        _ => bail!(
            "Could not uniquely map cross-origin iframe {frame_id} ({frame_url}) to a debugger flat session; multiple sessions report the same matched frame id"
        ),
    }
    let url_matches = sessions
        .iter()
        .filter(|session| is_flat_session_iframe_candidate(session))
        .filter(|session| session.target_info.url == frame_url)
        .collect::<Vec<_>>();
    let title = frame_title.trim();
    if !title.is_empty() {
        let title_matches = url_matches
            .iter()
            .copied()
            .filter(|session| session.target_info.title == title)
            .collect::<Vec<_>>();
        if title_matches.len() == 1 {
            return Ok(title_matches[0].session_id.as_str());
        }
        if title_matches.len() > 1 {
            bail!(
                "Could not uniquely map cross-origin iframe {frame_id} ({frame_url}) to a debugger flat session; multiple sessions share the same URL and title"
            );
        }
    }
    match url_matches.as_slice() {
        [session] => Ok(session.session_id.as_str()),
        [] => bail!(
            "Could not map cross-origin iframe {frame_id} ({frame_url}) to a debugger flat session"
        ),
        _ => bail!(
            "Could not uniquely map cross-origin iframe {frame_id} ({frame_url}) to a debugger flat session"
        ),
    }
}

fn parse_ax_element_info(tree: &Value) -> Option<AxElementInfo> {
    let nodes = tree.get("nodes").and_then(Value::as_array)?;
    nodes
        .iter()
        .filter(|node| {
            !node
                .get("ignored")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .find_map(|node| {
            let role = ax_value_to_string(node.get("role")?)?;
            if role.is_empty() {
                return None;
            }
            Some(AxElementInfo {
                role,
                name: node.get("name").and_then(ax_value_to_string),
                value: node.get("value").and_then(ax_value_to_string),
            })
        })
}

fn parse_ax_only_elements(
    tree: &Value,
    seen: &mut HashSet<String>,
    start_ref_id: u32,
    limit: usize,
) -> Vec<AxOnlySnapshotElement> {
    let Some(nodes) = tree.get("nodes").and_then(Value::as_array) else {
        return Vec::new();
    };
    let depths = ax_node_depths(nodes);
    let mut out = Vec::new();
    let mut next_ref = start_ref_id;
    for node in nodes {
        if out.len() >= limit {
            break;
        }
        if node
            .get("ignored")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let Some(role) = node.get("role").and_then(ax_value_to_string) else {
            continue;
        };
        let name = node.get("name").and_then(ax_value_to_string);
        let value = node.get("value").and_then(ax_value_to_string);
        let text = name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .or_else(|| value.as_deref().filter(|value| !value.trim().is_empty()))
            .unwrap_or("")
            .trim()
            .to_string();
        if !should_include_ax_only_node(&role, &text) {
            continue;
        }
        let Some(signature) = ax_node_signature(&role, &text) else {
            continue;
        };
        if !seen.insert(signature) {
            continue;
        }
        let backend_dom_node_id = node.get("backendDOMNodeId").and_then(Value::as_u64);
        let operable = backend_dom_node_id.is_some() && is_ax_operable_role(&role);
        let mut attrs = HashMap::new();
        attrs.insert("source".to_string(), "ax_only".to_string());
        if operable {
            attrs.insert("ax_operable".to_string(), "true".to_string());
        } else {
            attrs.insert("readonly".to_string(), "true".to_string());
        }
        if let Some(node_id) = node.get("nodeId").and_then(Value::as_str) {
            attrs.insert("ax_node_id".to_string(), node_id.to_string());
        }
        if let Some(backend_dom_node_id) = backend_dom_node_id {
            attrs.insert(
                "backend_dom_node_id".to_string(),
                backend_dom_node_id.to_string(),
            );
        }
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            attrs.insert("ax_value".to_string(), value);
        }
        let depth = node
            .get("nodeId")
            .and_then(Value::as_str)
            .and_then(|node_id| depths.get(node_id).copied())
            .unwrap_or(0);
        let locator = operable.then(|| ElementLocator {
            ref_id: next_ref,
            role: role.clone(),
            text: text.clone(),
            selector: ax_backend_dom_node_locator(backend_dom_node_id.unwrap_or_default()),
        });
        out.push(AxOnlySnapshotElement {
            element: ElementRef {
                ref_id: next_ref,
                role,
                text,
                locator: locator
                    .as_ref()
                    .map(|locator| locator.selector.clone())
                    .unwrap_or_default(),
                depth,
                attrs,
            },
            locator,
        });
        next_ref = next_ref.saturating_add(1);
    }
    out
}

fn parse_flat_session_ax_only_elements(
    tree: &Value,
    seen: &mut HashSet<String>,
    start_ref_id: u32,
    limit: usize,
    session: &FlatSessionInfo,
) -> Vec<AxOnlySnapshotElement> {
    parse_ax_only_elements(tree, seen, start_ref_id, limit)
        .into_iter()
        .map(|mut ax_element| {
            let frame_locator = ax_element
                .locator
                .as_ref()
                .and_then(|locator| ax_backend_dom_node_id_from_locator(&locator.selector))
                .map(|backend_node_id| ElementLocator {
                    ref_id: ax_element.element.ref_id,
                    role: ax_element.element.role.clone(),
                    text: ax_element.element.text.clone(),
                    selector: frame_ax_backend_dom_node_locator(
                        &session.session_id,
                        backend_node_id,
                    ),
                });
            if let Some(locator) = frame_locator {
                ax_element.element.locator = locator.selector.clone();
                ax_element.locator = Some(locator);
            } else {
                ax_element.locator = None;
                ax_element.element.locator.clear();
            }
            ax_element
                .element
                .attrs
                .insert("source".to_string(), "flat_session_ax".to_string());
            if ax_element.locator.is_some() {
                ax_element
                    .element
                    .attrs
                    .insert("ax_operable".to_string(), "true".to_string());
                ax_element.element.attrs.remove("readonly");
            } else {
                ax_element
                    .element
                    .attrs
                    .insert("readonly".to_string(), "true".to_string());
                ax_element.element.attrs.remove("ax_operable");
            }
            ax_element
                .element
                .attrs
                .insert("frame_session_id".to_string(), session.session_id.clone());
            ax_element
                .element
                .attrs
                .insert("frame".to_string(), "oopif".to_string());
            if !session.target_info.target_id.is_empty() {
                ax_element.element.attrs.insert(
                    "frame_target_id".to_string(),
                    session.target_info.target_id.clone(),
                );
            }
            if !session.target_info.r#type.is_empty() {
                ax_element
                    .element
                    .attrs
                    .insert("frame_type".to_string(), session.target_info.r#type.clone());
            }
            if !session.target_info.url.is_empty() {
                ax_element
                    .element
                    .attrs
                    .insert("frame_url".to_string(), session.target_info.url.clone());
            }
            if !session.target_info.title.is_empty() {
                ax_element
                    .element
                    .attrs
                    .insert("frame_title".to_string(), session.target_info.title.clone());
            }
            ax_element
        })
        .collect()
}

fn is_flat_session_iframe_candidate(session: &FlatSessionInfo) -> bool {
    !session.session_id.trim().is_empty() && session.target_info.r#type == "iframe"
}

fn ax_node_depths(nodes: &[Value]) -> HashMap<String, u32> {
    let mut parent_by_child = HashMap::new();
    for node in nodes {
        let Some(parent_id) = node.get("nodeId").and_then(Value::as_str) else {
            continue;
        };
        if let Some(children) = node.get("childIds").and_then(Value::as_array) {
            for child in children.iter().filter_map(Value::as_str) {
                parent_by_child.insert(child.to_string(), parent_id.to_string());
            }
        }
    }
    let mut depths = HashMap::new();
    for node in nodes {
        let Some(node_id) = node.get("nodeId").and_then(Value::as_str) else {
            continue;
        };
        let mut depth = 0u32;
        let mut current = node_id;
        let mut guard = 0u32;
        while let Some(parent) = parent_by_child.get(current) {
            depth = depth.saturating_add(1);
            current = parent;
            guard = guard.saturating_add(1);
            if guard > 64 {
                break;
            }
        }
        depths.insert(node_id.to_string(), depth);
    }
    depths
}

fn should_include_ax_only_node(role: &str, text: &str) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    !matches!(
        role,
        "generic" | "none" | "StaticText" | "InlineTextBox" | "LineBreak"
    )
}

fn is_ax_operable_role(role: &str) -> bool {
    matches!(
        role.to_ascii_lowercase().as_str(),
        "button"
            | "link"
            | "checkbox"
            | "radio"
            | "switch"
            | "textbox"
            | "searchbox"
            | "combobox"
            | "listbox"
            | "menuitem"
            | "menuitemcheckbox"
            | "menuitemradio"
            | "option"
            | "tab"
            | "slider"
            | "spinbutton"
    )
}

fn ax_node_signature(role: &str, text: &str) -> Option<String> {
    let role = role.trim();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if role.is_empty() || text.is_empty() {
        return None;
    }
    Some(format!(
        "{}\u{1f}{}",
        role.to_ascii_lowercase(),
        text.to_ascii_lowercase()
    ))
}

fn ax_value_to_string(value: &Value) -> Option<String> {
    let raw = value.get("value")?;
    match raw {
        Value::String(s) => Some(s.clone()),
        Value::Bool(v) => Some(v.to_string()),
        Value::Number(v) => Some(v.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn lock_tests() -> tokio::sync::MutexGuard<'static, ()> {
        // Shared with the sync registry tests so neither races on the global
        // REGISTRY when the suite runs in parallel.
        crate::browser::global_state_test_lock().lock().await
    }

    #[test]
    fn stale_ref_error_classifier_recognises_common_phrases() {
        for msg in [
            "Element not found for selector",
            "not found for selector",
            "Cannot find context with specified id",
            "Execution context was destroyed.",
        ] {
            assert!(is_stale_ref_error(&anyhow!(msg)), "{msg}");
        }
        assert!(!is_stale_ref_error(&anyhow!("permission denied")));
    }

    #[test]
    fn point_coord_reads_numeric_coordinate() {
        let point = json!({ "x": 12.5, "y": 7 });
        assert_eq!(point_coord(&point, "x").unwrap(), 12.5);
        assert_eq!(point_coord(&point, "y").unwrap(), 7.0);
        assert!(point_coord(&json!({ "x": "12" }), "x").is_err());
    }

    #[test]
    fn validate_clip_requires_positive_bounds() {
        assert!(validate_clip(&json!({
            "x": 0,
            "y": 1,
            "width": 10,
            "height": 20,
            "scale": 1,
        }))
        .is_ok());
        assert!(validate_clip(&json!({
            "x": 0,
            "y": 1,
            "width": 0,
            "height": 20,
            "scale": 1,
        }))
        .is_err());
    }

    #[test]
    fn parses_frame_clip_result_with_metadata() {
        let result = json!({
            "ok": true,
            "url": "https://frame.example.test/",
            "title": "Frame",
            "clip": {
                "x": 12,
                "y": 34,
                "width": 56,
                "height": 78,
                "scale": 1
            }
        });
        let clip = parse_frame_clip_result(&result).unwrap();
        assert_eq!(clip.url, "https://frame.example.test/");
        assert_eq!(clip.title, "Frame");
        assert_eq!(point_coord(&clip.clip, "x").unwrap(), 12.0);
    }

    #[test]
    fn pointer_target_from_clip_uses_clip_center_and_session() {
        let pointer = pointer_target_from_clip(
            Some("session-1".to_string()),
            &json!({
                "x": 10,
                "y": 20,
                "width": 30,
                "height": 40,
                "scale": 1
            }),
        )
        .unwrap();

        assert_eq!(
            pointer,
            PointerTarget {
                session_id: Some("session-1".to_string()),
                x: 25.0,
                y: 40.0,
            }
        );
    }

    #[test]
    fn drag_move_points_interpolates_only_within_one_coordinate_space() {
        let source = PointerTarget {
            session_id: None,
            x: 0.0,
            y: 0.0,
        };
        let target = PointerTarget {
            session_id: None,
            x: 40.0,
            y: 80.0,
        };
        let moves = drag_move_points(&source, &target);
        assert_eq!(moves.len(), 4);
        assert_eq!(moves[0].x, 10.0);
        assert_eq!(moves[0].y, 20.0);
        assert_eq!(moves[3], target);

        let frame_target = PointerTarget {
            session_id: Some("frame-session".to_string()),
            x: 7.0,
            y: 9.0,
        };
        let cross_session_moves = drag_move_points(&source, &frame_target);
        assert_eq!(cross_session_moves, vec![source, frame_target]);
    }

    #[test]
    fn frame_clip_result_requires_valid_clip_and_url() {
        assert!(parse_frame_clip_result(&json!({
            "url": "https://frame.example.test/",
            "clip": { "x": 0, "y": 0, "width": 0, "height": 1, "scale": 1 }
        }))
        .is_err());
        assert!(parse_frame_clip_result(&json!({
            "clip": { "x": 0, "y": 0, "width": 1, "height": 1, "scale": 1 }
        }))
        .is_err());
    }

    #[test]
    fn selects_unique_flat_session_for_frame_context() {
        let sessions = vec![
            FlatSessionInfo {
                session_id: "s1".to_string(),
                target_info: FlatTargetInfo {
                    r#type: "iframe".to_string(),
                    url: "https://a.example.test/".to_string(),
                    title: "A".to_string(),
                    ..FlatTargetInfo::default()
                },
                ..FlatSessionInfo::default()
            },
            FlatSessionInfo {
                session_id: "worker".to_string(),
                target_info: FlatTargetInfo {
                    r#type: "worker".to_string(),
                    url: "https://a.example.test/".to_string(),
                    title: "A".to_string(),
                    ..FlatTargetInfo::default()
                },
                ..FlatSessionInfo::default()
            },
        ];

        assert_eq!(
            select_flat_session_for_frame_context(&sessions, 17, "https://a.example.test/", "")
                .unwrap(),
            "s1"
        );
    }

    #[test]
    fn selects_flat_session_by_title_when_url_is_shared() {
        let sessions = vec![
            FlatSessionInfo {
                session_id: "s1".to_string(),
                target_info: FlatTargetInfo {
                    r#type: "iframe".to_string(),
                    url: "https://shared.example.test/".to_string(),
                    title: "One".to_string(),
                    ..FlatTargetInfo::default()
                },
                ..FlatSessionInfo::default()
            },
            FlatSessionInfo {
                session_id: "s2".to_string(),
                target_info: FlatTargetInfo {
                    r#type: "iframe".to_string(),
                    url: "https://shared.example.test/".to_string(),
                    title: "Two".to_string(),
                    ..FlatTargetInfo::default()
                },
                ..FlatSessionInfo::default()
            },
        ];

        assert_eq!(
            select_flat_session_for_frame_context(
                &sessions,
                17,
                "https://shared.example.test/",
                "Two"
            )
            .unwrap(),
            "s2"
        );
    }

    #[test]
    fn flat_session_frame_context_mapping_fails_closed_when_ambiguous() {
        let sessions = vec![
            FlatSessionInfo {
                session_id: "s1".to_string(),
                target_info: FlatTargetInfo {
                    r#type: "iframe".to_string(),
                    url: "https://shared.example.test/".to_string(),
                    title: "Same".to_string(),
                    ..FlatTargetInfo::default()
                },
                ..FlatSessionInfo::default()
            },
            FlatSessionInfo {
                session_id: "s2".to_string(),
                target_info: FlatTargetInfo {
                    r#type: "iframe".to_string(),
                    url: "https://shared.example.test/".to_string(),
                    title: "Same".to_string(),
                    ..FlatTargetInfo::default()
                },
                ..FlatSessionInfo::default()
            },
        ];

        assert!(select_flat_session_for_frame_context(
            &sessions,
            17,
            "https://shared.example.test/",
            "Same"
        )
        .is_err());
        assert!(select_flat_session_for_frame_context(
            &sessions,
            17,
            "https://missing.example.test/",
            ""
        )
        .is_err());
    }

    #[test]
    fn flat_session_frame_context_prefers_matched_frame_id() {
        let sessions = vec![
            FlatSessionInfo {
                session_id: "s1".to_string(),
                target_info: FlatTargetInfo {
                    r#type: "iframe".to_string(),
                    url: "https://shared.example.test/".to_string(),
                    title: "Same".to_string(),
                    ..FlatTargetInfo::default()
                },
                matched_frame: Some(FlatMatchedFrame {
                    status: "matched".to_string(),
                    frame_id: Some(17),
                }),
            },
            FlatSessionInfo {
                session_id: "s2".to_string(),
                target_info: FlatTargetInfo {
                    r#type: "iframe".to_string(),
                    url: "https://shared.example.test/".to_string(),
                    title: "Same".to_string(),
                    ..FlatTargetInfo::default()
                },
                matched_frame: Some(FlatMatchedFrame {
                    status: "matched".to_string(),
                    frame_id: Some(18),
                }),
            },
        ];

        assert_eq!(
            select_flat_session_for_frame_context(
                &sessions,
                17,
                "https://shared.example.test/",
                "Same"
            )
            .unwrap(),
            "s1"
        );
    }

    #[test]
    fn parses_accessibility_node_role_name_value() {
        let tree = json!({
            "nodes": [
                { "ignored": true, "role": { "value": "generic" } },
                {
                    "ignored": false,
                    "role": { "value": "button" },
                    "name": { "value": "Submit" },
                    "value": { "value": "Ready" }
                }
            ]
        });
        assert_eq!(
            parse_ax_element_info(&tree),
            Some(AxElementInfo {
                role: "button".to_string(),
                name: Some("Submit".to_string()),
                value: Some("Ready".to_string()),
            })
        );
    }

    #[test]
    fn parses_ax_only_elements_and_deduplicates_dom_signatures() {
        let tree = json!({
            "nodes": [
                {
                    "nodeId": "1",
                    "ignored": false,
                    "role": { "value": "RootWebArea" },
                    "name": { "value": "Dashboard" },
                    "childIds": ["2", "3", "4"]
                },
                {
                    "nodeId": "2",
                    "ignored": false,
                    "role": { "value": "button" },
                    "name": { "value": "Save" }
                },
                {
                    "nodeId": "3",
                    "ignored": false,
                    "role": { "value": "link" },
                    "name": { "value": "Docs" },
                    "value": { "value": "https://example.com/docs" }
                },
                {
                    "nodeId": "4",
                    "ignored": false,
                    "role": { "value": "generic" },
                    "name": { "value": "Layout wrapper" }
                }
            ]
        });
        let mut seen = HashSet::from([ax_node_signature("button", "Save").unwrap()]);
        let elements = parse_ax_only_elements(&tree, &mut seen, 50, 10);
        assert_eq!(elements.len(), 2);
        assert_eq!(elements[0].element.ref_id, 50);
        assert_eq!(elements[0].element.role, "RootWebArea");
        assert_eq!(elements[0].element.text, "Dashboard");
        assert_eq!(
            elements[0]
                .element
                .attrs
                .get("readonly")
                .map(String::as_str),
            Some("true")
        );
        assert!(elements[0].locator.is_none());
        assert_eq!(elements[1].element.role, "link");
        assert_eq!(elements[1].element.depth, 1);
        assert_eq!(
            elements[1]
                .element
                .attrs
                .get("ax_value")
                .map(String::as_str),
            Some("https://example.com/docs")
        );
    }

    #[test]
    fn ax_only_backend_dom_nodes_can_be_operable_refs() {
        let tree = json!({
            "nodes": [
                {
                    "nodeId": "1",
                    "backendDOMNodeId": 42,
                    "ignored": false,
                    "role": { "value": "button" },
                    "name": { "value": "Authorize" }
                },
                {
                    "nodeId": "2",
                    "backendDOMNodeId": 43,
                    "ignored": false,
                    "role": { "value": "generic" },
                    "name": { "value": "Panel" }
                }
            ]
        });
        let mut seen = HashSet::new();
        let elements = parse_ax_only_elements(&tree, &mut seen, 80, 10);
        assert_eq!(elements.len(), 1);
        let first = &elements[0];
        assert_eq!(first.element.ref_id, 80);
        assert_eq!(first.element.role, "button");
        assert_eq!(first.element.text, "Authorize");
        assert_eq!(
            first.element.attrs.get("ax_operable").map(String::as_str),
            Some("true")
        );
        assert!(!first.element.attrs.contains_key("readonly"));
        assert_eq!(
            first
                .locator
                .as_ref()
                .map(|locator| locator.selector.as_str()),
            Some("ax_backend_dom_node_id:42")
        );
        assert_eq!(
            ax_backend_dom_node_id_from_locator(&first.element.locator),
            Some(42)
        );
    }

    #[test]
    fn frame_ax_backend_dom_node_locator_round_trips_session_and_node() {
        let locator = frame_ax_backend_dom_node_locator("session:with:colon", 42);
        assert_eq!(
            frame_ax_backend_dom_node_id_from_locator(&locator),
            Some(("session:with:colon".to_string(), 42))
        );
        assert_eq!(
            frame_ax_backend_dom_node_id_from_locator("frame_ax_backend_dom_node_id::42"),
            None
        );
        assert_eq!(
            frame_ax_backend_dom_node_id_from_locator("ax_backend_dom_node_id:42"),
            None
        );
    }

    #[test]
    fn flat_session_ax_backend_dom_nodes_become_session_operable_refs() {
        let tree = json!({
            "nodes": [
                {
                    "nodeId": "1",
                    "backendDOMNodeId": 42,
                    "ignored": false,
                    "role": { "value": "button" },
                    "name": { "value": "Authorize" }
                }
            ]
        });
        let session = FlatSessionInfo {
            session_id: "session-1".to_string(),
            target_info: FlatTargetInfo {
                target_id: "target-1".to_string(),
                r#type: "iframe".to_string(),
                title: "Embedded Login".to_string(),
                url: "https://login.example.test/".to_string(),
            },
            ..FlatSessionInfo::default()
        };
        let mut seen = HashSet::new();
        let elements = parse_flat_session_ax_only_elements(&tree, &mut seen, 90, 10, &session);

        assert_eq!(elements.len(), 1);
        let first = &elements[0];
        assert_eq!(
            first
                .locator
                .as_ref()
                .map(|locator| locator.selector.as_str()),
            Some("frame_ax_backend_dom_node_id:session-1:42")
        );
        assert_eq!(
            first.element.locator,
            "frame_ax_backend_dom_node_id:session-1:42"
        );
        assert_eq!(first.element.ref_id, 90);
        assert_eq!(first.element.role, "button");
        assert_eq!(first.element.text, "Authorize");
        assert_eq!(
            first.element.attrs.get("source").map(String::as_str),
            Some("flat_session_ax")
        );
        assert_eq!(
            first.element.attrs.get("readonly").map(String::as_str),
            None
        );
        assert_eq!(
            first.element.attrs.get("ax_operable").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            first
                .element
                .attrs
                .get("backend_dom_node_id")
                .map(String::as_str),
            Some("42")
        );
        assert_eq!(
            first
                .element
                .attrs
                .get("frame_session_id")
                .map(String::as_str),
            Some("session-1")
        );
        assert_eq!(
            first.element.attrs.get("frame_type").map(String::as_str),
            Some("iframe")
        );
        assert_eq!(
            first.element.attrs.get("frame_url").map(String::as_str),
            Some("https://login.example.test/")
        );
    }

    #[test]
    fn flat_session_ax_nodes_without_backend_dom_node_stay_read_only() {
        let tree = json!({
            "nodes": [
                {
                    "nodeId": "1",
                    "ignored": false,
                    "role": { "value": "heading" },
                    "name": { "value": "Security Notice" }
                }
            ]
        });
        let session = FlatSessionInfo {
            session_id: "session-1".to_string(),
            target_info: FlatTargetInfo {
                r#type: "iframe".to_string(),
                url: "https://login.example.test/".to_string(),
                ..FlatTargetInfo::default()
            },
            ..FlatSessionInfo::default()
        };
        let mut seen = HashSet::new();
        let elements = parse_flat_session_ax_only_elements(&tree, &mut seen, 91, 10, &session);

        assert_eq!(elements.len(), 1);
        let first = &elements[0];
        assert!(first.locator.is_none());
        assert!(first.element.locator.is_empty());
        assert_eq!(
            first.element.attrs.get("readonly").map(String::as_str),
            Some("true")
        );
        assert!(!first.element.attrs.contains_key("ax_operable"));
    }

    #[test]
    fn flat_session_iframe_candidates_are_iframe_sessions_only() {
        assert!(is_flat_session_iframe_candidate(&FlatSessionInfo {
            session_id: "session-1".to_string(),
            target_info: FlatTargetInfo {
                r#type: "iframe".to_string(),
                ..FlatTargetInfo::default()
            },
            ..FlatSessionInfo::default()
        }));
        assert!(!is_flat_session_iframe_candidate(&FlatSessionInfo {
            session_id: String::new(),
            target_info: FlatTargetInfo {
                r#type: "iframe".to_string(),
                ..FlatTargetInfo::default()
            },
            ..FlatSessionInfo::default()
        }));
        assert!(!is_flat_session_iframe_candidate(&FlatSessionInfo {
            session_id: "session-1".to_string(),
            target_info: FlatTargetInfo {
                r#type: "worker".to_string(),
                ..FlatTargetInfo::default()
            },
            ..FlatSessionInfo::default()
        }));
    }

    #[test]
    fn action_scripts_use_iframe_aware_selector_resolver() {
        let params = ActParams {
            ref_id: Some(1),
            ..ActParams::default()
        };
        let selector = serde_json::to_string("iframe#child >>> button.save").unwrap();
        let script = build_action_script(ActKind::Click, &params, &selector).unwrap();
        assert!(script.contains("__hopeResolveSelector"));
        assert!(script.contains("iframe#child >>> button.save"));
        assert!(!script.contains("document.querySelector"));
    }

    #[test]
    fn frame_locator_round_trips_frame_id_and_selector() {
        let locator = frame_locator(17, "button.pay-now");
        assert_eq!(locator, "frame:17 >>> button.pay-now");
        assert_eq!(frame_locator_parts(&locator), Some((17, "button.pay-now")));
        assert_eq!(frame_locator_parts("iframe#child >>> button"), None);
        assert_eq!(frame_locator_parts("frame:bad >>> button"), None);
    }

    #[test]
    fn cross_frame_drag_target_must_be_in_same_frame() {
        let target = ElementLocator {
            ref_id: 2,
            role: "button".to_string(),
            text: "Drop".to_string(),
            selector: frame_locator(17, "div.drop-zone"),
        };
        assert_eq!(
            same_frame_drag_target_selector(17, &target).unwrap(),
            "div.drop-zone"
        );

        let other_frame = ElementLocator {
            selector: frame_locator(18, "div.drop-zone"),
            ..target.clone()
        };
        assert!(same_frame_drag_target_selector(17, &other_frame)
            .unwrap_err()
            .to_string()
            .contains("different cross-origin iframe"));

        let root_target = ElementLocator {
            selector: "div.drop-zone".to_string(),
            ..target
        };
        assert!(same_frame_drag_target_selector(17, &root_target)
            .unwrap_err()
            .to_string()
            .contains("root-frame"));
    }

    #[test]
    fn frame_action_params_keep_only_serialisable_user_inputs() {
        let params = ActParams {
            text: Some("hello".to_string()),
            key: Some("Enter".to_string()),
            values: Some(vec!["a".to_string(), "b".to_string()]),
            ..ActParams::default()
        };
        assert_eq!(
            frame_action_params(&params),
            json!({
                "text": "hello",
                "key": "Enter",
                "values": ["a", "b"],
            })
        );
        assert_eq!(act_kind_wire_name(ActKind::DoubleClick), "double_click");
    }

    #[test]
    fn cdp_policy_allows_only_known_high_level_methods() {
        assert!(validate_cdp_method("Runtime.evaluate").is_ok());
        assert!(validate_cdp_method("Runtime.callFunctionOn").is_ok());
        assert!(validate_cdp_method("DOM.resolveNode").is_ok());
        assert!(validate_cdp_method("Accessibility.getFullAXTree").is_ok());
        assert!(validate_cdp_method("Accessibility.getPartialAXTree").is_ok());
        assert!(validate_cdp_method("Page.captureScreenshot").is_ok());
        let err = validate_cdp_method("DOM.getDocument").unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }

    #[test]
    fn cdp_policy_blocks_high_risk_domains() {
        let err = validate_cdp_method("Target.getTargets").unwrap_err();
        assert!(err.to_string().contains("blocked"));
        let err = validate_cdp_method("Browser.getVersion").unwrap_err();
        assert!(err.to_string().contains("blocked"));
    }

    #[test]
    fn raw_cdp_allows_advanced_methods_but_enforces_blocklist() {
        // raw_cdp bypasses the ALLOWED_CDP_METHODS whitelist, so well-formed
        // methods outside it are accepted (that is the escape hatch's purpose).
        assert!(validate_raw_cdp_method("Accessibility.getFullAXTree").is_ok());
        assert!(validate_raw_cdp_method("DOMSnapshot.captureSnapshot").is_ok());
        assert!(validate_raw_cdp_method("Input.dispatchMouseEvent").is_ok());
        assert!(validate_raw_cdp_method("Page.navigate").is_ok());
        assert!(validate_raw_cdp_method("Runtime.getProperties").is_ok());
        // Network.enable is a legitimate, non-credential method and stays usable.
        assert!(validate_raw_cdp_method("Network.enable").is_ok());
        // The safety blocklist still applies: dangerous domains and the
        // cookie/credential-bearing Network.* methods are rejected.
        assert!(validate_raw_cdp_method("Browser.getVersion").is_err());
        assert!(validate_raw_cdp_method("Target.getTargets").is_err());
        assert!(validate_raw_cdp_method("Storage.clearDataForOrigin").is_err());
        assert!(validate_raw_cdp_method("Fetch.enable").is_err());
        assert!(validate_raw_cdp_method("Network.getCookies").is_err());
        assert!(validate_raw_cdp_method("Network.getAllCookies").is_err());
        assert!(validate_raw_cdp_method("Network.clearBrowserCookies").is_err());
    }

    #[test]
    fn raw_cdp_rejects_malformed_methods() {
        for method in ["DOM", "DOM.", ".getDocument", "DOM.getDocument;alert(1)"] {
            assert!(validate_raw_cdp_method(method).is_err(), "{method}");
        }
    }

    #[tokio::test]
    async fn cleanup_extension_session_clears_registry_without_broker() {
        let _guard = lock_tests().await;
        registry::reset_for_tests();
        let ctx = BrowserBackendContext {
            session_id: Some("browser-cleanup-test".to_string()),
            ..BrowserBackendContext::default()
        };
        registry::record_agent_tab(&ctx, 101, None, None).unwrap();
        assert_eq!(registry::active_tab_id(&ctx), Some(101));

        let result = cleanup_extension_session("browser-cleanup-test").await;
        assert!(result.contains("Released local browser control registry"));
        assert_eq!(registry::active_tab_id(&ctx), None);
    }

    #[tokio::test]
    async fn stop_all_extension_control_clears_registry_without_broker() {
        let _guard = lock_tests().await;
        registry::reset_for_tests();
        let a = BrowserBackendContext {
            session_id: Some("browser-stop-a".to_string()),
            ..BrowserBackendContext::default()
        };
        let b = BrowserBackendContext {
            session_id: Some("browser-stop-b".to_string()),
            ..BrowserBackendContext::default()
        };
        registry::claim_user_tab(&a, 201, None, None, false).unwrap();
        registry::record_agent_tab(&b, 202, None, None).unwrap();

        let result = stop_all_extension_control().await;

        assert_eq!(result.stopped_tabs, 2);
        assert!(result
            .message
            .contains("Released local browser control registry"));
        assert_eq!(registry::active_tab_id(&a), None);
        assert_eq!(registry::active_tab_id(&b), None);
    }
}
