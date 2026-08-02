use crate::channel::types::InlineButton;
use anyhow::{Context, Result};
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
        })
    }

    /// Get the underlying teloxide Bot reference.
    pub fn bot(&self) -> &Bot {
        self.bot.inner()
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
            let keyboard = build_inline_keyboard(buttons);
            req = req.reply_markup(keyboard);
        }

        req.await.map_err(|e| self.request_error("sendMessage", e))
    }

    /// Send a text message with optional inline buttons, falling back to plain text if parse mode fails.
    pub async fn send_text_with_fallback(
        &self,
        chat_id: i64,
        text: &str,
        reply_to: Option<i32>,
        thread_id: Option<i32>,
        buttons: &[Vec<InlineButton>],
    ) -> Result<teloxide::types::Message> {
        // Try with HTML first
        match self
            .send_text(
                chat_id,
                text,
                Some(TgParseMode::Html),
                reply_to,
                thread_id,
                buttons,
            )
            .await
        {
            Ok(msg) => Ok(msg),
            Err(_) => {
                // Fallback: strip HTML tags and send as plain text
                let plain = strip_html_tags(text);
                self.send_text(chat_id, &plain, None, reply_to, thread_id, buttons)
                    .await
            }
        }
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
            let body = crate::truncate_utf8(&redacted, 200);
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
            crate::truncate_utf8(&message, 1000)
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
    let cfg = crate::config::cached_config();
    validate_api_root_with_policy(api_root, cfg.ssrf.default_policy, &cfg.ssrf.trusted_hosts).await
}

async fn validate_api_root_with_policy(
    api_root: Option<&str>,
    policy: crate::security::ssrf::SsrfPolicy,
    trusted_hosts: &[String],
) -> Result<Option<reqwest::Url>> {
    let Some(parsed) = parse_api_root(api_root)? else {
        return Ok(None);
    };
    let checked = crate::security::ssrf::check_url(parsed.as_str(), policy, trusted_hosts)
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
    crate::logging::redact_sensitive(&without_token)
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
fn build_inline_keyboard(buttons: &[Vec<InlineButton>]) -> InlineKeyboardMarkup {
    let rows: Vec<Vec<InlineKeyboardButton>> = buttons
        .iter()
        .map(|row| {
            row.iter()
                .map(|b| {
                    if let Some(ref url) = b.url {
                        InlineKeyboardButton::url(
                            b.text.clone(),
                            url.parse()
                                .unwrap_or_else(|_| "https://example.com".parse().unwrap()),
                        )
                    } else {
                        let cb = b.callback_data.clone().unwrap_or_else(|| b.text.clone());
                        InlineKeyboardButton::callback(b.text.clone(), cb)
                    }
                })
                .collect()
        })
        .collect();
    InlineKeyboardMarkup::new(rows)
}

/// Strip HTML tags from text (simple implementation for fallback).
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{
        build_send_message_draft_body, parse_api_root, redact_telegram_error,
        send_message_draft_soft_degrades, validate_api_root_with_policy,
    };
    use crate::security::ssrf::SsrfPolicy;
    use reqwest::StatusCode;

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
