//! Session title generation configuration (`AppConfig.session_title`).

use serde::{Deserialize, Serialize};

use crate::provider::ModelChain;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTitleConfig {
    #[serde(default = "default_session_title_enabled")]
    pub enabled: bool,
    /// Deprecated — superseded by `modelOverride`. Kept for backward
    /// compatibility: still read when `modelOverride` is unset, but the GUI
    /// no longer writes these two fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// Model chain override for title generation. `None` = fall through to
    /// the deprecated `provider_id`/`model_id` pair (if both set) →
    /// `function_models.automation` (title generation is exactly the kind
    /// of cheap, low-stakes background call that default is meant for) →
    /// the current chat's own model (a guaranteed final fallback, so title
    /// generation never fails outright even with zero config).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<ModelChain>,
}

fn default_session_title_enabled() -> bool {
    true
}

impl Default for SessionTitleConfig {
    fn default() -> Self {
        Self {
            enabled: default_session_title_enabled(),
            provider_id: None,
            model_id: None,
            model_override: None,
        }
    }
}
