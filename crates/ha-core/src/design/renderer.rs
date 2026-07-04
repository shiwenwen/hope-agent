//! 产物渲染器：把产物源（body/css/js）编译为**自包含 `index.html`**。
//!
//! 核心分水岭（见 `docs/architecture/design-space.md` §5）：产物是自包含 HTML，
//! iframe 直接加载渲染，**绝不在浏览器里编译 React/JSX/Tailwind**。
//!
//! Phase 1：骨架包裹 + 各 kind 视口 + 内联 css/js。
//! Phase 3 追加：设计系统 token 注入（`:root --ds-*`）、`data-ds-oid` 标注 +
//! `oidmap.json`、inspector bridge、deck 翻页器、mobile 设备框等。

/// 产物形态（kind）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    Web,
    Mobile,
    Deck,
    Dashboard,
    Poster,
    Document,
    Email,
    Image,
}

impl ArtifactKind {
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "web" => Self::Web,
            "mobile" => Self::Mobile,
            "deck" => Self::Deck,
            "dashboard" => Self::Dashboard,
            "poster" => Self::Poster,
            "document" => Self::Document,
            "email" => Self::Email,
            "image" => Self::Image,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Mobile => "mobile",
            Self::Deck => "deck",
            Self::Dashboard => "dashboard",
            Self::Poster => "poster",
            Self::Document => "document",
            Self::Email => "email",
            Self::Image => "image",
        }
    }

    /// 默认视口（宽, 高）。高为 0 表示自适应内容高。
    pub fn default_viewport(self) -> (i64, i64) {
        match self {
            Self::Web => (1440, 0),
            Self::Mobile => (390, 844),
            Self::Deck => (1280, 720),
            Self::Dashboard => (1440, 0),
            Self::Poster => (1080, 1080),
            Self::Document => (820, 0),
            Self::Email => (600, 0),
            Self::Image => (0, 0),
        }
    }
}

/// 产物源码各部分。
#[derive(Debug, Clone, Default)]
pub struct ArtifactParts {
    /// body 结构 HTML（不含 `<html>`/`<head>`/`<body>` 外壳）。
    pub body_html: String,
    /// 用户 CSS（内联进 `<style>`）。
    pub css: String,
    /// 用户 JS（内联进 `<script>`，可选）。
    pub js: String,
}

