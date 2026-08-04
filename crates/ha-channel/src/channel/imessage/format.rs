/// Convert Markdown text to iMessage-compatible plain text.
///
/// iMessage has no rich text formatting in its API, so we strip all Markdown
/// syntax and return plain text.
pub fn markdown_to_imessage(markdown: &str) -> String {
    let mut result = String::with_capacity(markdown.len());
    let mut chars = markdown.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            // Bold/italic markers: skip * and _ when used as formatting
            '*' | '_' => {
                // Consume consecutive identical markers (**, ***, __, ___)
                while chars.peek() == Some(&ch) {
                    chars.next();
                }
            }
            // Strikethrough: ~~text~~ (single ~ falls through to the `_` arm)
            '~' if chars.peek() == Some(&'~') => {
                chars.next(); // consume second ~
            }
            // Inline code: `code`
            '`' => {
                // Skip ``` (code block markers)
                if chars.peek() == Some(&'`') {
                    chars.next();
                    if chars.peek() == Some(&'`') {
                        chars.next();
                        // Skip optional language identifier after ```
                        // Consume until newline
                        while let Some(&c) = chars.peek() {
                            if c == '\n' {
                                break;
                            }
                            chars.next();
                        }
                    }
                }
                // Single backtick: just skip it
            }
            // Headers: # at start of line
            '#' => {
                // Consume consecutive # and trailing space
                while chars.peek() == Some(&'#') {
                    chars.next();
                }
                if chars.peek() == Some(&' ') {
                    chars.next();
                }
            }
            // Links: [text](url) -> text (url)
            '[' => {
                let mut link_text = String::new();
                let mut found_close = false;
                while let Some(&c) = chars.peek() {
                    if c == ']' {
                        chars.next();
                        found_close = true;
                        break;
                    }
                    link_text.push(c);
                    chars.next();
                }
                if found_close && chars.peek() == Some(&'(') {
                    chars.next(); // consume (
                    let mut url = String::new();
                    while let Some(&c) = chars.peek() {
                        if c == ')' {
                            chars.next();
                            break;
                        }
                        url.push(c);
                        chars.next();
                    }
                    result.push_str(&link_text);
                    if !url.is_empty() {
                        result.push_str(" (");
                        result.push_str(&url);
                        result.push(')');
                    }
                } else {
                    // Not a valid link, emit as-is
                    result.push('[');
                    result.push_str(&link_text);
                    if found_close {
                        result.push(']');
                    }
                }
            }
            // Images: ![alt](url) -> alt (url)
            '!' if chars.peek() == Some(&'[') => {
                chars.next(); // consume [
                let mut alt_text = String::new();
                let mut found_close = false;
                while let Some(&c) = chars.peek() {
                    if c == ']' {
                        chars.next();
                        found_close = true;
                        break;
                    }
                    alt_text.push(c);
                    chars.next();
                }
                if found_close && chars.peek() == Some(&'(') {
                    chars.next(); // consume (
                    let mut url = String::new();
                    while let Some(&c) = chars.peek() {
                        if c == ')' {
                            chars.next();
                            break;
                        }
                        url.push(c);
                        chars.next();
                    }
                    if !alt_text.is_empty() {
                        result.push_str(&alt_text);
                    }
                    if !url.is_empty() {
                        if !alt_text.is_empty() {
                            result.push_str(" (");
                            result.push_str(&url);
                            result.push(')');
                        } else {
                            result.push_str(&url);
                        }
                    }
                } else {
                    result.push('!');
                    result.push('[');
                    result.push_str(&alt_text);
                    if found_close {
                        result.push(']');
                    }
                }
            }
            // Blockquote: > at start of line
            '>' if result.is_empty() || result.ends_with('\n') => {
                if chars.peek() == Some(&' ') {
                    chars.next();
                }
            }
            // Horizontal rule: --- or *** or ___ (at least 3)
            // We just pass through; these are rare edge cases
            // Everything else: pass through
            _ => {
                result.push(ch);
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text_passthrough() {
        assert_eq!(markdown_to_imessage("Hello world"), "Hello world");
    }

    #[test]
    fn test_strip_bold() {
        assert_eq!(markdown_to_imessage("**bold text**"), "bold text");
    }

    #[test]
    fn test_strip_italic() {
        assert_eq!(markdown_to_imessage("*italic*"), "italic");
    }

    #[test]
    fn test_strip_inline_code() {
        assert_eq!(markdown_to_imessage("use `code` here"), "use code here");
    }

    #[test]
    fn test_strip_header() {
        assert_eq!(markdown_to_imessage("## Header"), "Header");
    }

    #[test]
    fn test_link_conversion() {
        assert_eq!(
            markdown_to_imessage("[click here](https://example.com)"),
            "click here (https://example.com)"
        );
    }

    #[test]
    fn test_strip_strikethrough() {
        assert_eq!(markdown_to_imessage("~~deleted~~"), "deleted");
    }
}
