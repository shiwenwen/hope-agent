//! Triggers + cycle orchestration for knowledge maintenance (WS6). Mirrors the
//! dreaming pipeline: a `MAINTENANCE_RUNNING` AtomicBool serial lock, idle + cron
//! triggers (Primary-instance gated by the caller in `app_init`), and a manual
//! entry point. One cycle scans every internal KB, queues proposals (dedup via
//! the unique fingerprint index), and — only when `auto_approve` is on — applies
//! them inline.

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use cron::Schedule;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use super::config::MaintenanceConfig;
use super::generators;
use super::types::{MaintenanceReport, ProposalStatus};

/// Which trigger fired a cycle.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MaintenanceTrigger {
    Idle,
    Cron,
    Manual,
}

impl MaintenanceTrigger {
    pub fn as_str(&self) -> &'static str {
        match self {
            MaintenanceTrigger::Idle => "idle",
            MaintenanceTrigger::Cron => "cron",
            MaintenanceTrigger::Manual => "manual",
        }
    }
}

static MAINTENANCE_RUNNING: AtomicBool = AtomicBool::new(false);
static LAST_CYCLE_EPOCH_SECS: AtomicI64 = AtomicI64::new(0);
static LOOP_SPAWNED: AtomicBool = AtomicBool::new(false);
static LAST_REPORT: Mutex<Option<MaintenanceReport>> = Mutex::new(None);

/// Read-only peek at whether a cycle is in progress (GUI button state).
pub fn maintenance_running() -> bool {
    MAINTENANCE_RUNNING.load(Ordering::Acquire)
}

/// Snapshot of the most recent cycle's report (process-local).
pub fn last_report() -> Option<MaintenanceReport> {
    LAST_REPORT.lock().ok().and_then(|g| g.clone())
}

fn try_claim() -> Option<RunningGuard> {
    MAINTENANCE_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .ok()
        .map(|_| RunningGuard)
}

struct RunningGuard;
impl Drop for RunningGuard {
    fn drop(&mut self) {
        MAINTENANCE_RUNNING.store(false, Ordering::Release);
        LAST_CYCLE_EPOCH_SECS.store(now_secs(), Ordering::Release);
    }
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Guardian heartbeat hook (synchronous). Reuses the dreaming activity clock so
/// "idle" means the same thing across both background pipelines.
pub fn check_idle_trigger(cfg: &MaintenanceConfig) -> bool {
    if !cfg.enabled || !cfg.idle_trigger.enabled || maintenance_running() {
        return false;
    }
    let now = now_secs();
    let last_activity = crate::memory::dreaming::last_activity_epoch_secs();
    if last_activity == 0 {
        return false; // never saw activity — no surprise cycle after boot
    }
    let idle_secs = (cfg.idle_trigger.idle_minutes.max(1) as i64) * 60;
    if now - last_activity < idle_secs {
        return false;
    }
    let last_cycle = LAST_CYCLE_EPOCH_SECS.load(Ordering::Acquire);
    if last_cycle != 0 && now - last_cycle < idle_secs {
        return false; // debounce within one idle window
    }
    true
}

/// Public trigger entry — runs one cycle inline and returns the report.
pub async fn manual_run(trigger: MaintenanceTrigger) -> MaintenanceReport {
    run_cycle(trigger).await
}

fn skipped(note: &str) -> MaintenanceReport {
    MaintenanceReport {
        note: Some(note.to_string()),
        ..Default::default()
    }
}

async fn run_cycle(trigger: MaintenanceTrigger) -> MaintenanceReport {
    let _guard = match try_claim() {
        Some(g) => g,
        None => return skipped("already running"),
    };
    let started = std::time::Instant::now();
    let cfg = crate::config::cached_config()
        .knowledge_maintenance
        .clamped();
    // `enabled` gates only the automatic (idle/cron) triggers; a manual "Scan now"
    // works whenever `manual_enabled` is set, independent of the master switch.
    match trigger {
        MaintenanceTrigger::Manual => {
            if !cfg.manual_enabled {
                return skipped("manual disabled");
            }
        }
        MaintenanceTrigger::Idle | MaintenanceTrigger::Cron => {
            if !cfg.enabled {
                return skipped("disabled");
            }
        }
    }

    let reg = match crate::get_knowledge_db() {
        Some(r) => r,
        None => return skipped("knowledge db not initialized"),
    };
    // Bound queue growth: drop decided rows + stale drafts older than 90 days. A
    // dismissed suggestion's fingerprint stays blocked until then (its row lives),
    // so dismissals stick for the window before a recurring situation can resurface.
    let prune_cutoff = now_secs() * 1000 - 90 * 86_400_000;
    if let Err(e) = reg.prune_proposals(prune_cutoff) {
        app_warn!("knowledge", "maintenance::cycle", "prune failed: {}", e);
    }
    let kbs = match reg.list(false) {
        Ok(k) => k,
        Err(e) => return skipped(&format!("list kbs failed: {e}")),
    };

    let mut report = MaintenanceReport::default();
    let mut changed_kbs: Vec<String> = Vec::new();
    // Global (KB-independent) tasks run only for the first internal KB this cycle.
    let mut global_done = false;

    'kbs: for meta in kbs {
        let kb_id = meta.kb.id.clone();
        // Background autonomous maintenance never writes external (bound) vaults,
        // even when the KB opted into external writes (WS7) — skip every external
        // root regardless of `allow_external_writes`. Only the GUI / agent tools
        // write external on demand.
        if matches!(
            crate::knowledge::resolve_kb_dir(&kb_id),
            Ok(r) if r.is_external
        ) {
            continue;
        }
        let run_global = !global_done;
        global_done = true;
        let proposals = match generators::generate(&kb_id, &cfg, run_global).await {
            Ok(p) => p,
            Err(e) => {
                app_warn!(
                    "knowledge",
                    "maintenance::cycle",
                    "generate failed for kb {}: {}",
                    kb_id,
                    e
                );
                continue;
            }
        };
        let mut kb_added = false;
        for np in proposals {
            if report.generated >= cfg.max_proposals_per_cycle {
                break 'kbs;
            }
            match reg.insert_proposal(&kb_id, &np) {
                Ok(Some(id)) => {
                    report.generated += 1;
                    *report
                        .by_kind
                        .entry(np.kind.as_str().to_string())
                        .or_insert(0) += 1;
                    kb_added = true;
                    if cfg.auto_approve && !np.kind.ignores_auto_approve() {
                        match super::approve_proposal(id).await {
                            Ok(_) => report.auto_applied += 1,
                            Err(e) => app_warn!(
                                "knowledge",
                                "maintenance::auto_apply",
                                "auto-apply proposal {} failed: {}",
                                id,
                                e
                            ),
                        }
                    }
                }
                Ok(None) => report.skipped_existing += 1,
                Err(e) => app_warn!(
                    "knowledge",
                    "maintenance::cycle",
                    "persist proposal failed (kb {}): {}",
                    kb_id,
                    e
                ),
            }
        }
        if kb_added {
            changed_kbs.push(kb_id);
        }
    }

