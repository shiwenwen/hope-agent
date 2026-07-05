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
    // 合并（净化：属性名限字母/-，值去 `<`/`"`）。
    for (k, v) in props {
        let key = sanitize_css_ident(k);
        let val = sanitize_css_value(v);
        if key.is_empty() {
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

fn sanitize_css_value(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '<' && *c != '>' && *c != '"' && *c != ';' && *c != '{' && *c != '}')
        .collect::<String>()
        .trim()
        .to_string()
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

/// 找到属性名在 open tag 里的字节起点（须是单词边界，避免 `data-style` 误匹配 `style`）。
fn find_attr_pos(open_tag: &str, attr: &str) -> Option<usize> {
    let needle = format!("{attr}=");
    let mut from = 0;
    while let Some(rel) = open_tag[from..].find(&needle) {
        let pos = from + rel;
        let ok_before =
            pos == 0 || matches!(open_tag.as_bytes()[pos - 1], b' ' | b'\t' | b'\n' | b'\r');
        if ok_before {
            return Some(pos);
        }
        from = pos + needle.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

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
