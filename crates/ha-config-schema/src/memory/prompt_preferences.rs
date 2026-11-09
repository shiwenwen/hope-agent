//! Wire types for bounded memory writing preferences.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPromptStyle {
    #[default]
    Default,
    Concise,
    Detailed,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryPromptPreference {
    pub style: MemoryPromptStyle,
    pub supplemental: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryPromptPreferences {
    pub extraction: MemoryPromptPreference,
    pub profile: MemoryPromptPreference,
    pub dreaming: MemoryPromptPreference,
}

/// A missing stage inherits the global stage; an explicit default stage
/// restores system wording even when the global stage is customized.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryPromptOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extraction: Option<MemoryPromptPreference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<MemoryPromptPreference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dreaming: Option<MemoryPromptPreference>,
}
