use crate::channel::types::{InlineButton, ReplyStreamError, ReplyStreamErrorKind};
use anyhow::{Context, Result};
use ha_core::security::http_stream::read_bytes_capped;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use teloxide::adaptors::{throttle::Limits, Throttle};
use teloxide::prelude::*;
use teloxide::requests::RequesterExt;
use teloxide::types::{
    BotCommand, CallbackQueryId, ChatAction, ChatId, InlineKeyboardButton, InlineKeyboardMarkup,
    InputFile, Me, MessageId, ParseMode as TgParseMode, ReplyParameters, ThreadId,
};

type ThrottledBot = Throttle<Bot>;

/// `sendRichMessage` returns a complete `Message`, including the accepted rich
/// document. A legal 32,768-character payload can exceed 128 KiB after UTF-8,
/// JSON structure, 500 blocks, and media metadata. Keep a bounded 2 MiB window
/// plus one sentinel byte so valid acknowledgements are not misclassified as
/// ambiguous while hostile/custom endpoints still cannot grow memory without
/// limit.
const TELEGRAM_RICH_RESPONSE_MAX_BYTES: usize = 2 * 1024 * 1024;

/// A named multipart upload referenced from a rich-message body as
/// `attach://<name>`.
pub(crate) struct RichUpload {
    pub(crate) name: String,
    pub(crate) filename: String,
    pub(crate) mime_type: Option<String>,
    pub(crate) bytes: Vec<u8>,
}

impl std::fmt::Debug for RichUpload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RichUpload")
            .field("name", &self.name)
            .field("filename", &self.filename)
            .field("mime_type", &self.mime_type)
            .field("bytes_len", &self.bytes.len())
            .finish()
    }
}

/// Minimal projection of the `Message` returned by `sendRichMessage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RichApiMessage {
    pub(crate) message_id: i64,
}

/// Telegram's structured error hints. They are diagnostic only: rich-message
/// mutations are deliberately never retried by this API layer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub(crate) struct TelegramRichResponseParameters {
    #[serde(default)]
    pub(crate) migrate_to_chat_id: Option<i64>,
    #[serde(default)]
    pub(crate) retry_after: Option<u64>,
}

/// Delivery-aware error returned by the raw rich-message endpoints.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct TelegramRichApiError {
    pub(crate) kind: ReplyStreamErrorKind,
    pub(crate) http_status: Option<u16>,
    pub(crate) error_code: Option<i64>,
    pub(crate) parameters: TelegramRichResponseParameters,
    message: String,
}

impl TelegramRichApiError {
    fn new(
        kind: ReplyStreamErrorKind,
        http_status: Option<u16>,
        error_code: Option<i64>,
        parameters: TelegramRichResponseParameters,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            http_status,
            error_code,
            parameters,
            message: ha_core::logging::redact_sensitive(&message.into()),
        }
    }

    fn invalid_content(message: impl Into<String>) -> Self {
        Self::new(
            ReplyStreamErrorKind::InvalidContent,
            None,
            None,
            TelegramRichResponseParameters::default(),
            message,
        )
    }

    pub(crate) fn into_reply_stream_error(self) -> ReplyStreamError {
        ReplyStreamError::new(self.kind, self.message)
    }
}

impl std::fmt::Display for TelegramRichApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::fmt::Debug for TelegramRichApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramRichApiError")
            .field("kind", &self.kind)
            .field("http_status", &self.http_status)
            .field("error_code", &self.error_code)
            .field("parameters", &self.parameters)
            .field("message", &"[REDACTED]")
            .finish()
    }
}

impl std::error::Error for TelegramRichApiError {}

type TelegramRichApiResult<T> = std::result::Result<T, TelegramRichApiError>;

#[derive(Debug, Deserialize)]
struct TelegramRichEnvelope {
    ok: Option<bool>,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error_code: Option<i64>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    parameters: Option<TelegramRichResponseParameters>,
}

/// Thin wrapper around teloxide's `Bot` to isolate framework details.
pub struct TelegramBotApi {
    bot: ThrottledBot,
    /// Shared `reqwest::Client` clone used for inbound media downloads.
    /// Cloning is cheap (`Arc`-internal); we keep our own handle so
    /// [`download_file_to_disk`] can hit the Telegram file CDN with the
    /// same proxy / timeout settings teloxide is using, while routing
    /// the bytes through [`inbound_media_common::stream_to_disk`] (cap
    /// + cleanup) instead of teloxide's downloader which has neither.
    http_client: reqwest::Client,
    draft_preview_enabled: AtomicBool,
    // The official cloud endpoint tracks the current Bot API. A custom root
    // has no capability endpoint, so it conservatively uses the 10.1 HTML
    // representation instead of assuming 10.2 structured blocks.
    prefers_rich_blocks: bool,
}

impl TelegramBotApi {
    /// Create a new Telegram Bot API client.
    ///
    /// Uses a custom reqwest client with proper timeouts to prevent long-polling
    /// requests from hanging indefinitely on network issues.
    ///
    /// `api_root` 让用户切到自托管 Bot API server（处理 >50MB 文件 / 内网部署）
    /// 或区域反代。设置后所有 send_* / get_* 都走该 base URL（teloxide 内部
    /// `bot.set_api_url(url)`），与官方注释"respects custom apiRoot"对齐。
    pub async fn new(token: &str, proxy_url: Option<&str>, api_root: Option<&str>) -> Result<Self> {
        let api_root = validate_api_root(api_root).await?;
        let prefers_rich_blocks = api_root.is_none();

        // Build a custom reqwest client with timeouts.
        // connect_timeout: fail fast if the server is unreachable (10s)
        // timeout: overall request timeout, must be longer than long-poll timeout (30s)
        //          to allow the server to hold the connection. Set to 60s.
        let mut client_builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            // Telegram's Bot API does not require redirects. Disabling them
            // prevents a custom reverse proxy from forwarding the bot token in
            // the request path to a different, unchecked destination.
            .redirect(reqwest::redirect::Policy::none());

        if let Some(proxy) = proxy_url {
            let proxy = reqwest::Proxy::all(proxy)
                .map_err(|_| anyhow::anyhow!("Invalid Telegram proxy URL"))?;
            client_builder = client_builder.proxy(proxy);
        }

        let client = client_builder
            .build()
            .context("Failed to build Telegram HTTP client")?;
        let mut bot = Bot::with_client(token, client.clone());
        if let Some(root) = api_root {
            bot = bot.set_api_url(root);
        }
        let bot = bot.throttle(Limits::default());

        Ok(Self {
            bot,
            http_client: client,
            draft_preview_enabled: AtomicBool::new(true),
            prefers_rich_blocks,
        })
    }

