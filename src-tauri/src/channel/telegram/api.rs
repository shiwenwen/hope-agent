use anyhow::Result;
use teloxide::prelude::*;
use teloxide::types::{
    ChatAction, ChatId, InputFile, Me, MessageId, ParseMode as TgParseMode,
    ReplyParameters, ThreadId,
};

/// Thin wrapper around teloxide's `Bot` to isolate framework details.
pub struct TelegramBotApi {
    bot: Bot,
    /// Stored proxy URL for raw HTTP requests (sendMessageDraft etc.)
    proxy_url: Option<String>,
}

impl TelegramBotApi {
    /// Create a new Telegram Bot API client.
    ///
    /// If `proxy_url` is provided, it's set via `HTTPS_PROXY` env before creating
    /// the client (teloxide reads proxy from env via `client_from_env()`).
    pub fn new(token: &str, proxy_url: Option<&str>, _api_root: Option<&str>) -> Self {
        // Teloxide's Bot::new() uses `client_from_env()` which reads HTTPS_PROXY.
        // For a per-account proxy, we temporarily set the env var.
        // This is safe because bot creation is synchronous.
        let bot = if let Some(proxy) = proxy_url {
            let prev = std::env::var("HTTPS_PROXY").ok();
            std::env::set_var("HTTPS_PROXY", proxy);
            let bot = Bot::new(token);
            // Restore previous value
            match prev {
                Some(val) => std::env::set_var("HTTPS_PROXY", val),
                None => std::env::remove_var("HTTPS_PROXY"),
            }
            bot
        } else {
            Bot::new(token)
        };

        Self { bot, proxy_url: proxy_url.map(|s| s.to_string()) }
    }

    /// Get the underlying teloxide Bot reference.
    pub fn bot(&self) -> &Bot {
        &self.bot
    }

    /// Verify the bot token and return bot info.
    pub async fn get_me(&self) -> Result<Me> {
        self.bot.get_me().await.map_err(|e| anyhow::anyhow!("getMe failed: {}", e))
    }

    /// Send a text message.
    pub async fn send_text(
        &self,
        chat_id: i64,
        text: &str,
        parse_mode: Option<TgParseMode>,
        reply_to: Option<i32>,
        thread_id: Option<i32>,
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

        req.await.map_err(|e| anyhow::anyhow!("sendMessage failed: {}", e))
    }

    /// Send a text message, falling back to plain text if parse mode fails.
    pub async fn send_text_with_fallback(
        &self,
        chat_id: i64,
        text: &str,
        reply_to: Option<i32>,
        thread_id: Option<i32>,
    ) -> Result<teloxide::types::Message> {
        // Try with HTML first
        match self.send_text(chat_id, text, Some(TgParseMode::Html), reply_to, thread_id).await {
            Ok(msg) => Ok(msg),
            Err(_) => {
                // Fallback: strip HTML tags and send as plain text
                let plain = strip_html_tags(text);
                self.send_text(chat_id, &plain, None, reply_to, thread_id).await
            }
        }
    }

    /// Send a typing indicator (chat action).
    pub async fn send_typing(&self, chat_id: i64) -> Result<()> {
        self.bot
            .send_chat_action(ChatId(chat_id), ChatAction::Typing)
            .await
            .map_err(|e| anyhow::anyhow!("sendChatAction failed: {}", e))?;
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
        let mut req = self.bot.edit_message_text(ChatId(chat_id), MessageId(message_id), text);
        if let Some(pm) = parse_mode {
            req = req.parse_mode(pm);
        }
        req.await.map_err(|e| anyhow::anyhow!("editMessageText failed: {}", e))?;
        Ok(())
    }

    /// Delete a message.
    pub async fn delete_message(&self, chat_id: i64, message_id: i32) -> Result<()> {
        self.bot
            .delete_message(ChatId(chat_id), MessageId(message_id))
            .await
            .map_err(|e| anyhow::anyhow!("deleteMessage failed: {}", e))?;
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
        reply_to: Option<i32>,
        thread_id: Option<i32>,
    ) -> Result<()> {
        let token = self.bot.token();
        // Use the bot's API URL base (respects custom apiRoot)
        let api_url_owned = self.bot.api_url();
        let api_url = api_url_owned.as_str().trim_end_matches('/');
        let url = format!("{}/bot{}/sendMessageDraft", api_url, token);

        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "draft_id": draft_id,
        });

        if let Some(reply_id) = reply_to {
            body["reply_parameters"] = serde_json::json!({
                "message_id": reply_id,
            });
        }
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }

        // Build reqwest client with proxy if configured (same proxy as the Bot)
        let client = if let Some(ref proxy) = self.proxy_url {
            reqwest::Client::builder()
                .proxy(reqwest::Proxy::all(proxy)
                    .map_err(|e| anyhow::anyhow!("Invalid proxy URL: {}", e))?)
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?
        } else {
            reqwest::Client::new()
        };
        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("sendMessageDraft request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("sendMessageDraft failed ({}): {}", status, crate::truncate_utf8(&text, 200));
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

        req.await.map_err(|e| anyhow::anyhow!("getUpdates failed: {}", e))
    }

    /// Download a file by file_id (returns the file path on Telegram servers).
    pub async fn get_file(&self, file_id: &str) -> Result<teloxide::types::File> {
        use teloxide::types::FileId;
        self.bot
            .get_file(FileId(file_id.to_string()))
            .await
            .map_err(|e| anyhow::anyhow!("getFile failed: {}", e))
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
        req.await.map_err(|e| anyhow::anyhow!("sendPhoto failed: {}", e))
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
        req.await.map_err(|e| anyhow::anyhow!("sendDocument failed: {}", e))
    }
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
