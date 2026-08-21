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

/// Compile Google Chat's structured mention token for the standard-Markdown
/// body while leaving ordinary Markdown untouched. Tokens inside inline or
/// fenced code are intentionally not interpreted.
pub fn compile_standard_markdown_mentions(md: &str) -> String {
    let mut output = String::with_capacity(md.len());
    let bytes = md.as_bytes();
    let mut index = 0usize;
    let mut line_start = 0usize;
    let mut fence_delimiter = None;
    let mut inline_delimiter = None;
    while index < bytes.len() {
        if matches!(bytes[index], b'`' | b'~') {
            let delimiter = bytes[index];
            let run_length = bytes[index..]
                .iter()
                .take_while(|byte| **byte == delimiter)
                .count();
            let fence_position = index - line_start <= 3
                && bytes[line_start..index].iter().all(|byte| *byte == b' ');
            let line_end = bytes[index + run_length..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |offset| index + run_length + offset);
            let valid_fence_closer = fence_position
                && bytes[index + run_length..line_end]
                    .iter()
                    .all(|byte| matches!(*byte, b' ' | b'\t' | b'\r'));
            if let Some((opening_delimiter, opening_length)) = fence_delimiter {
                if delimiter == opening_delimiter
                    && run_length >= opening_length
                    && valid_fence_closer
                {
                    fence_delimiter = None;
                }
            } else if let Some(opening_length) = inline_delimiter {
                if delimiter == b'`' && run_length == opening_length {
                    inline_delimiter = None;
                }
            } else if run_length >= 3 && fence_position {
                fence_delimiter = Some((delimiter, run_length));
            } else if delimiter == b'`' {
                inline_delimiter = Some(run_length);
            }
            output.push_str(&md[index..index + run_length]);
            index += run_length;
            continue;
        }
        if fence_delimiter.is_none()
            && inline_delimiter.is_none()
            && !has_indented_code_prefix(&bytes[line_start..index])
            && !is_markdown_escaped(bytes, index)
            && bytes[index..].starts_with(b"<users/")
        {
            if let Some(relative_end) = bytes[index..].iter().position(|byte| *byte == b'>') {
                let end = index + relative_end;
                let identity = &md[index + "<users/".len()..end];
                if !identity.is_empty()
                    && identity.len() <= 320
                    && identity.chars().all(|ch| {
                        ch.is_ascii_alphanumeric() || matches!(ch, '@' | '.' | '_' | '-' | '+')
                    })
                {
                    output.push_str("<chat-user data-user=\"users/");
                    output.push_str(identity);
                    output.push_str("\">");
                    index = end + 1;
                    continue;
                }
            }
        }
        let ch = md[index..]
            .chars()
            .next()
            .expect("index remains on a UTF-8 boundary");
        output.push(ch);
        index += ch.len_utf8();
        if ch == '\n' {
            line_start = index;
        }
    }
    output
}

fn is_markdown_escaped(bytes: &[u8], index: usize) -> bool {
    bytes[..index]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

fn has_indented_code_prefix(line_prefix: &[u8]) -> bool {
    let mut columns = 0usize;
    for byte in line_prefix {
        match byte {
            b' ' => columns += 1,
            b'\t' => columns += 4 - (columns % 4),
            _ => break,
        }
        if columns >= 4 {
            return true;
        }
    }
    false
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

    #[test]
    fn standard_markdown_compiles_mentions_separately() {
        let input = "Hello <users/12345> and <users/all>";
        assert_eq!(
            compile_standard_markdown_mentions(input),
            "Hello <chat-user data-user=\"users/12345\"> and <chat-user data-user=\"users/all\">"
        );
    }

    #[test]
    fn standard_markdown_does_not_compile_mentions_inside_code() {
        let input = "`<users/123>`\n``<users/all>``\n````txt\n```\n<users/all>\n````";
        assert_eq!(compile_standard_markdown_mentions(input), input);
    }

    #[test]
    fn standard_markdown_requires_matching_inline_backtick_run() {
        let input = "``sample ` <users/all> sample`` then <users/123>";
        assert_eq!(
            compile_standard_markdown_mentions(input),
            "``sample ` <users/all> sample`` then <chat-user data-user=\"users/123\">"
        );
    }

    #[test]
    fn standard_markdown_requires_a_line_level_fence_closer() {
        let input = "```js\nconst marker = \"```\"; <users/all>\n```\n<users/123>";
        assert_eq!(
            compile_standard_markdown_mentions(input),
            "```js\nconst marker = \"```\"; <users/all>\n```\n<chat-user data-user=\"users/123\">"
        );
    }

    #[test]
    fn standard_markdown_preserves_mentions_inside_tilde_fences() {
        let input = "~~~md\n<users/all>\n```\n<users/123>\n~~~~\n<users/456>";
        assert_eq!(
            compile_standard_markdown_mentions(input),
            "~~~md\n<users/all>\n```\n<users/123>\n~~~~\n<chat-user data-user=\"users/456\">"
        );
    }

    #[test]
    fn standard_markdown_preserves_mentions_inside_indented_code() {
        let input = "    <users/all>\n\t<users/123>\n   <users/456>";
        assert_eq!(
            compile_standard_markdown_mentions(input),
            "    <users/all>\n\t<users/123>\n   <chat-user data-user=\"users/456\">"
        );
    }

    #[test]
    fn standard_markdown_preserves_escaped_mentions() {
        let input = r"\<users/all> \\<users/123> \\\<users/456>";
        assert_eq!(
            compile_standard_markdown_mentions(input),
            r#"\<users/all> \\<chat-user data-user="users/123"> \\\<users/456>"#
        );
    }

    #[test]
    fn standard_markdown_rejects_injectable_mention_identity() {
        let input = "<users/123\" onmouseover=\"x>";
        assert_eq!(compile_standard_markdown_mentions(input), input);
    }
}
