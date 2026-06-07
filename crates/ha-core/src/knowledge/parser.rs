//! Markdown + wikilink parsing for the knowledge index.
//!
//! Walks standard Markdown with `pulldown-cmark` to find code spans/blocks and
//! headings, then scans the raw text for `[[wikilinks]]` and `#tags`, **skipping
//! anything inside code** (fenced / indented / inline). All positions follow the
//! D14 coordinate contract: code-point offsets + 1-based line / 0-based
//! code-point column, computed relative to the *original full file* (frontmatter
//! and original CRLF included) via [`PosMap`].

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::ops::Range;
use unicode_normalization::UnicodeNormalization;

use super::types::LinkType;

/// A position in the file under the D14 coordinate contract.
#[derive(Debug, Clone, Copy)]
pub struct Pos {
    /// Unicode code-point offset from start of file.
    pub offset: u32,
    /// 1-based line number (split on `\n`).
    pub line: u32,
    /// 0-based code-point column within the line (tab counts as one).
    pub col: u32,
}

/// Byte-offset → (code-point offset, line, col) mapper over the full file.
pub struct PosMap<'a> {
    text: &'a str,
    /// Byte offset of each line start (line 0 starts at byte 0).
    line_starts: Vec<usize>,
    /// Code-point count at each line start.
    cp_at_line_start: Vec<usize>,
}

impl<'a> PosMap<'a> {
    pub fn new(text: &'a str) -> Self {
        let mut line_starts = vec![0usize];
        let mut cp_at_line_start = vec![0usize];
        let mut cp = 0usize;
        for (i, ch) in text.char_indices() {
            cp += 1;
            if ch == '\n' {
                line_starts.push(i + ch.len_utf8());
                cp_at_line_start.push(cp);
            }
        }
        Self {
            text,
            line_starts,
            cp_at_line_start,
        }
    }

    /// Map a byte offset (must be a char boundary) to a [`Pos`].
    pub fn pos(&self, byte_off: usize) -> Pos {
        let byte_off = byte_off.min(self.text.len());
        let line_idx = match self.line_starts.binary_search(&byte_off) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let line_start = self.line_starts[line_idx];
        // `\r` only appears at a line's end (before `\n`), so counting code
        // points from the line start to a mid-line offset never includes it.
        let col = self.text[line_start..byte_off].chars().count();
        Pos {
            offset: (self.cp_at_line_start[line_idx] + col) as u32,
            line: (line_idx + 1) as u32,
            col: col as u32,
        }
    }
}

/// A heading occurrence with its level and start position.
#[derive(Debug, Clone)]
pub struct Heading {
    pub level: u32,
    pub title: String,
    pub byte_start: usize,
}

/// An Obsidian-style `^block-id` anchor and the block it terminates (Phase 3 G).
///
/// A block anchor is a `^id` token (`[A-Za-z0-9-]+`) that is the trailing content
/// of a line, outside code. The *referenced block* is the leaf block the anchor
/// terminates (a trailing `text ^id`) or — when the anchor sits alone on its own
/// line — the nearest preceding leaf block. The block `text` has the anchor token
/// stripped, so `![[Note#^id]]` transclusion shows the block content alone.
#[derive(Debug, Clone)]
pub struct ParsedBlock {
    /// The identifier after `^` (letters / digits / dashes).
    pub block_id: String,
    /// Block start position (D14) — first non-blank char of the block.
    pub start: Pos,
    /// Block end position (D14) — end of block content, anchor token excluded.
    pub end: Pos,
    /// Block source text with the trailing `^id` token removed, trimmed.
    pub text: String,
}

/// A wikilink occurrence with parsed components and source position.
#[derive(Debug, Clone)]
pub struct ParsedLink {
    pub target_ref: String,
    pub anchor: Option<String>,
    pub alias: Option<String>,
    pub link_type: LinkType,
    pub raw_text: String,
    pub start: Pos,
    pub end: Pos,
    pub heading_path: Option<String>,
}

/// Result of parsing a note file.
pub struct ParsedDoc {
    /// frontmatter `title` > first H1 > (filled by caller from file stem).
    pub title: Option<String>,
    /// frontmatter parsed to JSON (opaque), `None` if absent / unparseable.
    pub frontmatter_json: Option<String>,
    /// Byte offset where the body begins (after a frontmatter block, else 0).
    pub body_start_byte: usize,
    pub headings: Vec<Heading>,
    pub links: Vec<ParsedLink>,
    /// Obsidian `^block-id` anchors, in document order (first id wins on dup).
    pub blocks: Vec<ParsedBlock>,
    /// Normalized (NFC + lowercase), de-duplicated tags.
    pub tags: Vec<String>,
}

