//! Safe Markdown compiler for Telegram Bot API 10.1/10.2 rich messages.
//!
//! Markdown is parsed once into a deliberately small IR. Both the typed block
//! payload and the HTML compatibility payload are rendered from that IR, so a
//! raw Markdown/HTML fragment can never bypass link filtering or escaping.

use std::{error::Error, fmt, mem};

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag};
use serde_json::{json, Value};
use url::Url;

pub(crate) const RICH_MESSAGE_MAX_CHARS: usize = 32_768;
pub(crate) const RICH_MESSAGE_MAX_BLOCKS: usize = 500;
const RICH_MESSAGE_MAX_DEPTH: usize = 16;
const RICH_MESSAGE_MAX_TABLE_COLUMNS: usize = 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RichFormat {
    Blocks,
    Html,
}

/// A finite set prevents model-produced chain-of-thought from entering the
/// draft-only Telegram thinking block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ThinkingLabel {
    Generating,
    RunningTools,
    Finalizing,
}

impl ThinkingLabel {
    fn text(self) -> &'static str {
        match self {
            Self::Generating => "Generating…",
            Self::RunningTools => "Running tools…",
            Self::Finalizing => "Finalizing…",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RichSegment {
    /// Telegram Bot API 10.2 `InputRichBlock` values. Native media support may
    /// append media blocks before calling [`Self::input_message`].
    pub(crate) blocks: Vec<Value>,
    /// Telegram Bot API 10.1 HTML fallback rendered from the same safe IR.
    pub(crate) html: String,
    text_chars: usize,
}

impl RichSegment {
    /// Counts top-level and nested blocks using Telegram's rich-message rules.
    /// This remains accurate when the native integration appends media blocks.
    pub(crate) fn block_count(&self) -> usize {
        count_json_blocks(&self.blocks)
    }

    /// Number of Unicode scalar values in the text compiled into this segment.
    pub(crate) fn text_chars(&self) -> usize {
        self.text_chars
    }

    /// Builds an `InputRichMessage` with exactly one content representation.
    /// Passing `None` always produces the final form with no thinking block.
    pub(crate) fn input_message(
        &self,
        format: RichFormat,
        thinking_label: Option<ThinkingLabel>,
    ) -> Value {
        let thinking_label = thinking_label.filter(|label| {
            self.text_chars.saturating_add(label.text().chars().count()) <= RICH_MESSAGE_MAX_CHARS
                && self.block_count() < RICH_MESSAGE_MAX_BLOCKS
        });

        match format {
            RichFormat::Blocks => {
                let mut blocks = self.blocks.clone();
                if let Some(label) = thinking_label {
                    blocks.insert(
                        0,
                        json!({
                            "type": "thinking",
                            "text": label.text(),
                        }),
                    );
                }
                json!({
                    "blocks": blocks,
                    "skip_entity_detection": true,
                })
            }
            RichFormat::Html => {
                let mut html = String::new();
                if let Some(label) = thinking_label {
                    html.push_str("<tg-thinking>");
                    push_html_text(&mut html, label.text());
                    html.push_str("</tg-thinking>");
                }
                html.push_str(&self.html);
                json!({
                    "html": html,
                    "skip_entity_detection": true,
                })
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RichCompileError {
    NestingLimitExceeded,
    TableColumnLimitExceeded { columns: usize },
    InvalidDocument,
}

impl fmt::Display for RichCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NestingLimitExceeded => write!(
                formatter,
                "rich Markdown exceeds the {RICH_MESSAGE_MAX_DEPTH}-level nesting limit"
            ),
            Self::TableColumnLimitExceeded { columns } => write!(
                formatter,
                "rich Markdown table has {columns} columns; maximum is {RICH_MESSAGE_MAX_TABLE_COLUMNS}"
            ),
            Self::InvalidDocument => write!(formatter, "invalid rich Markdown document"),
        }
    }
}

impl Error for RichCompileError {}

/// Compiles Markdown and prefers splitting between top-level blocks. A single
/// oversized block is losslessly flattened to safe plain-text paragraphs so
/// every segment remains valid rich HTML/blocks; formatting may degrade at
/// that exceptional boundary, but content never falls into byte-sliced HTML.
pub(crate) fn compile_markdown(markdown: &str) -> Result<Vec<RichSegment>, RichCompileError> {
    let nodes = parse_nodes(markdown)?;
    let blocks = nodes_to_blocks(nodes)?;

    let mut segments = Vec::new();
    let mut current = Vec::new();
    let mut current_chars = 0usize;
    let mut current_blocks = 0usize;

    for source_block in blocks {
        let source_chars = source_block.text_chars();
        let source_blocks = source_block.block_count();
        let normalized =
            if source_chars > RICH_MESSAGE_MAX_CHARS || source_blocks > RICH_MESSAGE_MAX_BLOCKS {
                flatten_oversized_block(source_block)
            } else {
                vec![source_block]
            };

        for block in normalized {
            let chars = block.text_chars();
            let block_count = block.block_count();
            debug_assert!(chars <= RICH_MESSAGE_MAX_CHARS);
            debug_assert!(block_count <= RICH_MESSAGE_MAX_BLOCKS);

            let would_overflow = !current.is_empty()
                && (current_chars.saturating_add(chars) > RICH_MESSAGE_MAX_CHARS
                    || current_blocks.saturating_add(block_count) > RICH_MESSAGE_MAX_BLOCKS);
            if would_overflow {
                segments.push(render_segment(mem::take(&mut current), current_chars));
                current_chars = 0;
                current_blocks = 0;
            }

            current_chars = current_chars.saturating_add(chars);
            current_blocks = current_blocks.saturating_add(block_count);
            current.push(block);
        }
    }

    if !current.is_empty() || segments.is_empty() {
        segments.push(render_segment(current, current_chars));
    }
    Ok(segments)
}

/// Fail-safe representation for structurally invalid or over-nested Markdown.
/// Raw input becomes escaped paragraph text and is split on Unicode scalar
/// boundaries, so callers never need to fall back to byte-sliced legacy HTML.
pub(crate) fn compile_plain_text(text: &str) -> Vec<RichSegment> {
    split_chars(text, RICH_MESSAGE_MAX_CHARS)
        .into_iter()
        .map(|chunk| {
            let chars = chunk.chars().count();
            render_segment(vec![Block::Paragraph(vec![Inline::Text(chunk)])], chars)
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Container(Container, Vec<Node>),
    Text(String),
    Code(String),
    InlineMath(String),
    DisplayMath(String),
    Break,
    Rule,
    TaskMarker(bool),
}

#[derive(Clone, Debug, PartialEq)]
enum Container {
    Paragraph,
    Heading(u8),
    BlockQuote,
    CodeBlock(Option<String>),
    RawHtml,
    List(Option<u64>),
    Item,
    Table(Vec<CellAlign>),
    TableHead,
    TableRow,
    TableCell,
    Emphasis,
    Strong,
    Strikethrough,
    Link(String),
    Image(String),
    Transparent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CellAlign {
    Left,
    Center,
    Right,
}

impl CellAlign {
    fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }
}

impl From<Alignment> for CellAlign {
    fn from(alignment: Alignment) -> Self {
        match alignment {
            Alignment::Center => Self::Center,
            Alignment::Right => Self::Right,
            Alignment::None | Alignment::Left => Self::Left,
        }
    }
}

struct Frame {
    container: Option<Container>,
    children: Vec<Node>,
}

fn parse_nodes(markdown: &str) -> Result<Vec<Node>, RichCompileError> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_MATH;
    let mut stack = vec![Frame {
        container: None,
        children: Vec::new(),
    }];

    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(tag) => {
                // The root frame isn't part of Telegram's nesting depth.
                if stack.len() > RICH_MESSAGE_MAX_DEPTH {
                    return Err(RichCompileError::NestingLimitExceeded);
                }
                stack.push(Frame {
                    container: Some(container_from_tag(tag)?),
                    children: Vec::new(),
                });
            }
            Event::End(_) => {
                if stack.len() <= 1 {
                    return Err(RichCompileError::InvalidDocument);
                }
                let Some(frame) = stack.pop() else {
                    return Err(RichCompileError::InvalidDocument);
                };
                let Some(container) = frame.container else {
                    return Err(RichCompileError::InvalidDocument);
                };
                let Some(parent) = stack.last_mut() else {
                    return Err(RichCompileError::InvalidDocument);
                };
                parent
                    .children
                    .push(Node::Container(container, frame.children));
            }
            Event::Text(text) => push_node(&mut stack, Node::Text(text.into_string()))?,
            // pulldown-cmark represents code/math as leaf events rather than
            // `Start(Tag)` containers, but mixed paragraph output can add an
            // extra RichText wrapper. Charge that wrapper explicitly so the
            // serialized tree can never exceed the 16-level limit.
            Event::Code(code) => push_wrapped_inline(&mut stack, Node::Code(code.into_string()))?,
            Event::InlineMath(expression) => {
                push_wrapped_inline(&mut stack, Node::InlineMath(expression.into_string()))?
            }
            Event::DisplayMath(expression) => {
                push_wrapped_inline(&mut stack, Node::DisplayMath(expression.into_string()))?
            }
            // Raw HTML is deliberately retained only as literal text. It is
            // escaped by both output renderers and can never become a tag.
            Event::Html(html) | Event::InlineHtml(html) => {
                push_node(&mut stack, Node::Text(html.into_string()))?
            }
            Event::FootnoteReference(label) => {
                push_node(&mut stack, Node::Text(format!("[^{label}]")))?
            }
            Event::SoftBreak | Event::HardBreak => push_node(&mut stack, Node::Break)?,
            // A horizontal rule serializes as a leaf divider block. Unlike a
            // paragraph it has no Start event of its own, so charge that final
            // output level explicitly.
            Event::Rule => push_wrapped_inline(&mut stack, Node::Rule)?,
            Event::TaskListMarker(checked) => push_node(&mut stack, Node::TaskMarker(checked))?,
        }
    }

    if stack.len() != 1 {
        return Err(RichCompileError::InvalidDocument);
    }
    stack
        .pop()
        .map(|frame| frame.children)
        .ok_or(RichCompileError::InvalidDocument)
}

fn push_node(stack: &mut [Frame], node: Node) -> Result<(), RichCompileError> {
    let Some(frame) = stack.last_mut() else {
        return Err(RichCompileError::InvalidDocument);
    };
    frame.children.push(node);
    Ok(())
}

fn push_wrapped_inline(stack: &mut [Frame], node: Node) -> Result<(), RichCompileError> {
    // `stack` includes the root frame. Therefore its length is exactly the
    // output depth after adding one leaf RichText wrapper.
    if stack.len() > RICH_MESSAGE_MAX_DEPTH {
        return Err(RichCompileError::NestingLimitExceeded);
    }
    push_node(stack, node)
}

fn container_from_tag(tag: Tag<'_>) -> Result<Container, RichCompileError> {
    Ok(match tag {
        Tag::Paragraph => Container::Paragraph,
        Tag::Heading { level, .. } => Container::Heading(level as u8),
        Tag::BlockQuote(_) => Container::BlockQuote,
        Tag::CodeBlock(kind) => Container::CodeBlock(code_language(kind)),
        Tag::HtmlBlock => Container::RawHtml,
        Tag::List(start) => Container::List(start),
        Tag::Item => Container::Item,
        Tag::Table(alignments) => {
            if alignments.len() > RICH_MESSAGE_MAX_TABLE_COLUMNS {
                return Err(RichCompileError::TableColumnLimitExceeded {
                    columns: alignments.len(),
                });
            }
            Container::Table(alignments.into_iter().map(CellAlign::from).collect())
        }
        Tag::TableHead => Container::TableHead,
        Tag::TableRow => Container::TableRow,
        Tag::TableCell => Container::TableCell,
        Tag::Emphasis => Container::Emphasis,
        Tag::Strong => Container::Strong,
        Tag::Strikethrough => Container::Strikethrough,
        Tag::Link { dest_url, .. } => Container::Link(dest_url.into_string()),
        Tag::Image { dest_url, .. } => Container::Image(dest_url.into_string()),
        Tag::FootnoteDefinition(_)
        | Tag::DefinitionList
        | Tag::DefinitionListTitle
        | Tag::DefinitionListDefinition
        | Tag::Superscript
        | Tag::Subscript
        | Tag::MetadataBlock(_) => Container::Transparent,
    })
}

fn code_language(kind: CodeBlockKind<'_>) -> Option<String> {
    let CodeBlockKind::Fenced(info) = kind else {
        return None;
    };
    let language: String = info
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '+' | '.' | '#')
        })
        .take(64)
        .collect();
    (!language.is_empty()).then_some(language)
}

