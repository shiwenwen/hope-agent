pub mod author;
pub mod auto_review;
mod discovery;
mod frontmatter;
mod prompt;
mod requirements;
mod slash;
mod types;

#[cfg(test)]
mod tests;

pub use discovery::*;
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
