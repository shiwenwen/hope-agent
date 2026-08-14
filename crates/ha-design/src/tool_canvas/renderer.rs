use anyhow::Result;
use pulldown_cmark::{html as markdown_html, Event, Options, Parser};
use serde_json::Value;
use std::path::Path;

const OFFLINE_CSP_INTERACTIVE: &str = "default-src 'none'; img-src data: blob:; style-src 'unsafe-inline'; script-src 'unsafe-inline' 'unsafe-eval'; font-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'";
const LEGACY_OFFLINE_CSP_STATIC_V0: &str = "default-src 'none'; img-src data: blob:; style-src 'unsafe-inline'; script-src 'none'; font-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'";
const ARTIFACT_OFFLINE_CSP_STATIC_V0: &str = "default-src 'none'; img-src data: blob:; style-src 'unsafe-inline'; font-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'";
pub(crate) const OFFLINE_CSP_STATIC: &str = "default-src 'none'; img-src data: blob:; style-src 'unsafe-inline'; script-src 'sha256-JqLDS1iiS31wIW1ENPDQITO83oAAbogGDpIEvGz+wo8='; font-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'";

/// Privileged, app-authored bridge shared by Canvas and Artifact reading
/// surfaces. Static previews authorize exactly these bytes through a CSP hash;
/// they do not gain arbitrary inline-script execution.
pub(crate) const ARTIFACT_SELECTION_BRIDGE_SCRIPT: &str = r#"(function(){
'use strict';
var VERSION=1;
var MAX_TEXT=20000;
var token=null;
var pointerDown=false;
var timer=0;
var suppressSelectionUntil=0;
function send(type,payload){
  if(!token||parent===window)return;
  var message={type:type,version:VERSION,token:token};
  if(payload){for(var key in payload){if(Object.prototype.hasOwnProperty.call(payload,key)){message[key]=payload[key];}}}
  try{parent.postMessage(message,'*');}catch(_){}
}
function clearSelectionMenu(){send('hope_artifact_text_selection_clear');}
function readSelection(){
  var active=document.activeElement;
  if(active){
    var tag=(active.tagName||'').toLowerCase();
    var inputType=(active.type||'text').toLowerCase();
    if((tag==='textarea'||(tag==='input'&&inputType!=='password'))&&typeof active.selectionStart==='number'&&typeof active.selectionEnd==='number'&&active.selectionStart!==active.selectionEnd){
      var inputText=String(active.value||'').slice(active.selectionStart,active.selectionEnd);
      if(inputText.trim())return {text:inputText,rect:active.getBoundingClientRect()};
    }
  }
  var selection=window.getSelection();
  var text=selection?String(selection):'';
  if(!selection||selection.isCollapsed||!text.trim()||selection.rangeCount<1)return null;
  return {text:text,rect:selection.getRangeAt(0).getBoundingClientRect()};
}
function reportSelection(){
  if(pointerDown||!token||Date.now()<suppressSelectionUntil)return;
  try{
    var picked=readSelection();
    if(!picked){clearSelectionMenu();return;}
    var text=picked.text;
    var rect=picked.rect;
    if(!rect||![rect.left,rect.top,rect.right,rect.bottom].every(Number.isFinite)){clearSelectionMenu();return;}
    send('hope_artifact_text_selection',{
      text:text.slice(0,MAX_TEXT),
      truncated:text.length>MAX_TEXT,
      rect:{left:rect.left,top:rect.top,right:rect.right,bottom:rect.bottom}
    });
  }catch(_){clearSelectionMenu();}
}
function scheduleReport(){clearTimeout(timer);timer=setTimeout(reportSelection,80);}
function finishPointerSelection(){
  if(!pointerDown)return;
  pointerDown=false;
  scheduleReport();
}
window.addEventListener('message',function(event){
  var data=event.data;
  if(event.source!==parent||!data||data.type!=='hope_artifact_selection_activate'||data.version!==VERSION||typeof data.token!=='string'||data.token.length<1||data.token.length>256)return;
  token=data.token;
});
document.addEventListener('pointerdown',function(event){
  if(event.button===2){pointerDown=false;suppressSelectionUntil=Date.now()+400;clearSelectionMenu();return;}
  pointerDown=true;clearSelectionMenu();
},true);
document.addEventListener('pointerup',function(event){
  if(event.button===2){suppressSelectionUntil=Date.now()+400;clearSelectionMenu();return;}
  if(event.button===0)finishPointerSelection();else pointerDown=false;
},true);
document.addEventListener('pointercancel',function(){pointerDown=false;clearSelectionMenu();},true);
document.addEventListener('pointerout',function(event){
  if(pointerDown&&event.pointerType==='mouse'&&!event.relatedTarget)finishPointerSelection();
},true);
document.addEventListener('touchend',finishPointerSelection,true);
document.addEventListener('touchcancel',function(){pointerDown=false;clearSelectionMenu();},true);
window.addEventListener('blur',function(){pointerDown=false;clearSelectionMenu();});
document.addEventListener('selectionchange',function(){if(!pointerDown)scheduleReport();},true);
document.addEventListener('select',scheduleReport,true);
document.addEventListener('scroll',clearSelectionMenu,true);
document.addEventListener('keyup',function(event){if(event.key==='Escape')clearSelectionMenu();else scheduleReport();},true);
})();"#;

