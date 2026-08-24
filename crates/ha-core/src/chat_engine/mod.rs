pub mod active_persisters;
pub mod active_turn;
pub mod context;
// pub：ha-acp 的 ACP stdio server 复用 StreamCoordinator（流持久化协调）。
pub mod durability;
mod engine;
pub mod finalize;
// `im_error_message`：IM 侧引擎错误文案，dispatcher（ha-channel）渲染用。
pub mod im_error_message;
pub(crate) mod im_system_message;
pub(crate) mod persister;
// `quote`：IM 引用前缀渲染，主对话镜像（ha-channel）与 kernel 共用。
pub mod quote;
pub mod sink_registry;
pub(crate) mod spool;
pub mod stop;
pub mod stream_broadcast;
pub mod stream_seq;
pub mod turn_injection;
pub mod turn_kernel;
mod types;

use crate::turn_durability::{FlushReason, TurnDurabilitySink};
use anyhow::Context as _;
use std::collections::HashSet;
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
pub use crate::agent::{resolve_plan_context_for_session, PlanResolvedContext};
pub use types::*;

/// Delete run-owned typed-resource snapshots that never crossed an Initial
/// Context durability barrier. Every journal block is verified first; any
/// checksum, sequence, or payload ambiguity fails closed and leaves files in
/// place for later inspection instead of risking deletion of committed data.
pub(crate) fn reconcile_typed_resource_snapshots(
    snapshot: &crate::session::StreamRunSnapshot,
) -> anyhow::Result<usize> {
    let mut referenced = HashSet::new();
    let mut attempts = snapshot.attempts.iter().collect::<Vec<_>>();
    attempts.sort_by_key(|attempt| attempt.attempt_no);
    if attempts
        .first()
        .is_some_and(|attempt| attempt.attempt_no != 1)
        || attempts
            .windows(2)
            .any(|pair| pair[1].attempt_no != pair[0].attempt_no.saturating_add(1))
    {
        anyhow::bail!("typed-resource stream attempt metadata is incomplete");
    }
    if snapshot.journal.iter().any(|block| {
        !attempts
            .iter()
            .any(|attempt| attempt.attempt_no == block.attempt_no)
    }) {
        anyhow::bail!("typed-resource journal has no matching attempt metadata");
    }

    for (attempt_index, attempt) in attempts.iter().enumerate() {
        let attempt_no = attempt.attempt_no;
        let mut previous = attempts[..attempt_index]
            .iter()
            .map(|prior| prior.durable_seq)
            .max()
            .unwrap_or(0);

        let mut blocks = snapshot
            .journal
            .iter()
            .filter(|block| block.attempt_no == attempt_no)
            .collect::<Vec<_>>();
        blocks.sort_by_key(|block| block.block_no);
        for block in blocks {
            if block.run_id != snapshot.run.run_id {
                anyhow::bail!("typed-resource journal run identity mismatch");
            }
            if !crate::session::verify_block(block) {
                anyhow::bail!(
                    "typed-resource journal checksum mismatch at {}..{}",
                    block.seq_start,
                    block.seq_end
                );
            }
            if block.seq_start != previous.saturating_add(1) {
                anyhow::bail!(
                    "typed-resource journal sequence gap after {} before {}",
                    previous,
                    block.seq_start
                );
            }
            let events: Vec<crate::session::JournalEvent> = serde_json::from_str(&block.payload)
                .context("parse typed-resource journal block")?;
            if events.first().map(crate::session::JournalEvent::start_seq) != Some(block.seq_start)
                || events.last().map(|event| event.seq) != Some(block.seq_end)
                || events
                    .iter()
                    .any(|event| event.start_seq() == 0 || event.start_seq() > event.seq)
                || events
                    .windows(2)
                    .any(|pair| pair[1].start_seq() != pair[0].seq.saturating_add(1))
            {
                anyhow::bail!("typed-resource journal payload range mismatch");
            }

            for journal_event in events {
                let event: serde_json::Value = serde_json::from_str(&journal_event.event)
                    .context("parse typed-resource journal event")?;
                if event.get("type").and_then(|value| value.as_str())
                    != Some("initial_context_committed")
                    || event
                        .get("resourceSnapshotVersion")
                        .and_then(|value| value.as_u64())
                        != Some(2)
                {
                    continue;
                }
                if event
                    .get("attemptNo")
                    .and_then(|value| value.as_u64())
                    .is_some_and(|event_attempt| event_attempt != u64::from(attempt_no))
                {
                    anyhow::bail!("typed-resource journal attempt identity mismatch");
                }
                let snapshots = event
                    .get("fileSnapshots")
                    .and_then(|value| value.as_array())
                    .context("typed-resource Initial Context has no snapshot array")?;
                for item in snapshots {
                    match item.get("snapshotName") {
                        None | Some(serde_json::Value::Null) => {}
                        Some(serde_json::Value::String(name)) => {
                            referenced.insert(name.clone());
                        }
                        Some(_) => {
                            anyhow::bail!("typed-resource snapshot name is not a string")
                        }
                    }
                }
            }
            previous = block.seq_end;
        }

        if previous != attempt.durable_seq {
            anyhow::bail!(
                "typed-resource journal durable sequence mismatch: attempt {} journal={} metadata={}",
                attempt_no,
                previous,
                attempt.durable_seq
            );
        }
    }

    let verified_durable = attempts
        .iter()
        .map(|attempt| attempt.durable_seq)
        .max()
        .unwrap_or(0);
    if verified_durable != snapshot.run.durable_seq {
        anyhow::bail!(
            "typed-resource run durable sequence mismatch: attempts={} run={}",
            verified_durable,
            snapshot.run.durable_seq
        );
    }

    crate::attachments::reconcile_run_typed_resource_snapshots(
        &snapshot.run.session_id,
        &snapshot.run.run_id,
        &referenced,
    )
}