    /// Get the underlying teloxide Bot reference.
    pub fn bot(&self) -> &Bot {
        self.bot.inner()
    }

    /// Use Bot API 10.2 structured blocks only for the official cloud root.
    /// Custom roots have no version/capability endpoint and remain on the
    /// Bot API 10.1 Rich HTML shape.
    pub(crate) fn prefers_rich_blocks(&self) -> bool {
        self.prefers_rich_blocks
    }

    /// Verify the bot token and return bot info.
    pub async fn get_me(&self) -> Result<Me> {
        self.bot
            .get_me()
            .await
            .map_err(|e| self.request_error("getMe", e))
    }

    /// Send a text message, optionally with inline keyboard buttons.
    pub async fn send_text(
        &self,
        chat_id: i64,
        text: &str,
        parse_mode: Option<TgParseMode>,
        reply_to: Option<i32>,
        thread_id: Option<i32>,
        buttons: &[Vec<InlineButton>],
    ) -> Result<teloxide::types::Message> {
        let mut req = self.bot.send_message(ChatId(chat_id), text);

        if let Some(pm) = parse_mode {
            req = req.parse_mode(pm);
        }
        if let Some(reply_id) = reply_to {
            req = req.reply_parameters(ReplyParameters::new(MessageId(reply_id)));
        }
        if let Some(tid) = thread_id {
            req = req.message_thread_id(ThreadId(teloxide::types::MessageId(tid)));
        }
        if !buttons.is_empty() {
            // Fail locally before the one non-idempotent mutation. Never
            // replace an invalid destination with an unrelated URL.
            let keyboard = build_inline_keyboard(buttons)?;
            req = req.reply_markup(keyboard);
        }

        req.await.map_err(|e| self.request_error("sendMessage", e))
    }

    /// Send formatter-produced safe HTML exactly once.
    ///
    /// The legacy method name is retained for call-site compatibility, but a
    /// mutation must never be followed by a fallback send: timeout, transport,
    /// and 5xx errors can all arrive after Telegram accepted the first message.
    pub async fn send_text_with_fallback(
        &self,
        chat_id: i64,
        text: &str,
        reply_to: Option<i32>,
        thread_id: Option<i32>,
        buttons: &[Vec<InlineButton>],
    ) -> Result<teloxide::types::Message> {
        self.send_text(
            chat_id,
            text,
            Some(TgParseMode::Html),
            reply_to,
            thread_id,
            buttons,
        )
        .await
    }

    /// Send a typing indicator (chat action).
    pub async fn send_typing(&self, chat_id: i64) -> Result<()> {
        self.bot
            .send_chat_action(ChatId(chat_id), ChatAction::Typing)
            .await
            .map_err(|e| self.request_error("sendChatAction", e))?;
        Ok(())
    }

    /// Edit an existing text message.
    pub async fn edit_message_text(
        &self,
        chat_id: i64,
        message_id: i32,
        text: &str,
        parse_mode: Option<TgParseMode>,
    ) -> Result<()> {
        let mut req = self
            .bot
            .edit_message_text(ChatId(chat_id), MessageId(message_id), text);
        if let Some(pm) = parse_mode {
            req = req.parse_mode(pm);
        }
        req.await
            .map_err(|e| self.request_error("editMessageText", e))?;
        Ok(())
    }

    /// Delete a message.
    pub async fn delete_message(&self, chat_id: i64, message_id: i32) -> Result<()> {
        self.bot
            .delete_message(ChatId(chat_id), MessageId(message_id))
            .await
            .map_err(|e| self.request_error("deleteMessage", e))?;
        Ok(())
    }

    /// Stream a partial rich message. The endpoint accepts JSON only; direct
    /// uploads and unresolved `attach://` references are rejected locally.
    /// `Ok(false)` is a negative acknowledgement: callers must not open or
    /// advance a preview stream unless the returned value is `true`.
    pub(crate) async fn send_rich_message_draft(
        &self,
        body: serde_json::Value,
    ) -> TelegramRichApiResult<bool> {
        ensure_rich_body_object(&body)?;
        if !collect_attach_names(&body)?.is_empty() {
            return Err(TelegramRichApiError::invalid_content(
                "Telegram sendRichMessageDraft does not support direct file uploads",
            ));
        }

        let method = "sendRichMessageDraft";
        let url = self.rich_method_url(method);
        let request = self.http_client.post(url).json(&body);
        let envelope = self.execute_rich_request(method, request).await?;
        envelope
            .result
            .and_then(|result| result.as_bool())
            .ok_or_else(|| {
                self.rich_ambiguous_error(
                    method,
                    "success response did not contain a Boolean result",
                )
            })
    }

    /// Persist a rich message using JSON when no upload is present, otherwise
    /// multipart/form-data. `body` is the complete Bot API request body.
    pub(crate) async fn send_rich_message(
        &self,
        body: serde_json::Value,
        uploads: Vec<RichUpload>,
    ) -> TelegramRichApiResult<RichApiMessage> {
        validate_rich_upload_contract(&body, &uploads)?;

        let method = "sendRichMessage";
        let url = self.rich_method_url(method);
        let request = if uploads.is_empty() {
            self.http_client.post(url).json(&body)
        } else {
            let form = build_rich_multipart_form(body, uploads)?;
            self.http_client.post(url).multipart(form)
        };
        let envelope = self.execute_rich_request(method, request).await?;
        let message_id = envelope
            .result
            .and_then(|result| result.get("message_id").and_then(serde_json::Value::as_i64))
            .ok_or_else(|| {
                self.rich_ambiguous_error(method, "success response did not contain a message_id")
            })?;
        Ok(RichApiMessage { message_id })
    }

    fn rich_method_url(&self, method: &str) -> String {
        let api_url_owned = self.bot.inner().api_url();
        let api_url = api_url_owned.as_str().trim_end_matches('/');
        format!("{}/bot{}/{}", api_url, self.bot.inner().token(), method)
    }

