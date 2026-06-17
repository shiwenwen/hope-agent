//! Sprite (inspiration mode) types — a proactive, transient writing companion
//! for the knowledge-space chat panel. The sprite reacts to the note the user
//! is *currently editing*; the frontend gathers the observation inputs on
//! edit-idle and posts them via `kb_sprite_observe_cmd`.

use serde::{Deserialize, Serialize};

/// One recent chat message, supplied by the frontend (the panel already holds
/// `session.messages`, so the backend needs no DB read for conversation sense).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpriteMsg {
    pub role: String,
    pub text: String,
}

/// Inputs for one observation, gathered by the frontend on edit-idle.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpriteObserveParams {
    #[serde(default)]
    pub session_id: Option<String>,
    pub kb_id: String,
    pub note_path: String,
    pub agent_id: String,
    /// Current document text (already client-truncated to a budget).
    pub doc_content: String,
    /// Coarse "what just changed" hint (net-added text), if available.
    #[serde(default)]
    pub recent_edit: Option<String>,
    /// Recent conversation turns from the panel (newest last), for context.
    #[serde(default)]
    pub recent_messages: Option<Vec<SpriteMsg>>,
}

/// Suggestion category — drives the bubble's badge + tone.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpriteCategory {
    /// Writing suggestion — how to continue / improve.
    Writing,
    /// Feedback on what was just written.
    Review,
    /// Encouragement / emotional value.
    Encourage,
    /// A timely reminder.
    Remind,
    /// A connection to another note or memory.
    Connect,
}

impl SpriteCategory {
    /// Best-effort parse of the LLM's category string; unknown → `Writing`.
    pub fn from_wire(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "review" => Self::Review,
            "encourage" => Self::Encourage,
            "remind" => Self::Remind,
            "connect" => Self::Connect,
            _ => Self::Writing,
        }
    }
}

/// A spoken suggestion (parsed from the LLM's JSON envelope).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpriteSuggestion {
    pub category: SpriteCategory,
    pub text: String,
}

/// Outcome of an observation — for command-side logging only. The actual
/// suggestion reaches the UI via the `sprite:suggestion` EventBus event.
#[derive(Debug, Clone)]
pub enum SpriteOutcome {
    Spoke(SpriteSuggestion),
    Skipped(&'static str),
    Silent,
}
