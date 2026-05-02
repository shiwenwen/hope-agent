use anyhow::Result;
use chrono::Utc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::db::CronDB;
use super::delivery::{deliver_results, DeliveryOutcome};
use super::types::*;

/// Public wrapper for execute_job, callable from Tauri commands.
pub async fn execute_job_public(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<crate::session::SessionDB>,
    job: &CronJob,
) {
    match cron_db.claim_immediate_job_for_execution(job) {
        Ok(Some(claimed)) => execute_claimed_job(cron_db, session_db, claimed).await,
        Ok(None) => {
            app_warn!(
                "cron",
                "executor",
                "Job '{}' ({}) is already running, skipping",
                job.name,
                job.id
            );
        }
        Err(e) => {
            app_error!(
                "cron",
                "executor",
                "Failed to claim job '{}': {}",
                job.name,
                e
            );
        }
    }
}

/// Execute a job whose running marker was already claimed by the DB.
pub(crate) async fn execute_claimed_job(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<crate::session::SessionDB>,
    claimed: ClaimedCronJob,
) {
    let start_time = std::time::Instant::now();
    let started_at = claimed.claimed_at.clone();
    let job = claimed.job;

    app_info!(
        "cron",
        "executor",
        "Executing job '{}' ({})",
        job.name,
        job.id
    );

    // Extract prompt and agent_id from payload
    let (prompt, agent_id) = match &job.payload {
        CronPayload::AgentTurn { prompt, agent_id } => (
            prompt.clone(),
            agent_id.clone().unwrap_or_else(|| "default".to_string()),
        ),
    };

    // Create an isolated session for this cron run
    let session_id = match session_db.create_session(&agent_id) {
        Ok(meta) => {
            let _ = session_db.update_session_title(&meta.id, &job.name);
            let _ = session_db.mark_session_cron(&meta.id);
            meta.id
        }
        Err(e) => {
            app_error!(
                "cron",
                "executor",
                "Failed to create session for job '{}': {}",
                job.name,
                e
            );
            record_failure(
                cron_db,
                &job,
                &started_at,
                start_time,
                "no_session",
                &e.to_string(),
                "",
            );
            return;
        }
    };

    // Persist the cron prompt before execution so `run_chat_engine` can reuse
    // the same DB contract as interactive chat without duplicating user rows.
    let mut user_msg = crate::session::NewMessage::user(&prompt);
    user_msg.attachments_meta = Some(
        serde_json::json!({
            "cron_trigger": {
                "job_id": &job.id,
                "job_name": &job.name,
            }
        })
        .to_string(),
    );
    let _ = session_db.append_message(&session_id, &user_msg);

    // Build agent from app config (with 5-minute timeout to prevent blocking scheduler)
    const CRON_JOB_TIMEOUT_SECS: u64 = 300;
    let cancel_flag = super::cancel::register(&job.id);
    let result = match tokio::time::timeout(
        std::time::Duration::from_secs(CRON_JOB_TIMEOUT_SECS),
        build_and_run_agent_with_cancel(
            &agent_id,
            &prompt,
            &session_id,
            session_db,
            cancel_flag.clone(),
        ),
    )
    .await
    {
        Ok(r) => r,
        Err(_) => {
            app_error!(
                "cron",
                "executor",
                "Job '{}' timed out after {}s",
                job.name,
                CRON_JOB_TIMEOUT_SECS
            );
            Err(anyhow::anyhow!(
                "Cron job timed out after {}s",
                CRON_JOB_TIMEOUT_SECS
            ))
        }
    };

    let duration_ms = start_time.elapsed().as_millis() as u64;
    let finished_at = Utc::now().to_rfc3339();
    let was_cancelled = cancel_flag.load(Ordering::SeqCst);
    super::cancel::remove(&job.id);

    if was_cancelled {
        app_warn!(
            "cron",
            "executor",
            "Job '{}' ({}) cancelled after {}ms",
            job.name,
            job.id,
            duration_ms
        );
        record_cancelled(cron_db, &job, &started_at, duration_ms, &session_id);
        return;
    }

    match result {
        Ok(response) => {
            app_info!(
                "cron",
                "executor",
                "Job '{}' completed successfully ({}ms)",
                job.name,
                duration_ms
            );

            // Record success run log
            let preview = if response.len() > 500 {
                Some(crate::truncate_utf8(&response, 500).to_string())
            } else {
                Some(response.clone())
            };
            let run_log = CronRunLog {
                id: 0,
                job_id: job.id.clone(),
                session_id: session_id.clone(),
                status: "success".to_string(),
                started_at,
                finished_at: Some(finished_at),
                duration_ms: Some(duration_ms),
                result_preview: preview,
                error: None,
            };
            let _ = cron_db.add_run_log(&run_log);
            let _ = cron_db.update_after_run(&job.id, true, &job.schedule);

            deliver_results(&job, DeliveryOutcome::Success { text: &response }).await;

            let _ = cron_db.clear_running(&job.id);

            // Emit Tauri event
            emit_cron_event(&job.id, &job.name, "success", job.notify_on_complete);
        }
        Err(e) => {
            app_error!("cron", "executor", "Job '{}' failed: {}", job.name, e);
            let err_text = e.to_string();
            persist_failure_message_if_missing(session_db, &session_id, &err_text);

            // Notify IM channel targets of the failure before bookkeeping.
            deliver_results(&job, DeliveryOutcome::Failure { error: &err_text }).await;

            record_failure(
                cron_db,
                &job,
                &started_at,
                start_time,
                "error",
                &err_text,
                &session_id,
            );
        }
    }
}