    /// Execute one non-idempotent Telegram mutation exactly once.
    async fn execute_rich_request(
        &self,
        method: &str,
        request: reqwest::RequestBuilder,
    ) -> TelegramRichApiResult<TelegramRichEnvelope> {
        let response = request.send().await.map_err(|error| {
            self.rich_ambiguous_error(method, &format!("request failed: {error}"))
        })?;
        let status = response.status();

        // 5xx and redirects can arrive after the upstream accepted a mutation.
        // Never parse them into a fallback-safe error and never retry.
        if status.is_server_error() || (!status.is_success() && !status.is_client_error()) {
            return Err(telegram_rich_ambiguous_response_error(
                method,
                status,
                &format!("returned HTTP {status}"),
                self.bot.inner().token(),
            ));
        }

        let bytes =
            match read_bytes_capped(response, TELEGRAM_RICH_RESPONSE_MAX_BYTES.saturating_add(1))
                .await
            {
                Ok(bytes) => bytes,
                Err(_) if status.is_client_error() => {
                    return Err(telegram_rich_rejection_error(
                        method,
                        status,
                        None,
                        self.bot.inner().token(),
                    ));
                }
                Err(error) => {
                    return Err(telegram_rich_ambiguous_response_error(
                        method,
                        status,
                        &format!("could not read the response: {error}"),
                        self.bot.inner().token(),
                    ));
                }
            };

        if bytes.len() > TELEGRAM_RICH_RESPONSE_MAX_BYTES {
            // A 4xx proves Telegram rejected the request even if its diagnostic
            // body is hostile or oversized. A 2xx body is part of the mutation
            // acknowledgement, so truncation makes the outcome ambiguous.
            if status.is_client_error() {
                return Err(telegram_rich_rejection_error(
                    method,
                    status,
                    None,
                    self.bot.inner().token(),
                ));
            }
            return Err(telegram_rich_ambiguous_response_error(
                method,
                status,
                "response body exceeded the safe size limit",
                self.bot.inner().token(),
            ));
        }

        decode_telegram_rich_envelope(method, status, &bytes, self.bot.inner().token())
    }

    fn rich_ambiguous_error(&self, method: &str, detail: &str) -> TelegramRichApiError {
        let detail = redact_telegram_error(detail, self.bot.inner().token());
        TelegramRichApiError::new(
            ReplyStreamErrorKind::Ambiguous,
            None,
            None,
            TelegramRichResponseParameters::default(),
            format!(
                "Telegram {method} delivery outcome is ambiguous: {}",
                ha_core::truncate_utf8(&detail, 1000)
            ),
        )
    }

