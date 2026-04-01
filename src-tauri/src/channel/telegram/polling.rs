use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::api::TelegramBotApi;
use crate::channel::types::*;

/// Run the Telegram long-polling loop.
///
/// Continuously calls `getUpdates` and converts each update into a `MsgContext`,
/// sending it to the inbound channel for processing by the worker.
pub async fn run_polling_loop(
    api: Arc<TelegramBotApi>,
    account_id: String,
    bot_id: i64,
    bot_username: String,
    inbound_tx: mpsc::Sender<MsgContext>,
    cancel: CancellationToken,
) {
    let mut offset: i32 = 0;
    let poll_timeout: u32 = 30; // seconds
    let mut consecutive_errors: u32 = 0;
    let max_backoff_secs: u64 = 30;

    app_info!(
        "channel",
        "telegram::polling",
        "Polling loop started for account '{}'",
        account_id
    );

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                app_info!("channel", "telegram::polling", "Polling cancelled for account '{}'", account_id);
                break;
            }
            result = tokio::time::timeout(
                std::time::Duration::from_secs(poll_timeout as u64 + 15),
                api.get_updates(offset, poll_timeout, &["message", "edited_message", "callback_query"])
            ) => {
                match result {
                    Err(_timeout) => {
                        app_warn!("channel", "telegram::polling", "Poll timed out for account '{}', reconnecting", account_id);
                        continue;
                    }
                    Ok(result) => match result {
                        Ok(updates) => {
                            consecutive_errors = 0;

                            for update in updates {
                                offset = update.id.0 as i32 + 1;

                                if let Some(msg_ctx) = convert_update(&api, &update, &account_id, bot_id, &bot_username).await {
                                    if let Err(e) = inbound_tx.send(msg_ctx).await {
                                        app_error!("channel", "telegram::polling", "Failed to send inbound message: {}", e);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            consecutive_errors += 1;
                            let backoff = std::cmp::min(
                                (2u64.pow(consecutive_errors.min(5))) as u64,
                                max_backoff_secs,
                            );
                            // Log first 3 errors as warn, then only every 10th to avoid spam
                            if consecutive_errors <= 3 || consecutive_errors % 10 == 0 {
                                app_warn!("channel", "telegram::polling",
                                    "Poll error (attempt {}): {}. Retrying in {}s",
                                    consecutive_errors, e, backoff);
                            } else {
                                app_debug!("channel", "telegram::polling",
                                    "Poll error (attempt {}): {}. Retrying in {}s",
                                    consecutive_errors, e, backoff);
                            }

                            tokio::select! {
                                _ = cancel.cancelled() => break,
                                _ = tokio::time::sleep(std::time::Duration::from_secs(backoff)) => {}
                            }
                        }
                    }
                }
            }
        }
    }

    app_info!(
        "channel",
        "telegram::polling",
        "Polling loop stopped for account '{}'",
        account_id
    );
}

/// Convert a teloxide Update into our MsgContext.
/// Returns None if the update doesn't contain a processable message.
async fn convert_update(
    api: &TelegramBotApi,
    update: &teloxide::types::Update,
    account_id: &str,
    bot_id: i64,
    bot_username: &str,
) -> Option<MsgContext> {
    use teloxide::types::UpdateKind;

    match &update.kind {
        UpdateKind::Message(msg) => {
            convert_message(api, msg, account_id, bot_id, bot_username).await
        }
        UpdateKind::EditedMessage(msg) => {
            convert_message(api, msg, account_id, bot_id, bot_username).await
        }
        UpdateKind::CallbackQuery(cb) => convert_callback_query(cb, account_id),
        _ => None,
    }
}