pub(crate) fn artifact_selection_bridge_tag() -> String {
    format!(
        "<script data-hope-artifact-selection-bridge=\"1\">{ARTIFACT_SELECTION_BRIDGE_SCRIPT}</script>"
    )
}

/// True only when the browser-enforced CSP permits the exact app-authored
/// selection bridge and no author-controlled script. A channel token cannot
/// authenticate code running in the same iframe, so host selection actions
/// must stay disabled for every other projection.
pub(crate) fn selection_bridge_is_script_isolated(html: &str) -> bool {
    if !html.contains("data-hope-artifact-selection-bridge=\"1\"") {
        return false;
    }
    let exact_csp_tag =
        format!("<meta http-equiv=\"Content-Security-Policy\" content=\"{OFFLINE_CSP_STATIC}\">");
    let Some(csp_start) = html.find(&exact_csp_tag) else {
        return false;
    };
    let lower = html.to_ascii_lowercase();
    let Some(head_start) = lower.find("<head") else {
        return false;
    };
    let Some(head_end) = lower[head_start..]
        .find("</head>")
        .map(|at| head_start + at)
    else {
        return false;
    };
    let first_csp = lower.find("http-equiv=\"content-security-policy\"");
    let first_script = lower.find("<script");
    head_start < csp_start
        && csp_start < head_end
        && first_csp == Some(csp_start + "<meta ".len())
        && first_script.is_none_or(|script_start| csp_start < script_start)
}

/// Add the bridge to a complete HTML projection without changing its content
/// model. The operation is idempotent because managed Artifact restore paths
/// may hand us an already-rendered historical projection.
pub(crate) fn inject_artifact_selection_bridge(html: &str) -> String {
    if html.contains("data-hope-artifact-selection-bridge=\"1\"") {
        return html.to_string();
    }
    let bridge = artifact_selection_bridge_tag();
    let lower = html.to_ascii_lowercase();
    let insertion_at = lower
        .rfind("</body>")
        .or_else(|| lower.rfind("</html>"))
        .unwrap_or(html.len());
    let mut output = String::with_capacity(html.len() + bridge.len());
    output.push_str(&html[..insertion_at]);
    output.push_str(&bridge);
    output.push_str(&html[insertion_at..]);
    output
}

