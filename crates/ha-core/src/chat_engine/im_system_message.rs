//! Friendly IM system-event notices.
//!
//! Mirrors the GUI's inline status banners (model fallback, profile
//! rotation, context compaction, thinking auto-disabled, vision
//! auto-disabled) onto IM as
//! short standalone markdown messages. Format: emoji prefix +
//! single-line italic body. Routed through
//! [`crate::channel::worker::pipeline::StreamPipeline::system_notice_tx`]
//! so notices land as their own IM message and don't tangle with the
//! per-round LLM text accumulator.
//!
//! See sibling [`im_error_message`] for error / cancel notices.

use serde_json::Value;

/// Format a chat-engine system event as an IM-side notice. Returns
/// `None` when:
/// - `event` isn't an object
/// - `type` is missing or not one of the four recognized variants
/// - it's a noisy `context_compacted` (Tier 0 / 1 reactive micro-compact)
pub fn format_im_system_event(event: &Value) -> Option<String> {
    let obj = event.as_object()?;
    let kind = obj.get("type").and_then(Value::as_str)?;
    match kind {
        "model_fallback" => Some(format_model_fallback(obj)),
        "profile_rotation" => Some(format_profile_rotation(obj)),
        "context_compacted" => format_context_compacted(obj),
        "thinking_auto_disabled" => Some(format_thinking_auto_disabled()),
        "vision_auto_disabled" => Some(format_vision_auto_disabled()),
        _ => None,
    }
}

fn format_model_fallback(obj: &serde_json::Map<String, Value>) -> String {
    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("unknown model");
    let reason = obj
        .get("reason")
        .and_then(Value::as_str)
        .map(friendly_reason)
        .unwrap_or("error");
    let attempt = obj.get("attempt").and_then(Value::as_u64);
    let total = obj.get("total").and_then(Value::as_u64);
    match (attempt, total) {
        (Some(a), Some(t)) => {
            format!("⤵️ _Switching to **{model}** — {reason}, attempt {a}/{t}_")
        }
        _ => format!("⤵️ _Switching to **{model}** — {reason}_"),
    }
}

fn format_profile_rotation(obj: &serde_json::Map<String, Value>) -> String {
    let to_profile = obj
        .get("to_profile")
        .and_then(Value::as_str)
        .unwrap_or("next profile");
    let reason = obj
        .get("reason")
        .and_then(Value::as_str)
        .map(friendly_reason)
        .unwrap_or("error");
    format!("🔄 _Rotating auth profile to **{to_profile}** ({reason})_")
}

fn format_context_compacted(obj: &serde_json::Map<String, Value>) -> Option<String> {
    let data = obj.get("data").and_then(Value::as_object)?;
    let tier = data
        .get("tier_applied")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if tier < 2 {
        return None;
    }
    // Tier 3/4 emit live-only start markers before the actual compaction;
    // the final event arrives next with real `messages_affected`. Suppress
    // starts so IM doesn't get two notices per compaction.
    if matches!(
        data.get("description").and_then(Value::as_str),
        Some("summarizing" | "emergency_compacting")
    ) {
        return None;
    }
    let msgs = data.get("messages_affected").and_then(Value::as_u64);
    let body = match msgs {
        Some(0) | None => format!("📚 _Context compacted (tier {tier})_"),
        Some(1) => format!("📚 _Context compacted (tier {tier}, 1 msg)_"),
        Some(n) => format!("📚 _Context compacted (tier {tier}, {n} msgs)_"),
    };
    Some(body)
}

fn format_thinking_auto_disabled() -> String {
    "🧠 _Reasoning unavailable on this model — continuing without thinking._".to_string()
}

fn format_vision_auto_disabled() -> String {
    "🖼️ _This model can't read images — continuing with the image(s) ignored._".to_string()
}

