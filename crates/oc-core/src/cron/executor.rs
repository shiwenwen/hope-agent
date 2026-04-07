use anyhow::Result;
use chrono::Utc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::db::CronDB;
use super::types::*;

/// Public wrapper for execute_job, callable from Tauri commands.
pub async fn execute_job_public(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<crate::session::SessionDB>,
    job: &CronJob,
) {
    execute_job(cron_db, session_db, job).await;
}

/// Execute a single cron job: build agent, run chat, record result.
pub(crate) async fn execute_job(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<crate::session::SessionDB>,
    job: &CronJob,
) {
    let start_time = std::time::Instant::now();
    let started_at = Utc::now().to_rfc3339();

    // Atomically claim the job — skip if already running
    match cron_db.try_mark_running(&job.id) {
        Ok(true) => {} // claimed successfully
        Ok(false) => {
            app_warn!(
                "cron",
                "executor",
                "Job '{}' ({}) is already running, skipping",
                job.name,
                job.id
            );
            return;
        }
        Err(e) => {
            app_error!("cron", "executor", "Failed to claim job '{}': {}", job.name, e);
            return;
        }
    }

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
                job,
                &started_at,
                start_time,
                "no_session",
                &e.to_string(),
                "",
            );
            return;
        }
    };

    // Build agent from provider store (with 5-minute timeout to prevent blocking scheduler)
    const CRON_JOB_TIMEOUT_SECS: u64 = 300;
    let result = match tokio::time::timeout(
        std::time::Duration::from_secs(CRON_JOB_TIMEOUT_SECS),
        build_and_run_agent(&agent_id, &prompt, &session_id, session_db),
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

    match result {
        Ok(response) => {
            app_info!(
                "cron",
                "executor",
                "Job '{}' completed successfully ({}ms)",
                job.name,
                duration_ms
            );

            // Save user prompt and assistant response into the session
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
            let _ = session_db.append_message(
                &session_id,
                &crate::session::NewMessage::assistant(&response),
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
            let _ = cron_db.clear_running(&job.id);

            // Emit Tauri event
            emit_cron_event(&job.id, &job.name, "success", job.notify_on_complete);
        }
        Err(e) => {
            app_error!("cron", "executor", "Job '{}' failed: {}", job.name, e);

            // Write the prompt + error message into the session so the user can see what happened
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
            let mut err_msg = crate::session::NewMessage::assistant(&e.to_string());
            err_msg.is_error = Some(true);
            let _ = session_db.append_message(&session_id, &err_msg);

            record_failure(
                cron_db,
                job,
                &started_at,
                start_time,
                "error",
                &e.to_string(),
                &session_id,
            );
        }
    }
}

/// Build an AssistantAgent and run a chat message with full failover logic.
///
/// Uses the same error classification and retry strategy as the regular chat flow:
/// - Retryable errors (RateLimit/Overloaded/Timeout): retry same model up to MAX_RETRIES
///   with exponential backoff before falling back to the next model.
/// - Terminal errors (ContextOverflow): surface immediately, no fallback.
/// - Non-retryable errors (Auth/Billing/ModelNotFound/Unknown): skip to next model.
pub async fn build_and_run_agent(
    agent_id: &str,
    message: &str,
    session_id: &str,
    _session_db: &Arc<crate::session::SessionDB>,
) -> Result<String> {
    build_and_run_agent_with_context(agent_id, message, session_id, _session_db, None).await
}

/// Build an AssistantAgent and run a chat message with full failover logic and optional custom system context.
pub async fn build_and_run_agent_with_context(
    agent_id: &str,
    message: &str,
    session_id: &str,
    _session_db: &Arc<crate::session::SessionDB>,
    extra_system_context: Option<&str>,
) -> Result<String> {
    use crate::agent::AssistantAgent;
    use crate::failover;
    use crate::provider;

    const MAX_RETRIES: u32 = 2;
    const RETRY_BASE_MS: u64 = 1000;
    const RETRY_MAX_MS: u64 = 10_000;

    // Load provider store from disk
    let store = provider::load_store().unwrap_or_default();

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

    // Try each model in the chain with proper failover
    let mut last_error = String::new();
    for (idx, model_ref) in model_chain.iter().enumerate() {
        let prov = match provider::find_provider(&store.providers, &model_ref.provider_id) {
            Some(p) => p,
            None => continue,
        };

        let model_label = format!("{}::{}", model_ref.provider_id, model_ref.model_id);

        // Per-model retry loop
        let mut retry_count: u32 = 0;
        loop {
            let mut agent = AssistantAgent::new_from_provider(prov, &model_ref.model_id);
            agent.set_agent_id(agent_id);
            agent.set_session_id(session_id);
            let ctx = extra_system_context.unwrap_or(
                "## Execution Context\n\
                 You are running as a **scheduled task** (cron job), not an interactive chat.\n\
                 - No user is actively waiting — execute the prompt directly and concisely.\n\
                 - This is an isolated session with no prior conversation history.\n\
                 - Focus on completing the task described in the user message.",
            );
            agent.set_extra_system_context(ctx.to_string());

            let cancel = Arc::new(AtomicBool::new(false));
            match agent.chat(message, &[], None, cancel, |_delta| {}).await {
                Ok((response, _thinking)) => {
                    if idx > 0 {
                        app_info!(
                            "cron",
                            "failover",
                            "Fallback model {} succeeded",
                            model_label
                        );
                    }
                    return Ok(response);
                }
                Err(e) => {
                    last_error = e.to_string();
                    let reason = failover::classify_error(&last_error);

                    // Terminal error — surface immediately, no point trying other models
                    if reason.is_terminal() {
                        app_error!(
                            "cron",
                            "failover",
                            "Model {} hit terminal error ({:?}): {}",
                            model_label,
                            reason,
                            last_error
                        );
                        return Err(anyhow::anyhow!("{}", last_error));
                    }

                    // Retryable error — retry same model with backoff
                    if reason.is_retryable() && retry_count < MAX_RETRIES {
                        retry_count += 1;
                        let delay =
                            failover::retry_delay_ms(retry_count - 1, RETRY_BASE_MS, RETRY_MAX_MS);
                        app_warn!(
                            "cron",
                            "failover",
                            "Model {} retryable error ({:?}), attempt {}/{}, retrying in {}ms: {}",
                            model_label,
                            reason,
                            retry_count,
                            MAX_RETRIES,
                            delay,
                            last_error
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue;
                    }

                    // Non-retryable or retries exhausted — skip to next model
                    app_warn!(
                        "cron",
                        "failover",
                        "Model {} failed ({:?}), skipping to next model: {}",
                        model_label,
                        reason,
                        last_error
                    );
                    break;
                }
            }
        }
    }

    Err(anyhow::anyhow!(
        "All models failed. Last error: {}",
        last_error
    ))
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
