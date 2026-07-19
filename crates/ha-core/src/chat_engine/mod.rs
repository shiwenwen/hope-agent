pub mod active_persisters;
pub mod active_turn;
pub mod context;
pub(crate) mod durability;
mod engine;
pub mod finalize;
pub(crate) mod im_error_message;
pub(crate) mod im_mirror;
pub(crate) mod im_system_message;
pub(crate) mod persister;
pub(crate) mod quote;
pub mod sink_registry;
pub(crate) mod spool;
pub mod stream_broadcast;
pub mod stream_seq;
pub mod turn_injection;
mod types;

use crate::turn_durability::{FlushReason, TurnDurabilitySink};
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
    /// Backward-compatible alias for `accepted_seq`.
    pub last_seq: u64,
    pub accepted_seq: u64,
    pub durable_seq: u64,
    pub committed_seq: u64,
    pub persistence_run_id: Option<String>,
    pub stream_id: Option<String>,
    pub turn_id: Option<String>,
    pub status: Option<crate::session::ChatTurnStatus>,
    pub last_terminal_status: Option<crate::session::ChatTurnStatus>,
    pub interrupt_reason: Option<crate::session::ChatTurnInterruptReason>,
}

/// Snapshot the current stream state for a session.
pub fn session_stream_state(session_id: &str) -> SessionStreamState {
    let durable = durability::active_snapshot(session_id);
    let persisted_run = if durable.is_none() {
        crate::get_session_db().and_then(|db| db.latest_stream_run(session_id).ok().flatten())
    } else {
        None
    };
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
    let accepted_seq = durable
        .as_ref()
        .map(|snapshot| snapshot.accepted_seq)
        .or_else(|| persisted_run.as_ref().map(|run| run.accepted_seq))
        .unwrap_or_else(|| stream_seq::last_seq(session_id));
    let durable_seq = durable
        .as_ref()
        .map(|snapshot| snapshot.durable_seq)
        .or_else(|| persisted_run.as_ref().map(|run| run.durable_seq))
        .unwrap_or(0);
    let committed_seq = durable
        .as_ref()
        .map(|snapshot| snapshot.committed_seq)
        .or_else(|| persisted_run.as_ref().map(|run| run.committed_seq))
        .unwrap_or(0);
    SessionStreamState {
        active,
        last_seq: accepted_seq,
        accepted_seq,
        durable_seq,
        committed_seq,
        persistence_run_id: durable
            .as_ref()
            .map(|snapshot| snapshot.persistence_run_id.clone())
            .or_else(|| persisted_run.as_ref().map(|run| run.run_id.clone())),
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

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStreamSnapshot {
    pub session_id: String,
    pub stream_id: Option<String>,
    pub turn_id: Option<String>,
    pub persistence_run_id: String,
    pub through_seq: u64,
    pub durable_seq: u64,
    pub committed_seq: u64,
    pub status: String,
    pub blocks: Vec<serde_json::Value>,
    pub usage: Option<serde_json::Value>,
    /// Ordered durable events are included for the additive frontend replay
    /// handshake. `blocks` is the convenient full snapshot representation.
    pub events: Vec<crate::session::JournalEvent>,
}

pub fn session_stream_snapshot(session_id: &str) -> anyhow::Result<Option<SessionStreamSnapshot>> {
    let (stream_id, turn_id, run_id, through_seq, durable_seq, committed_seq, status, events) =
        if let Some(snapshot) = durability::active_snapshot(session_id) {
            let through_seq = snapshot
                .events
                .last()
                .map(|event| event.seq)
                .unwrap_or(snapshot.durable_seq);
            (
                snapshot.stream_id,
                snapshot.turn_id,
                snapshot.persistence_run_id,
                through_seq,
                snapshot.durable_seq,
                snapshot.committed_seq,
                snapshot.status,
                snapshot.events,
            )
        } else {
            let Some(db) = crate::get_session_db() else {
                return Ok(None);
            };
            let Some(snapshot) = db.latest_stream_run_snapshot(session_id)? else {
                return Ok(None);
            };
            let (_, through_seq, events, _) =
                crate::session::select_recoverable_attempt_prefix(&snapshot);
            (
                snapshot.run.stream_id,
                snapshot.run.turn_id,
                snapshot.run.run_id,
                through_seq,
                snapshot.run.durable_seq,
                snapshot.run.committed_seq,
                snapshot.run.status,
                events,
            )
        };
    let (blocks, usage) = snapshot_blocks(&events);
    Ok(Some(SessionStreamSnapshot {
        session_id: session_id.to_string(),
        stream_id,
        turn_id,
        persistence_run_id: run_id,
        through_seq,
        durable_seq,
        committed_seq,
        status,
        blocks,
        usage,
        events,
    }))
}

fn snapshot_blocks(
    events: &[crate::session::JournalEvent],
) -> (Vec<serde_json::Value>, Option<serde_json::Value>) {
    let mut blocks: Vec<serde_json::Value> = Vec::new();
    let mut usage = None;
    for journal_event in events {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&journal_event.event) else {
            continue;
        };
        match event.get("type").and_then(|value| value.as_str()) {
            Some(kind @ ("text_delta" | "thinking_delta")) => {
                let block_type = if kind == "text_delta" {
                    "text"
                } else {
                    "thinking"
                };
                let content = event
                    .get("content")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                if let Some(last) = blocks.last_mut().filter(|last| {
                    last.get("type").and_then(|value| value.as_str()) == Some(block_type)
                }) {
                    if let Some(existing) = last.get_mut("content").and_then(|value| value.as_str())
                    {
                        let mut merged = existing.to_string();
                        merged.push_str(content);
                        last["content"] = serde_json::json!(merged);
                        last["seqEnd"] = serde_json::json!(journal_event.seq);
                    }
                } else {
                    blocks.push(serde_json::json!({
                        "type": block_type,
                        "content": content,
                        "seqStart": journal_event.start_seq(),
                        "seqEnd": journal_event.seq,
                    }));
                }
            }
            Some("tool_call") => blocks.push(serde_json::json!({
                "type": "tool",
                "callId": event.get("call_id"),
                "name": event.get("name"),
                "arguments": event.get("arguments"),
                "seqStart": journal_event.seq,
                "seqEnd": journal_event.seq,
            })),
            Some("tool_result") => {
                let call_id = event.get("call_id").and_then(|value| value.as_str());
                if let Some(block) = blocks.iter_mut().rev().find(|block| {
                    block.get("type").and_then(|value| value.as_str()) == Some("tool")
                        && block.get("callId").and_then(|value| value.as_str()) == call_id
                }) {
                    block["result"] = event.get("result").cloned().unwrap_or_default();
                    block["durationMs"] = event.get("duration_ms").cloned().unwrap_or_default();
                    block["isError"] = event.get("is_error").cloned().unwrap_or_default();
                    block["toolMetadata"] = event.get("tool_metadata").cloned().unwrap_or_default();
                    block["mediaItems"] = event.get("media_items").cloned().unwrap_or_default();
                    block["seqEnd"] = serde_json::json!(journal_event.seq);
                }
            }
            Some("usage") => match (&mut usage, event) {
                (Some(serde_json::Value::Object(existing)), serde_json::Value::Object(next)) => {
                    existing.extend(next);
                }
                (slot, next) => *slot = Some(next),
            },
            _ => {}
        }
    }
    (blocks, usage)
}

pub const CHAT_STOP_WATCHDOG_GRACE: Duration = Duration::from_secs(5);

/// Recover a unified persistence run when its owning async engine future is
/// dropped before it can execute the normal finalizer (HTTP disconnect,
/// runtime task cancellation, or an unwinding panic). Only journal/spool
/// bytes that were already acknowledged as durable are materialized.
pub fn spawn_abandoned_stream_recovery(
    db: Arc<crate::session::SessionDB>,
    session_id: String,
    turn_id: Option<String>,
    source: ChatSource,
    persistence_run_id: String,
) {
    if tokio::runtime::Handle::try_current().is_err() {
        // No runtime is available during process teardown. The same run stays
        // `running` and startup recovery will converge it on the next launch.
        return;
    }
    tokio::spawn(async move {
        let result = converge_abandoned_stream(
            db.clone(),
            &session_id,
            turn_id.as_deref(),
            source,
            &persistence_run_id,
        )
        .await;
        if let Err(error) = result {
            app_warn!(
                "chat",
                "abandoned_stream_recovery",
                "run {} remains recoverable after runtime cancellation: {}",
                persistence_run_id,
                error
            );
        }
    });
}

async fn converge_abandoned_stream(
    db: Arc<crate::session::SessionDB>,
    session_id: &str,
    turn_id: Option<&str>,
    source: ChatSource,
    run_id: &str,
) -> anyhow::Result<()> {
    // Import any frames that were acknowledged through the emergency spool
    // before the engine future disappeared. A damaged tail never hides its
    // checksum-valid prefix.
    let run_id_for_spool = run_id.to_string();
    let spool = crate::blocking::run_blocking(move || {
        crate::chat_engine::spool::read_batches(&run_id_for_spool)
    })
    .await?;
    let spool_integrity_error = spool.integrity_error.clone();
    if !spool.batches.is_empty() {
        let grouped = spool.batches.clone();
        let grouped_db = db.clone();
        if grouped_db
            .run(move |db| db.append_stream_journal_batches(&grouped))
            .await
            .is_err()
        {
            for batch in spool.batches {
                let batch_db = db.clone();
                if batch_db
                    .run(move |db| db.append_stream_journal_batch(&batch))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    }

    let run_id_for_snapshot = run_id.to_string();
    let snapshot = db
        .clone()
        .run(move |db| db.stream_run_snapshot(&run_id_for_snapshot))
        .await?
        .ok_or_else(|| anyhow::anyhow!("persistence run disappeared"))?;
    if snapshot.run.status != "running" {
        return Ok(());
    }
    if snapshot.run.session_id != session_id || snapshot.run.turn_id.as_deref() != turn_id {
        anyhow::bail!("persistence run identity no longer matches dropped engine");
    }

    let (attempt_no, through_seq, events, journal_error) =
        crate::session::select_recoverable_attempt_prefix(&snapshot);
    let provider_kind = snapshot
        .attempts
        .iter()
        .find(|attempt| attempt.attempt_no == attempt_no)
        .and_then(|attempt| attempt.provider_shape.as_deref())
        .or(snapshot.run.provider_shape.as_deref())
        .and_then(finalize::ProviderApiKind::from_shape);

    let run_id_for_context = run_id.to_string();
    let (stored_context, checkpoint_seq, context_revision) = db
        .clone()
        .run(move |db| db.recovery_context_for_prefix(&run_id_for_context, attempt_no, through_seq))
        .await?;
    let mut history: Vec<serde_json::Value> = stored_context
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();
    finalize::rebuild::append_journal_suffix_to_history(
        &mut history,
        &events,
        checkpoint_seq,
        provider_kind,
    )?;
    let reason = finalize::TerminationReason::RuntimeCancel;
    history.push(serde_json::json!({
        "role": "assistant",
        "content": finalize::copy::model_marker(&reason),
    }));
    let context_json = serde_json::to_string(&history)?;
    let trailing = crate::session::trailing_text_from_journal_events(&events);
    let assistant = crate::session::journal_events_have_assistant_output(&events)
        .then(|| crate::session::NewMessage::assistant(&trailing).with_source(source));
    let recovery_event = Some(
        crate::session::NewMessage::event(&finalize::copy::user_notice(&reason))
            .with_source(source),
    );
    let error = journal_error.or(spool_integrity_error);
    let commit = crate::session::CommitInterruptedTurn {
        run_id: Some(run_id.to_string()),
        attempt_no,
        session_id: session_id.to_string(),
        assistant,
        context_json,
        expected_context_revision: context_revision,
        turn_id: turn_id.map(ToOwned::to_owned),
        final_seq: through_seq,
        status: crate::session::ChatTurnStatus::Interrupted,
        interrupt_reason: Some("runtime_cancel".to_string()),
        error,
        recovery_event,
    };
    let committed = db
        .clone()
        .run(move |db| db.commit_interrupted_turn(&commit))
        .await?;

    let run_id_for_cleanup = run_id.to_string();
    if spool.integrity_error.is_some() {
        crate::blocking::run_blocking(move || {
            crate::chat_engine::spool::quarantine(&run_id_for_cleanup)
        })
        .await?;
    } else {
        crate::blocking::run_blocking(move || {
            crate::chat_engine::spool::remove(&run_id_for_cleanup)
        })
        .await?;
    }

    if let Some(stream_id) = snapshot.run.stream_id.as_deref() {
        let _ = stream_seq::end_if_stream(session_id, stream_id);
    }
    if source.broadcasts_to_user_ui() {
        stream_broadcast::broadcast_stream_end(
            session_id,
            snapshot.run.stream_id.as_deref(),
            turn_id,
            Some(crate::session::ChatTurnStatus::Interrupted),
            Some(crate::session::ChatTurnInterruptReason::RuntimeCancel),
            None,
        );
    }
    if let Some(turn_id) = turn_id {
        active_turn::force_release(session_id, turn_id);
    }
    app_info!(
        "chat",
        "abandoned_stream_recovery",
        "atomically recovered runtime-cancelled run {} through seq {} assistant_id={}",
        run_id,
        committed.committed_seq,
        committed.assistant_message_id
    );
    Ok(())
}

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

        // New streams converge from their durable journal. This branch races
        // safely with the normal engine finalizer: context revision + turn
        // status CAS allow exactly one transaction to win.
        if let Some(durability) = durability::active(&session_id) {
            let convergence = async {
                let durable_seq = durability.flush(FlushReason::Stop).await?;
                durability.reconcile_spool_to_sqlite().await?;
                let (attempt_no, final_seq, visible_events, provider_kind) =
                    if durability.is_persistent() {
                        let run_id = durability.persistence_run_id().to_string();
                        let snapshot = db
                            .clone()
                            .run(move |db| db.stream_run_snapshot(&run_id))
                            .await?
                            .ok_or_else(|| anyhow::anyhow!("stop persistence run disappeared"))?;
                        let (attempt_no, through_seq, events, integrity_error) =
                            crate::session::select_recoverable_attempt_prefix(&snapshot);
                        if let Some(error) = integrity_error {
                            anyhow::bail!("stop journal integrity error: {error}");
                        }
                        let provider_kind = snapshot
                            .attempts
                            .iter()
                            .find(|attempt| attempt.attempt_no == attempt_no)
                            .and_then(|attempt| attempt.provider_shape.as_deref())
                            .or(snapshot.run.provider_shape.as_deref())
                            .and_then(finalize::ProviderApiKind::from_shape);
                        (attempt_no, through_seq, events, provider_kind)
                    } else {
                        let snapshot = durability.snapshot();
                        (
                            durability.current_attempt_no(),
                            durable_seq,
                            snapshot.events,
                            durability
                                .current_provider_shape()
                                .as_deref()
                                .and_then(finalize::ProviderApiKind::from_shape),
                        )
                    };
                let (stored_context, context_checkpoint_seq, context_revision) =
                    if durability.is_persistent() {
                        let run_id = durability.persistence_run_id().to_string();
                        db.clone()
                            .run(move |db| {
                                db.recovery_context_for_prefix(&run_id, attempt_no, final_seq)
                            })
                            .await?
                    } else {
                        let session_id_for_context = session_id.clone();
                        let (context, revision) = db
                            .clone()
                            .run(move |db| db.load_context_with_revision(&session_id_for_context))
                            .await?;
                        (context, 0, revision)
                    };
                let reason = finalize::TerminationReason::UserStop;
                let mut history: Vec<serde_json::Value> = stored_context
                    .as_deref()
                    .and_then(|json| serde_json::from_str(json).ok())
                    .unwrap_or_default();
                let trailing = crate::session::trailing_text_from_journal_events(&visible_events);
                finalize::rebuild::append_journal_suffix_to_history(
                    &mut history,
                    &visible_events,
                    context_checkpoint_seq,
                    provider_kind,
                )?;
                history.push(serde_json::json!({
                    "role": "assistant",
                    "content": finalize::copy::model_marker(&reason),
                }));
                let context_json = serde_json::to_string(&history)?;
                let assistant = crate::session::journal_events_have_assistant_output(
                    &visible_events,
                )
                .then(|| {
                    let usage = durability.usage();
                    let mut message =
                        crate::session::NewMessage::assistant(&trailing).with_source(source);
                    message.tokens_in = usage.input_tokens;
                    message.tokens_out = usage.output_tokens;
                    message.tokens_in_last =
                        usage.last_context_input_tokens.or(usage.last_input_tokens);
                    message.model = usage.model;
                    message.ttft_ms = usage.ttft_ms;
                    message
                });
                let commit = crate::session::CommitInterruptedTurn {
                    run_id: durability
                        .is_persistent()
                        .then(|| durability.persistence_run_id().to_string()),
                    attempt_no,
                    session_id: session_id.clone(),
                    assistant,
                    context_json,
                    expected_context_revision: context_revision,
                    turn_id: Some(turn_id.clone()),
                    final_seq,
                    status: crate::session::ChatTurnStatus::Interrupted,
                    interrupt_reason: Some("user_stop".to_string()),
                    error: None,
                    recovery_event: None,
                };
                db.clone()
                    .run(move |db| db.commit_interrupted_turn(&commit))
                    .await?;
                Ok::<(), anyhow::Error>(())
            }
            .await;

            let (status, interrupt, error) = match convergence {
                Ok(()) => {
                    durability.mark_interrupted("interrupted");
                    (
                        crate::session::ChatTurnStatus::Interrupted,
                        Some(crate::session::ChatTurnInterruptReason::UserStop),
                        None,
                    )
                }
                Err(convergence_error) => {
                    let message =
                        format!("stop persistence convergence failed: {convergence_error}");
                    // Keep the DB run recoverable. Terminalizing it without
                    // materializing the journal would make already displayed
                    // bytes unreachable on restart.
                    durability.mark_interrupted("failed");
                    (
                        crate::session::ChatTurnStatus::Failed,
                        Some(crate::session::ChatTurnInterruptReason::Unknown),
                        Some(message),
                    )
                }
            };
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
                error.as_deref(),
            );
            active_turn::force_release(&session_id, &turn_id);
            return;
        }

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

#[cfg(test)]
mod abandoned_stream_tests {
    use super::*;
    use crate::session::{CreateStreamRun, JournalBatch, JournalEvent, NewMessage, SessionDB};

    #[tokio::test]
    async fn dropped_engine_materializes_only_its_durable_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(SessionDB::open(&dir.path().join("abandoned.db")).expect("db"));
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("session");
        let user_id = db
            .append_message(&session.id, &NewMessage::user("hello"))
            .expect("user");
        let turn = db
            .create_chat_turn(&session.id, "http", Some("stream-drop"), Some(user_id))
            .expect("turn");
        let run_id = uuid::Uuid::new_v4().to_string();
        db.create_stream_run(&CreateStreamRun {
            run_id: run_id.clone(),
            session_id: session.id.clone(),
            source: "http".to_string(),
            stream_id: Some("stream-drop".to_string()),
            turn_id: Some(turn.id.clone()),
            provider_shape: Some("anthropic".to_string()),
        })
        .expect("run");
        db.begin_stream_attempt(
            &run_id,
            1,
            Some("provider"),
            Some("model"),
            Some("anthropic"),
        )
        .expect("attempt");
        db.append_stream_journal_batch(&JournalBatch {
            run_id: run_id.clone(),
            attempt_no: 1,
            block_no: 1,
            seq_start: 1,
            seq_end: 1,
            events: vec![JournalEvent::single(
                1,
                serde_json::json!({"type":"text_delta","content":"durable tail"}).to_string(),
            )],
        })
        .expect("journal");

        converge_abandoned_stream(
            db.clone(),
            &session.id,
            Some(&turn.id),
            ChatSource::Http,
            &run_id,
        )
        .await
        .expect("converge");

        let snapshot = db
            .stream_run_snapshot(&run_id)
            .expect("snapshot")
            .expect("run exists");
        assert_eq!(snapshot.run.status, "interrupted");
        assert_eq!(snapshot.run.committed_seq, 1);
        assert_eq!(snapshot.attempts[0].status, "interrupted");
        let persisted_turn = db
            .get_chat_turn(&turn.id)
            .expect("turn")
            .expect("turn exists");
        assert_eq!(
            persisted_turn.status,
            crate::session::ChatTurnStatus::Interrupted
        );
        let messages = db.load_session_messages(&session.id).expect("messages");
        assert!(messages.iter().any(|message| {
            message.role == crate::session::MessageRole::Assistant
                && message.content == "durable tail"
        }));
        let context = db
            .load_context(&session.id)
            .expect("context")
            .expect("context exists");
        assert!(context.contains("运行任务被取消"));
    }

    #[tokio::test]
    async fn dropped_engine_before_first_attempt_still_closes_run_atomically() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(SessionDB::open(&dir.path().join("no-attempt.db")).expect("db"));
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("session");
        let turn = db
            .create_chat_turn(&session.id, "http", None, None)
            .expect("turn");
        let run_id = uuid::Uuid::new_v4().to_string();
        db.create_stream_run(&CreateStreamRun {
            run_id: run_id.clone(),
            session_id: session.id.clone(),
            source: "http".to_string(),
            stream_id: None,
            turn_id: Some(turn.id.clone()),
            provider_shape: None,
        })
        .expect("run");

        converge_abandoned_stream(
            db.clone(),
            &session.id,
            Some(&turn.id),
            ChatSource::Http,
            &run_id,
        )
        .await
        .expect("converge without attempt");

        let snapshot = db
            .stream_run_snapshot(&run_id)
            .expect("snapshot")
            .expect("run exists");
        assert_eq!(snapshot.run.status, "interrupted");
        assert_eq!(snapshot.run.committed_seq, 0);
        assert!(snapshot.attempts.is_empty());
    }
}