/// Map serialized [`crate::failover::FailoverReason`] (snake_case) to a
/// short human phrase. Falls back to the raw string for unknown variants
/// so future enum additions still render something sensible.
fn friendly_reason(reason: &str) -> &str {
    match reason {
        "auth" => "auth issue",
        "rate_limit" => "rate limit",
        "overloaded" => "overloaded",
        "timeout" => "network",
        "billing" => "quota",
        "model_not_found" => "model unavailable",
        "context_overflow" => "context overflow",
        "unknown" => "error",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn model_fallback_full_payload() {
        let event = json!({
            "type": "model_fallback",
            "model": "OpenAI / gpt-4o",
            "from_model": "Anthropic / claude-sonnet-4-6",
            "provider_id": "openai",
            "model_id": "gpt-4o",
            "reason": "auth",
            "attempt": 2,
            "total": 3,
            "error": "401 Unauthorized",
        });
        assert_eq!(
            format_im_system_event(&event).as_deref(),
            Some("⤵️ _Switching to **OpenAI / gpt-4o** — auth issue, attempt 2/3_"),
        );
    }

    #[test]
    fn model_fallback_missing_attempt_total_still_renders() {
        let event = json!({
            "type": "model_fallback",
            "model": "OpenAI / gpt-4o",
            "reason": "rate_limit",
        });
        assert_eq!(
            format_im_system_event(&event).as_deref(),
            Some("⤵️ _Switching to **OpenAI / gpt-4o** — rate limit_"),
        );
    }

    #[test]
    fn model_fallback_missing_model_falls_back() {
        let event = json!({
            "type": "model_fallback",
            "reason": "timeout",
            "attempt": 1,
            "total": 2,
        });
        let out = format_im_system_event(&event).expect("should render");
        assert!(out.contains("unknown model"));
        assert!(out.contains("network"));
    }

    #[test]
    fn profile_rotation_full_payload() {
        let event = json!({
            "type": "profile_rotation",
            "provider_id": "openai",
            "model_id": "gpt-4o",
            "from_profile": "primary",
            "to_profile": "secondary",
            "reason": "rate_limit",
        });
        assert_eq!(
            format_im_system_event(&event).as_deref(),
            Some("🔄 _Rotating auth profile to **secondary** (rate limit)_"),
        );
    }

    #[test]
    fn profile_rotation_missing_to_profile_falls_back() {
        let event = json!({
            "type": "profile_rotation",
            "from_profile": "primary",
        });
        let out = format_im_system_event(&event).expect("should render");
        assert!(out.contains("next profile"));
        assert!(out.contains("error"));
    }

    #[test]
    fn context_compacted_tier_3_shows_msg_count() {
        let event = json!({
            "type": "context_compacted",
            "data": {
                "tier_applied": 3,
                "messages_affected": 12,
                "description": "summarize",
            }
        });
        assert_eq!(
            format_im_system_event(&event).as_deref(),
            Some("📚 _Context compacted (tier 3, 12 msgs)_"),
        );
    }

    #[test]
    fn context_compacted_tier_2_singular() {
        let event = json!({
            "type": "context_compacted",
            "data": {
                "tier_applied": 2,
                "messages_affected": 1,
            }
        });
        assert_eq!(
            format_im_system_event(&event).as_deref(),
            Some("📚 _Context compacted (tier 2, 1 msg)_"),
        );
    }

    #[test]
    fn context_compacted_tier_1_suppressed() {
        let event = json!({
            "type": "context_compacted",
            "data": { "tier_applied": 1, "messages_affected": 3 }
        });
        assert_eq!(format_im_system_event(&event), None);
    }

    #[test]
    fn context_compacted_tier_0_suppressed() {
        let event = json!({
            "type": "context_compacted",
            "data": { "tier_applied": 0, "messages_affected": 5 }
        });
        assert_eq!(format_im_system_event(&event), None);
    }

    #[test]
    fn context_compacted_summarizing_start_marker_suppressed() {
        // Tier 3 emits this BEFORE the actual compaction completes — must
        // not turn into an IM notice (the final event with real
        // `messages_affected` arrives next).
        let event = json!({
            "type": "context_compacted",
            "data": {
                "tier_applied": 3,
                "description": "summarizing",
                "messages_to_summarize": 12,
            }
        });
        assert_eq!(format_im_system_event(&event), None);
    }

    #[test]
    fn context_compacted_emergency_start_marker_suppressed() {
        let event = json!({
            "type": "context_compacted",
            "data": {
                "tier_applied": 4,
                "description": "emergency_compacting",
                "attempt": 1,
                "max_attempts": 1,
            }
        });
        assert_eq!(format_im_system_event(&event), None);
    }

    #[test]
    fn context_compacted_missing_msgs_omits_count() {
        let event = json!({
            "type": "context_compacted",
            "data": { "tier_applied": 3 }
        });
        assert_eq!(
            format_im_system_event(&event).as_deref(),
            Some("📚 _Context compacted (tier 3)_"),
        );
    }

    #[test]
    fn context_compacted_no_data_returns_none() {
        let event = json!({ "type": "context_compacted" });
        assert_eq!(format_im_system_event(&event), None);
    }

    #[test]
    fn thinking_auto_disabled_payload() {
        let event = json!({
            "type": "thinking_auto_disabled",
            "provider_id": "openai",
            "provider_name": "OpenAI",
            "model_id": "gpt-4o-mini",
        });
        assert_eq!(
            format_im_system_event(&event).as_deref(),
            Some("🧠 _Reasoning unavailable on this model — continuing without thinking._"),
        );
    }

    #[test]
    fn unknown_event_type_returns_none() {
        let event = json!({ "type": "tool_call", "name": "exec" });
        assert_eq!(format_im_system_event(&event), None);
    }

    #[test]
    fn missing_type_returns_none() {
        let event = json!({ "data": "stuff" });
        assert_eq!(format_im_system_event(&event), None);
    }

    #[test]
    fn non_object_payload_returns_none() {
        let event = json!("hello");
        assert_eq!(format_im_system_event(&event), None);
        let event = json!([1, 2, 3]);
        assert_eq!(format_im_system_event(&event), None);
    }

    #[test]
    fn unknown_failover_reason_falls_through() {
        // Future FailoverReason variants should still render with the raw token
        // rather than producing "error".
        let event = json!({
            "type": "model_fallback",
            "model": "OpenAI / gpt-5",
            "reason": "novel_kind_of_failure",
        });
        let out = format_im_system_event(&event).expect("should render");
        assert!(out.contains("novel_kind_of_failure"));
    }
}
