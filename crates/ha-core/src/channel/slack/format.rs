/// Convert standard Markdown to Slack's mrkdwn format.
///
/// Slack mrkdwn differences from standard Markdown:
/// - Bold: `*text*` (not `**text**`)
/// - Italic: `_text_` (same)
/// - Strike: `~text~` (not `~~text~~`)
/// - Code: `` `text` `` (same)
/// - Code block: ` ```text``` ` (same)
/// - Links: `<url|text>` (not `[text](url)`)
/// - Blockquote: `>` (same)
pub fn markdown_to_mrkdwn(md: &str) -> String {
    let mut result = String::with_capacity(md.len());
    let mut in_code_block = false;
    let mut chars = md.chars().peekable();

    while let Some(ch) = chars.next() {
        // Handle code blocks: ``` ... ```
        if ch == '`' && chars.peek() == Some(&'`') {
            chars.next(); // consume second `
            if chars.peek() == Some(&'`') {
                chars.next(); // consume third `

                if in_code_block {
                    // Closing code block
                    result.push_str("```");
                    in_code_block = false;
                } else {
                    // Opening code block - pass through with optional language
                    result.push_str("```");
                    in_code_block = true;
                }
                continue;
            } else {
                // Only two backticks - treat as inline code (pass through)
                result.push('`');
                result.push('`');
                continue;
            }
        }

        // Inside code block: pass through unchanged
        if in_code_block {
            result.push(ch);
            continue;
        }

        // Inline code: `code` - pass through unchanged
        if ch == '`' {
            result.push('`');
            while let Some(c) = chars.next() {
                result.push(c);
                if c == '`' {
                    break;
                }
            }
            continue;
        }

        // Bold: **text** -> *text*
        if ch == '*' && chars.peek() == Some(&'*') {
            chars.next(); // consume second *
            result.push('*');
            while let Some(c) = chars.next() {
                if c == '*' && chars.peek() == Some(&'*') {
                    chars.next(); // consume closing **
                    break;
                }
                result.push(c);
            }
            result.push('*');
            continue;
        }

        // Strikethrough: ~~text~~ -> ~text~
        if ch == '~' && chars.peek() == Some(&'~') {
            chars.next(); // consume second ~
            result.push('~');
            while let Some(c) = chars.next() {
                if c == '~' && chars.peek() == Some(&'~') {
                    chars.next(); // consume closing ~~
                    break;
                }
                result.push(c);
            }
            result.push('~');
            continue;
        }

        // Links: [text](url) -> <url|text>
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
                result.push('<');
                result.push_str(&url);
                result.push('|');
                result.push_str(&link_text);
                result.push('>');
            } else {
                // Not a link, output as-is
                result.push('[');
                result.push_str(&link_text);
                if found_bracket {
                    result.push(']');
                }
            }
            continue;
        }

        // Everything else: pass through
        result.push(ch);
    }

    // Close any unclosed code block
    if in_code_block {
        result.push_str("```");
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bold_conversion() {
        assert_eq!(markdown_to_mrkdwn("**bold**"), "*bold*");
    }

    #[test]
    fn test_italic_passthrough() {
        // Single underscore italic is the same in both formats
        assert_eq!(markdown_to_mrkdwn("_italic_"), "_italic_");
    }

    #[test]
    fn test_strikethrough_conversion() {
        assert_eq!(markdown_to_mrkdwn("~~strike~~"), "~strike~");
    }

    #[test]
    fn test_link_conversion() {
        assert_eq!(
            markdown_to_mrkdwn("[click here](https://example.com)"),
            "<https://example.com|click here>"
        );
    }

    #[test]
    fn test_inline_code_passthrough() {
        assert_eq!(markdown_to_mrkdwn("`code`"), "`code`");
    }

    #[test]
    fn test_code_block_passthrough() {
        let input = "```rust\nfn main() {}\n```";
        assert_eq!(markdown_to_mrkdwn(input), input);
    }

    #[test]
    fn test_code_block_no_lang() {
        let input = "```\nhello\n```";
        assert_eq!(markdown_to_mrkdwn(input), input);
    }

    #[test]
    fn test_blockquote_passthrough() {
        let input = "> this is a quote";
        assert_eq!(markdown_to_mrkdwn(input), input);
    }

    #[test]
    fn test_bold_inside_code_block_unchanged() {
        let input = "```\n**not bold**\n```";
        assert_eq!(markdown_to_mrkdwn(input), input);
    }

    #[test]
    fn test_link_inside_code_block_unchanged() {
        let input = "```\n[text](url)\n```";
        assert_eq!(markdown_to_mrkdwn(input), input);
    }

    #[test]
    fn test_strike_inside_code_block_unchanged() {
        let input = "```\n~~not strike~~\n```";
        assert_eq!(markdown_to_mrkdwn(input), input);
    }

    #[test]
    fn test_bold_inside_inline_code_unchanged() {
        // Inside inline code, content is passed through
        assert_eq!(markdown_to_mrkdwn("`**bold**`"), "`**bold**`");
    }

    #[test]
    fn test_mixed_formatting() {
        let input = "Hello **world**, check `code` and [link](http://x.com)";
        let result = markdown_to_mrkdwn(input);
        assert!(result.contains("*world*"));
        assert!(result.contains("`code`"));
        assert!(result.contains("<http://x.com|link>"));
    }

    #[test]
    fn test_nested_bold_and_link() {
        // Bold wrapping a link isn't deeply nested - each is handled sequentially
        let input = "**bold** and [text](url) and ~~strike~~";
        let result = markdown_to_mrkdwn(input);
        assert!(result.contains("*bold*"));
        assert!(result.contains("<url|text>"));
        assert!(result.contains("~strike~"));
    }

    #[test]
    fn test_plain_text_passthrough() {
        let input = "Just plain text with no formatting.";
        assert_eq!(markdown_to_mrkdwn(input), input);
    }

    #[test]
    fn test_empty_string() {
        assert_eq!(markdown_to_mrkdwn(""), "");
    }

    #[test]
    fn test_bracket_without_link() {
        // Square bracket not followed by parenthesis
        assert_eq!(markdown_to_mrkdwn("[not a link]"), "[not a link]");
    }

    #[test]
    fn test_multiple_links() {
        let input = "[a](http://a.com) and [b](http://b.com)";
        let result = markdown_to_mrkdwn(input);
        assert_eq!(result, "<http://a.com|a> and <http://b.com|b>");
    }

    #[test]
    fn test_unclosed_code_block() {
        let input = "```\nunclosed";
        let result = markdown_to_mrkdwn(input);
        assert_eq!(result, "```\nunclosed```");
    }

    #[test]
    fn test_multiline_with_formatting() {
        let input = "Line 1 **bold**\nLine 2 ~~strike~~\nLine 3 [link](url)";
        let result = markdown_to_mrkdwn(input);
        assert!(result.contains("*bold*"));
        assert!(result.contains("~strike~"));
        assert!(result.contains("<url|link>"));
        assert!(result.contains('\n'));
    }
}
