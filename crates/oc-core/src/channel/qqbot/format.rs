/// Convert Markdown text to QQ Bot plain text.
///
/// QQ Bot has limited markdown support, so we strip formatting markers
/// and keep the content as plain text. This is the same approach used
/// by the Feishu channel plugin.
pub fn markdown_to_qqbot_text(md: &str) -> String {
    let mut result = String::with_capacity(md.len());
    let mut chars = md.chars().peekable();
    let mut in_code_block = false;

    while let Some(ch) = chars.next() {
        // Code blocks: ```lang\n...\n```
        if ch == '`' && chars.peek() == Some(&'`') {
            chars.next(); // consume second `
            if chars.peek() == Some(&'`') {
                chars.next(); // consume third `

                if in_code_block {
                    // Closing code block
                    in_code_block = false;
                } else {
                    // Opening code block — skip the language tag
                    in_code_block = true;
                    while let Some(&c) = chars.peek() {
                        if c == '\n' {
                            chars.next();
                            break;
                        }
                        chars.next();
                    }
                }
                continue;
            } else {
                // Two backticks — treat as inline code, extract content
                while let Some(c) = chars.next() {
                    if c == '`' && chars.peek() == Some(&'`') {
                        chars.next();
                        break;
                    }
                    result.push(c);
                }
                continue;
            }
        }

        // Inside code block: pass through content as-is
        if in_code_block {
            result.push(ch);
            continue;
        }

        // Inline code: `code` — strip backticks, keep content
        if ch == '`' {
            while let Some(c) = chars.next() {
                if c == '`' {
                    break;
                }
                result.push(c);
            }
            continue;
        }

        // Bold: **text** — strip markers, keep content
        if ch == '*' && chars.peek() == Some(&'*') {
            chars.next(); // consume second *
            while let Some(c) = chars.next() {
                if c == '*' && chars.peek() == Some(&'*') {
                    chars.next();
                    break;
                }
                result.push(c);
            }
            continue;
        }

        // Italic: *text* — strip markers, keep content
        if ch == '*' {
            while let Some(c) = chars.next() {
                if c == '*' {
                    break;
                }
                result.push(c);
            }
            continue;
        }

        // Strikethrough: ~~text~~ — strip markers, keep content
        if ch == '~' && chars.peek() == Some(&'~') {
            chars.next(); // consume second ~
            while let Some(c) = chars.next() {
                if c == '~' && chars.peek() == Some(&'~') {
                    chars.next();
                    break;
                }
                result.push(c);
            }
            continue;
        }

        // Links: [text](url) — keep text, discard URL
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
                // Skip the URL
                let mut depth = 1;
                while let Some(c) = chars.next() {
                    if c == '(' {
                        depth += 1;
                    } else if c == ')' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
                result.push_str(&link_text);
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

        // Blockquote: > text — strip the > marker
        if ch == '>' && (result.is_empty() || result.ends_with('\n')) {
            if chars.peek() == Some(&' ') {
                chars.next(); // consume space after >
            }
            // Content continues on this line, just let it through
            continue;
        }

        // Heading: # text — strip # markers
        if ch == '#' && (result.is_empty() || result.ends_with('\n')) {
            while chars.peek() == Some(&'#') {
                chars.next();
            }
            if chars.peek() == Some(&' ') {
                chars.next();
            }
            // Content continues on this line
            continue;
        }

        // Default: pass through
        result.push(ch);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text() {
        assert_eq!(markdown_to_qqbot_text("Hello, world!"), "Hello, world!");
    }

    #[test]
    fn test_bold() {
        assert_eq!(markdown_to_qqbot_text("**bold**"), "bold");
    }

    #[test]
    fn test_italic() {
        assert_eq!(markdown_to_qqbot_text("*italic*"), "italic");
    }

    #[test]
    fn test_inline_code() {
        assert_eq!(markdown_to_qqbot_text("`code`"), "code");
    }

    #[test]
    fn test_code_block() {
        let input = "```rust\nfn main() {}\n```";
        assert_eq!(markdown_to_qqbot_text(input), "fn main() {}\n");
    }

    #[test]
    fn test_code_block_no_lang() {
        let input = "```\nhello\n```";
        assert_eq!(markdown_to_qqbot_text(input), "hello\n");
    }

    #[test]
    fn test_link() {
        let input = "[click here](https://example.com)";
        assert_eq!(markdown_to_qqbot_text(input), "click here");
    }

    #[test]
    fn test_strikethrough() {
        assert_eq!(markdown_to_qqbot_text("~~deleted~~"), "deleted");
    }

    #[test]
    fn test_heading() {
        assert_eq!(markdown_to_qqbot_text("## Title"), "Title");
    }

    #[test]
    fn test_blockquote() {
        assert_eq!(markdown_to_qqbot_text("> quoted text"), "quoted text");
    }

    #[test]
    fn test_mixed() {
        let input = "Hello **world**, check `code` and [link](http://x.com)";
        let result = markdown_to_qqbot_text(input);
        assert_eq!(result, "Hello world, check code and link");
    }

    #[test]
    fn test_empty() {
        assert_eq!(markdown_to_qqbot_text(""), "");
    }

    #[test]
    fn test_unicode() {
        let input = "**加粗** 你好世界";
        assert_eq!(markdown_to_qqbot_text(input), "加粗 你好世界");
    }

    #[test]
    fn test_multiline_blockquote() {
        let input = "> line 1\n> line 2";
        assert_eq!(markdown_to_qqbot_text(input), "line 1\nline 2");
    }
}