    /// Send a message draft for streaming (Bot API 9.3+).
    ///
    /// This is a purpose-built method for streaming partial messages during generation.
    /// Unlike `editMessageText`, it has no rate limiting and renders progressively
    /// without flicker. Call repeatedly with accumulated text, then finalize with
    /// `send_text()` to commit the message.
    ///
    /// teloxide 0.13 doesn't have native support, so we use a raw HTTP request.
    pub async fn send_message_draft(
        &self,
        chat_id: i64,
        text: &str,
        draft_id: i64,
        parse_mode: Option<&str>,
        thread_id: Option<i32>,
    ) -> Result<()> {
        if !self.draft_preview_enabled.load(Ordering::Relaxed) {
            return Ok(());
        }

        let token = self.bot.inner().token();
        // Use the bot's API URL base (respects custom apiRoot)
        let api_url_owned = self.bot.inner().api_url();
        let api_url = api_url_owned.as_str().trim_end_matches('/');
        let url = format!("{}/bot{}/sendMessageDraft", api_url, token);

        let body = build_send_message_draft_body(chat_id, text, draft_id, parse_mode, thread_id);

        // Reuse the Bot's client so proxy, timeouts, TLS roots, and the
        // no-redirect policy stay identical across SDK and raw endpoints.
        let resp = self
            .http_client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| self.request_error("sendMessageDraft", e))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let redacted = redact_telegram_error(&text, self.bot.inner().token());
            let body = ha_core::truncate_utf8(&redacted, 200);
            if send_message_draft_soft_degrades(status) {
                self.draft_preview_enabled.store(false, Ordering::Relaxed);
                app_warn!(
                    "channel",
                    "telegram::api",
                    "sendMessageDraft rejected with {}; disabling draft preview until account restart: {}",
                    status,
                    body
                );
                return Ok(());
            }
            anyhow::bail!("sendMessageDraft failed ({}): {}", status, body);
        }

        Ok(())
    }

    /// Get updates using long-polling.
    pub async fn get_updates(
        &self,
        offset: i32,
        timeout: u32,
        allowed_updates: &[&str],
    ) -> Result<Vec<teloxide::types::Update>> {
        use teloxide::types::AllowedUpdate;

        let mut req = self.bot.get_updates().offset(offset).timeout(timeout);

        // Map string allowed_updates to teloxide enum
        let updates: Vec<AllowedUpdate> = allowed_updates
            .iter()
            .filter_map(|s| match *s {
                "message" => Some(AllowedUpdate::Message),
                "edited_message" => Some(AllowedUpdate::EditedMessage),
                "callback_query" => Some(AllowedUpdate::CallbackQuery),
                "channel_post" => Some(AllowedUpdate::ChannelPost),
                _ => None,
            })
            .collect();

        if !updates.is_empty() {
            req = req.allowed_updates(updates);
        }

        req.await.map_err(|e| self.request_error("getUpdates", e))
    }

    /// POST answerCallbackQuery — acknowledge a callback query (dismisses loading spinner).
    pub async fn answer_callback_query(
        &self,
        callback_query_id: &str,
        text: Option<&str>,
    ) -> Result<()> {
        let mut req = self
            .bot
            .answer_callback_query(CallbackQueryId(callback_query_id.to_string()));
        if let Some(t) = text {
            req = req.text(t);
        }
        req.await
            .map_err(|e| self.request_error("answerCallbackQuery", e))?;
        Ok(())
    }

    /// POST editMessageReplyMarkup — remove inline keyboard from a message.
    pub async fn remove_inline_keyboard(&self, chat_id: i64, message_id: i32) -> Result<()> {
        self.bot
            .edit_message_reply_markup(ChatId(chat_id), MessageId(message_id))
            .await
            .map_err(|e| self.request_error("editMessageReplyMarkup", e))?;
        Ok(())
    }

    /// Register bot menu commands via setMyCommands API.
    pub async fn set_my_commands(&self, commands: Vec<BotCommand>) -> Result<()> {
        self.bot
            .set_my_commands(commands)
            .await
            .map_err(|e| self.request_error("setMyCommands", e))?;
        Ok(())
    }

    /// Download a file by file_id (returns the file path on Telegram servers).
    pub async fn get_file(&self, file_id: &str) -> Result<teloxide::types::File> {
        use teloxide::types::FileId;
        self.bot
            .get_file(FileId(file_id.to_string()))
            .await
            .map_err(|e| self.request_error("getFile", e))
    }

    /// Download a file by `file_id` to `dest`, enforcing `cap_bytes`.
    ///
    /// Goes around teloxide's `bot.download_file` because it has no size
    /// cap and no failure cleanup. We `get_file` first to resolve the
    /// CDN path, reject early when the returned metadata already
    /// exceeds the cap, then push the bytes through
    /// [`crate::channel::inbound_media_common::stream_to_disk`] using
    /// our own `reqwest::Client` clone — same proxy / timeout settings
    /// teloxide is configured with.
    pub async fn download_file_to_disk(
        &self,
        file_id: &str,
        dest: &std::path::Path,
        cap_bytes: u64,
    ) -> Result<u64> {
        let file = self.get_file(file_id).await?;
        if (file.size as u64) > cap_bytes {
            anyhow::bail!(
                "Telegram file size {} bytes exceeds {} byte cap",
                file.size,
                cap_bytes
            );
        }
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        validate_telegram_file_path(&file.path)?;
        let api_url_owned = self.bot.inner().api_url();
        let api_url = api_url_owned.as_str().trim_end_matches('/');
        let token = self.bot.inner().token();
        let url = format!("{}/file/bot{}/{}", api_url, token, file.path);
        let builder = self.http_client.get(&url);
        crate::channel::inbound_media_common::stream_to_disk(builder, dest, cap_bytes)
            .await
            .map_err(|e| self.request_error("downloadFile", e))
    }

    /// Send a photo.
    pub async fn send_photo(
        &self,
        chat_id: i64,
        photo: InputFile,
        caption: Option<&str>,
        thread_id: Option<i32>,
    ) -> Result<teloxide::types::Message> {
        let mut req = self.bot.send_photo(ChatId(chat_id), photo);
        if let Some(c) = caption {
            req = req.caption(c);
        }
        if let Some(tid) = thread_id {
            req = req.message_thread_id(ThreadId(teloxide::types::MessageId(tid)));
        }
        req.await.map_err(|e| self.request_error("sendPhoto", e))
    }

    /// Send a video.
    pub async fn send_video(
        &self,
        chat_id: i64,
        video: InputFile,
        caption: Option<&str>,
        thread_id: Option<i32>,
    ) -> Result<teloxide::types::Message> {
        let mut req = self.bot.send_video(ChatId(chat_id), video);
        if let Some(c) = caption {
            req = req.caption(c);
        }
        if let Some(tid) = thread_id {
            req = req.message_thread_id(telegram_thread_id(tid));
        }
        req.await.map_err(|e| self.request_error("sendVideo", e))
    }

    /// Send an audio file.
    pub async fn send_audio(
        &self,
        chat_id: i64,
        audio: InputFile,
        caption: Option<&str>,
        thread_id: Option<i32>,
    ) -> Result<teloxide::types::Message> {
        let mut req = self.bot.send_audio(ChatId(chat_id), audio);
        if let Some(c) = caption {
            req = req.caption(c);
        }
        if let Some(tid) = thread_id {
            req = req.message_thread_id(telegram_thread_id(tid));
        }
        req.await.map_err(|e| self.request_error("sendAudio", e))
    }

    /// Send a voice message.
    pub async fn send_voice(
        &self,
        chat_id: i64,
        voice: InputFile,
        caption: Option<&str>,
        thread_id: Option<i32>,
    ) -> Result<teloxide::types::Message> {
        let mut req = self.bot.send_voice(ChatId(chat_id), voice);
        if let Some(c) = caption {
            req = req.caption(c);
        }
        if let Some(tid) = thread_id {
            req = req.message_thread_id(telegram_thread_id(tid));
        }
        req.await.map_err(|e| self.request_error("sendVoice", e))
    }

    /// Send an animation.
    pub async fn send_animation(
        &self,
        chat_id: i64,
        animation: InputFile,
        caption: Option<&str>,
        thread_id: Option<i32>,
    ) -> Result<teloxide::types::Message> {
        let mut req = self.bot.send_animation(ChatId(chat_id), animation);
        if let Some(c) = caption {
            req = req.caption(c);
        }
        if let Some(tid) = thread_id {
            req = req.message_thread_id(telegram_thread_id(tid));
        }
        req.await
            .map_err(|e| self.request_error("sendAnimation", e))
    }

    /// Send a sticker. Telegram stickers do not support captions.
    pub async fn send_sticker(
        &self,
        chat_id: i64,
        sticker: InputFile,
        thread_id: Option<i32>,
    ) -> Result<teloxide::types::Message> {
        let mut req = self.bot.send_sticker(ChatId(chat_id), sticker);
        if let Some(tid) = thread_id {
            req = req.message_thread_id(telegram_thread_id(tid));
        }
        req.await.map_err(|e| self.request_error("sendSticker", e))
    }

    /// Send a document (file).
    pub async fn send_document(
        &self,
        chat_id: i64,
        document: InputFile,
        caption: Option<&str>,
        thread_id: Option<i32>,
    ) -> Result<teloxide::types::Message> {
        let mut req = self.bot.send_document(ChatId(chat_id), document);
        if let Some(c) = caption {
            req = req.caption(c);
        }
        if let Some(tid) = thread_id {
            req = req.message_thread_id(ThreadId(teloxide::types::MessageId(tid)));
        }
        req.await.map_err(|e| self.request_error("sendDocument", e))
    }

    fn request_error(&self, operation: &str, error: impl std::fmt::Display) -> anyhow::Error {
        let message = redact_telegram_error(&error.to_string(), self.bot.inner().token());
        anyhow::anyhow!(
            "{} failed: {}",
            operation,
            ha_core::truncate_utf8(&message, 1000)
        )
    }
}

/// Parse and normalize a user-supplied Telegram Bot API root without doing
/// network I/O. A path prefix is allowed for reverse proxies mounted below the
/// origin; URL credentials, query strings, and fragments are rejected because
/// the raw draft/file paths cannot preserve them safely or consistently.
fn parse_api_root(api_root: Option<&str>) -> Result<Option<reqwest::Url>> {
    let Some(raw) = api_root.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let url = reqwest::Url::parse(raw).context("Invalid Telegram Bot API base URL")?;
    if !matches!(url.scheme(), "http" | "https") {
        anyhow::bail!("Telegram Bot API base URL must use http:// or https://");
    }
    if url.cannot_be_a_base() || url.host_str().is_none() {
        anyhow::bail!("Telegram Bot API base URL must include a valid host");
    }
    if !url.username().is_empty() || url.password().is_some() {
        anyhow::bail!("Telegram Bot API base URL must not include credentials");
    }
    if url.query().is_some() || url.fragment().is_some() {
        anyhow::bail!("Telegram Bot API base URL must not include a query or fragment");
    }

    // Canonicalize trailing slashes from the original serialized value. Using
    // `Url::set_path(url.path())` would double-encode an existing `%xx`
    // sequence in a valid reverse-proxy path.
    let normalized = raw.trim_end_matches('/');
    let normalized =
        reqwest::Url::parse(normalized).context("Invalid Telegram Bot API base URL")?;
    Ok(Some(normalized))
}

