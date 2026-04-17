//! Auto-review pipeline: analyze a conversation after a turn hook fires and
//! decide whether to create / patch / skip a reusable skill.
//!
//! Phase B'1 scaffolding — filled out in the next commit.

mod config;
mod pipeline;
mod prompts;
mod triggers;

pub use config::{AutoReviewPromotion, SkillsAutoReviewConfig};
pub use pipeline::{run_review_cycle, ReviewDecision, ReviewReport, ReviewTrigger};
pub use triggers::{maybe_trigger_post_turn, touch_turn_stats, AutoReviewGate};
