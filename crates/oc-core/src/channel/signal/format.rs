use regex::Regex;

/// Convert Markdown to Signal-native format.
///
/// Signal supports basic formatting natively (bold, italic, strikethrough,
/// monospace), so most Markdown passes through as-is. The main conversion
/// is turning `[text](url)` links into `text (url)` since Signal does not
/// render Markdown links.
pub fn markdown_to_signal(md: &str) -> String {
    // Convert [text](url) -> text (url)
    let re = Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").expect("valid regex");
    re.replace_all(md, "$1 ($2)").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_passthrough_plain_text() {
        let input = "Hello, world!";
        assert_eq!(markdown_to_signal(input), input);
    }

    #[test]
    fn test_passthrough_bold_italic() {
        let input = "**bold** and *italic*";
        assert_eq!(markdown_to_signal(input), input);
    }

    #[test]
    fn test_passthrough_code_block() {
        let input = "```rust\nfn main() {}\n```";
        assert_eq!(markdown_to_signal(input), input);
    }

    #[test]
    fn test_link_conversion() {
        let input = "[click here](https://example.com)";
        assert_eq!(markdown_to_signal(input), "click here (https://example.com)");
    }

    #[test]
    fn test_multiple_links() {
        let input = "Visit [Google](https://google.com) or [GitHub](https://github.com)";
        assert_eq!(
            markdown_to_signal(input),
            "Visit Google (https://google.com) or GitHub (https://github.com)"
        );
    }

    #[test]
    fn test_passthrough_strikethrough() {
        let input = "~~deleted~~";
        assert_eq!(markdown_to_signal(input), input);
    }

    #[test]
    fn test_passthrough_inline_code() {
        let input = "Use `println!` for output";
        assert_eq!(markdown_to_signal(input), input);
    }

    #[test]
    fn test_passthrough_empty() {
        assert_eq!(markdown_to_signal(""), "");
    }

    #[test]
    fn test_passthrough_unicode() {
        let input = "你好世界 **加粗**";
        assert_eq!(markdown_to_signal(input), input);
    }

    #[test]
    fn test_mixed_content() {
        let input = "**Bold** text with [a link](https://example.com) and `code`";
        assert_eq!(
            markdown_to_signal(input),
            "**Bold** text with a link (https://example.com) and `code`"
        );
    }
}
