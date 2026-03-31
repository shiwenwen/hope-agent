use anyhow::Result;
use std::path::Path;

/// Build the complete index.html file for a canvas project.
/// Wraps user HTML/CSS/JS in a safe template with live-reload support.
pub fn build_html_page(html: Option<&str>, css: Option<&str>, js: Option<&str>) -> String {
    let user_html = html.unwrap_or("<p>Empty canvas</p>");
    let user_css = css.unwrap_or("");
    let user_js = js.unwrap_or("");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
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
    import('https://html2canvas.hertzen.com/dist/html2canvas.min.js').then(function() {{
      html2canvas(document.body).then(function(canvas) {{
        var dataUrl = canvas.toDataURL('image/png');
        parent.postMessage({{ type: 'canvas_snapshot_result', requestId: event.data.requestId, dataUrl: dataUrl }}, '*');
      }});
    }}).catch(function() {{
      // Fallback: use a simple serialized HTML representation
      parent.postMessage({{ type: 'canvas_snapshot_result', requestId: event.data.requestId, error: 'html2canvas not available' }}, '*');
    }});
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
    )
}

/// Build a Markdown preview page.
pub fn build_markdown_page(content: &str) -> String {
    let escaped = content.replace('`', "\\`").replace("${", "\\${");
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<script src="https://cdn.jsdelivr.net/npm/marked/marked.min.js"></script>
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
<div id="content"></div>
<script>
document.getElementById('content').innerHTML = marked.parse(`{escaped}`);
</script>
</body>
</html>"#,
        escaped = escaped,
    )
}

/// Build a code preview page with syntax highlighting.
pub fn build_code_page(content: &str, language: Option<&str>) -> String {
    let lang = language.unwrap_or("plaintext");
    let escaped_content = content
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/highlight.js@11/styles/github.min.css">
<script src="https://cdn.jsdelivr.net/npm/highlight.js@11/highlight.min.js"></script>
<style>
*, *::before, *::after {{ box-sizing: border-box; }}
body {{ margin: 0; padding: 0; }}
pre {{ margin: 0; padding: 16px; font-size: 14px; line-height: 1.5; }}
code {{ font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace; }}
</style>
</head>
<body>
<pre><code class="language-{lang}">{escaped_content}</code></pre>
<script>hljs.highlightAll();</script>
</body>
</html>"#,
        lang = lang,
        escaped_content = escaped_content,
    )
}

/// Build an SVG preview page.
pub fn build_svg_page(content: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
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
    )
}

/// Build a Mermaid diagram page.
pub fn build_mermaid_page(content: &str) -> String {
    let escaped = content.replace('`', "\\`").replace("${", "\\${");
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<script src="https://cdn.jsdelivr.net/npm/mermaid/dist/mermaid.min.js"></script>
<style>
body {{ margin: 0; padding: 24px; display: flex; justify-content: center; background: #fff; }}
.mermaid {{ max-width: 100%; }}
</style>
</head>
<body>
<div class="mermaid">
{escaped}
</div>
<script>mermaid.initialize({{ startOnLoad: true, theme: 'default' }});</script>
</body>
</html>"#,
        escaped = escaped,
    )
}

/// Build a Chart.js visualization page.
pub fn build_chart_page(content: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<script src="https://cdn.jsdelivr.net/npm/chart.js"></script>
<style>
body {{ margin: 0; padding: 24px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; }}
.chart-container {{ position: relative; width: 100%; max-width: 800px; margin: 0 auto; }}
</style>
</head>
<body>
<div class="chart-container">
  <canvas id="chart"></canvas>
</div>
<script>
try {{
  var config = {content};
  new Chart(document.getElementById('chart'), config);
}} catch(e) {{
  document.body.innerHTML = '<pre style="color:red">Chart config error: ' + e.message + '</pre>';
}}
</script>
</body>
</html>"#,
        content = content,
    )
}

/// Build a slides/presentation page.
pub fn build_slides_page(html: Option<&str>, css: Option<&str>) -> String {
    let user_html = html.unwrap_or("<section><h1>Empty Presentation</h1></section>");
    let user_css = css.unwrap_or("");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
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
    )
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

    let index_html = match content_type {
        "markdown" => build_markdown_page(content.unwrap_or("")),
        "code" => build_code_page(content.unwrap_or(""), language),
        "svg" => build_svg_page(content.unwrap_or("")),
        "mermaid" => build_mermaid_page(content.unwrap_or("")),
        "chart" => build_chart_page(content.unwrap_or("{}")),
        "slides" => build_slides_page(html, css),
        _ => build_html_page(html, css, js), // "html" and default
    };

    std::fs::write(project_dir.join("index.html"), &index_html)?;

    // Also save raw source files for reference / version tracking
    if let Some(css_content) = css {
        std::fs::write(project_dir.join("style.css"), css_content)?;
    }
    if let Some(js_content) = js {
        std::fs::write(project_dir.join("script.js"), js_content)?;
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
        std::fs::write(project_dir.join(format!("content.{}", ext)), text_content)?;
    }

    Ok(())
}
