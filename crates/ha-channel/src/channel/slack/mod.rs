//! Slack Bot channel.
//!
//! - **Official API**: <https://api.slack.com/apis/socket-mode>,
//!   <https://api.slack.com/methods> (Web API)
//! - **SDK / Reference**: <https://github.com/slackapi/python-slack-sdk>
//! - **Protocol**: Socket Mode WebSocket（一次性 wss URL）+ Web API REST
//! - **Last reviewed**: 2026-08-05

pub mod api;
pub mod format;
pub mod inbound_media;
pub mod media;
pub mod socket;

use anyhow::Result;
use async_trait::async_trait;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use api::{
    SlackApi, SlackStreamApiError, SlackStreamApiErrorKind, SlackStreamChunk,
    SlackStreamTaskStatus, SLACK_STREAM_MARKDOWN_MAX_CHARS,
};
use ha_core::channel::traits::{chunk_text, ChannelPlugin, ChannelReplyStream};
use ha_core::channel::types::*;
use url::Url;

/// Slack Block Kit `action_id` fields are limited to 255 characters.
pub(crate) const SLACK_ACTION_ID_MAX_CHARS: usize = 255;
const SLACK_BUTTON_ACTION_PREFIX: &str = "ha_button";
const SLACK_ACTIONS_PER_BLOCK: usize = 25;
const SLACK_FINAL_BLOCKS_MAX: usize = 50;
const SLACK_SECTION_TEXT_MAX_CHARS: usize = 3_000;
const SLACK_BUTTON_TEXT_MAX_CHARS: usize = 75;
const SLACK_BUTTON_VALUE_MAX_CHARS: usize = 2_000;
const SLACK_BUTTON_URL_MAX_CHARS: usize = 3_000;

/// Running account state.
struct RunningAccount {
    api: Arc<SlackApi>,
    // Diagnostics-only — retained for future filtering of bot-authored events.
    #[allow(dead_code)]
    bot_id: String,
    #[allow(dead_code)]
    bot_name: String,
}

/// Slack channel plugin implementation (Socket Mode).
pub struct SlackPlugin {
    /// Running accounts keyed by account_id.
    accounts: Mutex<HashMap<String, RunningAccount>>,
}

impl SlackPlugin {
    pub fn new() -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
        }
    }

    /// Extract bot token (xoxb-...) from credentials JSON.
    fn extract_bot_token(credentials: &serde_json::Value) -> Result<String> {
        credentials
            .get("botToken")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'botToken' in Slack credentials"))
    }

    /// Extract app token (xapp-...) from credentials JSON.
    fn extract_app_token(credentials: &serde_json::Value) -> Result<String> {
        credentials
            .get("appToken")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'appToken' in Slack credentials"))
    }

    /// Extract optional proxy URL from settings or global config.
    fn extract_proxy(settings: &serde_json::Value) -> Option<String> {
        if let Some(proxy) = settings.get("proxy").and_then(|v| v.as_str()) {
            if !proxy.is_empty() {
                return Some(proxy.to_string());
            }
        }
        ha_core::provider::active_custom_proxy_url()
    }

    /// Get the API for a running account.
    async fn get_api(&self, account_id: &str) -> Result<Arc<SlackApi>> {
        let accounts = self.accounts.lock().await;
        accounts
            .get(account_id)
            .map(|a| a.api.clone())
            .ok_or_else(|| anyhow::anyhow!("Slack account '{}' is not running", account_id))
    }
}

fn slack_action_id(index: usize) -> String {
    format!("{SLACK_BUTTON_ACTION_PREFIX}:{index}")
}

fn slack_button_element(button: &InlineButton, index: usize) -> serde_json::Value {
    let callback = button.callback_id();
    let mut element = serde_json::json!({
        "type": "button",
        "text": {"type": "plain_text", "text": &button.text},
        "action_id": slack_action_id(index),
        "value": callback,
    });
    if let Some(url) = button.url.as_deref().filter(|url| !url.is_empty()) {
        element["url"] = serde_json::Value::String(url.to_string());
    }
    element
}

fn slack_required_action_blocks(button_rows: &[Vec<InlineButton>]) -> usize {
    button_rows
        .iter()
        .map(|row| row.len().div_ceil(SLACK_ACTIONS_PER_BLOCK))
        .sum()
}

