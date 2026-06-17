use std::time::Duration;

use futures_util::future::join_all;

use crate::channel::ReplyPayload;

use super::types::CronJob;

#[derive(Debug, Clone, Copy)]
pub enum DeliveryOutcome<'a> {
    Success { text: &'a str },
    Failure { error: &'a str },
}

/// Per-target send timeout. A single target hanging must not block the scheduler
/// from clearing `running_at`.
const SEND_TIMEOUT_SECS: u64 = 10;

/// G2: deliver a background-completion **injection** turn to a cron job's
/// targets. A background job/subagent spawned during a cron run completes after
/// the inline run already delivered its own response; `inject_and_run_parent`
/// then runs a fresh billed turn against the (now-idle) cron session whose
/// output would otherwise reach nobody. Resolve the owning job from the session
/// and fan its result out the same way the inline run does. No-op when the
/// session isn't a cron run, the job is gone, or it has no delivery targets.
pub async fn deliver_injection_for_session(session_id: &str, text: &str) {
    let Some(cron_db) = crate::globals::get_cron_db() else {
        return;
    };
    let job = match cron_db.find_job_by_session(session_id) {
        Ok(Some(job)) if !job.delivery_targets.is_empty() => job,
        Ok(_) => return,
        Err(e) => {
            app_warn!(
                "cron",
                "delivery",
                "find_job_by_session({}) failed: {}",
                session_id,
                e
            );
            return;
        }
    };
    deliver_results(&job, DeliveryOutcome::Success { text }).await;
}

/// Fan-out a finished cron job's result to each configured IM channel target in parallel.
///
/// - Success → send the raw response text (no prefix / header).
/// - Failure → send `⚠️ [Cron] {name} failed: {error}`.
///
/// Targets whose account has been deleted since the job was created are skipped
/// with a warning. Per-target send failures / timeouts are logged, never surfaced —
/// one broken channel doesn't fail the job or block sibling deliveries.
pub async fn deliver_results(job: &CronJob, outcome: DeliveryOutcome<'_>) {
    if job.delivery_targets.is_empty() {
        return;
    }
    let Some(registry) = crate::get_channel_registry() else {
        return;
    };
    let store = crate::config::cached_config();

    let text = match outcome {
        DeliveryOutcome::Success { text } => text.to_string(),
        DeliveryOutcome::Failure { error } => {
            format!("⚠️ [Cron] {} failed: {}", job.name, error)
        }
    };

    let sends = job.delivery_targets.iter().map(|target| {
        let text = text.clone();
        let registry = registry.clone();
        let store = store.clone();
        async move {
            let Some(account) = store.channels.find_account(&target.account_id) else {
                app_warn!(
                    "cron",
                    "delivery",
                    "target account '{}' no longer exists, skipping",
                    target.account_id
                );
                return;
            };

            let mut payload = ReplyPayload::text(text);
            payload.thread_id = target.thread_id.clone();

            let send = registry.send_reply(account, &target.chat_id, &payload);
            match tokio::time::timeout(Duration::from_secs(SEND_TIMEOUT_SECS), send).await {
                Ok(Ok(_)) => app_info!(
                    "cron",
                    "delivery",
                    "delivered job '{}' to {}:{}",
                    job.name,
                    target.channel_id,
                    target.chat_id
                ),
                Ok(Err(e)) => app_warn!(
                    "cron",
                    "delivery",
                    "deliver job '{}' to {}:{} failed: {}",
                    job.name,
                    target.channel_id,
                    target.chat_id,
                    e
                ),
                Err(_) => app_warn!(
                    "cron",
                    "delivery",
                    "deliver job '{}' to {}:{} timeout after {}s",
                    job.name,
                    target.channel_id,
                    target.chat_id,
                    SEND_TIMEOUT_SECS
                ),
            }
        }
    });

    join_all(sends).await;
}
