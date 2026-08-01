/// Convert Markdown to Discord-native format.
///
/// Discord uses standard Markdown natively, so this is essentially a passthrough.
/// No conversion is needed — bold, italic, code blocks, links, etc. all work
/// directly in Discord's message rendering.
pub fn markdown_to_discord(md: &str) -> String {
    md.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_passthrough_plain_text() {
        let input = "Hello, world!";
        assert_eq!(markdown_to_discord(input), input);
    }

    #[test]
    fn test_passthrough_bold_italic() {
        let input = "**bold** and *italic*";
        assert_eq!(markdown_to_discord(input), input);
    }

    #[test]
    fn test_passthrough_code_block() {
        let input = "```rust\nfn main() {}\n```";
        assert_eq!(markdown_to_discord(input), input);
    }

    #[test]
    fn test_passthrough_inline_code() {
        let input = "Use `println!` for output";
        assert_eq!(markdown_to_discord(input), input);
    }

    #[test]
    fn test_passthrough_links() {
        let input = "[click here](https://example.com)";
        assert_eq!(markdown_to_discord(input), input);
    }

    #[test]
    fn test_passthrough_strikethrough() {
        let input = "~~deleted~~";
        assert_eq!(markdown_to_discord(input), input);
    }

    #[test]
    fn test_passthrough_blockquote() {
        let input = "> This is a quote\n> continued";
        assert_eq!(markdown_to_discord(input), input);
    }

    #[test]
    fn test_passthrough_multiline() {
        let input = "# Heading\n\nParagraph with **bold**.\n\n- item 1\n- item 2";
        assert_eq!(markdown_to_discord(input), input);
    }

    #[test]
    fn test_passthrough_empty() {
        assert_eq!(markdown_to_discord(""), "");
    }

    #[test]
    fn test_passthrough_unicode() {
        let input = "你好世界 🌍 **加粗**";
        assert_eq!(markdown_to_discord(input), input);
    }
}
