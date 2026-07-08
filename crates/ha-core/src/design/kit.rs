//! 设计系统「套件视图」（Kit）——把一个设计系统的 tokens 渲染成一张**自包含 HTML**
//! 套件页（色板 / 字阶 / 间距 / 圆角+阴影 / 组件 showcase），让抽象 token 表「看得见」。
//!
//! **架构对齐（B1-1）**：与产物一样走「后端生成自包含 HTML → 沙箱 iframe」路线，浏览器
//! 零编译、零网络。token 注入复用 `renderer::tokens_root_css`（同一安全过滤：仅 `--ds-*`、
//! 值滤 `}{<;`）。组件全部引用 `var(--ds-*)`，故套件即系统的真实视觉。
//!
//! **light/dark = 表面切换**（诚实分歧，见 design-space.md 决策账本）：我方每个系统是**单
//! token 集**、无暗色变体，故 dark 切换只覆盖 `--ds-color-bg/fg/muted/border` 为暗色让组件
//! 在暗底可见，**不是**暗色 token 重映射。

use std::collections::BTreeMap;

use super::renderer::{html_escape, tokens_root_css};

/// 取某前缀下的 token（键、去前缀短名、值），按 BTreeMap 顺序（已排序）。
fn group<'a>(
    tokens: &'a BTreeMap<String, String>,
    prefix: &str,
) -> Vec<(&'a str, String, &'a str)> {
    tokens
        .iter()
        .filter(|(k, _)| k.starts_with(prefix))
        .map(|(k, v)| (k.as_str(), k[prefix.len()..].replace('-', " "), v.as_str()))
        .collect()
}

