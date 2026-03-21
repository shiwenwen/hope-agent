use anyhow::Result;
use serde_json::Value;

use crate::browser_state::{self, ElementRef, get_browser_state};

/// Image base64 prefix marker — detected by agent.rs for multimodal content
pub const IMAGE_BASE64_PREFIX: &str = "__IMAGE_BASE64__";

pub(crate) async fn tool_browser(args: &Value) -> Result<String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

    match action {
        "connect" => action_connect(args).await,
        "launch" => action_launch(args).await,
        "disconnect" => action_disconnect().await,
        "list_pages" => action_list_pages().await,
        "new_page" => action_new_page(args).await,
        "select_page" => action_select_page(args).await,
        "close_page" => action_close_page(args).await,
        "navigate" => action_navigate(args).await,
        "go_back" => action_go_back().await,
        "go_forward" => action_go_forward().await,
        "take_snapshot" => action_take_snapshot().await,
        "take_screenshot" => action_take_screenshot(args).await,
        "click" => action_click(args).await,
        "fill" => action_fill(args).await,
        "fill_form" => action_fill_form(args).await,
        "hover" => action_hover(args).await,
        "drag" => action_drag(args).await,
        "press_key" => action_press_key(args).await,
        "upload_file" => action_upload_file(args).await,
        "evaluate" => action_evaluate(args).await,
        "wait_for" => action_wait_for(args).await,
        "handle_dialog" => action_handle_dialog(args).await,
        "resize" => action_resize(args).await,
        "scroll" => action_scroll(args).await,
        _ => Err(anyhow::anyhow!(
            "Unknown browser action: '{}'. Available: connect, launch, disconnect, list_pages, new_page, select_page, close_page, navigate, go_back, go_forward, take_snapshot, take_screenshot, click, fill, fill_form, hover, drag, press_key, upload_file, evaluate, wait_for, handle_dialog, resize, scroll",
            action
        )),
    }
}

// ── Helpers ──────────────────────────────────────────────────────

async fn require_browser() -> Result<()> {
    browser_state::ensure_connected().await
}

fn get_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| {
        v.as_str().or_else(|| v.get("text").and_then(|t| t.as_str()))
    })
}

fn get_u32(args: &Value, key: &str) -> Option<u32> {
    args.get(key).and_then(|v| v.as_u64()).map(|v| v as u32)
}

fn get_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|v| v.as_i64())
}

fn get_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|v| v.as_bool())
}

// ══════════════════════════════════════════════════════════════════
// Connection Actions
// ══════════════════════════════════════════════════════════════════

async fn action_connect(args: &Value) -> Result<String> {
    let url = get_str(args, "url").unwrap_or("http://localhost:9222");

    let mut state = get_browser_state().lock().await;
    if state.is_connected() {
        state.disconnect().await;
    }

    state.connect(url).await?;

    let page_count = state.pages.len();
    let active = state.active_page_id.clone().unwrap_or_default();

    Ok(format!(
        "Connected to Chrome at {}. Found {} page(s). Active page: {}",
        url, page_count, active
    ))
}

async fn action_launch(args: &Value) -> Result<String> {
    let executable = get_str(args, "executable_path");
    let headless = get_bool(args, "headless").unwrap_or(false);

    let mut state = get_browser_state().lock().await;
    if state.is_connected() {
        state.disconnect().await;
    }

    state.launch(executable, headless).await?;

    let page_count = state.pages.len();

    Ok(format!(
        "Chrome launched successfully{}. {} page(s) available.",
        if headless { " (headless)" } else { "" },
        page_count
    ))
}

async fn action_disconnect() -> Result<String> {
    let mut state = get_browser_state().lock().await;
    if !state.is_connected() {
        return Ok("Not connected to any browser.".to_string());
    }
    state.disconnect().await;
    Ok("Browser disconnected.".to_string())
}

// ══════════════════════════════════════════════════════════════════
// Page/Tab Management
// ══════════════════════════════════════════════════════════════════

