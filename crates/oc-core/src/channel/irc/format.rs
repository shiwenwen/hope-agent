use regex::Regex;

/// Convert Markdown to IRC-friendly plain text.
///
/// IRC has no standard rich-text format, so we strip markdown formatting:
/// - `**bold**` / `__bold__` -> `bold`
/// - `*italic*` / `_italic_` -> `italic`
/// - `~~strikethrough~~` -> `strikethrough`
/// - `[text](url)` -> `text (url)`
/// - `![alt](url)` -> `alt (url)`
/// - Headings `# text` -> `text`
/// - Code blocks and inline code are kept as-is (readable in plain text)
pub fn markdown_to_irc(markdown: &str) -> String {
    let mut result = markdown.to_string();

    // Convert images: ![alt](url) -> alt (url)
    let re_img = Regex::new(r"!\[([^\]]*)\]\(([^)]+)\)").unwrap();
    result = re_img.replace_all(&result, "$1 ($2)").to_string();

    // Convert links: [text](url) -> text (url)
    let re_link = Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap();
    result = re_link.replace_all(&result, "$1 ($2)").to_string();

    // Strip bold: **text** or __text__
    let re_bold_star = Regex::new(r"\*\*(.+?)\*\*").unwrap();
    result = re_bold_star.replace_all(&result, "$1").to_string();

    let re_bold_under = Regex::new(r"__(.+?)__").unwrap();
    result = re_bold_under.replace_all(&result, "$1").to_string();

    // Strip italic: *text* or _text_ (but not inside code or underscores in words)
    let re_italic_star = Regex::new(r"\*(.+?)\*").unwrap();
    result = re_italic_star.replace_all(&result, "$1").to_string();

    // Only strip _italic_ at word boundaries to avoid mangling snake_case.
    // Uses fancy_regex because the `regex` crate doesn't support lookaround.
    let re_italic_under = fancy_regex::Regex::new(r"(?<!\w)_(.+?)_(?!\w)").unwrap();
    result = re_italic_under.replace_all(&result, "$1").to_string();

    // Strip strikethrough: ~~text~~
    let re_strike = Regex::new(r"~~(.+?)~~").unwrap();
    result = re_strike.replace_all(&result, "$1").to_string();

    // Strip heading markers: # Heading -> Heading
    let re_heading = Regex::new(r"(?m)^#{1,6}\s+").unwrap();
    result = re_heading.replace_all(&result, "").to_string();

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_bold() {
        assert_eq!(markdown_to_irc("**bold text**"), "bold text");
        assert_eq!(markdown_to_irc("__bold text__"), "bold text");
    }

    #[test]
    fn test_strip_italic() {
        assert_eq!(markdown_to_irc("*italic text*"), "italic text");
    }

    #[test]
    fn test_strip_strikethrough() {
        assert_eq!(markdown_to_irc("~~deleted~~"), "deleted");
    }

    #[test]
    fn test_convert_link() {
        assert_eq!(
            markdown_to_irc("[click here](https://example.com)"),
            "click here (https://example.com)"
        );
    }

    #[test]
    fn test_convert_image() {
        assert_eq!(
            markdown_to_irc("![alt text](https://example.com/img.png)"),
            "alt text (https://example.com/img.png)"
        );
    }

    #[test]
    fn test_strip_heading() {
        assert_eq!(markdown_to_irc("# Heading 1"), "Heading 1");
        assert_eq!(markdown_to_irc("### Heading 3"), "Heading 3");
    }

    #[test]
    fn test_preserve_code_block() {
        let input = "```rust\nfn main() {}\n```";
        assert_eq!(markdown_to_irc(input), input);
    }

    #[test]
    fn test_preserve_inline_code() {
        let input = "Use `println!` for output";
        assert_eq!(markdown_to_irc(input), input);
    }

    #[test]
    fn test_plain_text_passthrough() {
        let input = "Hello, world!";
        assert_eq!(markdown_to_irc(input), input);
    }

    #[test]
    fn test_empty() {
        assert_eq!(markdown_to_irc(""), "");
    }

    #[test]
    fn test_mixed_formatting() {
        assert_eq!(
            markdown_to_irc("**bold** and *italic* with [link](http://x.com)"),
            "bold and italic with link (http://x.com)"
        );
    }

    #[test]
    fn test_snake_case_preserved() {
        assert_eq!(
            markdown_to_irc("use my_variable_name here"),
            "use my_variable_name here"
        );
    }
}