/// Convert a teloxide Message into our MsgContext.
async fn convert_message(
    api: &TelegramBotApi,
    msg: &teloxide::types::Message,
    account_id: &str,
    bot_id: i64,
    bot_username: &str,
) -> Option<MsgContext> {
    // Extract sender info
    let from = msg.from.as_ref()?;

    // Skip messages from the bot itself
    if from.id.0 as i64 == bot_id {
        return None;
    }

    // Determine chat type
    let chat_type = match msg.chat.kind {
        teloxide::types::ChatKind::Private(_) => ChatType::Dm,
        teloxide::types::ChatKind::Public(ref public) => match public.kind {
            teloxide::types::PublicChatKind::Supergroup(ref sg) => {
                if sg.is_forum {
                    ChatType::Forum
                } else {
                    ChatType::Group
                }
            }
            teloxide::types::PublicChatKind::Group => ChatType::Group,
            teloxide::types::PublicChatKind::Channel(_) => ChatType::Channel,
        },
    };

    // Check if bot was mentioned or replied to (for groups).
    // Instead of filtering here, we pass the flag downstream so the worker
    // can decide based on per-group `requireMention` configuration.
    let was_mentioned = match chat_type {
        ChatType::Dm => true, // DMs are always "addressed"
        ChatType::Group | ChatType::Forum | ChatType::Channel => {
            is_bot_addressed(msg, bot_id, bot_username)
        }
    };

    // Extract text
    let text = msg.text().map(|t| t.to_string());

    // Extract media
    let mut media = Vec::new();
    if let Some(photos) = msg.photo() {
        if let Some(best) = photos.iter().max_by_key(|p| p.width * p.height) {
            let file_id = best.file.id.to_string();
            let file_path = download_inbound_media_to_temp(api, &file_id, "photo", "jpg").await;
            media.push(InboundMedia {
                media_type: MediaType::Photo,
                file_id,
                file_url: file_path,
                mime_type: Some("image/jpeg".to_string()),
                file_size: Some(best.file.size as u64),
                caption: msg.caption().map(|c| c.to_string()),
            });
        }
    }
    if let Some(doc) = msg.document() {
        let file_id = doc.file.id.to_string();
        let ext = doc
            .file_name
            .as_deref()
            .and_then(|n| std::path::Path::new(n).extension().and_then(|e| e.to_str()))
            .unwrap_or("bin");
        let file_path = download_inbound_media_to_temp(api, &file_id, "document", ext).await;
        media.push(InboundMedia {
            media_type: MediaType::Document,
            file_id,
            file_url: file_path,
            mime_type: doc.mime_type.as_ref().map(|m| m.to_string()),
            file_size: Some(doc.file.size as u64),
            caption: msg.caption().map(|c| c.to_string()),
        });
    }

    // Skip if no text and no media
    if text.is_none() && media.is_empty() {
        return None;
    }

    // Thread ID for forum topics
    let thread_id = msg.thread_id.map(|tid| tid.to_string());

    // Reply-to message ID
    let reply_to = msg.reply_to_message().map(|r| r.id.0.to_string());

    let sender_name = {
        let mut name = from.first_name.clone();
        if let Some(ref last) = from.last_name {
            name.push(' ');
            name.push_str(last);
        }
        name
    };

    Some(MsgContext {
        channel_id: ChannelId::Telegram,
        account_id: account_id.to_string(),
        sender_id: from.id.0.to_string(),
        sender_name: Some(sender_name),
        sender_username: from.username.clone(),
        chat_id: msg.chat.id.0.to_string(),
        chat_type,
        chat_title: msg.chat.title().map(|t| t.to_string()),
        thread_id,
        message_id: msg.id.0.to_string(),
        text,
        media,
        reply_to_message_id: reply_to,
        timestamp: msg.date,
        was_mentioned,
        raw: serde_json::json!({ "update_id": 0 }), // minimal raw payload
    })
}

