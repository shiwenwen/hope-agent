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
    submit_approval_response, ApprovalReasonKind, ApprovalReasonPayload, ApprovalResolutionSource,
    ApprovalResponse,
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
    forbids_allow_always: bool,
}

/// Registry of pending text-reply approvals, keyed by (account_id, chat_id).
/// Only used for channels that don't support buttons.
static TEXT_PENDING: OnceLock<Mutex<HashMap<(String, String), Vec<PendingTextApproval>>>> =
    OnceLock::new();

fn get_text_pending() -> &'static Mutex<HashMap<(String, String), Vec<PendingTextApproval>>> {
    TEXT_PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Throttle for the "you have N pending approvals" hint — one nudge per
/// (account, chat) per the configured interval (see
/// `permission.imApprovalHintThrottleSecs`, default 60s). Backed by
/// [`TtlCache`] so stale entries auto-expire (bounded memory across
/// long-lived IM deployments). Capacity 1024 is generous for any
/// plausible per-process chat count.
static HINT_THROTTLE_CACHE: OnceLock<TtlCache<(String, String), ()>> = OnceLock::new();

fn get_hint_throttle() -> &'static TtlCache<(String, String), ()> {
    HINT_THROTTLE_CACHE.get_or_init(|| TtlCache::new(1024))
}

fn hint_throttle_duration() -> Duration {
    let secs = crate::config::cached_config()
        .permission
        .im_approval_hint_throttle_secs;
    Duration::from_secs(secs)
}

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

/// Drop all pending text-reply approval state for a whole session. Called by
/// the session cleanup watcher on delete / purge: resolves the session →
/// (account, chat) IM conversation and clears that chat's `TEXT_PENDING` stack,
/// so a deleted session leaves no stale IM approval entries that could hijack a
/// later reply (SURFACE-2 / INCOG-4). No-op when the session has no attached IM
/// conversation.
pub async fn drop_pending_for_session(session_id: &str) {
    let Some(channel_db) = crate::globals::get_channel_db() else {
        return;
    };
    let conv = match channel_db.get_conversation_by_session(session_id) {
        Ok(Some(c)) => c,
        Ok(None) => return,
        Err(e) => {
            app_warn!(
                "channel",
                "approval",
                "drop_pending_for_session lookup failed for {}: {}",
                session_id,
                e
            );
            return;
        }
    };
    let key = (conv.account_id, conv.chat_id);
    get_text_pending().lock().await.remove(&key);
}

