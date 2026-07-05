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
    Motion,
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
            "motion" => Self::Motion,
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
            Self::Motion => "motion",
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
            Self::Motion => (1280, 720),
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
///
/// `tokens` 是设计系统展开的 CSS 变量（`("--ds-color-primary","#..")`），注入
/// `:root`；产物 CSS 引用变量即可换皮。空 = 不注入（用骨架默认值）。
pub fn build_artifact_html(
    kind: ArtifactKind,
    title: &str,
    parts: &ArtifactParts,
    tokens: &[(String, String)],
    editable: bool,
) -> (String, Vec<super::patch::OidEntry>) {
    let (vw, _vh) = kind.default_viewport();
    let esc_title = html_escape(title);

    // 编辑态注入 data-ds-oid（可视化微调锚点）+ 产出 oidmap；导出态用干净源码。
    let (annotated_body, oidmap) = if editable {
        super::patch::annotate(&parts.body_html)
    } else {
        (parts.body_html.clone(), Vec::new())
    };

    // inspector bridge（仅可编辑 kind）：dormant，收到父窗 ds_activate 才启用；
    // 选中元素回传、样式/文本 live preview。沙箱零网络。导出态不注入（干净产物）。
    let inspector_js = if editable { INSPECTOR_BRIDGE } else { "" };

    // 设计系统 token → :root CSS 变量。
    let root_css = if tokens.is_empty() {
        String::new()
    } else {
        let mut vars = String::from(":root{");
        for (k, v) in tokens {
            // 仅允许 --ds-* 变量名；值滤除 `}`/`{`/`<`/`;` 防注入逃逸（`;` 防单个
            // token 值塞入多条声明——extracted/url 来源的 token 由 LLM 可控）。
            if !k.starts_with("--ds-") {
                continue;
            }
            let safe_v: String = v
                .chars()
                .filter(|c| *c != '}' && *c != '{' && *c != '<' && *c != ';')
                .collect();
            vars.push_str(k);
            vars.push(':');
            vars.push_str(safe_v.trim());
            vars.push(';');
        }
        vars.push('}');
        vars
    };

    // deck 翻页器：一份文件多页，←/→/Space 切换，右下角页码。
    let deck_js = if kind == ArtifactKind::Deck {
        r#"<script>
(function(){
  var slides=[].slice.call(document.querySelectorAll('.ds-slide'));
  if(!slides.length)return;var i=0;
  var pager=document.createElement('div');
  pager.style.cssText='position:fixed;right:16px;bottom:12px;font:12px system-ui;color:#888;z-index:9';
  document.body.appendChild(pager);
  function show(n){i=Math.max(0,Math.min(slides.length-1,n));
    slides.forEach(function(s,k){s.classList.toggle('active',k===i)});
    pager.textContent=(i+1)+' / '+slides.length;}
  document.addEventListener('keydown',function(e){
    if(e.key==='ArrowRight'||e.key===' '){e.preventDefault();show(i+1)}
    else if(e.key==='ArrowLeft'){e.preventDefault();show(i-1)}});
  document.addEventListener('click',function(e){
    show(e.clientX>window.innerWidth/2?i+1:i-1)});
  show(0);
})();
</script>"#
    } else {
        ""
    };

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
        ArtifactKind::Motion => {
            "body{display:flex;align-items:center;justify-content:center;min-height:100vh;\
             margin:0;background:#0b0b0c}\n\
             .ds-stage{width:1280px;height:720px;overflow:hidden;position:relative;\
             background:var(--ds-color-bg,#0b0b0c)}"
        }
        _ => "",
    };

    // body 包裹：mobile/poster/document/email 套 .ds-frame；motion 套 .ds-stage；其余直接放。
    let wrapped_body = match kind {
        ArtifactKind::Mobile
        | ArtifactKind::Poster
        | ArtifactKind::Document
        | ArtifactKind::Email => {
            format!("<div class=\"ds-frame\">{annotated_body}</div>")
        }
        ArtifactKind::Motion => format!("<div class=\"ds-stage\">{annotated_body}</div>"),
        _ => annotated_body,
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

    let html = format!(
        "<!doctype html>\n<html lang=\"zh\" data-ds-kind=\"{kind}\">\n<head>\n\
<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"{viewport}\">\n\
<title>{title}</title>\n\
<style>\n{root}\n{base}\n{frame}\n{user_css}\n</style>\n\
</head>\n<body>\n{body}\n{user_js}\n{deck_js}\n{inspector_js}\n</body>\n</html>\n",
        kind = kind.as_str(),
        viewport = viewport_meta,
        title = esc_title,
        root = root_css,
        base = base_css,
        frame = frame_css,
        user_css = parts.css,
        body = wrapped_body,
        user_js = user_js,
        deck_js = deck_js,
        inspector_js = inspector_js,
    );
    (html, oidmap)
}