/// Build an AssistantAgent and run a chat message with full failover logic.
///
/// Cron now delegates to the shared chat engine so provider auth, Codex OAuth,
/// failover, compaction, and persistence stay aligned with interactive chat.
pub async fn build_and_run_agent_with_cancel(
    agent_id: &str,
    message: &str,
    session_id: &str,
    session_db: &Arc<crate::session::SessionDB>,
    cancel: Arc<AtomicBool>,
) -> Result<String> {
    build_and_run_agent_with_context(
        agent_id,
        message,
        session_id,
        session_db,
        None,
        Some(cancel),
    )
    .await
}

/// Build an AssistantAgent and run a chat message via the shared chat engine
/// with optional extra system context.
pub async fn build_and_run_agent_with_context(
    agent_id: &str,
    message: &str,
    session_id: &str,
    session_db: &Arc<crate::session::SessionDB>,
    extra_system_context: Option<&str>,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<String> {
    use crate::provider;

    // Load app config from disk
    let store = crate::config::cached_config();

    // Load agent config for model resolution
    let agent_model_config = crate::agent_loader::load_agent(agent_id)
        .map(|def| def.config.model)
        .unwrap_or_default();

    let (primary, fallbacks) = provider::resolve_model_chain(&agent_model_config, &store);

    // Build model chain
    let mut model_chain = Vec::new();
    if let Some(p) = primary {
        model_chain.push(p);
    }
    for fb in fallbacks {
        if !model_chain
            .iter()
            .any(|m| m.provider_id == fb.provider_id && m.model_id == fb.model_id)
        {
            model_chain.push(fb);
        }
    }

    if model_chain.is_empty() {
        return Err(anyhow::anyhow!(
            "No model configured for cron job execution"
        ));
    }

    let agent_def = crate::agent_loader::load_agent(agent_id).ok();
    let engine_params = crate::chat_engine::ChatEngineParams {
        session_id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        message: message.to_string(),
        attachments: Vec::new(),
        session_db: session_db.clone(),
        model_chain,
        providers: store.providers.clone(),
        codex_token: None,
        resolved_temperature: agent_def
            .as_ref()
            .and_then(|def| def.config.model.temperature)
            .or(store.temperature),
        compact_config: store.compact.clone(),
        extra_system_context: Some(
            extra_system_context
                .unwrap_or(
                    "## Execution Context\n\
                 You are running as a **scheduled task** (cron job), not an interactive chat.\n\
                 - No user is actively waiting — execute the prompt directly and concisely.\n\
                 - This is an isolated session with no prior conversation history.\n\
                 - Focus on completing the task described in the user message.",
                )
                .to_string(),
        ),
        reasoning_effort: crate::agent::live_reasoning_effort(None).await,
        cancel: cancel.unwrap_or_else(|| Arc::new(AtomicBool::new(false))),
        plan_context_override: None,
        skill_allowed_tools: Vec::new(),
        denied_tools: Vec::new(),
        subagent_depth: 0,
        steer_run_id: None,
        auto_approve_tools: false,
        follow_global_reasoning_effort: true,
        post_turn_effects: true,
        abort_on_cancel: false,
        persist_final_error_event: true,
        // Cron is a background/non-interactive runner. Reuse the channel bucket
        // until the status UI grows a dedicated cron source.
        source: crate::chat_engine::stream_seq::ChatSource::Channel,
        event_sink: Arc::new(crate::chat_engine::NoopEventSink),
    };

    match crate::chat_engine::run_chat_engine(engine_params).await {
        Ok(result) => Ok(result.response),
        Err(e) => Err(anyhow::anyhow!("{}", e)),
    }
}

pub fn cancel_running_job(job_id: &str) -> Result<Option<bool>> {
    let Some(cron_db) = crate::get_cron_db() else {
        return Ok(None);
    };
    let Some(job) = cron_db.get_job(job_id)? else {
        return Ok(None);
    };
    if job.running_at.is_none() {
        return Ok(Some(false));
    }
    Ok(Some(super::cancel::cancel(job_id)))
}

fn persist_failure_message_if_missing(
    session_db: &Arc<crate::session::SessionDB>,
    session_id: &str,
    err_text: &str,
) {
    let already_persisted = session_db
        .load_session_messages_latest(session_id, 1)
        .ok()
        .and_then(|(msgs, _, _)| msgs.last().cloned())
        .map(|msg| msg.content == err_text)
        .unwrap_or(false);

    if already_persisted {
        return;
    }

    let mut err_msg = crate::session::NewMessage::assistant(err_text);
    err_msg.is_error = Some(true);
    let _ = session_db.append_message(session_id, &err_msg);
}

/// Record a failure run log and update job state.
pub(crate) fn record_failure(
    cron_db: &Arc<CronDB>,
    job: &CronJob,
    started_at: &str,
    start_time: std::time::Instant,
    status: &str,
    error: &str,
    session_id: &str,
) {
    let duration_ms = start_time.elapsed().as_millis() as u64;
    let finished_at = Utc::now().to_rfc3339();

    let run_log = CronRunLog {
        id: 0,
        job_id: job.id.clone(),
        session_id: session_id.to_string(),
        status: status.to_string(),
        started_at: started_at.to_string(),
        finished_at: Some(finished_at),
        duration_ms: Some(duration_ms),
        result_preview: None,
        error: Some(error.to_string()),
    };
    let _ = cron_db.add_run_log(&run_log);
    let _ = cron_db.update_after_run(&job.id, false, &job.schedule);
    let _ = cron_db.clear_running(&job.id);

    // Emit Tauri event
    emit_cron_event(&job.id, &job.name, "error", job.notify_on_complete);
}

fn record_cancelled(
    cron_db: &Arc<CronDB>,
    job: &CronJob,
    started_at: &str,
    duration_ms: u64,
    session_id: &str,
) {
    let finished_at = Utc::now().to_rfc3339();
    let run_log = CronRunLog {
        id: 0,
        job_id: job.id.clone(),
        session_id: session_id.to_string(),
        status: "cancelled".to_string(),
        started_at: started_at.to_string(),
        finished_at: Some(finished_at),
        duration_ms: Some(duration_ms),
        result_preview: None,
        error: Some("Cancelled by user".to_string()),
    };
    let _ = cron_db.add_run_log(&run_log);
    let _ = cron_db.clear_running(&job.id);
    emit_cron_event(&job.id, &job.name, "cancelled", job.notify_on_complete);
}

/// Emit an event to notify the frontend of a cron run result.
pub(crate) fn emit_cron_event(job_id: &str, job_name: &str, status: &str, notify: bool) {
    if let Some(bus) = crate::get_event_bus() {
        let payload = serde_json::json!({
            "job_id": job_id,
            "job_name": job_name,
            "status": status,
            "notify": notify,
        });
        bus.emit("cron:run_completed", payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cron::{CronPayload, CronSchedule, NewCronJob};
    use rusqlite::params;
    use std::path::{Path, PathBuf};
    use uuid::Uuid;

    fn temp_db_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "hope-agent-cron-executor-{label}-{}.db",
            Uuid::new_v4()
        ))
    }

    fn cleanup_db_files(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn record_cancelled_writes_log_clears_running_and_preserves_failures() {
        let path = temp_db_path("cancelled-log");
        let db = Arc::new(CronDB::open(&path).expect("open db"));
        let job = db
            .add_job(&NewCronJob {
                name: "Hydrate".into(),
                description: None,
                schedule: CronSchedule::Every {
                    interval_ms: 300_000,
                    start_at: None,
                },
                payload: CronPayload::AgentTurn {
                    prompt: "drink water".into(),
                    agent_id: None,
                },
                max_failures: Some(5),
                notify_on_complete: Some(false),
                delivery_targets: None,
            })
            .expect("add job");
        {
            let conn = db.conn.lock().expect("lock");
            conn.execute(
                "UPDATE cron_jobs SET consecutive_failures=2 WHERE id=?1",
                params![job.id],
            )
            .expect("seed failures");
        }
        let claimed = db
            .claim_immediate_job_for_execution(&job)
            .expect("claim")
            .expect("claimed job");

        record_cancelled(&db, &claimed.job, &claimed.claimed_at, 42, "session-cancel");

        let stored = db.get_job(&job.id).expect("load").expect("job exists");
        assert!(stored.running_at.is_none());
        assert_eq!(stored.consecutive_failures, 2);
        let logs = db.get_run_logs(&job.id, 10).expect("logs");
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status, "cancelled");
        assert_eq!(logs[0].session_id, "session-cancel");
        assert_eq!(logs[0].duration_ms, Some(42));
        assert_eq!(logs[0].error.as_deref(), Some("Cancelled by user"));

        cleanup_db_files(&path);
    }
}