#[derive(Clone, Debug, PartialEq)]
enum Block {
    Paragraph(Vec<Inline>),
    Heading {
        size: u8,
        text: Vec<Inline>,
    },
    Pre {
        text: String,
        language: Option<String>,
    },
    Divider,
    List {
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Quote(Vec<Block>),
    Table {
        rows: Vec<TableRow>,
    },
    Math(String),
}

impl Block {
    fn text_chars(&self) -> usize {
        match self {
            Self::Paragraph(text) | Self::Heading { text, .. } => inline_chars(text),
            Self::Pre { text, .. } | Self::Math(text) => text.chars().count(),
            Self::Divider => 0,
            Self::List { items, .. } => items
                .iter()
                .flat_map(|item| &item.blocks)
                .map(Self::text_chars)
                .sum(),
            Self::Quote(blocks) => blocks.iter().map(Self::text_chars).sum(),
            Self::Table { rows } => rows
                .iter()
                .flat_map(|row| &row.cells)
                .map(|cell| inline_chars(&cell.text))
                .sum(),
        }
    }

    fn block_count(&self) -> usize {
        match self {
            Self::List { items, .. } => {
                1 + items
                    .iter()
                    .map(|item| 1 + item.blocks.iter().map(Self::block_count).sum::<usize>())
                    .sum::<usize>()
            }
            Self::Quote(blocks) => 1 + blocks.iter().map(Self::block_count).sum::<usize>(),
            Self::Table { rows } => 1 + rows.len(),
            Self::Paragraph(_)
            | Self::Heading { .. }
            | Self::Pre { .. }
            | Self::Divider
            | Self::Math(_) => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ListItem {
    checked: Option<bool>,
    blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq)]
struct TableRow {
    cells: Vec<TableCell>,
}

#[derive(Clone, Debug, PartialEq)]
struct TableCell {
    text: Vec<Inline>,
    is_header: bool,
    align: CellAlign,
}

#[derive(Clone, Debug, PartialEq)]
enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Code(String),
    Link { target: SafeLink, text: Vec<Inline> },
    Image { target: SafeLink, alt: Vec<Inline> },
    Math(String),
    Break,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SafeLink {
    Url(String),
    Email(String),
}

impl SafeLink {
    fn parse(raw: &str) -> Option<Self> {
        let parsed = Url::parse(raw).ok()?;
        match parsed.scheme() {
            "http" | "https"
                if parsed.host_str().is_some()
                    && parsed.username().is_empty()
                    && parsed.password().is_none() =>
            {
                Some(Self::Url(parsed.to_string()))
            }
            "mailto" if parsed.query().is_none() && parsed.fragment().is_none() => {
                let address = parsed.path();
                is_safe_email(address).then(|| Self::Email(address.to_owned()))
            }
            _ => None,
        }
    }

    fn href(&self) -> String {
        match self {
            Self::Url(url) => url.clone(),
            Self::Email(address) => format!("mailto:{address}"),
        }
    }
}

fn is_safe_email(address: &str) -> bool {
    let mut parts = address.split('@');
    let Some(local) = parts.next() else {
        return false;
    };
    let Some(domain) = parts.next() else {
        return false;
    };
    if local.is_empty() || domain.is_empty() || parts.next().is_some() {
        return false;
    }
    address.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '@' | '.' | '_' | '+' | '-')
    })
}