async fn validate_api_root(api_root: Option<&str>) -> Result<Option<reqwest::Url>> {
    let cfg = ha_core::config::cached_config();
    validate_api_root_with_policy(api_root, cfg.ssrf.default_policy, &cfg.ssrf.trusted_hosts).await
}

async fn validate_api_root_with_policy(
    api_root: Option<&str>,
    policy: ha_core::security::ssrf::SsrfPolicy,
    trusted_hosts: &[String],
) -> Result<Option<reqwest::Url>> {
    let Some(parsed) = parse_api_root(api_root)? else {
        return Ok(None);
    };
    let checked = ha_core::security::ssrf::check_url(parsed.as_str(), policy, trusted_hosts)
        .await
        .context("Telegram Bot API base URL blocked")?;
    Ok(Some(checked))
}

fn redact_telegram_error(message: &str, token: &str) -> String {
    let without_token = if token.is_empty() {
        message.to_string()
    } else {
        message.replace(token, "[REDACTED]")
    };
    ha_core::logging::redact_sensitive(&without_token)
}

fn ensure_rich_body_object(body: &serde_json::Value) -> TelegramRichApiResult<()> {
    if body.is_object() {
        Ok(())
    } else {
        Err(TelegramRichApiError::invalid_content(
            "Telegram rich-message request body must be a JSON object",
        ))
    }
}

fn valid_attach_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn collect_attach_names(body: &serde_json::Value) -> TelegramRichApiResult<BTreeSet<String>> {
    // InputRichMessage can contain InputMedia objects at arbitrary nesting
    // depths. Only these InputMedia fields accept `attach://`; rich text,
    // captions, callback data, and other ordinary strings do not.
    fn is_file_reference_field(name: &str) -> bool {
        matches!(name, "media" | "thumbnail" | "cover" | "photo")
    }

    fn visit(
        value: &serde_json::Value,
        field_name: Option<&str>,
        names: &mut BTreeSet<String>,
    ) -> TelegramRichApiResult<()> {
        match value {
            serde_json::Value::String(value) => {
                if field_name.is_some_and(is_file_reference_field) {
                    let Some(name) = value.strip_prefix("attach://") else {
                        return Ok(());
                    };
                    if !valid_attach_name(name) {
                        return Err(TelegramRichApiError::invalid_content(
                            "Telegram rich-message attachment name must match [A-Za-z0-9_-]{1,64}",
                        ));
                    }
                    names.insert(name.to_string());
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    visit(value, field_name, names)?;
                }
            }
            serde_json::Value::Object(values) => {
                for (name, value) in values {
                    visit(value, Some(name), names)?;
                }
            }
            serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            }
        }
        Ok(())
    }

    ensure_rich_body_object(body)?;
    let mut names = BTreeSet::new();
    if let Some(rich_message) = body.get("rich_message") {
        visit(rich_message, None, &mut names)?;
    }
    Ok(names)
}

fn validate_rich_upload_contract(
    body: &serde_json::Value,
    uploads: &[RichUpload],
) -> TelegramRichApiResult<()> {
    ensure_rich_body_object(body)?;
    let references = collect_attach_names(body)?;
    let top_level = body.as_object().expect("body shape checked above");
    let mut upload_names = BTreeSet::new();

    for upload in uploads {
        if !valid_attach_name(&upload.name) {
            return Err(TelegramRichApiError::invalid_content(
                "Telegram rich-message upload name must match [A-Za-z0-9_-]{1,64}",
            ));
        }
        if !upload_names.insert(upload.name.clone()) {
            return Err(TelegramRichApiError::invalid_content(
                "Telegram rich-message upload names must be unique",
            ));
        }
        if top_level.contains_key(&upload.name) {
            return Err(TelegramRichApiError::invalid_content(
                "Telegram rich-message upload name conflicts with a request field",
            ));
        }
        if upload.filename.is_empty() || upload.filename.chars().any(char::is_control) {
            return Err(TelegramRichApiError::invalid_content(
                "Telegram rich-message upload filename is invalid",
            ));
        }
        if upload.bytes.is_empty() {
            return Err(TelegramRichApiError::invalid_content(
                "Telegram rich-message uploads must not be empty",
            ));
        }
    }

    if references != upload_names {
        return Err(TelegramRichApiError::invalid_content(
            "Telegram rich-message attach references and multipart uploads must match exactly",
        ));
    }
    Ok(())
}

fn rich_multipart_text_fields(
    body: &serde_json::Value,
) -> TelegramRichApiResult<Vec<(String, String)>> {
    ensure_rich_body_object(body)?;
    let mut fields = Vec::new();
    for (name, value) in body.as_object().expect("body shape checked above") {
        let value = match value {
            serde_json::Value::Null => continue,
            serde_json::Value::String(value) => value.clone(),
            serde_json::Value::Bool(value) => value.to_string(),
            serde_json::Value::Number(value) => value.to_string(),
            serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                serde_json::to_string(value).map_err(|_| {
                    TelegramRichApiError::invalid_content(
                        "Telegram rich-message request contains an invalid JSON field",
                    )
                })?
            }
        };
        fields.push((name.clone(), value));
    }
    Ok(fields)
}

fn build_rich_multipart_form(
    body: serde_json::Value,
    uploads: Vec<RichUpload>,
) -> TelegramRichApiResult<reqwest::multipart::Form> {
    // Keep this validation inside the builder as a second boundary even when
    // callers have already selected multipart via `send_rich_message`.
    validate_rich_upload_contract(&body, &uploads)?;
    let mut form = reqwest::multipart::Form::new();
    for (name, value) in rich_multipart_text_fields(&body)? {
        form = form.text(name, value);
    }
    for upload in uploads {
        let mut part = reqwest::multipart::Part::bytes(upload.bytes).file_name(upload.filename);
        if let Some(mime_type) = upload
            .mime_type
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            part = part.mime_str(mime_type).map_err(|_| {
                TelegramRichApiError::invalid_content(
                    "Telegram rich-message upload MIME type is invalid",
                )
            })?;
        }
        form = form.part(upload.name, part);
    }
    Ok(form)
}

