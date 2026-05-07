//! Feishu / Lark channel (飞书 / Lark Suite).
//!
//! - **Official API**: <https://open.feishu.cn/document/> (cn) /
//!   <https://open.larksuite.com/document/> (intl)
//! - **SDK / Reference**: <https://github.com/larksuite/oapi-sdk-nodejs>
//!   (官方 Node SDK，长连接帧协议 + 鉴权刷新参考实现)
//! - **Protocol**: WebSocket 事件订阅（pbbp2 protobuf 帧）+ REST
//!   `/open-apis/im/v1/messages` + `tenant_access_token` (TTL 7200s)
//! - **Last reviewed**: 2026-05-05

pub mod api;
pub mod auth;
pub mod data_cache;
pub mod format;
pub mod media;
pub mod proto;
pub mod ws_event;

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::channel::traits::ChannelPlugin;
use crate::channel::types::*;

use self::api::FeishuApi;
use self::auth::FeishuAuth;

/// Running account state for a single Feishu bot.
struct RunningAccount {
    api: Arc<FeishuApi>,
    // Diagnostics-only — retained for future filtering of bot-authored events.
    #[allow(dead_code)]
    bot_name: String,
    #[allow(dead_code)]
    bot_open_id: String,
}

/// Key under which we wrap the `callback_id` string inside button
/// `behaviors[callback].value`. Schema 2.0 requires `value` to be an object
/// with string key/value, so we round-trip our id through this single key —
/// shared with [`ws_event::extract_hope_callback`].
pub(super) const HOPE_CALLBACK_KEY: &str = "hope_callback";

/// Build a schema-2.0 interactive card body for ask_user / approval buttons.
fn build_button_card_v2(
    text: Option<&str>,
    buttons: &[Vec<crate::channel::types::InlineButton>],
) -> serde_json::Value {
    let columns: Vec<_> = buttons
        .iter()
        .flatten()
        .map(|b| {
            serde_json::json!({
                "tag": "column",
                "width": "auto",
                "elements": [{
                    "tag": "button",
                    "text": {"tag": "plain_text", "content": &b.text},
                    "type": "primary",
                    "behaviors": [{
                        "type": "callback",
                        "value": {HOPE_CALLBACK_KEY: b.callback_id()},
                    }],
                }],
            })
        })
        .collect();

    let mut body_elements: Vec<serde_json::Value> = Vec::new();
    if let Some(t) = text.filter(|s| !s.is_empty()) {
        body_elements.push(serde_json::json!({
            "tag": "markdown",
            "content": t,
        }));
    }
    body_elements.push(serde_json::json!({
        "tag": "column_set",
        "horizontal_align": "left",
        "columns": columns,
    }));

    serde_json::json!({
        "schema": "2.0",
        "config": {"streaming_mode": false},
        "body": {"elements": body_elements},
    })
}

/// Feishu (飞书) / Lark channel plugin implementation.
pub struct FeishuPlugin {
    accounts: Mutex<HashMap<String, RunningAccount>>,
}

impl FeishuPlugin {
    pub fn new() -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
        }
    }

    /// Extract credentials from the JSON config blob.
    fn extract_credentials(credentials: &serde_json::Value) -> Result<(String, String, String)> {
        let app_id = credentials
            .get("appId")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'appId' in Feishu credentials"))?;

        let app_secret = credentials
            .get("appSecret")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'appSecret' in Feishu credentials"))?;

        let domain = credentials
            .get("domain")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "feishu".to_string());

        Ok((app_id, app_secret, domain))
    }

    /// Get the API for a running account.
    async fn get_account(&self, account_id: &str) -> Result<Arc<FeishuApi>> {
        let accounts = self.accounts.lock().await;
        accounts
            .get(account_id)
            .map(|a| a.api.clone())
            .ok_or_else(|| anyhow::anyhow!("Feishu account '{}' is not running", account_id))
    }
}

#[async_trait]
impl ChannelPlugin for FeishuPlugin {
    fn meta(&self) -> ChannelMeta {
        ChannelMeta {
            id: ChannelId::Feishu,
            display_name: "Feishu / Lark".to_string(),
            description: "Feishu (飞书) / Lark Bot".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            chat_types: vec![ChatType::Dm, ChatType::Group],
            supports_polls: false,
            supports_reactions: false,
            supports_draft: false,
            supports_edit: true,
            supports_unsend: true,
            supports_reply: true,
            supports_threads: false,
            supports_media: vec![
                MediaType::Photo,
                MediaType::Video,
                MediaType::Audio,
                MediaType::Document,
            ],
            supports_typing: false,
            supports_buttons: true,
            max_message_length: Some(4096),
            supports_card_stream: true,
        }
    }

    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound_tx: mpsc::Sender<MsgContext>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let (app_id, app_secret, domain) = Self::extract_credentials(&account.credentials)?;

        let auth = Arc::new(FeishuAuth::new(&app_id, &app_secret, &domain));
        let api = Arc::new(FeishuApi::new(auth));

