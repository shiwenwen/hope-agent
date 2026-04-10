//! IM channel integration for the `ask_user_question` tool.
//!
//! Listens for `ask_user_request` EventBus events and routes them to the IM
//! channel the owning session belongs to. Mirrors the structure of
//! [`super::approval`]: button-capable channels get native inline buttons,
//! channels without button support fall back to a numbered text prompt that
//! users answer with replies like `1a`, `2b`, or `done` (for multi-select).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

use crate::channel::db::ChannelDB;
use crate::channel::registry::ChannelRegistry;
use crate::channel::types::{InlineButton, ReplyPayload};
use crate::plan::{self, PlanQuestionAnswer, PlanQuestionGroup};

/// Callback data prefix for ask_user buttons across all channels.
pub(crate) const ASK_USER_PREFIX: &str = "ask_user:";

// ── Pending state for in-progress IM answers ─────────────────────

/// One question's in-progress answer accumulator (button channels only need
/// selected values; multi-select and text fallbacks use the same state).
#[derive(Debug, Clone, Default)]
struct QuestionProgress {
    selected: Vec<String>,
    custom_input: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingAskUser {
    request_id: String,
    group: PlanQuestionGroup,
    progress: HashMap<String, QuestionProgress>,
}

impl PendingAskUser {
    fn new(group: PlanQuestionGroup) -> Self {
        let mut progress = HashMap::new();
        for q in &group.questions {
            progress.insert(q.question_id.clone(), QuestionProgress::default());
        }
        Self {
            request_id: group.request_id.clone(),
            group,
            progress,
        }
    }

    fn into_answers(self) -> Vec<PlanQuestionAnswer> {
        self.group
            .questions
            .iter()
            .map(|q| {
                let prog = self
                    .progress
                    .get(&q.question_id)
                    .cloned()
                    .unwrap_or_default();
                PlanQuestionAnswer {
                    question_id: q.question_id.clone(),
                    selected: prog.selected,
                    custom_input: prog.custom_input,
                }
            })
            .collect()
    }

