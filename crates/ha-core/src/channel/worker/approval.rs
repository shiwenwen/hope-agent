//! IM channel tool approval interaction.
//!
//! When a tool requires approval during an IM channel conversation, this module
//! intercepts the `"approval_required"` EventBus event, sends an approval prompt
//! to the IM channel (with buttons if supported, text fallback otherwise), and
//! routes the user's response back to `submit_approval_response()`.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::channel::db::ChannelDB;
use crate::channel::registry::ChannelRegistry;
use crate::channel::types::{InlineButton, ReplyPayload};
use crate::tools::approval::{
    submit_approval_response, ApprovalReasonKind, ApprovalReasonPayload, ApprovalResponse,
};
use crate::ttl_cache::TtlCache;

use std::sync::Arc;

/// Callback data prefix for approval buttons across all channels.
const APPROVAL_PREFIX: &str = "approval:";

// ── Pending text-reply approvals ─────────────────────────────────

/// Tracks a pending approval that awaits a text reply (for channels without buttons).
#[derive(Debug, Clone)]
struct PendingTextApproval {
    request_id: String,
}

/// Registry of pending text-reply approvals, keyed by (account_id, chat_id).
/// Only used for channels that don't support buttons.
static TEXT_PENDING: OnceLock<Mutex<HashMap<(String, String), Vec<PendingTextApproval>>>> =
    OnceLock::new();

fn get_text_pending() -> &'static Mutex<HashMap<(String, String), Vec<PendingTextApproval>>> {
    TEXT_PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Throttle for the "you have N pending approvals" hint — one nudge per
/// (account, chat) per [`HINT_THROTTLE`]. Backed by [`TtlCache`] so stale
/// entries auto-expire (bounded memory across long-lived IM deployments).
/// Capacity 1024 is generous for any plausible per-process chat count.
static HINT_THROTTLE_CACHE: OnceLock<TtlCache<(String, String), ()>> = OnceLock::new();

fn get_hint_throttle() -> &'static TtlCache<(String, String), ()> {
    HINT_THROTTLE_CACHE.get_or_init(|| TtlCache::new(1024))
}

const HINT_THROTTLE: Duration = Duration::from_secs(60);

/// Remove any in-memory pending text-reply state for `request_id`. Called by
/// the tool execution path when an approval is timed out / cancelled /
/// otherwise resolved without an IM reply, so stale entries don't
/// accumulate. Mirrors [`super::ask_user::drop_pending_by_request_id`].
pub async fn drop_pending_by_request_id(request_id: &str) {
    let mut map = get_text_pending().lock().await;
    let mut empty_keys = Vec::new();
    for (key, list) in map.iter_mut() {
        list.retain(|p| p.request_id != request_id);
        if list.is_empty() {
            empty_keys.push(key.clone());
        }
    }
    for k in empty_keys {
        map.remove(&k);
    }
}

// ── InlineButton helper ──────────────────────────────────────────

impl InlineButton {
    /// Returns the effective callback identifier: `callback_data` if set, otherwise `text`.
    pub fn callback_id(&self) -> &str {
        self.callback_data.as_deref().unwrap_or(&self.text)
    }
}

// ── Approval button builder ──────────────────────────────────────

/// Build the standard 3-button row for approval prompts.
/// The `callback_data` format is `approval:{request_id}:{action}`.
pub(crate) fn build_approval_buttons(request_id: &str) -> Vec<Vec<InlineButton>> {
    vec![vec![
        InlineButton {
            text: "✅ Allow Once".to_string(),
            callback_data: Some(format!("{}{}:allow_once", APPROVAL_PREFIX, request_id)),
            url: None,
        },
        InlineButton {
            text: "🔓 Always Allow".to_string(),
            callback_data: Some(format!("{}{}:allow_always", APPROVAL_PREFIX, request_id)),
            url: None,
        },
        InlineButton {
            text: "❌ Deny".to_string(),
            callback_data: Some(format!("{}{}:deny", APPROVAL_PREFIX, request_id)),
            url: None,
        },
    ]]
}

