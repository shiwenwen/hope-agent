use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::channel::types::{ChatType, ChannelId, MsgContext};

use super::api::{
    GetUpdatesResponse, MessageItem, WeChatApi, WeChatMessage, DEFAULT_WECHAT_CDN_BASE_URL,
    MESSAGE_ITEM_TYPE_TEXT, MESSAGE_ITEM_TYPE_VOICE, MESSAGE_TYPE_BOT,
};
use super::media;
use super::WeChatSharedState;

const RETRY_DELAY: Duration = Duration::from_secs(2);
const BACKOFF_DELAY: Duration = Duration::from_secs(30);

pub(crate) async fn run_polling_loop(
    api: Arc<WeChatApi>,
    shared: Arc<WeChatSharedState>,
    account_id: String,
    inbound_tx: mpsc::Sender<MsgContext>,
    cancel: CancellationToken,
) {
    let mut next_timeout_ms = 35_000;
    let mut consecutive_failures = 0usize;
    let mut sync_buf = shared.load_sync_buf(&account_id).unwrap_or_default();

    app_info!(
        "channel",
        "wechat::polling",
        "Started WeChat polling for account '{}'",
        account_id
    );

    loop {
        let response = tokio::select! {
            _ = cancel.cancelled() => {
                app_info!(
                    "channel",
                    "wechat::polling",
                    "WeChat polling cancelled for account '{}'",
                    account_id
                );
                break;
            }
            result = api.get_updates(&sync_buf, Some(next_timeout_ms)) => result,
        };

        match response {
            Ok(resp) => {
                if let Some(timeout_ms) = resp.longpolling_timeout_ms.filter(|v| *v > 0) {
                    next_timeout_ms = timeout_ms.min(60_000);
                }

                if is_api_error(&resp) {
                    consecutive_failures += 1;
                    app_warn!(
                        "channel",
                        "wechat::polling",
                        "WeChat getUpdates failed for '{}' (ret={:?} errcode={:?} errmsg={:?})",
                        account_id,
                        resp.ret,
                        resp.errcode,
                        resp.errmsg
                    );

                    if should_stop_for_expired_session(&resp) {
                        app_warn!(
                            "channel",
                            "wechat::polling",
                            "WeChat session expired for '{}'; pausing API calls for 1 hour",
                            account_id
                        );
                        shared.pause_account(&account_id).await;
                        if sleep_or_cancel(&cancel, Duration::from_secs(3600)).await {
                            break;
                        }
                        shared.clear_pause(&account_id).await;
                        consecutive_failures = 0;
                        continue;
                    }

                    let delay = if consecutive_failures >= 3 {
                        consecutive_failures = 0;
                        BACKOFF_DELAY
                    } else {
                        RETRY_DELAY
                    };
                    if sleep_or_cancel(&cancel, delay).await {
                        break;
                    }
                    continue;
                }

                consecutive_failures = 0;

                if let Some(buf) = resp.get_updates_buf.as_deref().filter(|v| !v.is_empty()) {
                    sync_buf = buf.to_string();
                    if let Err(err) = shared.save_sync_buf(&account_id, &sync_buf) {
                        app_warn!(
                            "channel",
                            "wechat::polling",
                            "Failed to persist sync buffer for '{}': {}",
                            account_id,
                            err
                        );
                    }
                }

                for update in resp.msgs {
                    if update.message_type == Some(MESSAGE_TYPE_BOT) {
                        continue;
                    }

                    let Some(sender_id) = update.from_user_id.as_deref().filter(|v| !v.is_empty()) else {
                        continue;
                    };

                    if let Some(context_token) = update.context_token.as_deref().filter(|v| !v.is_empty()) {
                        if let Err(err) = shared.set_context_token(&account_id, sender_id, context_token).await {
                            app_warn!(
                                "channel",
                                "wechat::polling",
                                "Failed to cache context token for '{}': {}",
                                account_id,
                                err
                            );
                        }
                    }

                    // Clone item_list before moving update into convert_update
                    let item_list = update.item_list.clone();
                    let msg_id_str = update
                        .message_id
                        .map(|v| v.to_string())
                        .unwrap_or_default();

                    if let Some(mut msg) = convert_update(&account_id, update) {
                        // Extract inbound media from non-text items
                        for item in &item_list {
                            if item.item_type == MESSAGE_ITEM_TYPE_TEXT {
                                continue;
                            }
                            match media::download_inbound_media(
                                &msg_id_str,
                                item,
                                DEFAULT_WECHAT_CDN_BASE_URL,
                            )
                            .await
                            {
                                Ok(Some(inbound_media)) => {
                                    msg.media.push(inbound_media);
                                }
                                Ok(None) => {}
                                Err(err) => {
                                    app_warn!(
                                        "channel",
                                        "wechat::polling",
                                        "Failed to download inbound media for msg '{}': {}",
                                        msg_id_str,
                                        err
                                    );
                                }
                            }
                        }

                        if let Err(err) = inbound_tx.send(msg).await {
                            app_error!(
                                "channel",
                                "wechat::polling",
                                "Failed to forward WeChat inbound message: {}",
                                err
                            );
                            break;
                        }
                    }
                }
            }
            Err(err) => {
                consecutive_failures += 1;
                app_warn!(
                    "channel",
                    "wechat::polling",
                    "WeChat polling error for '{}': {}",
                    account_id,
                    err
                );
                let delay = if consecutive_failures >= 3 {
                    consecutive_failures = 0;
                    BACKOFF_DELAY
                } else {
                    RETRY_DELAY
                };
                if sleep_or_cancel(&cancel, delay).await {
                    break;
                }
            }
        }
    }
}