    report.duration_ms = started.elapsed().as_millis() as u64;

    // Notify the GUI to refresh the review queue per affected KB.
    if let Some(bus) = crate::get_event_bus() {
        for kb_id in &changed_kbs {
            bus.emit(
                "knowledge:changed",
                serde_json::json!({ "kbId": kb_id, "op": "maintenance" }),
            );
        }
        bus.emit(
            "knowledge:maintenance_complete",
            serde_json::to_value(&report).unwrap_or(serde_json::Value::Null),
        );
    }
    crate::dashboard::learning::emit(
        "kb_maintenance_cycle",
        None,
        None,
        Some(&serde_json::json!({
            "trigger": trigger.as_str(),
            "generated": report.generated,
            "autoApplied": report.auto_applied,
        })),
    );

    app_info!(
        "knowledge",
        "maintenance::cycle",
        "{}-trigger cycle: generated={}, autoApplied={}, skippedExisting={}, {}ms",
        trigger.as_str(),
        report.generated,
        report.auto_applied,
        report.skipped_existing,
        report.duration_ms
    );

    if let Ok(mut slot) = LAST_REPORT.lock() {
        *slot = Some(report.clone());
    }
    report
}

/// Lenient cron parse: accept a 5-field POSIX expr by prepending seconds.
fn parse_cron_lenient(expr: &str) -> Result<Schedule, cron::error::Error> {
    match Schedule::from_str(expr) {
        Ok(s) => Ok(s),
        Err(primary) if expr.split_whitespace().count() == 5 => {
            Schedule::from_str(&format!("0 {}", expr.trim())).map_err(|_| primary)
        }
        Err(e) => Err(e),
    }
}

/// Spawn the cron-trigger loop. Idempotent (guarded) — call once at startup,
/// Primary-only. Re-reads config on every wake; `config:changed` pings it.
pub fn spawn_maintenance_cron_loop() {
    if LOOP_SPAWNED.swap(true, Ordering::AcqRel) {
        return;
    }
    let notify = Arc::new(Notify::new());

    if let Some(bus) = crate::get_event_bus() {
        let mut rx = bus.subscribe();
        let notify_for_sub = notify.clone();
        tokio::spawn(async move {
            while let Ok(evt) = rx.recv().await {
                if evt.name == "config:changed" {
                    notify_for_sub.notify_one();
                }
            }
        });
    }

    tokio::spawn(async move {
        loop {
            let cfg = crate::config::cached_config()
                .knowledge_maintenance
                .clamped();
            if !cfg.enabled || !cfg.cron_trigger.enabled {
                notify.notified().await;
                continue;
            }
            let schedule = match parse_cron_lenient(&cfg.cron_trigger.cron_expr) {
                Ok(s) => s,
                Err(e) => {
                    app_warn!(
                        "knowledge",
                        "maintenance::cron_loop",
                        "invalid cron {:?}: {}; waiting for config change",
                        cfg.cron_trigger.cron_expr,
                        e
                    );
                    notify.notified().await;
                    continue;
                }
            };
            let Some(next) = schedule.upcoming(Utc).next() else {
                notify.notified().await;
                continue;
            };
            let wait_secs = (next - Utc::now()).num_seconds().max(0) as u64;
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(wait_secs)) => {
                    let _ = manual_run(MaintenanceTrigger::Cron).await;
                }
                _ = notify.notified() => {}
            }
        }
    });
}

/// Reject every pending proposal across all KBs (owner "clear queue" action).
pub fn reject_all(kb_id: &str) -> anyhow::Result<usize> {
    let reg =
        crate::get_knowledge_db().ok_or_else(|| anyhow::anyhow!("knowledge db not initialized"))?;
    let pending = reg.list_proposals(kb_id, Some(ProposalStatus::Draft))?;
    let mut n = 0;
    for p in pending {
        if reg
            .set_proposal_status(p.id, ProposalStatus::Rejected, None)
            .is_ok()
        {
            n += 1;
        }
    }
    Ok(n)
}