/// Public-facing snapshot of a session's chat stream state. Returned by the
/// `get_session_stream_state` command so the frontend can decide whether to
/// reattach an EventBus listener for an in-flight chat after reloading.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStreamState {
    pub active: bool,
    /// Exact in-memory turn admission still exists. This can briefly outlive a
    /// terminal stream while its owner finishes safe post-turn cleanup.
    pub admission_active: bool,
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
        admission_active: active_turn.is_some(),
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

/// The engine gives an already-visible provider/tool loop six seconds for
/// cooperative cleanup.  The watchdog intentionally fires later: it is the
/// last-resort durable convergence path, not a competing normal finalizer.
pub const CHAT_STOP_WATCHDOG_GRACE: Duration = Duration::from_secs(8);
const CHAT_STOP_RECOVERY_RETRY_MIN: Duration = Duration::from_millis(250);
const CHAT_STOP_RECOVERY_RETRY_MAX: Duration = Duration::from_secs(5);

fn stop_recovery_retry_delay(failures: u32) -> Duration {
    let multiplier = 1u32 << failures.saturating_sub(1).min(4);
    CHAT_STOP_RECOVERY_RETRY_MIN
        .saturating_mul(multiplier)
        .min(CHAT_STOP_RECOVERY_RETRY_MAX)
}

