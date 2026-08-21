//! 浏览器特征 crate（阶段 4 首刀，自 ha-core 迁出）：Chrome Extension
//! backend / CDP backend / broker / 观察缓冲 / `browser` 工具 / 状态与 UI
//! 辅助。装配契约与其它特征 crate 相同：每个调 `ha_core::init_runtime`
//! 的二进制必须先调 [`wire()`]。

// `app_*!` 系宏由 ha-base 导出（与 ha-core 同一接法）。
#[macro_use]
extern crate ha_base;

pub mod browser;
pub mod browser_state;
pub mod browser_ui;
mod tool;

/// knowledge 网页收集的抓取脚本（原 knowledge/source.rs，抓取逻辑属浏览器域）。
const BROWSER_CAPTURE_JS: &str = r#"(() => {
  const MAX_TEXT = 220000;
  const BLOCK_TAGS = new Set(['ADDRESS','ARTICLE','ASIDE','BLOCKQUOTE','BR','CAPTION','DIV','DL','FIELDSET','FIGCAPTION','FIGURE','FOOTER','FORM','H1','H2','H3','H4','H5','H6','HEADER','HR','LI','MAIN','NAV','OL','P','PRE','SECTION','TABLE','TD','TH','TR','UL']);
  const DROP_SELECTORS = 'script,style,noscript,template,svg,canvas,iframe,button,input,select,textarea,[hidden],[aria-hidden="true"]';
  function cleanText(value) {
    return String(value || '').replace(/\u00a0/g, ' ').replace(/[ \t\r\f\v]+/g, ' ').replace(/\n{3,}/g, '\n\n').trim();
  }
  function isHidden(el) {
    if (!el || !el.getBoundingClientRect) return false;
    const style = window.getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden') return true;
    const rect = el.getBoundingClientRect();
    return rect.width === 0 && rect.height === 0;
  }
  function appendLine(lines, value) {
    const text = cleanText(value);
    if (!text) return;
    if (lines.join('\n').length + text.length > MAX_TEXT) return;
    lines.push(text);
  }
  function walk(node, lines, depth) {
    if (!node || lines.join('\n').length > MAX_TEXT) return;
    if (node.nodeType === Node.TEXT_NODE) {
      appendLine(lines, node.nodeValue);
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    const el = node;
    if (el.matches && el.matches(DROP_SELECTORS)) return;
    if (isHidden(el)) return;
    const tag = el.tagName;
    if (/^H[1-6]$/.test(tag)) {
      appendLine(lines, `${'#'.repeat(Number(tag.slice(1)))} ${el.innerText || el.textContent || ''}`);
      return;
    }
    if (tag === 'LI') {
      appendLine(lines, `- ${el.innerText || el.textContent || ''}`);
      return;
    }
    if (tag === 'A') {
      const text = cleanText(el.innerText || el.textContent || '');
      const href = el.href || '';
      appendLine(lines, href && text && href !== text ? `[${text}](${href})` : text);
      return;
    }
    if (tag === 'PRE' || tag === 'CODE') {
      appendLine(lines, el.innerText || el.textContent || '');
      return;
    }
    const before = lines.length;
    for (const child of Array.from(el.childNodes)) walk(child, lines, depth + 1);
    if (BLOCK_TAGS.has(tag) && lines.length > before) lines.push('');
  }
  function readableRoot() {
    return document.querySelector('article')
      || document.querySelector('main')
      || document.querySelector('[role="main"]')
      || document.querySelector('.article')
      || document.querySelector('.post')
      || document.body;
  }
  function pageMarkdown() {
    const root = readableRoot();
    const lines = [];
    walk(root, lines, 0);
    const markdown = cleanText(lines.join('\n'));
    return markdown || cleanText(document.body && document.body.innerText);
  }
  const selection = window.getSelection && window.getSelection();
  const selectionText = cleanText(selection ? selection.toString() : '');
  return {
    url: location.href,
    title: document.title || '',
    selectionText,
    pageText: pageMarkdown()
  };
})()"#;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserCapturePayload {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    selection_text: String,
    #[serde(default)]
    page_text: String,
}

/// 装配入口（幂等）：`browser` 分发条目 + kernel 四件套钩子
/// （broker spawn / turn finalize / 会话清理级联 / 活动标签页抓取）。
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        fn browser_handler<'a>(
            args: &'a serde_json::Value,
            ctx: &'a ha_core::tools::ToolExecContext,
        ) -> ha_core::tools::registry::BuiltinToolFuture<'a> {
            Box::pin(tool::tool_browser(args, ctx))
        }
        ha_core::tools::registry::register_external_tools(vec![
            ha_core::tools::registry::BuiltinToolEntry {
                name: ha_core::tools::TOOL_BROWSER,
                aliases: &[],
                handler: browser_handler,
            },
        ])
        .expect(
            "ha_browser::wire() must run before ha_core::init_runtime freezes the tool registry",
        );

        fn spawn_broker() {
            browser::BrowserExtensionBroker::spawn_global();
        }
        fn schedule_turn_finalize(session_id: &str) {
            browser::schedule_extension_turn_finalize(session_id);
        }
        fn cleanup_session(
            session_id: &str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>> {
            let sid = session_id.to_string();
            Box::pin(async move { browser::cleanup_extension_session(&sid).await })
        }
        fn capture_active_tab() -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = anyhow::Result<ha_core::browser_hooks::BrowserTabCapture>,
                    > + Send,
            >,
        > {
            Box::pin(async move {
                let backend = browser::acquire_backend_for(
                    browser::BrowserBackendContext::default(),
                    browser::BrowserBackendRequirement::ExtensionPreferred,
                )
                .await?;
                let active = backend
                    .active_tab_info()
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("no active browser tab to capture"))?;
                if !active.target_id.trim().is_empty() {
                    backend.select_page(&active.target_id).await?;
                }
                let raw = backend.evaluate(BROWSER_CAPTURE_JS).await?;
                let capture: BrowserCapturePayload = serde_json::from_value(raw).map_err(|e| {
                    anyhow::anyhow!("browser capture returned invalid payload: {e}")
                })?;
                // url / title 的 active-tab 兜底在此收敛（kernel 只拿最终值）。
                let url = if capture.url.trim().is_empty() {
                    active.url.clone()
                } else {
                    capture.url
                };
                let title = if capture.title.trim().is_empty() {
                    active.title.clone()
                } else {
                    capture.title
                };
                Ok(ha_core::browser_hooks::BrowserTabCapture {
                    url,
                    title,
                    selection_text: capture.selection_text,
                    page_text: capture.page_text,
                })
            })
        }
        fn render_web_fetch(
            request: ha_core::browser_hooks::WebFetchRenderRequest,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = anyhow::Result<ha_core::browser_hooks::WebFetchRenderResult>,
                    > + Send,
            >,
        > {
            Box::pin(browser::web_fetch_renderer::render(request))
        }
        ha_core::browser_hooks::register_browser_hooks(ha_core::browser_hooks::BrowserHooks {
            spawn_broker,
            schedule_turn_finalize,
            cleanup_session,
            capture_active_tab,
            render_web_fetch,
        })
        .expect("ha_browser::wire() registers the browser hooks exactly once");
    });
}
