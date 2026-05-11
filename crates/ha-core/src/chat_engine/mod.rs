pub mod active_persisters;
pub mod active_turn;
pub mod context;
mod engine;
pub(crate) mod im_error_message;
pub(crate) mod im_mirror;
pub(crate) mod im_system_message;
pub(crate) mod persister;
pub(crate) mod quote;
pub mod sink_registry;
pub mod stream_broadcast;
pub mod stream_seq;
mod types;

pub use context::*;
pub use engine::*;
pub use stream_seq::ChatSource;
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
    pub turn_id: Option<String>,
    pub status: Option<crate::session::ChatTurnStatus>,
    pub last_terminal_status: Option<crate::session::ChatTurnStatus>,
    pub interrupt_reason: Option<crate::session::ChatTurnInterruptReason>,
}

/// Snapshot the current stream state for a session.
pub fn session_stream_state(session_id: &str) -> SessionStreamState {
    let active_turn = active_turn::current(session_id);
    let latest_turn =
        crate::get_session_db().and_then(|db| db.get_latest_chat_turn(session_id).ok().flatten());
    let status = active_turn
        .as_ref()
        .and_then(|active| {
            crate::get_session_db().and_then(|db| db.get_chat_turn(&active.turn_id).ok().flatten())
        })
        .map(|turn| turn.status)
        .or_else(|| latest_turn.as_ref().map(|turn| turn.status));
    SessionStreamState {
        active: stream_seq::is_active(session_id),
        last_seq: stream_seq::last_seq(session_id),
        stream_id: stream_seq::stream_id(session_id),
        turn_id: active_turn
            .as_ref()
            .map(|turn| turn.turn_id.clone())
            .or_else(|| latest_turn.as_ref().map(|turn| turn.id.clone())),
        status,
        last_terminal_status: latest_turn
            .as_ref()
            .map(|turn| turn.status)
            .filter(|status| status.is_terminal()),
        interrupt_reason: latest_turn.and_then(|turn| turn.interrupt_reason),
    }
}