/// 编译自包含 `index.html`。
pub fn build_artifact_html(kind: ArtifactKind, title: &str, parts: &ArtifactParts) -> String {
    let (vw, _vh) = kind.default_viewport();
    let esc_title = html_escape(title);

    // 骨架基础样式：中性 reset + 变量占位（Phase 3 由设计系统 token 覆盖）。
    let base_css = r#"*,*::before,*::after{box-sizing:border-box}
html,body{margin:0;padding:0}
body{font-family:var(--ds-font-sans,system-ui,-apple-system,"Segoe UI",Roboto,"Helvetica Neue",Arial,"PingFang SC","Microsoft YaHei",sans-serif);
color:var(--ds-color-fg,#111827);background:var(--ds-color-bg,#ffffff);line-height:1.5;-webkit-font-smoothing:antialiased}
img,svg,video{max-width:100%;height:auto;display:block}
a{color:var(--ds-color-primary,#2563eb)}"#;

    // kind 专属容器样式。
    let frame_css = match kind {
        ArtifactKind::Mobile => {
            "body{display:flex;justify-content:center;background:#0b0b0c}\n\
             .ds-frame{width:390px;min-height:844px;background:var(--ds-color-bg,#fff);\
             border-radius:44px;overflow:hidden;box-shadow:0 20px 60px rgba(0,0,0,.4);margin:24px 0}"
        }
        ArtifactKind::Deck => {
            "body{background:#0b0b0c}\n\
             .ds-slide{width:1280px;min-height:720px;margin:0 auto;background:var(--ds-color-bg,#fff);\
             display:none}\n.ds-slide.active{display:block}"
        }
        ArtifactKind::Poster => {
            "body{display:flex;justify-content:center;align-items:flex-start;background:#0b0b0c}\n\
             .ds-frame{margin:24px 0;box-shadow:0 20px 60px rgba(0,0,0,.4)}"
        }
        ArtifactKind::Document => {
            "body{background:#f5f5f5}\n\
             .ds-frame{max-width:820px;margin:0 auto;padding:56px 64px;background:var(--ds-color-bg,#fff);\
             min-height:100vh;box-shadow:0 0 0 1px rgba(0,0,0,.04)}"
        }
        ArtifactKind::Email => {
            "body{background:#f0f0f0}\n\
             .ds-frame{max-width:600px;margin:0 auto;background:var(--ds-color-bg,#fff)}"
        }
        _ => "",
    };

    // body 包裹：mobile/poster/document/email 套 .ds-frame；web/dashboard 直接放。
    let wrapped_body = match kind {
        ArtifactKind::Mobile
        | ArtifactKind::Poster
        | ArtifactKind::Document
        | ArtifactKind::Email => {
            format!("<div class=\"ds-frame\">{}</div>", parts.body_html)
        }
        _ => parts.body_html.clone(),
    };

    let viewport_meta = if vw > 0 {
        format!("width={vw}, initial-scale=1")
    } else {
        "width=device-width, initial-scale=1".to_string()
    };

    let user_js = if parts.js.trim().is_empty() {
        String::new()
    } else {
        format!("<script>\n{}\n</script>", parts.js)
    };

    format!(
        "<!doctype html>\n<html lang=\"zh\" data-ds-kind=\"{kind}\">\n<head>\n\
<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"{viewport}\">\n\
<title>{title}</title>\n\
<style>\n{base}\n{frame}\n{user_css}\n</style>\n\
</head>\n<body>\n{body}\n{user_js}\n</body>\n</html>\n",
        kind = kind.as_str(),
        viewport = viewport_meta,
        title = esc_title,
        base = base_css,
        frame = frame_css,
        user_css = parts.css,
        body = wrapped_body,
        user_js = user_js,
    )
}

/// 占位产物（新建空产物时用，让预览 iframe 有内容）。
pub fn placeholder_parts(kind: ArtifactKind, title: &str) -> ArtifactParts {
    let esc = html_escape(title);
    let body = format!(
        "<main style=\"display:flex;flex-direction:column;align-items:center;justify-content:center;\
min-height:60vh;gap:12px;padding:48px;text-align:center;color:#9ca3af\">\
<div style=\"font-size:15px;font-weight:600;color:#4b5563\">{esc}</div>\
<div style=\"font-size:13px\">{hint}</div></main>",
        hint = "空白产物 · 在对话中描述你想要的设计",
    );
    let _ = kind;
    ArtifactParts {
        body_html: body,
        css: String::new(),
        js: String::new(),
    }
}

/// 最小 HTML 转义（属性 / 文本共用）。
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_roundtrip() {
        for k in [
            ArtifactKind::Web,
            ArtifactKind::Mobile,
            ArtifactKind::Deck,
            ArtifactKind::Dashboard,
            ArtifactKind::Poster,
            ArtifactKind::Document,
            ArtifactKind::Email,
            ArtifactKind::Image,
        ] {
            assert_eq!(ArtifactKind::from_str(k.as_str()), Some(k));
        }
        assert_eq!(ArtifactKind::from_str("nope"), None);
    }

    #[test]
    fn build_is_self_contained() {
        let parts = ArtifactParts {
            body_html: "<h1>Hi</h1>".into(),
            css: ".x{color:red}".into(),
            js: "console.log(1)".into(),
        };
        let html = build_artifact_html(ArtifactKind::Web, "T", &parts);
        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("<h1>Hi</h1>"));
        assert!(html.contains(".x{color:red}"));
        assert!(html.contains("console.log(1)"));
        // 零网络：不引外链
        assert!(!html.contains("http://"));
        assert!(!html.contains("https://"));
    }

    #[test]
    fn escapes_title() {
        let parts = ArtifactParts::default();
        let html = build_artifact_html(ArtifactKind::Web, "<script>", &parts);
        assert!(html.contains("&lt;script&gt;"));
    }
}
