/// Convert Markdown text to Feishu-compatible plain text.
///
/// Feishu's text message type does not support Markdown rendering,
/// so we strip formatting markers while preserving the content.
pub fn markdown_to_feishu_text(md: &str) -> String {
    let mut result = String::with_capacity(md.len());

    // Process line by line for block-level patterns
    let lines: Vec<&str> = md.lines().collect();
    let total_lines = lines.len();

    let mut i = 0;
    while i < total_lines {
        let line = lines[i];
        let trimmed = line.trim_start();

        // Code block fences (```)
        if trimmed.starts_with("```") {
            // Skip the opening fence line
            i += 1;
            // Collect content until closing fence
            while i < total_lines {
                let inner = lines[i];
                if inner.trim_start().starts_with("```") {
                    i += 1; // skip closing fence
                    break;
                }
                result.push_str(inner);
                result.push('\n');
                i += 1;
            }
            continue;
        }

        // Heading: remove # prefix
        if trimmed.starts_with('#') {
            let content = trimmed.trim_start_matches('#').trim_start();
            result.push_str(&strip_inline_formatting(content));
            result.push('\n');
            i += 1;
            continue;
        }

        // Blockquote: remove > prefix
        if trimmed.starts_with('>') {
            let content = trimmed[1..].trim_start();
            result.push_str(&strip_inline_formatting(content));
            result.push('\n');
            i += 1;
            continue;
        }

        // Normal line: strip inline formatting
        result.push_str(&strip_inline_formatting(line));
        result.push('\n');
        i += 1;
    }

    // Remove trailing newline if the original didn't have one
    if !md.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// Strip inline Markdown formatting from a single line.
fn strip_inline_formatting(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            // Bold: **text** or __text__
            '*' if chars.peek() == Some(&'*') => {
                chars.next(); // consume second *
                let content = collect_until_marker(&mut chars, "**");
                result.push_str(&content);
            }
            // Italic: *text*
            '*' => {
                let content = collect_until_char(&mut chars, '*');
                result.push_str(&content);
            }
            // Strikethrough: ~~text~~
            '~' if chars.peek() == Some(&'~') => {
                chars.next(); // consume second ~
                let content = collect_until_marker(&mut chars, "~~");
                result.push_str(&content);
            }
            // Inline code: `code`
            '`' => {
                let content = collect_until_char(&mut chars, '`');
                result.push_str(&content);
            }
            // Links: [text](url)
            '[' => {
                let text = collect_until_char(&mut chars, ']');
                // Check for (url) following
                if chars.peek() == Some(&'(') {
                    chars.next(); // consume '('
                    let url = collect_until_char(&mut chars, ')');
                    if url.is_empty() {
                        result.push_str(&text);
                    } else {
                        result.push_str(&text);
                        result.push_str(" (");
                        result.push_str(&url);
                        result.push(')');
                    }
                } else {
                    // Not a link, restore brackets
                    result.push('[');
                    result.push_str(&text);
                    result.push(']');
                }
            }
            // Everything else passes through
            _ => result.push(c),
        }
    }

    result
}

/// Collect characters until a two-character marker is found.
/// Returns the collected content (marker consumed).
fn collect_until_marker(chars: &mut std::iter::Peekable<std::str::Chars>, marker: &str) -> String {
    let marker_chars: Vec<char> = marker.chars().collect();
    let mut buf = String::new();

    while let Some(&c) = chars.peek() {
        if c == marker_chars[0] {
            // Check if the next char matches the second char of the marker
            chars.next();
            if chars.peek() == Some(&marker_chars[1]) {
                chars.next(); // consume the second char
                return buf;
            } else {
                buf.push(c);
            }
        } else {
            buf.push(c);
            chars.next();
        }
    }

    buf
}

/// Collect characters until a single-character delimiter is found.
/// Returns the collected content (delimiter consumed).
fn collect_until_char(chars: &mut std::iter::Peekable<std::str::Chars>, delim: char) -> String {
    let mut buf = String::new();
    for c in chars.by_ref() {
        if c == delim {
            return buf;
        }
        buf.push(c);
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text_passthrough() {
        assert_eq!(markdown_to_feishu_text("hello world"), "hello world");
    }

    #[test]
    fn test_bold_stripped() {
        assert_eq!(
            markdown_to_feishu_text("this is **bold** text"),
            "this is bold text"
        );
    }

    #[test]
    fn test_italic_stripped() {
        assert_eq!(
            markdown_to_feishu_text("this is *italic* text"),
            "this is italic text"
        );
    }

    #[test]
    fn test_strikethrough_stripped() {
        assert_eq!(
            markdown_to_feishu_text("this is ~~struck~~ text"),
            "this is struck text"
        );
    }

    #[test]
    fn test_inline_code_stripped() {
        assert_eq!(
            markdown_to_feishu_text("run `cargo build` now"),
            "run cargo build now"
        );
    }

    #[test]
    fn test_code_block_stripped() {
        let input = "before\n```rust\nfn main() {}\n```\nafter";
        let expected = "before\nfn main() {}\nafter";
        assert_eq!(markdown_to_feishu_text(input), expected);
    }

    #[test]
    fn test_heading_stripped() {
        assert_eq!(markdown_to_feishu_text("# Title"), "Title");
        assert_eq!(markdown_to_feishu_text("## Subtitle"), "Subtitle");
        assert_eq!(markdown_to_feishu_text("### Deep"), "Deep");
    }

    #[test]
    fn test_blockquote_stripped() {
        assert_eq!(markdown_to_feishu_text("> quoted text"), "quoted text");
    }

    #[test]
    fn test_link_converted() {
        assert_eq!(
            markdown_to_feishu_text("[click here](https://example.com)"),
            "click here (https://example.com)"
        );
    }

    #[test]
    fn test_link_empty_url() {
        assert_eq!(markdown_to_feishu_text("[text]()"), "text");
    }

    #[test]
    fn test_mixed_formatting() {
        let input = "# Welcome\n\nThis is **bold** and *italic*.\n\n> A quote\n\n```\ncode\n```\n\n[link](https://x.com)";
        let expected =
            "Welcome\n\nThis is bold and italic.\n\nA quote\n\ncode\n\nlink (https://x.com)";
        assert_eq!(markdown_to_feishu_text(input), expected);
    }

    #[test]
    fn test_multiline_preserves_newlines() {
        assert_eq!(
            markdown_to_feishu_text("line1\nline2\nline3"),
            "line1\nline2\nline3"
        );
    }

    #[test]
    fn test_trailing_newline_preserved() {
        assert_eq!(markdown_to_feishu_text("hello\n"), "hello\n");
    }

    #[test]
    fn test_no_trailing_newline() {
        assert_eq!(markdown_to_feishu_text("hello"), "hello");
    }
}
