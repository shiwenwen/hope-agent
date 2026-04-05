use crate::channel::types::*;
use crate::provider;
use oc_core::app_warn;

// ── List Plugins ─────────────────────────────────────────────────

#[tauri::command]
pub async fn channel_list_plugins() -> Result<Vec<serde_json::Value>, String> {
    let registry = crate::get_channel_registry()
        .ok_or_else(|| "Channel registry not initialized".to_string())?;

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
pub async fn channel_list_accounts() -> Result<Vec<ChannelAccountConfig>, String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    Ok(store.channels.accounts)
}

#[tauri::command]
pub async fn channel_add_account(
    channel_id: String,
    label: String,
    agent_id: Option<String>,
    credentials: serde_json::Value,
    settings: serde_json::Value,
    security: SecurityConfig,
) -> Result<String, String> {
    let id = format!(
        "{}-{}",
        channel_id,
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("0000")
    );

    let parsed_channel_id: ChannelId =
        serde_json::from_value(serde_json::Value::String(channel_id.clone()))
            .map_err(|e| format!("Invalid channel_id '{}': {}", channel_id, e))?;

    let account = ChannelAccountConfig {
        id: id.clone(),
        channel_id: parsed_channel_id,
        label,
        enabled: true,
        agent_id,
        credentials,
        settings,
        security,
    };

    // Save to config
    let mut store = provider::load_store().map_err(|e| e.to_string())?;
    store.channels.accounts.push(account.clone());
    provider::save_store(&store).map_err(|e| e.to_string())?;

    // Auto-start if enabled
    if account.enabled {
        if let Some(registry) = crate::get_channel_registry() {
            if let Err(e) = registry.start_account(&account).await {
                app_warn!(
                    "channel",
                    "commands",
                    "Failed to auto-start new account '{}': {}",
                    id,
                    e
                );
            }
        }
    }

    Ok(id)
}

