//! GUI / HTTP → IM mirror — user-message quote prefix.
//!
//! When a desktop / HTTP turn's final assistant reply is mirrored to an
//! IM chat, the IM user has no idea **what question** the agent is
//! answering — they only see the response in isolation. This helper
//! renders a markdown blockquote of the latest user message that
//! triggered the turn so the IM user gets the same context the desktop
//! user sees in their chat thread.
//!
//! Channel-source turns (the IM user themselves asked) are skipped —
//! they already see their own message right above the reply.
//!
//! The quote prefix is **only** prepended to the IM-bound chunk; the
//! `messages` table / `sessions.context_json` keep the unmodified
//! assistant text so subsequent context windows + desktop history are
//! unaffected.

use crate::slash_commands::truncate_description;

/// Hard char-count cap on the body included in the quote — keeps long
/// prompts from monopolizing the IM message above the actual reply.
const MAX_QUOTE_CHARS: usize = 240;

/// Snapshot of the user message that triggered a desktop / HTTP turn,
/// passed to [`build_user_quote_prefix`] so the quote helper doesn't
/// need to re-query SQL — `run_chat_engine` already has the input text
/// + attachment count + source string in hand at turn entry.
#[derive(Debug, Clone)]
pub struct LastUserView<'a> {
    /// Lowercase wire-format source string (`"desktop"` / `"http"` /
    /// `"channel"` / `"subagent"` / `"parent_injection"`). Mirrors
    /// `ChatSource::as_str` so callers can pass it cheaply without
    /// converting to an owned String.
    pub source: &'a str,
    /// User message text — `&str` to avoid an allocation on every turn;
    /// the helper trims and truncates internally.
    pub text: &'a str,
    /// User-uploaded attachments count. The desktop / HTTP `chat` path
    /// serialises a JSON array and counts its length; structured
    /// payloads (plan_trigger, channel_inbound) don't contribute here.
    pub attachment_count: usize,
}

/// Build a markdown blockquote of the user message snapshot in `last_user`.
/// Returns `None` when:
///
/// - the snapshot is `None` (no user message, defensively handled),
/// - the source is anything other than `desktop` / `http` (IM-channel
///   turns naturally show the user's question right above the reply,
///   subagent / parent_injection / cron are not human-authored),
/// - the user message has neither text nor attachments.
///
/// Returns the prefix string (already terminated with a blank line) so
/// the caller can do `format!("{prefix}{response}")` without further
/// formatting work.
pub fn build_user_quote_prefix(last_user: Option<&LastUserView<'_>>) -> Option<String> {
    let view = last_user?;
    if !matches!(view.source, "desktop" | "http") {
        return None;
    }
    render_quote_prefix(view.text, view.attachment_count)
}

/// Render the user-message body + optional attachment count as a
/// `> `-prefixed markdown blockquote followed by a blank line.
///
/// Uses a single forward-pass state machine over `lines()`:
/// - line 0 non-empty: `> 💬 {line}`
/// - line 0 empty: `>` (rare but keeps the quote connected)
/// - subsequent non-empty: `> {line}`
/// - subsequent empty: `>`
///
/// Exposed for tests so the truncate / multi-line / attachment branches
/// are exercised without touching the DB.
pub(crate) fn render_quote_prefix(text: &str, attachment_count: usize) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() && attachment_count == 0 {
        return None;
    }

    let mut quote_lines: Vec<String> = Vec::new();
    if !trimmed.is_empty() {
        let body = truncate_description(trimmed, MAX_QUOTE_CHARS);
        for (idx, line) in body.lines().enumerate() {
            let is_first = idx == 0;
            let is_blank = line.trim().is_empty();
            let rendered = match (is_first, is_blank) {
                (true, false) => format!("> 💬 {line}"),
                (true, true) => ">".to_string(),
                (false, false) => format!("> {line}"),
                (false, true) => ">".to_string(),
            };
            quote_lines.push(rendered);
        }
    }

    if attachment_count > 0 {
        let label = if attachment_count == 1 {
            "1 attachment".to_string()
        } else {
            format!("{attachment_count} attachments")
        };
        quote_lines.push(format!("> [📎 {label}]"));
    }

    if quote_lines.is_empty() {
        return None;
    }

    // Blank line after the quote so most markdown renderers (and IM
    // native quote conversions) close the blockquote before the
    // assistant reply.
    Some(format!("{}\n\n", quote_lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_no_attachments_returns_none() {
        assert!(render_quote_prefix("", 0).is_none());
        assert!(render_quote_prefix("   \n\t", 0).is_none());
    }

    #[test]
    fn single_line_text() {
        let q = render_quote_prefix("hello world", 0).unwrap();
        assert_eq!(q, "> 💬 hello world\n\n");
    }

    #[test]
    fn multi_line_text() {
        let q = render_quote_prefix("line one\nline two\nline three", 0).unwrap();
        assert_eq!(q, "> 💬 line one\n> line two\n> line three\n\n");
    }

    #[test]
    fn truncates_long_text_and_appends_ellipsis() {
        let long = "a".repeat(300);
        let q = render_quote_prefix(&long, 0).unwrap();
        // truncate_description(s, 240) keeps 239 + '…' = 240 chars body.
        assert!(q.starts_with("> 💬 "));
        assert!(q.ends_with("…\n\n"));
    }

    #[test]
    fn attachments_only_no_text() {
        let q = render_quote_prefix("", 2).unwrap();
        assert_eq!(q, "> [📎 2 attachments]\n\n");
    }

    #[test]
    fn attachments_only_singular() {
        let q = render_quote_prefix("", 1).unwrap();
        assert_eq!(q, "> [📎 1 attachment]\n\n");
    }

    #[test]
    fn text_plus_attachments() {
        let q = render_quote_prefix("look at this", 3).unwrap();
        assert_eq!(q, "> 💬 look at this\n> [📎 3 attachments]\n\n");
    }

    #[test]
    fn blank_line_inside_user_message_collapses_to_bare_marker() {
        // Sanity: empty middle line keeps the blockquote connected as `>`.
        let q = render_quote_prefix("first\n\nthird", 0).unwrap();
        assert_eq!(q, "> 💬 first\n>\n> third\n\n");
    }

    #[test]
    fn build_user_quote_prefix_skips_non_desktop_http() {
        for src in ["channel", "subagent", "parent_injection", "unknown"] {
            assert!(
                build_user_quote_prefix(Some(&LastUserView {
                    source: src,
                    text: "hi",
                    attachment_count: 0,
                }))
                .is_none(),
                "source {} should be filtered out",
                src
            );
        }
    }

    #[test]
    fn build_user_quote_prefix_emits_for_desktop_and_http() {
        for src in ["desktop", "http"] {
            let q = build_user_quote_prefix(Some(&LastUserView {
                source: src,
                text: "hello",
                attachment_count: 0,
            }))
            .unwrap();
            assert_eq!(q, "> 💬 hello\n\n");
        }
    }
}