/// Inspector bridge：dormant，收到父窗 `ds_activate` 才启用点选；选中元素回传父窗
/// （oid / tag / 关键样式 / 文本 / 是否叶子），支持 `ds_preview_style` / `ds_set_text`
/// live preview。**沙箱零网络**，只通过 postMessage 通信。
const INSPECTOR_BRIDGE: &str = r#"<script>
(function(){
  var active=false, hovered=null, selected=null;
  var CSS_PROPS=['color','background-color','font-size','font-weight','text-align',
    'padding','margin','border-radius','line-height','letter-spacing','width','height'];
  function elByOid(oid){return document.querySelector('[data-ds-oid="'+oid+'"]')}
  function info(el){
    var cs=getComputedStyle(el), styles={};
    CSS_PROPS.forEach(function(p){styles[p]=cs.getPropertyValue(p)});
    var r=el.getBoundingClientRect();
    return {oid:el.getAttribute('data-ds-oid'),tag:el.tagName.toLowerCase(),
      styles:styles,text:el.textContent||'',isLeaf:el.childElementCount===0,
      rect:{x:r.x,y:r.y,w:r.width,h:r.height}};
  }
  function clearHover(){if(hovered){hovered.style.outline='';hovered=null}}
  function clearSel(){if(selected){selected.style.outline='';selected=null}}
  document.addEventListener('mouseover',function(e){
    if(!active)return;var el=e.target.closest('[data-ds-oid]');if(!el||el===selected)return;
    clearHover();hovered=el;el.style.outline='1px solid rgba(37,99,235,.5)';
  },true);
  document.addEventListener('mouseout',function(){if(active)clearHover()},true);
  document.addEventListener('click',function(e){
    if(!active)return;var el=e.target.closest('[data-ds-oid]');if(!el)return;
    e.preventDefault();e.stopPropagation();
    clearSel();clearHover();selected=el;el.style.outline='2px solid #2563eb';
    parent.postMessage({type:'ds_selected',payload:info(el)},'*');
  },true);
  window.addEventListener('message',function(e){
    var d=e.data||{};
    if(d.type==='ds_activate'){active=true}
    else if(d.type==='ds_deactivate'){active=false;clearSel();clearHover()}
    else if(d.type==='ds_preview_style'){
      var el=elByOid(d.oid);if(!el)return;
      (d.props||[]).forEach(function(kv){el.style.setProperty(kv[0],kv[1])});
    }
    else if(d.type==='ds_set_text'){var el=elByOid(d.oid);if(el)el.textContent=d.text}
    else if(d.type==='ds_reselect'){var el=elByOid(d.oid);
      if(el){clearSel();selected=el;el.style.outline='2px solid #2563eb';
        parent.postMessage({type:'ds_selected',payload:info(el)},'*')}}
  });
})();
</script>"#;

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
            ArtifactKind::Motion,
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
        let (html, map) = build_artifact_html(ArtifactKind::Web, "T", &parts, &[], true);
        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("<h1"));
        assert!(html.contains(">Hi</h1>"));
        assert!(html.contains(".x{color:red}"));
        assert!(html.contains("console.log(1)"));
        assert!(html.contains("data-ds-oid=\"0\""));
        assert_eq!(map.len(), 1);
        // 零网络：不引外链
        assert!(!html.contains("http://"));
        assert!(!html.contains("https://"));
    }

    #[test]
    fn escapes_title() {
        let parts = ArtifactParts::default();
        let (html, _) = build_artifact_html(ArtifactKind::Web, "<script>", &parts, &[], true);
        assert!(html.contains("&lt;script&gt;"));
    }
}
