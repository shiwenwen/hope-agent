use axum::extract::Path;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use ha_core::channel::accounts::{self, UpdateAccountParams};
use ha_core::channel::types::*;

use crate::error::AppError;
use crate::routes::helpers::{channel_db, channel_registry as registry};

/// `GET /api/channel/plugins`
pub async fn list_plugins() -> Result<Json<Vec<Value>>, AppError> {
    let plugins = registry()?.list_plugins();
    Ok(Json(
        plugins
            .into_iter()
            .map(|(meta, caps)| json!({ "meta": meta, "capabilities": caps }))
            .collect(),
    ))
}

/// `GET /api/channel/accounts`
pub async fn list_accounts() -> Result<Json<Vec<ChannelAccountConfig>>, AppError> {
    Ok(Json(
        ha_core::config::cached_config().channels.accounts.clone(),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddAccountBody {
    pub channel_id: String,
    pub label: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    pub credentials: Value,
    pub settings: Value,
    pub security: SecurityConfig,
}

/// `POST /api/channel/accounts`
pub async fn add_account(Json(body): Json<AddAccountBody>) -> Result<Json<Value>, AppError> {
    let id = accounts::add_account(
        body.channel_id,
        body.label,
        body.agent_id,
        body.credentials,
        body.settings,
        body.security,
    )
    .await?;
    Ok(Json(json!({ "id": id })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAccountBody {
    pub label: Option<String>,
    pub enabled: Option<bool>,
    pub agent_id: Option<String>,
    pub auto_approve_tools: Option<bool>,
    pub credentials: Option<Value>,
    pub settings: Option<Value>,
    pub security: Option<SecurityConfig>,
}

/// `PUT /api/channel/accounts/{id}`
pub async fn update_account(
    Path(account_id): Path<String>,
    Json(body): Json<UpdateAccountBody>,
) -> Result<Json<Value>, AppError> {
    accounts::update_account(
        &account_id,
        UpdateAccountParams {
            label: body.label,
            enabled: body.enabled,
            agent_id: body.agent_id,
            auto_approve_tools: body.auto_approve_tools,
            credentials: body.credentials,
            settings: body.settings,
            security: body.security,
        },
    )
    .await?;
    Ok(Json(json!({ "updated": true })))
}

/// `DELETE /api/channel/accounts/{id}`
pub async fn remove_account(Path(account_id): Path<String>) -> Result<Json<Value>, AppError> {
    accounts::remove_account(&account_id).await?;
    Ok(Json(json!({ "deleted": true })))
}

/// `POST /api/channel/accounts/{id}/start`
pub async fn start_account(Path(account_id): Path<String>) -> Result<Json<Value>, AppError> {
    let account = ha_core::config::cached_config()
        .channels
        .find_account(&account_id)
        .ok_or_else(|| AppError::not_found(format!("Account '{}' not found", account_id)))?
        .clone();
    registry()?
        .start_account(&account)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "started": true })))
}

/// `POST /api/channel/accounts/{id}/stop`
pub async fn stop_account(Path(account_id): Path<String>) -> Result<Json<Value>, AppError> {
    registry()?
        .stop_account(&account_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "stopped": true })))
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncCommandsBody {
    /// Optional account id; absent → re-sync every running account.
    #[serde(default)]
    pub account_id: Option<String>,
}

/// `POST /api/channel/sync-commands`
///
/// Re-sync the IM bot menu (Telegram setMyCommands / Discord application
/// commands) for one or all running accounts. The auto-sync listener
/// (`app_init::spawn_channel_menu_resync_listener`) covers the common case;
/// this route exposes a manual trigger for the settings UI and for ops
/// recovery after a missed event.
pub async fn sync_commands(body: Option<Json<SyncCommandsBody>>) -> Result<Json<Value>, AppError> {
    let account_id = body.and_then(|Json(b)| b.account_id);
    let reg = registry()?;
    let count = match account_id {
        Some(id) => {
            if reg
                .sync_commands_for_account(&id)
                .await
                .map_err(|e| AppError::internal(e.to_string()))?
            {
                1
            } else {
                0
            }
        }
        None => reg.sync_commands_for_all().await,
    };
    Ok(Json(json!({ "synced": count })))
}