    fn is_complete(&self) -> bool {
        self.group.questions.iter().all(|q| {
            let prog = self
                .progress
                .get(&q.question_id)
                .cloned()
                .unwrap_or_default();
            !prog.selected.is_empty() || prog.custom_input.is_some()
        })
    }
}

/// Pending button-based ask_user groups keyed by request_id.
static BUTTON_PENDING: OnceLock<Mutex<HashMap<String, PendingAskUser>>> = OnceLock::new();

fn get_button_pending() -> &'static Mutex<HashMap<String, PendingAskUser>> {
    BUTTON_PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Pending text-reply ask_user groups keyed by (account_id, chat_id) — LIFO.
static TEXT_PENDING: OnceLock<Mutex<HashMap<(String, String), Vec<PendingAskUser>>>> =
    OnceLock::new();

fn get_text_pending() -> &'static Mutex<HashMap<(String, String), Vec<PendingAskUser>>> {
    TEXT_PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Remove any in-memory pending state for the given request_id from both the
/// button and text-reply maps. Called by the tool execution path when a
/// question group is cancelled, timed out, or answered through a non-IM
/// channel, so stale entries don't accumulate.
pub async fn drop_pending_by_request_id(request_id: &str) {
    {
        let mut map = get_button_pending().lock().await;
        map.remove(request_id);
    }
    {
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
}

// ── Button / prompt rendering ─────────────────────────────────────

/// Render the prompt text for a group. Includes context and all questions with
/// their options numbered so the user can reference them either via button or
/// text reply.
fn format_prompt(group: &PlanQuestionGroup) -> String {
    let mut out = String::new();
    out.push_str("❓ Question from AI\n");
    if let Some(ctx) = &group.context {
        out.push('\n');
        out.push_str(ctx);
        out.push('\n');
    }
    for (qi, q) in group.questions.iter().enumerate() {
        out.push_str(&format!("\n{}. {}", qi + 1, q.text));
        if q.multi_select {
            out.push_str("  (multi-select)");
        }
        out.push('\n');
        for (oi, opt) in q.options.iter().enumerate() {
            let marker = option_marker(qi, oi);
            let rec = if opt.recommended { " ★" } else { "" };
            out.push_str(&format!("  {marker}. {}{rec}\n", opt.label));
            if let Some(desc) = &opt.description {
                out.push_str(&format!("     {desc}\n"));
            }
        }
    }
    out
}

/// Build a marker like "1a" / "2b" for question `qi` option `oi`.
fn option_marker(qi: usize, oi: usize) -> String {
    let letter = (b'a' + oi as u8) as char;
    format!("{}{}", qi + 1, letter)
}

/// Extra hint text sent to channels without button support.
fn text_reply_hint(group: &PlanQuestionGroup) -> String {
    let has_multi = group.questions.iter().any(|q| q.multi_select);
    if has_multi {
        "\nReply with option markers like `1a` (single-select) or `1a,1c` (multi-select). Type `done` when finished."
            .to_string()
    } else {
        "\nReply with an option marker like `1a`, `2b`, or type free text to provide a custom answer.".to_string()
    }
}

/// Build inline button rows for button-capable channels.
/// Each question's options form one row; multi-select questions get a
/// trailing "Done" button row.
fn build_buttons(group: &PlanQuestionGroup) -> Vec<Vec<InlineButton>> {
    let mut rows: Vec<Vec<InlineButton>> = Vec::new();
    for (qi, q) in group.questions.iter().enumerate() {
        let mut row = Vec::new();
        for (oi, opt) in q.options.iter().enumerate() {
            let marker = option_marker(qi, oi);
            let text = if opt.recommended {
                format!("★ {}", opt.label)
            } else {
                opt.label.clone()
            };
            row.push(InlineButton {
                text: format!("[{marker}] {text}"),
                callback_data: Some(format!(
                    "{}{}:select:{}:{}",
                    ASK_USER_PREFIX, group.request_id, q.question_id, opt.value
                )),
                url: None,
            });
            // Split into chunks of 3 to keep Telegram rows short.
            if row.len() == 3 {
                rows.push(std::mem::take(&mut row));
            }
        }
        if !row.is_empty() {
            rows.push(std::mem::take(&mut row));
        }
        if q.multi_select {
            rows.push(vec![InlineButton {
                text: format!("✅ Done with Q{}", qi + 1),
                callback_data: Some(format!(
                    "{}{}:done:{}",
                    ASK_USER_PREFIX, group.request_id, q.question_id
                )),
                url: None,
            }]);
        }
    }
    // Top-level cancel
    rows.push(vec![InlineButton {
        text: "❌ Cancel".to_string(),
        callback_data: Some(format!("{}{}:cancel", ASK_USER_PREFIX, group.request_id)),
        url: None,
    }]);
    rows
}

// ── EventBus listener ─────────────────────────────────────────────

/// Spawn a background task that forwards `ask_user_request` events to
/// whichever IM channel the owning session belongs to. Idempotent — callers
/// should only invoke once at startup.
pub fn spawn_channel_ask_user_listener(
    channel_db: Arc<ChannelDB>,
    registry: Arc<ChannelRegistry>,
) {
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
                        "ask_user",
                        "ask_user listener lagged {} events",
                        n
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };

            if event.name != plan::EVENT_ASK_USER_REQUEST {
                continue;
            }

            let group: PlanQuestionGroup = match serde_json::from_value(event.payload.clone()) {
                Ok(g) => g,
                Err(e) => {
                    app_warn!(
                        "channel",
                        "ask_user",
                        "Failed to parse ask_user group: {}",
                        e
                    );
                    continue;
                }
            };

            // Look up which channel conversation this session belongs to.
            let conversation = match channel_db.get_conversation_by_session(&group.session_id) {
                Ok(Some(conv)) => conv,
                Ok(None) => continue, // Not an IM session
                Err(e) => {
                    app_warn!(
                        "channel",
                        "ask_user",
                        "Failed to look up channel session {}: {}",
                        group.session_id,
                        e
                    );
                    continue;
                }
            };

            let store = crate::config::cached_config();
            let account_config = match store.channels.find_account(&conversation.account_id) {
                Some(c) => c.clone(),
                None => continue,
            };

            let channel_id: crate::channel::types::ChannelId =
                match serde_json::from_value(serde_json::Value::String(
                    conversation.channel_id.clone(),
                )) {
                    Ok(id) => id,
                    Err(_) => continue,
                };

            let supports_buttons = registry
                .get_plugin(&channel_id)
                .map(|p| p.capabilities().supports_buttons)
                .unwrap_or(false);

            let prompt_text = format_prompt(&group);

            let payload = if supports_buttons {
                // Register pending state keyed by request_id.
                {
                    let mut pending = get_button_pending().lock().await;
                    pending.insert(
                        group.request_id.clone(),
                        PendingAskUser::new(group.clone()),
                    );
                }
                ReplyPayload {
                    text: Some(prompt_text),
                    buttons: build_buttons(&group),
                    thread_id: conversation.thread_id.clone(),
                    ..ReplyPayload::text("")
                }
            } else {
                // Register for text-reply routing.
                {
                    let key = (
                        conversation.account_id.clone(),
                        conversation.chat_id.clone(),
                    );
                    let mut pending = get_text_pending().lock().await;
                    pending
                        .entry(key)
                        .or_default()
                        .push(PendingAskUser::new(group.clone()));
                }
                let text = format!("{}{}", prompt_text, text_reply_hint(&group));
                ReplyPayload {
                    text: Some(text),
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
                    "ask_user",
                    "Failed to send ask_user prompt to channel: {}",
                    e
                );
            }
        }
    });
}