fn convert_update(account_id: &str, update: WeChatMessage) -> Option<MsgContext> {
    let sender_id = update.from_user_id.clone()?;
    let chat_id = sender_id.clone();
    let text = extract_body(&update.item_list).filter(|value| !value.is_empty());
    let message_id = update
        .message_id
        .map(|value| value.to_string())
        .or(update.client_id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let timestamp = update
        .create_time_ms
        .and_then(|millis| Utc.timestamp_millis_opt(millis).single())
        .unwrap_or_else(Utc::now);

    Some(MsgContext {
        channel_id: ChannelId::WeChat,
        account_id: account_id.to_string(),
        sender_id: sender_id.clone(),
        sender_name: None,
        sender_username: None,
        chat_id,
        chat_type: ChatType::Dm,
        chat_title: None,
        thread_id: None,
        message_id,
        text,
        media: Vec::new(),
        reply_to_message_id: None,
        timestamp,
        was_mentioned: false,
        raw: serde_json::to_value(update).unwrap_or(serde_json::Value::Null),
    })
}

fn extract_body(items: &[MessageItem]) -> Option<String> {
    for item in items {
        if item.item_type == MESSAGE_ITEM_TYPE_TEXT {
            if let Some(text) = item
                .text_item
                .as_ref()
                .and_then(|text_item| text_item.text.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(text.to_string());
            }
        }

        if item.item_type == MESSAGE_ITEM_TYPE_VOICE {
            if let Some(text) = item
                .voice_item
                .as_ref()
                .and_then(|voice_item| voice_item.text.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(text.to_string());
            }
        }

        if let Some(ref_msg) = item.ref_msg.as_ref().and_then(|ref_msg| ref_msg.message_item.as_deref()) {
            if let Some(text) = extract_body(std::slice::from_ref(ref_msg)) {
                return Some(text);
            }
        }
    }

    None
}

fn is_api_error(response: &GetUpdatesResponse) -> bool {
    response.ret.unwrap_or(0) != 0 || response.errcode.unwrap_or(0) != 0
}

fn should_stop_for_expired_session(response: &GetUpdatesResponse) -> bool {
    response.ret == Some(-14) || response.errcode == Some(-14)
}

async fn sleep_or_cancel(cancel: &CancellationToken, delay: Duration) -> bool {
    tokio::select! {
        _ = cancel.cancelled() => true,
        _ = tokio::time::sleep(delay) => false,
    }
}
