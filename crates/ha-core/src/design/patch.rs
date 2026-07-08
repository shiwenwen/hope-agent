//! 可视化微调的确定性回写引擎（D1）。
//!
//! 核心：产物是**纯 HTML**，渲染 DOM 与源码结构一一对应，因此"选中元素→改属性→
//! 回写源码"是**确定性字节范围 patch**（对症旧版 JSX→React→DOM 有损映射的根因）。
//!
//! - `annotate`：遍历 body 源码，为每个 start tag 注入 `data-ds-oid="N"`（文档顺序），
//!   同时产出 `oidmap`（oid → 源码里该 start tag 的字节范围）。渲染用注入版，回写用
//!   oidmap 定位**源码**。
//! - `apply_style_patch`：合并 inline style 到目标元素 start tag。
//! - `apply_text_patch`：替换目标元素的**内部文本**（bridge 只对叶子元素开放）。
//!
//! 见 docs/architecture/design-space.md §7。

use serde::{Deserialize, Serialize};

/// oidmap 条目：目标元素 start tag 在**源码**（body.html）里的字节范围。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OidEntry {
    pub oid: u32,
    pub tag: String,
    /// start tag `<...>` 在源码里的起始字节（`<` 位置）。
    pub open_start: usize,
    /// start tag `<...>` 结束后的字节（`>` 之后）。
    pub open_end: usize,
    /// 是否 void 元素（无内部内容 / 无闭合）。
    pub void: bool,
}

const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// raw-text 元素：内容是 CDATA，绝不能把其中的 `<` 当标签扫描。
const RAW_TEXT_TAGS: &[&str] = &["script", "style", "textarea", "title"];

fn is_void(tag: &str) -> bool {
    VOID_TAGS.contains(&tag.to_ascii_lowercase().as_str())
}

