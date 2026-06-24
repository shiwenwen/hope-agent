use std::time::Duration;

use futures_util::future::join_all;

use crate::channel::ReplyPayload;

use super::types::CronJob;

#[derive(Debug, Clone, Copy)]
pub enum DeliveryOutcome<'a> {
    Success { text: &'a str },
    Failure { error: &'a str },
}

/// Per-target, per-attempt send timeout. A single target hanging must not block
/// the scheduler from clearing `running_at`.
const SEND_TIMEOUT_SECS: u64 = 10;

/// §8: bounded retry for transient send failures (timeout / channel error).
/// Unlike `async_jobs` tool retry — which is gated behind a config flag because
/// the eligible tools are *billed* — IM delivery is free, so it retries by
/// default with a small fixed attempt count rather than a user knob. Total
/// attempts (the initial try counts); backoff is exponential from
/// `SEND_BACKOFF_BASE_MS`.
///
/// Tradeoff: this is **at-least-once**, not at-most-once. A timed-out send may
/// actually have landed, so a retry can rarely duplicate a message — but
/// silently losing the only copy of a periodic result (IM rate-limited / token
/// expired / server restarting) is the worse failure for a scheduled task.
const MAX_SEND_ATTEMPTS: u32 = 3;
const SEND_BACKOFF_BASE_MS: u64 = 500;

/// §8: aggregate outcome of fanning one run's result out to all of a job's
/// delivery targets. Drives the run-log `delivery_status` so the GUI can show
/// whether the result actually reached its IM destinations.
#[derive(Debug, Clone, Default)]
pub struct DeliveryReport {
    /// Targets we actually attempted to send to (whitelisted + account present).
    pub attempted: usize,
    pub succeeded: usize,
    pub failed: usize,
    /// Skipped: not in the `channel_conversations` whitelist.
    pub skipped_unverified: usize,
    /// Skipped: the sending account no longer exists (target marked stale).
    pub skipped_missing_account: usize,
}

impl DeliveryReport {
    fn total(&self) -> usize {
        self.attempted + self.skipped_unverified + self.skipped_missing_account
    }

    /// Run-log `delivery_status` string, or `None` when the job had no targets
    /// (nothing to fan out — distinct from a fan-out that delivered to nobody).
    pub fn run_log_status(&self) -> Option<&'static str> {
        if self.total() == 0 {
            return None;
        }
        if self.succeeded == self.total() {
            Some("delivered")
        } else if self.succeeded > 0 {
            Some("partial")
        } else {
            Some("failed")
        }
    }
}

/// Per-target send outcome, carried back from each concurrent send so the
/// aggregate report and the stale-flag writeback can be computed once.
enum TargetResult {
    /// Delivered. `was_stale` = the target was flagged stale but the account
    /// resolved this time, so the flag should be cleared.
    Delivered {
        was_stale: bool,
    },
    Failed,
    SkippedUnverified,
    MissingAccount,
}

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
    // Injection has no run log of its own (the inline run already wrote one), so
    // the report is informational only here.
    let _ = deliver_results(&job, DeliveryOutcome::Success { text }).await;
}