fn validate_slack_buttons(
    button_rows: &[Vec<InlineButton>],
) -> std::result::Result<usize, ReplyStreamError> {
    let required_blocks = slack_required_action_blocks(button_rows);
    if button_rows.iter().any(Vec::is_empty) || required_blocks >= SLACK_FINAL_BLOCKS_MAX {
        return Err(ReplyStreamError::new(
            ReplyStreamErrorKind::InvalidContent,
            "Slack reply buttons require non-empty rows and must leave one block for message content",
        ));
    }

    for button in button_rows.iter().flatten() {
        let text_chars = button.text.chars().count();
        if text_chars == 0
            || text_chars > SLACK_BUTTON_TEXT_MAX_CHARS
            || button.text.chars().any(char::is_control)
        {
            return Err(ReplyStreamError::new(
                ReplyStreamErrorKind::InvalidContent,
                "Slack button text must contain 1 to 75 characters",
            ));
        }
        let callback = button.callback_id();
        if callback.is_empty()
            || callback.chars().count() > SLACK_BUTTON_VALUE_MAX_CHARS
            || callback.chars().any(char::is_control)
        {
            return Err(ReplyStreamError::new(
                ReplyStreamErrorKind::InvalidContent,
                "Slack button value exceeds the 2000-character limit",
            ));
        }
        if let Some(raw_url) = button.url.as_deref() {
            if raw_url.chars().count() > SLACK_BUTTON_URL_MAX_CHARS {
                return Err(ReplyStreamError::new(
                    ReplyStreamErrorKind::InvalidContent,
                    "Slack button URL exceeds the 3000-character limit",
                ));
            }
            let parsed = Url::parse(raw_url).map_err(|_| {
                ReplyStreamError::new(
                    ReplyStreamErrorKind::InvalidContent,
                    "Slack button URL is invalid",
                )
            })?;
            if !matches!(parsed.scheme(), "http" | "https")
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err(ReplyStreamError::new(
                    ReplyStreamErrorKind::InvalidContent,
                    "Slack button URL must be credential-free HTTP(S)",
                ));
            }
        }
    }

    debug_assert!(
        slack_action_id(button_rows.iter().map(Vec::len).sum())
            .chars()
            .count()
            <= SLACK_ACTION_ID_MAX_CHARS
    );
    Ok(required_blocks)
}

fn slack_actions_blocks(
    button_rows: &[Vec<InlineButton>],
    max_blocks: usize,
) -> Vec<serde_json::Value> {
    let mut blocks = Vec::new();
    let mut button_index = 0usize;

    for row in button_rows {
        for group in row.chunks(SLACK_ACTIONS_PER_BLOCK) {
            if group.is_empty() {
                continue;
            }
            if blocks.len() == max_blocks {
                app_warn!(
                    "channel",
                    "slack",
                    "Slack button blocks exceed the available {}-block limit; remaining buttons omitted",
                    max_blocks
                );
                return blocks;
            }
            let elements = group
                .iter()
                .map(|button| {
                    let element = slack_button_element(button, button_index);
                    button_index += 1;
                    element
                })
                .collect::<Vec<_>>();
            blocks.push(serde_json::json!({
                "type": "actions",
                "elements": elements,
            }));
        }
    }
    blocks
}