fn nodes_to_blocks(nodes: Vec<Node>) -> Result<Vec<Block>, RichCompileError> {
    let mut blocks = Vec::new();
    let mut pending = Vec::new();

    for node in nodes {
        match node {
            Node::Container(Container::Paragraph, children) => {
                flush_pending(&mut pending, &mut blocks);
                if let [Node::DisplayMath(expression)] = children.as_slice() {
                    blocks.push(Block::Math(expression.clone()));
                } else {
                    blocks.push(Block::Paragraph(nodes_to_inlines(children)));
                }
            }
            Node::Container(Container::Heading(size), children) => {
                flush_pending(&mut pending, &mut blocks);
                blocks.push(Block::Heading {
                    size,
                    text: nodes_to_inlines(children),
                });
            }
            Node::Container(Container::CodeBlock(language), children) => {
                flush_pending(&mut pending, &mut blocks);
                blocks.push(Block::Pre {
                    text: nodes_plain_text(&children),
                    language,
                });
            }
            Node::Container(Container::BlockQuote, children) => {
                flush_pending(&mut pending, &mut blocks);
                blocks.push(Block::Quote(nodes_to_blocks(children)?));
            }
            Node::Container(Container::List(start), children) => {
                flush_pending(&mut pending, &mut blocks);
                blocks.push(list_from_nodes(start, children)?);
            }
            Node::Container(Container::Table(alignments), children) => {
                flush_pending(&mut pending, &mut blocks);
                blocks.push(table_from_nodes(alignments, children)?);
            }
            Node::DisplayMath(expression) => {
                flush_pending(&mut pending, &mut blocks);
                blocks.push(Block::Math(expression));
            }
            Node::Rule => {
                flush_pending(&mut pending, &mut blocks);
                blocks.push(Block::Divider);
            }
            Node::Container(Container::Item, children) => {
                flush_pending(&mut pending, &mut blocks);
                blocks.extend(nodes_to_blocks(children)?);
            }
            other => inline_from_node(other, &mut pending),
        }
    }

    flush_pending(&mut pending, &mut blocks);
    Ok(blocks)
}

