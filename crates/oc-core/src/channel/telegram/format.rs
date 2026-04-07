/// Convert Markdown text to Telegram-compatible HTML.
///
/// Telegram supports a limited subset of HTML:
/// <b>bold</b>, <i>italic</i>, <code>code</code>,
/// <pre><code class="language-X">block</code></pre>,
/// <a href="url">link</a>, <s>strike</s>,
/// <blockquote>quote</blockquote>, <tg-spoiler>spoiler</tg-spoiler>
pub fn markdown_to_telegram_html(md: &str) -> String {
    let mut result = String::with_capacity(md.len() * 2);
    let mut chars = md.chars().peekable();
    let mut in_code_block = false;
    let mut code_block_lang = String::new();

    while let Some(ch) = chars.next() {
        // Code blocks: ```lang\n...\n```
        if ch == '`' && chars.peek() == Some(&'`') {
            chars.next(); // consume second `
            if chars.peek() == Some(&'`') {
                chars.next(); // consume third `

                if in_code_block {
                    // Closing code block
                    result.push_str("</code></pre>");
                    in_code_block = false;
                    code_block_lang.clear();
                } else {
                    // Opening code block — read optional language
                    in_code_block = true;
                    code_block_lang.clear();
                    while let Some(&c) = chars.peek() {
                        if c == '\n' {
                            chars.next();
                            break;
                        }
                        code_block_lang.push(c);
                        chars.next();
                    }

                    if code_block_lang.is_empty() {
                        result.push_str("<pre><code>");
                    } else {
                        result.push_str(&format!(
                            "<pre><code class=\"language-{}\">",
                            escape_html_attr(&code_block_lang.trim())
                        ));
                    }
                }
                continue;
            } else {
                // Only two backticks — treat as inline code
                result.push_str("<code>");
                // Read until next ``
                let mut code_content = String::new();
                let mut found_close = false;
                while let Some(c) = chars.next() {
                    if c == '`' && chars.peek() == Some(&'`') {
                        chars.next();
                        found_close = true;
                        break;
                    }
                    code_content.push(c);
                }
                result.push_str(&escape_html(&code_content));
                result.push_str("</code>");
                if !found_close {
                    // Unclosed — just continue
                }
                continue;
            }
        }

        // Inside code block: pass through as-is (escaped)
        if in_code_block {
            result.push_str(&escape_html_char(ch));
            continue;
        }

        // Inline code: `code`
        if ch == '`' {
            result.push_str("<code>");
            while let Some(c) = chars.next() {
                if c == '`' {
                    break;
                }
                result.push_str(&escape_html_char(c));
            }
            result.push_str("</code>");
            continue;
        }

        // Bold: **text**
        if ch == '*' && chars.peek() == Some(&'*') {
            chars.next();
            result.push_str("<b>");
            while let Some(c) = chars.next() {
                if c == '*' && chars.peek() == Some(&'*') {
                    chars.next();
                    break;
                }
                result.push_str(&escape_html_char(c));
            }
            result.push_str("</b>");
            continue;
        }

        // Italic: *text* (single asterisk, not followed by another *)
        if ch == '*' {
            result.push_str("<i>");
            while let Some(c) = chars.next() {
                if c == '*' {
                    break;
                }
                result.push_str(&escape_html_char(c));
            }
            result.push_str("</i>");
            continue;
        }

        // Strikethrough: ~~text~~
        if ch == '~' && chars.peek() == Some(&'~') {
            chars.next();
            result.push_str("<s>");
            while let Some(c) = chars.next() {
                if c == '~' && chars.peek() == Some(&'~') {
                    chars.next();
                    break;
                }
                result.push_str(&escape_html_char(c));
            }
            result.push_str("</s>");
            continue;
        }

        // Links: [text](url)
        if ch == '[' {
            let mut link_text = String::new();
            let mut found_bracket = false;
            while let Some(c) = chars.next() {
                if c == ']' {
                    found_bracket = true;
                    break;
                }
                link_text.push(c);
            }

            if found_bracket && chars.peek() == Some(&'(') {
                chars.next(); // consume (
                let mut url = String::new();
                while let Some(c) = chars.next() {
                    if c == ')' {
                        break;
                    }
                    url.push(c);
                }
                result.push_str(&format!(
                    "<a href=\"{}\">{}</a>",
                    escape_html_attr(&url),
                    escape_html(&link_text)
                ));
            } else {
                // Not a link, output as-is
                result.push('[');
                result.push_str(&escape_html(&link_text));
                if found_bracket {
                    result.push(']');
                }
            }
            continue;
        }

        // Blockquote: > text (at start of line)
        if ch == '>' && (result.is_empty() || result.ends_with('\n')) {
            // Consume optional space after >
            if chars.peek() == Some(&' ') {
                chars.next();
            }
            result.push_str("<blockquote>");
            let mut quote_content = String::new();
            while let Some(c) = chars.next() {
                if c == '\n' {
                    // Check if next line also starts with >
                    if chars.peek() == Some(&'>') {
                        quote_content.push('\n');
                        chars.next(); // consume >
                        if chars.peek() == Some(&' ') {
                            chars.next(); // consume space
                        }
                    } else {
                        break;
                    }
                } else {
                    quote_content.push(c);
                }
            }
            result.push_str(&escape_html(&quote_content));
            result.push_str("</blockquote>\n");
            continue;
        }

        // Heading: # text → bold (Telegram doesn't support headings)
        if ch == '#' && (result.is_empty() || result.ends_with('\n')) {
            // Consume all # and optional space
            while chars.peek() == Some(&'#') {
                chars.next();
            }
            if chars.peek() == Some(&' ') {
                chars.next();
            }
            result.push_str("<b>");
            while let Some(c) = chars.next() {
                if c == '\n' {
                    break;
                }
                result.push_str(&escape_html_char(c));
            }
            result.push_str("</b>\n");
            continue;
        }

        // Default: escape and pass through
        result.push_str(&escape_html_char(ch));
    }

    // Close any unclosed code block
    if in_code_block {
        result.push_str("</code></pre>");
    }

    result
}