/// Self-heal stored projections produced before the selection bridge existed.
/// Only Hope's two exact historical static policies are upgraded; an imported
/// document's custom CSP remains authoritative.
pub(crate) fn upgrade_artifact_selection_bridge(html: &str) -> String {
    let replace_managed_csp = |document: String, previous: &str| {
        document.replace(
            &format!("content=\"{previous}\""),
            &format!("content=\"{OFFLINE_CSP_STATIC}\""),
        )
    };
    let with_current_csp = replace_managed_csp(
        replace_managed_csp(html.to_string(), LEGACY_OFFLINE_CSP_STATIC_V0),
        ARTIFACT_OFFLINE_CSP_STATIC_V0,
    );
    inject_artifact_selection_bridge(&with_current_csp)
}

/// Build the complete index.html file for a canvas project.
/// Wraps user HTML/CSS/JS in a safe template with live-reload support.
pub fn build_html_page(html: Option<&str>, css: Option<&str>, js: Option<&str>) -> String {
    let user_html = html.unwrap_or("<p>Empty canvas</p>");
    let user_css = css.unwrap_or("");
    let user_js = js.unwrap_or("");

    inject_artifact_selection_bridge(&format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<meta http-equiv="Content-Security-Policy" content="{csp}">
<style>
*, *::before, *::after {{ box-sizing: border-box; }}
body {{ margin: 0; padding: 16px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; }}
{user_css}
</style>
</head>
<body>
{user_html}
<script>
// Canvas messaging bridge
window.addEventListener('message', function(event) {{
  if (event.data && event.data.type === 'canvas_eval') {{
    try {{
      var result = eval(event.data.code);
      parent.postMessage({{ type: 'canvas_eval_result', requestId: event.data.requestId, result: String(result) }}, '*');
    }} catch(e) {{
      parent.postMessage({{ type: 'canvas_eval_result', requestId: event.data.requestId, error: e.message }}, '*');
    }}
  }}
  if (event.data && event.data.type === 'canvas_snapshot') {{
    parent.postMessage({{ type: 'canvas_snapshot_result', requestId: event.data.requestId, error: 'Offline snapshot runtime unavailable; use the app-owned browser capture path.' }}, '*');
  }}
}});
</script>
<script>
{user_js}
</script>
</body>
</html>"#,
        user_css = user_css,
        user_html = user_html,
        user_js = user_js,
        csp = OFFLINE_CSP_INTERACTIVE,
    ))
}

