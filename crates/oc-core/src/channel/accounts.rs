//! Channel account CRUD helpers shared by the Tauri command layer and the
//! HTTP server. Both used to inline nearly-identical versions of this logic;
//! keeping it here means lifecycle management (auto-start, restart-on-change,
//! WeChat persisted-state cleanup) stays in exactly one place.

use anyhow::{anyhow, Result};
use serde_json::Value;
use uuid::Uuid;

use super::types::{ChannelAccountConfig, ChannelId, SecurityConfig};
use crate::provider;

/// Patch for [`update_account`]. `None` fields are left untouched; an empty
/// `agent_id` string clears the account's override back to the default.
#[derive(Debug, Default)]
pub struct UpdateAccountParams {
    pub label: Option<String>,
    pub enabled: Option<bool>,
    pub agent_id: Option<String>,
    pub auto_approve_tools: Option<bool>,
    pub credentials: Option<Value>,
    pub settings: Option<Value>,
    pub security: Option<SecurityConfig>,
}

/// Create a new channel account, persist it, and auto-start if enabled.
/// Returns the generated account id.
pub async fn add_account(
    channel_id: String,
    label: String,
    agent_id: Option<String>,
    credentials: Value,
    settings: Value,
    security: SecurityConfig,
) -> Result<String> {
    let id = format!(
        "{}-{}",
        channel_id,
        Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("0000")
    );
    let parsed_channel_id: ChannelId =
        serde_json::from_value(Value::String(channel_id.clone()))
            .map_err(|e| anyhow!("Invalid channel_id '{}': {}", channel_id, e))?;

    let account = ChannelAccountConfig {
        id: id.clone(),
        channel_id: parsed_channel_id,
        label,
        enabled: true,
        agent_id,
        credentials,
        settings,
        security,
        auto_approve_tools: false,
    };

    let mut store = provider::load_store()?;
    store.channels.accounts.push(account.clone());
    provider::save_store(&store)?;

    if account.enabled {
        if let Some(registry) = crate::get_channel_registry() {
            if let Err(e) = registry.start_account(&account).await {
                crate::app_warn!(
                    "channel",
                    "accounts",
                    "Failed to auto-start new account '{}': {}",
                    id,
                    e
                );
            }
        }
    }

    Ok(id)
}

/// Apply `params` to the named account and manage registry lifecycle
/// transitions (start/stop/restart) based on the before/after enabled state.
pub async fn update_account(account_id: &str, params: UpdateAccountParams) -> Result<()> {
    let mut store = provider::load_store()?;
    let account = store
        .channels
        .find_account_mut(account_id)
        .ok_or_else(|| anyhow!("Account '{}' not found", account_id))?;
    let was_enabled = account.enabled;

    if let Some(l) = params.label {
        account.label = l;
    }
    if let Some(e) = params.enabled {
        account.enabled = e;
    }
    if let Some(aid) = params.agent_id {
        account.agent_id = if aid.is_empty() { None } else { Some(aid) };
    }
    if let Some(aat) = params.auto_approve_tools {
        account.auto_approve_tools = aat;
    }
    if let Some(c) = params.credentials {
        account.credentials = c;
    }
    if let Some(s) = params.settings {
        account.settings = s;
    }
    if let Some(sec) = params.security {
        account.security = sec;
    }

    let updated = account.clone();
    provider::save_store(&store)?;

    if let Some(registry) = crate::get_channel_registry() {
        if was_enabled && !updated.enabled {
            let _ = registry.stop_account(account_id).await;
        } else if !was_enabled && updated.enabled {
            registry
                .start_account(&updated)
                .await
                .map_err(|e| anyhow!("Failed to start account: {}", e))?;
        } else if updated.enabled {
            registry
                .restart_account(&updated)
                .await
                .map_err(|e| anyhow!("Failed to restart account: {}", e))?;
        }
    }

    Ok(())
}

/// Stop, unregister, and clean up a channel account. For WeChat accounts this
/// also removes the persisted iLink state on disk.
pub async fn remove_account(account_id: &str) -> Result<()> {
    if let Some(registry) = crate::get_channel_registry() {
        let _ = registry.stop_account(account_id).await;
    }

    let mut store = provider::load_store()?;
    let removed_channel_id = store
        .channels
        .find_account(account_id)
        .map(|a| a.channel_id.clone());
    store.channels.accounts.retain(|a| a.id != account_id);
    provider::save_store(&store)?;

    if matches!(removed_channel_id, Some(ChannelId::WeChat)) {
        super::wechat::clear_persisted_account_state(account_id)
            .map_err(|e| anyhow!("{}", e))?;
    }

    Ok(())
}