/// Fan-out a finished cron job's result to each configured IM channel target in
/// parallel, returning a [`DeliveryReport`] summarizing what reached whom.
///
/// - Success → send the raw response text, optionally prefixed with
///   `[Cron] {name}` when the job opts in (`prefix_delivery_with_name`).
/// - Failure → send `⚠️ [Cron] {name} failed: {error}`.
///
/// Transient per-target send failures / timeouts are retried with backoff
/// (`MAX_SEND_ATTEMPTS`). Targets whose account has been deleted are skipped and
/// flagged `stale` (written back so the GUI marks them); whitelist misses are
/// skipped + audited. One broken channel never fails the job or blocks siblings.
pub async fn deliver_results(job: &CronJob, outcome: DeliveryOutcome<'_>) -> DeliveryReport {
    let mut report = DeliveryReport::default();
    if job.delivery_targets.is_empty() {
        return report;
    }
    let Some(registry) = crate::get_channel_registry() else {
        return report;
    };
    let store = crate::config::cached_config();

    let text = match outcome {
        DeliveryOutcome::Success { text } => {
            if job.prefix_delivery_with_name {
                format!("[Cron] {}\n\n{}", job.name, text)
            } else {
                text.to_string()
            }
        }
        DeliveryOutcome::Failure { error } => {
            format!("⚠️ [Cron] {} failed: {}", job.name, error)
        }
    };

    let sends = job
        .delivery_targets
        .iter()
        .enumerate()
        .map(|(idx, target)| {
            let text = text.clone();
            let registry = registry.clone();
            let store = store.clone();
            let was_stale = target.stale;
            async move {
                // Delivery whitelist (runtime half of OQ5). Only fan out to chats
                // this system has on record in `channel_conversations` — the same
                // source `list_channel_targets` exposes to the model when it picks
                // targets. A prompt-injected model could otherwise name an
                // attacker-controlled chat_id and turn cron's account-authenticated,
                // periodically-firing delivery into a silent exfil channel. Because
                // the destination is constrained to a recorded IM conversation (not
                // an arbitrary URL), delivery intentionally does not go through an
                // SSRF check — this whitelist *is* the boundary. Unknown or
                // unverifiable target → skip + audit warn (fail-closed per target).
                let whitelisted = crate::get_channel_db()
                    .map(|db| {
                        db.conversation_exists(
                            &target.channel_id,
                            &target.account_id,
                            &target.chat_id,
                            target.thread_id.as_deref(),
                        )
                        .unwrap_or(false)
                    })
                    .unwrap_or(false);
                if !whitelisted {
                    app_warn!(
                        "cron",
                        "delivery",
                        "refusing delivery of job '{}' to unrecorded target {}:{} \
                         (not in channel_conversations whitelist)",
                        job.name,
                        target.channel_id,
                        target.chat_id
                    );
                    return (idx, TargetResult::SkippedUnverified);
                }

                let Some(account) = store.channels.find_account(&target.account_id) else {
                    app_warn!(
                        "cron",
                        "delivery",
                        "target account '{}' no longer exists, marking target stale + skipping",
                        target.account_id
                    );
                    return (idx, TargetResult::MissingAccount);
                };

                // §8: bounded retry on transient send failure / timeout.
                let mut attempt = 1u32;
                loop {
                    let mut payload = ReplyPayload::text(text.clone());
                    payload.thread_id = target.thread_id.clone();
                    let send = registry.send_reply(account, &target.chat_id, &payload);
                    let err: String = match tokio::time::timeout(
                        Duration::from_secs(SEND_TIMEOUT_SECS),
                        send,
                    )
                    .await
                    {
                        Ok(Ok(_)) => {
                            app_info!(
                                "cron",
                                "delivery",
                                "delivered job '{}' to {}:{} (attempt {})",
                                job.name,
                                target.channel_id,
                                target.chat_id,
                                attempt
                            );
                            return (idx, TargetResult::Delivered { was_stale });
                        }
                        Ok(Err(e)) => format!("{e}"),
                        Err(_) => format!("send timeout after {SEND_TIMEOUT_SECS}s"),
                    };

                    if attempt < MAX_SEND_ATTEMPTS {
                        let backoff =
                            SEND_BACKOFF_BASE_MS.saturating_mul(1u64 << (attempt - 1));
                        app_warn!(
                            "cron",
                            "delivery",
                            "deliver job '{}' to {}:{} failed (attempt {}/{}), retrying in {}ms: {}",
                            job.name,
                            target.channel_id,
                            target.chat_id,
                            attempt,
                            MAX_SEND_ATTEMPTS,
                            backoff,
                            err
                        );
                        tokio::time::sleep(Duration::from_millis(backoff)).await;
                        attempt += 1;
                        continue;
                    }

                    app_warn!(
                        "cron",
                        "delivery",
                        "deliver job '{}' to {}:{} failed after {} attempts: {}",
                        job.name,
                        target.channel_id,
                        target.chat_id,
                        attempt,
                        err
                    );
                    return (idx, TargetResult::Failed);
                }
            }
        });

    let results = join_all(sends).await;

    // Aggregate + collect which accounts to flip stale on. We key by account_id
    // (stable) rather than by index, because the writeback re-reads the job's
    // *current* targets — not this claim-time snapshot — to avoid clobbering a
    // delivery-target edit the user made via the GUI/tool during the run (which
    // for a long job can be hours old here). mark/clear sets are disjoint: a
    // given account is either present (→ deliver, maybe clear) or missing
    // (→ mark) for the whole fan-out.
    let mut mark_stale: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut clear_stale: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (idx, res) in results {
        let account_id = || job.delivery_targets[idx].account_id.clone();
        match res {
            TargetResult::Delivered { was_stale } => {
                report.attempted += 1;
                report.succeeded += 1;
                if was_stale {
                    clear_stale.insert(account_id());
                }
            }
            TargetResult::Failed => {
                report.attempted += 1;
                report.failed += 1;
            }
            TargetResult::SkippedUnverified => {
                report.skipped_unverified += 1;
            }
            TargetResult::MissingAccount => {
                report.skipped_missing_account += 1;
                mark_stale.insert(account_id());
            }
        }
    }

    // §8: persist stale-flag changes via an atomic read-modify-write keyed by
    // account_id (never re-validates the schedule; never overwrites the whole
    // target list from a stale snapshot — see set construction above).
    if !mark_stale.is_empty() || !clear_stale.is_empty() {
        if let Some(cron_db) = crate::globals::get_cron_db() {
            if let Err(e) =
                cron_db.apply_delivery_target_stale_flags(&job.id, &mark_stale, &clear_stale)
            {
                app_warn!(
                    "cron",
                    "delivery",
                    "failed to persist stale target flags for job '{}': {}",
                    job.name,
                    e
                );
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::DeliveryReport;

    #[test]
    fn run_log_status_none_when_no_targets() {
        assert_eq!(DeliveryReport::default().run_log_status(), None);
    }

    #[test]
    fn run_log_status_delivered_when_all_succeed() {
        let r = DeliveryReport {
            attempted: 2,
            succeeded: 2,
            ..Default::default()
        };
        assert_eq!(r.run_log_status(), Some("delivered"));
    }

    #[test]
    fn run_log_status_partial_on_mixed_outcomes() {
        // One delivered, one failed.
        let r = DeliveryReport {
            attempted: 2,
            succeeded: 1,
            failed: 1,
            ..Default::default()
        };
        assert_eq!(r.run_log_status(), Some("partial"));
        // One delivered, one skipped (account gone) is still partial.
        let r = DeliveryReport {
            attempted: 1,
            succeeded: 1,
            skipped_missing_account: 1,
            ..Default::default()
        };
        assert_eq!(r.run_log_status(), Some("partial"));
    }

    #[test]
    fn run_log_status_failed_when_none_delivered() {
        // All failed.
        let r = DeliveryReport {
            attempted: 2,
            failed: 2,
            ..Default::default()
        };
        assert_eq!(r.run_log_status(), Some("failed"));
        // All skipped (whitelist miss + missing account) — had targets, reached
        // nobody → failed (distinct from None = no targets configured).
        let r = DeliveryReport {
            skipped_unverified: 1,
            skipped_missing_account: 1,
            ..Default::default()
        };
        assert_eq!(r.run_log_status(), Some("failed"));
    }
}
