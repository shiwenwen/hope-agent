//! Evidence quote resolution — the backend-authorized read for expanding
//! an evidence ref into a short source excerpt (Evidence Layer, design
//! §5.3 / §8.1).
//!
//! Expanding a quote MUST go through the backend so the incognito gate
//! can't be bypassed by a frontend that merely hides the control. The two
//! owner-plane shells (Tauri command + HTTP route) both call
//! [`evidence_quote`]; the incognito check + redaction + length cap live
//! here so every caller inherits them.

use super::types::EvidenceQuote;

/// Default excerpt cap (design §12: "Evidence quote 默认 300-500 chars").
pub const QUOTE_MAX_CHARS: usize = 400;

/// Resolve a redacted, length-capped excerpt for an evidence ref pointing
/// at a **specific** session message.
///
/// Two hard gates, both fail-closed:
/// 1. **Precise anchor required.** Without a `message_id` we refuse rather
///    than guess a message — returning, say, the session's latest message
///    would misattribute arbitrary later content as the source (and could
///    surface unrelated sensitive text). Precise per-claim anchors arrive
///    with claim extraction; until then a session ref is display-only.
/// 2. **Session must provably exist and be non-incognito.** Missing /
///    deleted / errored metadata → unavailable; incognito → unavailable.
///    Incognito sources never surface (design §8.1; burn-on-close).
///
/// The excerpt is the named message, redacted via
/// [`crate::logging::redact_sensitive`] and capped to `QUOTE_MAX_CHARS`. The
/// HTTP shell is owner-plane (API-key trust, like
/// `GET /api/sessions/{id}/messages`), so this never widens exposure beyond
/// the existing session-message endpoints — it narrows it.
pub fn evidence_quote(session_id: &str, message_id: Option<i64>) -> EvidenceQuote {
    // Gate 1: no precise anchor → never guess (would misattribute).
    let Some(mid) = message_id else {
        return EvidenceQuote::unavailable(session_id, None, "message_id_required");
    };

    // Gate 2: fail-closed — only a present, non-incognito session may surface.
    match crate::session::lookup_session_meta(Some(session_id)) {
        Some(meta) if meta.incognito => {
            return EvidenceQuote::unavailable(session_id, Some(mid), "incognito");
        }
        Some(_) => {}
        // Missing / deleted / errored meta: we can't prove it wasn't an
        // incognito source, so refuse.
        None => return EvidenceQuote::unavailable(session_id, Some(mid), "not_found"),
    }

    let Some(db) = crate::get_session_db() else {
        return EvidenceQuote::unavailable(session_id, Some(mid), "no_session_db");
    };

    let messages = match db.load_session_messages(session_id) {
        Ok(m) => m,
        Err(_) => return EvidenceQuote::unavailable(session_id, Some(mid), "load_failed"),
    };

    let Some(msg) = messages.iter().find(|m| m.id == mid) else {
        return EvidenceQuote::unavailable(session_id, Some(mid), "not_found");
    };

    let (quote, truncated) = redact_and_truncate(&msg.content, QUOTE_MAX_CHARS);
    EvidenceQuote {
        session_id: session_id.to_string(),
        message_id: Some(msg.id),
        role: Some(msg.role.as_str().to_string()),
        quote,
        truncated,
        available: true,
        reason: None,
    }
}

/// Redact sensitive tokens, then cap to `max_chars` codepoints. Pure so the
/// redaction + truncation contract is deterministically testable.
pub(crate) fn redact_and_truncate(content: &str, max_chars: usize) -> (String, bool) {
    let redacted = crate::logging::redact_sensitive(content);
    let chars: Vec<char> = redacted.chars().collect();
    if chars.len() > max_chars {
        let head: String = chars[..max_chars].iter().collect();
        (format!("{}…", head.trim_end()), true)
    } else {
        (redacted, false)
    }
}

impl EvidenceQuote {
    /// Build an "unavailable" result that leaks nothing about the source.
    pub(crate) fn unavailable(session_id: &str, message_id: Option<i64>, reason: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            message_id,
            role: None,
            quote: String::new(),
            truncated: false,
            available: false,
            reason: Some(reason.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_long_content_with_ellipsis() {
        let content = "a".repeat(500);
        let (quote, truncated) = redact_and_truncate(&content, 400);
        assert!(truncated);
        // 400 chars + the ellipsis marker.
        assert_eq!(quote.chars().count(), 401);
        assert!(quote.ends_with('…'));
    }

    #[test]
    fn keeps_short_content_verbatim() {
        let (quote, truncated) = redact_and_truncate("hello world", 400);
        assert!(!truncated);
        assert_eq!(quote, "hello world");
    }

    #[test]
    fn redacts_embedded_secrets() {
        let content = r#"here is my config {"api_key":"sk-secret-123"} ok"#;
        let (quote, _) = redact_and_truncate(content, 400);
        assert!(quote.contains("[REDACTED]"));
        assert!(!quote.contains("sk-secret-123"));
    }

    #[test]
    fn truncates_by_codepoint_not_byte() {
        // Multi-byte chars must not be split mid-codepoint.
        let content = "中".repeat(500);
        let (quote, truncated) = redact_and_truncate(&content, 10);
        assert!(truncated);
        assert_eq!(quote.chars().count(), 11); // 10 + ellipsis
    }

    #[test]
    fn unavailable_leaks_nothing() {
        let q = EvidenceQuote::unavailable("sess-x", Some(5), "incognito");
        assert!(!q.available);
        assert!(q.quote.is_empty());
        assert_eq!(q.reason.as_deref(), Some("incognito"));
        assert!(q.role.is_none());
    }

    #[test]
    fn quote_requires_precise_message_anchor() {
        // A session-level ref with no message_id must never be expanded to a
        // guessed message — refuse before touching the DB.
        let q = evidence_quote("sess-x", None);
        assert!(!q.available);
        assert_eq!(q.reason.as_deref(), Some("message_id_required"));
        assert!(q.quote.is_empty());
    }

    #[test]
    fn quote_fails_closed_on_missing_session_meta() {
        // No global session DB in unit context → meta lookup yields None →
        // fail closed (can't prove the source wasn't incognito).
        let q = evidence_quote("sess-does-not-exist", Some(123));
        assert!(!q.available);
        assert_eq!(q.reason.as_deref(), Some("not_found"));
        assert!(q.quote.is_empty());
    }
}
