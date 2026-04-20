//! Pre-canned personality presets offered in Step 4 of the wizard.
//!
//! Each preset maps to a [`PersonalityConfig`] that the front-end applies to
//! the default agent on "Next". Presets intentionally leave `traits`,
//! `principles`, `boundaries`, `quirks` empty so the structured editor
//! later in the settings UI remains a clean slate the user can extend.

use crate::agent_config::{PersonaMode, PersonalityConfig};

/// Stable string id used by both Tauri commands and HTTP routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonalityPreset {
    Default,
    Engineer,
    Creative,
    Companion,
}

impl PersonalityPreset {
    pub fn id(self) -> &'static str {
        match self {
            PersonalityPreset::Default => "default",
            PersonalityPreset::Engineer => "engineer",
            PersonalityPreset::Creative => "creative",
            PersonalityPreset::Companion => "companion",
        }
    }

    pub fn to_config(self) -> PersonalityConfig {
        match self {
            PersonalityPreset::Default => PersonalityConfig {
                mode: PersonaMode::Structured,
                role: Some("General AI assistant".into()),
                vibe: Some("balanced".into()),
                tone: Some("neutral".into()),
                communication_style: Some("clear and concise".into()),
                ..PersonalityConfig::default()
            },
            PersonalityPreset::Engineer => PersonalityConfig {
                mode: PersonaMode::Structured,
                role: Some("Senior software engineer".into()),
                vibe: Some("rigorous".into()),
                tone: Some("precise".into()),
                communication_style: Some("technical, evidence-first, code-oriented".into()),
                ..PersonalityConfig::default()
            },
            PersonalityPreset::Creative => PersonalityConfig {
                mode: PersonaMode::Structured,
                role: Some("Creative collaborator".into()),
                vibe: Some("exploratory".into()),
                tone: Some("energetic".into()),
                communication_style: Some("vivid, rich in analogies and examples".into()),
                ..PersonalityConfig::default()
            },
            PersonalityPreset::Companion => PersonalityConfig {
                mode: PersonaMode::Structured,
                role: Some("Day-to-day life helper".into()),
                vibe: Some("warm".into()),
                tone: Some("friendly".into()),
                communication_style: Some(
                    "natural conversational, like chatting with a friend".into(),
                ),
                ..PersonalityConfig::default()
            },
        }
    }
}

/// Resolve a string id to a preset. Returns `None` for unknown ids so
/// callers can surface a clean validation error.
pub fn personality_preset_by_id(id: &str) -> Option<PersonalityPreset> {
    match id {
        "default" => Some(PersonalityPreset::Default),
        "engineer" => Some(PersonalityPreset::Engineer),
        "creative" => Some(PersonalityPreset::Creative),
        "companion" => Some(PersonalityPreset::Companion),
        _ => None,
    }
}
