use anyhow::Result;
use serde_json::Value;

use super::{get_bool, get_str, require_browser};
use crate::agent::MEDIA_ITEMS_PREFIX;
use crate::attachments::{self, MediaItem, MediaKind};
use crate::browser_state::{get_browser_state, ElementRef};
use crate::tools::image_markers;

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

pub(super) async fn action_take_snapshot() -> Result<String> {
    require_browser().await?;
    let page = {
        let state = get_browser_state().lock().await;
        state.get_active_page()?.clone()
    };

    let json_str: String = page
        .evaluate(SNAPSHOT_JS)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to take snapshot: {}", e))?
        .into_value()
        .map_err(|e| anyhow::anyhow!("Snapshot returned invalid data: {}", e))?;

    let data: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| anyhow::anyhow!("Failed to parse snapshot: {}", e))?;

    let url = data
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let title = data
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("untitled");
    let viewport_w = data
        .get("viewport")
        .and_then(|v| v.get("w"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let viewport_h = data
        .get("viewport")
        .and_then(|v| v.get("h"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let truncated = data
        .get("truncated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
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
        output.push_str(
            "\n[Truncated: max 300 elements reached. Use 'evaluate' to query specific areas.]\n",
        );
    }

    let mut state = get_browser_state().lock().await;
    state.element_refs = new_refs;
    state.snapshot_url = Some(url.to_string());

    Ok(output)
}

// ══════════════════════════════════════════════════════════════════
// Screenshot
// ══════════════════════════════════════════════════════════════════

pub(super) async fn action_take_screenshot(
    args: &Value,
    session_id: Option<&str>,
) -> Result<String> {
    require_browser().await?;
    let format = get_str(args, "format").unwrap_or("png");
    let full_page = get_bool(args, "full_page").unwrap_or(false);

    let page = {
        let state = get_browser_state().lock().await;
        state.get_active_page()?.clone()
    };

    use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
    use chromiumoxide::page::ScreenshotParams;

    let cdp_format = match format {
        "jpeg" | "jpg" => CaptureScreenshotFormat::Jpeg,
        _ => CaptureScreenshotFormat::Png,
    };

    let params = ScreenshotParams::builder()
        .format(cdp_format)
        .full_page(full_page)
        .build();

    let screenshot_bytes = page
        .screenshot(params)
        .await
        .map_err(|e| anyhow::anyhow!("Screenshot failed: {}", e))?;

    let url = page
        .url()
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "unknown".to_string());

    let mime = if format == "jpeg" || format == "jpg" {
        "image/jpeg"
    } else {
        "image/png"
    };

    let caption = format!(
        "Screenshot captured (url: {}, format: {}{})",
        url,
        format,
        if full_page { ", full page" } else { "" }
    );
    let ext = if mime == "image/jpeg" { "jpg" } else { "png" };
    let display_filename = format!("browser_screenshot.{ext}");

    match attachments::save_attachment_bytes(session_id, &display_filename, &screenshot_bytes) {
        Ok(saved_path) => {
            let item = MediaItem::from_saved_path(
                session_id,
                &saved_path,
                &display_filename,
                mime.to_string(),
                screenshot_bytes.len() as u64,
                MediaKind::Image,
                Some(caption.clone()),
            );
            let items_json =
                serde_json::to_string(&vec![item]).unwrap_or_else(|_| "[]".to_string());
            let file_marker = image_markers::build_image_file_marker(mime, &saved_path, &caption);
            Ok(format!("{MEDIA_ITEMS_PREFIX}{items_json}\n{file_marker}"))
        }
        Err(e) => {
            app_warn!(
                "tool",
                "browser",
                "Failed to save browser screenshot as attachment; falling back to inline base64: {}",
                e
            );
            let b64_data = base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &screenshot_bytes,
            );
            Ok(image_markers::build_image_base64_marker(
                mime, &b64_data, &caption,
            ))
        }
    }
}
