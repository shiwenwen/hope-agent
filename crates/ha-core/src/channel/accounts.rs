//! Channel account CRUD helpers shared by the Tauri command layer and the
//! HTTP server. Both used to inline nearly-identical versions of this logic;
//! keeping it here means lifecycle management (auto-start, restart-on-change,
//! WeChat persisted-state cleanup) stays in exactly one place.

use anyhow::{anyhow, Result};
use serde_json::Value;
use uuid::Uuid;

use super::types::{ChannelAccountConfig, ChannelId, SecurityConfig};

/// Extract a stable identity string from a channel account's credential blob
/// so we can detect "same bot added twice". Returns `None` when credentials
/// don't carry a meaningful identifier yet (e.g. WeChat before QR login,
/// iMessage which has no credentials at all) — in that case duplicate
/// detection is skipped rather than false-positive on empty strings.
fn credential_fingerprint(channel_id: &ChannelId, credentials: &Value) -> Option<String> {
    let str_field = |key: &str| -> Option<String> {
        credentials
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    match channel_id {
        ChannelId::Telegram | ChannelId::Discord | ChannelId::WhatsApp => str_field("token"),
        ChannelId::Slack => str_field("botToken"),
        ChannelId::Feishu | ChannelId::QqBot => str_field("appId"),
        ChannelId::Line => str_field("channelAccessToken"),
        ChannelId::WeChat => str_field("userId")
            .or_else(|| str_field("remoteAccountId"))
            .or_else(|| str_field("token")),
        ChannelId::Signal => str_field("account"),
        ChannelId::Irc => {
            let server = str_field("server")?;
            let port = credentials
                .get("port")
                .and_then(|v| v.as_u64())
                .unwrap_or(6697);
            let nick = str_field("nick")?;
            Some(format!("{}:{}|{}", server, port, nick))
        }
        ChannelId::GoogleChat => {
            let raw = str_field("credentialsJson")?;
            if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
                if let Some(email) = parsed.get("client_email").and_then(|v| v.as_str()) {
                    let trimmed = email.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
            Some(raw)
        }
        ChannelId::IMessage => None,
        ChannelId::Custom(_) => str_field("token"),
    }
}

/// Find an existing account whose credentials resolve to the same
/// fingerprint, optionally excluding the account being updated. Only accounts
/// on the same `channel_id` are compared — sharing a token string across
/// different platforms would be a coincidence, not a duplicate bot.
fn find_duplicate_account<'a>(
    accounts: &'a [ChannelAccountConfig],
    channel_id: &ChannelId,
    fingerprint: &str,
    exclude_id: Option<&str>,
) -> Option<&'a ChannelAccountConfig> {
    accounts.iter().find(|a| {
        if exclude_id.is_some_and(|eid| eid == a.id) {
            return false;
        }
        if &a.channel_id != channel_id {
            return false;
        }
        credential_fingerprint(&a.channel_id, &a.credentials).as_deref() == Some(fingerprint)
    })
}

/// Error prefix used by `add_account` / `update_account` when duplicate
/// credentials are detected. The frontend matches on this prefix to show a
/// localized "this bot is already added" hint. Kept as a prefix (not a bare
/// code) so CLI / HTTP consumers still get a readable message.
pub const DUPLICATE_CREDENTIAL_ERROR_PREFIX: &str = "DUPLICATE_CREDENTIAL";

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
    let parsed_channel_id: ChannelId = serde_json::from_value(Value::String(channel_id.clone()))
        .map_err(|e| anyhow!("Invalid channel_id '{}': {}", channel_id, e))?;

    let mut store = crate::config::load_config()?;

    if let Some(fp) = credential_fingerprint(&parsed_channel_id, &credentials) {
        if let Some(existing) =
            find_duplicate_account(&store.channels.accounts, &parsed_channel_id, &fp, None)
        {
            return Err(anyhow!(
                "{}: {}",
                DUPLICATE_CREDENTIAL_ERROR_PREFIX,
                existing.label
            ));
        }
    }

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

    store.channels.accounts.push(account.clone());
    crate::config::save_config(&store)?;

    if account.enabled {
        if let Some(registry) = crate::get_channel_registry() {
            if let Err(e) = registry.start_account(&account).await {
                super::start_watchdog::register_failure(&account, &e).await;
            }
        }
    }

    Ok(id)
}

/// Apply `params` to the named account and manage registry lifecycle
/// transitions (start/stop/restart) based on the before/after enabled state.
pub async fn update_account(account_id: &str, params: UpdateAccountParams) -> Result<()> {
    let mut store = crate::config::load_config()?;

    let idx = store
        .channels
        .accounts
        .iter()
        .position(|a| a.id == account_id)
        .ok_or_else(|| anyhow!("Account '{}' not found", account_id))?;
    let channel_id = store.channels.accounts[idx].channel_id.clone();

    if let Some(new_credentials) = params.credentials.as_ref() {
        if let Some(fp) = credential_fingerprint(&channel_id, new_credentials) {
            if let Some(existing) =
                find_duplicate_account(&store.channels.accounts, &channel_id, &fp, Some(account_id))
            {
                return Err(anyhow!(
                    "{}: {}",
                    DUPLICATE_CREDENTIAL_ERROR_PREFIX,
                    existing.label
                ));
            }
        }
    }

    let account = &mut store.channels.accounts[idx];
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
    crate::config::save_config(&store)?;

    if let Some(registry) = crate::get_channel_registry() {
        if was_enabled && !updated.enabled {
            let _ = registry.stop_account(account_id).await;
        } else if updated.enabled {
            // start_account on a fresh enable, restart_account when already running.
            let result = if was_enabled {
                registry.restart_account(&updated).await
            } else {
                registry.start_account(&updated).await
            };
            if let Err(e) = result {
                super::start_watchdog::register_failure(&updated, &e).await;
                return Err(anyhow!("Failed to start account: {}", e));
            }
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

    let mut store = crate::config::load_config()?;
    let removed_channel_id = store
        .channels
        .find_account(account_id)
        .map(|a| a.channel_id.clone());
    store.channels.accounts.retain(|a| a.id != account_id);
    crate::config::save_config(&store)?;

    if matches!(removed_channel_id, Some(ChannelId::WeChat)) {
        super::wechat::clear_persisted_account_state(account_id).map_err(|e| anyhow!("{}", e))?;
    }

    Ok(())
}