        // Validate by fetching bot info
        let bot_info = api.get_bot_info().await?;
        let bot_name = bot_info.app_name.clone();
        let bot_open_id = bot_info.open_id.clone();

        app_info!(
            "channel",
            "feishu",
            "Bot authenticated: {} (open_id={})",
            bot_name,
            bot_open_id
        );

        // Store running account state
        {
            let mut accounts = self.accounts.lock().await;
            accounts.insert(
                account.id.clone(),
                RunningAccount {
                    api: api.clone(),
                    bot_name: bot_name.clone(),
                    bot_open_id: bot_open_id.clone(),
                },
            );
        }

        // Spawn the gateway event loop
        let account_id = account.id.clone();
        tokio::spawn(ws_event::run_feishu_gateway(
            api,
            account_id,
            bot_open_id,
            inbound_tx,
            cancel,
        ));

        Ok(())
    }

    async fn stop_account(&self, account_id: &str) -> Result<()> {
        let mut accounts = self.accounts.lock().await;
        accounts.remove(account_id);
        Ok(())
    }

    async fn send_message(
        &self,
        account_id: &str,
        chat_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        let api = self.get_account(account_id).await?;

        // Dispatcher 一般每次只塞一个 media（[`partition_media_by_channel`]），
        // 这里仍循环以便未来 dispatcher 改批量时不需要再改插件层。
        if !payload.media.is_empty() {
            let reply_to = payload.reply_to_message_id.as_deref();
            let mut last_id = String::from("no_content");
            for m in &payload.media {
                last_id = media::send_outbound_media(&api, chat_id, m, reply_to).await?;
            }
            if payload.text.is_none() && payload.buttons.is_empty() {
                return Ok(DeliveryResult::ok(last_id));
            }
        }

        if !payload.buttons.is_empty() {
            let card = build_button_card_v2(payload.text.as_deref(), &payload.buttons);
            let reply_to = payload.reply_to_message_id.as_deref();
            let msg_id = api.send_interactive_card(chat_id, card, reply_to).await?;
            return Ok(DeliveryResult::ok(msg_id));
        }

        if let Some(ref text) = payload.text {
            if text.is_empty() {
                return Ok(DeliveryResult::ok("empty"));
            }

            let reply_to = payload.reply_to_message_id.as_deref();
            let message_id = api.send_message(chat_id, text, reply_to).await?;
            return Ok(DeliveryResult::ok(message_id));
        }

        Ok(DeliveryResult::ok("no_content"))
    }

    async fn send_typing(&self, _account_id: &str, _chat_id: &str) -> Result<()> {
        // Feishu does not support typing indicators
        Ok(())
    }

    async fn edit_message(
        &self,
        account_id: &str,
        _chat_id: &str,
        message_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        let api = self.get_account(account_id).await?;

        if let Some(ref text) = payload.text {
            api.update_message(message_id, text).await?;
        }

        Ok(DeliveryResult::ok(message_id.to_string()))
    }

    async fn delete_message(
        &self,
        account_id: &str,
        _chat_id: &str,
        message_id: &str,
    ) -> Result<()> {
        let api = self.get_account(account_id).await?;
        api.delete_message(message_id).await
    }

    async fn create_card_stream(
        &self,
        account_id: &str,
        initial_text: &str,
    ) -> Result<CardStreamHandle> {
        let api = self.get_account(account_id).await?;
        let card_id = api.create_streaming_card(initial_text).await?;
        Ok(CardStreamHandle {
            card_id,
            element_id: api::STREAMING_ELEMENT_ID.to_string(),
        })
    }

    async fn send_card_message(
        &self,
        account_id: &str,
        chat_id: &str,
        card_id: &str,
        reply_to_message_id: Option<&str>,
        _thread_id: Option<&str>,
    ) -> Result<DeliveryResult> {
        let api = self.get_account(account_id).await?;
        let msg_id = api
            .send_card_reference(chat_id, card_id, reply_to_message_id)
            .await?;
        Ok(DeliveryResult::ok(msg_id))
    }

    async fn update_card_element(
        &self,
        account_id: &str,
        card_id: &str,
        element_id: &str,
        content: &str,
        sequence: i64,
    ) -> std::result::Result<(), CardStreamError> {
        let api = self
            .get_account(account_id)
            .await
            .map_err(|e| CardStreamError::Other(e.to_string()))?;
        api.update_card_element(card_id, element_id, content, sequence)
            .await
    }

    async fn close_card_stream(
        &self,
        account_id: &str,
        card_id: &str,
        sequence: i64,
    ) -> Result<()> {
        let api = self.get_account(account_id).await?;
        api.close_card_streaming(card_id, sequence).await
    }

    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth> {
        let (app_id, app_secret, domain) = Self::extract_credentials(&account.credentials)?;
        let auth = Arc::new(FeishuAuth::new(&app_id, &app_secret, &domain));
        let api = FeishuApi::new(auth);

        match api.get_bot_info().await {
            Ok(info) => Ok(ChannelHealth {
                is_running: false,
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(true),
                error: None,
                uptime_secs: None,
                bot_name: Some(info.app_name),
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
                DmPolicy::Allowlist | DmPolicy::Pairing => {
                    security.user_allowlist.contains(&msg.sender_id)
                        || security.admin_ids.contains(&msg.sender_id)
                }
            },
            ChatType::Group => {
                // Group policy: disabled → deny all
                if security.group_policy == GroupPolicy::Disabled {
                    return false;
                }

                // Allowlist mode: group must be in allowlist
                if security.group_policy == GroupPolicy::Allowlist {
                    if security.groups.is_empty() {
                        if !security.group_allowlist.is_empty()
                            && !security.group_allowlist.contains(&msg.chat_id)
                        {
                            return false;
                        }
                    } else {
                        let has_config = security.groups.contains_key(&msg.chat_id)
                            || security.groups.contains_key("*");
                        if !has_config {
                            return false;
                        }
                    }
                }

                // Legacy group_allowlist backward compat
                if !security.group_allowlist.is_empty()
                    && security.groups.is_empty()
                    && !security.group_allowlist.contains(&msg.chat_id)
                {
                    return false;
                }

                // Account-level user allowlist
                if !security.user_allowlist.is_empty()
                    && !security.user_allowlist.contains(&msg.sender_id)
                    && !security.admin_ids.contains(&msg.sender_id)
                {
                    return false;
                }

                true
            }
            // Feishu doesn't have Forum/Channel chat types
            _ => false,
        }
    }

    fn markdown_to_native(&self, markdown: &str) -> String {
        format::markdown_to_feishu_text(markdown)
    }

    fn chunk_message(&self, text: &str) -> Vec<String> {
        crate::channel::traits::chunk_text(text, 4096)
    }

    async fn validate_credentials(&self, credentials: &serde_json::Value) -> Result<String> {
        let (app_id, app_secret, domain) = Self::extract_credentials(credentials)?;
        let auth = Arc::new(FeishuAuth::new(&app_id, &app_secret, &domain));
        let api = FeishuApi::new(auth);
        let info = api.get_bot_info().await?;
        Ok(info.app_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::types::InlineButton;

    #[test]
    fn button_card_is_schema_2_with_behaviors_callback() {
        let buttons = vec![vec![
            InlineButton {
                text: "✅ Allow Once".to_string(),
                callback_data: Some("approval:abc:allow_once".to_string()),
                url: None,
            },
            InlineButton {
                text: "❌ Deny".to_string(),
                callback_data: Some("approval:abc:deny".to_string()),
                url: None,
            },
        ]];

        let card = build_button_card_v2(Some("approve?"), &buttons);

        assert_eq!(card["schema"], "2.0");
        assert_eq!(card["config"]["streaming_mode"], false);

        let elements = card["body"]["elements"].as_array().unwrap();
        assert_eq!(elements.len(), 2, "expected markdown + column_set");
        assert_eq!(elements[0]["tag"], "markdown");
        assert_eq!(elements[0]["content"], "approve?");
        assert_eq!(elements[1]["tag"], "column_set");

        let columns = elements[1]["columns"].as_array().unwrap();
        assert_eq!(columns.len(), 2);

        let first_button = &columns[0]["elements"][0];
        assert_eq!(first_button["tag"], "button");
        assert_eq!(first_button["type"], "primary");
        let behaviors = first_button["behaviors"].as_array().unwrap();
        assert_eq!(behaviors.len(), 1);
        assert_eq!(behaviors[0]["type"], "callback");
        assert_eq!(
            behaviors[0]["value"]["hope_callback"],
            "approval:abc:allow_once"
        );

        let second_button = &columns[1]["elements"][0];
        assert_eq!(
            second_button["behaviors"][0]["value"]["hope_callback"],
            "approval:abc:deny"
        );
    }

    #[test]
    fn button_card_omits_markdown_when_text_empty() {
        let buttons = vec![vec![InlineButton {
            text: "OK".to_string(),
            callback_data: Some("ask_user:1:ok".to_string()),
            url: None,
        }]];

        let card_none = build_button_card_v2(None, &buttons);
        let elements_none = card_none["body"]["elements"].as_array().unwrap();
        assert_eq!(elements_none.len(), 1, "no text => no markdown element");
        assert_eq!(elements_none[0]["tag"], "column_set");

        let card_empty = build_button_card_v2(Some(""), &buttons);
        let elements_empty = card_empty["body"]["elements"].as_array().unwrap();
        assert_eq!(elements_empty.len(), 1, "empty text => no markdown element");
    }

    #[test]
    fn button_card_falls_back_to_button_text_when_no_callback_data() {
        let buttons = vec![vec![InlineButton {
            text: "RawText".to_string(),
            callback_data: None,
            url: None,
        }]];
        let card = build_button_card_v2(Some("t"), &buttons);
        let columns = card["body"]["elements"][1]["columns"].as_array().unwrap();
        // callback_id() returns the button text when callback_data is None.
        assert_eq!(
            columns[0]["elements"][0]["behaviors"][0]["value"]["hope_callback"],
            "RawText"
        );
    }
}
