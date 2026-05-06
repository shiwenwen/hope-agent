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
///
/// **Escape rules** (per <https://api.slack.com/reference/surfaces/formatting#escaping>):
/// raw `<` `>` `&` in user-supplied text MUST be replaced with `&lt;` `&gt;`
/// `&amp;` to avoid being parsed as control characters (mention/link/entity
/// delimiters). Inside code spans / code blocks Slack treats content as
/// preformatted but还需要转义这三字符以避免渲染异常。URL 部分（mrkdwn `<url|text>`
/// 的 url 段）不转义 `&`（URL 通常合法保留），但仍转义 `<` `>` 防止破坏语法。
pub fn markdown_to_mrkdwn(md: &str) -> String {
    let mut result = String::with_capacity(md.len());
    let mut in_code_block = false;
    let mut chars = md.chars().peekable();

    /// Slack mrkdwn 控制字符：`<` 触发 mention/link/channel-ref 解析（`<@>`
    /// `<#>` `<url>` `<url|text>`），`&` 触发 entity 解析。**这两个**必须转义。
    /// `>` 仅在**行首**有 blockquote 语义；行内出现无害。如果行首 `>` 也转
    /// 会丢失 blockquote 渲染——故 `>` 一律不转，blockquote 语义保留，业务侧
    /// raw `>` 不存在被错误解析的风险。
    fn push_escaped(out: &mut String, ch: char) {
        match ch {
            '<' => out.push_str("&lt;"),
            '&' => out.push_str("&amp;"),
            _ => out.push(ch),
        }
    }

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

        // Inside code block: 转义 `<>&` 防止 Slack mrkdwn 解析器把 `<` 当成
        // mention 起始；其它字符按原文。
        if in_code_block {
            push_escaped(&mut result, ch);
            continue;
        }

        // Inline code: `code` — 同上，content 仍要转义 `<>&`
        if ch == '`' {
            result.push('`');
            for c in chars.by_ref() {
                if c == '`' {
                    result.push('`');
                    break;
                }
                push_escaped(&mut result, c);
            }
            continue;
        }

        // Bold: **text** -> *text*；inner content 内的 `<` `&` 仍要 escape
        // 防 `**<@U123> & x**` 内的 mention/entity 起始字符破坏 Slack 解析
        if ch == '*' && chars.peek() == Some(&'*') {
            chars.next(); // consume second *
            result.push('*');
            while let Some(c) = chars.next() {
                if c == '*' && chars.peek() == Some(&'*') {
                    chars.next(); // consume closing **
                    break;
                }
                push_escaped(&mut result, c);
            }
            result.push('*');
            continue;
        }

        // Strikethrough: ~~text~~ -> ~text~；同样 escape inner content
        if ch == '~' && chars.peek() == Some(&'~') {
            chars.next(); // consume second ~
            result.push('~');
            while let Some(c) = chars.next() {
                if c == '~' && chars.peek() == Some(&'~') {
                    chars.next(); // consume closing ~~
                    break;
                }
                push_escaped(&mut result, c);
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
                for c in chars.by_ref() {
                    if c == ')' {
                        break;
                    }
                    url.push(c);
                }
                // mrkdwn `<url|text>`: URL 段中允许 `&`（query params 常用），但
                // `<` `>` 必须转义防止破坏语法；text 段全转
                result.push('<');
                for c in url.chars() {
                    match c {
                        '<' => result.push_str("&lt;"),
                        '>' => result.push_str("&gt;"),
                        _ => result.push(c),
                    }
                }
                result.push('|');
                for c in link_text.chars() {
                    push_escaped(&mut result, c);
                }
                result.push('>');
            } else {
                // Not a link, output as-is — but still escape contents
                result.push('[');
                for c in link_text.chars() {
                    push_escaped(&mut result, c);
                }
                if found_bracket {
                    result.push(']');
                }
            }
            continue;
        }

        // Everything else: 转义 `<` 与 `&` 后透传
        push_escaped(&mut result, ch);
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
    fn test_escape_lt_amp_in_plain_text() {
        // 用户输入含 < 必须转义防止被解析为 mention/link 起始；& 转义防止
        // entity；> 不转以保留 blockquote
        assert_eq!(
            markdown_to_mrkdwn("if a < b && c > d"),
            "if a &lt; b &amp;&amp; c > d"
        );
    }

    #[test]
    fn test_escape_inside_code_span() {
        assert_eq!(markdown_to_mrkdwn("`x < y`"), "`x &lt; y`");
    }

    #[test]
    fn test_escape_inside_code_block() {
        let input = "```\nx < y && z\n```";
        let expected = "```\nx &lt; y &amp;&amp; z\n```";
        assert_eq!(markdown_to_mrkdwn(input), expected);
    }

    #[test]
    fn test_escape_link_text_keeps_url_amp() {
        // URL 段保留 `&`（query params 常用），text 段转 `<`
        assert_eq!(
            markdown_to_mrkdwn("[a < b](https://x.com/?a=1&b=2)"),
            "<https://x.com/?a=1&b=2|a &lt; b>"
        );
    }

    #[test]
    fn test_blockquote_preserved() {
        // `>` 不被转义，blockquote 渲染保留
        assert_eq!(markdown_to_mrkdwn("> quoted"), "> quoted");
    }

    #[test]
    fn test_escape_inside_bold() {
        // `**<@U123> & x**` 内的 `<` `&` 必须转义，否则 mention/entity 起始
        // 字符破坏 Slack 解析；`>` 保持原样（行内不触发 blockquote）
        assert_eq!(
            markdown_to_mrkdwn("**<@U123> & x**"),
            "*&lt;@U123> &amp; x*"
        );
    }

    #[test]
    fn test_escape_inside_strikethrough() {
        assert_eq!(markdown_to_mrkdwn("~~<#C1> & y~~"), "~&lt;#C1> &amp; y~");
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