/// Render the Smart-mode judge rationale as a one-line suffix, or empty for
/// other AskReason kinds. Other kinds (protected path, dangerous command, …)
/// are intentionally not surfaced to IM users yet — see review-followups.
fn smart_judge_line(reason: Option<&ApprovalReasonPayload>) -> String {
    let Some(r) = reason else {
        return String::new();
    };
    if r.kind != ApprovalReasonKind::SmartJudge {
        return String::new();
    }
    let Some(detail) = r.detail.as_deref() else {
        return String::new();
    };
    let trimmed = detail.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let snippet = crate::truncate_utf8(trimmed, 280);
    format!("\n💭 Smart Judge: {}", snippet)
}

/// Format the approval prompt text (plain text, no HTML — works across all channels).
fn format_approval_text(command: &str, reason: Option<&ApprovalReasonPayload>) -> String {
    let preview = crate::truncate_utf8(command, 500);
    format!(
        "🔐 Tool approval required\n\n{}{}",
        preview,
        smart_judge_line(reason)
    )
}

/// Short visible tag for a `request_id`, used to disambiguate multiple
/// pending approvals when the user replies. Six UTF-8 chars keeps
/// collisions effectively impossible at the per-(account, chat) scope —
/// `truncate_utf8` stays safe even if the id generator ever moves off
/// ASCII UUIDs.
fn id_tag(request_id: &str) -> &str {
    crate::truncate_utf8(request_id, 6)
}

/// Format the text-only approval prompt (for channels without buttons).
/// Includes the `#tag` so the user can target a specific pending approval
/// (`yes#abc123`) when several are queued; bare replies (`yes` / `1`) fall
/// back to LIFO order.
///
/// `stack_depth` is the number of pending approvals (including this one)
/// in the current (account, chat). When >1 the reply hint nudges the user
/// to disambiguate with `#tag`.
fn format_text_approval(
    command: &str,
    reason: Option<&ApprovalReasonPayload>,
    request_id: &str,
    stack_depth: usize,
) -> String {
    let preview = crate::truncate_utf8(command, 500);
    let tag = id_tag(request_id);
    let stack_hint = if stack_depth > 1 {
        format!("\n\n({stack_depth} pending — append `#{tag}` to target this one specifically)")
    } else {
        String::new()
    };
    format!(
        "🔐 Tool approval required #{tag}:\n{preview}{smart}\n\nReply within 5 min:\n  1 / yes / ok — Allow once\n  2 / always   — Always allow\n  3 / no / deny — Deny\n(中文也可: 同意 / 总是 / 拒绝){stack_hint}",
        smart = smart_judge_line(reason)
    )
}

// ── Text reply parsing ───────────────────────────────────────────

/// Parsed approval verb plus an optional `#<id>` suffix the user appended
/// to target a specific pending approval.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedReply<'a> {
    response: ApprovalResponse,
    id_suffix: Option<&'a str>,
}

/// Match `raw` against the approval reply whitelist.
///
/// Whitespace-trimmed, case-insensitive, supports both English and Chinese
/// aliases. An optional `#<id>` suffix routes to a specific pending approval
/// instead of the LIFO top (`yes#abc123` / `3#abc123`).
///
/// `AllowAlways` is matched before `AllowOnce` so the literal `yes always`
/// resolves to `AllowAlways` instead of being eaten by the `yes` arm.
fn parse_approval_reply(raw: &str) -> Option<ParsedReply<'_>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (verb_part, id_suffix) = match trimmed.split_once('#') {
        Some((v, id)) => {
            let id_trimmed = id.trim();
            if id_trimmed.is_empty() {
                return None;
            }
            (v.trim_end(), Some(id_trimmed))
        }
        None => (trimmed, None),
    };
    // Whitelist is pure ASCII + CJK; CJK has no case variants and
    // `to_ascii_lowercase` is allocation-free for already-lowercase input,
    // so this is both correct and cheaper than `to_lowercase`.
    let lower = verb_part.to_ascii_lowercase();
    let response = match lower.as_str() {
        "2" | "a" | "always" | "yes always" | "yesalways" | "总是" | "总是允许" | "永远"
        | "始终" => ApprovalResponse::AllowAlways,
        "1" | "y" | "yes" | "ok" | "okay" | "allow" | "approve" | "好" | "好的" | "同意"
        | "允许" | "可以" | "行" => ApprovalResponse::AllowOnce,
        "3" | "n" | "no" | "deny" | "block" | "stop" | "cancel" | "不" | "不行" | "拒绝" | "否"
        | "取消" => ApprovalResponse::Deny,
        _ => return None,
    };
    Some(ParsedReply {
        response,
        id_suffix,
    })
}