fn classify_telegram_rejection(code: i64) -> ReplyStreamErrorKind {
    match code {
        404 => ReplyStreamErrorKind::Unsupported,
        429 => ReplyStreamErrorKind::RateLimited,
        500..=599 => ReplyStreamErrorKind::Ambiguous,
        _ => ReplyStreamErrorKind::Rejected,
    }
}

fn telegram_rich_ambiguous_response_error(
    method: &str,
    status: reqwest::StatusCode,
    detail: &str,
    token: &str,
) -> TelegramRichApiError {
    let detail = redact_telegram_error(detail, token);
    TelegramRichApiError::new(
        ReplyStreamErrorKind::Ambiguous,
        Some(status.as_u16()),
        None,
        TelegramRichResponseParameters::default(),
        format!(
            "Telegram {method} delivery outcome is ambiguous: {}",
            ha_core::truncate_utf8(&detail, 1000)
        ),
    )
}

fn telegram_rich_rejection_error(
    method: &str,
    status: reqwest::StatusCode,
    envelope: Option<&TelegramRichEnvelope>,
    token: &str,
) -> TelegramRichApiError {
    let error_code = envelope.and_then(|envelope| envelope.error_code);
    let classification_code = if status.is_client_error() {
        i64::from(status.as_u16())
    } else {
        error_code.unwrap_or_default()
    };
    let parameters = envelope
        .and_then(|envelope| envelope.parameters.clone())
        .unwrap_or_default();
    let description = envelope
        .and_then(|envelope| envelope.description.as_deref())
        .map(|description| redact_telegram_error(description, token));
    let detail = description
        .as_deref()
        .map(|description| ha_core::truncate_utf8(description, 500))
        .unwrap_or("request was rejected");

    TelegramRichApiError::new(
        classify_telegram_rejection(classification_code),
        Some(status.as_u16()),
        error_code,
        parameters,
        format!("Telegram {method} rejected the request (HTTP {status}): {detail}"),
    )
}

fn decode_telegram_rich_envelope(
    method: &str,
    status: reqwest::StatusCode,
    body: &[u8],
    token: &str,
) -> TelegramRichApiResult<TelegramRichEnvelope> {
    if status.is_server_error() || (!status.is_success() && !status.is_client_error()) {
        return Err(telegram_rich_ambiguous_response_error(
            method,
            status,
            &format!("returned HTTP {status}"),
            token,
        ));
    }

    if status.is_client_error() {
        let envelope = serde_json::from_slice::<TelegramRichEnvelope>(body).ok();
        return Err(telegram_rich_rejection_error(
            method,
            status,
            envelope.as_ref(),
            token,
        ));
    }

    let envelope = serde_json::from_slice::<TelegramRichEnvelope>(body).map_err(|_| {
        telegram_rich_ambiguous_response_error(
            method,
            status,
            "returned a non-JSON success response",
            token,
        )
    })?;
    match envelope.ok {
        Some(false) => Err(telegram_rich_rejection_error(
            method,
            status,
            Some(&envelope),
            token,
        )),
        Some(true) if envelope.result.is_some() => Ok(envelope),
        Some(true) => Err(telegram_rich_ambiguous_response_error(
            method,
            status,
            "success response did not contain a result",
            token,
        )),
        None => Err(telegram_rich_ambiguous_response_error(
            method,
            status,
            "response did not contain the ok field",
            token,
        )),
    }
}

fn build_send_message_draft_body(
    chat_id: i64,
    text: &str,
    draft_id: i64,
    parse_mode: Option<&str>,
    thread_id: Option<i32>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
        "draft_id": draft_id,
    });

    if let Some(mode) = parse_mode.filter(|mode| !mode.is_empty()) {
        body["parse_mode"] = serde_json::json!(mode);
    }
    if let Some(tid) = thread_id {
        body["message_thread_id"] = serde_json::json!(tid);
    }

    body
}

fn telegram_thread_id(tid: i32) -> ThreadId {
    ThreadId(MessageId(tid))
}

fn send_message_draft_soft_degrades(status: reqwest::StatusCode) -> bool {
    status.is_client_error()
}

/// Convert our `InlineButton` rows into teloxide's `InlineKeyboardMarkup`.
fn build_inline_keyboard(buttons: &[Vec<InlineButton>]) -> Result<InlineKeyboardMarkup> {
    let rows: Vec<Vec<InlineKeyboardButton>> = buttons
        .iter()
        .map(|row| -> Result<Vec<InlineKeyboardButton>> {
            row.iter()
                .map(|b| -> Result<InlineKeyboardButton> {
                    if let Some(ref url) = b.url {
                        let url = url.parse().context("Invalid Telegram inline button URL")?;
                        Ok(InlineKeyboardButton::url(b.text.clone(), url))
                    } else {
                        let cb = b.callback_data.clone().unwrap_or_else(|| b.text.clone());
                        Ok(InlineKeyboardButton::callback(b.text.clone(), cb))
                    }
                })
                .collect()
        })
        .collect::<Result<_>>()?;
    Ok(InlineKeyboardMarkup::new(rows))
}

#[cfg(test)]
mod tests {
    use super::{
        build_inline_keyboard, build_send_message_draft_body, collect_attach_names,
        decode_telegram_rich_envelope, parse_api_root, redact_telegram_error,
        rich_multipart_text_fields, send_message_draft_soft_degrades,
        validate_api_root_with_policy, validate_rich_upload_contract, RichUpload,
    };
    use crate::channel::types::InlineButton;
    use crate::channel::types::ReplyStreamErrorKind;
    use ha_core::security::ssrf::SsrfPolicy;
    use reqwest::StatusCode;

    fn upload(name: &str) -> RichUpload {
        RichUpload {
            name: name.to_string(),
            filename: "image.png".to_string(),
            mime_type: Some("image/png".to_string()),
            bytes: vec![1, 2, 3],
        }
    }

