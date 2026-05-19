use std::collections::BTreeMap;

/// Parsed IRC message.
#[derive(Debug, Clone)]
pub struct IrcMessage {
    pub tags: BTreeMap<String, Option<String>>,
    pub prefix: Option<String>,
    pub command: String,
    pub params: Vec<String>,
}

/// Parse a raw IRC protocol line into an `IrcMessage`.
///
/// IRC line format: `[:prefix] COMMAND [params] [:trailing]`
///
/// Examples:
/// - `:nick!user@host PRIVMSG #channel :Hello world`
/// - `PING :server.example.com`
/// - `:server 001 mynick :Welcome to the IRC network`
pub fn parse_irc_line(line: &str) -> Option<IrcMessage> {
    let raw = line.replace(['\r', '\n'], "");
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let mut cursor = raw;
    let mut tags = BTreeMap::new();
    let mut prefix: Option<String> = None;

    // Parse IRCv3 message tags.
    if let Some(rest) = cursor.strip_prefix('@') {
        let idx = rest.find(' ')?;
        tags = parse_message_tags(&rest[..idx]);
        cursor = rest[idx + 1..].trim_start();
    }

    // Parse optional prefix
    if cursor.starts_with(':') {
        let idx = cursor.find(' ')?;
        if idx <= 1 {
            return None;
        }
        prefix = Some(cursor[1..idx].to_string());
        cursor = cursor[idx + 1..].trim_start();
    }

    if cursor.is_empty() {
        return None;
    }

    // Parse command
    let (command, rest) = match cursor.find(' ') {
        Some(idx) => (&cursor[..idx], &cursor[idx + 1..]),
        None => (cursor, ""),
    };

    let command = command.trim().to_uppercase();
    if command.is_empty() {
        return None;
    }

    // Parse params and trailing
    let mut params = Vec::new();
    let mut cursor = rest;

    while !cursor.is_empty() {
        cursor = cursor.trim_start();
        if cursor.is_empty() {
            break;
        }
        if cursor.starts_with(':') {
            // Trailing parameter (everything after the colon)
            params.push(cursor[1..].to_string());
            break;
        }
        match cursor.find(' ') {
            Some(idx) => {
                params.push(cursor[..idx].to_string());
                cursor = &cursor[idx + 1..];
            }
            None => {
                params.push(cursor.to_string());
                break;
            }
        }
    }

    Some(IrcMessage {
        tags,
        prefix,
        command,
        params,
    })
}

fn parse_message_tags(raw: &str) -> BTreeMap<String, Option<String>> {
    let mut tags = BTreeMap::new();

    for tag in raw.split(';') {
        if tag.is_empty() {
            continue;
        }
        let (key, value) = tag.split_once('=').unwrap_or((tag, ""));
        if key.is_empty() {
            continue;
        }
        let value = if value.is_empty() {
            None
        } else {
            Some(unescape_tag_value(value))
        };
        tags.insert(key.to_string(), value);
    }

    tags
}

fn unescape_tag_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        match chars.next() {
            Some(':') => out.push(';'),
            Some('s') => out.push(' '),
            Some('\\') => out.push('\\'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }

    out
}

/// Extract the nick from an IRC prefix like `nick!user@host`.
///
/// Returns the nick portion, or the entire prefix if it doesn't match
/// the standard `nick!user@host` format.
pub fn extract_nick(prefix: &str) -> &str {
    if let Some(idx) = prefix.find('!') {
        &prefix[..idx]
    } else if let Some(idx) = prefix.find('@') {
        &prefix[..idx]
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_privmsg() {
        let msg = parse_irc_line(":nick!user@host PRIVMSG #channel :Hello world").unwrap();
        assert!(msg.tags.is_empty());
        assert_eq!(msg.prefix.as_deref(), Some("nick!user@host"));
        assert_eq!(msg.command, "PRIVMSG");
        assert_eq!(msg.params, vec!["#channel", "Hello world"]);
    }

    #[test]
    fn test_parse_ping() {
        let msg = parse_irc_line("PING :server.example.com").unwrap();
        assert!(msg.prefix.is_none());
        assert_eq!(msg.command, "PING");
        assert_eq!(msg.params, vec!["server.example.com"]);
    }

    #[test]
    fn test_parse_welcome() {
        let msg = parse_irc_line(":server 001 mynick :Welcome to the IRC network").unwrap();
        assert_eq!(msg.prefix.as_deref(), Some("server"));
        assert_eq!(msg.command, "001");
        assert_eq!(msg.params, vec!["mynick", "Welcome to the IRC network"]);
    }

    #[test]
    fn test_parse_empty() {
        assert!(parse_irc_line("").is_none());
        assert!(parse_irc_line("  ").is_none());
    }

    #[test]
    fn test_parse_no_trailing() {
        let msg = parse_irc_line(":server 433 * mynick").unwrap();
        assert_eq!(msg.command, "433");
        assert_eq!(msg.params, vec!["*", "mynick"]);
    }

    #[test]
    fn test_extract_nick_full() {
        assert_eq!(extract_nick("nick!user@host"), "nick");
    }

    #[test]
    fn test_extract_nick_at_only() {
        assert_eq!(extract_nick("nick@host"), "nick");
    }

    #[test]
    fn test_extract_nick_plain() {
        assert_eq!(extract_nick("nick"), "nick");
    }

    #[test]
    fn test_parse_crlf_stripped() {
        let msg = parse_irc_line("PING :test\r\n").unwrap();
        assert_eq!(msg.command, "PING");
        assert_eq!(msg.params, vec!["test"]);
    }

    #[test]
    fn test_parse_command_case_insensitive() {
        let msg = parse_irc_line("ping :test").unwrap();
        assert_eq!(msg.command, "PING");
    }

    #[test]
    fn test_parse_privmsg_dm() {
        let msg = parse_irc_line(":alice!alice@host PRIVMSG mybot :hi there").unwrap();
        assert_eq!(msg.command, "PRIVMSG");
        assert_eq!(msg.params, vec!["mybot", "hi there"]);
    }

    #[test]
    fn test_parse_ircv3_message_tags() {
        let msg = parse_irc_line(
            "@time=2026-05-19T09:00:00.000Z;account=alice;foo :alice!u@h PRIVMSG #chan :hi",
        )
        .unwrap();

        assert_eq!(
            msg.tags.get("time").and_then(|v| v.as_deref()),
            Some("2026-05-19T09:00:00.000Z")
        );
        assert_eq!(
            msg.tags.get("account").and_then(|v| v.as_deref()),
            Some("alice")
        );
        assert_eq!(msg.tags.get("foo"), Some(&None));
        assert_eq!(msg.prefix.as_deref(), Some("alice!u@h"));
        assert_eq!(msg.command, "PRIVMSG");
    }

    #[test]
    fn test_parse_ircv3_message_tag_escaping_and_duplicates() {
        let msg =
            parse_irc_line("@tag=old;tag=semi\\:space\\spath\\\\drop\\b :s NOTICE * :ok").unwrap();

        assert_eq!(
            msg.tags.get("tag").and_then(|v| v.as_deref()),
            Some("semi;space path\\dropb")
        );
    }
}
