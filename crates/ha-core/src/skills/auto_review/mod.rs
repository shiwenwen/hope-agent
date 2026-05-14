//! Auto-review pipeline: analyze a conversation after a turn hook fires and
//! decide whether to create / patch / skip a reusable skill.
//!
//! Five-gate waterfall:
//!   gate 1 — `triggers::touch_and_maybe_trigger` (signal-driven trigger)
//!   gate 2 — `heuristics::pre_gate` (pre-LLM cheap rejection)
//!   gate 3 — `pipeline::run_review_cycle` (LLM review + dedup)
//!   gate 4 — `pipeline::apply_create` self-score floor (hard threshold)
//!   gate 5 — `heuristics::post_lint` (deterministic body lint)

mod config;
pub mod heuristics;
mod pipeline;
mod prompts;
mod triggers;

pub use config::{AutoReviewPromotion, SkillsAutoReviewConfig};
pub use pipeline::{run_review_cycle, ReviewDecision, ReviewReport, ReviewTrigger};
pub use triggers::{
    acquire_manual, sweep_stale, touch_and_maybe_trigger, AutoReviewGate, TriggerSignals,
};