fn flush_pending(pending: &mut Vec<Inline>, blocks: &mut Vec<Block>) {
    if !pending.is_empty() {
        blocks.push(Block::Paragraph(mem::take(pending)));
    }
}

fn list_from_nodes(start: Option<u64>, children: Vec<Node>) -> Result<Block, RichCompileError> {
    let mut items = Vec::new();
    for child in children {
        let Node::Container(Container::Item, mut item_nodes) = child else {
            continue;
        };
        let checked = take_task_marker(&mut item_nodes);
        let mut blocks = nodes_to_blocks(item_nodes)?;
        if blocks.is_empty() {
            blocks.push(Block::Paragraph(Vec::new()));
        }
        items.push(ListItem { checked, blocks });
    }
    Ok(Block::List { start, items })
}

fn take_task_marker(nodes: &mut Vec<Node>) -> Option<bool> {
    let mut index = 0;
    while index < nodes.len() {
        match &mut nodes[index] {
            Node::TaskMarker(checked) => {
                let checked = *checked;
                nodes.remove(index);
                return Some(checked);
            }
            // Never steal a marker from a nested list item.
            Node::Container(Container::List(_), _) => {}
            Node::Container(_, children) => {
                if let Some(checked) = take_task_marker(children) {
                    return Some(checked);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn table_from_nodes(
    alignments: Vec<CellAlign>,
    children: Vec<Node>,
) -> Result<Block, RichCompileError> {
    let mut rows = Vec::new();
    for child in children {
        match child {
            Node::Container(Container::TableHead, row_nodes) => {
                if let Some(row) = table_row_from_nodes(row_nodes, true, &alignments)? {
                    rows.push(row);
                }
            }
            Node::Container(Container::TableRow, row_nodes) => {
                if let Some(row) = table_row_from_nodes(row_nodes, false, &alignments)? {
                    rows.push(row);
                }
            }
            _ => {}
        }
    }
    Ok(Block::Table { rows })
}

fn table_row_from_nodes(
    row_nodes: Vec<Node>,
    is_header: bool,
    alignments: &[CellAlign],
) -> Result<Option<TableRow>, RichCompileError> {
    // Be tolerant if a parser version wraps header cells in a row node.
    if let [Node::Container(Container::TableRow, _)] = row_nodes.as_slice() {
        let Some(Node::Container(Container::TableRow, nested)) = row_nodes.into_iter().next()
        else {
            return Err(RichCompileError::InvalidDocument);
        };
        return table_row_from_nodes(nested, is_header, alignments);
    }

    let mut cells = Vec::new();
    for node in row_nodes {
        let Node::Container(Container::TableCell, children) = node else {
            continue;
        };
        if cells.len() >= RICH_MESSAGE_MAX_TABLE_COLUMNS {
            return Err(RichCompileError::TableColumnLimitExceeded {
                columns: cells.len() + 1,
            });
        }
        let align = alignments
            .get(cells.len())
            .copied()
            .unwrap_or(CellAlign::Left);
        cells.push(TableCell {
            text: nodes_to_inlines(children),
            is_header,
            align,
        });
    }

    Ok((!cells.is_empty()).then_some(TableRow { cells }))
}

fn nodes_to_inlines(nodes: Vec<Node>) -> Vec<Inline> {
    let mut inlines = Vec::new();
    for node in nodes {
        inline_from_node(node, &mut inlines);
    }
    inlines
}

fn inline_from_node(node: Node, output: &mut Vec<Inline>) {
    match node {
        Node::Text(text) => push_inline(output, Inline::Text(text)),
        Node::Code(code) => push_inline(output, Inline::Code(code)),
        Node::InlineMath(expression) | Node::DisplayMath(expression) => {
            push_inline(output, Inline::Math(expression));
        }
        Node::Break => push_inline(output, Inline::Break),
        Node::Rule => push_inline(output, Inline::Text("—".to_owned())),
        Node::TaskMarker(_) => {}
        Node::Container(Container::Strong, children) => {
            push_inline(output, Inline::Strong(nodes_to_inlines(children)))
        }
        Node::Container(Container::Emphasis, children) => {
            push_inline(output, Inline::Emphasis(nodes_to_inlines(children)))
        }
        Node::Container(Container::Strikethrough, children) => {
            push_inline(output, Inline::Strikethrough(nodes_to_inlines(children)))
        }
        Node::Container(Container::Link(target), children) => {
            let text = nodes_to_inlines(children);
            if let Some(target) = SafeLink::parse(&target) {
                push_inline(output, Inline::Link { target, text });
            } else {
                for inline in text {
                    push_inline(output, inline);
                }
            }
        }
        // RichReply.media is an independent attachment channel and does not
        // replace a Markdown image target embedded in the response body.
        // Preserve safe image destinations together with their visible alt
        // text; unsafe schemes still degrade to alt text only.
        Node::Container(Container::Image(target), children) => {
            let alt = nodes_to_inlines(children);
            if let Some(target) = SafeLink::parse(&target) {
                push_inline(output, Inline::Image { target, alt });
            } else {
                for inline in alt {
                    push_inline(output, inline);
                }
            }
        }
        Node::Container(Container::RawHtml, children)
        | Node::Container(Container::Transparent, children)
        | Node::Container(Container::Paragraph, children)
        | Node::Container(Container::Heading(_), children)
        | Node::Container(Container::TableHead, children)
        | Node::Container(Container::TableRow, children)
        | Node::Container(Container::TableCell, children)
        | Node::Container(Container::Item, children) => {
            for inline in nodes_to_inlines(children) {
                push_inline(output, inline);
            }
        }
        Node::Container(Container::CodeBlock(_), children)
        | Node::Container(Container::BlockQuote, children)
        | Node::Container(Container::List(_), children)
        | Node::Container(Container::Table(_), children) => {
            let text = nodes_plain_text(&children);
            if !text.is_empty() {
                push_inline(output, Inline::Text(text));
            }
        }
    }
}

fn push_inline(output: &mut Vec<Inline>, inline: Inline) {
    if let Inline::Text(text) = inline {
        if text.is_empty() {
            return;
        }
        if let Some(Inline::Text(previous)) = output.last_mut() {
            previous.push_str(&text);
        } else {
            output.push(Inline::Text(text));
        }
    } else {
        output.push(inline);
    }
}

fn nodes_plain_text(nodes: &[Node]) -> String {
    let mut text = String::new();
    for node in nodes {
        match node {
            Node::Text(value)
            | Node::Code(value)
            | Node::InlineMath(value)
            | Node::DisplayMath(value) => text.push_str(value),
            Node::Break => text.push('\n'),
            Node::Rule => text.push_str("---"),
            Node::TaskMarker(checked) => {
                text.push_str(if *checked { "[x] " } else { "[ ] " });
            }
            Node::Container(_, children) => text.push_str(&nodes_plain_text(children)),
        }
    }
    text
}

fn append_labeled_target_plain(target: &SafeLink, label: &[Inline], output: &mut String) {
    let mut label_text = String::new();
    append_inlines_plain(label, &mut label_text);
    let href = target.href();
    if label_text.is_empty() {
        output.push_str(&href);
    } else if label_text == href {
        // Avoid turning an autolink into `url (url)`.
        output.push_str(&label_text);
    } else {
        output.push_str(&label_text);
        output.push_str(" (");
        output.push_str(&href);
        output.push(')');
    }
}

fn labeled_target_plain(target: &SafeLink, label: &[Inline]) -> String {
    let mut output = String::new();
    append_labeled_target_plain(target, label, &mut output);
    output
}

fn inline_chars(inlines: &[Inline]) -> usize {
    inlines
        .iter()
        .map(|inline| match inline {
            Inline::Text(text) | Inline::Code(text) | Inline::Math(text) => text.chars().count(),
            Inline::Strong(children)
            | Inline::Emphasis(children)
            | Inline::Strikethrough(children) => inline_chars(children),
            Inline::Link { target, text } => labeled_target_plain(target, text).chars().count(),
            Inline::Image { target, alt } => labeled_target_plain(target, alt).chars().count(),
            Inline::Break => 1,
        })
        .sum()
}

fn append_inlines_plain(inlines: &[Inline], output: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Text(text) | Inline::Code(text) | Inline::Math(text) => output.push_str(text),
            Inline::Strong(children)
            | Inline::Emphasis(children)
            | Inline::Strikethrough(children) => append_inlines_plain(children, output),
            Inline::Link { target, text } => append_labeled_target_plain(target, text, output),
            Inline::Image { target, alt } => append_labeled_target_plain(target, alt, output),
            Inline::Break => output.push('\n'),
        }
    }
}

fn append_block_plain(block: &Block, output: &mut String) {
    match block {
        Block::Paragraph(inlines) | Block::Heading { text: inlines, .. } => {
            append_inlines_plain(inlines, output)
        }
        Block::Pre { text, .. } | Block::Math(text) => output.push_str(text),
        Block::Divider => output.push_str("---"),
        Block::Quote(blocks) => {
            for (index, block) in blocks.iter().enumerate() {
                if index > 0 {
                    output.push('\n');
                }
                append_block_plain(block, output);
            }
        }
        Block::List { start, items } => {
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.push('\n');
                }
                if let Some(start) = start {
                    output.push_str(&format!("{}. ", start.saturating_add(index as u64)));
                } else {
                    output.push_str("- ");
                }
                if let Some(checked) = item.checked {
                    output.push_str(if checked { "[x] " } else { "[ ] " });
                }
                for (block_index, block) in item.blocks.iter().enumerate() {
                    if block_index > 0 {
                        output.push('\n');
                    }
                    append_block_plain(block, output);
                }
            }
        }
        Block::Table { rows } => {
            for (row_index, row) in rows.iter().enumerate() {
                if row_index > 0 {
                    output.push('\n');
                }
                for (cell_index, cell) in row.cells.iter().enumerate() {
                    if cell_index > 0 {
                        output.push_str(" | ");
                    }
                    append_inlines_plain(&cell.text, output);
                }
            }
        }
    }
}

fn split_chars(value: &str, max_chars: usize) -> Vec<String> {
    if value.is_empty() {
        return vec![String::new()];
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut count = 0usize;
    for (index, _) in value.char_indices() {
        if count == max_chars {
            chunks.push(value[start..index].to_string());
            start = index;
            count = 0;
        }
        count += 1;
    }
    chunks.push(value[start..].to_string());
    chunks
}

fn flatten_oversized_block(block: Block) -> Vec<Block> {
    let mut text = String::new();
    append_block_plain(&block, &mut text);
    split_chars(&text, RICH_MESSAGE_MAX_CHARS)
        .into_iter()
        .map(|chunk| match &block {
            Block::Pre { language, .. } => Block::Pre {
                text: chunk,
                language: language.clone(),
            },
            Block::Math(_) => Block::Math(chunk),
            Block::Heading { size, .. } => Block::Heading {
                size: *size,
                text: vec![Inline::Text(chunk)],
            },
            _ => Block::Paragraph(vec![Inline::Text(chunk)]),
        })
        .collect()
}

fn render_segment(blocks: Vec<Block>, text_chars: usize) -> RichSegment {
    let typed_blocks = blocks.iter().map(block_to_value).collect();
    let mut html = String::new();
    render_blocks_html(&blocks, &mut html);
    RichSegment {
        blocks: typed_blocks,
        html,
        text_chars,
    }
}

fn block_to_value(block: &Block) -> Value {
    match block {
        Block::Paragraph(text) => json!({
            "type": "paragraph",
            "text": rich_text_value(text),
        }),
        Block::Heading { size, text } => json!({
            "type": "heading",
            "text": rich_text_value(text),
            "size": size,
        }),
        Block::Pre { text, language } => {
            let mut value = json!({
                "type": "pre",
                "text": text,
            });
            if let Some(language) = language {
                value["language"] = Value::String(language.clone());
            }
            value
        }
        Block::Divider => json!({ "type": "divider" }),
        Block::List { start, items } => {
            let items = items
                .iter()
                .enumerate()
                .map(|(index, item)| list_item_to_value(*start, index, item))
                .collect::<Vec<_>>();
            json!({
                "type": "list",
                "items": items,
            })
        }
        Block::Quote(blocks) => json!({
            "type": "blockquote",
            "blocks": blocks.iter().map(block_to_value).collect::<Vec<_>>(),
        }),
        Block::Table { rows } => json!({
            "type": "table",
            "cells": rows.iter().map(table_row_to_value).collect::<Vec<_>>(),
            "is_bordered": true,
            "is_striped": true,
        }),
        Block::Math(expression) => json!({
            "type": "mathematical_expression",
            "expression": expression,
        }),
    }
}

fn list_item_to_value(start: Option<u64>, index: usize, item: &ListItem) -> Value {
    let mut value = json!({
        "blocks": item.blocks.iter().map(block_to_value).collect::<Vec<_>>(),
    });
    if let Some(checked) = item.checked {
        value["has_checkbox"] = Value::Bool(true);
        if checked {
            value["is_checked"] = Value::Bool(true);
        }
    }
    if let Some(start) = start {
        let number = start.saturating_add(index as u64).min(i64::MAX as u64) as i64;
        value["value"] = Value::from(number);
        value["type"] = Value::String("1".to_owned());
    }
    value
}

fn table_row_to_value(row: &TableRow) -> Value {
    Value::Array(
        row.cells
            .iter()
            .map(|cell| {
                let mut value = json!({
                    "text": rich_text_value(&cell.text),
                    "align": cell.align.as_str(),
                    "valign": "top",
                });
                if cell.is_header {
                    value["is_header"] = Value::Bool(true);
                }
                value
            })
            .collect(),
    )
}

fn rich_text_value(inlines: &[Inline]) -> Value {
    let mut values = inlines.iter().map(inline_to_value).collect::<Vec<_>>();
    match values.len() {
        0 => Value::String(String::new()),
        1 => values.pop().unwrap_or(Value::String(String::new())),
        _ => Value::Array(values),
    }
}

fn inline_to_value(inline: &Inline) -> Value {
    match inline {
        Inline::Text(text) => Value::String(text.clone()),
        Inline::Strong(text) => json!({
            "type": "bold",
            "text": rich_text_value(text),
        }),
        Inline::Emphasis(text) => json!({
            "type": "italic",
            "text": rich_text_value(text),
        }),
        Inline::Strikethrough(text) => json!({
            "type": "strikethrough",
            "text": rich_text_value(text),
        }),
        Inline::Code(text) => json!({
            "type": "code",
            "text": text,
        }),
        Inline::Link { target, text } => match target {
            SafeLink::Url(url) => json!({
                "type": "url",
                "text": rich_text_value(text),
                "url": url,
            }),
            SafeLink::Email(address) => json!({
                "type": "email_address",
                "text": rich_text_value(text),
                "email_address": address,
            }),
        },
        Inline::Image { target, alt } => {
            let text = labeled_target_plain(target, alt);
            match target {
                SafeLink::Url(url) => json!({
                    "type": "url",
                    "text": text,
                    "url": url,
                }),
                SafeLink::Email(address) => json!({
                    "type": "email_address",
                    "text": text,
                    "email_address": address,
                }),
            }
        }
        Inline::Math(expression) => json!({
            "type": "mathematical_expression",
            "expression": expression,
        }),
        Inline::Break => Value::String("\n".to_owned()),
    }
}

fn render_blocks_html(blocks: &[Block], html: &mut String) {
    for block in blocks {
        match block {
            Block::Paragraph(text) => {
                html.push_str("<p>");
                render_inlines_html(text, html);
                html.push_str("</p>");
            }
            Block::Heading { size, text } => {
                let size = (*size).clamp(1, 6);
                html.push('<');
                html.push('h');
                html.push(char::from(b'0' + size));
                html.push('>');
                render_inlines_html(text, html);
                html.push_str("</h");
                html.push(char::from(b'0' + size));
                html.push('>');
            }
            Block::Pre { text, language } => {
                html.push_str("<pre>");
                if let Some(language) = language {
                    html.push_str("<code class=\"language-");
                    push_html_attribute(html, language);
                    html.push_str("\">");
                    push_html_text(html, text);
                    html.push_str("</code>");
                } else {
                    push_html_text(html, text);
                }
                html.push_str("</pre>");
            }
            Block::Divider => html.push_str("<hr/>"),
            Block::List { start, items } => render_list_html(*start, items, html),
            Block::Quote(blocks) => {
                html.push_str("<blockquote>");
                render_blocks_html(blocks, html);
                html.push_str("</blockquote>");
            }
            Block::Table { rows } => {
                html.push_str("<table bordered striped>");
                for row in rows {
                    html.push_str("<tr>");
                    for cell in &row.cells {
                        let tag = if cell.is_header { "th" } else { "td" };
                        html.push('<');
                        html.push_str(tag);
                        html.push_str(" align=\"");
                        html.push_str(cell.align.as_str());
                        html.push_str("\" valign=\"top\">");
                        render_inlines_html(&cell.text, html);
                        html.push_str("</");
                        html.push_str(tag);
                        html.push('>');
                    }
                    html.push_str("</tr>");
                }
                html.push_str("</table>");
            }
            Block::Math(expression) => {
                html.push_str("<tg-math-block>");
                push_html_text(html, expression);
                html.push_str("</tg-math-block>");
            }
        }
    }
}

fn render_list_html(start: Option<u64>, items: &[ListItem], html: &mut String) {
    if let Some(start) = start {
        html.push_str("<ol");
        if start != 1 {
            html.push_str(" start=\"");
            html.push_str(&start.min(i64::MAX as u64).to_string());
            html.push('"');
        }
        html.push('>');
    } else {
        html.push_str("<ul>");
    }

    for item in items {
        html.push_str("<li>");
        if let Some(checked) = item.checked {
            html.push_str("<input type=\"checkbox\"");
            if checked {
                html.push_str(" checked");
            }
            html.push('>');
        }
        if let [Block::Paragraph(text)] = item.blocks.as_slice() {
            render_inlines_html(text, html);
        } else {
            render_blocks_html(&item.blocks, html);
        }
        html.push_str("</li>");
    }

    html.push_str(if start.is_some() { "</ol>" } else { "</ul>" });
}

fn render_inlines_html(inlines: &[Inline], html: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => push_html_text(html, text),
            Inline::Strong(text) => {
                html.push_str("<b>");
                render_inlines_html(text, html);
                html.push_str("</b>");
            }
            Inline::Emphasis(text) => {
                html.push_str("<i>");
                render_inlines_html(text, html);
                html.push_str("</i>");
            }
            Inline::Strikethrough(text) => {
                html.push_str("<s>");
                render_inlines_html(text, html);
                html.push_str("</s>");
            }
            Inline::Code(text) => {
                html.push_str("<code>");
                push_html_text(html, text);
                html.push_str("</code>");
            }
            Inline::Link { target, text } => {
                html.push_str("<a href=\"");
                push_html_attribute(html, &target.href());
                html.push_str("\">");
                render_inlines_html(text, html);
                html.push_str("</a>");
            }
            Inline::Image { target, alt } => {
                html.push_str("<a href=\"");
                push_html_attribute(html, &target.href());
                html.push_str("\">");
                push_html_text(html, &labeled_target_plain(target, alt));
                html.push_str("</a>");
            }
            Inline::Math(expression) => {
                html.push_str("<tg-math>");
                push_html_text(html, expression);
                html.push_str("</tg-math>");
            }
            Inline::Break => html.push_str("<br>"),
        }
    }
}