// ── Shared callback handler (eliminates boilerplate in channel plugins) ──

/// Spawn a background task to handle an approval callback and log the result.
/// Used by channel plugins (Slack, Feishu, QQ Bot, LINE, Google Chat) that
/// don't need platform-specific post-processing after the approval.
pub fn spawn_callback_handler(data: &str, source: &'static str) {
    let data = data.to_string();
    tokio::spawn(async move {
        match handle_approval_callback(&data).await {
            Ok(label) => app_info!("channel", source, "Approval: {}", label),
            Err(e) => app_warn!("channel", source, "Approval failed: {}", e),
        }
    });
}

// ── EventBus listener ────────────────────────────────────────────

/// Spawn a background task that listens for `"approval_required"` events on
/// the EventBus and forwards them to the appropriate IM channel.
pub fn spawn_channel_approval_listener(channel_db: Arc<ChannelDB>, registry: Arc<ChannelRegistry>) {
    let Some(bus) = crate::globals::get_event_bus() else {
        return;
    };
    let mut rx = bus.subscribe();

    tokio::spawn(async move {
        loop {
            let event = match rx.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    app_warn!(
                        "channel",
                        "approval",
                        "Approval listener lagged {} events",
                        n
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            };

            match event.name.as_str() {
                "approval_required" => {} // fall through to dispatch below
                "approval_timed_out" => {
                    handle_timeout_event(
                        event.payload.clone(),
                        channel_db.clone(),
                        registry.clone(),
                    )
                    .await;
                    continue;
                }
                _ => continue,
            }

            // Deserialize the approval request
            let request: crate::tools::approval::ApprovalRequest =
                match serde_json::from_value(event.payload.clone()) {
                    Ok(r) => r,
                    Err(e) => {
                        app_warn!(
                            "channel",
                            "approval",
                            "Failed to parse approval request: {}",
                            e
                        );
                        continue;
                    }
                };

            let Some(ref session_id) = request.session_id else {
                continue;
            };

            // Look up which channel conversation this session belongs to
            let conversation = match channel_db.get_conversation_by_session(session_id) {
                Ok(Some(conv)) => conv,
                Ok(None) => continue,
                Err(e) => {
                    app_warn!(
                        "channel",
                        "approval",
                        "Failed to look up channel session {}: {}",
                        session_id,
                        e
                    );
                    continue;
                }
            };

            // Load account config
            let store = crate::config::cached_config();
            let account_config = match store.channels.find_account(&conversation.account_id) {
                Some(c) => c.clone(),
                None => continue,
            };

            let channel_id: crate::channel::types::ChannelId = match serde_json::from_value(
                serde_json::Value::String(conversation.channel_id.clone()),
            ) {
                Ok(id) => id,
                Err(_) => continue,
            };

            let supports_buttons = registry
                .get_plugin(&channel_id)
                .map(|p| p.capabilities().supports_buttons)
                .unwrap_or(false);

            // Send the approval prompt to the IM channel
            let payload = if supports_buttons {
                ReplyPayload {
                    text: Some(format_approval_text(
                        &request.command,
                        request.reason.as_ref(),
                    )),
                    buttons: build_approval_buttons(&request.request_id),
                    thread_id: conversation.thread_id.clone(),
                    ..ReplyPayload::text("")
                }
            } else {
                // Register for text-reply routing. Compute stack_depth inside
                // the same lock so the rendered prompt's "N pending" line
                // matches what `try_handle_approval_reply` will see.
                let key = (
                    conversation.account_id.clone(),
                    conversation.chat_id.clone(),
                );
                let stack_depth = {
                    let mut pending = get_text_pending().lock().await;
                    let list = pending.entry(key).or_default();
                    list.push(PendingTextApproval {
                        request_id: request.request_id.clone(),
                    });
                    list.len()
                };

                ReplyPayload {
                    text: Some(format_text_approval(
                        &request.command,
                        request.reason.as_ref(),
                        &request.request_id,
                        stack_depth,
                    )),
                    thread_id: conversation.thread_id.clone(),
                    ..ReplyPayload::text("")
                }
            };

            if let Err(e) = registry
                .send_reply(&account_config, &conversation.chat_id, &payload)
                .await
            {
                app_warn!(
                    "channel",
                    "approval",
                    "Failed to send approval prompt to channel: {}",
                    e
                );
            }
        }
    });
}