/// 从 `from` 起找 `</{tag}`（大小写不敏感）的起始字节（`<` 位置）。
fn find_close_ci(bytes: &[u8], from: usize, tag: &str) -> Option<usize> {
    let tl = tag.as_bytes();
    let mut i = from;
    while i + 2 + tl.len() <= bytes.len() {
        if bytes[i] == b'<'
            && bytes[i + 1] == b'/'
            && bytes[i + 2..i + 2 + tl.len()].eq_ignore_ascii_case(tl)
            && matches!(
                bytes.get(i + 2 + tl.len()).copied(),
                Some(b'>')
                    | Some(b'/')
                    | Some(b' ')
                    | Some(b'\t')
                    | Some(b'\n')
                    | Some(b'\r')
                    | None
            )
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// 遍历 body 源码，注入 `data-ds-oid` 并产出 oidmap（映射回源码字节范围）。
pub fn annotate(source: &str) -> (String, Vec<OidEntry>) {
    let bytes = source.as_bytes();
    let n = bytes.len();
    let mut out = String::with_capacity(n + 64);
    let mut map = Vec::new();
    let mut oid: u32 = 0;
    let mut i = 0usize;

    while i < n {
        let b = bytes[i];
        if b != b'<' {
            // Copy the whole non-tag run as one UTF-8 slice. Never `byte as char`
            // (that Latin-1-reinterprets each byte and mojibakes multibyte text such
            // as Chinese). `i` stays byte-indexed so oidmap offsets are unchanged; the
            // run begins/ends on ASCII boundaries (`<` and the tag exits are ASCII),
            // so `source[start..i]` is always a valid char-boundary slice.
            let start = i;
            i += 1;
            while i < n && bytes[i] != b'<' {
                i += 1;
            }
            out.push_str(&source[start..i]);
            continue;
        }
        // 注释 / CDATA / doctype / 结束标签：原样拷贝到对应结束，不注入。
        if source[i..].starts_with("<!--") {
            let end = source[i..].find("-->").map(|p| i + p + 3).unwrap_or(n);
            out.push_str(&source[i..end]);
            i = end;
            continue;
        }
        if bytes.get(i + 1) == Some(&b'!') || bytes.get(i + 1) == Some(&b'/') {
            // <!doctype ...> 或 </tag>
            let end = find_tag_end(bytes, i).unwrap_or(n);
            out.push_str(&source[i..end]);
            i = end;
            continue;
        }
        let next = bytes.get(i + 1).copied();
        let is_start = matches!(next, Some(c) if c.is_ascii_alphabetic());
        if !is_start {
            out.push('<');
            i += 1;
            continue;
        }
        // start tag：提取 tag 名 + 找 `>`。
        let Some(open_end) = find_tag_end(bytes, i) else {
            out.push_str(&source[i..]);
            break;
        };
        let tag_str = &source[i..open_end]; // 含 `<` 与 `>`
        let name_end = tag_str[1..]
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .map(|p| p + 1)
            .unwrap_or(tag_str.len() - 1);
        let tag = tag_str[1..name_end].to_string();
        let void = is_void(&tag) || tag_str.trim_end().ends_with("/>");

        // 记录源码范围（未注入前的坐标）。
        map.push(OidEntry {
            oid,
            tag: tag.clone(),
            open_start: i,
            open_end,
            void,
        });

        // 注入 data-ds-oid 到 tag 名之后。
        out.push_str(&tag_str[..name_end]);
        out.push_str(&format!(" data-ds-oid=\"{oid}\""));
        out.push_str(&tag_str[name_end..]);

        oid += 1;

        // raw-text 元素：其内容是 CDATA，原样拷贝到匹配闭合标签前，绝不扫描其中的 `<`
        // （否则内联脚本里的 `document.write("<div>")` 会被误注 oid、破坏脚本 + 偏移坐标）。
        // 闭合标签本身交回主循环的 `</` 分支照常拷贝。
        if !void && RAW_TEXT_TAGS.contains(&tag.to_ascii_lowercase().as_str()) {
            let content_end = find_close_ci(bytes, open_end, &tag).unwrap_or(n);
            out.push_str(&source[open_end..content_end]);
            i = content_end;
            continue;
        }

        i = open_end;
    }

    (out, map)
}

/// 找到从 `start`（`<`）开始的标签的结束位置（`>` 之后一位），尊重引号。
fn find_tag_end(bytes: &[u8], start: usize) -> Option<usize> {
    let n = bytes.len();
    let mut i = start + 1;
    let mut quote: Option<u8> = None;
    while i < n {
        let c = bytes[i];
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => {
                if c == b'"' || c == b'\'' {
                    quote = Some(c);
                } else if c == b'>' {
                    return Some(i + 1);
                }
            }
        }
        i += 1;
    }
    None
}

/// BLAKE3 hex（stale-write 守卫用）。
pub fn body_hash(source: &str) -> String {
    blake3::hash(source.as_bytes()).to_hex().to_string()
}

/// patch 结果。
#[derive(Debug, Clone)]
pub struct PatchResult {
    pub new_source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatchError {
    Stale,
    OidNotFound(u32),
    NoClose(u32),
    VoidText,
    NotLeaf(u32),
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PatchError::Stale => write!(f, "stale write: source changed, please re-select"),
            PatchError::OidNotFound(o) => write!(f, "oid {o} not found"),
            PatchError::NoClose(o) => write!(f, "element close tag not found for oid {o}"),
            PatchError::VoidText => write!(f, "cannot text-edit a void element"),
            PatchError::NotLeaf(o) => {
                write!(f, "cannot text-edit oid {o}: it contains child elements")
            }
        }
    }
}

impl std::error::Error for PatchError {}

fn find_entry(map: &[OidEntry], oid: u32) -> Option<&OidEntry> {
    map.iter().find(|e| e.oid == oid)
}

