pub mod activation;
pub mod author;
pub mod auto_review;
pub mod commands;
mod discovery;
pub mod fork_helper;
mod frontmatter;
mod prompt;
mod requirements;
mod slash;
mod types;

#[cfg(test)]
mod tests;

pub use activation::{
    activate_skills_for_paths, activated_skill_names, clear_session_activation,
    reset_activation_cache,
};
pub use discovery::*;
pub use fork_helper::{extract_fork_result, spawn_skill_fork, MAX_RESULT_CHARS};
pub use prompt::*;
pub use requirements::*;
pub use slash::*;
pub use types::*;

use serde::{Deserialize, Serialize};

/// Root `AppConfig.skills` section. Phase B' introduces the `autoReview`
/// subtree; future Phase B'' work can add more (e.g. `autoPatch`, `sharing`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsConfig {
    #[serde(default)]
    pub auto_review: auto_review::SkillsAutoReviewConfig,

    /// When `hope-agent server` is running, allow the HTTP `POST
    /// /api/skills/{name}/install` route to spawn package-manager processes
    /// (`brew install`, `npm install -g`, `go install`, `uv tool install`).
    ///
    /// Disabled by default: the feature is effectively a remote command
    /// execution primitive if API Key leaks, and in headless server
    /// environments package managers are often not on PATH anyway. Enable
    /// only on trusted deployments where the operator wants UI-driven
    /// dependency install.
    ///
    /// Has no effect on the Tauri desktop shell — clicking "Install" in the
    /// native GUI is always allowed (the button itself is the user consent).
    #[serde(default)]
    pub allow_remote_install: bool,
}
