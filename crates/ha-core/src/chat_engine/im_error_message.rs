//! Friendly IM error / cancel notices.
//!
//! When a chat turn aborts, the IM end used to surface either
//! `_(answering interrupted)_` or `⚠️ Sorry, I encountered an error...` —
//! both stripped the actual cause. This module turns the engine's
//! [`failover::FailoverReason`] classification into a per-class headline
//! plus a sanitized blockquote of the raw error, matching the eviction
//! notice style (emoji prefix + plain English + actionable hint).

use once_cell::sync::Lazy;
use regex::Regex;

use crate::failover::FailoverReason;
use crate::logging::redact_sensitive;
use crate::util::truncate_utf8;

/// Friendly notice shown when the user actively cancels (Stop button or
/// `/cancel` slash). Replaces the older `_(answering interrupted)_`.
pub const CANCEL_NOTICE: &str = "🛑 **Response stopped** — send a new message to continue.";

/// Final blockquote budget — keeps the IM message visually compact.
const RAW_MAX_BYTES: usize = 500;
/// Hard cap fed into `sanitize_raw` to bound regex work. Provider error
/// strings can carry a full HTTP body when JSON parse fails — see
/// [`agent::errors::parse_error_response`]. 8 KiB is well above any
/// realistic credential token, so credentials still can't survive
/// straddling the cut.
const RAW_PRE_CAP: usize = 8 * 1024;

/// Inputs needed to render an IM-friendly error notice.
pub struct ImErrorContext<'a> {
    pub reason: FailoverReason,
    pub raw: &'a str,
    /// `true` only when reason is [`FailoverReason::Auth`] **and** the
    /// failing provider is Codex — gates the "re-authorize via desktop
    /// app" headline.
    pub is_codex_auth: bool,
}

/// Render a markdown notice ready for `send_text_chunks`. Format:
///
/// ```text
/// {headline}
/// > {sanitized truncated raw}
/// ```
///
/// **Auth-class errors omit the blockquote entirely.** Provider 401 /
/// "invalid API key" responses commonly echo the credential inline
/// (`Incorrect API key provided: sk-...`) and the headline already
/// directs the user to re-check their key — dropping raw avoids
/// leaking the token to a possibly-shared IM chat. Other classes
/// still attach a sanitized blockquote for diagnostic context.
///
/// If `raw` is empty after sanitization the blockquote is omitted.
pub fn format_im_engine_error(ctx: ImErrorContext<'_>) -> String {
    let is_auth = matches!(ctx.reason, FailoverReason::Auth);
    let head = headline(ctx.reason, ctx.is_codex_auth);
    if is_auth {
        return head.to_string();
    }
    let capped = truncate_utf8(ctx.raw, RAW_PRE_CAP);
    let cleaned = sanitize_raw(capped);
    let cleaned = truncate_utf8(&cleaned, RAW_MAX_BYTES);
    if cleaned.is_empty() {
        head.to_string()
    } else {
        format!("{head}\n> {cleaned}")
    }
}

fn headline(reason: FailoverReason, is_codex_auth: bool) -> &'static str {
    match reason {
        FailoverReason::Auth if is_codex_auth => {
            "🔐 **Codex session expired** — re-authorize via the desktop app, then retry."
        }
        FailoverReason::Auth => {
            "🔐 **Authentication failed** — please re-check the API key in settings."
        }
        FailoverReason::Billing => "💳 **Quota or billing issue** — check your provider account.",
        FailoverReason::RateLimit => "⏱️ **Rate limited** — try again in a moment.",
        FailoverReason::Overloaded => "🌐 **Provider service is busy** — try again shortly.",
        FailoverReason::Timeout => "🌐 **Network issue talking to the provider** — please retry.",
        FailoverReason::ContextOverflow => {
            "📚 **Conversation got too long** — try `/compact` or start a new session."
        }
        FailoverReason::ModelNotFound => "🤖 **Model unavailable** — pick another in settings.",
        FailoverReason::Unknown => "⚠️ **Something went wrong**.",
    }
}