// ── Text-reply handler (channels without buttons) ─────────────────

/// Try to interpret an inbound IM message as an ask_user text reply.
/// Returns `true` if the message was consumed.
///
/// Accepted reply formats:
/// - `1a`         single option for Q1
/// - `1a,1c`      multi-select for Q1
/// - `done`       finalise all answers (multi-select)
/// - `cancel`     abort the group
/// - `<text>`     free-form custom input for the first unanswered question
pub async fn try_handle_ask_user_reply(
    msg: &crate::channel::types::MsgContext,
) -> bool {
    let text = match msg.text.as_deref() {
        Some(t) => t.trim().to_string(),
        None => return false,
    };
    if text.is_empty() {
        return false;
    }

    let key = (msg.account_id.clone(), msg.chat_id.clone());
    let mut pending_map = get_text_pending().lock().await;
    let entry = match pending_map.get_mut(&key) {
        Some(v) if !v.is_empty() => v,
        _ => return false,
    };
    // Operate on the most recent group (LIFO).
    let last_idx = entry.len() - 1;
    let current = &mut entry[last_idx];

    let lowered = text.to_lowercase();
    if lowered == "cancel" {
        let request_id = current.request_id.clone();
        entry.pop();
        if entry.is_empty() {
            pending_map.remove(&key);
        }
        drop(pending_map);
        plan::cancel_pending_plan_question(&request_id).await;
        return true;
    }

    let should_finish = lowered == "done" || !current.group.questions.iter().any(|q| q.multi_select);

    // Try to parse option markers. A reply like "1a,1c" splits into markers.
    let mut parsed_any = false;
    for token in text.split(|c: char| c == ',' || c.is_whitespace()) {
        let tok = token.trim();
        if tok.is_empty() || tok.eq_ignore_ascii_case("done") || tok.eq_ignore_ascii_case("cancel")
        {
            continue;
        }
        if let Some((qi, oi)) = parse_marker(tok) {
            if qi < current.group.questions.len() {
                let q = &current.group.questions[qi];
                if oi < q.options.len() {
                    let value = q.options[oi].value.clone();
                    let prog = current
                        .progress
                        .entry(q.question_id.clone())
                        .or_default();
                    if q.multi_select {
                        if !prog.selected.contains(&value) {
                            prog.selected.push(value);
                        }
                    } else {
                        prog.selected = vec![value];
                    }
                    parsed_any = true;
                }
            }
        }
    }

    // If nothing parsed and there's exactly one question needing a custom answer,
    // treat the whole text as a custom input for the first unanswered question.
    if !parsed_any {
        if let Some(first_unanswered) = current.group.questions.iter().find(|q| {
            let prog = current
                .progress
                .get(&q.question_id)
                .cloned()
                .unwrap_or_default();
            prog.selected.is_empty() && prog.custom_input.is_none()
        }) {
            if first_unanswered.allow_custom {
                let qid = first_unanswered.question_id.clone();
                let prog = current.progress.entry(qid).or_default();
                prog.custom_input = Some(text.clone());
                parsed_any = true;
            }
        }
    }

    if !parsed_any {
        return false;
    }

    if should_finish && current.is_complete() {
        let request_id = current.request_id.clone();
        let pending = entry.pop().unwrap();
        if entry.is_empty() {
            pending_map.remove(&key);
        }
        drop(pending_map);
        let answers = pending.into_answers();
        if let Err(e) = plan::submit_plan_question_response(&request_id, answers).await {
            app_warn!(
                "channel",
                "ask_user",
                "Failed to submit ask_user answers ({}): {}",
                request_id,
                e
            );
        }
    }

    true
}