/// Escape a single character for HTML.
fn escape_html_char(ch: char) -> String {
    match ch {
        '&' => "&amp;".to_string(),
        '<' => "&lt;".to_string(),
        '>' => "&gt;".to_string(),
        '"' => "&quot;".to_string(),
        _ => ch.to_string(),
    }
}

/// Escape a string for HTML content.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape a string for HTML attribute values.
fn escape_html_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bold() {
        assert_eq!(markdown_to_telegram_html("**bold**"), "<b>bold</b>");
    }

    #[test]
    fn test_italic() {
        assert_eq!(markdown_to_telegram_html("*italic*"), "<i>italic</i>");
    }

    #[test]
    fn test_inline_code() {
        assert_eq!(markdown_to_telegram_html("`code`"), "<code>code</code>");
    }

    #[test]
    fn test_code_block() {
        let input = "```rust\nfn main() {}\n```";
        let expected = "<pre><code class=\"language-rust\">fn main() {}</code></pre>";
        assert_eq!(markdown_to_telegram_html(input), expected);
    }

    #[test]
    fn test_code_block_no_lang() {
        let input = "```\nhello\n```";
        let expected = "<pre><code>hello</code></pre>";
        assert_eq!(markdown_to_telegram_html(input), expected);
    }

    #[test]
    fn test_link() {
        let input = "[click](https://example.com)";
        let expected = "<a href=\"https://example.com\">click</a>";
        assert_eq!(markdown_to_telegram_html(input), expected);
    }

    #[test]
    fn test_strikethrough() {
        assert_eq!(markdown_to_telegram_html("~~strike~~"), "<s>strike</s>");
    }

    #[test]
    fn test_heading_to_bold() {
        assert_eq!(markdown_to_telegram_html("## Title"), "<b>Title</b>\n");
    }

    #[test]
    fn test_blockquote() {
        let input = "> quote text";
        assert!(markdown_to_telegram_html(input).contains("<blockquote>"));
    }

    #[test]
    fn test_html_escaping() {
        assert_eq!(markdown_to_telegram_html("a < b & c"), "a &lt; b &amp; c");
    }

    #[test]
    fn test_mixed() {
        let input = "Hello **world**, check `code` and [link](http://x.com)";
        let result = markdown_to_telegram_html(input);
        assert!(result.contains("<b>world</b>"));
        assert!(result.contains("<code>code</code>"));
        assert!(result.contains("<a href=\"http://x.com\">link</a>"));
    }
}
