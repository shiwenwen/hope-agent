pub mod active_persisters;
pub mod active_turn;
pub mod context;
mod engine;
pub(crate) mod im_mirror;
pub(crate) mod persister;
pub mod sink_registry;
pub mod stream_broadcast;
pub mod stream_seq;
mod types;

pub use context::*;
pub use engine::*;
// Re-export plan-context API from `crate::agent` so chat_engine callers can
// keep `use crate::chat_engine::PlanResolvedContext;` ergonomics. The
// canonical home is `crate::agent::plan_context` (avoids agent →
// chat_engine cycle when `streaming_loop`'s mid-turn probe needs to
// resolve fresh plan extra context).
pub use crate::agent::{
    merge_extra_system_context, resolve_plan_context_for_session, PlanResolvedContext,
};
pub use types::*;

/// Public-facing snapshot of a session's chat stream state. Returned by the
/// `get_session_stream_state` command so the frontend can decide whether to
/// reattach an EventBus listener for an in-flight chat after reloading.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStreamState {
    pub active: bool,
    pub last_seq: u64,
    pub stream_id: Option<String>,
}

/// Snapshot the current stream state for a session.
pub fn session_stream_state(session_id: &str) -> SessionStreamState {
    SessionStreamState {
        active: stream_seq::is_active(session_id),
        last_seq: stream_seq::last_seq(session_id),
        stream_id: stream_seq::stream_id(session_id),
    }
}