/// Parse a note file's full raw content.
pub fn parse_document(full: &str) -> ParsedDoc {
    let posmap = PosMap::new(full);

    // 1. Frontmatter envelope: leading `---\n ... \n---`.
    let (frontmatter_raw, body_start_byte) = split_frontmatter(full);
    let frontmatter = frontmatter_raw.map(parse_frontmatter_to_json);
    let frontmatter_json = frontmatter
        .as_ref()
        .and_then(|v| serde_json::to_string(v).ok());

    let body = &full[body_start_byte..];

    // 2. Walk markdown structure (over the body slice, so YAML frontmatter never
    //    confuses pulldown-cmark). All ranges are body-relative → add
    //    `body_start_byte` to map back to full-file coordinates.
    let mut code_ranges: Vec<Range<usize>> = Vec::new();
    let mut headings: Vec<Heading> = Vec::new();
    let mut first_h1: Option<String> = None;
    // Leaf-block source spans (full-file coords) a `^block-id` anchor can attach
    // to: paragraphs, list items, headings. `into_offset_iter` pairs a `Start`
    // event with the whole element's range.
    let mut leaf_spans: Vec<Range<usize>> = Vec::new();

    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let mut iter = Parser::new_ext(body, opts).into_offset_iter();

    let mut cur_heading: Option<(u32, usize, String)> = None; // (level, byte_start, accum text)
    let mut code_block_start: Option<usize> = None;

    while let Some((ev, range)) = iter.next() {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                leaf_spans.push((body_start_byte + range.start)..(body_start_byte + range.end));
                cur_heading = Some((
                    heading_level(level),
                    body_start_byte + range.start,
                    String::new(),
                ));
            }
            Event::Start(Tag::Paragraph) | Event::Start(Tag::Item) => {
                leaf_spans.push((body_start_byte + range.start)..(body_start_byte + range.end));
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, byte_start, title)) = cur_heading.take() {
                    let title = title.trim().to_string();
                    if level == 1 && first_h1.is_none() && !title.is_empty() {
                        first_h1 = Some(title.clone());
                    }
                    headings.push(Heading {
                        level,
                        title,
                        byte_start,
                    });
                }
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_)))
            | Event::Start(Tag::CodeBlock(CodeBlockKind::Indented)) => {
                code_block_start = Some(body_start_byte + range.start);
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(start) = code_block_start.take() {
                    code_ranges.push(start..(body_start_byte + range.end));
                }
            }
            Event::Code(_) => {
                code_ranges.push((body_start_byte + range.start)..(body_start_byte + range.end));
            }
            Event::Text(t) => {
                if let Some((_, _, ref mut accum)) = cur_heading {
                    accum.push_str(&t);
                }
            }
            _ => {}
        }
    }

    // 3. Scan the full file for wikilinks + tags, skipping code ranges. We scan
    //    over `full` (not `body`) but ignore anything before `body_start_byte`
    //    via the frontmatter cut, since links/tags in frontmatter aren't body
    //    references. Coordinates stay full-file.
    let links = scan_wikilinks(full, body_start_byte, &code_ranges, &headings, &posmap);
    let blocks = scan_blocks(full, body_start_byte, &code_ranges, &leaf_spans, &posmap);
    let mut tags = scan_tags(full, body_start_byte, &code_ranges);

    // Merge frontmatter tags.
    if let Some(fm) = &frontmatter {
        collect_frontmatter_tags(fm, &mut tags);
    }
    tags.sort();
    tags.dedup();

    let title = frontmatter
        .as_ref()
        .and_then(|v| v.get("title"))
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or(first_h1);

    ParsedDoc {
        title,
        frontmatter_json,
        body_start_byte,
        headings,
        links,
        blocks,
        tags,
    }
}

fn heading_level(level: HeadingLevel) -> u32 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Build the `a > b > c` heading path active at a byte offset, by walking the
/// headings before it and keeping the level hierarchy.
pub fn heading_path_at(byte_off: usize, headings: &[Heading]) -> Option<String> {
    let mut stack: Vec<&Heading> = Vec::new();
    for h in headings {
        if h.byte_start > byte_off {
            break;
        }
        while let Some(last) = stack.last() {
            if last.level >= h.level {
                stack.pop();
            } else {
                break;
            }
        }
        stack.push(h);
    }
    if stack.is_empty() {
        None
    } else {
        Some(
            stack
                .iter()
                .map(|h| h.title.as_str())
                .collect::<Vec<_>>()
                .join(" > "),
        )
    }
}

/// Detect a leading YAML frontmatter block. Returns `(Some(yaml), body_start)`
/// or `(None, 0)`. Mirrors the hand-rolled detection in `skills/frontmatter.rs`.
fn split_frontmatter(full: &str) -> (Option<&str>, usize) {
    // Allow an optional UTF-8 BOM.
    let start = full.strip_prefix('\u{feff}').map(|_| 3).unwrap_or(0);
    let rest = &full[start..];
    if !(rest.starts_with("---\n") || rest.starts_with("---\r\n")) {
        return (None, 0);
    }
    let after_open = if rest.starts_with("---\r\n") {
        start + 5
    } else {
        start + 4
    };
    // Closing fence: a line that is exactly `---` (with optional CR).
    let hay = &full[after_open..];
    let mut idx = 0usize;
    for line in hay.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" {
            let yaml = &full[after_open..after_open + idx];
            let body_start = after_open + idx + line.len();
            return (Some(yaml), body_start);
        }
        idx += line.len();
    }
    (None, 0)
}