#[tauri::command]
pub async fn channel_update_account(
    account_id: String,
    label: Option<String>,
    enabled: Option<bool>,
    agent_id: Option<String>,
    credentials: Option<serde_json::Value>,
    settings: Option<serde_json::Value>,
    security: Option<SecurityConfig>,
) -> Result<(), String> {
    let mut store = provider::load_store().map_err(|e| e.to_string())?;

    let account = store
        .channels
        .find_account_mut(&account_id)
        .ok_or_else(|| format!("Account '{}' not found", account_id))?;

    let was_enabled = account.enabled;

    if let Some(l) = label {
        account.label = l;
    }
    if let Some(e) = enabled {
        account.enabled = e;
    }
    // agent_id: Some("xxx") = set, Some("") = clear to default, None = no change
    if let Some(ref aid) = agent_id {
        account.agent_id = if aid.is_empty() {
            None
        } else {
            Some(aid.clone())
        };
    }
    if let Some(c) = credentials {
        account.credentials = c;
    }
    if let Some(s) = settings {
        account.settings = s;
    }
    if let Some(sec) = security {
        account.security = sec;
    }

    let updated_account = account.clone();
    provider::save_store(&store).map_err(|e| e.to_string())?;

    // Handle enable/disable state changes
    if let Some(registry) = crate::get_channel_registry() {
        if was_enabled && !updated_account.enabled {
            let _ = registry.stop_account(&account_id).await;
        } else if !was_enabled && updated_account.enabled {
            if let Err(e) = registry.start_account(&updated_account).await {
                return Err(format!("Failed to start account: {}", e));
            }
        } else if updated_account.enabled {
            // Restart to apply config changes
            if let Err(e) = registry.restart_account(&updated_account).await {
                return Err(format!("Failed to restart account: {}", e));
            }
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn channel_remove_account(account_id: String) -> Result<(), String> {
    // Stop if running
    if let Some(registry) = crate::get_channel_registry() {
        let _ = registry.stop_account(&account_id).await;
    }

    // Remove from config
    let mut store = provider::load_store().map_err(|e| e.to_string())?;
    let removed_channel_id = store
        .channels
        .find_account(&account_id)
        .map(|account| account.channel_id.clone());
    store.channels.accounts.retain(|a| a.id != account_id);
    provider::save_store(&store).map_err(|e| e.to_string())?;

    if matches!(removed_channel_id, Some(ChannelId::WeChat)) {
        crate::channel::wechat::clear_persisted_account_state(&account_id)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

// ── Lifecycle ────────────────────────────────────────────────────

#[tauri::command]
pub async fn channel_start_account(account_id: String) -> Result<(), String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    let account = store
        .channels
        .find_account(&account_id)
        .ok_or_else(|| format!("Account '{}' not found", account_id))?
        .clone();

    let registry = crate::get_channel_registry()
        .ok_or_else(|| "Channel registry not initialized".to_string())?;

    registry
        .start_account(&account)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn channel_stop_account(account_id: String) -> Result<(), String> {
    let registry = crate::get_channel_registry()
        .ok_or_else(|| "Channel registry not initialized".to_string())?;

    registry
        .stop_account(&account_id)
        .await
        .map_err(|e| e.to_string())
}

// ── Health ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn channel_health(account_id: String) -> Result<ChannelHealth, String> {
    let registry = crate::get_channel_registry()
        .ok_or_else(|| "Channel registry not initialized".to_string())?;

    // Get running status
    let mut health = registry.health(&account_id).await;

    // If not running, try probe from config
    if !health.is_running {
        let store = provider::load_store().map_err(|e| e.to_string())?;
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
pub async fn channel_health_all() -> Result<Vec<(String, ChannelHealth)>, String> {
    let registry = crate::get_channel_registry()
        .ok_or_else(|| "Channel registry not initialized".to_string())?;

    Ok(registry.list_running().await)
}

// ── Validation ───────────────────────────────────────────────────

#[tauri::command]
pub async fn channel_validate_credentials(
    channel_id: String,
    credentials: serde_json::Value,
) -> Result<String, String> {
    let parsed_channel_id: ChannelId =
        serde_json::from_value(serde_json::Value::String(channel_id.clone()))
            .map_err(|e| format!("Invalid channel_id '{}': {}", channel_id, e))?;

    let registry = crate::get_channel_registry()
        .ok_or_else(|| "Channel registry not initialized".to_string())?;

    let plugin = registry
        .get_plugin(&parsed_channel_id)
        .ok_or_else(|| format!("No plugin for channel: {}", channel_id))?;

    plugin
        .validate_credentials(&credentials)
        .await
        .map_err(|e| e.to_string())
}

// ── Test Message ─────────────────────────────────────────────────

#[tauri::command]
pub async fn channel_send_test_message(
    account_id: String,
    chat_id: String,
    text: String,
) -> Result<DeliveryResult, String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    let account = store
        .channels
        .find_account(&account_id)
        .ok_or_else(|| format!("Account '{}' not found", account_id))?;

    let registry = crate::get_channel_registry()
        .ok_or_else(|| "Channel registry not initialized".to_string())?;

    let payload = ReplyPayload::text(text);
    registry
        .send_reply(account, &chat_id, &payload)
        .await
        .map_err(|e| e.to_string())
}

// ── Sessions ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn channel_list_sessions(
    channel_id: String,
    account_id: String,
) -> Result<Vec<serde_json::Value>, String> {
    let channel_db =
        crate::get_channel_db().ok_or_else(|| "Channel DB not initialized".to_string())?;

    let conversations = channel_db
        .list_conversations(&channel_id, &account_id)
        .map_err(|e| e.to_string())?;

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
) -> Result<crate::channel::wechat::login::WeChatLoginStart, String> {
    crate::channel::wechat::login::start_login(account_id.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn channel_wechat_wait_login(
    session_key: String,
    timeout_ms: Option<u64>,
) -> Result<crate::channel::wechat::login::WeChatLoginWait, String> {
    crate::channel::wechat::login::wait_login(&session_key, timeout_ms)
        .await
        .map_err(|e| e.to_string())
}