    #[test]
    fn rich_multipart_fields_preserve_full_body_and_callback_data() {
        let body = serde_json::json!({
            "chat_id": -100123,
            "callback_query_id": "callback-1",
            "disable_notification": true,
            "rich_message": {
                "blocks": [{
                    "type": "photo",
                    "photo": {"type": "photo", "media": "attach://image_1"}
                }]
            },
            "reply_markup": {
                "inline_keyboard": [[{"text": "Continue", "callback_data": "next:1"}]]
            },
            "omitted": null,
        });

        validate_rich_upload_contract(&body, &[upload("image_1")]).unwrap();
        let fields = rich_multipart_text_fields(&body)
            .unwrap()
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>();

        assert_eq!(fields["chat_id"], "-100123");
        assert_eq!(fields["callback_query_id"], "callback-1");
        assert_eq!(fields["disable_notification"], "true");
        assert!(!fields.contains_key("omitted"));
        let markup: serde_json::Value = serde_json::from_str(&fields["reply_markup"]).unwrap();
        assert_eq!(markup["inline_keyboard"][0][0]["callback_data"], "next:1");
        let rich_message: serde_json::Value =
            serde_json::from_str(&fields["rich_message"]).unwrap();
        assert_eq!(
            rich_message["blocks"][0]["photo"]["media"],
            "attach://image_1"
        );
    }

    #[test]
    fn rich_upload_contract_is_bidirectional_and_names_are_strict() {
        let body = serde_json::json!({
            "chat_id": 1,
            "rich_message": {
                "media": [{
                    "id": "photo-1",
                    "media": {"type": "photo", "media": "attach://photo_1"}
                }]
            }
        });

        assert!(validate_rich_upload_contract(&body, &[upload("photo_1")]).is_ok());
        assert_eq!(
            collect_attach_names(&body).unwrap(),
            std::collections::BTreeSet::from(["photo_1".to_string()])
        );
        assert_eq!(
            validate_rich_upload_contract(&body, &[]).unwrap_err().kind,
            ReplyStreamErrorKind::InvalidContent
        );
        assert_eq!(
            validate_rich_upload_contract(&body, &[upload("photo_1"), upload("extra")])
                .unwrap_err()
                .kind,
            ReplyStreamErrorKind::InvalidContent
        );
        assert_eq!(
            validate_rich_upload_contract(&body, &[upload("photo_1"), upload("photo_1")])
                .unwrap_err()
                .kind,
            ReplyStreamErrorKind::InvalidContent
        );

        let invalid = serde_json::json!({
            "rich_message": {
                "media": [{
                    "id": "bad",
                    "media": {"type": "photo", "media": "attach://bad.name"}
                }]
            }
        });
        assert_eq!(
            collect_attach_names(&invalid).unwrap_err().kind,
            ReplyStreamErrorKind::InvalidContent
        );
    }

    #[test]
    fn rich_attachment_scan_ignores_text_and_covers_input_media_file_fields() {
        let body = serde_json::json!({
            "chat_id": 1,
            "rich_message": {
                "html": "<p>Literal attach://body text</p>",
                "blocks": [{
                    "type": "video",
                    "video": {
                        "type": "video",
                        "media": "attach://video_1",
                        "thumbnail": "attach://thumb_1",
                        "cover": "attach://cover_1"
                    },
                    "caption": {"text": "attach://caption_text"}
                }]
            },
            "reply_markup": {
                "inline_keyboard": [[{
                    "text": "Continue",
                    "callback_data": "attach://callback_text"
                }]]
            }
        });

        assert_eq!(
            collect_attach_names(&body).unwrap(),
            std::collections::BTreeSet::from([
                "cover_1".to_string(),
                "thumb_1".to_string(),
                "video_1".to_string(),
            ])
        );

        let text_only = serde_json::json!({
            "media": "attach://unrelated_top_level_field",
            "rich_message": {
                "html": "attach://ordinary_text",
                "blocks": [{"type": "paragraph", "text": "attach://also_text"}]
            },
            "reply_markup": {
                "inline_keyboard": [[{"callback_data": "attach://callback"}]]
            }
        });
        assert!(collect_attach_names(&text_only).unwrap().is_empty());
        assert!(validate_rich_upload_contract(&text_only, &[]).is_ok());
    }

    #[test]
    fn rich_envelopes_parse_minimal_results_and_structured_parameters() {
        let message = decode_telegram_rich_envelope(
            "sendRichMessage",
            StatusCode::OK,
            br#"{"ok":true,"result":{"message_id":922337}}"#,
            "token",
        )
        .unwrap();
        assert_eq!(message.result.unwrap()["message_id"], 922337);

        let draft = decode_telegram_rich_envelope(
            "sendRichMessageDraft",
            StatusCode::OK,
            br#"{"ok":true,"result":true}"#,
            "token",
        )
        .unwrap();
        assert_eq!(draft.result.unwrap(), true);

        let token = "123456:secret";
        let body = format!(
            r#"{{"ok":false,"error_code":429,"description":"retry bot{token}","parameters":{{"migrate_to_chat_id":-100123,"retry_after":7}}}}"#
        );
        let error = decode_telegram_rich_envelope(
            "sendRichMessage",
            StatusCode::OK,
            body.as_bytes(),
            token,
        )
        .unwrap_err();
        assert_eq!(error.kind, ReplyStreamErrorKind::RateLimited);
        assert_eq!(error.error_code, Some(429));
        assert_eq!(error.parameters.migrate_to_chat_id, Some(-100123));
        assert_eq!(error.parameters.retry_after, Some(7));
        assert!(!error.to_string().contains(token));
        assert!(!format!("{error:?}").contains(token));

        // Some reverse proxies preserve Telegram's JSON error envelope while
        // rewriting the HTTP status to 200. A provider-side 5xx still means
        // the mutation outcome is unknown and must never unlock legacy replay.
        let error = decode_telegram_rich_envelope(
            "sendRichMessage",
            StatusCode::OK,
            br#"{"ok":false,"error_code":500,"description":"upstream failure"}"#,
            token,
        )
        .unwrap_err();
        assert_eq!(error.kind, ReplyStreamErrorKind::Ambiguous);
        assert_eq!(error.error_code, Some(500));
    }

    #[test]
    fn rich_delivery_classification_is_conservative() {
        let unsupported = decode_telegram_rich_envelope(
            "sendRichMessage",
            StatusCode::NOT_FOUND,
            b"not-json",
            "token",
        )
        .unwrap_err();
        assert_eq!(unsupported.kind, ReplyStreamErrorKind::Unsupported);

        let rejected = decode_telegram_rich_envelope(
            "sendRichMessage",
            StatusCode::BAD_REQUEST,
            br#"{"ok":false,"description":"bad body"}"#,
            "token",
        )
        .unwrap_err();
        assert_eq!(rejected.kind, ReplyStreamErrorKind::Rejected);

        for (status, body) in [
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                br#"{"ok":false}"#.as_slice(),
            ),
            (StatusCode::OK, b"not-json".as_slice()),
            (StatusCode::OK, br#"{"ok":true}"#.as_slice()),
        ] {
            let error = decode_telegram_rich_envelope("sendRichMessage", status, body, "token")
                .unwrap_err();
            assert_eq!(error.kind, ReplyStreamErrorKind::Ambiguous);
        }
    }