/// Wire payload of the `approval_timed_out` EventBus event. Tools side
/// (`tools::approval::check_and_request_approval` timeout branch) emits
/// this so the IM channel listener can notify the user — the actual list
/// cleanup is independently handled by [`drop_pending_by_request_id`]
/// also called from the tools side, so this listener never has to touch
/// `TEXT_PENDING`.
#[derive(serde::Deserialize)]
struct ApprovalTimedOut {
    request_id: String,
    session_id: Option<String>,
    #[serde(default)]
    timeout_secs: u64,
}

/// Tell the IM user the approval prompt expired. Best-effort — if the
/// channel is offline we just log; the tool-side timeout (deny / proceed
/// per config) has already taken effect, and the IM-side `TEXT_PENDING`
/// entry has already been cleared by the tools side calling
/// [`drop_pending_by_request_id`].
async fn handle_timeout_event(
    payload: serde_json::Value,
    channel_db: Arc<ChannelDB>,
    registry: Arc<ChannelRegistry>,
) {
    let event: ApprovalTimedOut = match serde_json::from_value(payload) {
        Ok(e) => e,
        Err(err) => {
            app_warn!(
                "channel",
                "approval",
                "Failed to parse approval_timed_out payload: {}",
                err
            );
            return;
        }
    };
    let Some(session_id) = event.session_id else {
        return;
    };

    let conversation = match channel_db.get_conversation_by_session(&session_id) {
        Ok(Some(c)) => c,
        Ok(None) => return, // not an IM session — desktop handles its own UI
        Err(e) => {
            app_warn!(
                "channel",
                "approval",
                "Timeout lookup failed for session {}: {}",
                session_id,
                e
            );
            return;
        }
    };

    let store = crate::config::cached_config();
    let account_config = match store.channels.find_account(&conversation.account_id) {
        Some(c) => c.clone(),
        None => return,
    };

    let tag = id_tag(&event.request_id);
    let timeout_secs = event.timeout_secs;
    let body = format!(
        "⏱ Tool approval #{tag} timed out after {timeout_secs}s. The tool call has been denied — ask me again if you still want it to run."
    );
    let payload = ReplyPayload {
        text: Some(body),
        thread_id: conversation.thread_id.clone(),
        ..ReplyPayload::text("")
    };
    if let Err(e) = registry
        .send_reply(&account_config, &conversation.chat_id, &payload)
        .await
    {
        app_warn!(
            "channel",
            "approval",
            "Failed to send approval-timeout notice: {}",
            e
        );
    }
}

// ── Text-reply approval handler ──────────────────────────────────