async fn stopped_turn_is_latest_session_generation(
    db: &Arc<crate::session::SessionDB>,
    session_id: &str,
    turn_id: &str,
) -> bool {
    if active_turn::current(session_id).is_some_and(|active| active.turn_id != turn_id) {
        return false;
    }
    let session_id = session_id.to_string();
    let turn_id = turn_id.to_string();
    db.clone()
        .run(move |db| db.get_latest_chat_turn(&session_id))
        .await
        .ok()
        .flatten()
        .is_some_and(|latest| latest.id == turn_id)
}

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
            finalize::TerminationReason::RuntimeCancel,
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
    reason: finalize::TerminationReason,
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
    if snapshot.run.session_id != session_id || snapshot.run.turn_id.as_deref() != turn_id {
        anyhow::bail!("persistence run identity no longer matches dropped engine");
    }

    let reconciliation_snapshot = snapshot.clone();
    match crate::blocking::run_blocking(move || {
        reconcile_typed_resource_snapshots(&reconciliation_snapshot)
    })
    .await
    {
        Ok(removed) if removed > 0 => app_info!(
            "chat",
            "context_materialization_recovery",
            "removed {} uncommitted typed-resource snapshot(s) for run {}",
            removed,
            run_id
        ),
        Ok(_) => {}
        Err(error) => app_warn!(
            "chat",
            "context_materialization_recovery",
            "left typed-resource snapshots for run {} in place because reconciliation failed: {}",
            run_id,
            error
        ),
    }
    if snapshot.run.status != "running" {
        return Ok(());
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
    let terminal_status = reason.to_chat_turn_status();
    let interrupt_reason = reason.to_chat_turn_interrupt_reason();
    let is_user_stop = matches!(reason, finalize::TerminationReason::UserStop);
    history.push(serde_json::json!({
        "role": "assistant",
        "content": finalize::copy::model_marker(&reason),
    }));
    let context_json = serde_json::to_string(&history)?;
    let trailing = crate::session::trailing_text_from_journal_events(&events);
    let assistant = crate::session::journal_events_have_assistant_output(&events)
        .then(|| crate::session::NewMessage::assistant(&trailing).with_source(source));
    let recovery_event = (!is_user_stop).then(|| {
        crate::session::NewMessage::event(&finalize::copy::user_notice(&reason)).with_source(source)
    });
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
        status: terminal_status,
        interrupt_reason: Some(interrupt_reason.as_str().to_string()),
        error,
        recovery_event,
        request_plan: crate::session::RequestPlanCommit::RecoverAllForRun,
    };
    let committed = db
        .clone()
        .run(move |db| db.commit_interrupted_turn(&commit))
        .await?;

    // The DB transaction is the terminal truth. Release the visible stream
    // before best-effort spool housekeeping so a filesystem cleanup error
    // cannot leave an already-committed turn looking active forever.
    if let Some(stream_id) = snapshot.run.stream_id.as_deref() {
        let _ = stream_seq::end_if_stream(session_id, stream_id);
    }
    if source.broadcasts_to_user_ui() {
        stream_broadcast::broadcast_stream_end(
            session_id,
            snapshot.run.stream_id.as_deref(),
            turn_id,
            Some(terminal_status),
            Some(interrupt_reason),
            None,
        );
    }
    if let Some(turn_id) = turn_id {
        active_turn::force_release(session_id, turn_id);
    }

    let run_id_for_cleanup = run_id.to_string();
    let spool_cleanup = if spool.integrity_error.is_some() {
        crate::blocking::run_blocking(move || {
            crate::chat_engine::spool::quarantine(&run_id_for_cleanup)
        })
        .await
    } else {
        crate::blocking::run_blocking(move || {
            crate::chat_engine::spool::remove(&run_id_for_cleanup)
        })
        .await
    };
    if let Err(error) = spool_cleanup {
        app_warn!(
            "chat",
            "abandoned_stream_recovery",
            "Recovered run {} but could not archive/remove its spool; startup cleanup will retry: {}",
            run_id,
            error
        );
    }
    app_info!(
        "chat",
        "abandoned_stream_recovery",
        "atomically recovered abandoned run {} through seq {} assistant_id={} reason={}",
        run_id,
        committed.committed_seq,
        committed.assistant_message_id,
        interrupt_reason.as_str()
    );
    Ok(())
}