fn split_slack_section_text(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut sections = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0usize;
    for ch in text.chars() {
        if current_chars == SLACK_SECTION_TEXT_MAX_CHARS {
            sections.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        current.push(ch);
        current_chars += 1;
    }
    if !current.is_empty() {
        sections.push(current);
    }
    sections
}

fn slack_message_blocks(
    text: &str,
    button_rows: &[Vec<InlineButton>],
) -> std::result::Result<Option<Vec<serde_json::Value>>, ReplyStreamError> {
    if button_rows.iter().all(Vec::is_empty) {
        return Ok(None);
    }

    let action_blocks = validate_slack_buttons(button_rows)?;
    let section_limit = SLACK_FINAL_BLOCKS_MAX.saturating_sub(action_blocks);
    let sections = split_slack_section_text(text);
    let mut blocks = sections
        .iter()
        .take(section_limit)
        .map(|section| {
            serde_json::json!({
                "type": "section",
                "text": {"type": "mrkdwn", "text": section},
            })
        })
        .collect::<Vec<_>>();
    if sections.len() > section_limit {
        app_warn!(
            "channel",
            "slack",
            "Slack message text exceeds the available Block Kit section limit; extra sections omitted"
        );
    }
    blocks.extend(slack_actions_blocks(button_rows, action_blocks));
    Ok(Some(blocks))
}

fn slack_final_blocks(
    button_rows: &[Vec<InlineButton>],
) -> std::result::Result<Vec<serde_json::Value>, ReplyStreamError> {
    let required_blocks = validate_slack_buttons(button_rows)?;
    Ok(slack_actions_blocks(button_rows, required_blocks))
}

fn map_slack_stream_error(error: SlackStreamApiError) -> ReplyStreamError {
    let kind = match error.kind {
        SlackStreamApiErrorKind::Unsupported => ReplyStreamErrorKind::Unsupported,
        SlackStreamApiErrorKind::InvalidTarget => ReplyStreamErrorKind::InvalidTarget,
        SlackStreamApiErrorKind::InvalidContent => ReplyStreamErrorKind::InvalidContent,
        SlackStreamApiErrorKind::Rejected => ReplyStreamErrorKind::Rejected,
        SlackStreamApiErrorKind::RateLimited => ReplyStreamErrorKind::RateLimited,
        SlackStreamApiErrorKind::Transient => ReplyStreamErrorKind::Transient,
        SlackStreamApiErrorKind::Expired => ReplyStreamErrorKind::Expired,
        SlackStreamApiErrorKind::Ambiguous => ReplyStreamErrorKind::Ambiguous,
    };
    ReplyStreamError::new(kind, ha_core::logging::redact_sensitive(&error.to_string()))
}

fn ambiguous_after_stream_acceptance(error: ReplyStreamError) -> ReplyStreamError {
    ReplyStreamError::new(
        ReplyStreamErrorKind::Ambiguous,
        format!(
            "Slack accepted the reply stream, so commit is unsafe to retry: {}",
            ha_core::logging::redact_sensitive(&error.to_string())
        ),
    )
}

fn slack_task_status(status: ReplyStreamTaskStatus) -> SlackStreamTaskStatus {
    match status {
        ReplyStreamTaskStatus::Pending => SlackStreamTaskStatus::Pending,
        ReplyStreamTaskStatus::InProgress => SlackStreamTaskStatus::InProgress,
        ReplyStreamTaskStatus::Complete => SlackStreamTaskStatus::Complete,
        ReplyStreamTaskStatus::Error => SlackStreamTaskStatus::Error,
    }
}

fn slack_task_chunk(task: &ReplyStreamTask) -> SlackStreamChunk {
    // ReplyStreamTask deliberately contains no tool arguments or raw result;
    // only its bounded, user-visible projection reaches Slack.
    SlackStreamChunk::TaskUpdate {
        id: task.id.clone(),
        title: task.title.clone(),
        status: slack_task_status(task.status),
        details: task.details.clone(),
    }
}

fn initial_stream_chunks(frame: &ReplyStreamFrame) -> Vec<SlackStreamChunk> {
    let mut chunks = Vec::new();
    if !frame.markdown_snapshot.is_empty() {
        chunks.push(SlackStreamChunk::MarkdownText(
            frame.markdown_snapshot.clone(),
        ));
    }
    if let Some(title) = frame
        .plan_title
        .as_deref()
        .filter(|title| !title.is_empty())
    {
        chunks.push(SlackStreamChunk::PlanUpdate {
            title: title.to_string(),
        });
    }
    chunks.extend(frame.tasks.iter().map(slack_task_chunk));
    chunks
}

fn stream_task_state(tasks: &[ReplyStreamTask]) -> BTreeMap<String, ReplyStreamTask> {
    tasks
        .iter()
        .cloned()
        .map(|task| (task.id.clone(), task))
        .collect()
}

struct SlackReplyStream {
    api: Arc<SlackApi>,
    channel: String,
    ts: String,
    revision: u64,
    markdown_snapshot: String,
    plan_title: Option<String>,
    tasks: BTreeMap<String, ReplyStreamTask>,
}

impl SlackReplyStream {
    async fn stop_after_commit_failure(&self) -> std::result::Result<(), ReplyStreamError> {
        let terminal = [SlackStreamChunk::MarkdownText(
            "\n\n_流式回复未能完成_".to_string(),
        )];
        self.api
            .chat_stop_stream(&self.channel, &self.ts, &terminal, &[])
            .await
            .map_err(map_slack_stream_error)?;
        Ok(())
    }

    fn validate_delta<'a>(
        &self,
        frame: &'a ReplyStreamFrame,
    ) -> std::result::Result<&'a str, ReplyStreamError> {
        if frame.revision <= self.revision {
            return Err(ReplyStreamError::new(
                ReplyStreamErrorKind::InvalidContent,
                "Slack stream revision must increase monotonically",
            ));
        }
        let expected_delta = frame
            .markdown_snapshot
            .strip_prefix(&self.markdown_snapshot)
            .ok_or_else(|| {
                ReplyStreamError::new(
                    ReplyStreamErrorKind::InvalidContent,
                    "Slack append stream cannot replace an accepted Markdown prefix",
                )
            })?;
        if expected_delta != frame.markdown_delta {
            return Err(ReplyStreamError::new(
                ReplyStreamErrorKind::InvalidContent,
                "Slack stream delta does not match its Markdown snapshot",
            ));
        }
        Ok(expected_delta)
    }

    fn changed_chunks(&self, frame: &ReplyStreamFrame, delta: &str) -> Vec<SlackStreamChunk> {
        let mut chunks = Vec::new();
        if !delta.is_empty() {
            chunks.push(SlackStreamChunk::MarkdownText(delta.to_string()));
        }
        if frame.plan_title != self.plan_title {
            if let Some(title) = frame
                .plan_title
                .as_deref()
                .filter(|title| !title.is_empty())
            {
                chunks.push(SlackStreamChunk::PlanUpdate {
                    title: title.to_string(),
                });
            }
        }
        chunks.extend(frame.tasks.iter().filter_map(|task| {
            if self.tasks.get(&task.id) == Some(task) {
                None
            } else {
                Some(slack_task_chunk(task))
            }
        }));
        chunks
    }
}

