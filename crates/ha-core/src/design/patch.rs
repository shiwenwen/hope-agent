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

/// Deck 缩略图轨：给每个 `class` 含 `ds-slide` 词、且尚无 `id` 的 start tag 注入 `id="ds-slide-N"`
/// （文档序）。无 JS 的缩略图 iframe 借 `#ds-slide-N` + `.ds-slide:target{display:block}` 纯 CSS 点亮
/// 该页（主预览走 JS `.active`、URL 无 hash，故 `:target` 不匹配、零副作用）。仅用于 deck kind。
/// 复用 `annotate` 同款字节 tokenizer——注释 / doctype / 结束标签 / script·style raw-text 都安全跳过。
pub fn inject_deck_slide_ids(source: &str) -> String {
    let bytes = source.as_bytes();
    let n = bytes.len();
    let mut out = String::with_capacity(n + 32);
    let mut i = 0usize;
    let mut slide: u32 = 0;
    while i < n {
        if bytes[i] != b'<' {
            let start = i;
            i += 1;
            while i < n && bytes[i] != b'<' {
                i += 1;
            }
            out.push_str(&source[start..i]);
            continue;
        }
        if source[i..].starts_with("<!--") {
            let end = source[i..].find("-->").map(|p| i + p + 3).unwrap_or(n);
            out.push_str(&source[i..end]);
            i = end;
            continue;
        }
        if bytes.get(i + 1) == Some(&b'!') || bytes.get(i + 1) == Some(&b'/') {
            let end = find_tag_end(bytes, i).unwrap_or(n);
            out.push_str(&source[i..end]);
            i = end;
            continue;
        }
        let is_start = matches!(bytes.get(i + 1).copied(), Some(c) if c.is_ascii_alphabetic());
        if !is_start {
            out.push('<');
            i += 1;
            continue;
        }
        let Some(open_end) = find_tag_end(bytes, i) else {
            out.push_str(&source[i..]);
            break;
        };
        let tag_str = &source[i..open_end];
        let name_end = tag_str[1..]
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .map(|p| p + 1)
            .unwrap_or(tag_str.len() - 1);
        let tag = tag_str[1..name_end].to_string();
        let void = is_void(&tag) || tag_str.trim_end().ends_with("/>");
        if tag_has_ds_slide_class(tag_str) && !tag_has_attr(tag_str, "id") {
            out.push_str(&tag_str[..name_end]);
            out.push_str(&format!(" id=\"ds-slide-{slide}\""));
            out.push_str(&tag_str[name_end..]);
            slide += 1;
        } else {
            out.push_str(tag_str);
        }
        // raw-text 元素：内容 CDATA，原样拷贝到闭合标签前（不扫描其中的 `<`）。
        if !void && RAW_TEXT_TAGS.contains(&tag.to_ascii_lowercase().as_str()) {
            let content_end = find_close_ci(bytes, open_end, &tag).unwrap_or(n);
            out.push_str(&source[open_end..content_end]);
            i = content_end;
            continue;
        }
        i = open_end;
    }
    out
}

/// start tag（含 `<`/`>` 的切片）是否有名为 `name` 的属性（前有属性边界空白、后接 `=`）。
fn tag_has_attr(tag_str: &str, name: &str) -> bool {
    let lower = tag_str.to_ascii_lowercase();
    let b = lower.as_bytes();
    let mut from = 0usize;
    while let Some(p) = lower[from..].find(name) {
        let pos = from + p;
        let prev_ok = pos > 0 && matches!(b[pos - 1], b' ' | b'\t' | b'\n' | b'\r' | b'/');
        let after = &lower[pos + name.len()..];
        let eq_ok = after.trim_start().starts_with('=');
        if prev_ok && eq_ok {
            return true;
        }
        from = pos + name.len();
    }
    false
}