/// Parse an option marker like "1a" or "10c" → (question_index, option_index).
fn parse_marker(tok: &str) -> Option<(usize, usize)> {
    let tok = tok.trim().to_lowercase();
    if tok.len() < 2 {
        return None;
    }
    let letter = tok.chars().last().filter(|c| c.is_ascii_alphabetic())?;
    let oi = (letter as u8 - b'a') as usize;
    let qi: usize = tok[..tok.len() - 1].parse().ok()?;
    if qi == 0 {
        return None;
    }
    Some((qi - 1, oi))
}

// ── Callback handler (button-capable channels) ────────────────────

/// Check whether a callback data string belongs to an ask_user flow.
pub fn is_ask_user_callback(data: &str) -> bool {
    data.starts_with(ASK_USER_PREFIX)
}

/// Parse an `ask_user:{request_id}:select:{question_id}:{option_value}` or
/// `ask_user:{request_id}:done:{question_id}` or `ask_user:{request_id}:cancel`
/// callback and update pending state / submit when complete.
///
/// Returns a short human-readable label for UI feedback.
pub async fn handle_ask_user_callback(callback_data: &str) -> anyhow::Result<&'static str> {
    let rest = callback_data
        .strip_prefix(ASK_USER_PREFIX)
        .ok_or_else(|| anyhow::anyhow!("Not an ask_user callback"))?;

    let mut parts = rest.splitn(4, ':');
    let request_id = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("Missing request_id"))?
        .to_string();
    let action = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("Missing action"))?;

    match action {
        "cancel" => {
            get_button_pending().lock().await.remove(&request_id);
            plan::cancel_pending_plan_question(&request_id).await;
            Ok("❌ Cancelled")
        }
        "select" => {
            let question_id = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("Missing question_id"))?
                .to_string();
            let option_value = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("Missing option_value"))?
                .to_string();

            let (should_submit, pending_for_submit) = {
                let mut map = get_button_pending().lock().await;
                let Some(pending) = map.get_mut(&request_id) else {
                    return Err(anyhow::anyhow!("No pending ask_user with id {}", request_id));
                };
                let q = pending
                    .group
                    .questions
                    .iter()
                    .find(|q| q.question_id == question_id)
                    .cloned();
                if let Some(q) = q {
                    let prog = pending.progress.entry(question_id.clone()).or_default();
                    if q.multi_select {
                        if prog.selected.contains(&option_value) {
                            prog.selected.retain(|v| v != &option_value);
                        } else {
                            prog.selected.push(option_value);
                        }
                    } else {
                        prog.selected = vec![option_value];
                    }
                }
                // Single-select complete → submit; multi-select waits for "done".
                let has_multi = pending.group.questions.iter().any(|q| q.multi_select);
                if !has_multi && pending.is_complete() {
                    let p = map.remove(&request_id);
                    (true, p)
                } else {
                    (false, None)
                }
            };

            if should_submit {
                if let Some(pending) = pending_for_submit {
                    let answers = pending.into_answers();
                    plan::submit_plan_question_response(&request_id, answers).await?;
                    return Ok("✅ Answered");
                }
            }
            Ok("✓ Selected")
        }
        "done" => {
            let mut map = get_button_pending().lock().await;
            let Some(pending) = map.remove(&request_id) else {
                return Err(anyhow::anyhow!("No pending ask_user with id {}", request_id));
            };
            drop(map);
            let answers = pending.into_answers();
            plan::submit_plan_question_response(&request_id, answers).await?;
            Ok("✅ Answered")
        }
        _ => Err(anyhow::anyhow!("Unknown ask_user action: {}", action)),
    }
}

/// Spawn a background task to handle an ask_user callback and log the result.
pub fn spawn_callback_handler(data: &str, source: &'static str) {
    let data = data.to_string();
    tokio::spawn(async move {
        match handle_ask_user_callback(&data).await {
            Ok(label) => app_info!("channel", source, "ask_user: {}", label),
            Err(e) => app_warn!("channel", source, "ask_user callback failed: {}", e),
        }
    });
}

/// Unified interactive-callback dispatcher for channel plugins.
///
/// Detects whether a callback string belongs to an approval or ask_user flow
/// and spawns the corresponding handler. Returns `true` if the callback was
/// consumed (the plugin should not treat it as a regular message).
pub fn try_dispatch_interactive_callback(data: &str, source: &'static str) -> bool {
    if super::approval::is_approval_callback(data) {
        super::approval::spawn_callback_handler(data, source);
        return true;
    }
    if is_ask_user_callback(data) {
        spawn_callback_handler(data, source);
        return true;
    }
    false
}
