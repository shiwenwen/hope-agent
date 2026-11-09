use anyhow::Result;
use ha_core::personal_assistant_im::{
    AssistantImError, TelegramInput, TelegramInputKind, ACTIVATE_PREFIX, MAX_MESSAGE_BYTES,
    MAX_REPLY_CHARS, PAIR_COMMAND,
};
use teloxide::prelude::*;
use teloxide::types::{
    ChatId, ChatKind, InlineKeyboardButton, InlineKeyboardMarkup, LinkPreviewOptions, Message,
    MessageKind, Update, UpdateKind,
};

use crate::channel::telegram::api::TelegramBotApi;

/// Reuses the platform SDK and HTTP boundary, but has no ordinary account,
/// command parser, attachment processor, callback resolver or shared bus.
pub(super) struct TelegramClient {
    api: TelegramBotApi,
    pub bot_id: i64,
    pub username: String,
}

impl TelegramClient {
    pub async fn connect(token: &str) -> Result<Self> {
        let proxy = ha_core::provider::active_custom_proxy_url();
        Self::connect_with_proxy(token, proxy.as_deref()).await
    }

    pub async fn connect_with_proxy(token: &str, proxy: Option<&str>) -> Result<Self> {
        // Identity verification always uses the canonical platform namespace.
        // A custom API root cannot create a second identity for the same bot.
        let api = TelegramBotApi::new(token, proxy, None)
            .await
            .map_err(|_| AssistantImError::Unavailable)?;
        let me = api
            .get_me()
            .await
            .map_err(|_| AssistantImError::Unavailable)?;
        let bot_id = i64::try_from(me.id.0).map_err(|_| AssistantImError::InvalidInput)?;
        let username = me.username().to_owned();
        Ok(Self {
            api,
            bot_id,
            username,
        })
    }

    pub async fn updates(&self, offset: i64) -> Result<Vec<Update>> {
        let offset = i32::try_from(offset).map_err(|_| AssistantImError::InvalidInput)?;
        self.api
            .get_updates(offset, 20, &["message", "callback_query"])
            .await
            .map_err(|_| AssistantImError::Unavailable.into())
    }

    pub async fn acknowledge_activation(&self, update: &Update) {
        if let UpdateKind::CallbackQuery(callback) = &update.kind {
            let _ = self
                .api
                .answer_callback_query(&callback.id.0, Some("已启用个人助手"))
                .await;
        }
    }

    /// The raw Bot has no throttle queue or application retry. All destination
    /// scheduling and the non-replay marker precede this one mutation.
    pub async fn send(&self, chat_id: i64, text: &str, activation: Option<&str>) -> Result<i64> {
        if chat_id <= 0 || text.is_empty() || text.chars().count() > MAX_REPLY_CHARS {
            return Err(AssistantImError::InvalidInput.into());
        }
        let mut request = self
            .api
            .bot()
            .send_message(ChatId(chat_id), text)
            .link_preview_options(LinkPreviewOptions {
                is_disabled: true,
                url: None,
                prefer_small_media: false,
                prefer_large_media: false,
                show_above_text: false,
            });
        if let Some(callback) = activation {
            if !callback.starts_with(ACTIVATE_PREFIX) || callback.len() > 64 {
                return Err(AssistantImError::InvalidInput.into());
            }
            request = request.reply_markup(InlineKeyboardMarkup::new(vec![vec![
                InlineKeyboardButton::callback("启用", callback),
            ]]));
        }
        let message = request.await.map_err(|_| AssistantImError::Unavailable)?;
        Ok(i64::from(message.id.0))
    }
}

fn private_message(message: &Message) -> bool {
    matches!(message.chat.kind, ChatKind::Private(_))
        && message.thread_id.is_none()
        && message.sender_chat.is_none()
        && message.via_bot.is_none()
        && message.sender_business_bot.is_none()
        && matches!(&message.kind, MessageKind::Common(common)
            if common.business_connection_id.is_none() && !common.is_from_offline)
        && message.forward_origin().is_none()
}