/// 合并 inline style：把 `props`（("color","#fff") …）写进目标 start tag 的 `style` 属性。
///
/// 若目标 tag 已有 `style`，同名属性覆盖、其余保留；否则新增 `style`。
pub fn apply_style_patch(
    source: &str,
    map: &[OidEntry],
    oid: u32,
    props: &[(String, String)],
    expected_hash: Option<&str>,
) -> Result<PatchResult, PatchError> {
    if let Some(h) = expected_hash {
        if body_hash(source) != h {
            return Err(PatchError::Stale);
        }
    }
    let e = find_entry(map, oid).ok_or(PatchError::OidNotFound(oid))?;
    let open = &source[e.open_start..e.open_end]; // "<tag ...>" 或 "<tag ... />"
    let self_closing = open.trim_end().ends_with("/>");

    // 解析现有 style 属性值。
    let mut existing: Vec<(String, String)> = Vec::new();
    if let Some(style_val) = extract_attr(open, "style") {
        for decl in style_val.split(';') {
            let decl = decl.trim();
            if let Some((k, v)) = decl.split_once(':') {
                existing.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
    }
    // 合并（净化：属性名限字母/-；值走安全白名单，非法函数/结构一律拒）。
    for (k, v) in props {
        let key = sanitize_css_ident(k);
        let val = sanitize_css_value(v);
        // 值被白名单拒（空）→ 跳过，绝不用空值覆写既有属性（既是安全也避免误清空）。
        if key.is_empty() || val.is_empty() {
            continue;
        }
        if let Some(slot) = existing.iter_mut().find(|(ek, _)| *ek == key) {
            slot.1 = val;
        } else {
            existing.push((key, val));
        }
    }
    let style_str = existing
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("; ");

    // 重建 open tag：移除旧 style，插入新 style（放到 `>` / `/>` 前）。
    let without_style = remove_attr(open, "style");
    let insert_at = if self_closing {
        without_style.rfind("/>").unwrap_or(without_style.len() - 1)
    } else {
        without_style.rfind('>').unwrap_or(without_style.len() - 1)
    };
    let mut new_open = String::new();
    new_open.push_str(without_style[..insert_at].trim_end());
    new_open.push_str(&format!(" style=\"{style_str}\""));
    if self_closing {
        new_open.push_str(" />");
    } else {
        new_open.push('>');
    }

    let mut new_source = String::with_capacity(source.len() + 32);
    new_source.push_str(&source[..e.open_start]);
    new_source.push_str(&new_open);
    new_source.push_str(&source[e.open_end..]);
    Ok(PatchResult { new_source })
}

/// 属性编辑白名单（B5，红线）：只放行 `href`/`src`/`alt`——绝不允许写任意属性，否则可注入
/// `onclick`/`onerror` 等事件处理器或 `style`（`style` 走 `apply_style_patch` 的 CSS 白名单）。
pub const ALLOWED_ATTRS: &[&str] = &["href", "src", "alt"];

/// HTML 属性值转义（`&`/`<`/`>`/`"` → 实体；属性用双引号包裹故必须转义 `"`）。
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// 属性值净化（B5，红线）：去控制字符 + 危险 scheme（`javascript:`/`vbscript:`/`data:text/html`）
/// 一律拒（返回 `None` = 跳过该属性，绝不写危险值）；`href` 额外拒 `data:`，`src` 只放行
/// `data:image/*`（保产物自包含），`alt` 纯文本。通过后 HTML 属性转义。
pub(crate) fn sanitize_attr_value(attr: &str, value: &str) -> Option<String> {
    let cleaned: String = value.trim().chars().filter(|c| !c.is_control()).collect();
    let lower = cleaned.trim_start().to_ascii_lowercase();
    if lower.starts_with("javascript:")
        || lower.starts_with("vbscript:")
        || lower.starts_with("data:text/html")
    {
        return None;
    }
    match attr {
        "href" => {
            if lower.starts_with("data:") {
                return None; // 链接不放行任何 data: URI
            }
            Some(escape_attr(&cleaned))
        }
        "src" => {
            // http/https/相对路径放行；data: 仅 data:image/*（守自包含 + 挡 data:text/*）。
            if lower.starts_with("data:") && !lower.starts_with("data:image/") {
                return None;
            }
            Some(escape_attr(&cleaned))
        }
        "alt" => Some(escape_attr(&cleaned)),
        _ => None,
    }
}

/// 编辑目标元素的属性（B5：`href`/`src`/`alt`）。逐属性 remove+insert 重建 open tag，
/// 单次 splice 回源。**只放行 `ALLOWED_ATTRS`**（红线）；值经 `sanitize_attr_value`，被拒的
/// 属性跳过（绝不写空 / 危险值）。空字符串值 = 显式清除该属性（alt 常见）。
pub fn apply_attr_patch(
    source: &str,
    map: &[OidEntry],
    oid: u32,
    attrs: &[(String, String)],
    expected_hash: Option<&str>,
) -> Result<PatchResult, PatchError> {
    if let Some(h) = expected_hash {
        if body_hash(source) != h {
            return Err(PatchError::Stale);
        }
    }
    let e = find_entry(map, oid).ok_or(PatchError::OidNotFound(oid))?;
    let open = &source[e.open_start..e.open_end];
    let self_closing = open.trim_end().ends_with("/>");
    let mut tag = open.to_string();
    for (attr, value) in attrs {
        let name = attr.trim().to_ascii_lowercase();
        if !ALLOWED_ATTRS.contains(&name.as_str()) {
            continue; // 红线：越界属性名静默跳过
        }
        let without = remove_attr(&tag, &name);
        // 空值 = 清除属性（remove 后不再插入）。
        if value.trim().is_empty() {
            tag = without;
            continue;
        }
        let Some(safe) = sanitize_attr_value(&name, value) else {
            // 危险 / 被拒值：保留原属性不动（不清除、不写坏值）。
            continue;
        };
        let insert_at = if self_closing {
            without
                .rfind("/>")
                .unwrap_or(without.len().saturating_sub(1))
        } else {
            without
                .rfind('>')
                .unwrap_or(without.len().saturating_sub(1))
        };
        let mut nt = String::with_capacity(without.len() + name.len() + safe.len() + 8);
        nt.push_str(without[..insert_at].trim_end());
        nt.push_str(&format!(" {name}=\"{safe}\""));
        if self_closing {
            nt.push_str(" />");
        } else {
            nt.push('>');
        }
        tag = nt;
    }
    let mut new_source = String::with_capacity(source.len() + 64);
    new_source.push_str(&source[..e.open_start]);
    new_source.push_str(&tag);
    new_source.push_str(&source[e.open_end..]);
    Ok(PatchResult { new_source })
}

/// 替换目标元素内部文本（bridge 只对叶子元素开放；`new_text` 会被 HTML 转义）。
pub fn apply_text_patch(
    source: &str,
    map: &[OidEntry],
    oid: u32,
    new_text: &str,
    expected_hash: Option<&str>,
) -> Result<PatchResult, PatchError> {
    if let Some(h) = expected_hash {
        if body_hash(source) != h {
            return Err(PatchError::Stale);
        }
    }
    let e = find_entry(map, oid).ok_or(PatchError::OidNotFound(oid))?;
    if e.void {
        return Err(PatchError::VoidText);
    }
    let inner_start = e.open_end;
    let inner_end = find_close_start(source, e).ok_or(PatchError::NoClose(oid))?;
    // Leaf-only: refuse to overwrite inner content that contains child elements —
    // that would silently delete the subtree. The inspector bridge only offers text
    // edit on leaves, but the service / HTTP / tool accept any oid, so guard here.
    if inner_has_child_element(&source[inner_start..inner_end]) {
        return Err(PatchError::NotLeaf(oid));
    }
    let escaped = super::renderer::html_escape(new_text);

    let mut new_source = String::with_capacity(source.len());
    new_source.push_str(&source[..inner_start]);
    new_source.push_str(&escaped);
    new_source.push_str(&source[inner_end..]);
    Ok(PatchResult { new_source })
}

/// Whether an element's inner content contains a child element (start / end / decl
/// tag). Used to reject text-patching a container (which would delete its subtree).
fn inner_has_child_element(inner: &str) -> bool {
    let b = inner.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'<' {
            let c = b[i + 1];
            if c.is_ascii_alphabetic() || c == b'/' || c == b'!' {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// 从 open tag 之后按标签深度匹配，找到本元素闭合标签 `</tag>` 的起始字节。
fn find_close_start(source: &str, e: &OidEntry) -> Option<usize> {
    let bytes = source.as_bytes();
    let n = bytes.len();
    let want = e.tag.to_ascii_lowercase();
    let mut depth = 1usize;
    let mut i = e.open_end;
    while i < n {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if source[i..].starts_with("<!--") {
            i = source[i..].find("-->").map(|p| i + p + 3).unwrap_or(n);
            continue;
        }
        let is_close = bytes.get(i + 1) == Some(&b'/');
        let end = find_tag_end(bytes, i)?;
        let tag_str = &source[i..end];
        // 取标签名。
        let name = if is_close {
            tag_str[2..tag_str.len() - 1].trim().to_ascii_lowercase()
        } else {
            let name_end = tag_str[1..]
                .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
                .map(|p| p + 1)
                .unwrap_or(tag_str.len() - 1);
            tag_str[1..name_end].to_ascii_lowercase()
        };
        if is_close {
            if name == want {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        } else if name == want && !tag_str.trim_end().ends_with("/>") && !is_void(&name) {
            depth += 1;
        }
        i = end;
    }
    None
}

fn sanitize_css_ident(s: &str) -> String {
    s.trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>()
        .to_ascii_lowercase()
}

/// 可视化微调可写的 CSS 值里允许出现的函数名（白名单，B0-7）。**不含 `url` / `image-set` /
/// `expression` 等可加载远程资源或执行的向量**，同时放行 calc/var/color/gradient/transform/
/// filter/grid 等全部合法值函数——收紧安全面而不弱化正常调值能力。
const SAFE_CSS_FUNCTIONS: &[&str] = &[
    "calc",
    "min",
    "max",
    "clamp",
    "var",
    "env",
    "rgb",
    "rgba",
    "hsl",
    "hsla",
    "hwb",
    "lab",
    "lch",
    "oklab",
    "oklch",
    "color",
    "color-mix",
    "linear-gradient",
    "radial-gradient",
    "conic-gradient",
    "repeating-linear-gradient",
    "repeating-radial-gradient",
    "repeating-conic-gradient",
    "translate",
    "translatex",
    "translatey",
    "translatez",
    "translate3d",
    "scale",
    "scalex",
    "scaley",
    "scalez",
    "scale3d",
    "rotate",
    "rotatex",
    "rotatey",
    "rotatez",
    "rotate3d",
    "skew",
    "skewx",
    "skewy",
    "matrix",
    "matrix3d",
    "perspective",
    "blur",
    "brightness",
    "contrast",
    "drop-shadow",
    "grayscale",
    "hue-rotate",
    "invert",
    "opacity",
    "saturate",
    "sepia",
    "cubic-bezier",
    "steps",
    "linear",
    "minmax",
    "repeat",
    "fit-content",
    "counter",
    "counters",
    "attr",
    "circle",
    "ellipse",
    "inset",
    "polygon",
    "path",
    "rect",
    "format",
    "local",
];

/// 值里每个 `name(` 函数名必须在白名单内（裸括号分组允许）；有一个越界即整值拒绝。
fn css_functions_allowed(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let b = lower.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c == b'(' {
            let mut j = i;
            while j > 0 && (b[j - 1].is_ascii_alphanumeric() || b[j - 1] == b'-') {
                j -= 1;
            }
            let name = &lower[j..i];
            if !name.is_empty() && !SAFE_CSS_FUNCTIONS.contains(&name) {
                return false;
            }
        }
    }
    true
}

/// 安全 CSS 值净化（B0-7，白名单）：
/// 1. 去结构性字符 `< > " ; { }`（防越出 style 属性 / 注入声明）；
/// 2. 函数白名单——非法函数（url/expression/image-set…）整值拒绝，返回空 = 调用方跳过该声明。
fn sanitize_css_value(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .filter(|c| *c != '<' && *c != '>' && *c != '"' && *c != ';' && *c != '{' && *c != '}')
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || !css_functions_allowed(cleaned) {
        return String::new();
    }
    cleaned.to_string()
}

/// 从 open tag 字符串里取属性值（仅支持双/单引号形式）。
fn extract_attr(open_tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=");
    let pos = find_attr_pos(open_tag, attr)?;
    let after = &open_tag[pos + needle.len()..];
    let after = after.trim_start();
    let quote = after.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &after[1..];
    let endq = rest.find(quote)?;
    Some(rest[..endq].to_string())
}

/// 移除 open tag 里的某属性（含前导空格）。
fn remove_attr(open_tag: &str, attr: &str) -> String {
    let Some(pos) = find_attr_pos(open_tag, attr) else {
        return open_tag.to_string();
    };
    let needle = format!("{attr}=");
    let after = &open_tag[pos + needle.len()..];
    let after_trim = after.trim_start();
    let ws = after.len() - after_trim.len();
    let Some(quote) = after_trim.chars().next() else {
        return open_tag.to_string();
    };
    if quote != '"' && quote != '\'' {
        return open_tag.to_string();
    }
    let rest = &after_trim[1..];
    let Some(endq) = rest.find(quote) else {
        return open_tag.to_string();
    };
    let attr_end = pos + needle.len() + ws + 1 + endq + 1;
    // 连带吃掉属性前的一个空格。
    let mut start = pos;
    if start > 0 && open_tag.as_bytes()[start - 1] == b' ' {
        start -= 1;
    }
    let mut s = String::with_capacity(open_tag.len());
    s.push_str(&open_tag[..start]);
    s.push_str(&open_tag[attr_end..]);
    s
}

/// 找到属性名在 open tag **顶层**（不在引号内）的字节起点；须词首边界（前为空白，避免
/// `data-style` 误命中 `style`）、紧跟 `=`。**引号感知（review 修复 #4）**：扫描时跳过带引号的
/// 属性值，避免值里的 ` name=` 子串（如 `alt="见 src=x"`）被误命中 → 移除失败 + 重复属性 →
/// 编辑被静默丢弃、旧值残留。仍要求 `name=` 紧邻（不含空格；我方渲染器只产此形态）。
fn find_attr_pos(open_tag: &str, attr: &str) -> Option<usize> {
    let bytes = open_tag.as_bytes();
    let alen = attr.len();
    let mut i = 0usize;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == b'"' || c == b'\'' {
            quote = Some(c);
            i += 1;
            continue;
        }
        let boundary = i > 0 && matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r');
        if boundary
            && i + alen < bytes.len()
            && bytes[i..i + alen].eq_ignore_ascii_case(attr.as_bytes())
            && bytes[i + alen] == b'='
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── B0-7: CSS 值白名单硬化 ──────────────────────────────────

    #[test]
    fn css_whitelist_allows_legit_value_functions() {
        for v in [
            "16px",
            "#3b5bdb",
            "calc(100% - 2rem)",
            "var(--ds-color-primary)",
            "rgba(0,0,0,.5)",
            "oklch(0.7 0.1 250)",
            "linear-gradient(90deg, red, blue)",
            "translateX(10px) rotate(4deg)",
            "drop-shadow(0 1px 2px rgba(0,0,0,.3))",
            "clamp(1rem, 2vw, 3rem)",
        ] {
            assert_eq!(sanitize_css_value(v), v, "合法值不该被白名单误拒: {v}");
        }
    }

    #[test]
    fn css_whitelist_rejects_resource_and_exec_vectors() {
        // url() / image-set() / expression() 含不在白名单的函数 → 整值拒绝（返回空）。
        for v in [
            "url(https://evil.example/x.png)",
            "url('data:text/html,<script>')",
            "image-set(url(a.png) 1x)",
            "expression(alert(1))",
            "URL(x)", // 大小写不敏感
        ] {
            assert_eq!(sanitize_css_value(v), "", "危险函数必须被白名单拒: {v}");
        }
    }

    #[test]
    fn css_whitelist_still_strips_structural_chars() {
        // 结构性字符仍被过滤（防越出 style 属性）；过滤后若无非法函数则保留。
        assert_eq!(sanitize_css_value("red\"; color: blue"), "red color: blue");
    }

    #[test]
    fn style_patch_drops_rejected_value_keeping_existing() {
        // 试图用 url() 覆写既有属性 → 被拒，既有值保留、不写空。
        // （map 偏移索引进原始 src，故传 src 而非 annotate 输出——对齐其它 style patch 测试。）
        let src = "<div style=\"background: #fff\">x</div>";
        let (_, map) = annotate(src);
        let r = apply_style_patch(
            src,
            &map,
            0,
            &[("background".into(), "url(https://evil/x.png)".into())],
            None,
        )
        .unwrap();
        assert!(
            r.new_source.contains("background: #fff"),
            "被拒的 url() 不该覆写既有背景: {}",
            r.new_source
        );
        assert!(!r.new_source.contains("url("), "url() 绝不能落进产物源码");
    }

    #[test]
    fn annotate_injects_oids() {
        let src = "<div class=\"a\"><p>hi</p><br></div>";
        let (out, map) = annotate(src);
        assert!(out.contains("data-ds-oid=\"0\""));
        assert!(out.contains("data-ds-oid=\"1\""));
        assert!(out.contains("data-ds-oid=\"2\"")); // br
        assert_eq!(map.len(), 3);
        assert_eq!(map[0].tag, "div");
        assert_eq!(map[1].tag, "p");
        assert!(map[2].void);
        // oid 范围能切回源码 start tag。
        assert_eq!(&src[map[1].open_start..map[1].open_end], "<p>");
    }

    #[test]
    fn annotate_skips_comments() {
        let src = "<!-- <div> --><span>x</span>";
        let (out, map) = annotate(src);
        assert_eq!(map.len(), 1);
        assert_eq!(map[0].tag, "span");
        assert!(out.contains("<!-- <div> -->"));
    }

    #[test]
    fn annotate_skips_raw_text_content() {
        // Regression: `<` inside <script>/<style> raw text must NOT be scanned as tags,
        // else `document.write("<div>")` gets a bogus data-ds-oid and corrupts the script.
        let src = r#"<div>hi</div><script>var s="<div class='x'>";document.write("<span>")</script><p>end</p>"#;
        let (out, map) = annotate(src);
        // Only div, script, p are real elements — the `<div>` / `<span>` inside the
        // script string are NOT counted or annotated.
        assert_eq!(
            map.iter().map(|e| e.tag.as_str()).collect::<Vec<_>>(),
            vec!["div", "script", "p"]
        );
        // Script body copied verbatim, untouched.
        assert!(
            out.contains(r#"var s="<div class='x'>";document.write("<span>")"#),
            "script body must be verbatim: {out}"
        );
        // No oid leaked into the script string.
        assert!(!out.contains("<div class='x' data-ds-oid"));
    }

    #[test]
    fn annotate_skips_style_raw_text() {
        let src = r#"<style>a::before{content:"<b>"}</style><h1>t</h1>"#;
        let (_out, map) = annotate(src);
        assert_eq!(
            map.iter().map(|e| e.tag.as_str()).collect::<Vec<_>>(),
            vec!["style", "h1"]
        );
    }

    #[test]
    fn annotate_preserves_non_ascii_text() {
        // Regression: text nodes must be copied as UTF-8, never `byte as char`
        // (which mojibakes multibyte characters — critical for a Chinese-first app).
        let src = "<h1>你好，世界</h1><p>café • 日本語 🎨</p>";
        let (out, map) = annotate(src);
        assert!(
            out.contains("你好，世界"),
            "Chinese text must survive: {out}"
        );
        assert!(
            out.contains("café • 日本語 🎨"),
            "mixed text must survive: {out}"
        );
        assert_eq!(map.len(), 2);
        // Byte ranges still slice the original start tags correctly.
        assert_eq!(&src[map[0].open_start..map[0].open_end], "<h1>");
        assert_eq!(&src[map[1].open_start..map[1].open_end], "<p>");
        // And a patch located via a multibyte-offset oidmap still lands right.
        let r = apply_style_patch(src, &map, 1, &[("color".into(), "#f00".into())], None).unwrap();
        assert!(r
            .new_source
            .contains("<p style=\"color: #f00\">café • 日本語 🎨</p>"));
    }

    #[test]
    fn style_patch_adds_and_merges() {
        let src = "<div>hi</div>";
        let (_, map) = annotate(src);
        let r = apply_style_patch(src, &map, 0, &[("color".into(), "#f00".into())], None).unwrap();
        assert_eq!(r.new_source, "<div style=\"color: #f00\">hi</div>");

        // 已有 style 合并 + 覆盖。
        let src2 = "<div style=\"color: #000; margin: 4px\">hi</div>";
        let (_, map2) = annotate(src2);
        let r2 = apply_style_patch(
            src2,
            &map2,
            0,
            &[
                ("color".into(), "#f00".into()),
                ("padding".into(), "8px".into()),
            ],
            None,
        )
        .unwrap();
        assert!(r2.new_source.contains("color: #f00"));
        assert!(r2.new_source.contains("margin: 4px"));
        assert!(r2.new_source.contains("padding: 8px"));
        assert!(r2.new_source.contains(">hi</div>"));
    }

    #[test]
    fn text_patch_replaces_leaf_inner() {
        let src = "<h1>old title</h1>";
        let (_, map) = annotate(src);
        let r = apply_text_patch(src, &map, 0, "new & shiny", None).unwrap();
        assert_eq!(r.new_source, "<h1>new &amp; shiny</h1>");
    }

    // ── B5 属性编辑（href/src/alt）+ 安全白名单 ─────────────────────

    #[test]
    fn attr_patch_sets_href() {
        let src = "<a href=\"/old\">go</a>";
        let (_, map) = annotate(src);
        let r = apply_attr_patch(
            src,
            &map,
            0,
            &[("href".into(), "https://x.com".into())],
            None,
        )
        .unwrap();
        assert_eq!(r.new_source, "<a href=\"https://x.com\">go</a>");
    }

    #[test]
    fn attr_patch_sets_img_src_alt() {
        let src = "<img src=\"a.png\" />";
        let (_, map) = annotate(src);
        let r = apply_attr_patch(
            src,
            &map,
            0,
            &[
                ("src".into(), "data:image/png;base64,AAAA".into()),
                ("alt".into(), "a \"quoted\" cat".into()),
            ],
            None,
        )
        .unwrap();
        assert!(r.new_source.contains("src=\"data:image/png;base64,AAAA\""));
        assert!(r.new_source.contains("alt=\"a &quot;quoted&quot; cat\""));
    }

    #[test]
    fn attr_patch_rejects_dangerous_and_offlist() {
        // javascript: href 被拒 → 保留原值不动。
        let src = "<a href=\"/safe\">x</a>";
        let (_, map) = annotate(src);
        let r = apply_attr_patch(
            src,
            &map,
            0,
            &[("href".into(), "javascript:alert(1)".into())],
            None,
        )
        .unwrap();
        assert_eq!(r.new_source, "<a href=\"/safe\">x</a>");

        // href 不放行 data:；src 不放行 data:text/*。
        assert_eq!(sanitize_attr_value("href", "data:text/html,x"), None);
        assert_eq!(sanitize_attr_value("src", "data:text/html,x"), None);
        assert!(sanitize_attr_value("src", "data:image/png;base64,AAAA").is_some());

        // 白名单外属性名（onclick / style）静默跳过，open tag 不变。
        let r2 = apply_attr_patch(
            src,
            &map,
            0,
            &[
                ("onclick".into(), "alert(1)".into()),
                ("style".into(), "color:red".into()),
            ],
            None,
        )
        .unwrap();
        assert_eq!(r2.new_source, src);
    }

    #[test]
    fn attr_patch_empty_value_clears() {
        let src = "<img src=\"a.png\" alt=\"old\" />";
        let (_, map) = annotate(src);
        let r = apply_attr_patch(src, &map, 0, &[("alt".into(), "".into())], None).unwrap();
        assert!(!r.new_source.contains("alt="));
        assert!(r.new_source.contains("src=\"a.png\""));
    }

    #[test]
    fn attr_patch_quote_aware_no_duplicate() {
        // review #4：前一属性的**值**里含 ` src=` 子串，不得误命中导致重复属性 / 编辑丢弃。
        let src = "<img alt=\"see src=old for ref\" src=\"a.png\" />";
        let (_, map) = annotate(src);
        let r = apply_attr_patch(src, &map, 0, &[("src".into(), "b.png".into())], None).unwrap();
        // 真正的 src 被替换，alt 的值原样保留，全程只一个 src= 属性。
        assert!(r.new_source.contains("src=\"b.png\""));
        assert!(r.new_source.contains("alt=\"see src=old for ref\""));
        assert_eq!(
            r.new_source.matches("src=\"").count(),
            1,
            "不得产生重复 src 属性"
        );
    }

    #[test]
    fn text_patch_rejects_container_keeps_leaf() {
        // oid 0 = div (has a child element) → refused, so we never silently delete the
        // subtree. oid 1 = span (leaf) still edits, exercising nested-close matching.
        let src = "<div><span>a</span></div>";
        let (_, map) = annotate(src);
        let err = apply_text_patch(src, &map, 0, "x", None).unwrap_err();
        assert_eq!(err, PatchError::NotLeaf(0));
        let r = apply_text_patch(src, &map, 1, "b", None).unwrap();
        assert_eq!(r.new_source, "<div><span>b</span></div>");
    }

    #[test]
    fn stale_guard_rejects() {
        let src = "<div>hi</div>";
        let (_, map) = annotate(src);
        let err = apply_style_patch(
            src,
            &map,
            0,
            &[("color".into(), "#f00".into())],
            Some("deadbeef"),
        );
        assert!(matches!(err, Err(PatchError::Stale)));
        // 正确 hash 放行。
        let h = body_hash(src);
        assert!(
            apply_style_patch(src, &map, 0, &[("color".into(), "#f00".into())], Some(&h)).is_ok()
        );
    }

    #[test]
    fn style_patch_self_closing() {
        let src = "<img src=\"a.png\" />";
        let (_, map) = annotate(src);
        let r = apply_style_patch(src, &map, 0, &[("width".into(), "20px".into())], None).unwrap();
        assert!(r.new_source.contains("style=\"width: 20px\""));
        assert!(r.new_source.trim_end().ends_with("/>"));
        assert!(r.new_source.contains("src=\"a.png\""));
    }
}