/// Resolve the persistence run owned by an exact stopped turn. Transient
/// spool/SQLite failures are retried while the caller retains the session Stop
/// gate, so a replacement turn cannot advance the context revision out from
/// under recoverable partial output. `false` means no exact run exists and the
/// caller may evaluate the one-minor legacy fallback.
async fn converge_exact_stopped_run(
    db: Arc<crate::session::SessionDB>,
    session_id: &str,
    turn_id: &str,
    source: ChatSource,
    retry_missing: bool,
) -> bool {
    let mut failures = 0u32;
    loop {
        let turn_id_for_run = turn_id.to_string();
        let exact_run = db
            .clone()
            .run(move |db| db.stream_run_snapshot_for_turn(&turn_id_for_run))
            .await;
        let failure = match exact_run {
            Ok(Some(snapshot)) if snapshot.run.status == "running" => {
                let run_id = snapshot.run.run_id;
                match converge_abandoned_stream(
                    db.clone(),
                    session_id,
                    Some(turn_id),
                    source,
                    finalize::TerminationReason::UserStop,
                    &run_id,
                )
                .await
                {
                    Ok(()) => return true,
                    Err(error) => (Some(run_id), error),
                }
            }
            Ok(Some(_)) => return true,
            Ok(None) if !retry_missing => return false,
            Ok(None) => (
                None,
                anyhow::anyhow!("expected stopped persistence run is temporarily unavailable"),
            ),
            Err(error) => (None, error),
        };

        failures = failures.saturating_add(1);
        if failures == 1 || failures.is_power_of_two() {
            let run = failure.0.as_deref().unwrap_or("unresolved");
            app_warn!(
                "chat",
                "stop_watchdog",
                "Retrying exact stopped-run recovery while admission remains closed: session={} turn={} run={} failures={} error={}",
                session_id,
                turn_id,
                run,
                failures,
                failure.1
            );
        }
        tokio::time::sleep(stop_recovery_retry_delay(failures)).await;
    }
}