/// Minimal YAML→JSON for frontmatter. Handles `key: scalar`, `key: [a, b]`, and
/// block lists (`key:` then `- item` lines). Sufficient for title + tags; not a
/// full YAML implementation (project convention: no YAML crate).
fn parse_frontmatter_to_json(yaml: &str) -> serde_json::Value {
    use serde_json::{Map, Value};
    let mut map = Map::new();
    let lines: Vec<&str> = yaml.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() || trimmed.trim_start().starts_with('#') {
            i += 1;
            continue;
        }
        // Only handle top-level (non-indented) keys.
        if line.starts_with(char::is_whitespace) {
            i += 1;
            continue;
        }
        if let Some(colon) = trimmed.find(':') {
            let key = trimmed[..colon].trim().to_string();
            let val = trimmed[colon + 1..].trim();
            if val.is_empty() {
                // Possible block list following.
                let mut items: Vec<Value> = Vec::new();
                let mut j = i + 1;
                while j < lines.len() {
                    let l = lines[j];
                    let lt = l.trim();
                    if l.starts_with(char::is_whitespace) && lt.starts_with("- ") {
                        items.push(Value::String(unquote(lt[2..].trim()).to_string()));
                        j += 1;
                    } else if lt.is_empty() {
                        j += 1;
                    } else {
                        break;
                    }
                }
                if items.is_empty() {
                    map.insert(key, Value::Null);
                } else {
                    map.insert(key, Value::Array(items));
                    i = j;
                    continue;
                }
            } else if val.starts_with('[') && val.ends_with(']') {
                let inner = &val[1..val.len() - 1];
                let items: Vec<Value> = inner
                    .split(',')
                    .map(|s| Value::String(unquote(s.trim()).to_string()))
                    .filter(|v| v.as_str().map(|s| !s.is_empty()).unwrap_or(false))
                    .collect();
                map.insert(key, Value::Array(items));
            } else {
                map.insert(key, Value::String(unquote(val).to_string()));
            }
        }
        i += 1;
    }
    Value::Object(map)
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Merge `props` into the note's frontmatter, returning the full file text.
///
/// **Non-destructive, line-level edit**: existing frontmatter lines are preserved
/// verbatim (original order, and any structure the minimal parser can't model —
/// nested maps, block scalars — is left untouched). Only the specific top-level
/// keys in `props` are rewritten: a value updates/inserts that key's line, and a
/// `null` value removes it (with its indented continuation lines). If the block
/// ends up empty the frontmatter fence is dropped entirely. (Rebuilding the whole
/// block from the lossy parser would silently destroy unrepresentable YAML.)
pub fn merge_frontmatter(full: &str, props: &serde_json::Map<String, serde_json::Value>) -> String {
    let (fm_raw, body_start) = split_frontmatter(full);
    let mut lines: Vec<String> = match fm_raw {
        Some(yaml) => yaml.lines().map(|l| l.to_string()).collect(),
        None => Vec::new(),
    };
    for (k, v) in props {
        apply_frontmatter_prop(&mut lines, k, v);
    }

    let body = &full[body_start..];
    // No frontmatter left → drop the fence; trim the blank line it used to leave.
    if !lines.iter().any(|l| !l.trim().is_empty()) {
        return body.trim_start_matches('\n').to_string();
    }
    let mut out = String::from("---\n");
    for l in &lines {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("---\n");
    if !body.starts_with('\n') {
        out.push('\n');
    }
    out.push_str(body);
    out
}

/// Update / insert / remove one top-level key in raw frontmatter lines, leaving
/// every other line (including nested structures under untouched keys) intact.
fn apply_frontmatter_prop(lines: &mut Vec<String>, key: &str, value: &serde_json::Value) {
    let span = find_top_level_key_span(lines, key);
    if value.is_null() {
        if let Some((start, len)) = span {
            lines.drain(start..start + len);
        }
        return;
    }
    let new_line = emit_yaml_kv(key, value).trim_end_matches('\n').to_string();
    match span {
        Some((start, len)) => {
            lines.splice(start..start + len, std::iter::once(new_line));
        }
        None => lines.push(new_line),
    }
}

/// Find the `(start, len)` span of a top-level `key:` entry — the matching line
/// plus its following indented continuation lines (nested map / block scalar) up
/// to the next top-level key or blank line. `None` if the key is absent.
fn find_top_level_key_span(lines: &[String], key: &str) -> Option<(usize, usize)> {
    for (i, l) in lines.iter().enumerate() {
        if l.starts_with(char::is_whitespace) {
            continue; // not a top-level line
        }
        let Some(colon) = l.find(':') else { continue };
        if l[..colon].trim() != key {
            continue;
        }
        let mut j = i + 1;
        while j < lines.len()
            && lines[j].starts_with(char::is_whitespace)
            && !lines[j].trim().is_empty()
        {
            j += 1;
        }
        return Some((i, j - i));
    }
    None
}

fn emit_yaml_kv(key: &str, v: &serde_json::Value) -> String {
    use serde_json::Value;
    match v {
        Value::String(s) => format!("{key}: {}\n", yaml_scalar(s)),
        Value::Bool(b) => format!("{key}: {b}\n"),
        Value::Number(n) => format!("{key}: {n}\n"),
        Value::Array(arr) => {
            let items: Vec<String> = arr
                .iter()
                .map(|it| match it {
                    Value::String(s) => yaml_scalar(s),
                    other => other.to_string(),
                })
                .collect();
            format!("{key}: [{}]\n", items.join(", "))
        }
        Value::Null => String::new(),
        // Nested object / unexpected → inline JSON (a valid YAML flow value).
        other => format!("{key}: {other}\n"),
    }
}

/// Quote a YAML scalar that could be misparsed; otherwise emit it bare. Quotes
/// for: empty / padded / metacharacter strings, **and** strings that a YAML
/// consumer would otherwise coerce to another type — reserved words
/// (true/false/null/yes/no/on/off/~) and number-like strings — so an external
/// vault app reads back the exact string we stored (the `.md` is the truth source).
fn yaml_scalar(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let reserved = matches!(
        lower.as_str(),
        "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~"
    );
    let number_like = s.parse::<i64>().is_ok() || s.parse::<f64>().is_ok();
    let needs_quote = s.is_empty()
        || s != s.trim()
        || s.contains([':', '#', '[', ']', ',', '"', '\'', '\n'])
        || reserved
        || number_like;
    if needs_quote {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn collect_frontmatter_tags(fm: &serde_json::Value, out: &mut Vec<String>) {
    let Some(tags) = fm.get("tags").or_else(|| fm.get("tag")) else {
        return;
    };
    match tags {
        serde_json::Value::Array(arr) => {
            for t in arr {
                if let Some(s) = t.as_str() {
                    push_tag(s, out);
                }
            }
        }
        serde_json::Value::String(s) => {
            for piece in s.split([',', ' ']) {
                push_tag(piece, out);
            }
        }
        _ => {}
    }
}

fn push_tag(raw: &str, out: &mut Vec<String>) {
    let t = raw.trim().trim_start_matches('#').trim();
    if t.is_empty() {
        return;
    }
    out.push(normalize_tag(t));
}

/// NFC + lowercase, the canonical tag form used by `note_tag` and resolve.
pub fn normalize_tag(s: &str) -> String {
    s.nfc().collect::<String>().to_lowercase()
}

/// NFC normalize (case preserved) — used for path/title resolve keys with a
/// separate case-fold compare.
pub fn nfc(s: &str) -> String {
    s.nfc().collect::<String>()
}

fn in_code(ranges: &[Range<usize>], pos: usize) -> bool {
    ranges.iter().any(|r| pos >= r.start && pos < r.end)
}

/// Scan `[[wikilink]]` / `![[embed]]` occurrences (skipping code).
fn scan_wikilinks(
    full: &str,
    body_start_byte: usize,
    code_ranges: &[Range<usize>],
    headings: &[Heading],
    posmap: &PosMap,
) -> Vec<ParsedLink> {
    let mut out = Vec::new();
    let bytes = full.as_bytes();
    let mut i = body_start_byte;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            // Find closing `]]` on the same content (no embedded newline).
            if let Some(close_rel) = find_close(&full[i + 2..]) {
                let inner_start = i + 2;
                let inner_end = inner_start + close_rel;
                let inner = &full[inner_start..inner_end];
                // Embeds carry a leading `!`.
                let is_embed = i > 0 && bytes[i - 1] == b'!';
                let link_start = if is_embed { i - 1 } else { i };
                let link_end = inner_end + 2; // include closing ]]

                if !in_code(code_ranges, i) {
                    if let Some(link) = parse_wikilink_inner(
                        inner, link_start, link_end, full, headings, posmap, is_embed,
                    ) {
                        out.push(link);
                    }
                }
                i = link_end;
                continue;
            }
        }
        // advance by one char boundary
        i += utf8_len(bytes[i]);
    }
    out
}

/// Find the byte index of the closing `]]` within `s`, refusing to cross a
/// newline (wikilinks are single-line).
fn find_close(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        match bytes[i] {
            b'\n' => return None,
            b']' if bytes[i + 1] == b']' => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// The resolution target inside a `[[ ]]` / `![[ ]]` reference — drop the
/// `|alias` then the `#anchor`, trimmed. The single source for splitting a
/// wikilink reference, so any non-parser caller (e.g. the transclusion owner
/// entry `service::note_read_ref`) resolves the same notes the link graph does.
pub fn wikilink_target(inner: &str) -> &str {
    let before_alias = inner.split_once('|').map(|(t, _)| t).unwrap_or(inner);
    before_alias
        .split_once('#')
        .map(|(t, _)| t)
        .unwrap_or(before_alias)
        .trim()
}

fn parse_wikilink_inner(
    inner: &str,
    link_start: usize,
    link_end: usize,
    full: &str,
    headings: &[Heading],
    posmap: &PosMap,
    is_embed: bool,
) -> Option<ParsedLink> {
    let inner = inner.trim();
    if inner.is_empty() {
        return None;
    }
    // alias: split on first `|`.
    let (target_and_anchor, alias) = match inner.split_once('|') {
        Some((t, a)) => (t.trim(), Some(a.trim().to_string())),
        None => (inner, None),
    };
    // anchor: split on first `#`. Target derives from the shared helper so the
    // split convention stays single-sourced.
    let anchor = target_and_anchor
        .split_once('#')
        .map(|(_, a)| a.trim().to_string());
    let target_ref = wikilink_target(inner).to_string();
    if target_ref.is_empty() && anchor.is_none() {
        return None;
    }
    let raw_text = &full[link_start..link_end];
    Some(ParsedLink {
        target_ref,
        anchor: anchor.filter(|s| !s.is_empty()),
        alias: alias.filter(|s| !s.is_empty()),
        link_type: if is_embed {
            LinkType::Embed
        } else {
            LinkType::Wiki
        },
        raw_text: raw_text.to_string(),
        start: posmap.pos(link_start),
        end: posmap.pos(link_end),
        heading_path: heading_path_at(link_start, headings),
    })
}

/// Scan Obsidian `^block-id` anchors (skipping code), mapping each to the leaf
/// block it terminates. First occurrence of a given id wins (later dups ignored,
/// matching Obsidian). See [`ParsedBlock`] for the attach rule.
fn scan_blocks(
    full: &str,
    body_start_byte: usize,
    code_ranges: &[Range<usize>],
    leaf_spans: &[Range<usize>],
    posmap: &PosMap,
) -> Vec<ParsedBlock> {
    let mut out: Vec<ParsedBlock> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Per-leaf-span (keyed by span start) floor: the byte a block may start at,
    // so two `^id` anchors sharing one folded paragraph don't let the second
    // block leak the first line + its anchor token.
    let mut span_floor: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let mut line_start = body_start_byte;
    for line in full[body_start_byte..].split_inclusive('\n') {
        let abs_line_start = line_start;
        line_start += line.len();
        let next_line_start = line_start;
        let content = line.trim_end_matches(['\r', '\n']);
        let Some((caret_off, id)) = trailing_block_anchor(content) else {
            continue;
        };
        let anchor_byte = abs_line_start + caret_off;
        if in_code(code_ranges, anchor_byte) {
            continue;
        }
        if seen.contains(&id) {
            continue;
        }
        let Some((start_byte, end_byte)) = resolve_block_span(
            full,
            anchor_byte,
            abs_line_start,
            next_line_start,
            leaf_spans,
            &mut span_floor,
        ) else {
            continue;
        };
        if end_byte <= start_byte {
            continue;
        }
        seen.insert(id.clone());
        out.push(ParsedBlock {
            block_id: id,
            start: posmap.pos(start_byte),
            end: posmap.pos(end_byte),
            text: full[start_byte..end_byte].to_string(),
        });
    }
    out
}

/// The trailing `^block-id` of a single line (anchor stripped of CR/LF already):
/// returns `(byte offset of '^', id)` when the line ends with `^id` preceded by
/// whitespace or start-of-line. Public so write tools can detect / dedupe ids.
pub fn line_block_anchor(line: &str) -> Option<String> {
    let line = line.trim_end_matches(['\r', '\n']);
    trailing_block_anchor(line).map(|(_, id)| id)
}

fn trailing_block_anchor(line: &str) -> Option<(usize, String)> {
    let t = line.trim_end();
    let bytes = t.as_bytes();
    let mut i = t.len();
    while i > 0 {
        let c = bytes[i - 1];
        if c.is_ascii_alphanumeric() || c == b'-' {
            i -= 1;
        } else {
            break;
        }
    }
    let id = &t[i..];
    if id.is_empty() {
        return None;
    }
    // The char immediately before the id must be the caret.
    if i == 0 || bytes[i - 1] != b'^' {
        return None;
    }
    let caret = i - 1;
    // The caret must start a token: preceded by whitespace or start-of-line.
    if caret > 0 && !matches!(bytes[caret - 1], b' ' | b'\t') {
        return None;
    }
    Some((caret, id.to_string()))
}

/// Resolve the leaf-block source span (`start..end`, anchor excluded) a `^id`
/// anchor attaches to: the innermost leaf span containing the caret (trailing
/// anchor), else the nearest preceding leaf span (own-line anchor). `span_floor`
/// (keyed by span start) records how far a prior anchor in the same span already
/// consumed, so a second `^id` in one folded paragraph starts after the first
/// (rather than leaking the earlier line + its anchor token).
fn resolve_block_span(
    full: &str,
    anchor_byte: usize,
    line_start: usize,
    next_line_start: usize,
    leaf_spans: &[Range<usize>],
    span_floor: &mut std::collections::HashMap<usize, usize>,
) -> Option<(usize, usize)> {
    // Innermost (smallest) leaf span containing the caret.
    if let Some(s) = leaf_spans
        .iter()
        .filter(|s| s.start <= anchor_byte && anchor_byte < s.end)
        .min_by_key(|s| s.end - s.start)
    {
        let floor = span_floor
            .get(&s.start)
            .copied()
            .unwrap_or(s.start)
            .max(s.start);
        let trimmed_end = floor + full[floor..anchor_byte].trim_end().len();
        if trimmed_end > floor {
            // The next anchor in this span starts at the following line.
            span_floor.insert(s.start, next_line_start.min(s.end));
            let lead = leading_ws(&full[floor..trimmed_end]);
            return Some((floor + lead, trimmed_end));
        }
        // Else the anchor is alone in its own block (`^id` on its own line) —
        // fall through to the preceding block.
    }
    let prev = leaf_spans
        .iter()
        .filter(|s| s.end <= line_start)
        .max_by_key(|s| s.end)?;
    let floor = span_floor
        .get(&prev.start)
        .copied()
        .unwrap_or(prev.start)
        .max(prev.start);
    let trimmed_end = floor + full[floor..prev.end].trim_end().len();
    if trimmed_end <= floor {
        return None;
    }
    span_floor.insert(prev.start, prev.end);
    let lead = leading_ws(&full[floor..trimmed_end]);
    Some((floor + lead, trimmed_end))
}

fn leading_ws(s: &str) -> usize {
    s.len() - s.trim_start().len()
}

/// Scan `#tag` occurrences (skipping code + heading markers).
fn scan_tags(full: &str, body_start_byte: usize, code_ranges: &[Range<usize>]) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = full.as_bytes();
    let mut i = body_start_byte;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            // Preceding char must be start-of-line or whitespace (so `a#b` and
            // URL fragments don't match).
            let prev_ws = i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r');
            // Next char must be a tag char (so `# Heading` — `#` then space —
            // is not a tag).
            let next_is_tag = full[i + 1..]
                .chars()
                .next()
                .map(is_tag_char)
                .unwrap_or(false);
            if prev_ws && next_is_tag && !in_code(code_ranges, i) {
                let tag: String = full[i + 1..]
                    .chars()
                    .take_while(|c| is_tag_char(*c))
                    .collect();
                if !tag.is_empty() && tag.chars().any(|c| c.is_alphanumeric()) {
                    out.push(normalize_tag(&tag));
                }
                i += 1 + tag.len();
                continue;
            }
        }
        i += utf8_len(bytes[i]);
    }
    out
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '/'
}