fn push_html_text(output: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(character),
        }
    }
}

fn push_html_attribute(output: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(character),
        }
    }
}

fn count_json_blocks(blocks: &[Value]) -> usize {
    blocks
        .iter()
        .map(|block| {
            let mut count = 1usize;
            if let Some(nested) = block.get("blocks").and_then(Value::as_array) {
                count = count.saturating_add(count_json_blocks(nested));
            }
            if let Some(items) = block.get("items").and_then(Value::as_array) {
                for item in items {
                    count = count.saturating_add(1);
                    if let Some(nested) = item.get("blocks").and_then(Value::as_array) {
                        count = count.saturating_add(count_json_blocks(nested));
                    }
                }
            }
            if block.get("type").and_then(Value::as_str) == Some("table") {
                count = count.saturating_add(
                    block
                        .get("cells")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len),
                );
            }
            count
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_segment(markdown: &str) -> RichSegment {
        compile_markdown(markdown)
            .expect("Markdown should compile")
            .into_iter()
            .next()
            .expect("compiler always returns one segment")
    }

    #[test]
    fn compiles_required_typed_blocks_from_one_ir() {
        let segment = first_segment(
            "# Heading\n\nParagraph with **bold**.\n\n```rust\nfn main() {}\n```\n\n---\n\n> quote\n\n- [x] done\n- [ ] todo\n\n| A | B |\n| :- | -: |\n| 1 | 2 |\n\n$$E = mc^2$$",
        );
        let types = segment
            .blocks
            .iter()
            .filter_map(|block| block.get("type").and_then(Value::as_str))
            .collect::<Vec<_>>();

        assert!(types.contains(&"heading"));
        assert!(types.contains(&"paragraph"));
        assert!(types.contains(&"pre"));
        assert!(types.contains(&"divider"));
        assert!(types.contains(&"blockquote"));
        assert!(types.contains(&"list"));
        assert!(types.contains(&"table"));
        assert!(types.contains(&"mathematical_expression"));
        assert!(segment.block_count() > segment.blocks.len());
        assert!(segment.html.contains("<table bordered striped>"));
        assert!(segment.html.contains("<input type=\"checkbox\" checked>"));
    }

    #[test]
    fn filters_links_and_escapes_raw_html() {
        let segment = first_segment(
            "[web](https://example.com/?a=1&b=2) [mail](mailto:a+b@example.com) [bad](javascript:alert(1)) [credentials](https://user:pass@example.com/private) <tg-thinking>secret</tg-thinking>",
        );
        let typed = Value::Array(segment.blocks.clone()).to_string();

        assert!(typed.contains("https://example.com/"));
        assert!(typed.contains("email_address"));
        assert!(!typed.contains("javascript:"));
        assert!(!typed.contains("user:pass"));
        assert!(segment
            .html
            .contains("href=\"https://example.com/?a=1&amp;b=2\""));
        assert!(segment.html.contains("href=\"mailto:a+b@example.com\""));
        assert!(!segment.html.contains("href=\"javascript:"));
        assert!(!segment.html.contains("href=\"https://user:pass@"));
        assert!(!segment.html.contains("<tg-thinking>secret</tg-thinking>"));
        assert!(segment.html.contains("&lt;tg-thinking&gt;"));
    }

    #[test]
    fn segments_only_at_top_level_boundaries() {
        let markdown = format!("{}\n\n{}", "a".repeat(20_000), "b".repeat(20_000));
        let segments = compile_markdown(&markdown).expect("two paragraphs should segment");

        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text_chars(), 20_000);
        assert_eq!(segments[1].text_chars(), 20_000);
    }

    #[test]
    fn splits_an_indivisible_oversized_block_without_losing_text() {
        let text = "a".repeat(RICH_MESSAGE_MAX_CHARS + 1);
        let segments = compile_markdown(&text).expect("oversized paragraph should flatten safely");

        assert_eq!(segments.len(), 2);
        assert_eq!(
            segments.iter().map(RichSegment::text_chars).sum::<usize>(),
            text.chars().count()
        );
        assert!(segments
            .iter()
            .all(|segment| segment.text_chars() <= RICH_MESSAGE_MAX_CHARS));
    }

    #[test]
    fn oversized_paragraph_keeps_trailing_link_label_and_url_within_budget() {
        let url = "https://example.com/docs";
        let visible_link = format!("guide ({url})");
        let markdown = format!("{} [guide]({url})", "x".repeat(RICH_MESSAGE_MAX_CHARS));
        let segments = compile_markdown(&markdown).expect("oversized link should flatten safely");

        assert_eq!(segments.len(), 2);
        assert_eq!(segments[1].blocks[0]["text"], format!(" {visible_link}"));
        assert_eq!(
            segments.iter().map(RichSegment::text_chars).sum::<usize>(),
            RICH_MESSAGE_MAX_CHARS + 1 + visible_link.chars().count()
        );
        assert!(segments
            .iter()
            .all(|segment| segment.text_chars() <= RICH_MESSAGE_MAX_CHARS));
    }

    #[test]
    fn oversized_paragraph_keeps_trailing_image_alt_and_url_within_budget() {
        let url = "https://example.com/diagram.png";
        let visible_image = format!("diagram ({url})");
        let markdown = format!("{} ![diagram]({url})", "x".repeat(RICH_MESSAGE_MAX_CHARS));
        let segments = compile_markdown(&markdown).expect("oversized image should flatten safely");

        assert_eq!(segments.len(), 2);
        assert_eq!(segments[1].blocks[0]["text"], format!(" {visible_image}"));
        assert_eq!(
            segments.iter().map(RichSegment::text_chars).sum::<usize>(),
            RICH_MESSAGE_MAX_CHARS + 1 + visible_image.chars().count()
        );
        assert!(segments
            .iter()
            .all(|segment| segment.text_chars() <= RICH_MESSAGE_MAX_CHARS));
    }

    #[test]
    fn splits_oversized_fenced_code_into_valid_self_contained_segments() {
        let code = "x".repeat(RICH_MESSAGE_MAX_CHARS + 1);
        let markdown = format!("```rust\n{code}\n```");
        let segments = compile_markdown(&markdown).expect("oversized code should split safely");

        assert!(segments.len() >= 2);
        assert!(segments
            .iter()
            .all(|segment| segment.text_chars() <= RICH_MESSAGE_MAX_CHARS));
        assert!(segments
            .iter()
            .all(|segment| segment.html.starts_with("<pre>")));
        assert!(segments
            .iter()
            .all(|segment| segment.html.ends_with("</pre>")));
    }

    #[test]
    fn enforces_nesting_and_table_column_limits() {
        let nested = format!("{}deep", "> ".repeat(RICH_MESSAGE_MAX_DEPTH + 1));
        assert!(matches!(
            compile_markdown(&nested),
            Err(RichCompileError::NestingLimitExceeded)
        ));

        let allowed_inline_code = format!(
            "{}`code`",
            "> ".repeat(RICH_MESSAGE_MAX_DEPTH.saturating_sub(2))
        );
        assert!(compile_markdown(&allowed_inline_code).is_ok());
        for wrapped_leaf in ["`code`", "$x + y$", "text $$x + y$$"] {
            let too_deep = format!(
                "{}{wrapped_leaf}",
                "> ".repeat(RICH_MESSAGE_MAX_DEPTH.saturating_sub(1))
            );
            assert!(matches!(
                compile_markdown(&too_deep),
                Err(RichCompileError::NestingLimitExceeded)
            ));
        }

        let allowed_rule = format!(
            "{}---",
            "> ".repeat(RICH_MESSAGE_MAX_DEPTH.saturating_sub(1))
        );
        assert!(compile_markdown(&allowed_rule).is_ok());
        let too_deep_rule = format!("{}---", "> ".repeat(RICH_MESSAGE_MAX_DEPTH));
        assert!(matches!(
            compile_markdown(&too_deep_rule),
            Err(RichCompileError::NestingLimitExceeded)
        ));

        let header = vec!["h"; RICH_MESSAGE_MAX_TABLE_COLUMNS + 1].join(" | ");
        let separator = vec!["---"; RICH_MESSAGE_MAX_TABLE_COLUMNS + 1].join(" | ");
        let row = vec!["v"; RICH_MESSAGE_MAX_TABLE_COLUMNS + 1].join(" | ");
        let table = format!("| {header} |\n| {separator} |\n| {row} |");
        assert!(matches!(
            compile_markdown(&table),
            Err(RichCompileError::TableColumnLimitExceeded { columns: 21 })
        ));
    }

    #[test]
    fn splits_at_the_500_block_limit() {
        let markdown = vec!["---"; RICH_MESSAGE_MAX_BLOCKS + 1].join("\n\n");
        let segments = compile_markdown(&markdown).expect("dividers can split safely");

        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].blocks.len(), RICH_MESSAGE_MAX_BLOCKS);
        assert_eq!(segments[0].block_count(), RICH_MESSAGE_MAX_BLOCKS);
        assert_eq!(segments[1].blocks.len(), 1);
    }

    #[test]
    fn input_message_uses_exactly_one_format_and_controlled_thinking() {
        let segment = first_segment("hello");
        let blocks = segment.input_message(RichFormat::Blocks, Some(ThinkingLabel::RunningTools));
        let block_object = blocks.as_object().expect("object payload");
        assert!(block_object.contains_key("blocks"));
        assert!(!block_object.contains_key("html"));
        assert!(!block_object.contains_key("markdown"));
        assert_eq!(blocks["blocks"][0]["type"], "thinking");
        assert_eq!(blocks["skip_entity_detection"], true);

        let draft_html = segment.input_message(RichFormat::Html, Some(ThinkingLabel::Generating));
        assert!(draft_html["html"]
            .as_str()
            .is_some_and(|html| html.starts_with("<tg-thinking>Generating…</tg-thinking>")));

        let final_html = segment.input_message(RichFormat::Html, None);
        assert!(!final_html["html"]
            .as_str()
            .is_some_and(|html| html.contains("<tg-thinking>")));

        let finalizing = segment.input_message(RichFormat::Blocks, Some(ThinkingLabel::Finalizing));
        assert_eq!(finalizing["blocks"][0]["text"], "Finalizing…");
    }
}
