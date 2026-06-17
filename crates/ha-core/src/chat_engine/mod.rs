pub mod active_persisters;
pub mod active_turn;
pub mod context;
mod engine;
pub mod finalize;
pub(crate) mod im_error_message;
pub(crate) mod im_mirror;
pub(crate) mod im_system_message;
pub(crate) mod persister;
pub(crate) mod quote;
pub mod sink_registry;
pub mod stream_broadcast;
pub mod stream_seq;
pub mod turn_injection;
mod types;

use std::sync::Arc;
use std::time::Duration;

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
    let active = stream_seq::is_active(session_id)
        || active_turn
            .as_ref()
            .is_some_and(|_| status.map(|s| !s.is_terminal()).unwrap_or(true));
    SessionStreamState {
        active,
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

pub const CHAT_STOP_WATCHDOG_GRACE: Duration = Duration::from_secs(5);

pub fn spawn_user_stop_watchdog(
    db: Arc<crate::session::SessionDB>,
    session_id: String,
    turn_id: String,
    source: ChatSource,
) {
    tokio::spawn(async move {
        tokio::time::sleep(CHAT_STOP_WATCHDOG_GRACE).await;

        let turn = match db.get_chat_turn(&turn_id) {
            Ok(Some(turn)) if !turn.status.is_terminal() => turn,
            _ => return,
        };
        let stream_id = turn.stream_id.clone().or_else(|| {
            active_turn::current(&session_id)
                .filter(|active| active.turn_id == turn_id)
                .and_then(|active| active.stream_id)
        });

        let flushed = active_persisters::cancel_flush_session(&session_id);
        if flushed > 0 {
            app_info!(
                "chat",
                "stop_watchdog",
                "Flushed {} active persister(s) before finalizing cancelled turn {}",
                flushed,
                turn_id
            );
        }

        let mut partial = finalize::rebuild::collect_partial_from_messages(&db, &session_id, None);
        partial.turn_id = Some(turn_id.clone());

        let outcome = finalize::finalize_turn_context(
            &db,
            &session_id,
            finalize::TerminationReason::UserStop,
            partial,
            source,
            None,
        )
        .await;

        let status = outcome
            .turn_status
            .or_else(|| db.get_chat_turn(&turn_id).ok().flatten().map(|t| t.status))
            .unwrap_or(crate::session::ChatTurnStatus::Interrupted);
        let interrupt = outcome
            .interrupt_reason
            .or(Some(crate::session::ChatTurnInterruptReason::UserStop));

        let _released_stream = stream_id
            .as_deref()
            .map(|id| stream_seq::end_if_stream(&session_id, id))
            .unwrap_or(false);

        stream_broadcast::broadcast_stream_end(
            &session_id,
            stream_id.as_deref(),
            Some(&turn_id),
            Some(status),
            interrupt,
            None,
        );
        if status.is_terminal() {
            active_turn::force_release(&session_id, &turn_id);
        }
    });
}
