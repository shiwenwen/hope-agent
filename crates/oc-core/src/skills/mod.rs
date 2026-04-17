pub mod activation;
pub mod author;
pub mod auto_review;
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
}
