//! Owner-managed display identity; persisted separately from shared Agent config.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantProfile {
    pub name: String,
    #[serde(default)]
    pub avatar: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantNoticeRoute {
    #[default]
    None,
    Desktop,
    Im,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantPolicySettings {
    pub research_enabled: bool,
    pub paused: bool,
    pub timezone: Option<String>,
    pub notice_route: AssistantNoticeRoute,
    pub quiet_start_hour: u32,
    pub quiet_end_hour: u32,
}

impl Default for AssistantPolicySettings {
    fn default() -> Self {
        Self {
            research_enabled: false,
            paused: false,
            timezone: None,
            notice_route: AssistantNoticeRoute::None,
            quiet_start_hour: 22,
            quiet_end_hour: 8,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantPolicyConfig {
    pub revision: i64,
    pub settings: AssistantPolicySettings,
}