/// Try to handle an inbound message as an approval text reply.
///
/// Returns `true` if the message was consumed as an approval reply,
/// `false` if it should proceed through normal message processing.
pub async fn try_handle_approval_reply(msg: &crate::channel::types::MsgContext) -> bool {
    let Some(raw) = msg.text.as_deref() else {
        return false;
    };
    let Some(parsed) = parse_approval_reply(raw) else {
        return false;
    };

    let key = (msg.account_id.clone(), msg.chat_id.clone());
    let request_id = {
        let mut pending = get_text_pending().lock().await;
        let Some(list) = pending.get_mut(&key) else {
            return false;
        };
        if list.is_empty() {
            pending.remove(&key);
            return false;
        }
        // `#<id>` suffix targets a specific pending approval by short tag
        // (`id_tag` prefix match). Without suffix, fall back to LIFO so the
        // most-recently-prompted approval is the default — matches what's
        // visually on screen.
        let popped = match parsed.id_suffix {
            Some(target) => list
                .iter()
                .position(|entry| id_tag(&entry.request_id) == target)
                .map(|idx| list.remove(idx)),
            None => list.pop(),
        };
        let Some(entry) = popped else {
            // Suffix provided but no pending entry matched. Treat as a
            // non-approval message so the user's typo doesn't get eaten.
            return false;
        };
        if list.is_empty() {
            pending.remove(&key);
        }
        entry.request_id
    };

    match submit_approval_response(&request_id, parsed.response).await {
        Ok(()) => true,
        Err(e) => {
            // Approval already expired (5-min timeout) — don't consume the message
            app_warn!(
                "channel",
                "approval",
                "Approval expired or invalid ({}), passing message through",
                e
            );
            false
        }
    }
}

/// Best-effort nudge for users whose chat has pending text-mode approvals
/// but who sent something that isn't a reply (e.g. a fresh question while
/// the approval prompt is still up). Sends one line per (account, chat)
/// per [`HINT_THROTTLE`], not on every non-matching message.
///
/// No-op for accounts on button-capable channels (they never have entries
/// in `TEXT_PENDING`). Called by the dispatcher after
/// [`try_handle_approval_reply`] returns `false`.
pub async fn maybe_send_pending_hint(
    msg: &crate::channel::types::MsgContext,
    registry: &ChannelRegistry,
) {
    let key = (msg.account_id.clone(), msg.chat_id.clone());

    let stack_depth = {
        let pending = get_text_pending().lock().await;
        pending.get(&key).map(|list| list.len()).unwrap_or(0)
    };
    if stack_depth == 0 {
        return;
    }

    // Throttle gate: skip if we already nudged this chat recently. The
    // `TtlCache` bounds memory (capacity 1024) so long-running IM
    // deployments don't accumulate one entry per ever-seen (account, chat).
    let throttle = get_hint_throttle();
    if throttle.get(&key, HINT_THROTTLE).is_some() {
        return;
    }
    throttle.put(key, ());

    let store = crate::config::cached_config();
    let Some(account_config) = store.channels.find_account(&msg.account_id).cloned() else {
        return;
    };

    let body = format!(
        "ℹ️ You have {stack_depth} pending tool approval(s). Treating this as a new message. Reply `1` / `yes` to allow, `3` / `no` to deny — or append `#<tag>` to target a specific one."
    );
    let payload = ReplyPayload {
        text: Some(body),
        thread_id: msg.thread_id.clone(),
        ..ReplyPayload::text("")
    };
    if let Err(e) = registry
        .send_reply(&account_config, &msg.chat_id, &payload)
        .await
    {
        app_warn!(
            "channel",
            "approval",
            "Failed to send pending-approval hint: {}",
            e
        );
    }
}

// ── Callback approval handler (for button-based channels) ────────