/// start tag 的 `class` 属性值是否含 `ds-slide` 词（空白分词、精确匹配）。
fn tag_has_ds_slide_class(tag_str: &str) -> bool {
    let lower = tag_str.to_ascii_lowercase();
    let lb = lower.as_bytes();
    let mut from = 0usize;
    while let Some(p) = lower[from..].find("class") {
        let pos = from + p;
        let prev_ok = pos > 0 && matches!(lb[pos - 1], b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'<');
        // `class` 后（跳空白）须紧接 `=`
        let mut j = pos + 5;
        while j < tag_str.len() && tag_str.as_bytes()[j].is_ascii_whitespace() {
            j += 1;
        }
        if prev_ok && j < tag_str.len() && tag_str.as_bytes()[j] == b'=' {
            j += 1;
            while j < tag_str.len() && tag_str.as_bytes()[j].is_ascii_whitespace() {
                j += 1;
            }
            let (val_start, terminator): (usize, Option<u8>) = match tag_str.as_bytes().get(j) {
                Some(&q @ (b'"' | b'\'')) => (j + 1, Some(q)),
                _ => (j, None),
            };
            let val_end = match terminator {
                Some(q) => tag_str[val_start..]
                    .find(q as char)
                    .map(|k| val_start + k)
                    .unwrap_or(tag_str.len()),
                None => tag_str[val_start..]
                    .find(|c: char| c.is_whitespace() || c == '>')
                    .map(|k| val_start + k)
                    .unwrap_or(tag_str.len()),
            };
            if tag_str[val_start..val_end]
                .split_whitespace()
                .any(|t| t == "ds-slide")
            {
                return true;
            }
        }
        from = pos + 5;
    }
    false
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
    /// 直属文本节点 patch：给定的子节点下标越界，或该下标不是文本节点。
    TextNodeNotFound(u32, usize),
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
            PatchError::TextNodeNotFound(o, idx) => {
                write!(f, "oid {o} child node {idx} is not an editable text node")
            }
        }
    }
}

/// 被删元素的重建上下文（结构 undo）：从**源码**（body.html，无 `data-ds-oid`，干净）捕获，
/// 供 `apply_insert_patch` 原样重新插回。锚点用 oid（`after_oid` 前一个元素兄弟 / `parent_oid`
/// 最近祖先），二者都在删除点**之前**，删后文档序 oid 不漂移故稳定。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemovedElement {
    /// 被删元素在源码里的完整切片（含内部内容与闭合）。空白前导 gap 时含 gap、文本 gap 时不含。
    pub html: String,
    /// 最近祖先元素 oid（`None` = 顶层元素，无 body 包裹）。
    pub parent_oid: Option<u32>,
    /// 同层前一个元素兄弟 oid（`None` = 是首个子元素）。重插锚点，优先于 `parent_oid`。
    pub after_oid: Option<u32>,
    /// 重插时从锚点（前兄弟 end / 父 open_end / 0）再前进的字节数——跳过**保留在原地的前导文本
    /// gap**。空白 gap 已并入 `html` 删掉故为 0；文本 gap 不吞（只删元素本体）、留在源里，重插须
    /// 落到它之后才字节精确。
    #[serde(default)]
    pub insert_offset: usize,
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

/// oid 元素的**完整字节范围** `[open_start, end)`：void / 自闭合元素 = start tag 本身；否则复用
/// `find_close_start`（平衡 + 引号 + 注释 + raw-text 感知，与 annotate/text-edit 同口径）定位匹配
/// `</tag>` 的起点，再经 `find_tag_end` 取其 `>` 之后。找不到闭合返回 `None`。
fn element_full_range(source: &str, e: &OidEntry) -> Option<(usize, usize)> {
    let open = &source[e.open_start..e.open_end];
    if e.void || is_void(&e.tag) || open.trim_end().ends_with("/>") {
        return Some((e.open_start, e.open_end));
    }
    let close_start = find_close_start(source, e)?;
    let close_end = find_tag_end(source.as_bytes(), close_start).unwrap_or(source.len());
    Some((e.open_start, close_end))
}