/// Drop all pending text-reply approval state for a specific (account, chat).
/// Backstop for the IM eviction watcher (G5 / SURFACE-4): after denying each
/// pending approval tool-side, clear the chat's `TEXT_PENDING` stack so a stale
/// text entry can't hijack a later reply in the taken-over chat. Takes the chat
/// coordinates directly (the evicted attach row is already gone from the DB, so
/// `drop_pending_for_session` can't resolve it).
pub async fn drop_pending_for_chat(account_id: &str, chat_id: &str) {
    let key = (account_id.to_string(), chat_id.to_string());
    get_text_pending().lock().await.remove(&key);
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
pub(crate) fn build_approval_buttons(
    request_id: &str,
    reason: Option<&ApprovalReasonPayload>,
) -> Vec<Vec<InlineButton>> {
    let mut row = vec![InlineButton {
        text: "✅ Allow Once".to_string(),
        callback_data: Some(format!("{}{}:allow_once", APPROVAL_PREFIX, request_id)),
        url: None,
    }];
    if !reason_forbids_allow_always(reason) {
        row.push(InlineButton {
            text: "🔓 Always Allow".to_string(),
            callback_data: Some(format!("{}{}:allow_always", APPROVAL_PREFIX, request_id)),
            url: None,
        });
    }
    row.push(InlineButton {
        text: "❌ Deny".to_string(),
        callback_data: Some(format!("{}{}:deny", APPROVAL_PREFIX, request_id)),
        url: None,
    });
    vec![row]
}

fn reason_forbids_allow_always(reason: Option<&ApprovalReasonPayload>) -> bool {
    matches!(
        reason.map(|r| r.kind),
        Some(
            ApprovalReasonKind::DangerousCommand
                | ApprovalReasonKind::ProtectedPath
                | ApprovalReasonKind::MacControlDangerousAction
                | ApprovalReasonKind::PlanModeAsk
        )
    )
}

/// Render the approval reason as a one-line suffix for IM prompts.
///
/// Protected path details are intentionally redacted: IM approvals can happen
/// in shared chats, and echoing a configured path such as an SSH key location
/// would leak more than the command preview itself.
fn reason_line(reason: Option<&ApprovalReasonPayload>) -> String {
    let Some(r) = reason else {
        return String::new();
    };
    let label = match r.kind {
        ApprovalReasonKind::EditTool => "✏ Edit Tool",
        ApprovalReasonKind::EditCommand => "✏ Edit Command",
        ApprovalReasonKind::DangerousCommand => "⚠ Dangerous Command",
        ApprovalReasonKind::ProtectedPath => "🛡 Protected Path",
        ApprovalReasonKind::AgentCustomList => "⚙ Agent Policy",
        ApprovalReasonKind::SmartJudge => "💭 Smart Judge",
        ApprovalReasonKind::BrowserEvaluate => "🌐 Browser JS",
        ApprovalReasonKind::MacControlAction => "🖥 Mac Control",
        ApprovalReasonKind::MacControlDangerousAction => "⚠ Mac Control",
        ApprovalReasonKind::PlanModeAsk => "🧭 Plan Mode",
    };
    let detail = match r.kind {
        ApprovalReasonKind::EditTool => Some("tool can modify files".to_string()),
        ApprovalReasonKind::EditCommand => prefixed_detail("matched edit-command rule", &r.detail),
        ApprovalReasonKind::DangerousCommand => {
            prefixed_detail("matched dangerous-command rule", &r.detail)
        }
        ApprovalReasonKind::ProtectedPath => {
            Some("matched a configured protected path".to_string())
        }
        ApprovalReasonKind::AgentCustomList => {
            Some("agent policy requires approval for this tool".to_string())
        }
        ApprovalReasonKind::SmartJudge => snippet_detail(&r.detail)
            .map(ToOwned::to_owned)
            .or_else(|| Some("no rationale returned; asking for approval".to_string())),
        ApprovalReasonKind::BrowserEvaluate => prefixed_detail("script", &r.detail),
        ApprovalReasonKind::MacControlAction => prefixed_detail("action", &r.detail),
        ApprovalReasonKind::MacControlDangerousAction => {
            prefixed_detail("potentially dangerous action", &r.detail)
        }
        ApprovalReasonKind::PlanModeAsk => {
            Some("plan mode requires asking before this tool".to_string())
        }
    };

    match detail {
        Some(detail) => format!("\n{label}: {detail}"),
        None => format!("\n{label}"),
    }
}

fn snippet_detail(detail: &Option<String>) -> Option<&str> {
    let trimmed = detail.as_deref()?.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(crate::truncate_utf8(trimmed, 280))
}

fn prefixed_detail(prefix: &str, detail: &Option<String>) -> Option<String> {
    let snippet = snippet_detail(detail)?;
    Some(format!("{prefix}: {snippet}"))
}

/// Format the approval prompt text (plain text, no HTML — works across all channels).
fn format_approval_text(command: &str, reason: Option<&ApprovalReasonPayload>) -> String {
    let preview = crate::truncate_utf8(command, 500);
    format!(
        "🔐 Tool approval required\n\n{}{}",
        preview,
        reason_line(reason)
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
///
/// `timeout_secs` comes from `permission.approval_timeout_secs` so the
/// deadline shown to the user matches the actual timeout. `0` is
/// rendered as "no time limit" — matches the tool-side behaviour where
/// `0` makes the approval wait forever.
fn format_text_approval(
    command: &str,
    reason: Option<&ApprovalReasonPayload>,
    request_id: &str,
    stack_depth: usize,
    timeout_secs: u64,
) -> String {
    let preview = crate::truncate_utf8(command, 500);
    let tag = id_tag(request_id);
    let stack_hint = if stack_depth > 1 {
        format!("\n\n({stack_depth} pending — append `#{tag}` to target this one specifically)")
    } else {
        String::new()
    };
    let reply_header = timeout_reply_header(timeout_secs);
    let allow_always_forbidden = reason_forbids_allow_always(reason);
    let always_line = if allow_always_forbidden {
        ""
    } else {
        "\n  2 / always   — Always allow"
    };
    let zh_hint = if allow_always_forbidden {
        "(中文也可: 同意 / 拒绝)"
    } else {
        "(中文也可: 同意 / 总是 / 拒绝)"
    };
    format!(
        "🔐 Tool approval required #{tag}:\n{preview}{smart}\n\n{reply_header}\n  1 / yes / ok — Allow once{always_line}\n  3 / no / deny — Deny\n{zh_hint}{stack_hint}",
        smart = reason_line(reason)
    )
}

/// Render the "reply within X" header line from a timeout in seconds.
/// `0` → no deadline; whole minutes are formatted as "X min", anything
/// else stays in seconds so weird values like 90 don't get rounded.
fn timeout_reply_header(timeout_secs: u64) -> String {
    if timeout_secs == 0 {
        "Reply (no time limit):".to_string()
    } else if timeout_secs % 60 == 0 {
        let mins = timeout_secs / 60;
        format!("Reply within {mins} min:")
    } else {
        format!("Reply within {timeout_secs}s:")
    }
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
    // so this is both correct and cheaper than `to_lowercase`. AllowAlways
    // is checked first so `"yes always"` doesn't get eaten by the
    // `"yes"` arm in AllowOnce.
    let lower = verb_part.to_ascii_lowercase();
    let response = if ALLOW_ALWAYS_ALIASES.contains(&lower.as_str()) {
        ApprovalResponse::AllowAlways
    } else if ALLOW_ONCE_ALIASES.contains(&lower.as_str()) {
        ApprovalResponse::AllowOnce
    } else if DENY_ALIASES.contains(&lower.as_str()) {
        ApprovalResponse::Deny
    } else {
        return None;
    };
    Some(ParsedReply {
        response,
        id_suffix,
    })
}

/// `AllowAlways` aliases. Matched **before** [`ALLOW_ONCE_ALIASES`] so
/// `"yes always"` doesn't get eaten by the AllowOnce `"yes"` entry.
/// Adding a language (jp / ko / es / …) is one line per array.
const ALLOW_ALWAYS_ALIASES: &[&str] = &[
    "2",
    "a",
    "always",
    "yes always",
    "yesalways",
    "总是",
    "总是允许",
    "永远",
    "始终",
];

const ALLOW_ONCE_ALIASES: &[&str] = &[
    "1", "y", "yes", "ok", "okay", "allow", "approve", "好", "好的", "同意", "允许", "可以", "行",
];

const DENY_ALIASES: &[&str] = &[
    "3", "n", "no", "deny", "block", "stop", "cancel", "不", "不行", "拒绝", "否", "取消",
];

// ── Shared callback handler (eliminates boilerplate in channel plugins) ──

pub fn spawn_callback_handler_with_source(
    data: &str,
    source: &'static str,
    callback_source: Option<super::ask_user::InteractiveCallbackSource>,
) {
    let data = data.to_string();
    tokio::spawn(async move {
        match handle_approval_callback_with_source(&data, callback_source, source).await {
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
                // G4 (SURFACE-2): an approval resolved on ANY surface (GUI / HTTP /
                // IM button / timeout / session-delete / eviction) must clear this
                // chat's stale text-reply entry, else it lingers in TEXT_PENDING
                // and a later message gets hijacked as an answer to a dead prompt.
                "approval:resolved" => {
                    if let Some(request_id) = event
                        .payload
                        .get("requestId")
                        .and_then(|v| v.as_str())
                    {
                        drop_pending_by_request_id(request_id).await;
                    }
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
                    buttons: build_approval_buttons(&request.request_id, request.reason.as_ref()),
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
                        forbids_allow_always: reason_forbids_allow_always(request.reason.as_ref()),
                    });
                    list.len()
                };

                ReplyPayload {
                    text: Some(format_text_approval(
                        &request.command,
                        request.reason.as_ref(),
                        &request.request_id,
                        stack_depth,
                        crate::tools::approval::approval_timeout_secs(),
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
    /// What the tool path did after the timeout. Determines whether the
    /// IM notification says "denied" (the tool call was blocked) or
    /// "continued anyway" (the tool ran with no human approval per
    /// `permission.approval_timeout_action=proceed`). Optional only for
    /// forward-compat with payloads emitted before the field existed;
    /// missing → assume default `Deny`.
    #[serde(default)]
    timeout_action: crate::config::ApprovalTimeoutAction,
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
    let body = match event.timeout_action {
        crate::config::ApprovalTimeoutAction::Deny => format!(
            "⏱ Tool approval #{tag} timed out after {timeout_secs}s. The tool call has been denied — ask me again if you still want it to run."
        ),
        // `proceed` means the tool path didn't block: it ran the tool
        // anyway. Tell the user clearly so they don't assume the action
        // was cancelled — side effects already happened.
        crate::config::ApprovalTimeoutAction::Proceed => format!(
            "⏱ Tool approval #{tag} timed out after {timeout_secs}s. The tool call continued anyway (per `permission.approval_timeout_action=proceed`) — any side effects have already happened."
        ),
    };
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
    // Snapshot the available tags before popping so we can build a
    // helpful "did you mean" reply when the suffix doesn't match.
    enum TextReplySelection {
        Popped(PendingTextApproval),
        Missing { available_tags: Vec<String> },
        AlwaysUnavailable { tag: String },
    }

    let selection = {
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
        let maybe_idx = match parsed.id_suffix {
            Some(target) => list
                .iter()
                .position(|entry| id_tag(&entry.request_id) == target)
                .map(Some)
                .unwrap_or(None),
            None => Some(list.len() - 1),
        };
        match maybe_idx {
            Some(idx)
                if parsed.response == ApprovalResponse::AllowAlways
                    && list[idx].forbids_allow_always =>
            {
                TextReplySelection::AlwaysUnavailable {
                    tag: id_tag(&list[idx].request_id).to_string(),
                }
            }
            Some(idx) => {
                let popped = list.remove(idx);
                if list.is_empty() {
                    pending.remove(&key);
                }
                TextReplySelection::Popped(popped)
            }
            None => {
                let available_tags: Vec<String> = list
                    .iter()
                    .map(|entry| id_tag(&entry.request_id).to_string())
                    .collect();
                TextReplySelection::Missing { available_tags }
            }
        }
    };

    let entry = match selection {
        TextReplySelection::Popped(entry) => entry,
        TextReplySelection::AlwaysUnavailable { tag } => {
            send_allow_always_unavailable_notice(msg, &tag).await;
            return true;
        }
        TextReplySelection::Missing { available_tags } => {
            // Suffix typo: the user clearly tried to reply to an approval
            // (verb parsed, `#<tag>` provided) but the tag doesn't match any
            // pending entry. Consume the message and tell them which tags are
            // valid — falling through to a fresh chat turn would silently
            // route the typo to the LLM and leave the approval pending.
            if let Some(target) = parsed.id_suffix {
                send_suffix_mismatch_notice(msg, target, &available_tags).await;
                return true;
            }
            return false;
        }
    };
    let request_id = entry.request_id;

    // G3 (SURFACE-3): mirror the button path's session<->chat check. The
    // TEXT_PENDING entry is keyed by (account, chat), but the pending approval's
    // session may have been re-attached to a DIFFERENT chat since the prompt was
    // sent (1:1 handover/takeover). Verify the replying conversation still owns
    // the approval's session before submitting; on mismatch, notify + consume
    // (don't leak the reply to the LLM, don't resolve from the wrong chat).
    match crate::tools::approval::pending_approval_session_id(&request_id).await {
        Ok(Some(session_id)) => {
            let reply_source = super::ask_user::InteractiveCallbackSource::new(
                msg.channel_id.clone(),
                msg.account_id.clone(),
                msg.chat_id.clone(),
                msg.thread_id.as_deref(),
            );
            if let Err(e) = super::ask_user::validate_callback_source_for_session(
                &session_id,
                Some(&reply_source),
                "text_reply",
            ) {
                app_warn!(
                    "channel",
                    "approval",
                    "Text approval reply source mismatch for {}: {}",
                    request_id,
                    e
                );
                send_source_mismatch_notice(msg, &request_id).await;
                return true;
            }
        }
        // No session id recorded — can't validate. Fall through to submit;
        // submit_approval_response itself returns NotPending if it's already gone.
        Ok(None) => {}
        Err(e) => {
            app_warn!(
                "channel",
                "approval",
                "Text approval reply session lookup failed for {}: {}",
                request_id,
                e
            );
            send_source_mismatch_notice(msg, &request_id).await;
            return true;
        }
    }

    match submit_approval_response(&request_id, parsed.response, ApprovalResolutionSource::Im).await
    {
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

/// Tell the user their `#<tag>` suffix didn't match any pending approval.
/// Lists the tags that ARE pending so the typo is fixable in one message.
async fn send_suffix_mismatch_notice(
    msg: &crate::channel::types::MsgContext,
    target: &str,
    available_tags: &[String],
) {
    let store = crate::config::cached_config();
    let Some(account_config) = store.channels.find_account(&msg.account_id).cloned() else {
        return;
    };
    let body = if available_tags.is_empty() {
        // Race: pending was popped between parse and our reply. Don't
        // surface a misleading "available tags: <none>" string.
        format!("ℹ️ Tag `#{target}` doesn't match any pending approval (it may have just been answered or timed out).")
    } else {
        let tag_list = available_tags
            .iter()
            .map(|t| format!("`#{t}`"))
            .collect::<Vec<_>>()
            .join(" / ");
        format!(
            "ℹ️ Tag `#{target}` doesn't match any pending approval. Currently pending: {tag_list}. Reply e.g. `yes#{first}` or `no#{first}`.",
            first = available_tags[0]
        )
    };
    let registry = match crate::globals::get_channel_registry() {
        Some(r) => r,
        None => return,
    };
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
            "Failed to send suffix-mismatch notice: {}",
            e
        );
    }
}

/// G3 (SURFACE-3): tell the user their text reply came from a different
/// conversation than the one the approval was sent to (the approval's session
/// has since been attached elsewhere). The approval is left pending; the user
/// must answer it from the chat where the prompt currently lives.
async fn send_source_mismatch_notice(msg: &crate::channel::types::MsgContext, request_id: &str) {
    let store = crate::config::cached_config();
    let Some(account_config) = store.channels.find_account(&msg.account_id).cloned() else {
        return;
    };
    let registry = match crate::globals::get_channel_registry() {
        Some(r) => r,
        None => return,
    };
    let tag = id_tag(request_id);
    let payload = ReplyPayload {
        text: Some(format!(
            "ℹ️ Approval `#{tag}` belongs to a different conversation now and can't be answered from here. Reply in the chat where the approval prompt currently appears."
        )),
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
            "Failed to send source-mismatch notice: {}",
            e
        );
    }
}

/// Tell text-reply users that this strict approval cannot be persisted.
/// Leave the approval pending so they can still reply `1` / `yes` or deny it.
async fn send_allow_always_unavailable_notice(msg: &crate::channel::types::MsgContext, tag: &str) {
    let store = crate::config::cached_config();
    let Some(account_config) = store.channels.find_account(&msg.account_id).cloned() else {
        return;
    };
    let registry = match crate::globals::get_channel_registry() {
        Some(r) => r,
        None => return,
    };
    let payload = ReplyPayload {
        text: Some(format!(
            "ℹ️ Approval `#{tag}` requires per-call confirmation. Reply `1` / `yes` to allow once, or `3` / `no` to deny."
        )),
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
            "Failed to send AllowAlways-unavailable notice: {}",
            e
        );
    }
}

/// Best-effort nudge for users whose chat has pending text-mode approvals
/// but who sent something that isn't a reply (e.g. a fresh question while
/// the approval prompt is still up). Sends one line per (account, chat)
/// per [`hint_throttle_duration`] (configurable), not on every non-
/// matching message.
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
    if throttle.get(&key, hint_throttle_duration()).is_some() {
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

pub async fn handle_approval_callback_with_source(
    callback_data: &str,
    callback_source: Option<super::ask_user::InteractiveCallbackSource>,
    source: &'static str,
) -> anyhow::Result<&'static str> {
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

    // G2 (MISC-11): an approval callback MUST carry a verifiable source so we can
    // confirm it came from the chat that received the prompt — otherwise a click
    // from a different conversation could resolve someone else's approval.
    // Fail-closed: look up the session and validate ALWAYS; a missing source
    // (`None`) can't be validated, so refuse. Safe for approvals — we just sent
    // the prompt message, so a real button click always carries it (Telegram's
    // no-message-callback edge only affects >48h-old / inline buttons, never a
    // live 5-min approval prompt). The shared `validate_callback_source_for_session`
    // keeps its permissive `None → Ok` for the *ask_user* path (Telegram
    // no-message Q&A answers are lower-risk and out of MISC-11 scope); approvals
    // gate here instead so that change can't regress ask_user.
    let session_id = crate::tools::approval::pending_approval_session_id(request_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Pending approval {} has no session id", request_id))?;
    let Some(source_ref) = callback_source.as_ref() else {
        return Err(anyhow::anyhow!(
            "Approval callback from {} has no source to validate against session {}; refusing (MISC-11 fail-closed)",
            source,
            session_id
        ));
    };
    super::ask_user::validate_callback_source_for_session(&session_id, Some(source_ref), source)?;

    submit_approval_response(request_id, response, ApprovalResolutionSource::Im).await?;
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
        reason(ApprovalReasonKind::SmartJudge, detail)
    }

    fn reason(kind: ApprovalReasonKind, detail: Option<&str>) -> ApprovalReasonPayload {
        ApprovalReasonPayload {
            kind,
            detail: detail.map(|s| s.to_string()),
        }
    }

    fn other_reason() -> ApprovalReasonPayload {
        reason(ApprovalReasonKind::DangerousCommand, Some("rm -rf"))
    }

    #[test]
    fn reason_line_renders_smart_judge() {
        assert_eq!(reason_line(None), "");
        assert_eq!(
            reason_line(Some(&smart(None))),
            "\n💭 Smart Judge: no rationale returned; asking for approval"
        );
        assert_eq!(
            reason_line(Some(&smart(Some("   ")))),
            "\n💭 Smart Judge: no rationale returned; asking for approval"
        );
        let line = reason_line(Some(&smart(Some("looks risky"))));
        assert_eq!(line, "\n💭 Smart Judge: looks risky");
    }

    #[test]
    fn reason_line_renders_all_known_reason_kinds() {
        let cases = [
            (
                ApprovalReasonKind::EditTool,
                None,
                "\n✏ Edit Tool: tool can modify files",
            ),
            (
                ApprovalReasonKind::EditCommand,
                Some("apply_patch"),
                "\n✏ Edit Command: matched edit-command rule: apply_patch",
            ),
            (
                ApprovalReasonKind::DangerousCommand,
                Some("rm -rf"),
                "\n⚠ Dangerous Command: matched dangerous-command rule: rm -rf",
            ),
            (
                ApprovalReasonKind::AgentCustomList,
                None,
                "\n⚙ Agent Policy: agent policy requires approval for this tool",
            ),
            (
                ApprovalReasonKind::BrowserEvaluate,
                Some("document.title"),
                "\n🌐 Browser JS: script: document.title",
            ),
            (
                ApprovalReasonKind::MacControlAction,
                Some("click"),
                "\n🖥 Mac Control: action: click",
            ),
            (
                ApprovalReasonKind::MacControlDangerousAction,
                Some("delete file"),
                "\n⚠ Mac Control: potentially dangerous action: delete file",
            ),
            (
                ApprovalReasonKind::PlanModeAsk,
                None,
                "\n🧭 Plan Mode: plan mode requires asking before this tool",
            ),
        ];

        for (kind, detail, expected) in cases {
            assert_eq!(
                reason_line(Some(&reason(kind, detail))),
                expected,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn reason_line_redacts_protected_path_detail() {
        let line = reason_line(Some(&reason(
            ApprovalReasonKind::ProtectedPath,
            Some("/Users/alice/.ssh/id_rsa"),
        )));
        assert_eq!(
            line,
            "\n🛡 Protected Path: matched a configured protected path"
        );
        assert!(!line.contains(".ssh"));
        assert!(!line.contains("id_rsa"));
    }

    #[test]
    fn reason_line_truncates_long_detail() {
        let long = "x".repeat(1000);
        let line = reason_line(Some(&smart(Some(&long))));
        assert!(line.starts_with("\n💭 Smart Judge: "));
        assert!(line.len() <= 320, "got len {}", line.len());
    }

    #[test]
    fn format_approval_text_includes_reason_line_for_smart_judge() {
        let txt = format_approval_text("exec ls", Some(&smart(Some("trusted dir"))));
        assert!(txt.starts_with("🔐 Tool approval required\n\nexec ls"));
        assert!(txt.contains("💭 Smart Judge: trusted dir"));
    }

    #[test]
    fn format_approval_text_omits_line_when_no_reason() {
        assert!(!format_approval_text("exec ls", None).contains("Smart Judge"));
        assert!(
            format_approval_text("exec ls", Some(&other_reason())).contains("Dangerous Command")
        );
    }

    #[test]
    fn format_text_approval_keeps_numeric_reply_block() {
        let txt = format_text_approval(
            "exec ls",
            Some(&smart(Some("ok per project rules"))),
            "abc123def456",
            1,
            300,
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
    fn format_text_approval_hides_always_for_strict_reason() {
        let txt = format_text_approval(
            "exec rm -rf /",
            Some(&ApprovalReasonPayload {
                kind: ApprovalReasonKind::DangerousCommand,
                detail: Some("rm -rf".to_string()),
            }),
            "abc123def456",
            1,
            300,
        );
        assert!(txt.contains("1 / yes / ok"));
        assert!(!txt.contains("2 / always"));
        assert!(!txt.contains("总是"));
        assert!(txt.contains("3 / no / deny"));
    }

    #[test]
    fn format_text_approval_renders_stack_hint_when_multiple_pending() {
        let single = format_text_approval("exec ls", None, "abcdef123456", 1, 300);
        assert!(!single.contains("pending"));

        let multi = format_text_approval("exec ls", None, "abcdef123456", 3, 300);
        assert!(multi.contains("3 pending"));
        assert!(multi.contains("#abcdef"));
    }

    #[test]
    fn format_text_approval_renders_configured_timeout() {
        // Default 5-minute timeout reads as "Reply within 5 min".
        let default = format_text_approval("exec ls", None, "abcdef123456", 1, 300);
        assert!(default.contains("Reply within 5 min:"));

        // Custom whole-minute timeout follows the same shape.
        let two_min = format_text_approval("exec ls", None, "abcdef123456", 1, 120);
        assert!(two_min.contains("Reply within 2 min:"));

        // Non-whole-minute timeout stays in seconds — no rounding.
        let ninety = format_text_approval("exec ls", None, "abcdef123456", 1, 90);
        assert!(ninety.contains("Reply within 90s:"));

        // `0` = no time limit; the deadline phrase changes so the user
        // doesn't assume a 5-min cutoff that doesn't exist.
        let unlimited = format_text_approval("exec ls", None, "abcdef123456", 1, 0);
        assert!(unlimited.contains("Reply (no time limit):"));
        assert!(!unlimited.contains("Reply within"));
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
    // `pub(crate)` test hook into a private struct).

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
                    forbids_allow_always: false,
                });
            pending
                .entry(key.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "newer-id-bbb".to_string(),
                    forbids_allow_always: false,
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
                    forbids_allow_always: false,
                });
            pending
                .entry(key.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "bbbbbb-newer".to_string(),
                    forbids_allow_always: false,
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
                    forbids_allow_always: false,
                });
            pending
                .entry(key_b.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "shared-id-xyz".to_string(),
                    forbids_allow_always: false,
                });
            pending
                .entry(key_b.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "unrelated-id-pdq".to_string(),
                    forbids_allow_always: false,
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
