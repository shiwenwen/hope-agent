//! 产物渲染器：把产物源（body/css/js）编译为**自包含 `index.html`**。
//!
//! 核心分水岭（见 `docs/architecture/design-space.md` §5）：**编译只在 ha-core 后端，浏览器
//! 零编译/零打包/零 JIT**——iframe 只加载已编译落盘的静态 `index.html`（旧版 atelier 因
//! in-browser 编译白屏被推倒重做）。9 静态 kind + audio 是纯自包含 HTML；`component`（交互式
//! React）经 `super::compile`（oxc 后端编译 JSX→JS）+ [`build_component_html`] 内联 vendored
//! React UMD 组装，仍是「浏览器载静态、不编译」。
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
    Audio,
    /// 交互式组件（React/JSX，后端 oxc 预编译，内联 React runtime）。
    Component,
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
            "audio" => Self::Audio,
            "component" => Self::Component,
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
            Self::Audio => "audio",
            Self::Component => "component",
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
            Self::Audio => (640, 0),
            Self::Component => (1024, 0),
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

/// 设计系统 token → `:root{--ds-*}` CSS 变量串。空 tokens = 空串（用骨架默认值）。
/// 单一来源——`build_artifact_html`（定稿产物）与 `build_stream_host_html`（流式占位页）
/// 共用，保证明暗自适应变量在两态字节一致。
fn tokens_root_css(tokens: &[(String, String)]) -> String {
    if tokens.is_empty() {
        return String::new();
    }
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
}

/// 骨架基础样式：中性 reset + 变量占位（设计系统 token 覆盖）。
fn reset_base_css() -> &'static str {
    r#"*,*::before,*::after{box-sizing:border-box}
