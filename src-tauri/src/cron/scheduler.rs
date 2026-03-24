use chrono::Utc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::db::CronDB;
use super::executor::execute_job;

// ── Scheduler ───────────────────────────────────────────────────

/// Start the background cron scheduler on a dedicated OS thread with its own tokio runtime.
/// This avoids requiring an existing tokio runtime at call time (e.g. during Tauri .setup()).
pub fn start_scheduler(
    cron_db: Arc<CronDB>,
    session_db: Arc<crate::session::SessionDB>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("cron-scheduler".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("Failed to create cron tokio runtime");

            rt.block_on(async move {
                // Startup recovery
                if let Err(e) = cron_db.recover_orphaned_runs() {
                    app_error!("cron", "scheduler", "Failed to recover orphaned runs: {}", e);
                }
                match cron_db.clear_all_running() {
                    Ok(n) if n > 0 => app_warn!("cron", "scheduler", "Cleared {} stale running markers from previous session", n),
                    Err(e) => app_error!("cron", "scheduler", "Failed to clear stale running markers: {}", e),
                    _ => {}
                }
                if let Err(e) = cron_db.mark_missed_at_jobs() {
                    app_error!("cron", "scheduler", "Failed to mark missed at jobs: {}", e);
                }

                // Run catch-up for overdue recurring jobs
                if let Ok(due_jobs) = cron_db.get_due_jobs(&Utc::now()) {
                    if !due_jobs.is_empty() {
                        app_info!("cron", "scheduler", "Catch-up: {} overdue jobs found at startup", due_jobs.len());
                        for job in due_jobs {
                            match cron_db.claim_job_for_execution(&job) {
                                Ok(true) => {
                                    let db = cron_db.clone();
                                    let sdb = session_db.clone();
                                    tokio::spawn(async move {
                                        execute_job(&db, &sdb, &job).await;
                                    });
                                }
                                Ok(false) => {}
                                Err(e) => {
                                    app_error!("cron", "scheduler", "Failed to claim catch-up job '{}': {}", job.name, e);
                                }
                            }
                        }
                    }
                }

                app_info!("cron", "scheduler", "Scheduler started");
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
                let tick_running = Arc::new(AtomicBool::new(false));

                loop {
                    interval.tick().await;

                    // Scheduler-level guard: skip if previous tick is still processing
                    if tick_running.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
                        app_debug!("cron", "scheduler", "Previous tick still running, skipping");
                        continue;
                    }

                    let now = Utc::now();
                    match cron_db.get_due_jobs(&now) {
                        Ok(due_jobs) => {
                            for job in due_jobs {
                                // Claim job first to prevent duplicate execution
                                match cron_db.claim_job_for_execution(&job) {
                                    Ok(true) => {
                                        let db = cron_db.clone();
                                        let sdb = session_db.clone();
                                        tokio::spawn(async move {
                                            execute_job(&db, &sdb, &job).await;
                                        });
                                    }
                                    Ok(false) => {
                                        app_debug!("cron", "scheduler", "Job '{}' already claimed, skipping", job.name);
                                    }
                                    Err(e) => {
                                        app_error!("cron", "scheduler", "Failed to claim job '{}': {}", job.name, e);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            app_error!("cron", "scheduler", "Failed to query due jobs: {}", e);
                        }
                    }

                    tick_running.store(false, Ordering::Release);
                }
            });
        })
        .expect("Failed to spawn cron scheduler thread")
}