#[async_trait]
impl ChannelReplyStream for SlackReplyStream {
    async fn push(
        &mut self,
        frame: &ReplyStreamFrame,
    ) -> std::result::Result<(), ReplyStreamError> {
        let delta = self.validate_delta(frame)?.to_string();
        let chunks = self.changed_chunks(frame, &delta);
        self.api
            .chat_append_stream(&self.channel, &self.ts, &chunks)
            .await
            .map_err(map_slack_stream_error)?;

        self.revision = frame.revision;
        self.markdown_snapshot = frame.markdown_snapshot.clone();
        if frame
            .plan_title
            .as_deref()
            .is_some_and(|title| !title.is_empty())
        {
            self.plan_title = frame.plan_title.clone();
        }
        for task in &frame.tasks {
            self.tasks.insert(task.id.clone(), task.clone());
        }
        Ok(())
    }

    async fn commit(
        self: Box<Self>,
        final_reply: &RichReply,
    ) -> std::result::Result<RichReplyReceipt, ReplyStreamError> {
        let suffix = match final_reply.markdown.strip_prefix(&self.markdown_snapshot) {
            Some(suffix) => suffix,
            None => {
                let invalid = ReplyStreamError::new(
                    ReplyStreamErrorKind::InvalidContent,
                    "Slack final reply cannot replace an accepted Markdown prefix",
                );
                if let Err(cleanup_error) = self.stop_after_commit_failure().await {
                    return Err(ambiguous_after_stream_acceptance(cleanup_error));
                }
                return Err(invalid);
            }
        };
        let chunks = if suffix.is_empty() {
            Vec::new()
        } else {
            vec![SlackStreamChunk::MarkdownText(suffix.to_string())]
        };
        let blocks = match slack_final_blocks(&final_reply.buttons) {
            Ok(blocks) => blocks,
            Err(invalid) => {
                if let Err(cleanup_error) = self.stop_after_commit_failure().await {
                    return Err(ambiguous_after_stream_acceptance(cleanup_error));
                }
                return Err(invalid);
            }
        };
        let stopped = match self
            .api
            .chat_stop_stream(&self.channel, &self.ts, &chunks, &blocks)
            .await
            .map_err(map_slack_stream_error)
        {
            Ok(stopped) => stopped,
            Err(error)
                if matches!(
                    error.kind,
                    ReplyStreamErrorKind::Ambiguous | ReplyStreamErrorKind::Expired
                ) =>
            {
                // A second stop could duplicate suffixes/blocks after an
                // ambiguous response, while an expired handle is no longer a
                // valid cleanup target.
                return Err(error);
            }
            Err(error) => {
                // Slack explicitly rejected the requested final payload, so
                // it had no effect. Consume the still-active stream with a
                // marker; if cleanup itself is uncertain, preserve ambiguity.
                if let Err(cleanup_error) = self.stop_after_commit_failure().await {
                    return Err(ambiguous_after_stream_acceptance(cleanup_error));
                }
                return Err(error);
            }
        };
        Ok(RichReplyReceipt::text_only(stopped.ts))
    }

    async fn fail(self: Box<Self>, error_text: &str) -> std::result::Result<(), ReplyStreamError> {
        let terminal_text = if error_text.trim().is_empty() {
            "⚠️ Reply generation failed."
        } else {
            error_text
        };
        let mut chunks = vec![SlackStreamChunk::MarkdownText(format!(
            "\n\n{terminal_text}"
        ))];
        chunks.extend(self.tasks.values().filter_map(|task| {
            matches!(
                task.status,
                ReplyStreamTaskStatus::Pending | ReplyStreamTaskStatus::InProgress
            )
            .then(|| {
                let mut failed_task = task.clone();
                failed_task.status = ReplyStreamTaskStatus::Error;
                failed_task.details = Some("生成中断".to_string());
                slack_task_chunk(&failed_task)
            })
        }));
        self.api
            .chat_stop_stream(&self.channel, &self.ts, &chunks, &[])
            .await
            .map_err(map_slack_stream_error)?;
        Ok(())
    }

