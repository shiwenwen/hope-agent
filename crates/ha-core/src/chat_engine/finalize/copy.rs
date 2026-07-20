//! Three-way copy bank for [`TerminationReason`].
//!
//! All non-natural turn endings produce three rendered strings:
//!
//! - [`model_marker`] — `context_json` `[系统事件] ...` body. Imperative
//!   tone: tells the model what happened and how the next turn should
//!   behave (don't apologize, don't re-run interrupted tools, etc.).
//! - [`user_notice`] — `messages.role=event` body. Declarative tone for
//!   the GUI's existing event banner pipeline.
//! - [`im_notice`] — IM-side notice text. Delegates to the existing
//!   `chat_engine::im_error_message` helpers so user-facing IM copy
//!   stays consistent with current channel output.
//!
//! All `<message>` interpolations are passed through
//! [`crate::logging::redact_sensitive`] before being inserted.

use crate::chat_engine::im_error_message::{
    format_im_engine_error, sanitize_raw, ImErrorContext, CANCEL_NOTICE,
};
use crate::failover::FailoverReason;
use crate::util::truncate_utf8;

use super::TerminationReason;

/// Hard cap on raw provider error text spliced into model-facing
/// markers. The model only needs enough signal to classify the failure;
/// longer dumps waste tokens.
const MODEL_RAW_MAX_BYTES: usize = 2_000;

/// User notice rows are surfaced as a small banner — keep them tight.
const USER_NOTICE_RAW_MAX_BYTES: usize = 500;

// ── Model marker ──────────────────────────────────────────────────────

/// Imperative `[系统事件]` text appended as the final assistant message
/// in `context_json`. This is what the model reads next turn.
///
/// Per-reason wording is fixed (no partial summary inline) — the
/// structural partial blocks (text / thinking / tool_use / tool_result)
/// are pushed *before* this marker, so the marker only needs to
/// classify what happened and steer the next turn.
pub fn model_marker(reason: &TerminationReason) -> String {
    match reason {
        TerminationReason::UserStop => {
            "[系统事件] 用户主动停止了此轮回复。上方的内容是停止前已产生的部分;\
             如非用户明确要求,不要为此道歉或重新生成,直接回应用户的下一条消息即可。"
                .to_string()
        }
        TerminationReason::RuntimeCancel => {
            "[系统事件] 此轮回复因运行任务被取消而中断。上方已产生并确认持久化的内容和工具调用已保留。"
                .to_string()
        }
        TerminationReason::NoProfileAvailable => {
            "[系统事件] 此轮无法启动:没有可用的 API 凭据(所有 Profile 被禁用、\
             处于冷却期或未配置)。这是配置问题,请提醒用户检查 Provider 设置后再重试。"
                .to_string()
        }
        TerminationReason::ProviderFailed {
            last_kind,
            last_message,
            ..
        } => model_marker_provider_failed(*last_kind, last_message),
        TerminationReason::CompactionFailed { detail } => {
            let detail = sanitize_for_model(detail);
            format!(
                "[系统事件] 对话已过长,紧急上下文压缩也无法恢复。详情:{}。\
                 请提醒用户开启新会话或精简对话。",
                detail
            )
        }
        TerminationReason::Shutdown => {
            "[系统事件] 应用在此轮回复过程中被关闭。上方已产生的部分内容和工具调用\
             已保留。用户重新打开会话后,请从中断处继续。"
                .to_string()
        }
        TerminationReason::Crash => {
            "[系统事件] 上一轮回复意外中断(进程异常退出)。上方已产生的部分内容已\
             保留,但可能不完整。"
                .to_string()
        }
        TerminationReason::Other { message } => {
            let msg = sanitize_for_model(message);
            format!(
                "[系统事件] 上一轮回复因内部异常而结束:{}。上方已产生的部分内容\
                 (如有)已保留。",
                msg
            )
        }
    }
}