fn utf8_len(first_byte: u8) -> usize {
    if first_byte < 0x80 {
        1
    } else if first_byte < 0xE0 {
        2
    } else if first_byte < 0xF0 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posmap_lines_cols_codepoints() {
        let text = "ab\ncдe\n"; // line2 has a 2-byte cyrillic char
        let pm = PosMap::new(text);
        // byte 0 = 'a'
        let p = pm.pos(0);
        assert_eq!((p.line, p.col, p.offset), (1, 0, 0));
        // start of line 2 (byte 3, after \n)
        let p = pm.pos(3);
        assert_eq!((p.line, p.col), (2, 0));
        // the 'e' after the cyrillic char: bytes c(1) д(2) => byte 6
        let p = pm.pos(6);
        assert_eq!((p.line, p.col), (2, 2)); // col counts code points, not bytes
    }

    #[test]
    fn wikilinks_parsed_with_alias_and_anchor() {
        let doc = parse_document("# Title\n\nSee [[folder/note#Heading|Alias]] here.\n");
        assert_eq!(doc.title.as_deref(), Some("Title"));
        assert_eq!(doc.links.len(), 1);
        let l = &doc.links[0];
        assert_eq!(l.target_ref, "folder/note");
        assert_eq!(l.anchor.as_deref(), Some("Heading"));
        assert_eq!(l.alias.as_deref(), Some("Alias"));
        assert_eq!(l.link_type, LinkType::Wiki);
    }

    #[test]
    fn wikilinks_in_code_are_skipped() {
        let doc = parse_document("Real [[a]]\n\n```\n[[notalink]]\n```\n\n`[[alsonot]]`\n");
        let targets: Vec<&str> = doc.links.iter().map(|l| l.target_ref.as_str()).collect();
        assert_eq!(targets, vec!["a"]);
    }

    #[test]
    fn tags_collected_inline_and_frontmatter() {
        let doc = parse_document(
            "---\ntitle: T\ntags: [alpha, Beta]\n---\n\nbody #gamma and `#nocode`\n",
        );
        assert!(doc.tags.contains(&"alpha".to_string()));
        assert!(doc.tags.contains(&"beta".to_string()));
        assert!(doc.tags.contains(&"gamma".to_string()));
        assert!(!doc.tags.contains(&"nocode".to_string()));
    }

    #[test]
    fn heading_marker_is_not_a_tag() {
        let doc = parse_document("# Heading\n\ntext\n");
        assert!(doc.tags.is_empty());
    }

    #[test]
    fn merge_frontmatter_creates_block_when_absent() {
        let mut props = serde_json::Map::new();
        props.insert("status".into(), serde_json::Value::String("active".into()));
        let out = merge_frontmatter("# Title\n\nbody\n", &props);
        assert!(out.starts_with("---\nstatus: active\n---\n\n# Title"));
        // Re-parsing keeps the title + sees the new key.
        let doc = parse_document(&out);
        assert_eq!(doc.title.as_deref(), Some("Title"));
        assert_eq!(
            doc.frontmatter_json
                .as_deref()
                .map(|s| s.contains("active")),
            Some(true)
        );
    }

    #[test]
    fn merge_frontmatter_updates_and_removes_keys() {
        let full = "---\ntitle: T\nstatus: draft\n---\n\nbody\n";
        let mut props = serde_json::Map::new();
        props.insert("status".into(), serde_json::Value::String("done".into()));
        props.insert("title".into(), serde_json::Value::Null); // null → remove
        let out = merge_frontmatter(full, &props);
        let doc = parse_document(&out);
        let fm = doc.frontmatter_json.unwrap();
        assert!(fm.contains("done"));
        assert!(!fm.contains("\"title\""));
        assert!(out.ends_with("\nbody\n"));
    }

    #[test]
    fn merge_frontmatter_emits_string_array() {
        let mut props = serde_json::Map::new();
        props.insert("tags".into(), serde_json::json!(["alpha", "beta gamma"]));
        let out = merge_frontmatter("body\n", &props);
        // A plain space needs no quoting inside a flow sequence (valid plain
        // scalar); only a comma would (and is quoted by `yaml_scalar`).
        assert!(out.contains("tags: [alpha, beta gamma]"), "got: {out}");
        // Round-trips: re-parsing recovers both tag items.
        let doc = parse_document(&out);
        assert!(doc.tags.contains(&"alpha".to_string()));
        assert!(doc.tags.contains(&"beta gamma".to_string()));
    }

    #[test]
    fn merge_frontmatter_preserves_nested_and_order() {
        // A nested map + a block scalar the minimal parser can't model, and a
        // deliberate (non-alphabetical) key order. Setting one unrelated key must
        // leave all of that intact (no data loss, no reorder).
        let full = "---\ntitle: T\nauthor:\n  name: X\n  email: y@z\ndesc: |\n  line one\n  line two\n---\n\nbody\n";
        let mut props = serde_json::Map::new();
        props.insert("status".into(), serde_json::Value::String("active".into()));
        let out = merge_frontmatter(full, &props);
        assert!(
            out.contains("author:\n  name: X\n  email: y@z"),
            "nested map lost: {out}"
        );
        assert!(
            out.contains("desc: |\n  line one\n  line two"),
            "block scalar lost: {out}"
        );
        // Original order preserved (title before author), new key appended.
        assert!(out.find("title:").unwrap() < out.find("author:").unwrap());
        assert!(out.contains("status: active"));
        assert!(out.ends_with("\nbody\n"));
    }

    #[test]
    fn merge_frontmatter_removes_nested_key_with_children() {
        let full = "---\ntitle: T\nauthor:\n  name: X\n---\n\nbody\n";
        let mut props = serde_json::Map::new();
        props.insert("author".into(), serde_json::Value::Null);
        let out = merge_frontmatter(full, &props);
        assert!(!out.contains("author"), "author block not removed: {out}");
        assert!(!out.contains("name: X"));
        assert!(out.contains("title: T"));
    }

    #[test]
    fn merge_frontmatter_quotes_type_coercing_strings() {
        let mut props = serde_json::Map::new();
        props.insert("flag".into(), serde_json::Value::String("true".into()));
        props.insert("code".into(), serde_json::Value::String("42".into()));
        let out = merge_frontmatter("body\n", &props);
        // Bare `true`/`42` would read back as bool/int in any real YAML parser.
        assert!(out.contains("flag: \"true\""), "got: {out}");
        assert!(out.contains("code: \"42\""), "got: {out}");
    }

    #[test]
    fn merge_frontmatter_drops_empty_block() {
        let full = "---\ntitle: T\n---\n\nbody\n";
        let mut props = serde_json::Map::new();
        props.insert("title".into(), serde_json::Value::Null);
        let out = merge_frontmatter(full, &props);
        assert!(!out.contains("---"), "stray fence left: {out}");
        assert!(out.contains("body"));
    }

    #[test]
    fn frontmatter_offsets_keep_full_file_coords() {
        let full = "---\ntitle: T\n---\n\n[[x]]\n";
        let doc = parse_document(full);
        assert!(doc.body_start_byte > 0);
        // The link is on line 5 of the full file.
        assert_eq!(doc.links[0].start.line, 5);
    }

    #[test]
    fn block_anchor_trailing_paragraph() {
        let doc = parse_document("# T\n\nThe quick brown fox. ^fox1\n\nNext para.\n");
        assert_eq!(doc.blocks.len(), 1);
        let b = &doc.blocks[0];
        assert_eq!(b.block_id, "fox1");
        assert_eq!(b.text, "The quick brown fox.");
        // Anchor sits on line 3 of the full file.
        assert_eq!(b.start.line, 3);
    }

    #[test]
    fn block_anchor_own_line_attaches_to_preceding() {
        let doc = parse_document("# T\n\nA paragraph spanning\ntwo lines.\n\n^para9\n\nAfter.\n");
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.blocks[0].block_id, "para9");
        assert_eq!(doc.blocks[0].text, "A paragraph spanning\ntwo lines.");
    }

    #[test]
    fn block_anchor_on_list_item_is_just_that_item() {
        let doc = parse_document("- first\n- second item ^b2\n- third\n");
        let ids: Vec<&str> = doc.blocks.iter().map(|b| b.block_id.as_str()).collect();
        assert_eq!(ids, vec!["b2"]);
        assert_eq!(doc.blocks[0].text, "- second item");
    }

    #[test]
    fn block_anchor_dup_id_first_wins() {
        let doc = parse_document("One. ^dup\n\nTwo. ^dup\n");
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.blocks[0].text, "One.");
    }

    #[test]
    fn block_anchor_in_code_is_skipped() {
        let doc = parse_document("Real. ^ok\n\n```\nnot a block ^nope\n```\n");
        let ids: Vec<&str> = doc.blocks.iter().map(|b| b.block_id.as_str()).collect();
        assert_eq!(ids, vec!["ok"]);
    }

    #[test]
    fn caret_without_leading_space_is_not_a_block_anchor() {
        // `x^2` (no space before caret) and `^{2}` (non-id char) must not match.
        let doc = parse_document("Pow x^2 here.\n\nMath ^{2} curly.\n");
        assert!(doc.blocks.is_empty(), "got: {:?}", doc.blocks);
    }

    #[test]
    fn block_anchor_no_blank_line_before_own_line_caret() {
        // No blank line: cmark folds `text` + `^id` into one paragraph; the
        // trailing-span path still strips the anchor off the prior text.
        let doc = parse_document("Some prose here.\n^inline1\n");
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.blocks[0].block_id, "inline1");
        assert_eq!(doc.blocks[0].text, "Some prose here.");
    }

    #[test]
    fn block_anchor_two_anchors_in_folded_paragraph_dont_leak() {
        // No blank line → cmark folds both lines into one paragraph. Each line
        // carries its own `^id`; the second block must NOT leak the first line
        // (or the first `^a` token) into its text.
        let doc = parse_document("line one ^a\nline two ^b\n");
        let by = |id: &str| doc.blocks.iter().find(|b| b.block_id == id).unwrap();
        assert_eq!(by("a").text, "line one");
        assert_eq!(by("b").text, "line two");
    }

    #[test]
    fn line_block_anchor_helper() {
        assert_eq!(line_block_anchor("text ^id-1").as_deref(), Some("id-1"));
        assert_eq!(line_block_anchor("^own").as_deref(), Some("own"));
        assert_eq!(line_block_anchor("no anchor here"), None);
        assert_eq!(line_block_anchor("x^2"), None);
    }

    #[test]
    fn wikilink_target_strips_anchor_and_alias() {
        assert_eq!(wikilink_target("Note"), "Note");
        assert_eq!(wikilink_target("folder/Note"), "folder/Note");
        assert_eq!(wikilink_target("Note#Heading"), "Note");
        assert_eq!(wikilink_target("Note|Alias"), "Note");
        // alias split happens first, so a `#` inside the alias is ignored.
        assert_eq!(
            wikilink_target("folder/Note#Heading|Alias#x"),
            "folder/Note"
        );
        assert_eq!(wikilink_target("  spaced  | a "), "spaced");
    }
}