    async fn abort(
        self: Box<Self>,
        reason: ReplyAbortReason,
    ) -> std::result::Result<(), ReplyStreamError> {
        // `chat.stopStream` acknowledges terminal ownership only. Slack keeps
        // the previously appended markdown visible, so callers must not treat
        // this success as proof that replaying the full logical result is safe.
        let (marker, task_details) = match reason {
            ReplyAbortReason::Cancelled => ("\n\n_已停止_", "已停止"),
            ReplyAbortReason::Failed => ("\n\n_生成中断_", "生成中断"),
            ReplyAbortReason::Detached => {
                ("\n\n_流式更新中断，完整结果见后续消息_", "流式更新中断")
            }
        };
        let mut chunks = vec![SlackStreamChunk::MarkdownText(marker.to_string())];
        chunks.extend(self.tasks.values().filter_map(|task| {
            matches!(
                task.status,
                ReplyStreamTaskStatus::Pending | ReplyStreamTaskStatus::InProgress
            )
            .then(|| {
                let mut stopped_task = task.clone();
                stopped_task.status = ReplyStreamTaskStatus::Error;
                stopped_task.details = Some(task_details.to_string());
                slack_task_chunk(&stopped_task)
            })
        }));
        self.api
            .chat_stop_stream(&self.channel, &self.ts, &chunks, &[])
            .await
            .map_err(map_slack_stream_error)?;
        Ok(())
    }
}

#[async_trait]
impl ChannelPlugin for SlackPlugin {
    fn meta(&self) -> ChannelMeta {
        ChannelMeta {
            id: ChannelId::Slack,
            display_name: "Slack".to_string(),
            description: "Slack Bot (Socket Mode)".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            chat_types: vec![ChatType::Dm, ChatType::Group, ChatType::Channel],
            supports_edit: true,
            supports_unsend: true,
            supports_reply: true,
            supports_threads: true,
            supports_typing: false,
            supports_buttons: true,
            supports_draft: false,
            supports_polls: false,
            supports_reactions: false,
            // Slack chat.postMessage 上限 4000 字符；UTF-8 字节计算下 CJK 字符
            // 占 3 bytes，留 20% 余量到 3200 字节避免 msg_too_long
            streaming_preview_max_bytes: Some(3200),
            supports_media: vec![
                MediaType::Photo,
                MediaType::Video,
                MediaType::Audio,
                MediaType::Document,
                MediaType::Sticker,
                MediaType::Voice,
                MediaType::Animation,
            ],
            supports_card_stream: false,
            native_reply: Some(NativeReplyCapabilities {
                preview_chat_types: vec![ChatType::Dm, ChatType::Group, ChatType::Channel],
                final_chat_types: vec![ChatType::Dm, ChatType::Group, ChatType::Channel],
                update_mode: ReplyStreamUpdateMode::Append,
                preview_persistence: ReplyStreamPreviewPersistence::Persistent,
                requires_reply_anchor: true,
                requires_recipient_user_id: true,
                requires_recipient_tenant_id: true,
                supports_task_updates: true,
                supports_plan_updates: true,
                supports_blocks: true,
                embedded_media_types: Vec::new(),
                max_embedded_media_items: None,
                refresh_after_secs: None,
                max_delta_chars: Some(SLACK_STREAM_MARKDOWN_MAX_CHARS as u32),
            }),
        }
    }

    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound_tx: mpsc::Sender<InboundEvent>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let bot_token = Self::extract_bot_token(&account.credentials)?;
        let app_token = Self::extract_app_token(&account.credentials)?;
        let proxy = Self::extract_proxy(&account.settings);

        let api = SlackApi::new(&bot_token, proxy.as_deref());

        // Validate token by calling auth.test
        let auth = api.auth_test().await?;
        let bot_id = auth.user_id.clone();
        let bot_name = auth.user.clone();
        app_info!(
            "channel",
            "slack",
            "Bot authenticated: {} (id={}, team={})",
            bot_name,
            bot_id,
            auth.team
        );

        let api = Arc::new(api);

        // Store running account state
        {
            let mut accounts = self.accounts.lock().await;
            accounts.insert(
                account.id.clone(),
                RunningAccount {
                    api: api.clone(),
                    bot_id: bot_id.clone(),
                    bot_name: bot_name.clone(),
                },
            );
        }

        // Spawn Socket Mode loop
        let account_id = account.id.clone();
        tokio::spawn(socket::run_socket_mode(
            api, app_token, account_id, bot_id, inbound_tx, cancel,
        ));

