/// Parse a slash command text into (command_name, args).
///
/// Examples:
///   "/new"          → Ok(("new", ""))
///   "/model gpt-4o" → Ok(("model", "gpt-4o"))
///   "/rename My Chat" → Ok(("rename", "My Chat"))
///   "hello"         → Err("Not a slash command")
pub fn parse(text: &str) -> Result<(String, String), String> {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return Err("Not a slash command".into());
    }

    let without_slash = &trimmed[1..];
    if without_slash.is_empty() {
        return Err("Empty command".into());
    }

    // Split on first whitespace
    match without_slash.find(char::is_whitespace) {
        Some(pos) => {
            let name = without_slash[..pos].to_lowercase();
            let args = without_slash[pos..].trim().to_string();
            Ok((name, args))
        }
        None => {
            let name = without_slash.to_lowercase();
            Ok((name, String::new()))
        }
    }
}

/// Quick check: does this text look like a slash command?
pub fn is_command(text: &str) -> bool {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return false;
    }
    let rest = &trimmed[1..];
    // Must start with a letter (not /123 or //)
    rest.starts_with(|c: char| c.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_no_args() {
        let (name, args) = parse("/new").unwrap();
        assert_eq!(name, "new");
        assert_eq!(args, "");
    }

    #[test]
    fn test_parse_with_args() {
        let (name, args) = parse("/model gpt-4o").unwrap();
        assert_eq!(name, "model");
        assert_eq!(args, "gpt-4o");
    }

    #[test]
    fn test_parse_with_multi_word_args() {
        let (name, args) = parse("/rename My Awesome Chat").unwrap();
        assert_eq!(name, "rename");
        assert_eq!(args, "My Awesome Chat");
    }

    #[test]
    fn test_parse_case_insensitive() {
        let (name, _) = parse("/NEW").unwrap();
        assert_eq!(name, "new");
    }

    #[test]
    fn test_parse_not_command() {
        assert!(parse("hello").is_err());
    }

    #[test]
    fn test_is_command() {
        assert!(is_command("/new"));
        assert!(is_command("/model gpt-4o"));
        assert!(is_command("  /help  "));
        assert!(!is_command("hello"));
        assert!(!is_command("/123"));
        assert!(!is_command("//"));
    }
}