/// 生成设计系统套件页（自包含 HTML，进沙箱 iframe）。`name` 作标题，`tokens` 为系统展开
/// 后的 `--ds-*` 变量。空 tokens 也能出页（用骨架默认值 + 组件 showcase）。`logos`/`images`
/// 是 harvest 的 data-uri 资产（B1-4），空则不出对应段。
pub fn build_kit_html(
    name: &str,
    tokens: &BTreeMap<String, String>,
    logos: &[String],
    images: &[String],
) -> String {
    let token_vec: Vec<(String, String)> =
        tokens.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let root = tokens_root_css(&token_vec);
    let esc_name = html_escape(name);

    let colors = group(tokens, "--ds-color-");
    let texts = group(tokens, "--ds-text-");
    let spaces = group(tokens, "--ds-space-");
    let radii = group(tokens, "--ds-radius-");
    let shadows = group(tokens, "--ds-shadow-");

    // ── 色板 ──
    let color_swatches: String = colors
        .iter()
        .map(|(key, short, val)| {
            format!(
                "<figure class=\"sw\"><div class=\"chip\" style=\"background:var({key})\"></div>\
<figcaption><b>{short}</b><code>{val}</code></figcaption></figure>",
                key = key,
                short = html_escape(short),
                val = html_escape(val),
            )
        })
        .collect();

    // ── 字体族 specimen ──
    let font_specimen = |var: &str, label: &str| -> String {
        if tokens.contains_key(var) {
            format!(
                "<div class=\"spec\" style=\"font-family:var({var})\">\
<span class=\"spec-l\">{label}</span>\
<p class=\"spec-t\">The quick brown fox · 设计系统字体样张 · 0123456789</p></div>",
                var = var,
                label = html_escape(label),
            )
        } else {
            String::new()
        }
    };
    let fonts = format!(
        "{}{}{}",
        font_specimen("--ds-font-sans", "Sans"),
        font_specimen("--ds-font-serif", "Serif"),
        font_specimen("--ds-font-mono", "Mono"),
    );

    // ── 字号阶 ──
    let text_scale: String = texts
        .iter()
        .map(|(key, short, val)| {
            format!(
                "<div class=\"row\"><span class=\"row-l\">{short} · <code>{val}</code></span>\
<span style=\"font-size:var({key})\">Aa 样张</span></div>",
                key = key,
                short = html_escape(short),
                val = html_escape(val),
            )
        })
        .collect();

    // ── 间距条 ──
    let space_bars: String = spaces
        .iter()
        .map(|(key, short, val)| {
            format!(
                "<div class=\"row\"><span class=\"row-l\">{short} · <code>{val}</code></span>\
<span class=\"bar\" style=\"width:var({key})\"></span></div>",
                key = key,
                short = html_escape(short),
                val = html_escape(val),
            )
        })
        .collect();

    // ── 圆角 + 阴影 ──
    let radius_boxes: String = radii
        .iter()
        .map(|(key, short, val)| {
            format!(
                "<figure class=\"sw\"><div class=\"rbox\" style=\"border-radius:var({key})\"></div>\
<figcaption><b>{short}</b><code>{val}</code></figcaption></figure>",
                key = key,
                short = html_escape(short),
                val = html_escape(val),
            )
        })
        .collect();
    let shadow_boxes: String = shadows
        .iter()
        .map(|(key, short, val)| {
            format!(
                "<figure class=\"sw\"><div class=\"rbox\" style=\"box-shadow:var({key})\"></div>\
<figcaption><b>{short}</b><code>{val}</code></figcaption></figure>",
                key = key,
                short = html_escape(short),
                val = html_escape(val),
            )
        })
        .collect();

    // ── logo / 配图（B1-4，仅放行 data:image/ 自包含资产）──
    let logo_html: String = logos
        .iter()
        .filter(|u| u.starts_with("data:image/"))
        .map(|u| {
            format!(
                "<div class=\"logo-tile\"><img src=\"{}\" alt=\"logo\" loading=\"lazy\"></div>",
                html_escape(u)
            )
        })
        .collect();
    let image_html: String = images
        .iter()
        .filter(|u| u.starts_with("data:image/"))
        .map(|u| {
            format!(
                "<figure class=\"img-tile\"><img src=\"{}\" alt=\"\" loading=\"lazy\"></figure>",
                html_escape(u)
            )
        })
        .collect();

    let section = |title: &str, inner: &str, cls: &str| -> String {
        if inner.trim().is_empty() {
            String::new()
        } else {
            format!(
                "<section><h2>{title}</h2><div class=\"{cls}\">{inner}</div></section>",
                title = html_escape(title),
                cls = cls,
                inner = inner,
            )
        }
    };

    format!(
        r##"<!doctype html><html lang="zh"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>{name} · 套件</title>
<style>
{root}
:root{{color-scheme:light}}
*{{box-sizing:border-box}}
body{{margin:0;font-family:var(--ds-font-sans,system-ui,-apple-system,"Segoe UI","PingFang SC",sans-serif);
background:var(--ds-color-bg,#fff);color:var(--ds-color-fg,#111827);line-height:1.55;padding:0 0 4rem}}
body.dark{{--ds-color-bg:#0b1020;--ds-color-fg:#e5e7eb;--ds-color-muted:#1e293b;--ds-color-border:#334155}}
header{{position:sticky;top:0;z-index:5;display:flex;align-items:center;gap:1rem;padding:1rem 1.5rem;
border-bottom:1px solid var(--ds-color-border,#e5e7eb);background:var(--ds-color-bg,#fff)}}
header h1{{font-size:1.15rem;margin:0;font-weight:650;letter-spacing:-.01em}}
.toggle{{margin-left:auto;border:1px solid var(--ds-color-border,#e5e7eb);background:transparent;color:inherit;
border-radius:var(--ds-radius-md,8px);padding:.35rem .7rem;font-size:.8rem;cursor:pointer}}
main{{max-width:64rem;margin:0 auto;padding:1.5rem}}
section{{padding:1.25rem 0;border-bottom:1px solid var(--ds-color-border,#eef0f3)}}
section:last-child{{border-bottom:0}}
h2{{font-size:.78rem;letter-spacing:.12em;text-transform:uppercase;color:var(--ds-color-secondary,#6b7280);margin:0 0 1rem;font-weight:600}}
.swatches{{display:grid;grid-template-columns:repeat(auto-fill,minmax(120px,1fr));gap:.9rem}}
.sw{{margin:0}}
.chip{{height:56px;border-radius:var(--ds-radius-md,8px);border:1px solid rgba(0,0,0,.06)}}
.rbox{{height:56px;background:var(--ds-color-muted,#f1f5f9);border:1px solid var(--ds-color-border,#e5e7eb)}}
figcaption{{margin-top:.4rem;font-size:.72rem;display:flex;flex-direction:column;gap:.1rem}}
figcaption b{{font-weight:600;text-transform:capitalize}}
code{{font-family:var(--ds-font-mono,ui-monospace,Menlo,monospace);font-size:.68rem;color:var(--ds-color-secondary,#6b7280)}}
.type-scale,.space-list{{display:flex;flex-direction:column;gap:.6rem}}
.spec{{padding:.75rem 0;border-bottom:1px dashed var(--ds-color-border,#eef0f3)}}
.spec-l{{font-size:.7rem;letter-spacing:.08em;text-transform:uppercase;color:var(--ds-color-secondary,#9ca3af)}}
.spec-t{{margin:.35rem 0 0;font-size:1.35rem}}
.row{{display:flex;align-items:center;gap:1rem}}
.row-l{{min-width:180px;font-size:.78rem}}
.bar{{height:14px;border-radius:4px;background:var(--ds-color-primary,#2563eb);display:inline-block}}
.components{{display:flex;flex-wrap:wrap;gap:1.25rem;align-items:flex-start}}
.demo-card{{border:1px solid var(--ds-color-border,#e5e7eb);border-radius:var(--ds-radius-lg,14px);
background:var(--ds-color-bg,#fff);box-shadow:var(--ds-shadow-md,0 4px 20px rgba(0,0,0,.06));padding:1.1rem;max-width:280px}}
.btn{{border:0;border-radius:var(--ds-radius-md,8px);padding:.5rem .95rem;font-size:.85rem;font-weight:550;cursor:pointer;font-family:inherit}}
.btn-primary{{background:var(--ds-color-primary,#2563eb);color:#fff}}
.btn-secondary{{background:var(--ds-color-muted,#f1f5f9);color:var(--ds-color-fg,#111827)}}
.btn-accent{{background:var(--ds-color-accent,#0ea5e9);color:#fff}}
.btn-outline{{background:transparent;color:var(--ds-color-primary,#2563eb);border:1px solid var(--ds-color-primary,#2563eb)}}
.field{{width:100%;border:1px solid var(--ds-color-border,#e5e7eb);border-radius:var(--ds-radius-md,8px);
padding:.5rem .7rem;font-size:.85rem;background:var(--ds-color-bg,#fff);color:inherit;font-family:inherit}}
.badges{{display:flex;gap:.4rem;flex-wrap:wrap}}
.badge{{font-size:.7rem;padding:.15rem .5rem;border-radius:999px;font-weight:600}}
.b-primary{{background:var(--ds-color-primary,#2563eb);color:#fff}}
.b-success{{background:var(--ds-color-success,#16a34a);color:#fff}}
.b-warning{{background:var(--ds-color-warning,#d97706);color:#fff}}
.b-danger{{background:var(--ds-color-danger,#dc2626);color:#fff}}
.stack{{display:flex;flex-direction:column;gap:.7rem}}
.logos{{display:flex;flex-wrap:wrap;gap:.9rem;align-items:center}}
.logo-tile{{display:flex;align-items:center;justify-content:center;height:56px;min-width:100px;padding:.5rem .9rem;
border:1px solid var(--ds-color-border,#e5e7eb);border-radius:var(--ds-radius-md,8px);background:var(--ds-color-muted,#f8fafc)}}
.logo-tile img{{max-height:40px;max-width:160px;object-fit:contain}}
.imagery{{display:grid;grid-template-columns:repeat(auto-fill,minmax(200px,1fr));gap:.9rem}}
.img-tile{{margin:0;border-radius:var(--ds-radius-md,8px);overflow:hidden;border:1px solid var(--ds-color-border,#eef0f3);aspect-ratio:16/10;background:var(--ds-color-muted,#f1f5f9)}}
.img-tile img{{width:100%;height:100%;object-fit:cover;display:block}}
</style>
<style id="ds-live"></style>
</head>
<body>
<header><h1>{name}</h1><button class="toggle" onclick="document.body.classList.toggle('dark')">明 / 暗</button></header>
<main>
{logos}
{colors}
{images}
{fonts}
{texts}
{spaces}
{radshadow}
<section><h2>组件 · Components</h2><div class="components">
<div class="demo-card"><div class="stack">
<div style="display:flex;gap:.5rem;flex-wrap:wrap">
<button class="btn btn-primary">主按钮</button>
<button class="btn btn-secondary">次按钮</button>
<button class="btn btn-accent">强调</button>
<button class="btn btn-outline">描边</button></div>
<input class="field" placeholder="输入框样张…">
<div class="badges"><span class="badge b-primary">主要</span><span class="badge b-success">成功</span>
<span class="badge b-warning">警告</span><span class="badge b-danger">危险</span></div>
</div></div>
<div class="demo-card"><div class="stack">
<strong style="font-size:1.05rem">卡片标题</strong>
<p style="margin:0;font-size:.85rem;color:var(--ds-color-secondary,#6b7280)">卡片正文——展示当前设计系统在真实组件里的排版、圆角、阴影与配色。</p>
<button class="btn btn-primary" style="align-self:flex-start">了解更多</button>
</div></div>
</div></section>
</main>
<script>
window.addEventListener('message',function(e){{var d=e.data;if(d&&d.type==='ds_kit_tokens'){{var s=document.getElementById('ds-live');if(s)s.textContent=String(d.css||'')}}}});
</script>
</body></html>"##,
        name = esc_name,
        root = root,
        logos = section("Logo", &logo_html, "logos"),
        images = section("配图 · Imagery", &image_html, "imagery"),
        colors = section("色彩 · Colors", &color_swatches, "swatches"),
        fonts = section("字体 · Typography", &fonts, "type-scale"),
        texts = section("字号阶 · Type scale", &text_scale, "type-scale"),
        spaces = section("间距 · Spacing", &space_bars, "space-list"),
        radshadow = section(
            "圆角 · 阴影 · Radius / Shadow",
            &format!("{radius_boxes}{shadow_boxes}"),
            "swatches"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("--ds-color-primary".into(), "#2563eb".into()),
            ("--ds-color-accent".into(), "#0ea5e9".into()),
            ("--ds-font-sans".into(), "system-ui".into()),
            ("--ds-text-lg".into(), "20px".into()),
            ("--ds-space-4".into(), "16px".into()),
            ("--ds-radius-md".into(), "10px".into()),
        ])
    }

    #[test]
    fn kit_is_self_contained_and_reflects_tokens() {
        let html = build_kit_html("测试系统", &sys(), &[], &[]);
        assert!(html.starts_with("<!doctype html>"));
        // 无外链 / 无网络（自包含红线）。
        assert!(!html.contains("http://") && !html.contains("https://"));
        assert!(!html.contains("<link") && !html.contains("cdn"));
        // token 注入到 :root。
        assert!(html.contains("--ds-color-primary:#2563eb"));
        // 色板引用 var，组件用 var（即系统真实视觉）。
        assert!(html.contains("var(--ds-color-primary)"));
        assert!(html.contains("class=\"btn btn-primary\""));
        // 名称转义 + light/dark 切换存在。
        assert!(html.contains("测试系统"));
        assert!(html.contains("classList.toggle('dark')"));
    }

    #[test]
    fn kit_escapes_name_and_values() {
        let mut t = sys();
        t.insert("--ds-color-x".into(), "#fff".into());
        let html = build_kit_html("<script>alert(1)</script>", &t, &[], &[]);
        assert!(!html.contains("<script>alert(1)"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn kit_handles_empty_tokens() {
        let html = build_kit_html("空系统", &BTreeMap::new(), &[], &[]);
        assert!(html.contains("空系统"));
        // 空 token 也出组件 showcase（用骨架默认值），不 panic、不空页。
        assert!(html.contains("btn-primary"));
    }

    #[test]
    fn kit_renders_data_uri_assets_only() {
        // B1-4：data:image/ 资产渲染进 Logo/配图段；非 data:image/ 的（防注入）被滤掉。
        let logos = vec![
            "data:image/png;base64,iVBORw0KGgo=".to_string(),
            "https://evil.example/x.png".to_string(), // 非 data → 滤掉
        ];
        let images = vec!["data:image/jpeg;base64,/9j/4AAQ".to_string()];
        let html = build_kit_html("带资产", &sys(), &logos, &images);
        assert!(html.contains("data:image/png;base64,iVBORw0KGgo="));
        assert!(html.contains("data:image/jpeg;base64,/9j/4AAQ"));
        assert!(html.contains("class=\"logo-tile\""));
        // 非 data:image/ 的 http URL 不进套件（守自包含 + 防注入）。
        assert!(!html.contains("evil.example"));
    }

    #[test]
    fn kit_no_assets_omits_asset_sections() {
        // 无资产 → 不出 Logo/配图段（section() 空 inner 返回空）；CSS 里的类名不算。
        let html = build_kit_html("无资产", &sys(), &[], &[]);
        assert!(!html.contains(">Logo</h2>"));
        assert!(!html.contains("配图 · Imagery</h2>"));
        assert!(!html.contains("<img"));
    }
}