fn model_marker_provider_failed(kind: FailoverReason, raw: &str) -> String {
    let msg = sanitize_for_model(raw);
    match kind {
        FailoverReason::EvaluationBudget => format!(
            "[系统事件] 本次受保护评测已达到不可变预算上限:{}。不得重试、切换凭据或继续产生外部副作用。",
            msg
        ),
        FailoverReason::Auth => format!(
            "[系统事件] 所有已配置模型都认证失败。最后一次错误:{}。请提醒用户检查 \
             API Key 或 OAuth 登录状态。",
            msg
        ),
        FailoverReason::Billing => format!(
            "[系统事件] 所有已配置模型都遇到计费或配额问题。最后一次错误:{}。\
             请提醒用户检查订阅状态或账户余额。",
            msg
        ),
        FailoverReason::RateLimit => format!(
            "[系统事件] 所有已配置模型都被上游限流。最后一次错误:{}。\
             上方已产生的部分内容已保留;请提醒用户稍后重试或切换备用模型。",
            msg
        ),
        FailoverReason::Overloaded => format!(
            "[系统事件] 所有已配置模型的上游服务暂时繁忙或不可用。最后一次错误:{}。\
             上方已产生的部分内容已保留;请提醒用户稍后重试或切换备用模型。",
            msg
        ),
        FailoverReason::Timeout => format!(
            "[系统事件] 所有已配置模型都因网络连接、DNS、代理或请求超时而不可达。\
             最后一次错误:{}。上方已产生的部分内容已保留;请提醒用户检查网络/代理或稍后重试。",
            msg
        ),
        FailoverReason::ContextOverflow => format!(
            "[系统事件] 对话超出所有模型的上下文窗口,自动压缩失败。详情:{}。\
             请提醒用户开启新会话或精简对话。",
            msg
        ),
        FailoverReason::ModelNotFound => format!(
            "[系统事件] 所有已配置模型均不可用。最后一次错误:{}。请提醒用户在设置\
             中选择其他模型。",
            msg
        ),
        FailoverReason::Unknown => format!(
            "[系统事件] 所有已配置模型都失败了。最后一次错误:{}。上方已产生的部分\
             内容已保留。",
            msg
        ),
    }
}

// ── User notice ───────────────────────────────────────────────────────

/// Declarative banner text written to a `messages.role=event` row.
/// Rendered by the existing GUI event pipeline (centered system row).
pub fn user_notice(reason: &TerminationReason) -> String {
    match reason {
        TerminationReason::UserStop => "已停止此次回复".to_string(),
        TerminationReason::RuntimeCancel => "回复任务已中断，已保留中断前的内容".to_string(),
        TerminationReason::NoProfileAvailable => {
            "无可用 API 凭据。请检查 Provider 设置".to_string()
        }
        TerminationReason::ProviderFailed {
            last_kind,
            last_message,
            ..
        } => user_notice_provider_failed(*last_kind, last_message),
        TerminationReason::CompactionFailed { .. } => {
            "对话过长,无法继续。建议开启新会话或精简对话".to_string()
        }
        TerminationReason::Shutdown => "应用已关闭,中断前的内容已保留".to_string(),
        TerminationReason::Crash => "上次会话异常中断,已保留中断前的内容".to_string(),
        TerminationReason::Other { message } => {
            let msg = sanitize_for_user(message);
            format!("内部异常:{}。已保留中断前的内容", msg)
        }
    }
}

fn user_notice_provider_failed(kind: FailoverReason, raw: &str) -> String {
    let msg = sanitize_for_user(raw);
    match kind {
        FailoverReason::EvaluationBudget => "本次评测已达到预算上限，已停止继续调用".to_string(),
        FailoverReason::Auth => "所有模型认证失败。请检查 API Key 或 OAuth 登录".to_string(),
        FailoverReason::Billing => "所有模型计费/配额问题。请检查订阅或余额".to_string(),
        FailoverReason::RateLimit => "所有模型被上游限流。可稍后重试或切换备用模型".to_string(),
        FailoverReason::Overloaded => {
            "所有模型上游服务暂时繁忙。可稍后重试或切换备用模型".to_string()
        }
        FailoverReason::Timeout => "所有模型网络不可达。请检查网络/代理/DNS,或稍后重试".to_string(),
        FailoverReason::ContextOverflow => "对话超出所有模型上下文窗口".to_string(),
        FailoverReason::ModelNotFound => "所有模型均不可用。请在设置中选择其他模型".to_string(),
        FailoverReason::Unknown => format!("所有模型失败:{}", msg),
    }
}

// ── IM notice ─────────────────────────────────────────────────────────