html,body{margin:0;padding:0}
body{font-family:var(--ds-font-sans,system-ui,-apple-system,"Segoe UI",Roboto,"Helvetica Neue",Arial,"PingFang SC","Microsoft YaHei",sans-serif);
color:var(--ds-color-fg,#111827);background:var(--ds-color-bg,#ffffff);line-height:1.5;-webkit-font-smoothing:antialiased}
img,svg,video{max-width:100%;height:auto;display:block}
a{color:var(--ds-color-primary,#2563eb)}"#
}

/// kind 专属容器样式（`.ds-frame` / `.ds-slide` / `.ds-stage`）。
fn kind_frame_css(kind: ArtifactKind) -> &'static str {
    match kind {
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
    }
}

/// 流式期把 kind 内容容器包成产物同款结构（供 `ds-stream-body` 落位）。
fn wrap_kind_body(kind: ArtifactKind, inner: &str) -> String {
    match kind {
        ArtifactKind::Mobile
        | ArtifactKind::Poster
        | ArtifactKind::Document
        | ArtifactKind::Email => format!("<div class=\"ds-frame\">{inner}</div>"),
        ArtifactKind::Motion => format!("<div class=\"ds-stage\">{inner}</div>"),
        _ => inner.to_string(),
    }
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

    // 设计系统 token → :root CSS 变量（单一来源 helper，与流式占位页共用）。
    let root_css = tokens_root_css(tokens);

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

    // 骨架基础样式 + kind 专属容器样式（单一来源 helper，与流式占位页共用）。
    let base_css = reset_base_css();
    let frame_css = kind_frame_css(kind);

    // body 包裹：mobile/poster/document/email 套 .ds-frame；motion 套 .ds-stage；其余直接放。
    let wrapped_body = wrap_kind_body(kind, &annotated_body);

    let viewport_meta = if vw > 0 {
        format!("width={vw}, initial-scale=1")
    } else {
        "width=device-width, initial-scale=1".to_string()
    };

    // 中和 user CSS/JS 里的 `</style>`/`</script>`（大小写不敏感），防其提前闭合 raw-text 块致
    // 整页版式错乱——与 build_component_html 对齐（沙盒已隔离，故是产物正确性而非安全问题）。
    let safe_user_css = neutralize_closing(&parts.css, "</style");
    let user_js = if parts.js.trim().is_empty() {
        String::new()
    } else {
        format!(
            "<script>\n{}\n</script>",
            neutralize_closing(&parts.js, "</script")
        )
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
        user_css = safe_user_css,
        body = wrapped_body,
        user_js = user_js,
        deck_js = deck_js,
        inspector_js = inspector_js,
    );
    (html, oidmap)
}

/// Inspector bridge：dormant，收到父窗 `ds_activate` 才启用点选；选中元素回传父窗
/// （oid / tag / 关键样式 / 文本 / 是否叶子），支持 `ds_preview_style` / `ds_set_text`
/// live preview。**就地文本编辑**：双击叶子文本元素 → `contenteditable` 原地改，
/// Enter / 失焦提交（发 `ds_text_commit`，父窗走 `apply_text_patch` + `expected_hash`
/// 确定性回写）、Esc 取消（还原）。**沙箱零网络**，只通过 postMessage 通信。
const INSPECTOR_BRIDGE: &str = r#"<script>
(function(){
  var active=false, hovered=null, selected=null, editing=null, editOrig=null;
  var commentMode=false, comments=[], pinLayer=null;
  var CSS_PROPS=['color','background-color','font-size','font-weight','font-style','text-align',
    'text-transform','text-decoration','line-height','letter-spacing',
    'padding','margin','gap','width','height','max-width','min-height',
    'border-radius','border-width','border-style','border-color','box-shadow','opacity',
    'display','align-items','justify-content','z-index'];
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
  // 结束就地编辑：commit 时把新 textContent（拍平任何 contenteditable 插入的标记）发父窗
  // 走确定性回写并回传最新 info 同步 inspector；取消 / 无变化则还原原文。先置 editing=null 防 blur 重入。
  function endEdit(commit){
    var el=editing;if(!el)return;editing=null;
    el.removeAttribute('contenteditable');el.style.outline='2px solid #2563eb';
    var newText=el.textContent||'',oid=el.getAttribute('data-ds-oid');
    if(commit&&newText!==editOrig){
      parent.postMessage({type:'ds_text_commit',oid:oid,text:newText},'*');
      parent.postMessage({type:'ds_selected',payload:info(el)},'*');
    }else{el.textContent=editOrig}
    editOrig=null;
  }
  // ── 批注钉：iframe 内渲染（坐标随锚元素、zoom 无关）；点钉回传父窗 ──
  function ensurePinLayer(){
    if(pinLayer)return pinLayer;
    pinLayer=document.createElement('div');
    pinLayer.setAttribute('data-ds-pinlayer','1');
    pinLayer.style.cssText='position:fixed;inset:0;pointer-events:none;z-index:2147483646';
    document.documentElement.appendChild(pinLayer);
    return pinLayer;
  }
  // 解析钉的锚元素：oid 命中(同 tag)优先；失配则按 snippet 前缀在同 tag 元素中重锚
  // （跨设计变更/重生成 oid 漂移时软着陆）；再无 → null（脱锚）。
  function resolveEl(c){
    if(c.oid!=null){var el=elByOid(String(c.oid));
      if(el&&(!c.tag||el.tagName.toLowerCase()===c.tag))return el}
    var pre=(c.snippet||'').slice(0,40);
    if(pre){var cands=document.querySelectorAll('[data-ds-oid]');
      for(var i=0;i<cands.length;i++){var e=cands[i];
        if(c.tag&&e.tagName.toLowerCase()!==c.tag)continue;
        if((e.outerHTML||'').indexOf(pre)===0)return e}}
    return null;
  }
  function pinPos(c,i){
    var el=resolveEl(c);
    if(el){var r=el.getBoundingClientRect();
      return {x:r.left+(c.relX||0)*r.width,y:r.top+(c.relY||0)*r.height,el:el}}
    return {x:window.innerWidth-22,y:22+i*26,el:null}; // 脱锚：右上角堆叠，不丢
  }
  function renderPins(){
    if(!commentMode){if(pinLayer)pinLayer.style.display='none';return}
    var layer=ensurePinLayer();layer.style.display='';layer.textContent='';
    comments.forEach(function(c,i){
      var p=pinPos(c,i),dot=document.createElement('button');
      dot.type='button';
      dot.style.cssText='position:absolute;transform:translate(-50%,-50%);pointer-events:auto;'+
        'width:22px;height:22px;border-radius:50% 50% 50% 2px;border:2px solid #fff;cursor:pointer;'+
        'font:600 11px system-ui;color:#fff;display:flex;align-items:center;justify-content:center;'+
        'box-shadow:0 1px 4px rgba(0,0,0,.35);left:'+p.x+'px;top:'+p.y+'px;'+
        'background:'+(c.resolved?'#16a34a':'#f59e0b');
      dot.textContent=String(i+1);
      dot.title=(c.body||'').slice(0,80);
      // 指针交互：小位移=点击(聚焦)，拖动=重锚到落点下的元素。
      (function(cm){
        var sx=0,sy=0,moved=false,dragging=false;
        dot.addEventListener('pointerdown',function(ev){ev.preventDefault();ev.stopPropagation();
          sx=ev.clientX;sy=ev.clientY;moved=false;dragging=true;
          try{dot.setPointerCapture(ev.pointerId)}catch(_){}});
        dot.addEventListener('pointermove',function(ev){if(!dragging)return;
          if(Math.abs(ev.clientX-sx)>4||Math.abs(ev.clientY-sy)>4)moved=true;
          if(moved){dot.style.left=ev.clientX+'px';dot.style.top=ev.clientY+'px'}});
        dot.addEventListener('pointerup',function(ev){if(!dragging)return;dragging=false;
          try{dot.releasePointerCapture(ev.pointerId)}catch(_){}
          if(!moved){parent.postMessage({type:'ds_comment_click',id:cm.id},'*');return}
          dot.style.pointerEvents='none';
          var tgt=document.elementFromPoint(ev.clientX,ev.clientY);
          dot.style.pointerEvents='auto';
          var nel=tgt&&tgt.closest?tgt.closest('[data-ds-oid]'):null;
          if(nel){var nr=nel.getBoundingClientRect();
            parent.postMessage({type:'ds_comment_relocate',id:cm.id,
              oid:Number(nel.getAttribute('data-ds-oid')),
              relX:nr.width?(ev.clientX-nr.left)/nr.width:0.5,
              relY:nr.height?(ev.clientY-nr.top)/nr.height:0.5},'*');
          }else{renderPins()} // 落空白 → 复位
        });
      })(c);
      layer.appendChild(dot);
    });
  }
  var reflowRaf=0;
  function scheduleReflow(){if(!commentMode||reflowRaf)return;
    reflowRaf=requestAnimationFrame(function(){reflowRaf=0;renderPins()})}
  window.addEventListener('scroll',scheduleReflow,true);
  window.addEventListener('resize',scheduleReflow);
  document.addEventListener('mouseover',function(e){
    if(!active||editing)return;var el=e.target.closest('[data-ds-oid]');if(!el||el===selected)return;
    clearHover();hovered=el;el.style.outline='1px solid rgba(37,99,235,.5)';
  },true);
  document.addEventListener('mouseout',function(){if(active&&!editing)clearHover()},true);
  document.addEventListener('click',function(e){
    if(commentMode){
      e.preventDefault();e.stopPropagation(); // 批注态吞掉所有点击，不泄漏到设计自身 handler
      var cel=e.target.closest('[data-ds-oid]');
      if(!cel)return; // 点在钉 / 空白 → 已吞事件、不落新钉（钉自身走 pointerup）
      var cr=cel.getBoundingClientRect();
      parent.postMessage({type:'ds_comment_place',
        oid:Number(cel.getAttribute('data-ds-oid')),
        relX:cr.width?(e.clientX-cr.left)/cr.width:0.5,
        relY:cr.height?(e.clientY-cr.top)/cr.height:0.5,
        tag:cel.tagName.toLowerCase(),
        snippet:(cel.outerHTML||'').slice(0,200)},'*');
      return;
    }
    if(!active)return;
    if(editing){if(editing.contains(e.target))return;endEdit(true)} // 编辑内点=移光标；点外=提交
    var el=e.target.closest('[data-ds-oid]');if(!el)return;
    e.preventDefault();e.stopPropagation();
    clearSel();clearHover();selected=el;el.style.outline='2px solid #2563eb';
    parent.postMessage({type:'ds_selected',payload:info(el)},'*');
  },true);
  document.addEventListener('dblclick',function(e){
    if(!active)return;var el=e.target.closest('[data-ds-oid]');
    if(!el||el.childElementCount!==0)return; // 仅叶子文本元素（有子元素则改会拍平内部标记）
    e.preventDefault();e.stopPropagation();
    if(editing&&editing!==el)endEdit(true);
    clearHover();clearSel();selected=el;editing=el;editOrig=el.textContent||'';
    el.setAttribute('contenteditable','true');el.style.outline='2px dashed #16a34a';el.focus();
    var s=window.getSelection(),r=document.createRange();r.selectNodeContents(el);s.removeAllRanges();s.addRange(r);
  },true);
  document.addEventListener('keydown',function(e){
    if(!editing)return;
    if(e.key==='Enter'&&!e.shiftKey){e.preventDefault();endEdit(true)}
    else if(e.key==='Escape'){e.preventDefault();endEdit(false)}
  },true);
  document.addEventListener('blur',function(e){if(editing&&e.target===editing)endEdit(true)},true);
  window.addEventListener('message',function(e){
    var d=e.data||{};
    if(d.type==='ds_activate'){active=true}
    else if(d.type==='ds_deactivate'){active=false;endEdit(false);clearSel();clearHover()}
    else if(d.type==='ds_preview_style'){
      var el=elByOid(d.oid);if(!el)return;
      (d.props||[]).forEach(function(kv){el.style.setProperty(kv[0],kv[1])});
    }
    else if(d.type==='ds_set_text'){var el=elByOid(d.oid);if(el)el.textContent=d.text}
    else if(d.type==='ds_reselect'){var el=elByOid(d.oid);
      if(el){clearSel();selected=el;el.style.outline='2px solid #2563eb';
        parent.postMessage({type:'ds_selected',payload:info(el)},'*')}}
    else if(d.type==='ds_comment_mode'){commentMode=!!d.on;renderPins()}
    else if(d.type==='ds_comments_set'){comments=Array.isArray(d.comments)?d.comments:[];renderPins()}
    else if(d.type==='ds_comment_focus'){
      var fc=comments.filter(function(x){return x.id===d.id})[0];
      if(fc&&fc.oid!=null){var fe=elByOid(String(fc.oid));
        if(fe){fe.scrollIntoView({block:'center',behavior:'smooth'});setTimeout(renderPins,320)}}}
  });
})();
</script>"#;

/// 流式占位页接收脚本：**dormant + postMessage + 零网络**（仿 `INSPECTOR_BRIDGE`）。
/// 父窗流式期发 `ds_stream_css`（把最新完整 CSS 灌进 `<style id=ds-user-css>`，head 先定稿
/// 故先有样式再有 body = 无 FOUC）/ `ds_stream_body`（把「到目前为止的完整 body」整体写进
/// `#ds-stream-body`，累积快照语义，failover 重试自动收敛不拼接）。挂载即回 `ds_stream_ready`
/// 让父窗补投最新快照。**不执行流式 body 里的 `<script>`**（innerHTML 不跑脚本）——JS 只在
/// 定稿 index.html 生效，故流式期天然无副作用。
/// 流式占位页的「生成中」spinner 样式（零文案，居中，尊重 prefers-reduced-motion）+ deck
/// 流式覆盖：frame_css 把 `.ds-slide` 设 `display:none`（靠 pager JS 点亮 active），但流式期
/// 不跑 JS，故这里同特异性、后出现地翻成 `display:block`——让 deck 各页流式期堆叠可见（定稿
/// 的真 index.html 无本段、回到分页器）。
const STREAM_HOST_STYLE: &str = "@keyframes ds-spin{to{transform:rotate(360deg)}}\n\
.ds-gen{position:fixed;inset:0;display:flex;align-items:center;justify-content:center;z-index:2147483647;pointer-events:none}\n\
.ds-gen-r{width:28px;height:28px;border:2.5px solid rgba(130,140,155,.22);border-top-color:rgba(130,140,155,.8);border-radius:50%;animation:ds-spin .8s linear infinite}\n\
@media(prefers-reduced-motion:reduce){.ds-gen-r{animation-duration:2.4s}}\n\
.ds-slide{display:block;margin-bottom:16px}";

const STREAM_HOST_SCRIPT: &str = r#"<script>
(function(){
  window.addEventListener('message',function(e){
    var d=e.data||{};
    if(d.type==='ds_stream_css'){
      var s=document.getElementById('ds-user-css');if(s)s.textContent=d.css||'';
    } else if(d.type==='ds_stream_body'){
      // 仅非空 body 帧才替换 innerHTML（清掉内嵌 spinner）；CSS-only 首帧 body 为空时
      // 不动，spinner 继续转、样式已就位。cumulative 语义下 body 只增不减（failover
      // 重启短暂回空也不清，避免闪回空白）。
      if(typeof d.html==='string'&&d.html.length){
        var r=document.getElementById('ds-stream-body');if(r)r.innerHTML=d.html;
      }
    }
  });
  parent.postMessage({type:'ds_stream_ready'},'*');
})();
</script>"#;

/// 流式占位页：与定稿产物**同款 head 铁序**（`root → base → frame`，token 一次注入不随流），
/// 空 body 容器 `#ds-stream-body` + 空 `<style id=ds-user-css>` 供增量替换 + 常驻接收脚本。
/// 编辑态语义 `false`——**不标 oid、不挂 inspector**（半流式 DOM 无法稳定算 oid）。定稿时由
/// `finalize` 落盘真 `index.html`（editable=true）经单次受控 swap 生效。
pub fn build_stream_host_html(
    kind: ArtifactKind,
    title: &str,
    tokens: &[(String, String)],
) -> String {
    let (vw, _vh) = kind.default_viewport();
    let esc_title = html_escape(title);
    let root_css = tokens_root_css(tokens);
    let base_css = reset_base_css();
    let frame_css = kind_frame_css(kind);
    // 首帧到达前（~1s TTFT）body 空 = 一屏空白；播一个居中 CSS spinner（零文案、免 i18n），
    // 读作「生成中」而非「坏了」。spinner 放 `#ds-stream-body` **内部**——首个非空 body 帧
    // 的 innerHTML 替换自然清掉它（放兄弟节点会永不移除、全程盖住内容）。
    let inner =
        "<div id=\"ds-stream-body\"><div class=\"ds-gen\"><div class=\"ds-gen-r\"></div></div></div>";
    let wrapped_body = wrap_kind_body(kind, inner);
    let viewport_meta = if vw > 0 {
        format!("width={vw}, initial-scale=1")
    } else {
        "width=device-width, initial-scale=1".to_string()
    };
    format!(
        "<!doctype html>\n<html lang=\"zh\" data-ds-kind=\"{kind}\" data-ds-streaming=\"1\">\n<head>\n\
<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"{viewport}\">\n\
<title>{title}</title>\n\
<style>\n{root}\n{base}\n{frame}\n{host}\n</style>\n\
<style id=\"ds-user-css\"></style>\n\
</head>\n<body>\n{body}\n{host_js}\n</body>\n</html>\n",
        kind = kind.as_str(),
        viewport = viewport_meta,
        title = esc_title,
        root = root_css,
        base = base_css,
        frame = frame_css,
        host = STREAM_HOST_STYLE,
        body = wrapped_body,
        host_js = STREAM_HOST_SCRIPT,
    )
}

// ── 交互式组件（Component kind）：后端 oxc 预编译 + 内联 React runtime ────────────
//
// vendored React 18 production UMD（`include_str!`，零网络、锁版本）。React 19 已删 UMD 构建，
// 故 pin React 18。编译在 ha-core（`design::compile`），iframe 只载已编译静态 JS——守红线。
const REACT_UMD: &str = include_str!("assets/react.production.min.js");
const REACT_DOM_UMD: &str = include_str!("assets/react-dom.production.min.js");

/// 中和内联 `<script>`/`<style>` 块里会**提前闭合该块**的 `</script` / `</style`（大小写不敏感，
/// HTML 解析器如此）——LLM 组件源里的字符串字面量 `"</script>"` 编译后会原样进 `<script>` 块、
/// 提前关闭脚本破坏整页。`<\/script` 在 JS/CSS 里语义等价、无害。字节级、ASCII needle，UTF-8 安全。
fn neutralize_closing(s: &str, needle_lower: &str) -> String {
    let sb = s.as_bytes();
    let nb = needle_lower.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(sb.len() + 16);
    let mut i = 0;
    while i < sb.len() {
        if i + nb.len() <= sb.len() && sb[i..i + nb.len()].eq_ignore_ascii_case(nb) {
            out.push(b'<');
            out.push(b'\\');
            out.extend_from_slice(&sb[i + 1..i + nb.len()]);
            i += nb.len();
        } else {
            out.push(sb[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// 空白 Component 的合法 JSX 占位源（新建无 brief 时用；直接 HTML 当 JSX 会编译失败）。
pub fn placeholder_component_source() -> &'static str {
    "function App() {\n\
  return (\n\
    <div style={{ minHeight: '60vh', display: 'flex', alignItems: 'center', justifyContent: 'center', padding: '48px', textAlign: 'center', color: '#9ca3af', fontFamily: 'system-ui, -apple-system, sans-serif' }}>\n\
      <div>在对话中描述你想要的交互组件，AI 会用 React 生成并即时运行。</div>\n\
    </div>\n\
  );\n\
}"
}

/// 组装交互式组件产物：内联 vendored React UMD + 已编译组件 JS + bootstrap（`createRoot`
/// 渲染全局 `App`）。**iframe 载静态、浏览器零编译**（守红线）；沙箱 `allow-scripts`、零网络。
///
/// `compiled_js` 是 `design::compile::compile_component` 的输出（classic JSX runtime，引用全局
/// `React`）。head 复用 token → `:root` + reset，用户 CSS 内联。
pub fn build_component_html(
    title: &str,
    compiled_js: &str,
    css: &str,
    tokens: &[(String, String)],
) -> String {
    let esc_title = html_escape(title);
    let root_css = tokens_root_css(tokens);
    let base_css = reset_base_css();
    // 中和 LLM 源里会提前闭合内联块的 </script> / </style>（守自包含产物完整）。
    let safe_component = neutralize_closing(compiled_js, "</script");
    let safe_css = neutralize_closing(css, "</style");
    format!(
        "<!doctype html>\n<html lang=\"zh\" data-ds-kind=\"component\">\n<head>\n\
<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>{title}</title>\n\
<style>\n{root}\n{base}\n{user_css}\n</style>\n\
</head>\n<body>\n<div id=\"ds-root\"></div>\n\
<script>{react}</script>\n\
<script>{react_dom}</script>\n\
<script>\n{component}\n</script>\n\
<script>\n(function(){{\n\
  try {{\n\
    var el = (typeof App !== 'undefined') ? App : (typeof Component !== 'undefined' ? Component : null);\n\
    if (!el) {{ throw new Error('No <App/> component defined'); }}\n\
    ReactDOM.createRoot(document.getElementById('ds-root')).render(React.createElement(el));\n\
  }} catch (e) {{\n\
    document.getElementById('ds-root').innerHTML =\n\
      '<pre style=\"color:#b23a34;padding:24px;white-space:pre-wrap;font:13px ui-monospace,monospace\">'\n\
      + String(e && e.message || e) + '</pre>';\n\
  }}\n\
}})();\n\
</script>\n\
</body>\n</html>\n",
        title = esc_title,
        root = root_css,
        base = base_css,
        user_css = safe_css,
        react = REACT_UMD,
        react_dom = REACT_DOM_UMD,
        component = safe_component,
    )
}

/// 组件编译失败时的静态错误页（产物仍可打开、清晰展示编译错误，可重新生成）。
pub fn build_component_error_html(title: &str, error: &str) -> String {
    let esc_title = html_escape(title);
    let esc_err = html_escape(error);
    format!(
        "<!doctype html>\n<html lang=\"zh\" data-ds-kind=\"component\">\n<head>\n\
<meta charset=\"utf-8\">\n<title>{title}</title>\n\
<style>body{{margin:0;font-family:system-ui,-apple-system,sans-serif;background:#faf7f7;color:#16181d}}\
.wrap{{max-width:720px;margin:8vh auto;padding:0 24px}}\
.tag{{font:600 12px ui-monospace,monospace;color:#b23a34;letter-spacing:.08em;text-transform:uppercase}}\
h1{{font-size:20px;margin:8px 0 12px}}\
pre{{background:#fff;border:1px solid #e4d7d5;border-radius:10px;padding:16px;overflow-x:auto;\
white-space:pre-wrap;font:12.5px ui-monospace,monospace;color:#8a2f2a}}</style>\n\
</head>\n<body>\n<div class=\"wrap\">\
<div class=\"tag\">Component compile failed</div>\
<h1>{title}</h1>\
<p style=\"color:#6b7280;font-size:13.5px\">组件源码未能编译。修正后可重新生成。</p>\
<pre>{err}</pre></div>\n</body>\n</html>\n",
        title = esc_title,
        err = esc_err,
    )
}

/// 占位产物（新建空产物时用，让预览 iframe 有内容）。
pub fn placeholder_parts(kind: ArtifactKind, title: &str) -> ArtifactParts {
    // Component 占位是**合法 JSX 源**（body_html 存 JSX，render() 会 oxc 编译；HTML 占位会编译失败）。
    if kind == ArtifactKind::Component {
        return ArtifactParts {
            body_html: placeholder_component_source().to_string(),
            css: String::new(),
            js: String::new(),
        };
    }
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

    #[test]
    fn component_html_inlines_react_and_bootstraps() {
        let html = build_component_html("T", "function App(){return null}", ".x{}", &[]);
        assert!(html.contains("<div id=\"ds-root\"></div>"));
        // vendored React UMD inlined (react.production.min.js banner) — zero network.
        assert!(html.contains("react.production.min.js"));
        assert!(html.contains("ReactDOM.createRoot"));
        assert!(html.contains("function App(){return null}"));
        assert!(html.contains(".x{}"));
        // self-contained: no remote script/link src.
        assert!(!html.contains("src=\"http"));
        assert!(!html.contains("<script src"));
    }

    #[test]
    fn component_error_html_shows_error_escaped() {
        let html = build_component_error_html("My App", "Unexpected token <");
        assert!(html.contains("compile failed"));
        assert!(html.contains("Unexpected token &lt;"));
        assert!(html.contains("My App"));
    }

    #[test]
    fn placeholder_component_source_is_valid_app() {
        let src = placeholder_component_source();
        assert!(src.contains("function App"));
    }

    #[test]
    fn component_html_neutralizes_closing_script_in_source() {
        // A component string literal containing "</script>" must not break the page.
        let js = "function App(){return React.createElement('div',null,'</script><img src=x onerror=alert(1)>')}";
        let html = build_component_html("T", js, "a::after{content:'</style>'}", &[]);
        // The raw closing sequences must be neutralized (backslash-escaped) inside the blocks.
        assert!(!html.contains("'</script>"), "raw </script leaked: {html}");
        assert!(html.contains("<\\/script"), "not neutralized: {html}");
        assert!(!html.contains("'</style>"), "raw </style leaked");
    }
}
