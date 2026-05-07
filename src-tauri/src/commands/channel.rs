use crate::channel::accounts::{self, UpdateAccountParams};
use crate::channel::types::*;
use crate::commands::CmdError;
use anyhow::Context;

// ── List Plugins ─────────────────────────────────────────────────

#[tauri::command]
pub async fn channel_list_plugins() -> Result<Vec<serde_json::Value>, CmdError> {
    let registry = crate::get_channel_registry()
        .ok_or_else(|| CmdError::msg("Channel registry not initialized"))?;

    let plugins = registry.list_plugins();
    let result: Vec<serde_json::Value> = plugins
        .into_iter()
        .map(|(meta, caps)| {
            serde_json::json!({
                "meta": meta,
                "capabilities": caps,
            })
        })
        .collect();

    Ok(result)
}

// ── Account Management ───────────────────────────────────────────

#[tauri::command]
pub async fn channel_list_accounts() -> Result<Vec<ChannelAccountConfig>, CmdError> {
    Ok(ha_core::config::cached_config().channels.accounts.clone())
}

#[tauri::command]
pub async fn channel_add_account(
    channel_id: String,
    label: String,
    agent_id: Option<String>,
    credentials: serde_json::Value,
    settings: serde_json::Value,
    security: SecurityConfig,
) -> Result<String, CmdError> {
    accounts::add_account(channel_id, label, agent_id, credentials, settings, security)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn channel_update_account(
    account_id: String,
    label: Option<String>,
    enabled: Option<bool>,
    agent_id: Option<String>,
    auto_approve_tools: Option<bool>,
    notify_primary_changes: Option<bool>,
    credentials: Option<serde_json::Value>,
    settings: Option<serde_json::Value>,
    security: Option<SecurityConfig>,
) -> Result<(), CmdError> {
    accounts::update_account(
        &account_id,
        UpdateAccountParams {
            label,
            enabled,
            agent_id,
            auto_approve_tools,
            notify_primary_changes,
            credentials,
            settings,
            security,
        },
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn channel_remove_account(account_id: String) -> Result<(), CmdError> {
    accounts::remove_account(&account_id)
        .await
        .map_err(Into::into)
}

// ── Lifecycle ────────────────────────────────────────────────────

#[tauri::command]
pub async fn channel_start_account(account_id: String) -> Result<(), CmdError> {
    let account = ha_core::config::cached_config()
        .channels
        .find_account(&account_id)
        .ok_or_else(|| CmdError::msg(format!("Account '{}' not found", account_id)))?
        .clone();

    let registry = crate::get_channel_registry()
        .ok_or_else(|| CmdError::msg("Channel registry not initialized"))?;

    registry.start_account(&account).await.map_err(Into::into)
}

#[tauri::command]
pub async fn channel_stop_account(account_id: String) -> Result<(), CmdError> {
    let registry = crate::get_channel_registry()
        .ok_or_else(|| CmdError::msg("Channel registry not initialized"))?;

    registry.stop_account(&account_id).await.map_err(Into::into)
}

// ── Slash command menu sync ──────────────────────────────────────

/// Re-sync the IM bot menu (Telegram setMyCommands / Discord application
/// commands) for one or all running accounts. Returns the count of accounts
/// successfully synced.
///
/// Usually fires automatically via the EventBus listener on skill changes
/// (see `app_init::spawn_channel_menu_resync_listener`); this command
/// covers the manual "Sync now" button and recovery from missed events.
#[tauri::command]
pub async fn channel_sync_commands(account_id: Option<String>) -> Result<usize, CmdError> {
    let registry = crate::get_channel_registry()
        .ok_or_else(|| CmdError::msg("Channel registry not initialized"))?;
    Ok(registry.sync_commands(account_id.as_deref()).await?)
}

// ── Health ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn channel_health(account_id: String) -> Result<ChannelHealth, CmdError> {
    let registry = crate::get_channel_registry()
        .ok_or_else(|| CmdError::msg("Channel registry not initialized"))?;

    // Get running status
    let mut health = registry.health(&account_id).await;

    // If not running, try probe from config
    if !health.is_running {
        let store = ha_core::config::cached_config();
        if let Some(account) = store.channels.find_account(&account_id) {
            if let Some(plugin) = registry.get_plugin(&account.channel_id) {
                if let Ok(probe_health) = plugin.probe(account).await {
                    health.probe_ok = probe_health.probe_ok;
                    health.bot_name = probe_health.bot_name;
                    health.error = probe_health.error;
                    health.last_probe = probe_health.last_probe;
                }
            }
        }
    }

    Ok(health)
}

#[tauri::command]
pub async fn channel_health_all() -> Result<Vec<(String, ChannelHealth)>, CmdError> {
    let registry = crate::get_channel_registry()
        .ok_or_else(|| CmdError::msg("Channel registry not initialized"))?;

    Ok(registry.list_running().await)
}

// ── Validation ───────────────────────────────────────────────────

#[tauri::command]
pub async fn channel_validate_credentials(
    channel_id: String,
    credentials: serde_json::Value,
) -> Result<String, CmdError> {
    let parsed_channel_id: ChannelId =
        serde_json::from_value(serde_json::Value::String(channel_id.clone()))
            .with_context(|| format!("Invalid channel_id '{}'", channel_id))?;

    let registry = crate::get_channel_registry()
        .ok_or_else(|| CmdError::msg("Channel registry not initialized"))?;

    let plugin = registry
        .get_plugin(&parsed_channel_id)
        .ok_or_else(|| CmdError::msg(format!("No plugin for channel: {}", channel_id)))?;

    plugin
        .validate_credentials(&credentials)
        .await
        .map_err(Into::into)
}

// ── Test Message ─────────────────────────────────────────────────

#[tauri::command]
pub async fn channel_send_test_message(
    account_id: String,
    chat_id: String,
    text: String,
) -> Result<DeliveryResult, CmdError> {
    let store = ha_core::config::cached_config();
    let account = store
        .channels
        .find_account(&account_id)
        .ok_or_else(|| CmdError::msg(format!("Account '{}' not found", account_id)))?;

    let registry = crate::get_channel_registry()
        .ok_or_else(|| CmdError::msg("Channel registry not initialized"))?;

    let payload = ReplyPayload::text(text);
    registry
        .send_reply(account, &chat_id, &payload)
        .await
        .map_err(Into::into)
}

// ── Sessions ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn channel_list_sessions(
    channel_id: String,
    account_id: String,
) -> Result<Vec<serde_json::Value>, CmdError> {
    let channel_db =
        crate::get_channel_db().ok_or_else(|| CmdError::msg("Channel DB not initialized"))?;

    let conversations = channel_db.list_conversations(&channel_id, &account_id)?;

    let result: Vec<serde_json::Value> = conversations
        .into_iter()
        .map(|c| {
            serde_json::json!({
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
        .collect();

    Ok(result)
}

// ── WeChat QR Login ─────────────────────────────────────────────

#[tauri::command]
pub async fn channel_wechat_start_login(
    account_id: Option<String>,
) -> Result<crate::channel::wechat::login::WeChatLoginStart, CmdError> {
    crate::channel::wechat::login::start_login(account_id.as_deref())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn channel_wechat_wait_login(
    session_key: String,
    timeout_ms: Option<u64>,
) -> Result<crate::channel::wechat::login::WeChatLoginWait, CmdError> {
    crate::channel::wechat::login::wait_login(&session_key, timeout_ms)
        .await
        .map_err(Into::into)
}

// ── Handover ─────────────────────────────────────────────────────

/// Hand the given session over to an IM chat — pushes a fresh attach row
/// for (channel, account, chat, thread) and promotes it to primary. Used
/// by the GUI Handover dialog (Phase B2) and `/handover` slash command.
///
/// `chat_type` defaults to `dm` when not supplied; the IM worker overwrites
/// it on the next inbound message from the chat.
#[tauri::command]
pub async fn channel_handover_session(
    session_id: String,
    channel_id: String,
    account_id: String,
    chat_id: String,
    thread_id: Option<String>,
    chat_type: Option<String>,
) -> Result<(), CmdError> {
    let channel_db = ha_core::get_channel_db()
        .ok_or_else(|| CmdError::msg("Channel DB not initialized"))?;

    let resolved_chat_type = match chat_type.as_deref() {
        Some("group") => crate::channel::types::ChatType::Group,
        Some("forum") => crate::channel::types::ChatType::Forum,
        Some("channel") => crate::channel::types::ChatType::Channel,
        _ => crate::channel::types::ChatType::Dm,
    };

    channel_db
        .attach_session(
            &channel_id,
            &account_id,
            &chat_id,
            thread_id.as_deref(),
            &session_id,
            ha_core::channel::db::ATTACH_SOURCE_HANDOVER,
            None,
            None,
            &resolved_chat_type,
        )
        .map_err(|e| CmdError::msg(format!("Handover failed: {}", e)))?;

    Ok(())
}