/// Build a Markdown preview page.
pub fn build_markdown_page(content: &str) -> String {
    let parser = Parser::new_ext(
        content,
        Options::ENABLE_TABLES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_FOOTNOTES,
    );
    let parser = parser.map(|event| match event {
        Event::Html(value) | Event::InlineHtml(value) => Event::Text(value),
        other => other,
    });
    let mut rendered = String::new();
    markdown_html::push_html(&mut rendered, parser);
    inject_artifact_selection_bridge(&format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<meta http-equiv="Content-Security-Policy" content="{csp}">
<style>
*, *::before, *::after {{ box-sizing: border-box; }}
body {{ margin: 0; padding: 24px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; line-height: 1.6; color: #1a1a1a; max-width: 800px; }}
pre {{ background: #f5f5f5; padding: 12px; border-radius: 6px; overflow-x: auto; }}
code {{ background: #f5f5f5; padding: 2px 6px; border-radius: 3px; font-size: 0.9em; }}
pre code {{ background: none; padding: 0; }}
img {{ max-width: 100%; }}
table {{ border-collapse: collapse; width: 100%; }}
th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}
th {{ background: #f5f5f5; }}
blockquote {{ border-left: 4px solid #ddd; margin-left: 0; padding-left: 16px; color: #555; }}
</style>
</head>
<body>
<main>{rendered}</main>
</body>
</html>"#,
        csp = OFFLINE_CSP_STATIC,
        rendered = rendered,
    ))
}

/// Build a code preview page with syntax highlighting.
pub fn build_code_page(content: &str, language: Option<&str>) -> String {
    let lang = language.unwrap_or("plaintext");
    let escaped_content = content
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    inject_artifact_selection_bridge(&format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<meta http-equiv="Content-Security-Policy" content="{csp}">
<style>
*, *::before, *::after {{ box-sizing: border-box; }}
body {{ margin: 0; padding: 0; }}
pre {{ margin: 0; padding: 16px; font-size: 14px; line-height: 1.5; }}
code {{ font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace; }}
</style>
</head>
<body>
<pre><code class="language-{lang}">{escaped_content}</code></pre>
</body>
</html>"#,
        csp = OFFLINE_CSP_STATIC,
        lang = lang,
        escaped_content = escaped_content,
    ))
}

/// Build an SVG preview page.
pub fn build_svg_page(content: &str) -> String {
    inject_artifact_selection_bridge(&format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<meta http-equiv="Content-Security-Policy" content="{csp}">
<style>
body {{ margin: 0; padding: 16px; display: flex; justify-content: center; align-items: center; min-height: 100vh; background: #fafafa; }}
svg {{ max-width: 100%; height: auto; }}
</style>
</head>
<body>
{content}
</body>
</html>"#,
        content = content,
        csp = OFFLINE_CSP_STATIC,
    ))
}

/// Build a Mermaid diagram page.
pub fn build_mermaid_page(content: &str) -> String {
    let escaped = escape_html(content);
    inject_artifact_selection_bridge(&format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<meta http-equiv="Content-Security-Policy" content="{csp}">
<style>
body {{ margin: 0; padding: 24px; background: #fff; color: #1a1a1a; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; }}
.notice {{ margin-bottom: 12px; color: #666; font-size: 13px; }}
pre {{ max-width: 100%; overflow: auto; border-radius: 8px; background: #f5f5f5; padding: 16px; line-height: 1.5; }}
</style>
</head>
<body>
<p class="notice">Mermaid source (offline semantic fallback)</p>
<pre><code>{escaped}</code></pre>
</body>
</html>"#,
        escaped = escaped,
        csp = OFFLINE_CSP_STATIC,
    ))
}

/// Build a Chart.js visualization page.
pub fn build_chart_page(content: &str) -> String {
    let config = serde_json::from_str::<Value>(content).unwrap_or(Value::Null);
    let title = config
        .pointer("/options/plugins/title/text")
        .and_then(Value::as_str)
        .unwrap_or("Chart");
    let labels = config
        .pointer("/data/labels")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let datasets = config
        .pointer("/data/datasets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut header = String::from("<th>Category</th>");
    for dataset in &datasets {
        header.push_str(&format!(
            "<th>{}</th>",
            escape_html(
                dataset
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("Value")
            )
        ));
    }
    let mut rows = String::new();
    for (index, label) in labels.iter().enumerate() {
        rows.push_str(&format!(
            "<tr><th>{}</th>",
            escape_html(&display_value(label))
        ));
        for dataset in &datasets {
            let value = dataset
                .get("data")
                .and_then(Value::as_array)
                .and_then(|values| values.get(index))
                .map(display_value)
                .unwrap_or_else(|| "—".to_string());
            rows.push_str(&format!("<td>{}</td>", escape_html(&value)));
        }
        rows.push_str("</tr>");
    }
    if rows.is_empty() {
        rows.push_str(&format!(
            "<tr><td><pre>{}</pre></td></tr>",
            escape_html(content)
        ));
    }
    inject_artifact_selection_bridge(&format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<meta http-equiv="Content-Security-Policy" content="{csp}">
<style>
body {{ margin: 0; padding: 24px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; }}
main {{ width: 100%; max-width: 900px; margin: 0 auto; }}
table {{ border-collapse: collapse; width: 100%; }} th,td {{ border: 1px solid #ddd; padding: 8px; text-align: right; }} th:first-child {{ text-align: left; }}
</style>
</head>
<body>
<main><h1>{title}</h1><p>Offline semantic chart fallback</p><table><thead><tr>{header}</tr></thead><tbody>{rows}</tbody></table></main>
</body>
</html>"#,
        csp = OFFLINE_CSP_STATIC,
        title = escape_html(title),
        header = header,
        rows = rows,
    ))
}

/// Build a slides/presentation page.
pub fn build_slides_page(html: Option<&str>, css: Option<&str>) -> String {
    let user_html = html.unwrap_or("<section><h1>Empty Presentation</h1></section>");
    let user_css = css.unwrap_or("");

    inject_artifact_selection_bridge(&format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<meta http-equiv="Content-Security-Policy" content="{csp}">
<style>
*, *::before, *::after {{ box-sizing: border-box; }}
body {{ margin: 0; padding: 0; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; overflow: hidden; background: #1a1a2e; color: #eee; }}
.slides {{ width: 100vw; height: 100vh; position: relative; }}
section {{ width: 100vw; height: 100vh; display: none; justify-content: center; align-items: center; flex-direction: column; padding: 48px; text-align: center; }}
section.active {{ display: flex; }}
section h1 {{ font-size: 2.5em; margin-bottom: 0.5em; }}
section h2 {{ font-size: 1.8em; margin-bottom: 0.5em; }}
section p {{ font-size: 1.2em; line-height: 1.6; max-width: 700px; }}
section ul, section ol {{ text-align: left; font-size: 1.1em; line-height: 1.8; }}
.slide-nav {{ position: fixed; bottom: 16px; right: 16px; color: #888; font-size: 14px; z-index: 10; }}
{user_css}
</style>
</head>
<body>
<div class="slides">
{user_html}
</div>
<div class="slide-nav"><span id="current">1</span> / <span id="total">1</span></div>
<script>
(function() {{
  var slides = document.querySelectorAll('.slides section');
  var current = 0;
  var total = slides.length;
  document.getElementById('total').textContent = total;
  function show(idx) {{
    slides.forEach(function(s) {{ s.classList.remove('active'); }});
    if (slides[idx]) slides[idx].classList.add('active');
    document.getElementById('current').textContent = idx + 1;
  }}
  show(0);
  document.addEventListener('keydown', function(e) {{
    if (e.key === 'ArrowRight' || e.key === ' ') {{ current = Math.min(current + 1, total - 1); show(current); }}
    if (e.key === 'ArrowLeft') {{ current = Math.max(current - 1, 0); show(current); }}
  }});
  document.addEventListener('click', function(e) {{
    if (e.clientX > window.innerWidth / 2) {{ current = Math.min(current + 1, total - 1); }}
    else {{ current = Math.max(current - 1, 0); }}
    show(current);
  }});
}})();
</script>
</body>
</html>"#,
        user_css = user_css,
        user_html = user_html,
        csp = OFFLINE_CSP_INTERACTIVE,
    ))
}

/// Write canvas project files to disk based on content type.
pub fn write_project_files(
    project_dir: &Path,
    content_type: &str,
    html: Option<&str>,
    css: Option<&str>,
    js: Option<&str>,
    content: Option<&str>,
    language: Option<&str>,
) -> Result<()> {
    std::fs::create_dir_all(project_dir)?;

    let index_html = render_project_page(content_type, html, css, js, content, language);

    ha_core::platform::write_atomic(&project_dir.join("index.html"), index_html.as_bytes())?;

    // Also save raw source files for reference / version tracking
    if let Some(css_content) = css {
        ha_core::platform::write_atomic(&project_dir.join("style.css"), css_content.as_bytes())?;
    }
    if let Some(js_content) = js {
        ha_core::platform::write_atomic(&project_dir.join("script.js"), js_content.as_bytes())?;
    }
    if let Some(text_content) = content {
        let ext = match content_type {
            "markdown" => "md",
            "svg" => "svg",
            "chart" => "json",
            "mermaid" => "mmd",
            "code" => language.unwrap_or("txt"),
            _ => "txt",
        };
        ha_core::platform::write_atomic(
            &project_dir.join(format!("content.{}", ext)),
            text_content.as_bytes(),
        )?;
    }

    Ok(())
}

/// Render a complete Canvas page without touching disk. Artifact migration
/// uses this for legacy versions whose HTML/CSS/JS were stored separately.
pub(crate) fn render_project_page(
    content_type: &str,
    html: Option<&str>,
    css: Option<&str>,
    js: Option<&str>,
    content: Option<&str>,
    language: Option<&str>,
) -> String {
    match content_type {
        "markdown" => build_markdown_page(content.unwrap_or("")),
        "code" => build_code_page(content.unwrap_or(""), language),
        "svg" => build_svg_page(content.unwrap_or("")),
        "mermaid" => build_mermaid_page(content.unwrap_or("")),
        "chart" => build_chart_page(content.unwrap_or("{}")),
        "slides" => build_slides_page(html, css),
        _ => build_html_page(html, css, js),
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn display_value(value: &Value) -> String {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};

    use super::*;

    #[test]
    fn static_csp_authorizes_only_the_exact_selection_bridge() {
        let digest = Sha256::digest(ARTIFACT_SELECTION_BRIDGE_SCRIPT.as_bytes());
        let encoded = base64::engine::general_purpose::STANDARD.encode(digest);
        assert!(OFFLINE_CSP_STATIC.contains(&format!("'sha256-{encoded}'")));
        assert!(!OFFLINE_CSP_STATIC.contains("script-src 'unsafe-inline'"));
        assert!(ARTIFACT_SELECTION_BRIDGE_SCRIPT.contains("!event.relatedTarget"));
        assert!(ARTIFACT_SELECTION_BRIDGE_SCRIPT.contains("'touchend',finishPointerSelection"));
        let trusted = build_markdown_page("safe text");
        assert!(selection_bridge_is_script_isolated(&trusted));
        let executable = build_html_page(Some("<p>text</p>"), None, Some("void 0"));
        assert!(!selection_bridge_is_script_isolated(&executable));
        let forged = build_html_page(
            Some(&format!(
                "<meta http-equiv=\"Content-Security-Policy\" content=\"{OFFLINE_CSP_STATIC}\">"
            )),
            None,
            Some("void 0"),
        );
        assert!(!selection_bridge_is_script_isolated(&forged));
    }

    #[test]
    fn every_canvas_projection_contains_one_selection_bridge() {
        for html in [
            build_html_page(Some("<p>HTML</p>"), None, None),
            build_markdown_page("# Markdown"),
            build_code_page("let value = 1;", Some("javascript")),
            build_svg_page("<svg><text>SVG</text></svg>"),
            build_mermaid_page("graph TD; A-->B"),
            build_chart_page(r#"{"data":{"labels":["A"]}}"#),
            build_slides_page(Some("<section>Slide</section>"), None),
        ] {
            assert_eq!(
                html.matches("data-hope-artifact-selection-bridge=\"1\"")
                    .count(),
                1
            );
            assert!(html.contains("hope_artifact_selection_activate"));
        }
    }

    #[test]
    fn selection_bridge_injection_is_idempotent() {
        let once = inject_artifact_selection_bridge("<html><body><p>x</p></body></html>");
        let twice = inject_artifact_selection_bridge(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn upgrades_known_static_projection_without_opening_arbitrary_scripts() {
        let old = format!(
            "<meta http-equiv=\"Content-Security-Policy\" content=\"{LEGACY_OFFLINE_CSP_STATIC_V0}\"><body>x</body>"
        );
        let upgraded = upgrade_artifact_selection_bridge(&old);
        assert!(upgraded.contains(OFFLINE_CSP_STATIC));
        assert!(!upgraded.contains("script-src 'none'"));
        assert!(!upgraded.contains("script-src 'unsafe-inline'"));
        assert!(upgraded.contains("data-hope-artifact-selection-bridge=\"1\""));

        let policy_as_content = format!("<body><pre>{LEGACY_OFFLINE_CSP_STATIC_V0}</pre></body>");
        let content_upgrade = upgrade_artifact_selection_bridge(&policy_as_content);
        assert!(content_upgrade.contains(&format!("<pre>{LEGACY_OFFLINE_CSP_STATIC_V0}</pre>")));
    }
}
