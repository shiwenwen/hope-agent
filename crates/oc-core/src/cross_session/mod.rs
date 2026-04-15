//! Cross-Session Behavior Awareness
//!
//! Gives each chat session a short, dynamically-refreshed markdown block
//! describing what the user is doing in other sessions right now.
//!
//! Two data paths:
//!   1. **Structured** (zero LLM cost): reads `session_facets` / `sessions` /
//!      in-memory `ActiveSessionRegistry` and renders a compact list.
//!   2. **LLM Digest** (opt-in): uses `AssistantAgent::side_query` to turn the
//!      candidates into a concrete "what is the user actually doing" paragraph.
//!      Runs async, never blocks the current chat turn.
//!
//! The suffix lives outside the cached system-prompt prefix so prompt-cache
//! hits on the static prefix are preserved even when the suffix changes.

pub mod awareness;
pub mod build;
pub mod collect;
pub mod config;
pub mod dirty;
pub mod llm_digest;
pub mod peek_tool;
pub mod registry;
pub mod render;
pub mod types;

pub use awareness::SessionAwareness;
pub use build::build_prompt_section;
pub use config::{
    resolve_for_session, CrossSessionConfig, CrossSessionMode, ExtractionModelRef,
    LlmExtractionConfig,
};
pub use dirty::{mark_all_except, on_other_session_activity, take_dirty};
pub use llm_digest::build_extraction_prompt_pub;
pub use peek_tool::{peek_sessions_schema, run_peek_sessions};
pub use registry::{active_since, active_snapshot, touch_active_session};
pub use types::{
    ActivityState, CrossSessionEntry, CrossSessionSnapshot, RefreshReason, SessionKind,
};