pub fn spawn_user_stop_watchdog(
    db: Arc<crate::session::SessionDB>,
    session_id: String,
    turn_id: String,
    source: ChatSource,
) {
    tokio::spawn(async move {
        tokio::time::sleep(CHAT_STOP_WATCHDOG_GRACE).await;

        // Re-establish the Stop generation gate before inspecting any
        // session-scoped legacy state. If a replacement turn won the race it
        // is now visible below; otherwise no new turn can start while this
        // watchdog converges the old one.
        let _stop_cleanup_guard = active_turn::begin_stop_cleanup(&session_id);

        let mut turn_load_failures = 0u32;
        let turn = loop {
            let turn_id_for_load = turn_id.clone();
            match db
                .clone()
                .run(move |db| db.get_chat_turn(&turn_id_for_load))
                .await
            {
                Ok(Some(turn)) if !turn.status.is_terminal() => break turn,
                Ok(Some(_)) | Ok(None) => return,
                Err(error) => {
                    turn_load_failures = turn_load_failures.saturating_add(1);
                    if turn_load_failures == 1 || turn_load_failures.is_power_of_two() {
                        app_warn!(
                            "chat",
                            "stop_watchdog",
                            "Retrying stopped-turn lookup while admission remains closed: session={} turn={} failures={} error={}",
                            session_id,
                            turn_id,
                            turn_load_failures,
                            error
                        );
                    }
                    tokio::time::sleep(stop_recovery_retry_delay(turn_load_failures)).await;
                }
            }
        };
        let stream_id = turn.stream_id.clone().or_else(|| {
            active_turn::current(&session_id)
                .filter(|active| active.turn_id == turn_id)
                .and_then(|active| active.stream_id)
        });

        // New streams converge from their durable journal. The registry is
        // keyed by session for the streaming hot path, so a delayed watchdog
        // must additionally bind the coordinator to its exact turn. A newer
        // turn may already own the session after the stopped engine released.
        let active_durability = durability::active(&session_id);
        let matching_durability = active_durability.as_ref().filter(|coordinator| {
            coordinator.snapshot().turn_id.as_deref() == Some(turn_id.as_str())
        });
        if let Some(durability) = matching_durability.cloned() {
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
                    request_plan: durability.interrupted_request_plan_commit(
                        crate::session::RequestPlanResponseOutcome::CancelledAfterResponse,
                    ),
                };
                db.clone()
                    .run(move |db| db.commit_interrupted_turn(&commit))
                    .await?;
                durability
                    .finalize_interrupted_request_after_turn_commit(
                        crate::session::RequestPlanResponseOutcome::CancelledAfterResponse,
                    )
                    .await?;
                Ok::<(), anyhow::Error>(())
            }
            .await;

            match convergence {
                Ok(()) => {
                    durability.mark_interrupted("interrupted");
                    let _released_stream = stream_id
                        .as_deref()
                        .map(|id| stream_seq::end_if_stream(&session_id, id))
                        .unwrap_or(false);
                    stream_broadcast::broadcast_stream_end(
                        &session_id,
                        stream_id.as_deref(),
                        Some(&turn_id),
                        Some(crate::session::ChatTurnStatus::Interrupted),
                        Some(crate::session::ChatTurnInterruptReason::UserStop),
                        None,
                    );
                    active_turn::force_release(&session_id, &turn_id);
                    return;
                }
                Err(convergence_error) if durability.is_persistent() => {
                    app_warn!(
                        "chat",
                        "stop_watchdog",
                        "Live stopped-run convergence failed; retaining admission gate for exact recovery: session={} turn={} run={} error={}",
                        session_id,
                        turn_id,
                        durability.persistence_run_id(),
                        convergence_error
                    );
                    // Unregister the failed live coordinator, but leave its DB
                    // run in `running` so exact journal recovery can converge
                    // it without exposing a replacement-turn window.
                    durability.mark_interrupted("recovering");
                    let _recovered =
                        converge_exact_stopped_run(db.clone(), &session_id, &turn_id, source, true)
                            .await;
                    return;
                }
                Err(convergence_error) => {
                    let message =
                        format!("stop persistence convergence failed: {convergence_error}");
                    durability.mark_interrupted("failed");
                    let _released_stream = stream_id
                        .as_deref()
                        .map(|id| stream_seq::end_if_stream(&session_id, id))
                        .unwrap_or(false);
                    stream_broadcast::broadcast_stream_end(
                        &session_id,
                        stream_id.as_deref(),
                        Some(&turn_id),
                        Some(crate::session::ChatTurnStatus::Failed),
                        Some(crate::session::ChatTurnInterruptReason::Unknown),
                        Some(&message),
                    );
                    active_turn::force_release(&session_id, &turn_id);
                    return;
                }
            }
        }

        // A different live coordinator proves this is not a legacy stream.
        // Never recover or reconstruct session-wide state across its boundary.
        if active_durability.is_some() {
            return;
        }

        // The old coordinator may already have been force-dropped and
        // unregistered. Recover its run by turn id rather than touching the
        // session's current coordinator or latest run. The Stop gate remains
        // held across transient failures and retry backoff.
        if converge_exact_stopped_run(db.clone(), &session_id, &turn_id, source, false).await {
            return;
        }

        // Everything below is the one-minor legacy fallback and is keyed only
        // by session. It is safe solely while the stopped turn is still the
        // latest generation and no different turn is active. The cleanup gate
        // above makes this check stable until fallback finalization completes.
        if !stopped_turn_is_latest_session_generation(&db, &session_id, &turn_id).await {
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

    #[test]
    fn typed_resource_reconciliation_keeps_refs_from_every_verified_attempt() {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", dir.path())], || {
            let db =
                SessionDB::open_ephemeral_for_test(&dir.path().join("snapshots.db")).expect("db");
            let session = db
                .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
                .expect("session");
            let run_id = uuid::Uuid::new_v4().to_string();
            let registration = db
                .create_stream_run(&CreateStreamRun {
                    run_id: run_id.clone(),
                    session_id: session.id.clone(),
                    source: "http".to_string(),
                    stream_id: None,
                    turn_id: None,
                    provider_shape: Some("anthropic".to_string()),
                })
                .expect("run");
            db.begin_stream_attempt(
                &run_id,
                1,
                Some("provider-1"),
                Some("model-1"),
                Some("anthropic"),
            )
            .expect("attempt 1");
            let owner = uuid::Uuid::parse_str(&run_id).expect("uuid").simple();
            let kept_name = format!("context-snapshot-run_{owner}-resource_ref_kept-selected.txt");
            db.append_stream_journal_batch(&JournalBatch {
                run_id: run_id.clone(),
                attempt_no: 1,
                block_no: 1,
                seq_start: 1,
                seq_end: 1,
                events: vec![JournalEvent::single(
                    1,
                    serde_json::json!({
                        "type": "initial_context_committed",
                        "attemptNo": 1,
                        "resourceSnapshotVersion": 2,
                        "fileSnapshots": [{"snapshotName": &kept_name}],
                    })
                    .to_string(),
                )],
            })
            .expect("attempt 1 journal");
            db.supersede_stream_attempt(
                &run_id,
                1,
                registration.context_revision,
                None,
                Some("retry"),
            )
            .expect("supersede attempt 1");
            db.begin_stream_attempt(
                &run_id,
                2,
                Some("provider-2"),
                Some("model-2"),
                Some("openai_chat"),
            )
            .expect("attempt 2");
            db.append_stream_journal_batch(&JournalBatch {
                run_id: run_id.clone(),
                attempt_no: 2,
                block_no: 1,
                seq_start: 2,
                seq_end: 2,
                events: vec![JournalEvent::single(
                    2,
                    serde_json::json!({"type": "stream_attempt_started"}).to_string(),
                )],
            })
            .expect("attempt 2 journal");

            let attachment_dir = crate::paths::attachments_dir(&session.id).expect("path");
            std::fs::create_dir_all(&attachment_dir).expect("attachment dir");
            let orphan_name =
                format!("context-snapshot-run_{owner}-resource_ref_orphan-selected.txt");
            std::fs::write(attachment_dir.join(&kept_name), b"kept").expect("kept snapshot");
            std::fs::write(attachment_dir.join(&orphan_name), b"orphan").expect("orphan snapshot");

            let snapshot = db
                .stream_run_snapshot(&run_id)
                .expect("snapshot")
                .expect("run exists");
            assert_eq!(
                reconcile_typed_resource_snapshots(&snapshot).expect("reconcile"),
                1
            );
            assert!(attachment_dir.join(&kept_name).exists());
            assert!(!attachment_dir.join(&orphan_name).exists());

            let missing_block_orphan =
                format!("context-snapshot-run_{owner}-resource_ref_missing-selected.txt");
            std::fs::write(
                attachment_dir.join(&missing_block_orphan),
                b"leave on missing journal block",
            )
            .expect("missing-block orphan");
            let mut missing_block_snapshot = snapshot.clone();
            missing_block_snapshot.journal.clear();
            assert!(reconcile_typed_resource_snapshots(&missing_block_snapshot).is_err());
            assert!(attachment_dir.join(missing_block_orphan).exists());

            let corrupt_orphan =
                format!("context-snapshot-run_{owner}-resource_ref_corrupt-selected.txt");
            std::fs::write(attachment_dir.join(&corrupt_orphan), b"leave in place")
                .expect("corrupt orphan");
            let mut corrupt_snapshot = snapshot;
            corrupt_snapshot.journal[0].checksum = "invalid".to_string();
            assert!(reconcile_typed_resource_snapshots(&corrupt_snapshot).is_err());
            assert!(attachment_dir.join(corrupt_orphan).exists());
        });
    }

    #[tokio::test]
    async fn legacy_stop_watchdog_rejects_a_completed_replacement_turn() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("legacy-generation.db"))
                .expect("db"),
        );
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("session");
        let stopped = db
            .create_chat_turn(&session.id, "desktop", None, None)
            .expect("stopped turn");
        db.finish_chat_turn_once(
            &stopped.id,
            crate::session::ChatTurnStatus::Interrupted,
            Some(crate::session::ChatTurnInterruptReason::UserStop),
            None,
            None,
        )
        .expect("finish stopped turn");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let replacement = db
            .create_chat_turn(&session.id, "desktop", None, None)
            .expect("replacement turn");
        db.finish_chat_turn_once(
            &replacement.id,
            crate::session::ChatTurnStatus::Completed,
            None,
            None,
            None,
        )
        .expect("finish replacement");

        assert!(
            !stopped_turn_is_latest_session_generation(&db, &session.id, &stopped.id).await,
            "a delayed legacy watchdog must not rebuild from a completed newer turn"
        );
    }

    #[tokio::test]
    async fn dropped_engine_materializes_only_its_durable_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("abandoned.db")).expect("db"),
        );
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
            finalize::TerminationReason::RuntimeCancel,
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
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("no-attempt.db")).expect("db"),
        );
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
            finalize::TerminationReason::RuntimeCancel,
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
