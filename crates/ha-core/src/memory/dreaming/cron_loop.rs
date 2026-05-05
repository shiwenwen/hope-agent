//! Cron scheduler for the Dreaming pipeline.
//!
//! Spawned once at startup (Primary-only) from `app_init`. Watches
//! `config:changed { category: "dreaming" }` and reschedules itself when
//! the user toggles the cron trigger or edits the expression. Inside one
//! cycle the `DREAMING_RUNNING` AtomicBool serialises with the idle
//! ticker, so racing triggers are safe.

use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use cron::Schedule;
use tokio::sync::Notify;

use super::triggers::{manual_run, DreamTrigger};

/// Parse a cron expression with a backstop: if the 6/7-field grammar
/// rejects a 5-field POSIX expression we silently prepend `"0 "` (seconds).
/// Pre-1.0 builds shipped a 5-field default (`"0 3 * * *"`) that the
/// `cron` crate rejects, and existing user configs still carry it. This
/// keeps the scheduler running for those installs without blocking on a
/// manual GUI re-save.
fn parse_cron_lenient(expr: &str) -> Result<Schedule, cron::error::Error> {
    match Schedule::from_str(expr) {
        Ok(s) => Ok(s),
        Err(primary) if expr.split_whitespace().count() == 5 => {
            Schedule::from_str(&format!("0 {}", expr.trim())).map_err(|_| primary)
        }
        Err(e) => Err(e),
    }
}

/// Spawn the cron-trigger loop. Idempotent: re-reads the config every
/// time it wakes up, so config edits propagate by simply pinging
/// `Notify`.
pub fn spawn_dreaming_cron_loop() {
    let notify = Arc::new(Notify::new());

    if let Some(bus) = crate::globals::get_event_bus() {
        let mut rx = bus.subscribe();
        let notify_for_sub = notify.clone();
        // Wake on any config:changed regardless of payload category — `mutate_config`
        // always emits `{ category: "app" }` while ha-settings emits the specific
        // category, so a "dreaming"-only filter misses every GUI save.
        tokio::spawn(async move {
            while let Ok(evt) = rx.recv().await {
                if evt.name == "config:changed" {
                    notify_for_sub.notify_one();
                }
            }
        });
    } else {
        app_warn!(
            "memory",
            "dreaming::cron_loop",
            "EventBus not initialized — cron trigger will not respond to live config edits"
        );
    }

    tokio::spawn(async move {
        loop {
            let cfg = crate::config::cached_config().dreaming.clone();
            if !cfg.enabled || !cfg.cron_trigger.enabled {
                notify.notified().await;
                continue;
            }

            let schedule = match parse_cron_lenient(&cfg.cron_trigger.cron_expr) {
                Ok(s) => s,
                Err(e) => {
                    app_warn!(
                        "memory",
                        "dreaming::cron_loop",
                        "invalid cron expression {:?}: {}; waiting for config change",
                        cfg.cron_trigger.cron_expr,
                        e
                    );
                    notify.notified().await;
                    continue;
                }
            };

            let next = match schedule.upcoming(Utc).next() {
                Some(t) => t,
                None => {
                    app_warn!(
                        "memory",
                        "dreaming::cron_loop",
                        "cron expression {:?} produced no upcoming time",
                        cfg.cron_trigger.cron_expr
                    );
                    notify.notified().await;
                    continue;
                }
            };

            let wait_secs = (next - Utc::now()).num_seconds().max(0) as u64;
            app_info!(
                "memory",
                "dreaming::cron_loop",
                "next dreaming cron fire in {}s ({})",
                wait_secs,
                next
            );

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(wait_secs)) => {
                    let report = manual_run(DreamTrigger::Cron).await;
                    app_info!(
                        "memory",
                        "dreaming::cron_trigger",
                        "cron-trigger cycle: scanned={}, promoted={}, note={:?}",
                        report.candidates_scanned,
                        report.promoted.len(),
                        report.note,
                    );
                }
                _ = notify.notified() => {
                    // Config changed — re-evaluate from the top of the loop.
                }
            }
        }
    });
}
