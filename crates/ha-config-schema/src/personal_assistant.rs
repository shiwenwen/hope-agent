//! Owner-managed display identity; persisted separately from shared Agent config.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantProfile {
    pub name: String,
    #[serde(default)]
    pub avatar: Option<String>,
}