/// Parse an approval callback string and submit the response.
///
/// `callback_data` format: `approval:{request_id}:{action}`
/// where action is one of: `allow_once`, `allow_always`, `deny`.
///
/// Returns `Ok(response_label)` on success for UI feedback, or `Err` on failure.
pub async fn handle_approval_callback(callback_data: &str) -> anyhow::Result<&'static str> {
    let rest = callback_data
        .strip_prefix(APPROVAL_PREFIX)
        .ok_or_else(|| anyhow::anyhow!("Not an approval callback"))?;

    let (request_id, action) = rest
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("Invalid approval callback format"))?;

    let (response, label) = match action {
        "allow_once" => (ApprovalResponse::AllowOnce, "✅ Allowed (once)"),
        "allow_always" => (ApprovalResponse::AllowAlways, "🔓 Always allowed"),
        "deny" => (ApprovalResponse::Deny, "❌ Denied"),
        _ => return Err(anyhow::anyhow!("Unknown approval action: {}", action)),
    };

    submit_approval_response(request_id, response).await?;
    Ok(label)
}

/// Check if a callback data string is an approval callback.
pub fn is_approval_callback(data: &str) -> bool {
    data.starts_with(APPROVAL_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smart(detail: Option<&str>) -> ApprovalReasonPayload {
        ApprovalReasonPayload {
            kind: ApprovalReasonKind::SmartJudge,
            detail: detail.map(|s| s.to_string()),
        }
    }

    fn other_reason() -> ApprovalReasonPayload {
        ApprovalReasonPayload {
            kind: ApprovalReasonKind::DangerousCommand,
            detail: Some("rm -rf".to_string()),
        }
    }

    #[test]
    fn smart_judge_line_renders_for_smart_only() {
        assert_eq!(smart_judge_line(None), "");
        assert_eq!(smart_judge_line(Some(&other_reason())), "");
        assert_eq!(smart_judge_line(Some(&smart(None))), "");
        assert_eq!(smart_judge_line(Some(&smart(Some("   ")))), "");
        let line = smart_judge_line(Some(&smart(Some("looks risky"))));
        assert_eq!(line, "\n💭 Smart Judge: looks risky");
    }

    #[test]
    fn smart_judge_line_truncates_long_detail() {
        let long = "x".repeat(1000);
        let line = smart_judge_line(Some(&smart(Some(&long))));
        assert!(line.starts_with("\n💭 Smart Judge: "));
        assert!(line.len() <= 320, "got len {}", line.len());
    }

    #[test]
    fn format_approval_text_includes_smart_judge_line() {
        let txt = format_approval_text("exec ls", Some(&smart(Some("trusted dir"))));
        assert!(txt.starts_with("🔐 Tool approval required\n\nexec ls"));
        assert!(txt.contains("💭 Smart Judge: trusted dir"));
    }

    #[test]
    fn format_approval_text_omits_line_when_no_smart_reason() {
        assert!(!format_approval_text("exec ls", None).contains("Smart Judge"));
        assert!(!format_approval_text("exec ls", Some(&other_reason())).contains("Smart Judge"));
    }

    #[test]
    fn format_text_approval_keeps_numeric_reply_block() {
        let txt = format_text_approval(
            "exec ls",
            Some(&smart(Some("ok per project rules"))),
            "abc123def456",
            1,
        );
        assert!(txt.contains("💭 Smart Judge: ok per project rules"));
        assert!(txt.contains("1 / yes / ok"));
        assert!(txt.contains("3 / no / deny"));
        // The visible #tag uses the 6-char prefix of the request id.
        assert!(txt.contains("#abc123"));
        // Smart Judge must precede the digit list so 1/2/3 parsing isn't shifted.
        let smart_idx = txt.find("Smart Judge").expect("has smart line");
        let reply_idx = txt.find("Reply within").expect("has reply block");
        assert!(smart_idx < reply_idx);
    }

    #[test]
    fn format_text_approval_renders_stack_hint_when_multiple_pending() {
        let single = format_text_approval("exec ls", None, "abcdef123456", 1);
        assert!(!single.contains("pending"));

        let multi = format_text_approval("exec ls", None, "abcdef123456", 3);
        assert!(multi.contains("3 pending"));
        assert!(multi.contains("#abcdef"));
    }

    #[test]
    fn parse_approval_reply_accepts_english_aliases() {
        for (input, expected) in [
            ("1", ApprovalResponse::AllowOnce),
            ("yes", ApprovalResponse::AllowOnce),
            ("YES", ApprovalResponse::AllowOnce),
            ("  Yes  ", ApprovalResponse::AllowOnce),
            ("y", ApprovalResponse::AllowOnce),
            ("ok", ApprovalResponse::AllowOnce),
            ("allow", ApprovalResponse::AllowOnce),
            ("2", ApprovalResponse::AllowAlways),
            ("always", ApprovalResponse::AllowAlways),
            ("yes always", ApprovalResponse::AllowAlways),
            ("3", ApprovalResponse::Deny),
            ("no", ApprovalResponse::Deny),
            ("N", ApprovalResponse::Deny),
            ("deny", ApprovalResponse::Deny),
            ("cancel", ApprovalResponse::Deny),
        ] {
            let parsed = parse_approval_reply(input).unwrap_or_else(|| panic!("failed: {input:?}"));
            assert_eq!(parsed.response, expected, "input {input:?}");
            assert!(parsed.id_suffix.is_none(), "input {input:?}");
        }
    }

    #[test]
    fn parse_approval_reply_accepts_chinese_aliases() {
        for (input, expected) in [
            ("好", ApprovalResponse::AllowOnce),
            ("好的", ApprovalResponse::AllowOnce),
            ("同意", ApprovalResponse::AllowOnce),
            ("允许", ApprovalResponse::AllowOnce),
            ("总是", ApprovalResponse::AllowAlways),
            ("永远", ApprovalResponse::AllowAlways),
            ("不", ApprovalResponse::Deny),
            ("拒绝", ApprovalResponse::Deny),
            ("取消", ApprovalResponse::Deny),
        ] {
            let parsed = parse_approval_reply(input).unwrap_or_else(|| panic!("failed: {input:?}"));
            assert_eq!(parsed.response, expected, "input {input:?}");
        }
    }

    #[test]
    fn parse_approval_reply_rejects_unrelated_text() {
        // Avoid false positives — "yesterday" must not match "yes" via
        // prefix or contains.
        assert!(parse_approval_reply("yesterday").is_none());
        assert!(parse_approval_reply("notnow").is_none());
        assert!(parse_approval_reply("好像").is_none());
        assert!(parse_approval_reply("").is_none());
        assert!(parse_approval_reply("   ").is_none());
        assert!(parse_approval_reply("帮我看看天气").is_none());
        assert!(parse_approval_reply("yes please").is_none());
    }

    #[test]
    fn parse_approval_reply_extracts_id_suffix() {
        let parsed = parse_approval_reply("yes#abc123").unwrap();
        assert_eq!(parsed.response, ApprovalResponse::AllowOnce);
        assert_eq!(parsed.id_suffix, Some("abc123"));

        let parsed = parse_approval_reply("3#xyz789").unwrap();
        assert_eq!(parsed.response, ApprovalResponse::Deny);
        assert_eq!(parsed.id_suffix, Some("xyz789"));

        // Trim whitespace around the suffix too.
        let parsed = parse_approval_reply("同意 #abc123 ").unwrap();
        assert_eq!(parsed.response, ApprovalResponse::AllowOnce);
        assert_eq!(parsed.id_suffix, Some("abc123"));

        // Empty suffix is rejected — `yes#` would otherwise route nowhere.
        assert!(parse_approval_reply("yes#").is_none());
        assert!(parse_approval_reply("yes#   ").is_none());
    }

    // The two tests below pin the **list-manipulation primitives** the
    // dispatcher path relies on (LIFO `pop` for bare verbs, `position +
    // remove` for `#tag` suffix). They deliberately do NOT call
    // `try_handle_approval_reply` end-to-end because that requires a live
    // `tools::approval::PENDING_APPROVALS` entry (which would need a
    // `pub(crate)` test hook into a private struct). End-to-end routing
    // coverage is registered as a follow-up in
    // `docs/plans/review-followups.md`.

    #[tokio::test]
    async fn text_pending_list_pop_is_lifo_for_bare_verb() {
        let key = ("acct-lifo".to_string(), "chat-lifo".to_string());
        {
            let mut pending = get_text_pending().lock().await;
            pending
                .entry(key.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "older-id-aaa".to_string(),
                });
            pending
                .entry(key.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "newer-id-bbb".to_string(),
                });
        }
        // Bare "yes" parses with no suffix → dispatcher uses `list.pop()`.
        let parsed = parse_approval_reply("yes").unwrap();
        assert!(parsed.id_suffix.is_none());

        let popped = {
            let mut pending = get_text_pending().lock().await;
            let list = pending.get_mut(&key).unwrap();
            let entry = list.pop().unwrap();
            if list.is_empty() {
                pending.remove(&key);
            }
            entry
        };
        assert_eq!(popped.request_id, "newer-id-bbb");

        let mut pending = get_text_pending().lock().await;
        pending.remove(&key);
    }

    #[tokio::test]
    async fn text_pending_id_tag_position_match_routes_to_non_top() {
        let key = ("acct-suffix".to_string(), "chat-suffix".to_string());
        {
            let mut pending = get_text_pending().lock().await;
            pending
                .entry(key.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "aaaaaa-older".to_string(),
                });
            pending
                .entry(key.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "bbbbbb-newer".to_string(),
                });
        }

        let parsed = parse_approval_reply("yes#aaaaaa").unwrap();
        assert_eq!(parsed.id_suffix, Some("aaaaaa"));

        // Dispatcher path on `#tag` reply: `iter().position(|e|
        // id_tag(&e.request_id) == target).map(|i| list.remove(i))`.
        let popped = {
            let mut pending = get_text_pending().lock().await;
            let list = pending.get_mut(&key).unwrap();
            let idx = list
                .iter()
                .position(|e| id_tag(&e.request_id) == "aaaaaa")
                .expect("targeted entry exists");
            let entry = list.remove(idx);
            if list.is_empty() {
                pending.remove(&key);
            }
            entry
        };
        assert_eq!(popped.request_id, "aaaaaa-older");
        let pending = get_text_pending().lock().await;
        let remaining = pending.get(&key).expect("newer entry still queued");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].request_id, "bbbbbb-newer");
        drop(pending);

        let mut pending = get_text_pending().lock().await;
        pending.remove(&key);
    }

    #[tokio::test]
    async fn drop_pending_by_request_id_clears_across_chats() {
        let key_a = ("acct-drop".to_string(), "chat-a".to_string());
        let key_b = ("acct-drop".to_string(), "chat-b".to_string());
        {
            let mut pending = get_text_pending().lock().await;
            pending
                .entry(key_a.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "shared-id-xyz".to_string(),
                });
            pending
                .entry(key_b.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "shared-id-xyz".to_string(),
                });
            pending
                .entry(key_b.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "unrelated-id-pdq".to_string(),
                });
        }

        drop_pending_by_request_id("shared-id-xyz").await;

        let pending = get_text_pending().lock().await;
        assert!(
            pending.get(&key_a).is_none(),
            "chat A entry should be cleared and the now-empty list removed",
        );
        let remaining_b = pending
            .get(&key_b)
            .expect("chat B still has unrelated entry");
        assert_eq!(remaining_b.len(), 1);
        assert_eq!(remaining_b[0].request_id, "unrelated-id-pdq");
        drop(pending);

        let mut pending = get_text_pending().lock().await;
        pending.remove(&key_b);
    }

    #[test]
    fn id_tag_clamps_to_six_chars_or_shorter() {
        assert_eq!(id_tag("abcdef123456"), "abcdef");
        assert_eq!(id_tag("ab"), "ab");
        assert_eq!(id_tag(""), "");
    }
}