async fn download_inbound_media_to_temp(
    api: &TelegramBotApi,
    file_id: &str,
    prefix: &str,
    ext: &str,
) -> Option<String> {
    let dir = match crate::paths::channel_dir("telegram") {
        Ok(d) => d.join("inbound-temp"),
        Err(err) => {
            app_warn!(
                "channel",
                "telegram::polling",
                "Failed to resolve telegram inbound temp dir: {}",
                err
            );
            return None;
        }
    };
    let safe_id = file_id.replace(['/', '\\', ':'], "_");
    let safe_ext = ext.trim_start_matches('.');
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = dir.join(format!("{}-{}-{}.{}", ts, safe_id, prefix, safe_ext));
    match api.download_file_to_path(file_id, &path).await {
        Ok(_) => Some(path.to_string_lossy().to_string()),
        Err(err) => {
            app_warn!(
                "channel",
                "telegram::polling",
                "Failed to download inbound media '{}': {}",
                file_id,
                err
            );
            None
        }
    }
}

/// Convert a Telegram CallbackQuery (inline button click) into a MsgContext.
///
/// Callback data with format "slash:<command> <arg>" is converted to "/<command> <arg>"
/// so the worker processes it as a normal slash command.
fn convert_callback_query(
    cb: &teloxide::types::CallbackQuery,
    account_id: &str,
) -> Option<MsgContext> {
    let data = cb.data.as_ref()?;
    let msg = cb.message.as_ref()?.regular_message()?;

    // Convert "slash:think high" → "/think high"
    let text = if let Some(rest) = data.strip_prefix("slash:") {
        format!("/{}", rest)
    } else {
        return None; // Unknown callback format, ignore
    };

    let from = &cb.from;

    let chat_type = match msg.chat.kind {
        teloxide::types::ChatKind::Private(_) => ChatType::Dm,
        teloxide::types::ChatKind::Public(ref public) => match public.kind {
            teloxide::types::PublicChatKind::Supergroup(ref sg) => {
                if sg.is_forum {
                    ChatType::Forum
                } else {
                    ChatType::Group
                }
            }
            teloxide::types::PublicChatKind::Group => ChatType::Group,
            teloxide::types::PublicChatKind::Channel(_) => ChatType::Channel,
        },
    };

    let thread_id = msg.thread_id.map(|tid| tid.to_string());

    let sender_name = {
        let mut name = from.first_name.clone();
        if let Some(ref last) = from.last_name {
            name.push(' ');
            name.push_str(last);
        }
        name
    };

    Some(MsgContext {
        channel_id: ChannelId::Telegram,
        account_id: account_id.to_string(),
        sender_id: from.id.0.to_string(),
        sender_name: Some(sender_name),
        sender_username: from.username.clone(),
        chat_id: msg.chat.id.0.to_string(),
        chat_type,
        chat_title: msg.chat.title().map(|t| t.to_string()),
        thread_id,
        message_id: msg.id.0.to_string(),
        text: Some(text),
        media: Vec::new(),
        reply_to_message_id: None,
        timestamp: msg.date,
        was_mentioned: true,
        raw: serde_json::json!({ "callback_query_id": cb.id }),
    })
}

/// Check if the bot is addressed in a group message.
///
/// Returns true if:
/// - The message is a reply to the bot's message
/// - The message text contains @bot_username
/// - The message text starts with a / command
fn is_bot_addressed(msg: &teloxide::types::Message, bot_id: i64, bot_username: &str) -> bool {
    // Reply to bot's message
    if let Some(reply) = msg.reply_to_message() {
        if let Some(from) = reply.from.as_ref() {
            if from.id.0 as i64 == bot_id {
                return true;
            }
        }
    }

    // @mention in text
    if let Some(text) = msg.text() {
        let mention = format!("@{}", bot_username);
        if text.contains(&mention) {
            return true;
        }
        // Also check entities for bot_command type
        if text.starts_with('/') {
            return true;
        }
    }

    // Check for mention entities
    if let Some(entities) = msg.entities() {
        for entity in entities {
            if let teloxide::types::MessageEntityKind::Mention = entity.kind {
                if let Some(text) = msg.text() {
                    // Safe UTF-8 extraction: use char boundaries instead of byte offsets
                    let mention_text: String = text
                        .chars()
                        .skip(entity.offset)
                        .take(entity.length)
                        .collect();
                    if mention_text.eq_ignore_ascii_case(&format!("@{}", bot_username)) {
                        return true;
                    }
                }
            }
        }
    }

    false
}
