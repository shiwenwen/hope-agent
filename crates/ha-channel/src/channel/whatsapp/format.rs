use std::sync::OnceLock;

use regex::Regex;

/// Convert Markdown to WhatsApp-safe plain text.
///
/// WhatsApp has its own formatting (*bold*, _italic_, ~strikethrough~, ```code```)
/// but since we are going through an external bridge service, stripping to plain text
/// is the safest approach to avoid double-formatting issues.
pub fn markdown_to_whatsapp(markdown: &str) -> String {
    let mut text = markdown.replace("\r\n", "\n");

    // Strip code blocks (keep inner code)
    text = regex_replace(code_block_regex(), &text, "$1");
    // Strip images
    text = regex_replace(image_regex(), &text, "");
    // Strip links, keep link text
    text = regex_replace(link_regex(), &text, "$1");
    // Strip heading markers
    text = regex_replace(heading_regex(), &text, "$1");
    // Strip block quotes
    text = regex_replace(quote_regex(), &text, "$1");
    // Strip inline formatting markers
    text = text
        .replace("**", "")
        .replace("__", "")
        .replace("~~", "")
        .replace('`', "");
    // Collapse excessive blank lines
    text = regex_replace(blank_line_regex(), &text, "\n\n");

    text.trim().to_string()
}

fn regex_replace(regex: &Regex, input: &str, replacement: &str) -> String {
    regex.replace_all(input, replacement).to_string()
}

fn code_block_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"```[^\n]*\n?([\s\S]*?)```").expect("valid regex"))
}

fn image_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"!\[[^\]]*\]\([^)]*\)").expect("valid regex"))
}

fn link_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\[([^\]]+)\]\([^)]*\)").expect("valid regex"))
}

fn heading_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?m)^#{1,6}\s+(.*)$").expect("valid regex"))
}

fn quote_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?m)^>\s?(.*)$").expect("valid regex"))
}

fn blank_line_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\n{3,}").expect("valid regex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strips_markdown_bold() {
        assert_eq!(markdown_to_whatsapp("**hello**"), "hello");
    }

    #[test]
    fn test_strips_links() {
        assert_eq!(
            markdown_to_whatsapp("[click here](https://example.com)"),
            "click here"
        );
    }

    #[test]
    fn test_strips_images() {
        assert_eq!(markdown_to_whatsapp("![alt](https://img.png)"), "");
    }

    #[test]
    fn test_strips_code_blocks() {
        let input = "```rust\nlet x = 1;\n```";
        assert_eq!(markdown_to_whatsapp(input), "let x = 1;");
    }

    #[test]
    fn test_strips_headings() {
        assert_eq!(markdown_to_whatsapp("## Title"), "Title");
    }

    #[test]
    fn test_collapses_blank_lines() {
        assert_eq!(markdown_to_whatsapp("a\n\n\n\nb"), "a\n\nb");
    }
}