/// IM-side text. Delegates to the existing `im_error_message` helpers
/// for `ProviderFailed` (its emoji + headline + sanitized blockquote
/// rendering is battle-tested), and provides new short strings for the
/// other reasons that did not have IM coverage before.
pub fn im_notice(reason: &TerminationReason) -> String {
    match reason {
        TerminationReason::UserStop => CANCEL_NOTICE.to_string(),
        TerminationReason::RuntimeCancel => {
            "⏸ **Response task was cancelled** — the durable partial response is preserved."
                .to_string()
        }
        TerminationReason::ProviderFailed {
            last_kind,
            last_message,
            is_codex_auth,
        } => format_im_engine_error(ImErrorContext {
            reason: *last_kind,
            raw: last_message,
            is_codex_auth: *is_codex_auth,
        }),
        TerminationReason::NoProfileAvailable => {
            "🔧 **No auth profile available** — configure provider in settings, then retry."
                .to_string()
        }
        TerminationReason::CompactionFailed { .. } => {
            "📚 **Conversation context too large to compact** — start a new session.".to_string()
        }
        TerminationReason::Shutdown => {
            "⏸ **App is shutting down** — partial response above is preserved.".to_string()
        }
        TerminationReason::Crash => {
            "⚠️ **The previous run ended unexpectedly** — the partial response is preserved."
                .to_string()
        }
        TerminationReason::Other { message } => {
            let msg = sanitize_for_user(message);
            format!("⚠️ Internal error: {msg}")
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────

fn sanitize_for_model(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "(no detail)".to_string();
    }
    // `sanitize_raw` already does redact_sensitive + bearer scrub +
    // known-token-shape scrub + whitespace collapse; reusing it keeps
    // model-facing and IM-facing copy consistently sanitized.
    let cleaned = sanitize_raw(trimmed);
    truncate_utf8(&cleaned, MODEL_RAW_MAX_BYTES).to_string()
}

fn sanitize_for_user(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "(no detail)".to_string();
    }
    let cleaned = sanitize_raw(trimmed);
    truncate_utf8(&cleaned, USER_NOTICE_RAW_MAX_BYTES).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_stop_copies_are_concise_and_non_error() {
        let r = TerminationReason::UserStop;
        assert!(model_marker(&r).contains("用户主动停止"));
        assert_eq!(user_notice(&r), "已停止此次回复");
        assert_eq!(im_notice(&r), CANCEL_NOTICE);
    }

    #[test]
    fn provider_failed_redacts_credentials_in_marker() {
        // TOKEN_RE in im_error_message::sanitize_raw requires ≥20 chars
        // after the `sk-` / `pat-` prefix; below mirrors a realistic
        // 40-char OpenAI key body, plus an `Authorization: Bearer`
        // header for defense-in-depth.
        let raw = "Invalid: sk-ant-abcdefghijklmnopqrstuvwx0123456789 Authorization: Bearer xyz";
        let r = TerminationReason::ProviderFailed {
            last_kind: FailoverReason::Auth,
            last_message: raw.into(),
            is_codex_auth: false,
        };
        let marker = model_marker(&r);
        assert!(marker.contains("认证失败"));
        assert!(
            !marker.contains("sk-ant-abcdefghijklmnopqrstuvwx0123456789"),
            "raw key leaked into marker: {marker}"
        );
        assert!(
            !marker.to_lowercase().contains("bearer xyz"),
            "bearer header leaked: {marker}"
        );
    }

    #[test]
    fn no_profile_marker_distinct_from_provider_failed() {
        let m = model_marker(&TerminationReason::NoProfileAvailable);
        assert!(m.contains("无法启动"));
        assert!(m.contains("配置"));
    }

    #[test]
    fn provider_timeout_notice_points_to_network() {
        let r = TerminationReason::ProviderFailed {
            last_kind: FailoverReason::Timeout,
            last_message: "Codex API request failed: error sending request for url".into(),
            is_codex_auth: false,
        };

        let marker = model_marker(&r);
        assert!(marker.contains("网络连接"));
        assert!(marker.contains("DNS"));
        assert_eq!(
            user_notice(&r),
            "所有模型网络不可达。请检查网络/代理/DNS,或稍后重试"
        );
    }

    #[test]
    fn shutdown_user_notice_is_calm_not_an_error() {
        let n = user_notice(&TerminationReason::Shutdown);
        assert!(!n.contains("失败"));
        assert!(n.contains("应用"));
    }

    #[test]
    fn other_with_empty_message_does_not_crash() {
        let r = TerminationReason::Other {
            message: "   ".into(),
        };
        let m = model_marker(&r);
        assert!(m.contains("内部异常"));
    }
}