/// Invalid/unsupported updates still return their event ID so the ledger can
/// durably consume them without retaining their body or touching the model.
pub(super) fn decode(update: &Update, bot_id: i64) -> TelegramInput {
    let mut result = TelegramInput {
        update_id: i64::from(update.id.0),
        sent_at: None,
        user_id: 0,
        chat_id: 0,
        kind: TelegramInputKind::Ignore,
    };
    match &update.kind {
        UpdateKind::Message(message) if private_message(message) => {
            let Some(sender) = message.from.as_ref().filter(|sender| !sender.is_bot) else {
                return result;
            };
            let Some(text) = message
                .text()
                .filter(|text| text.len() <= MAX_MESSAGE_BYTES)
            else {
                return result;
            };
            let Ok(user_id) = i64::try_from(sender.id.0) else {
                return result;
            };
            result.user_id = user_id;
            result.sent_at = Some(message.date.timestamp());
            result.chat_id = message.chat.id.0;
            result.kind = match text.strip_prefix(PAIR_COMMAND) {
                Some(code) => TelegramInputKind::PairCode(code.trim().to_owned()),
                None if text.starts_with("/ha_pair") => TelegramInputKind::Ignore,
                None => TelegramInputKind::Message(text.to_owned()),
            };
        }
        UpdateKind::CallbackQuery(callback) if !callback.from.is_bot => {
            let Some(message) = callback
                .message
                .as_ref()
                .and_then(|message| message.regular_message())
            else {
                return result;
            };
            let own_message = message.from.as_ref().is_some_and(|sender| {
                sender.is_bot && i64::try_from(sender.id.0).ok() == Some(bot_id)
            });
            if !private_message(message) || !own_message {
                return result;
            }
            let Some(token) = callback
                .data
                .as_deref()
                .and_then(|data| data.strip_prefix(ACTIVATE_PREFIX))
            else {
                return result;
            };
            let Ok(user_id) = i64::try_from(callback.from.id.0) else {
                return result;
            };
            result.user_id = user_id;
            result.chat_id = message.chat.id.0;
            result.kind = TelegramInputKind::Activate(token.to_owned());
        }
        // Edited messages and ordinary IM callback payloads never dispatch.
        _ => {}
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn wire_update(value: Value) -> Update {
        // Match teloxide's HTTP response parser. UpdateKind's borrowed-key
        // visitor falls back to Error for serde_json::from_value owned maps.
        let update: Update = serde_json::from_str(&value.to_string()).unwrap();
        assert!(
            !matches!(&update.kind, UpdateKind::Error(_)),
            "fixture must parse as a platform update"
        );
        update
    }

    fn private_update() -> Value {
        json!({"update_id": 20, "message": {
            "message_id": 8, "date": 1700000000,
            "chat": {"id": 42, "type": "private", "first_name": "Owner"},
            "from": {"id": 42, "is_bot": false, "first_name": "Owner"},
            "text": "check progress"
        }})
    }

    #[test]
    fn raw_boundary_rejects_groups_edits_bots_and_forwards() {
        let direct = wire_update(private_update());
        assert!(matches!(
            decode(&direct, 99).kind,
            TelegramInputKind::Message(_)
        ));
        let mut invalid = Vec::new();
        let mut group = private_update();
        group["message"]["chat"] = json!({"id": -7, "type": "group", "title": "Team"});
        invalid.push(group);
        let mut bot = private_update();
        bot["message"]["from"]["is_bot"] = json!(true);
        invalid.push(bot);
        let mut forwarded = private_update();
        forwarded["message"]["forward_origin"] = json!({
            "type": "hidden_user", "date": 1700000000, "sender_user_name": "Owner"
        });
        invalid.push(forwarded);
        let mut edited = private_update();
        edited["edited_message"] = edited.as_object_mut().unwrap().remove("message").unwrap();
        invalid.push(edited);
        for raw in invalid {
            let update = wire_update(raw);
            assert!(matches!(
                decode(&update, 99).kind,
                TelegramInputKind::Ignore
            ));
        }
    }

    #[test]
    fn ordinary_approval_callback_has_no_personal_assistant_action() {
        let mut payload = json!({
            "update_id": 21,
            "callback_query": {
                "id": "callback", "chat_instance": "chat-instance", "data": "approve:old",
                "from": {"id": 42, "is_bot": false, "first_name": "Owner"},
                "message": {
                    "message_id": 9, "date": 1700000000,
                    "chat": {"id": 42, "type": "private", "first_name": "Owner"},
                    "from": {"id": 99, "is_bot": true, "first_name": "Bot"},
                    "text": "old command"
                }
            }
        });
        let callback = wire_update(payload.clone());
        assert!(matches!(&callback.kind, UpdateKind::CallbackQuery(_)));
        assert!(matches!(
            decode(&callback, 99).kind,
            TelegramInputKind::Ignore
        ));
        payload["callback_query"]["data"] = json!(format!("{ACTIVATE_PREFIX}test-nonce"));
        let activation = wire_update(payload);
        assert!(
            matches!(decode(&activation,99).kind,TelegramInputKind::Activate(value) if value == "test-nonce")
        );
        assert!(matches!(
            decode(&activation, 100).kind,
            TelegramInputKind::Ignore
        ));
    }
}