        Ok(())
    }

    async fn stop_account(&self, account_id: &str) -> Result<()> {
        let mut accounts = self.accounts.lock().await;
        accounts.remove(account_id);
        Ok(())
    }

    async fn materialize_pending_media(
        &self,
        account: &ChannelAccountConfig,
        msg: &mut MsgContext,
    ) -> Result<()> {
        let pending = crate::channel::inbound_media_common::take_pending_refs::<
            inbound_media::ParsedMediaRef,
        >(msg);
        if pending.is_empty() {
            return Ok(());
        }
        let api = self.get_api(&account.id).await?;
        let results = futures_util::future::join_all(
            pending
                .iter()
                .map(|p| inbound_media::materialize_inbound(&api, p, &account.id)),
        )
        .await;
        for m in results.into_iter().flatten() {
            msg.media.push(m);
        }
        Ok(())
    }

    async fn send_message(
        &self,
        account_id: &str,
        chat_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        validate_slack_buttons(&payload.buttons)?;
        let api = self.get_api(account_id).await?;
        let thread_ts = payload.thread_id.as_deref();

        if !payload.media.is_empty() {
            let files = media::build_slack_files(&payload.media).await?;
            let initial_comment =
                media::merge_initial_comment(payload.text.as_deref(), &payload.media);
            let file_id = api
                .upload_files_external(chat_id, thread_ts, initial_comment.as_deref(), files)
                .await?;
            return Ok(DeliveryResult::ok(file_id));
        }

        if let Some(ref text) = payload.text {
            if text.is_empty() {
                return Ok(DeliveryResult::ok("empty"));
            }

            let blocks = slack_message_blocks(text, &payload.buttons)?;

            let blocks_ref = blocks.as_deref();
            let ts = api
                .chat_post_message(chat_id, text, thread_ts, blocks_ref)
                .await?;
            return Ok(DeliveryResult::ok(ts));
        }

        Ok(DeliveryResult::ok("no_content"))
    }

    async fn send_typing(&self, _account_id: &str, _chat_id: &str) -> Result<()> {
        // Slack doesn't have a persistent typing API for bots.
        Ok(())
    }

    fn validate_reply_buttons(
        &self,
        buttons: &[Vec<InlineButton>],
    ) -> std::result::Result<(), ReplyStreamError> {
        validate_slack_buttons(buttons).map(|_| ())
    }

    async fn open_reply_stream(
        &self,
        target: &ReplyStreamTarget,
        first: &ReplyStreamFrame,
    ) -> std::result::Result<Box<dyn ChannelReplyStream>, ReplyStreamError> {
        if !matches!(
            target.chat_type,
            ChatType::Dm | ChatType::Group | ChatType::Channel
        ) {
            return Err(ReplyStreamError::new(
                ReplyStreamErrorKind::Unsupported,
                "Slack native streaming does not support this chat type",
            ));
        }
        let thread_ts = target
            .thread_id
            .as_deref()
            .filter(|value| !value.is_empty())
            .or_else(|| {
                target
                    .reply_to_message_id
                    .as_deref()
                    .filter(|value| !value.is_empty())
            })
            .ok_or_else(|| {
                ReplyStreamError::new(
                    ReplyStreamErrorKind::InvalidTarget,
                    "Slack native streaming requires a reply anchor",
                )
            })?
            .to_string();
        let recipient_user_id = target
            .recipient_user_id
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ReplyStreamError::new(
                    ReplyStreamErrorKind::InvalidTarget,
                    "Slack native streaming requires a recipient user",
                )
            })?
            .to_string();
        let recipient_team_id = target
            .recipient_tenant_id
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ReplyStreamError::new(
                    ReplyStreamErrorKind::InvalidTarget,
                    "Slack native streaming requires a recipient workspace",
                )
            })?
            .to_string();
        let api = self.get_api(&target.account_id).await.map_err(|error| {
            ReplyStreamError::new(
                ReplyStreamErrorKind::Transient,
                ha_core::logging::redact_sensitive(&error.to_string()),
            )
        })?;

        let chunks = initial_stream_chunks(first);
        let opened = api
            .chat_start_stream(
                &target.chat_id,
                &thread_ts,
                &recipient_user_id,
                &recipient_team_id,
                &chunks,
            )
            .await
            .map_err(map_slack_stream_error)?;
        Ok(Box::new(SlackReplyStream {
            api,
            channel: opened.channel,
            ts: opened.ts,
            revision: first.revision,
            markdown_snapshot: first.markdown_snapshot.clone(),
            plan_title: first.plan_title.clone(),
            tasks: stream_task_state(&first.tasks),
        }))
    }

    async fn send_rich_reply(
        &self,
        target: &ReplyStreamTarget,
        reply: &RichReply,
    ) -> std::result::Result<RichReplyReceipt, ReplyStreamError> {
        // Validate all final-only Block Kit content before chat.startStream.
        // After Slack accepts the stream, falling back to a legacy send could
        // duplicate the already-visible Markdown.
        validate_slack_buttons(&reply.buttons)?;
        let first = ReplyStreamFrame {
            revision: 0,
            markdown_snapshot: reply.markdown.clone(),
            markdown_delta: reply.markdown.clone(),
            phase: ReplyStreamPhase::Finalizing,
            tasks: Vec::new(),
            plan_title: None,
        };
        let stream = self.open_reply_stream(target, &first).await?;
        stream
            .commit(reply)
            .await
            .map_err(ambiguous_after_stream_acceptance)
    }

    async fn edit_message(
        &self,
        account_id: &str,
        chat_id: &str,
        message_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        let api = self.get_api(account_id).await?;

        if let Some(ref text) = payload.text {
            let blocks = slack_message_blocks(text, &payload.buttons)?;
            api.chat_update(
                chat_id,
                message_id,
                text,
                Some(blocks.as_deref().unwrap_or(&[])),
            )
            .await?;
        }

        Ok(DeliveryResult::ok(message_id.to_string()))
    }

    async fn delete_message(
        &self,
        account_id: &str,
        chat_id: &str,
        message_id: &str,
    ) -> Result<()> {
        let api = self.get_api(account_id).await?;
        api.chat_delete(chat_id, message_id).await
    }

    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth> {
        let bot_token = Self::extract_bot_token(&account.credentials)?;
        let proxy = Self::extract_proxy(&account.settings);
        let api = SlackApi::new(&bot_token, proxy.as_deref());

        match api.auth_test().await {
            Ok(auth) => Ok(ChannelHealth {
                is_running: false, // probe doesn't check running state
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(true),
                error: None,
                uptime_secs: None,
                bot_name: Some(auth.user),
            }),
            Err(e) => Ok(ChannelHealth {
                is_running: false,
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(false),
                error: Some(e.to_string()),
                uptime_secs: None,
                bot_name: None,
            }),
        }
    }

    fn check_access(&self, account: &ChannelAccountConfig, msg: &MsgContext) -> bool {
        let security = &account.security;

        match msg.chat_type {
            ChatType::Dm => match security.dm_policy {
                DmPolicy::Open => true,
                DmPolicy::Allowlist => {
                    security.user_allowlist.contains(&msg.sender_id)
                        || security.admin_ids.contains(&msg.sender_id)
                }
                DmPolicy::Pairing => {
                    // Pairing not yet implemented -- fall back to allowlist
                    security.user_allowlist.contains(&msg.sender_id)
                        || security.admin_ids.contains(&msg.sender_id)
                }
            },
            ChatType::Group | ChatType::Forum => {
                // 1. Check group_policy: disabled -> deny all
                if security.group_policy == GroupPolicy::Disabled {
                    return false;
                }

                // 2. Resolve group config: exact match -> wildcard "*" -> None
                let group_config = security.groups.get(&msg.chat_id);
                let wildcard_config = security.groups.get("*");
                let effective_group_config = group_config.or(wildcard_config);

                // 3. Allowlist mode: group must be explicitly configured (or have wildcard)
                if security.group_policy == GroupPolicy::Allowlist {
                    if security.groups.is_empty() {
                        if !security.group_allowlist.is_empty()
                            && !security.group_allowlist.contains(&msg.chat_id)
                        {
                            return false;
                        }
                    } else if effective_group_config.is_none() {
                        return false;
                    }
                }

                // Legacy group_allowlist backward compatibility (for "open" policy too)
                if !security.group_allowlist.is_empty()
                    && security.groups.is_empty()
                    && !security.group_allowlist.contains(&msg.chat_id)
                {
                    return false;
                }

                // 4. Check group-level enabled flag
                if let Some(cfg) = effective_group_config {
                    if cfg.enabled == Some(false) {
                        return false;
                    }

                    // 5. Check topic-level enabled flag (if thread_id present)
                    if let Some(ref thread_id) = msg.thread_id {
                        if let Some(topic_cfg) = cfg.topics.get(thread_id) {
                            if topic_cfg.enabled == Some(false) {
                                return false;
                            }
                            // Topic-level sender allowlist
                            if !topic_cfg.allow_from.is_empty()
                                && !topic_cfg.allow_from.contains(&msg.sender_id)
                                && !security.admin_ids.contains(&msg.sender_id)
                            {
                                return false;
                            }
                        }
                    }

                    // 6. Group-level sender allowlist
                    if !cfg.allow_from.is_empty()
                        && !cfg.allow_from.contains(&msg.sender_id)
                        && !security.admin_ids.contains(&msg.sender_id)
                    {
                        return false;
                    }
                }

                // 7. Account-level user allowlist (if set)
                if !security.user_allowlist.is_empty()
                    && !security.user_allowlist.contains(&msg.sender_id)
                    && !security.admin_ids.contains(&msg.sender_id)
                {
                    return false;
                }

                true
            }
            ChatType::Channel => {
                // Channels default to disabled unless explicitly configured
                let channel_config = security.channels.get(&msg.chat_id);
                match channel_config {
                    Some(cfg) => cfg.enabled != Some(false),
                    None => false, // Not configured -> ignore
                }
            }
        }
    }

    fn markdown_to_native(&self, markdown: &str) -> String {
        format::markdown_to_mrkdwn(markdown)
    }

    fn chunk_message(&self, text: &str) -> Vec<String> {
        // A Block Kit section is capped at 3000 chars. Keeping each legacy
        // chunk within one section makes the button preflight's reserved body
        // block exact instead of allowing the final chunk to be truncated.
        chunk_text(text, SLACK_SECTION_TEXT_MAX_CHARS)
    }

    async fn validate_credentials(&self, credentials: &serde_json::Value) -> Result<String> {
        let bot_token = Self::extract_bot_token(credentials)?;
        let api = SlackApi::new(&bot_token, None);
        let auth = api.auth_test().await?;
        Ok(auth.user)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slack_button_uses_short_action_id_and_full_callback_value() {
        let callback = format!("ask_user:req:select:q:{}", "x".repeat(300));
        let button = InlineButton {
            text: "Pick".to_string(),
            callback_data: Some(callback.clone()),
            url: None,
        };

        let element = slack_button_element(&button, 7);

        assert_eq!(element["action_id"], "ha_button:7");
        assert_eq!(element["value"], callback);
    }

    #[test]
    fn slack_button_preserves_url() {
        let button = InlineButton {
            text: "Docs".to_string(),
            callback_data: None,
            url: Some("https://example.com".to_string()),
        };
        let element = slack_button_element(&button, 0);

        assert_eq!(element["url"], "https://example.com");
        assert_eq!(element["value"], "Docs");
    }

    #[test]
    fn slack_button_blocks_group_at_25_and_reserve_one_body_block() {
        let make_buttons = |count: usize| {
            (0..count)
                .map(|index| InlineButton {
                    text: format!("Button {index}"),
                    callback_data: Some(format!("callback:{index}")),
                    url: None,
                })
                .collect::<Vec<_>>()
        };

        let grouped = slack_final_blocks(&[make_buttons(51)])
            .ok()
            .expect("51 buttons should fit in three blocks");
        assert_eq!(grouped.len(), 3);
        assert_eq!(grouped[0]["elements"].as_array().unwrap().len(), 25);
        assert_eq!(grouped[1]["elements"].as_array().unwrap().len(), 25);
        assert_eq!(grouped[2]["elements"].as_array().unwrap().len(), 1);

        let capped = slack_final_blocks(&[make_buttons(1_225)])
            .ok()
            .expect("1,225 buttons should fit in 49 action blocks");
        assert_eq!(capped.len(), SLACK_FINAL_BLOCKS_MAX - 1);
        assert!(capped
            .iter()
            .all(|block| block["elements"].as_array().unwrap().len() <= 25));

        let overflow = slack_final_blocks(&[make_buttons(1_226)]).unwrap_err();
        assert_eq!(overflow.kind, ReplyStreamErrorKind::InvalidContent);
        let legacy_capped = slack_actions_blocks(&[make_buttons(1_226)], 49);
        assert_eq!(legacy_capped.len(), 49);
    }

    #[test]
    fn slack_button_fields_enforce_block_kit_character_limits() {
        let make_button = |text: String, callback_data: Option<String>, url: Option<String>| {
            vec![vec![InlineButton {
                text,
                callback_data,
                url,
            }]]
        };

        assert!(validate_slack_buttons(&make_button("x".repeat(75), None, None)).is_ok());
        assert!(validate_slack_buttons(&make_button("x".repeat(76), None, None)).is_err());
        assert!(validate_slack_buttons(&make_button(
            "Pick".to_string(),
            Some("x".repeat(2_001)),
            None,
        ))
        .is_err());
        assert!(validate_slack_buttons(&make_button(
            "Docs".to_string(),
            None,
            Some("x".repeat(3_001)),
        ))
        .is_err());
        assert!(validate_slack_buttons(&make_button(
            "Docs".to_string(),
            None,
            Some("javascript:alert(1)".to_string()),
        ))
        .is_err());
    }

    #[test]
    fn slack_message_sections_are_unicode_safe_and_leave_room_for_actions() {
        let text = format!("{}🙂中文", "a".repeat(SLACK_SECTION_TEXT_MAX_CHARS - 1));
        let button = InlineButton {
            text: "Continue".to_string(),
            callback_data: Some("continue".to_string()),
            url: None,
        };
        let blocks = slack_message_blocks(&text, &[vec![button]])
            .unwrap()
            .unwrap();

        assert_eq!(blocks.len(), 3);
        assert_eq!(
            blocks[0]["text"]["text"].as_str().unwrap().chars().count(),
            SLACK_SECTION_TEXT_MAX_CHARS
        );
        assert_eq!(blocks[1]["text"]["text"], "中文");
        assert_eq!(blocks[2]["type"], "actions");
    }
}
