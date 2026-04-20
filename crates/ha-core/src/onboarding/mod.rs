//! First-run onboarding wizard backend.
//!
//! The wizard is driven from two front-ends (GUI React component and CLI
//! text prompts) but both funnel through the same `state` and `apply`
//! helpers here so field semantics never drift between paths.
//!
//! Storage: [`OnboardingState`] is persisted as a single sub-object of
//! [`AppConfig`] (`~/.hope-agent/config.json`). Step-specific writes
//! (language, profile, personality, safety, skills, server) each target the
//! natural home of that data — user.json, agent.json, or other AppConfig
//! fields — and the shared autosave machinery in [`crate::backup`] tags
//! every snapshot with `onboarding/<step>` for easy rollback.

pub mod apply;
pub mod presets;
pub mod state;

pub use crate::config::{OnboardingState, CURRENT_ONBOARDING_VERSION};
pub use presets::{personality_preset_by_id, PersonalityPreset};
pub use state::{
    get_state, infer_legacy_completed, mark_completed, mark_skipped, reset, save_draft,
};