/// Strip query strings, run the project-wide secret redactor, redact
/// plain-text bearer tokens, scrub well-known LLM token shapes, and
/// collapse whitespace. Run **before** truncation — truncating first
/// risks chopping a token mid-string.
///
/// Layered approach:
/// - [`redact_sensitive`] handles JSON / URL-query shapes (`api_key`,
///   `access_token`, `refresh_token`, `password`, `secret`, …).
/// - `BEARER_RE` covers the plain-text `Authorization: Bearer xxx`
///   shape that lands in HTTP stack traces.
/// - `TOKEN_RE` is a last-line catch for the bare-credential shape that
///   provider 4xx bodies sometimes echo (e.g. `Incorrect API key
///   provided: sk-...`) — neither `redact_sensitive` nor `BEARER_RE`
///   recognizes this since the key isn't framed by JSON/URL/header.
///   Auth-class errors already skip the blockquote entirely (see
///   [`format_im_engine_error`]); `TOKEN_RE` is defense-in-depth for
///   non-Auth classes that could still leak a key (Billing/RateLimit
///   echoes referencing the key).
pub(crate) fn sanitize_raw(s: &str) -> String {
    static QUERY_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"\?[^\s)\"']*"#).unwrap());
    static BEARER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?i)bearer\s+\S+"#).unwrap());
    // Known LLM provider token shapes:
    // - OpenAI / Anthropic: `sk-...` (incl. `sk-proj-`, `sk-svcacct-`, `sk-ant-`)
    // - Google AI: `AIza...` (39 chars total, fixed shape)
    // - GitHub-style PAT: `pat-...`
    // - Slack: `xoxb-` / `xoxp-` / `xoxa-`
    // 20-char minimum body keeps us off short non-secret strings like
    // `pat-info` or `sk-help`.
    static TOKEN_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?:sk|pat)-[A-Za-z0-9_-]{20,}|AIza[A-Za-z0-9_-]{35}|xox[abp]-[A-Za-z0-9_-]{20,}",
        )
        .unwrap()
    });
    static WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"\s+"#).unwrap());

    let mut out = QUERY_RE.replace_all(s, "").into_owned();
    out = redact_sensitive(&out);
    out = BEARER_RE
        .replace_all(&out, "bearer [REDACTED]")
        .into_owned();
    out = TOKEN_RE.replace_all(&out, "[REDACTED]").into_owned();
    out = WS_RE.replace_all(&out, " ").into_owned();
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(reason: FailoverReason, raw: &'a str, is_codex_auth: bool) -> ImErrorContext<'a> {
        ImErrorContext {
            reason,
            raw,
            is_codex_auth,
        }
    }

    #[test]
    fn cancel_notice_is_stable() {
        // Guard against accidental edits — UX strings are part of the contract.
        assert_eq!(
            CANCEL_NOTICE,
            "🛑 **Response stopped** — send a new message to continue."
        );
    }

    #[test]
    fn auth_codex_carve_out() {
        let plain = format_im_engine_error(ctx(FailoverReason::Auth, "401", false));
        let codex = format_im_engine_error(ctx(FailoverReason::Auth, "401", true));
        assert!(plain.contains("Authentication failed"));
        assert!(codex.contains("Codex session expired"));
        assert!(codex.contains("desktop app"));
    }

    #[test]
    fn auth_omits_raw_blockquote_to_avoid_key_leak() {
        // Provider 401 bodies frequently echo the key inline; never quote
        // back the raw error for Auth-class regardless of sanitization.
        let raw =
            "Incorrect API key provided: sk-proj-abc123def456ghi789jkl012mno345 — please check.";
        let out = format_im_engine_error(ctx(FailoverReason::Auth, raw, false));
        assert!(
            !out.contains('>'),
            "Auth must not include blockquote: {out}"
        );
        assert!(!out.contains("sk-proj-"));
        assert_eq!(
            out,
            "🔐 **Authentication failed** — please re-check the API key in settings.",
        );
    }

    #[test]
    fn auth_codex_also_omits_raw() {
        let raw = "Codex token sk-abc123def456ghi789jkl012 invalid";
        let out = format_im_engine_error(ctx(FailoverReason::Auth, raw, true));
        assert!(!out.contains('>'));
        assert!(!out.contains("sk-abc"));
        assert!(out.contains("Codex session expired"));
    }

    #[test]
    fn formats_with_blockquote() {
        let out = format_im_engine_error(ctx(
            FailoverReason::Timeout,
            "error sending request: connection reset",
            false,
        ));
        assert!(out.starts_with("🌐 **Network issue"));
        assert!(out.contains("\n> error sending request: connection reset"));
    }

    #[test]
    fn empty_raw_omits_blockquote() {
        let out = format_im_engine_error(ctx(FailoverReason::Unknown, "", false));
        assert_eq!(out, "⚠️ **Something went wrong**.");
        assert!(!out.contains('>'));
    }

    #[test]
    fn whitespace_only_raw_omits_blockquote() {
        let out = format_im_engine_error(ctx(FailoverReason::Unknown, "   \n  \t  ", false));
        assert_eq!(out, "⚠️ **Something went wrong**.");
    }

    #[test]
    fn truncates_long_raw() {
        let raw: String = "x".repeat(2_000);
        let out = format_im_engine_error(ctx(FailoverReason::Unknown, &raw, false));
        let blockquote_body = out.split_once("\n> ").expect("blockquote present").1;
        assert!(blockquote_body.len() <= RAW_MAX_BYTES);
        assert!(blockquote_body.chars().all(|c| c == 'x'));
    }

    #[test]
    fn pre_caps_huge_raw_before_regex() {
        // 1 MiB of innocuous content — ensures we don't fan a megabyte
        // through 4 regex passes; final output is still bounded by
        // `RAW_MAX_BYTES`.
        let raw: String = "y".repeat(1024 * 1024);
        let out = format_im_engine_error(ctx(FailoverReason::Unknown, &raw, false));
        let blockquote_body = out.split_once("\n> ").expect("blockquote present").1;
        assert!(blockquote_body.len() <= RAW_MAX_BYTES);
    }

    #[test]
    fn strips_query_string_from_url() {
        let raw = "request failed for https://api.example.com/v1?token=secret123&user=me";
        let out = format_im_engine_error(ctx(FailoverReason::Timeout, raw, false));
        assert!(!out.contains("token=secret123"));
        assert!(!out.contains("?"));
        assert!(out.contains("https://api.example.com/v1"));
    }

    #[test]
    fn redacts_bearer_token_in_plain_text() {
        // Use a non-Auth class so the blockquote is rendered.
        let raw = "Authorization: Bearer sk-abc123def456ghi789jkl012 — gateway timeout";
        let out = format_im_engine_error(ctx(FailoverReason::Timeout, raw, false));
        assert!(!out.contains("sk-abc123def456"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_api_key_in_json_body() {
        // Provider 5xx bodies sometimes echo the key — not 401 specific. Use
        // RateLimit to exercise the non-Auth blockquote path.
        let raw = r#"{"error":{"message":"too many requests","api_key":"sk-xyz7891234567890123"}}"#;
        let out = format_im_engine_error(ctx(FailoverReason::RateLimit, raw, false));
        assert!(!out.contains("sk-xyz7891234567890123"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_access_token_in_json_body() {
        // Coverage gained by layering on `redact_sensitive`.
        let raw = r#"{"access_token":"oauth-secret-xyz","detail":"quota exceeded"}"#;
        let out = format_im_engine_error(ctx(FailoverReason::Billing, raw, false));
        assert!(!out.contains("oauth-secret-xyz"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_bare_openai_token_in_non_auth_error() {
        // Provider 5xx body echoing the key inline — neither JSON nor URL
        // nor `Bearer …`. Defense-in-depth `TOKEN_RE` should catch it.
        let raw = "rate limited; key sk-proj-aBcDeFgHiJkLmNoPqRsTuVwXyZ throttled until 12:00";
        let out = format_im_engine_error(ctx(FailoverReason::RateLimit, raw, false));
        assert!(!out.contains("sk-proj-aBcDeFgHiJk"), "got: {out}");
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_google_aiza_token() {
        let raw = "AIzaSyD-Example_KeyWithFortyOneCharsAaaaaaa quota exceeded";
        let out = format_im_engine_error(ctx(FailoverReason::Billing, raw, false));
        assert!(!out.contains("AIzaSyD-Example"), "got: {out}");
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn token_re_does_not_strip_short_lookalikes() {
        // Short `sk-` strings (help text, status codes) should survive.
        // 20-char minimum keeps non-secret content intact.
        let raw = "service unavailable; see https://example.com/sk-help";
        let out = format_im_engine_error(ctx(FailoverReason::Overloaded, raw, false));
        assert!(out.contains("sk-help"), "got: {out}");
    }

    #[test]
    fn collapses_multiline_whitespace() {
        let raw = "line1\n\n\n   line2     line3";
        let out = format_im_engine_error(ctx(FailoverReason::Unknown, raw, false));
        let body = out.split_once("\n> ").unwrap().1;
        assert_eq!(body, "line1 line2 line3");
    }

    #[test]
    fn sanitize_then_truncate_order() {
        // A long URL with a tail token: if we truncated first, we'd leak the
        // start of the token. Verify sanitize runs first by feeding a payload
        // where stripping the query frees up enough budget for the URL itself
        // to survive intact.
        let raw = format!(
            "request failed for https://api.example.com/v1?token={}",
            "S".repeat(800)
        );
        let out = format_im_engine_error(ctx(FailoverReason::Timeout, &raw, false));
        assert!(!out.contains('S'));
        assert!(out.contains("https://api.example.com/v1"));
    }
}