/// 删除元素（Wave 3-⑫）：把 oid 元素整段（含内部内容与闭合）从源码剔除。**平衡扫描**正确处理
/// 嵌套同名元素、引号内 / 注释内 / raw-text 内容里的假标签。「最后一个可见元素」保护在
/// `service::patch_element` 层（删后 body 为空则拒）。
pub fn apply_remove_patch(
    source: &str,
    map: &[OidEntry],
    oid: u32,
    expected_hash: Option<&str>,
) -> Result<PatchResult, PatchError> {
    if let Some(h) = expected_hash {
        if body_hash(source) != h {
            return Err(PatchError::Stale);
        }
    }
    let e = find_entry(map, oid).ok_or(PatchError::OidNotFound(oid))?;
    let (start, end) = element_full_range(source, e).ok_or(PatchError::NoClose(oid))?;
    let mut new_source = String::with_capacity(source.len());
    new_source.push_str(&source[..start]);
    new_source.push_str(&source[end..]);
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

/// 从 start tag 字符串 `<name ...>` 取小写标签名。
fn parse_tag_name(tag_str: &str) -> String {
    let name_end = tag_str[1..]
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .map(|p| p + 1)
        .unwrap_or(tag_str.len().saturating_sub(1));
    tag_str[1..name_end].to_ascii_lowercase()
}

/// 删元素并捕获重建上下文（结构 undo，owner-only）。**锚点即删除起点**是字节精确还原的关键：
/// 元素与其前一个兄弟（或父 open tag）之间的空白是「无主」的，若只删 `[start,end)` 会把这段
/// 空白留在原地、撤销时重插又补一份→空白翻倍。故删除范围取 `[anchor, end)`（含前导空白）、捕获的
/// `html` 也含它、重插又落在同一 anchor，`source[..anchor] + html + source[anchor..]` 逐字节还原。
///
/// 锚点计算全在**删除点之前**故删后文档序 oid 稳定：`parent` = 最近祖先（范围严格包含目标、
/// `open_start` 最大者）；`after` = 同层前一个元素兄弟（`full_range.end <= start` 且
/// `open_start >= 父内容起点` 中 `end` 最大者——兄弟闭合是目标前最后一个 token，后代 / 更早兄弟
/// 的 end 都更小）。`anchor` = 有兄弟则兄弟 end，否则父内容起点（`open_end`），否则 0（顶层首元素）。
pub fn remove_element_with_context(
    source: &str,
    map: &[OidEntry],
    oid: u32,
    expected_hash: Option<&str>,
) -> Result<(PatchResult, RemovedElement), PatchError> {
    if let Some(h) = expected_hash {
        if body_hash(source) != h {
            return Err(PatchError::Stale);
        }
    }
    let e = find_entry(map, oid).ok_or(PatchError::OidNotFound(oid))?;
    let (start, end) = element_full_range(source, e).ok_or(PatchError::NoClose(oid))?;

    // 最近祖先：范围严格包含 [start,end) 的元素里 open_start 最大者。
    let mut parent: Option<&OidEntry> = None;
    let mut parent_open = 0usize;
    for e2 in map {
        if e2.oid == oid {
            continue;
        }
        if let Some((s2, en2)) = element_full_range(source, e2) {
            if s2 < start && en2 >= end && (parent.is_none() || s2 > parent_open) {
                parent = Some(e2);
                parent_open = s2;
            }
        }
    }
    let parent_content_start = parent.map_or(0, |p| p.open_end);
    let parent_oid = parent.map(|p| p.oid);

    // 前一个元素兄弟：full_range.end <= start 且 open_start >= 父内容起点 中 end 最大者。
    let mut after_oid: Option<u32> = None;
    let mut after_end = 0usize;
    for e2 in map {
        if e2.oid == oid {
            continue;
        }
        if let Some((s2, en2)) = element_full_range(source, e2) {
            if en2 <= start && s2 >= parent_content_start && (after_oid.is_none() || en2 > after_end)
            {
                after_oid = Some(e2.oid);
                after_end = en2;
            }
        }
    }
    let candidate = if after_oid.is_some() {
        after_end
    } else {
        parent_content_start
    };
    // 前导 gap = [candidate, start)。**只有全空白**才并入删除范围（源码整洁 + 撤销字节精确）；
    // 含非空白（如 inline 布局 `<span>图标</span> 标签 <b>…</b>` 里的「 标签 」）则**只删元素本体**、
    // gap 留在原地——绝不静默吞掉相邻裸文本（review MEDIUM）。重插锚点用 insert_offset 跳过留下的 gap。
    let gap = &source[candidate..start];
    let (remove_start, insert_offset) = if gap.trim().is_empty() {
        (candidate, 0usize)
    } else {
        (start, gap.len())
    };

    let html = source[remove_start..end].to_string();
    let mut new_source = String::with_capacity(source.len());
    new_source.push_str(&source[..remove_start]);
    new_source.push_str(&source[end..]);
    Ok((
        PatchResult { new_source },
        RemovedElement {
            html,
            parent_oid,
            after_oid,
            insert_offset,
        },
    ))
}

/// 结构 undo 的重插：把 `html`（`removed_element_context` 捕获的干净源码切片）原样插回锚点位置。
/// **owner-only**（绝不进 agent `edit_element`）——`html` 是原样字节，不经 CSS/attr 白名单净化，
/// 只因它来自产物自身此前的源码、且经 `expected_hash` stale 守卫防串改。定位优先级：
/// `after_oid`（插到该兄弟完整范围之后）> `parent_oid`（插为首子、紧跟父 open tag）> 源码起点。
pub fn apply_insert_patch(
    source: &str,
    map: &[OidEntry],
    parent_oid: Option<u32>,
    after_oid: Option<u32>,
    insert_offset: usize,
    html: &str,
    expected_hash: Option<&str>,
) -> Result<PatchResult, PatchError> {
    if let Some(h) = expected_hash {
        if body_hash(source) != h {
            return Err(PatchError::Stale);
        }
    }
    let anchor = if let Some(aid) = after_oid {
        let e = find_entry(map, aid).ok_or(PatchError::OidNotFound(aid))?;
        element_full_range(source, e).ok_or(PatchError::NoClose(aid))?.1
    } else if let Some(pid) = parent_oid {
        find_entry(map, pid).ok_or(PatchError::OidNotFound(pid))?.open_end
    } else {
        0
    };
    // 跳过删除时留在原地的前导文本 gap（insert_offset 字节），落到它之后才字节精确。钳到源码长度防越界。
    let pos = (anchor + insert_offset).min(source.len());
    let mut new_source = String::with_capacity(source.len() + html.len());
    new_source.push_str(&source[..pos]);
    new_source.push_str(html);
    new_source.push_str(&source[pos..]);
    Ok(PatchResult { new_source })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ChildKind {
    Text,
    Element,
    Comment,
}

/// 直属子节点（相对源码的绝对字节范围）。用于 span 文本节点编辑：与 DOM `element.childNodes`
/// 下标一一对应（文本 run / 子元素 / 注释各算一个节点，空白 run 也算一个文本节点，元素间无文本
/// 则不产空节点——对齐 DOM 语义）。
#[derive(Debug, Clone, Copy)]
struct ChildNode {
    kind: ChildKind,
    start: usize,
    end: usize,
}

/// 把元素内部内容 `[inner_start, inner_end)` 按深度 0 切成子节点序列。子元素范围复用
/// `element_full_range`（平衡 / 引号 / 注释 / raw-text 感知），内部内容由 `find_close_start`
/// 保证平衡故不会在深度 0 出现游离闭合标签。
fn direct_child_nodes(source: &str, inner_start: usize, inner_end: usize) -> Vec<ChildNode> {
    let bytes = source.as_bytes();
    let mut nodes = Vec::new();
    let mut i = inner_start;
    let mut text_start = inner_start;
    while i < inner_end {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if i > text_start {
            nodes.push(ChildNode {
                kind: ChildKind::Text,
                start: text_start,
                end: i,
            });
        }
        if source[i..].starts_with("<!--") {
            let end = source[i..inner_end]
                .find("-->")
                .map(|p| i + p + 3)
                .unwrap_or(inner_end);
            nodes.push(ChildNode {
                kind: ChildKind::Comment,
                start: i,
                end,
            });
            i = end;
            text_start = i;
            continue;
        }
        let tag_end = find_tag_end(bytes, i).unwrap_or(inner_end).min(inner_end);
        let tag_str = &source[i..tag_end];
        let name = parse_tag_name(tag_str);
        let self_closing = tag_str.trim_end().ends_with("/>");
        let end = if self_closing || is_void(&name) {
            tag_end
        } else {
            let fake = OidEntry {
                oid: 0,
                tag: name,
                open_start: i,
                open_end: tag_end,
                void: false,
            };
            element_full_range(source, &fake)
                .map(|(_, e)| e)
                .unwrap_or(tag_end)
                .min(inner_end)
        };
        nodes.push(ChildNode {
            kind: ChildKind::Element,
            start: i,
            end,
        });
        i = end;
        text_start = i;
    }
    if text_start < inner_end {
        nodes.push(ChildNode {
            kind: ChildKind::Text,
            start: text_start,
            end: inner_end,
        });
    }
    nodes
}

/// 编辑**非叶子**元素的某个直属文本节点（决策4A：只改 `<h1>Big <span>x</span></h1>` 里的「Big 」，
/// 保留内部 `<span>` 子树）。`node_index` = DOM `element.childNodes` 下标（bridge 侧枚举同款）。
/// 该下标必须落在文本节点上，否则 `TextNodeNotFound`。`new_text` HTML 转义后回写。
pub fn apply_text_node_patch(
    source: &str,
    map: &[OidEntry],
    oid: u32,
    node_index: usize,
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
    let nodes = direct_child_nodes(source, inner_start, inner_end);
    let node = nodes
        .get(node_index)
        .filter(|n| n.kind == ChildKind::Text)
        .ok_or(PatchError::TextNodeNotFound(oid, node_index))?;
    let escaped = super::renderer::html_escape(new_text);
    let mut new_source = String::with_capacity(source.len());
    new_source.push_str(&source[..node.start]);
    new_source.push_str(&escaped);
    new_source.push_str(&source[node.end..]);
    Ok(PatchResult { new_source })
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
        } else if RAW_TEXT_TAGS.contains(&name.as_str()) && !tag_str.trim_end().ends_with("/>") {
            // raw-text 元素（script/style/textarea/title）内容是 CDATA——其中的 `<div>` / `</div>`
            // 不是标签，必须整段跳过，否则删除容器时被内嵌脚本里的 `</tag>` 字符串误判提前收尾、
            // 剪出错乱源码（review HIGH/MED）。与 annotate 同口径。
            if let Some(close) = find_close_ci(bytes, end, &name) {
                i = find_tag_end(bytes, close).unwrap_or(n);
                continue;
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
    fn remove_patch_balanced_nested() {
        // 嵌套同名 div：删外层必须连同内层与其闭合一起剔除（平衡扫描，不被内层 </div> 骗停）。
        let src = "<section><div><div>x</div></div><p>keep</p></section>";
        let (_, map) = annotate(src);
        // oid: 0=section,1=outer div,2=inner div,3=p
        let outer = map.iter().find(|e| e.oid == 1).unwrap();
        assert_eq!(outer.tag, "div");
        let r = apply_remove_patch(src, &map, 1, None).unwrap();
        assert_eq!(r.new_source, "<section><p>keep</p></section>");
    }

    #[test]
    fn remove_patch_ignores_fake_close_in_attr() {
        // 属性值里的 </div> 不该被当作闭合（find_tag_end 引号感知）。
        let src = "<section><div><img alt=\"a </div> b\"><span>x</span></div><p>keep</p></section>";
        let (_, map) = annotate(src);
        let div = map.iter().find(|e| e.tag == "div").unwrap();
        let r = apply_remove_patch(src, &map, div.oid, None).unwrap();
        assert_eq!(r.new_source, "<section><p>keep</p></section>");
    }

    #[test]
    fn remove_patch_ignores_fake_close_in_script() {
        // raw-text（script）内容里的 </div> 不该被当作闭合。
        let src =
            "<section><div><script>var a=\"</div>\";</script><h1>Hi</h1></div><p>k</p></section>";
        let (_, map) = annotate(src);
        let div = map.iter().find(|e| e.tag == "div").unwrap();
        let r = apply_remove_patch(src, &map, div.oid, None).unwrap();
        assert_eq!(r.new_source, "<section><p>k</p></section>");
    }

    #[test]
    fn remove_patch_void_element() {
        let src = "<div><img src=\"a.png\"><span>t</span></div>";
        let (_, map) = annotate(src);
        let img = map.iter().find(|e| e.tag == "img").unwrap();
        let r = apply_remove_patch(src, &map, img.oid, None).unwrap();
        assert_eq!(r.new_source, "<div><span>t</span></div>");
    }

    #[test]
    fn remove_patch_stale_guard() {
        let src = "<div><p>a</p></div>";
        let (_, map) = annotate(src);
        assert!(matches!(
            apply_remove_patch(src, &map, 0, Some("wronghash")),
            Err(PatchError::Stale)
        ));
    }

    // ── 结构 undo：removed_element_context + apply_insert_patch（删除可撤销）──────

    /// 删 oid 元素 → 撤销（重插）应逐字节还原；再重做（同 anchor 语义重删）→ 再撤销仍字节精确
    /// （守 redo 与 delete 对称的 byte-exact 红线，review HIGH）。
    fn assert_delete_undo_roundtrip(src: &str) {
        let (_, map) = annotate(src);
        for e in &map {
            let (removed_patch, removed) =
                remove_element_with_context(src, &map, e.oid, None).unwrap();
            let after_del = removed_patch.new_source;
            let undo1 = reinsert(&after_del, &removed);
            assert_eq!(undo1, src, "删 oid {} 再撤销未还原", e.oid);
            // 重做 = 在还原后的源上按同一 owner 语义重删（对齐前端 redo 走 remove_design_element_cmd）；
            // 重删产出应与首删一致，其 removed context 再撤销仍字节精确。
            let (_, map_undo) = annotate(&undo1);
            let (redo_patch, removed2) =
                remove_element_with_context(&undo1, &map_undo, e.oid, None).unwrap();
            assert_eq!(redo_patch.new_source, after_del, "重做删 oid {} 与首删不一致", e.oid);
            let undo2 = reinsert(&redo_patch.new_source, &removed2);
            assert_eq!(undo2, src, "删 oid {} 重做后再撤销未还原（byte-exact 红线）", e.oid);
        }
    }

    /// 在删后的源上按 removed context 重插（模拟真实 undo：源已变、oid 已重排）。
    fn reinsert(after_del: &str, removed: &RemovedElement) -> String {
        let (_, map2) = annotate(after_del);
        apply_insert_patch(
            after_del,
            &map2,
            removed.parent_oid,
            removed.after_oid,
            removed.insert_offset,
            &removed.html,
            None,
        )
        .expect("insert failed")
        .new_source
    }

    #[test]
    fn delete_undo_roundtrip_simple() {
        assert_delete_undo_roundtrip("<div><p>a</p><p>b</p><p>c</p></div>");
    }

    #[test]
    fn delete_undo_roundtrip_nested_and_first_child() {
        assert_delete_undo_roundtrip("<section><h1>T</h1><div><span>x</span><b>y</b></div></section>");
    }

    #[test]
    fn delete_undo_roundtrip_top_level_and_void() {
        assert_delete_undo_roundtrip("<h1>Title</h1><img src=\"a.png\"><p>body</p>");
    }

    #[test]
    fn delete_undo_roundtrip_whitespace_between_siblings() {
        assert_delete_undo_roundtrip("<ul>\n  <li>1</li>\n  <li>2</li>\n</ul>");
    }

    #[test]
    fn delete_undo_roundtrip_text_gap_between_siblings() {
        // 兄弟间有**非空白文本**（inline 布局）——删元素不吞相邻文本、undo/redo 仍字节精确。
        assert_delete_undo_roundtrip("<p><b>x</b> 价格说明 <i>y</i></p>");
        assert_delete_undo_roundtrip("<li><span>icon</span> 重要备注 <p>正文</p></li>");
    }

    #[test]
    fn delete_text_gap_keeps_adjacent_text() {
        // 删 <i>y</i>：其前有文本 gap「 价格说明 」——只删元素本体，绝不吞掉相邻裸文本。
        let src = "<p><b>x</b> 价格说明 <i>y</i></p>";
        let (_, map) = annotate(src);
        let i = map.iter().find(|e| e.tag == "i").unwrap();
        let (r, removed) = remove_element_with_context(src, &map, i.oid, None).unwrap();
        assert_eq!(r.new_source, "<p><b>x</b> 价格说明 </p>", "相邻文本被误删");
        assert!(!removed.html.contains("价格说明"), "html 不应吞相邻文本");
        assert!(removed.insert_offset > 0, "文本 gap 应有 insert_offset");
    }

    #[test]
    fn insert_patch_stale_guard() {
        let src = "<div><p>a</p></div>";
        let (_, map) = annotate(src);
        assert!(matches!(
            apply_insert_patch(src, &map, Some(0), None, 0, "<b>x</b>", Some("wronghash")),
            Err(PatchError::Stale)
        ));
    }

    // ── span 直属文本节点编辑（决策4A：改裸文本、保留内部 span）──────

    #[test]
    fn text_node_patch_edits_leading_bare_text_keeps_span() {
        // childNodes: [text "Big ", <span>Title</span>] → 改 index 0 的「Big 」。
        let src = "<h1>Big <span>Title</span></h1>";
        let (_, map) = annotate(src);
        let h1 = map.iter().find(|e| e.tag == "h1").unwrap();
        let r = apply_text_node_patch(src, &map, h1.oid, 0, "Huge ", None).unwrap();
        assert_eq!(r.new_source, "<h1>Huge <span>Title</span></h1>");
    }

    #[test]
    fn text_node_patch_edits_trailing_bare_text() {
        // childNodes: [<span>Title</span> (0), text " End" (1)] → 改 index 1。
        let src = "<h1><span>Title</span> End</h1>";
        let (_, map) = annotate(src);
        let h1 = map.iter().find(|e| e.tag == "h1").unwrap();
        let r = apply_text_node_patch(src, &map, h1.oid, 1, " Fin", None).unwrap();
        assert_eq!(r.new_source, "<h1><span>Title</span> Fin</h1>");
    }

    #[test]
    fn text_node_patch_rejects_element_index() {
        // index 1 落在 <span> 元素上，不是文本节点 → 拒。
        let src = "<h1>Big <span>Title</span></h1>";
        let (_, map) = annotate(src);
        let h1 = map.iter().find(|e| e.tag == "h1").unwrap();
        assert!(matches!(
            apply_text_node_patch(src, &map, h1.oid, 1, "x", None),
            Err(PatchError::TextNodeNotFound(_, 1))
        ));
    }

    #[test]
    fn text_node_patch_escapes_and_guards_stale() {
        let src = "<p>hi <b>x</b></p>";
        let (_, map) = annotate(src);
        let p = map.iter().find(|e| e.tag == "p").unwrap();
        // HTML 转义（文本节点 "hi "（含尾空格）整体被替换，故新文本自带尾空格才留空格）。
        let r = apply_text_node_patch(src, &map, p.oid, 0, "a<b>& ", None).unwrap();
        assert_eq!(r.new_source, "<p>a&lt;b&gt;&amp; <b>x</b></p>");
        // stale 守卫。
        assert!(matches!(
            apply_text_node_patch(src, &map, p.oid, 0, "z", Some("nope")),
            Err(PatchError::Stale)
        ));
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
    fn inject_deck_slide_ids_numbers_in_doc_order() {
        let src = "<section class=\"ds-slide\"><h1>A</h1></section>\
                   <section class=\"ds-slide active\"><h1>B</h1></section>";
        let out = inject_deck_slide_ids(src);
        assert!(out.contains("id=\"ds-slide-0\""));
        assert!(out.contains("id=\"ds-slide-1\""));
        // 非 .ds-slide 元素不注入。
        assert_eq!(out.matches("ds-slide-").count(), 2);
        // active class 保留。
        assert!(out.contains("class=\"ds-slide active\""));
    }

    #[test]
    fn inject_deck_slide_ids_skips_existing_id_and_non_slide() {
        let src = "<section id=\"keep\" class=\"ds-slide\">x</section>\
                   <div class=\"card\">y</div>";
        let out = inject_deck_slide_ids(src);
        // 已有 id 的 .ds-slide 不重复注入；非 slide 不注入。
        assert!(!out.contains("ds-slide-0"));
        assert!(out.contains("id=\"keep\""));
    }

    #[test]
    fn inject_deck_slide_ids_safe_in_script_rawtext() {
        // script 里的伪 `<section class="ds-slide">` 是字符串、不是真元素，不得注入。
        let src = "<script>var s='<section class=\"ds-slide\">'</script>\
                   <section class=\"ds-slide\">real</section>";
        let out = inject_deck_slide_ids(src);
        assert_eq!(
            out.matches("ds-slide-").count(),
            1,
            "only the real slide gets an id"
        );
        assert!(out.contains("id=\"ds-slide-0\""));
        // script 内容原样保留。
        assert!(out.contains("var s='<section class=\"ds-slide\">'"));
    }

    #[test]
    fn inject_deck_slide_ids_not_fooled_by_substring_class() {
        // class 值含 `ds-slide` 子串但非独立词（`ds-slide-wrap`）不匹配。
        let src = "<div class=\"ds-slide-wrap\">x</div>";
        let out = inject_deck_slide_ids(src);
        assert!(!out.contains("ds-slide-0"));
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
