/// Convert Markdown to Google Chat native format.
///
/// Google Chat supports a subset of Markdown-like formatting:
/// - Bold: `*text*` (Google Chat) vs `**text**` (Markdown)
/// - Italic: `_text_` (Google Chat) vs `*text*` (Markdown)
/// - Strikethrough: `~text~` (Google Chat) vs `~~text~~` (Markdown)
/// - Code: `` `code` `` (same)
/// - Code blocks: ` ```code``` ` (same)
/// - Links: `<url|text>` (Google Chat) vs `[text](url)` (Markdown)
///
/// For simplicity, we pass through most markdown as-is since Google Chat
/// understands basic formatting. The main conversions are:
/// - `[text](url)` -> `<url|text>` for links
/// - `~~text~~` -> `~text~` for strikethrough
pub fn markdown_to_googlechat(md: &str) -> String {
    let mut result = String::with_capacity(md.len());
    let chars: Vec<char> = md.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Convert markdown links [text](url) to Google Chat <url|text>
        if chars[i] == '[' {
            if let Some((link_text, url, end_idx)) = parse_markdown_link(&chars, i) {
                result.push('<');
                result.push_str(&url);
                result.push('|');
                result.push_str(&link_text);
                result.push('>');
                i = end_idx;
                continue;
            }
        }

        // Convert double tilde ~~text~~ to single ~text~
        if i + 1 < len && chars[i] == '~' && chars[i + 1] == '~' {
            // Find closing ~~
            if let Some(close_idx) = find_double_char(&chars, i + 2, '~') {
                result.push('~');
                for j in (i + 2)..close_idx {
                    result.push(chars[j]);
                }
                result.push('~');
                i = close_idx + 2;
                continue;
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Try to parse a markdown link starting at position `start` (which should be '[').
/// Returns (link_text, url, end_index_exclusive) or None.
fn parse_markdown_link(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let len = chars.len();
    if start >= len || chars[start] != '[' {
        return None;
    }

    // Find closing ']'
    let mut depth = 1;
    let mut i = start + 1;
    while i < len && depth > 0 {
        match chars[i] {
            '[' => depth += 1,
            ']' => depth -= 1,
            _ => {}
        }
        if depth > 0 {
            i += 1;
        }
    }
    if depth != 0 || i >= len {
        return None;
    }
    let bracket_close = i;

    // Expect '(' immediately after ']'
    if bracket_close + 1 >= len || chars[bracket_close + 1] != '(' {
        return None;
    }

    // Find closing ')'
    let paren_open = bracket_close + 1;
    let mut paren_depth = 1;
    let mut j = paren_open + 1;
    while j < len && paren_depth > 0 {
        match chars[j] {
            '(' => paren_depth += 1,
            ')' => paren_depth -= 1,
            _ => {}
        }
        if paren_depth > 0 {
            j += 1;
        }
    }
    if paren_depth != 0 {
        return None;
    }

    let link_text: String = chars[(start + 1)..bracket_close].iter().collect();
    let url: String = chars[(paren_open + 1)..j].iter().collect();

    Some((link_text, url, j + 1))
}

/// Find a pair of `ch` characters starting from `start`.
/// Returns the index of the first character of the pair, or None.
fn find_double_char(chars: &[char], start: usize, ch: char) -> Option<usize> {
    let len = chars.len();
    let mut i = start;
    while i + 1 < len {
        if chars[i] == ch && chars[i + 1] == ch {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text_passthrough() {
        let input = "Hello, world!";
        assert_eq!(markdown_to_googlechat(input), input);
    }

    #[test]
    fn test_bold_passthrough() {
        // Google Chat uses *bold* natively, and **bold** also renders
        let input = "**bold text**";
        assert_eq!(markdown_to_googlechat(input), "**bold text**");
    }

    #[test]
    fn test_italic_passthrough() {
        let input = "_italic text_";
        assert_eq!(markdown_to_googlechat(input), "_italic text_");
    }

    #[test]
    fn test_code_passthrough() {
        let input = "`inline code`";
        assert_eq!(markdown_to_googlechat(input), input);
    }

    #[test]
    fn test_code_block_passthrough() {
        let input = "```rust\nfn main() {}\n```";
        assert_eq!(markdown_to_googlechat(input), input);
    }

    #[test]
    fn test_link_conversion() {
        let input = "[click here](https://example.com)";
        assert_eq!(
            markdown_to_googlechat(input),
            "<https://example.com|click here>"
        );
    }

    #[test]
    fn test_strikethrough_conversion() {
        let input = "~~deleted text~~";
        assert_eq!(markdown_to_googlechat(input), "~deleted text~");
    }

    #[test]
    fn test_mixed_content() {
        let input = "Hello **bold** and [link](https://example.com) with ~~strike~~";
        let expected = "Hello **bold** and <https://example.com|link> with ~strike~";
        assert_eq!(markdown_to_googlechat(input), expected);
    }

    #[test]
    fn test_empty_string() {
        assert_eq!(markdown_to_googlechat(""), "");
    }

    #[test]
    fn test_unicode() {
        let input = "你好世界 **加粗** [链接](https://example.com)";
        let expected = "你好世界 **加粗** <https://example.com|链接>";
        assert_eq!(markdown_to_googlechat(input), expected);
    }

    #[test]
    fn test_incomplete_link() {
        // Incomplete link syntax should pass through
        let input = "[text without url]";
        assert_eq!(markdown_to_googlechat(input), input);
    }

    #[test]
    fn test_single_tilde_passthrough() {
        let input = "~single tilde~";
        assert_eq!(markdown_to_googlechat(input), input);
    }
}
