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
        self.default_agent_id.as_deref().unwrap_or("default")
    }
}
