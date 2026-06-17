use serde::{Deserialize, Serialize};

use super::types::ChannelAccountConfig;

/// Top-level channel configuration stored in AppConfig (config.json).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStoreConfig {
    /// All configured channel accounts (across all channels).
    #[serde(default)]
    pub accounts: Vec<ChannelAccountConfig>,
    /// Legacy channel-specific default Agent ID. Runtime dispatch now lets
    /// unbound channel conversations inherit `AppConfig.default_agent_id`;
    /// keep this field for backward-compatible config deserialization.
    #[serde(default)]
    pub default_agent_id: Option<String>,
    /// Provider/model override for channel conversations.
    /// If None, uses the global active_model from AppConfig.
    #[serde(default)]
    pub default_model: Option<crate::provider::ActiveModel>,
}

impl ChannelStoreConfig {
    /// Find an account by its ID.
    pub fn find_account(&self, account_id: &str) -> Option<&ChannelAccountConfig> {
        self.accounts.iter().find(|a| a.id == account_id)
    }

    /// Find a mutable account by its ID.
    pub fn find_account_mut(&mut self, account_id: &str) -> Option<&mut ChannelAccountConfig> {
        self.accounts.iter_mut().find(|a| a.id == account_id)
    }

    /// List all enabled accounts.
    pub fn enabled_accounts(&self) -> Vec<&ChannelAccountConfig> {
        self.accounts.iter().filter(|a| a.enabled).collect()
    }

    /// Legacy effective channel Agent ID when no app-level context is
    /// available.
    pub fn agent_id(&self) -> &str {
        self.default_agent_id
            .as_deref()
            .unwrap_or(crate::agent_loader::DEFAULT_AGENT_ID)
    }
}

/// WS8 — whether an IM turn originating from `(channel_id, account_id, chat_id)`
/// is allowed knowledge-base access. Reads the live account config:
/// - the account's `kbAccessOptIn` must be set, **and**
/// - for a group / non-DM chat, the specific `chat_id` must also be confirmed.
///
/// A DM only needs the account-level opt-in. Fails **closed** (returns `false`)
/// for an unknown account or a `channel_id` that doesn't match the account's
/// own channel — so a forged / stale identity can never widen access. This is the
/// single config-reading decision point behind `effective_kb_access`'s IM gate.
pub fn im_kb_access_allowed(
    channel_id: &str,
    account_id: &str,
    chat_id: &str,
    is_group: bool,
) -> bool {
    let cfg = crate::config::cached_config();
    let Some(account) = cfg.channels.find_account(account_id) else {
        return false;
    };
    account_kb_access_allowed(account, channel_id, chat_id, is_group)
}

/// Pure WS8 decision over a resolved account (split out so it is unit-testable
/// without touching the global config). Fails closed on a channel-id mismatch.
fn account_kb_access_allowed(
    account: &ChannelAccountConfig,
    channel_id: &str,
    chat_id: &str,
    is_group: bool,
) -> bool {
    // Defense in depth: the carried channel id must match the account's channel.
    if account.channel_id.to_string() != channel_id {
        return false;
    }
    if !account.kb_access_opt_in() {
        return false;
    }
    if is_group {
        account.kb_access_chat_confirmed(chat_id)
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::types::ChannelId;

    fn account(settings: serde_json::Value) -> ChannelAccountConfig {
        ChannelAccountConfig {
            id: "acc1".into(),
            channel_id: ChannelId::WeChat,
            label: "Test".into(),
            enabled: true,
            agent_id: None,
            credentials: serde_json::Value::Null,
            settings,
            security: Default::default(),
            auto_approve_tools: false,
            notify_session_eviction: true,
            notify_startup: true,
        }
    }

    #[test]
    fn deny_without_opt_in() {
        let acc = account(serde_json::Value::Null);
        assert!(!account_kb_access_allowed(&acc, "wechat", "dm1", false));
        assert!(!account_kb_access_allowed(&acc, "wechat", "g1", true));
    }

    #[test]
    fn dm_granted_with_opt_in() {
        let acc = account(serde_json::json!({"kbAccessOptIn": true}));
        // DM: account opt-in alone suffices.
        assert!(account_kb_access_allowed(&acc, "wechat", "dm1", false));
    }

    #[test]
    fn group_needs_per_chat_confirm() {
        let acc = account(serde_json::json!({"kbAccessOptIn": true}));
        // Group: opt-in alone is NOT enough.
        assert!(!account_kb_access_allowed(&acc, "wechat", "g1", true));

        let acc = account(serde_json::json!({
            "kbAccessOptIn": true,
            "kbAccessChats": ["g1"],
        }));
        assert!(account_kb_access_allowed(&acc, "wechat", "g1", true));
        // A different, unconfirmed group stays denied.
        assert!(!account_kb_access_allowed(&acc, "wechat", "g2", true));
    }

    #[test]
    fn channel_id_mismatch_fails_closed() {
        let acc = account(serde_json::json!({"kbAccessOptIn": true}));
        // Even fully opted in, a mismatched channel id denies (fail closed).
        assert!(!account_kb_access_allowed(&acc, "telegram", "dm1", false));
    }
}