async fn action_list_pages() -> Result<String> {
    require_browser().await?;
    let mut state = get_browser_state().lock().await;
    state.refresh_pages().await?;

    if state.pages.is_empty() {
        return Ok("No pages open.".to_string());
    }

    let active_id = state.active_page_id.clone().unwrap_or_default();
    let mut lines = vec!["Open pages:".to_string()];

    for (id, page) in &state.pages {
        let url = page.url().await
            .ok()
            .flatten()
            .unwrap_or_else(|| "about:blank".to_string());
        let marker = if *id == active_id { " [active]" } else { "" };
        lines.push(format!("  - {} {}{}", id, url, marker));
    }

    Ok(lines.join("\n"))
}

async fn action_new_page(args: &Value) -> Result<String> {
    require_browser().await?;
    let url = get_str(args, "url").unwrap_or("about:blank");

    let mut state = get_browser_state().lock().await;
    let browser = state.browser.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not connected"))?;

    let page = browser.new_page(url).await
        .map_err(|e| anyhow::anyhow!("Failed to create new page: {}", e))?;

    let target_id = page.target_id().as_ref().to_string();
    state.active_page_id = Some(target_id.clone());
    state.pages.insert(target_id.clone(), page);
    state.element_refs.clear();
    state.snapshot_url = None;

    Ok(format!("New page created: {} (url: {})", target_id, url))
}

async fn action_select_page(args: &Value) -> Result<String> {
    require_browser().await?;
    let page_id = get_str(args, "page_id")
        .ok_or_else(|| anyhow::anyhow!("Missing 'page_id' parameter"))?;

    let mut state = get_browser_state().lock().await;

    if !state.pages.contains_key(page_id) {
        let available: Vec<&String> = state.pages.keys().collect();
        return Err(anyhow::anyhow!(
            "Page '{}' not found. Available pages: {:?}",
            page_id, available
        ));
    }

    state.active_page_id = Some(page_id.to_string());
    state.element_refs.clear();
    state.snapshot_url = None;

    Ok(format!("Switched to page: {}", page_id))
}

async fn action_close_page(args: &Value) -> Result<String> {
    require_browser().await?;
    let page_id = get_str(args, "page_id")
        .ok_or_else(|| anyhow::anyhow!("Missing 'page_id' parameter"))?;

    let mut state = get_browser_state().lock().await;

    let page = state.pages.remove(page_id)
        .ok_or_else(|| anyhow::anyhow!("Page '{}' not found", page_id))?;

    let _ = page.close().await;

    if state.active_page_id.as_deref() == Some(page_id) {
        state.active_page_id = state.pages.keys().next().cloned();
        state.element_refs.clear();
        state.snapshot_url = None;
    }

    Ok(format!("Page '{}' closed.", page_id))
}

// ══════════════════════════════════════════════════════════════════
// Navigation
// ══════════════════════════════════════════════════════════════════

async fn action_navigate(args: &Value) -> Result<String> {
    require_browser().await?;
    let url = get_str(args, "url")
        .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;

    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    page.goto(url).await
        .map_err(|e| anyhow::anyhow!("Navigation failed: {}", e))?;

    // Wait a bit for page load
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let title: String = page.evaluate("document.title")
        .await
        .ok()
        .and_then(|r| r.into_value().ok())
        .unwrap_or_else(|| "untitled".to_string());

    let current_url = page.url().await
        .ok()
        .flatten()
        .unwrap_or_else(|| url.to_string());

    // Clear element refs (page changed)
    drop(state);
    let mut state = get_browser_state().lock().await;
    state.element_refs.clear();
    state.snapshot_url = None;

    Ok(format!("Navigated to: {} - \"{}\"", current_url, title))
}

async fn action_go_back() -> Result<String> {
    require_browser().await?;
    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    page.evaluate("history.back()").await
        .map_err(|e| anyhow::anyhow!("Go back failed: {}", e))?;

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let url = page.url().await.ok().flatten().unwrap_or_else(|| "unknown".to_string());

    drop(state);
    let mut state = get_browser_state().lock().await;
    state.element_refs.clear();
    state.snapshot_url = None;

    Ok(format!("Navigated back to: {}", url))
}