    #[test]
    fn legacy_keyboard_rejects_invalid_urls_instead_of_substituting_a_destination() {
        let buttons = vec![vec![InlineButton {
            text: "Open".to_string(),
            callback_data: None,
            url: Some("not a url".to_string()),
        }]];

        assert!(build_inline_keyboard(&buttons).is_err());
    }

    #[test]
    fn send_message_draft_body_only_uses_supported_fields() {
        let body = build_send_message_draft_body(123, "<b>hi</b>", 42, Some("HTML"), Some(7));

        assert_eq!(body["chat_id"], 123);
        assert_eq!(body["text"], "<b>hi</b>");
        assert_eq!(body["draft_id"], 42);
        assert_eq!(body["parse_mode"], "HTML");
        assert_eq!(body["message_thread_id"], 7);
        assert!(body.get("reply_parameters").is_none());
    }

    #[test]
    fn send_message_draft_only_soft_degrades_client_errors() {
        assert!(send_message_draft_soft_degrades(StatusCode::BAD_REQUEST));
        assert!(send_message_draft_soft_degrades(StatusCode::NOT_FOUND));
        assert!(!send_message_draft_soft_degrades(
            StatusCode::INTERNAL_SERVER_ERROR
        ));
    }

    #[test]
    fn api_root_normalizes_optional_root_and_keeps_path_prefix() {
        assert!(parse_api_root(None).unwrap().is_none());
        assert!(parse_api_root(Some("  ")).unwrap().is_none());

        let root = parse_api_root(Some(" https://tg.example.com/telegram/// "))
            .unwrap()
            .unwrap();
        assert_eq!(root.as_str(), "https://tg.example.com/telegram");

        let encoded = parse_api_root(Some("https://tg.example.com/my%20proxy/"))
            .unwrap()
            .unwrap();
        assert_eq!(encoded.as_str(), "https://tg.example.com/my%20proxy");
    }

    #[test]
    fn api_root_rejects_unsafe_or_incompatible_shapes() {
        for root in [
            "file:///tmp/telegram",
            "https://user:pass@tg.example.com",
            "https://tg.example.com?target=telegram",
            "https://tg.example.com/#fragment",
        ] {
            assert!(parse_api_root(Some(root)).is_err(), "root={root}");
        }
    }

    #[test]
    fn telegram_proxy_accepts_socks_schemes() {
        assert!(reqwest::Proxy::all("socks5://127.0.0.1:1080").is_ok());
        assert!(reqwest::Proxy::all("socks5h://proxy.example.com:1080").is_ok());
    }

    #[test]
    fn telegram_error_redaction_removes_bot_token_from_url_paths() {
        let token = "123456:ABC_secret";
        let raw =
            format!("error sending request for url (https://tg.example.com/bot{token}/getMe)");
        let redacted = redact_telegram_error(&raw, token);
        assert!(!redacted.contains(token));
        assert!(redacted.contains("bot[REDACTED]/getMe"));
    }

    #[tokio::test]
    async fn api_root_ssrf_check_blocks_metadata_even_in_allow_private_mode() {
        let error = validate_api_root_with_policy(
            Some("http://169.254.169.254"),
            SsrfPolicy::AllowPrivate,
            &[],
        )
        .await
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("Telegram Bot API base URL blocked"));
    }
}

/// Telegram Bot API 契约：`file.path` 是 base URL 之下的相对路径（如
/// `photos/file_1234.jpg`）。**恶意或被劫持的 upstream** 可以违反契约返回
/// `../` 段、绝对路径、编码遍历（`%2e%2e`）或空字节；把这些直接拼进
/// `{api_url}/file/bot{TOKEN}/{path}` 会让反代路径 normalize 后把
/// `bot<TOKEN>` 顺着遍历发往 SSRF gate 从没验过的位置——**token 泄漏 +
/// 未审目标请求**。提前拒绝，只放行「相对、无遍历、无编码、无空字节」。
fn validate_telegram_file_path(path: &str) -> anyhow::Result<()> {
    if path.is_empty() {
        anyhow::bail!("Telegram file.path is empty");
    }
    // 绝对路径（POSIX 或 Windows 风格）—— Bot API 契约里 file.path 应始终相对。
    if path.starts_with('/') || path.starts_with('\\') {
        anyhow::bail!("Telegram file.path must be relative (got '{}')", path);
    }
    // Windows 盘符（'C:'）也拒绝。
    if path.len() >= 2 && path.as_bytes()[1] == b':' {
        anyhow::bail!("Telegram file.path must be relative (got '{}')", path);
    }
    // 段级检查：任何 `.` / `..` / 空段（意味着 `//`）都拒绝。
    for seg in path.split(['/', '\\']) {
        if seg.is_empty() || seg == "." || seg == ".." {
            anyhow::bail!(
                "Telegram file.path contains traversal or empty segment (got '{}')",
                path
            );
        }
    }
    // 控制字符（含 NUL）与 `%` 编码 —— 一律拒。反代 normalize 后可能等价于
    // `..`/`\0`/绝对路径，而我们不打算逐一枚举 unicode/percent 变体。
    if path.contains(|c: char| c.is_control()) {
        anyhow::bail!("Telegram file.path contains control characters");
    }
    if path.contains('%') {
        anyhow::bail!(
            "Telegram file.path contains percent-encoded segment; Bot API does not need URL escaping"
        );
    }
    Ok(())
}

#[cfg(test)]
mod telegram_file_path_tests {
    use super::validate_telegram_file_path;

    #[test]
    fn accepts_relative_paths() {
        assert!(validate_telegram_file_path("photos/file_1.jpg").is_ok());
        assert!(validate_telegram_file_path("documents/nested/path/file.pdf").is_ok());
    }

    #[test]
    fn rejects_absolute_and_traversal() {
        for bad in [
            "",
            "/absolute",
            "\\windows",
            "C:/Users",
            "..",
            "../etc/passwd",
            "photos/../..",
            "photos/./file",
            "photos//file",
            "photos/%2e%2e/etc",
            "photos/file\x00.jpg",
        ] {
            assert!(
                validate_telegram_file_path(bad).is_err(),
                "should reject '{}'",
                bad
            );
        }
    }
}