/// `GET /api/channel/accounts/{id}/health`
pub async fn health(Path(account_id): Path<String>) -> Result<Json<ChannelHealth>, AppError> {
    let reg = registry()?;
    let mut h = reg.health(&account_id).await;
    if !h.is_running {
        let store = ha_core::config::cached_config();
        if let Some(account) = store.channels.find_account(&account_id) {
            if let Some(plugin) = reg.get_plugin(&account.channel_id) {
                if let Ok(probe) = plugin.probe(account).await {
                    h.probe_ok = probe.probe_ok;
                    h.bot_name = probe.bot_name;
                    h.error = probe.error;
                    h.last_probe = probe.last_probe;
                }
            }
        }
    }
    Ok(Json(h))
}

/// `GET /api/channel/health`
pub async fn health_all() -> Result<Json<Vec<(String, ChannelHealth)>>, AppError> {
    Ok(Json(registry()?.list_running().await))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidateBody {
    pub channel_id: String,
    pub credentials: Value,
}

/// `POST /api/channel/validate`
pub async fn validate_credentials(Json(body): Json<ValidateBody>) -> Result<Json<Value>, AppError> {
    let parsed: ChannelId = serde_json::from_value(Value::String(body.channel_id.clone()))
        .map_err(|e| AppError::bad_request(format!("Invalid channel_id: {}", e)))?;
    let plugin = registry()?.get_plugin(&parsed).ok_or_else(|| {
        AppError::not_found(format!("No plugin for channel: {}", body.channel_id))
    })?;
    let info = plugin
        .validate_credentials(&body.credentials)
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(json!({ "info": info })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestMessageBody {
    pub chat_id: String,
    pub text: String,
}

/// `POST /api/channel/accounts/{id}/test-message`
pub async fn send_test_message(
    Path(account_id): Path<String>,
    Json(body): Json<TestMessageBody>,
) -> Result<Json<DeliveryResult>, AppError> {
    let store = ha_core::config::cached_config();
    let account = store
        .channels
        .find_account(&account_id)
        .ok_or_else(|| AppError::not_found(format!("Account '{}' not found", account_id)))?;
    let payload = ReplyPayload::text(body.text);
    let res = registry()?
        .send_reply(account, &body.chat_id, &payload)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(res))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsQuery {
    pub channel_id: String,
    pub account_id: String,
}

/// `GET /api/channel/sessions?channel_id=...&account_id=...`
pub async fn list_sessions(
    axum::extract::Query(q): axum::extract::Query<ListSessionsQuery>,
) -> Result<Json<Vec<Value>>, AppError> {
    let conversations = channel_db()?.list_conversations(&q.channel_id, &q.account_id)?;
    Ok(Json(
        conversations
            .into_iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "channelId": c.channel_id,
                    "accountId": c.account_id,
                    "chatId": c.chat_id,
                    "threadId": c.thread_id,
                    "sessionId": c.session_id,
                    "senderId": c.sender_id,
                    "senderName": c.sender_name,
                    "chatType": c.chat_type,
                    "createdAt": c.created_at,
                    "updatedAt": c.updated_at,
                })
            })
            .collect(),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeChatStartLoginBody {
    #[serde(default)]
    pub account_id: Option<String>,
}

/// `POST /api/channel/wechat/login/start`
pub async fn wechat_start_login(
    Json(body): Json<WeChatStartLoginBody>,
) -> Result<Json<ha_core::channel::wechat::login::WeChatLoginStart>, AppError> {
    Ok(Json(
        ha_core::channel::wechat::login::start_login(body.account_id.as_deref()).await?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeChatWaitLoginBody {
    pub session_key: String,
    pub timeout_ms: Option<u64>,
}

/// `POST /api/channel/wechat/login/wait`
pub async fn wechat_wait_login(
    Json(body): Json<WeChatWaitLoginBody>,
) -> Result<Json<ha_core::channel::wechat::login::WeChatLoginWait>, AppError> {
    Ok(Json(
        ha_core::channel::wechat::login::wait_login(&body.session_key, body.timeout_ms).await?,
    ))
}