async fn action_go_forward() -> Result<String> {
    require_browser().await?;
    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    page.evaluate("history.forward()").await
        .map_err(|e| anyhow::anyhow!("Go forward failed: {}", e))?;

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let url = page.url().await.ok().flatten().unwrap_or_else(|| "unknown".to_string());

    drop(state);
    let mut state = get_browser_state().lock().await;
    state.element_refs.clear();
    state.snapshot_url = None;

    Ok(format!("Navigated forward to: {}", url))
}

// ══════════════════════════════════════════════════════════════════
// Snapshot (Accessibility Tree)
// ══════════════════════════════════════════════════════════════════

/// JavaScript injected into the page to extract an accessibility-like element tree
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

async fn action_take_snapshot() -> Result<String> {
    require_browser().await?;
    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    let json_str: String = page.evaluate(SNAPSHOT_JS)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to take snapshot: {}", e))?
        .into_value()
        .map_err(|e| anyhow::anyhow!("Snapshot returned invalid data: {}", e))?;

    let data: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| anyhow::anyhow!("Failed to parse snapshot: {}", e))?;

    let url = data.get("url").and_then(|v| v.as_str()).unwrap_or("unknown");
    let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("untitled");
    let viewport_w = data.get("viewport").and_then(|v| v.get("w")).and_then(|v| v.as_i64()).unwrap_or(0);
    let viewport_h = data.get("viewport").and_then(|v| v.get("h")).and_then(|v| v.as_i64()).unwrap_or(0);
    let truncated = data.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false);
    let elements = data.get("elements").and_then(|v| v.as_array());

    let mut output = format!(
        "[Page Snapshot] {} - \"{}\"\nViewport: {}x{}\n\n",
        url, title, viewport_w, viewport_h
    );

    let mut new_refs: Vec<ElementRef> = Vec::new();

    if let Some(elements) = elements {
        for el in elements {
            let ref_id = el.get("ref").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let depth = el.get("depth").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let role = el.get("role").and_then(|v| v.as_str()).unwrap_or("unknown");
            let text = el.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let selector = el.get("selector").and_then(|v| v.as_str()).unwrap_or("");
            let cx = el.get("cx").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let cy = el.get("cy").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let attrs = el.get("attrs").cloned().unwrap_or(serde_json::json!({}));

            let indent = "  ".repeat(depth);
            let mut line = format!("{}[ref={}] {}", indent, ref_id, role);

            if !text.is_empty() {
                line.push_str(&format!(" \"{}\"", text));
            }

            if let Some(url_attr) = attrs.get("url").and_then(|v| v.as_str()) {
                line.push_str(&format!(" url={}", url_attr));
            }
            if let Some(value) = attrs.get("value").and_then(|v| v.as_str()) {
                line.push_str(&format!(" value=\"{}\"", value));
            }
            if let Some(placeholder) = attrs.get("placeholder").and_then(|v| v.as_str()) {
                line.push_str(&format!(" placeholder=\"{}\"", placeholder));
            }
            if let Some(level) = attrs.get("level").and_then(|v| v.as_i64()) {
                line.push_str(&format!(" (h{})", level));
            }
            if attrs.get("checked").and_then(|v| v.as_bool()) == Some(true) {
                line.push_str(" [checked]");
            }
            if attrs.get("disabled").and_then(|v| v.as_bool()) == Some(true) {
                line.push_str(" [disabled]");
            }

            output.push_str(&line);
            output.push('\n');

            let mut attr_map = std::collections::HashMap::new();
            if let Some(obj) = attrs.as_object() {
                for (k, v) in obj {
                    if let Some(s) = v.as_str() {
                        attr_map.insert(k.clone(), s.to_string());
                    } else {
                        attr_map.insert(k.clone(), v.to_string());
                    }
                }
            }

            new_refs.push(ElementRef {
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

    if truncated {
        output.push_str("\n[Truncated: max 300 elements reached. Use 'evaluate' to query specific areas.]\n");
    }

    drop(state);
    let mut state = get_browser_state().lock().await;
    state.element_refs = new_refs;
    state.snapshot_url = Some(url.to_string());

    Ok(output)
}

// ══════════════════════════════════════════════════════════════════
// Screenshot
// ══════════════════════════════════════════════════════════════════

async fn action_take_screenshot(args: &Value) -> Result<String> {
    require_browser().await?;
    let format = get_str(args, "format").unwrap_or("png");
    let full_page = get_bool(args, "full_page").unwrap_or(false);

    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    use chromiumoxide::page::ScreenshotParams;
    use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;

    let cdp_format = match format {
        "jpeg" | "jpg" => CaptureScreenshotFormat::Jpeg,
        _ => CaptureScreenshotFormat::Png,
    };

    let params = ScreenshotParams::builder()
        .format(cdp_format)
        .full_page(full_page)
        .build();

    let screenshot_bytes = page.screenshot(params).await
        .map_err(|e| anyhow::anyhow!("Screenshot failed: {}", e))?;

    let b64_data = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &screenshot_bytes,
    );

    let url = page.url().await
        .ok()
        .flatten()
        .unwrap_or_else(|| "unknown".to_string());

    let mime = if format == "jpeg" || format == "jpg" {
        "image/jpeg"
    } else {
        "image/png"
    };

    Ok(format!(
        "{}{}__{}__\nScreenshot captured (url: {}, format: {}{})",
        IMAGE_BASE64_PREFIX, mime, b64_data,
        url, format,
        if full_page { ", full page" } else { "" }
    ))
}

// ══════════════════════════════════════════════════════════════════
// Element Interaction
// ══════════════════════════════════════════════════════════════════

async fn action_click(args: &Value) -> Result<String> {
    require_browser().await?;
    let ref_id = get_u32(args, "ref")
        .ok_or_else(|| anyhow::anyhow!("Missing 'ref' parameter (element ref ID from snapshot)"))?;
    let double_click = get_bool(args, "double_click").unwrap_or(false);

    let state = get_browser_state().lock().await;
    let element_info = state.find_ref(ref_id)?.clone();
    let page = state.get_active_page()?;

    let el = page.find_element(&element_info.selector).await
        .map_err(|e| anyhow::anyhow!(
            "Element ref={} (selector: {}) not found on page: {}. Take a new snapshot.",
            ref_id, element_info.selector, e
        ))?;

    el.scroll_into_view().await.ok();
    el.click().await
        .map_err(|e| anyhow::anyhow!("Click failed: {}", e))?;

    if double_click {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        el.click().await.ok();
    }

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    Ok(format!(
        "Clicked{} [ref={}] {} \"{}\"",
        if double_click { " (double)" } else { "" },
        ref_id, element_info.role, element_info.text
    ))
}

async fn action_fill(args: &Value) -> Result<String> {
    require_browser().await?;
    let ref_id = get_u32(args, "ref")
        .ok_or_else(|| anyhow::anyhow!("Missing 'ref' parameter"))?;
    let value = get_str(args, "value")
        .ok_or_else(|| anyhow::anyhow!("Missing 'value' parameter"))?;

    let state = get_browser_state().lock().await;
    let element_info = state.find_ref(ref_id)?.clone();
    let page = state.get_active_page()?;

    let el = page.find_element(&element_info.selector).await
        .map_err(|e| anyhow::anyhow!(
            "Element ref={} not found: {}. Take a new snapshot.",
            ref_id, e
        ))?;

    el.scroll_into_view().await.ok();

    // Click to focus, clear existing content, then type new value
    el.click().await.ok();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Clear existing content via JS
    let clear_js = format!(
        "(() => {{ const el = document.querySelector('{}'); if (el) {{ el.value = ''; el.dispatchEvent(new Event('input', {{bubbles: true}})); }} }})()",
        element_info.selector.replace('\'', "\\'")
    );
    page.evaluate(clear_js).await.ok();

    // Type the new value
    el.type_str(value).await
        .map_err(|e| anyhow::anyhow!("Failed to type text: {}", e))?;

    Ok(format!(
        "Filled [ref={}] {} with \"{}\"",
        ref_id, element_info.role, value
    ))
}

async fn action_fill_form(args: &Value) -> Result<String> {
    require_browser().await?;
    let fields = args.get("fields")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow::anyhow!("Missing 'fields' parameter (object mapping ref IDs to values)"))?;

    let mut results = Vec::new();

    for (ref_key, value) in fields {
        let ref_id: u32 = ref_key.parse()
            .map_err(|_| anyhow::anyhow!("Invalid ref ID: '{}'. Must be a number.", ref_key))?;
        let val = value.as_str()
            .ok_or_else(|| anyhow::anyhow!("Value for ref {} must be a string", ref_id))?;

        let sub_args = serde_json::json!({
            "ref": ref_id,
            "value": val
        });

        match action_fill(&sub_args).await {
            Ok(msg) => results.push(msg),
            Err(e) => results.push(format!("Error filling ref={}: {}", ref_id, e)),
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    Ok(results.join("\n"))
}

async fn action_hover(args: &Value) -> Result<String> {
    require_browser().await?;
    let ref_id = get_u32(args, "ref")
        .ok_or_else(|| anyhow::anyhow!("Missing 'ref' parameter"))?;

    let state = get_browser_state().lock().await;
    let element_info = state.find_ref(ref_id)?.clone();
    let page = state.get_active_page()?;

    let el = page.find_element(&element_info.selector).await
        .map_err(|e| anyhow::anyhow!("Element ref={} not found: {}", ref_id, e))?;

    el.scroll_into_view().await.ok();

    // Get center point and dispatch mouse move
    let point = el.clickable_point().await
        .map_err(|e| anyhow::anyhow!("Cannot get element position: {}", e))?;

    use chromiumoxide::cdp::browser_protocol::input::{
        DispatchMouseEventParams, DispatchMouseEventType,
    };

    page.execute(
        DispatchMouseEventParams::new(DispatchMouseEventType::MouseMoved, point.x, point.y)
    ).await
        .map_err(|e| anyhow::anyhow!("Hover failed: {}", e))?;

    Ok(format!(
        "Hovered [ref={}] {} \"{}\"",
        ref_id, element_info.role, element_info.text
    ))
}

async fn action_drag(args: &Value) -> Result<String> {
    require_browser().await?;
    let from_ref = get_u32(args, "ref")
        .ok_or_else(|| anyhow::anyhow!("Missing 'ref' parameter (source element)"))?;
    let to_ref = get_u32(args, "target_ref")
        .ok_or_else(|| anyhow::anyhow!("Missing 'target_ref' parameter (destination element)"))?;

    let state = get_browser_state().lock().await;
    let from_el = state.find_ref(from_ref)?.clone();
    let to_el = state.find_ref(to_ref)?.clone();
    let page = state.get_active_page()?;

    let from_elem = page.find_element(&from_el.selector).await
        .map_err(|e| anyhow::anyhow!("Source element ref={} not found: {}", from_ref, e))?;
    let to_elem = page.find_element(&to_el.selector).await
        .map_err(|e| anyhow::anyhow!("Target element ref={} not found: {}", to_ref, e))?;

    let from_point = from_elem.clickable_point().await?;
    let to_point = to_elem.clickable_point().await?;

    use chromiumoxide::cdp::browser_protocol::input::{
        DispatchMouseEventParams, DispatchMouseEventType, MouseButton,
    };

    // Mouse down at source
    let mut down = DispatchMouseEventParams::new(
        DispatchMouseEventType::MousePressed, from_point.x, from_point.y
    );
    down.button = Some(MouseButton::Left);
    down.click_count = Some(1);
    page.execute(down).await?;

    // Move to destination
    let mut mv = DispatchMouseEventParams::new(
        DispatchMouseEventType::MouseMoved, to_point.x, to_point.y
    );
    mv.button = Some(MouseButton::Left);
    page.execute(mv).await?;

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Mouse up at destination
    let mut up = DispatchMouseEventParams::new(
        DispatchMouseEventType::MouseReleased, to_point.x, to_point.y
    );
    up.button = Some(MouseButton::Left);
    up.click_count = Some(1);
    page.execute(up).await?;

    Ok(format!(
        "Dragged [ref={}] \"{}\" -> [ref={}] \"{}\"",
        from_ref, from_el.text, to_ref, to_el.text
    ))
}

async fn action_press_key(args: &Value) -> Result<String> {
    require_browser().await?;
    let key = get_str(args, "key")
        .ok_or_else(|| anyhow::anyhow!("Missing 'key' parameter (e.g. 'Enter', 'Tab', 'Escape', 'a')"))?;

    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    use chromiumoxide::cdp::browser_protocol::input::{DispatchKeyEventParams, DispatchKeyEventType};

    let mut down = DispatchKeyEventParams::new(DispatchKeyEventType::KeyDown);
    down.key = Some(key.to_string());
    page.execute(down).await
        .map_err(|e| anyhow::anyhow!("Key press failed: {}", e))?;

    let mut up = DispatchKeyEventParams::new(DispatchKeyEventType::KeyUp);
    up.key = Some(key.to_string());
    page.execute(up).await.ok();

    Ok(format!("Pressed key: {}", key))
}

async fn action_upload_file(args: &Value) -> Result<String> {
    require_browser().await?;
    let ref_id = get_u32(args, "ref")
        .ok_or_else(|| anyhow::anyhow!("Missing 'ref' parameter"))?;
    let file_path = get_str(args, "file_path")
        .ok_or_else(|| anyhow::anyhow!("Missing 'file_path' parameter"))?;

    if !std::path::Path::new(file_path).exists() {
        return Err(anyhow::anyhow!("File not found: {}", file_path));
    }

    let state = get_browser_state().lock().await;
    let element_info = state.find_ref(ref_id)?.clone();
    let page = state.get_active_page()?;

    // Get the DOM node and set file via CDP
    use chromiumoxide::cdp::browser_protocol::dom::{GetDocumentParams, QuerySelectorParams, SetFileInputFilesParams};

    let doc = page.execute(GetDocumentParams::default()).await
        .map_err(|e| anyhow::anyhow!("Failed to get document: {}", e))?;

    let node_id = doc.result.root.node_id;

    let query_result = page.execute(
        QuerySelectorParams::new(node_id, &element_info.selector)
    ).await
        .map_err(|e| anyhow::anyhow!("Element ref={} not found for file upload: {}", ref_id, e))?;

    let file_node_id = query_result.result.node_id;

    let mut set_files = SetFileInputFilesParams::new(vec![file_path.to_string()]);
    set_files.node_id = Some(file_node_id);
    page.execute(set_files).await
        .map_err(|e| anyhow::anyhow!("Failed to set file: {}", e))?;

    Ok(format!("Uploaded file '{}' to [ref={}]", file_path, ref_id))
}

// ══════════════════════════════════════════════════════════════════
// JavaScript Execution & Waiting
// ══════════════════════════════════════════════════════════════════

async fn action_evaluate(args: &Value) -> Result<String> {
    require_browser().await?;
    let expression = get_str(args, "expression")
        .ok_or_else(|| anyhow::anyhow!("Missing 'expression' parameter"))?;

    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    let result = page.evaluate(expression).await
        .map_err(|e| anyhow::anyhow!("Script evaluation failed: {}", e))?;

    // Try to extract as string, then fall back to JSON
    let value: serde_json::Value = result.into_value()
        .unwrap_or(serde_json::Value::Null);

    let display = if value.is_string() {
        value.as_str().unwrap_or("").to_string()
    } else if value.is_null() {
        "undefined".to_string()
    } else {
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
    };

    Ok(format!("Result: {}", display))
}

async fn action_wait_for(args: &Value) -> Result<String> {
    require_browser().await?;
    let text = get_str(args, "text")
        .ok_or_else(|| anyhow::anyhow!("Missing 'text' parameter"))?;
    let timeout_ms = get_u32(args, "timeout").unwrap_or(30000) as u64;

    let check_js = format!(
        "document.body.innerText.includes('{}')",
        text.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n")
    );

    let start = std::time::Instant::now();
    let poll_interval = std::time::Duration::from_millis(500);

    loop {
        {
            let state = get_browser_state().lock().await;
            let page = state.get_active_page()?;

            let found: bool = page.evaluate(check_js.as_str()).await
                .ok()
                .and_then(|r| r.into_value().ok())
                .unwrap_or(false);

            if found {
                return Ok(format!("Text \"{}\" found on page.", text));
            }
        }

        if start.elapsed().as_millis() as u64 >= timeout_ms {
            return Err(anyhow::anyhow!(
                "Timeout after {}ms waiting for text \"{}\"",
                timeout_ms, text
            ));
        }

        tokio::time::sleep(poll_interval).await;
    }
}

// ══════════════════════════════════════════════════════════════════
// Dialog, Resize, Scroll
// ══════════════════════════════════════════════════════════════════

async fn action_handle_dialog(args: &Value) -> Result<String> {
    require_browser().await?;
    let accept = get_bool(args, "accept")
        .ok_or_else(|| anyhow::anyhow!("Missing 'accept' parameter (true to accept, false to dismiss)"))?;
    let dialog_text = get_str(args, "dialog_text");

    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    use chromiumoxide::cdp::browser_protocol::page::HandleJavaScriptDialogParams;

    let mut params = HandleJavaScriptDialogParams::new(accept);
    if let Some(text) = dialog_text {
        params.prompt_text = Some(text.to_string());
    }

    page.execute(params).await
        .map_err(|e| anyhow::anyhow!("Handle dialog failed: {}. Is there a dialog open?", e))?;

    Ok(format!(
        "Dialog {}.{}",
        if accept { "accepted" } else { "dismissed" },
        dialog_text.map(|t| format!(" Prompt text: \"{}\"", t)).unwrap_or_default()
    ))
}

async fn action_resize(args: &Value) -> Result<String> {
    require_browser().await?;
    let width = get_u32(args, "width")
        .ok_or_else(|| anyhow::anyhow!("Missing 'width' parameter"))? as i64;
    let height = get_u32(args, "height")
        .ok_or_else(|| anyhow::anyhow!("Missing 'height' parameter"))? as i64;

    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;

    let params = SetDeviceMetricsOverrideParams::new(width, height, 1.0, false);
    page.execute(params).await
        .map_err(|e| anyhow::anyhow!("Resize failed: {}", e))?;

    Ok(format!("Viewport resized to {}x{}", width, height))
}

async fn action_scroll(args: &Value) -> Result<String> {
    require_browser().await?;
    let direction = get_str(args, "direction").unwrap_or("down");
    let amount = get_i64(args, "amount").unwrap_or(500);

    let state = get_browser_state().lock().await;
    let page = state.get_active_page()?;

    let (dx, dy) = match direction {
        "up" => (0, -amount),
        "down" => (0, amount),
        "left" => (-amount, 0),
        "right" => (amount, 0),
        _ => return Err(anyhow::anyhow!("Invalid direction: '{}'. Use: up, down, left, right", direction)),
    };

    let js = format!("window.scrollBy({}, {})", dx, dy);
    page.evaluate(js).await
        .map_err(|e| anyhow::anyhow!("Scroll failed: {}", e))?;

    Ok(format!("Scrolled {} by {} pixels", direction, amount.abs()))
}
