/// Convert Markdown to LINE-compatible plain text.
///
/// LINE's basic text message type does not support rich formatting (bold, italic,
/// code blocks, etc.), so we strip Markdown syntax and return plain text.
pub fn markdown_to_line(markdown: &str) -> String {
    let mut result = String::with_capacity(markdown.len());
    let mut chars = markdown.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            // Strip bold/italic markers: ** or *
            '*' => {
                // Consume consecutive '*' characters
                while chars.peek() == Some(&'*') {
                    chars.next();
                }
            }
            // Strip strikethrough markers: ~~
            '~' => {
                if chars.peek() == Some(&'~') {
                    chars.next();
                } else {
                    result.push(ch);
                }
            }
            // Convert Markdown links [text](url) to "text (url)"
            '[' => {
                let mut link_text = String::new();
                let mut found_close = false;
                for c in chars.by_ref() {
                    if c == ']' {
                        found_close = true;
                        break;
                    }
                    link_text.push(c);
                }
                if found_close && chars.peek() == Some(&'(') {
                    chars.next(); // consume '('
                    let mut url = String::new();
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                        url.push(c);
                    }
                    if link_text == url {
                        result.push_str(&url);
                    } else {
                        result.push_str(&link_text);
                        result.push_str(" (");
                        result.push_str(&url);
                        result.push(')');
                    }
                } else {
                    // Not a valid link, output as-is
                    result.push('[');
                    result.push_str(&link_text);
                    if found_close {
                        result.push(']');
                    }
                }
            }
            // Strip inline code backticks
            '`' => {
                // Consume consecutive backticks (```, ``)
                while chars.peek() == Some(&'`') {
                    chars.next();
                }
            }
            // Strip heading markers at start of line: # ## ### etc.
            '#' => {
                if result.is_empty() || result.ends_with('\n') {
                    // Consume remaining '#' and one trailing space
                    while chars.peek() == Some(&'#') {
                        chars.next();
                    }
                    if chars.peek() == Some(&' ') {
                        chars.next();
                    }
                } else {
                    result.push(ch);
                }
            }
            // Strip blockquote markers at start of line: > text
            '>' => {
                if result.is_empty() || result.ends_with('\n') {
                    if chars.peek() == Some(&' ') {
                        chars.next();
                    }
                } else {
                    result.push(ch);
                }
            }
            _ => result.push(ch),
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text_passthrough() {
        assert_eq!(markdown_to_line("Hello, world!"), "Hello, world!");
    }

    #[test]
    fn test_strip_bold() {
        assert_eq!(markdown_to_line("**bold text**"), "bold text");
    }

    #[test]
    fn test_strip_italic() {
        assert_eq!(markdown_to_line("*italic*"), "italic");
    }

    #[test]
    fn test_strip_inline_code() {
        assert_eq!(markdown_to_line("use `println!` here"), "use println! here");
    }

    #[test]
    fn test_strip_heading() {
        assert_eq!(markdown_to_line("## Heading"), "Heading");
    }

    #[test]
    fn test_convert_link() {
        assert_eq!(
            markdown_to_line("[click](https://example.com)"),
            "click (https://example.com)"
        );
    }

    #[test]
    fn test_link_same_text_and_url() {
        assert_eq!(
            markdown_to_line("[https://example.com](https://example.com)"),
            "https://example.com"
        );
    }

    #[test]
    fn test_strip_blockquote() {
        assert_eq!(markdown_to_line("> quoted text"), "quoted text");
    }

    #[test]
    fn test_strip_strikethrough() {
        assert_eq!(markdown_to_line("~~deleted~~"), "deleted");
    }

    #[test]
    fn test_empty() {
        assert_eq!(markdown_to_line(""), "");
    }

    #[test]
    fn test_unicode() {
        assert_eq!(markdown_to_line("**Hello** world"), "Hello world");
    }
}
