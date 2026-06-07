//! Pipeline — orchestrates one end-to-end Dreaming cycle.
//!
//! Called from every trigger (idle / cron / manual) via
//! [`super::triggers::manual_run`]. Responsibilities:
//! 1. Claim the global running flag (returns immediately on overlap).
//! 2. Load config + build a side_query-capable `AssistantAgent`.
//! 3. Scan recent candidates (blocking SQLite → `spawn_blocking`).
//! 4. Run the narrative side_query (bounded by `narrative_timeout_secs`).
//! 5. Apply promotions (`toggle_pin=true`) and write the diary markdown.
//! 6. Emit `dreaming:cycle_complete` on the EventBus so the Dashboard
//!    can refresh without polling.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde_json::json;

use super::config::DreamingConfig;
use super::narrative::{self};
use super::promotion;
use super::scanner;
use super::store;
use super::triggers::{try_claim, DreamTrigger};
use super::types::{DreamPhase, DreamReport, DreamRunStatus};

use crate::agent::AssistantAgent;

/// Process-local snapshot of the most recent `DreamReport`. Reset on
/// restart (the diary markdown on disk is the durable record); GUI uses
/// it to populate the status row before the first `cycle_complete`
/// event arrives.
static LAST_REPORT: OnceLock<Mutex<Option<DreamReport>>> = OnceLock::new();

fn last_report_slot() -> &'static Mutex<Option<DreamReport>> {
    LAST_REPORT.get_or_init(|| Mutex::new(None))
}

fn record_last_report(report: &DreamReport) {
    if let Ok(mut slot) = last_report_slot().lock() {
        *slot = Some(report.clone());
    }
}

/// Snapshot of the most recent in-process `DreamReport`. Returns `None`
/// before the first cycle of this process.
pub fn last_report_snapshot() -> Option<DreamReport> {
    last_report_slot().lock().ok().and_then(|s| s.clone())
}

/// Execute one dreaming cycle and return the report.
/// `report.note` carries a short reason when a cycle is skipped.
pub async fn run_cycle(trigger: DreamTrigger) -> DreamReport {
    let report = run_cycle_inner(trigger).await;
    record_last_report(&report);
    report
}

async fn run_cycle_inner(trigger: DreamTrigger) -> DreamReport {
    let started = Instant::now();

    // 1. Load config. Bail fast if the feature is disabled. These pre-run
    //    gates never create a durable run row (report.run_id stays None).
    let cfg = crate::config::cached_config().dreaming.clone();
    if !cfg.enabled {
        return skipped(trigger, started, "dreaming disabled in config");
    }

    // Manual button gating — honour manual_enabled even when called from
    // a manual trigger so the UI switch actually works.
    if matches!(trigger, DreamTrigger::Manual) && !cfg.manual_enabled {
        return skipped(trigger, started, "manual trigger disabled in config");
    }

    // 2. In-process guard — refuse same-process overlap (fast, no DB).
    let Some(_guard) = try_claim() else {
        return skipped(
            trigger,
            started,
            "another dreaming cycle is already running",
        );
    };

    // 3. Cross-process lease. Phase 0 is Light + global scope, so the lock
    //    key is fixed at "light:global". The lease guards desktop / server /
    //    ACP multi-process overlap. `_lease` is declared after `_guard`, so on
    //    every return it drops first (reverse declaration order) — releasing
    //    the cross-process lease before the in-process AtomicBool flag.
    let phase = DreamPhase::Light;
    let scope_key = "global";
    let lock_key = format!("{}:{}", phase.as_str(), scope_key);
    let run_id = uuid::Uuid::new_v4().to_string();
    // Size the lease from the configurable narrative timeout (which has no
    // upper bound) so a healthy run can't lose its lease mid-cycle and let
    // another process start a concurrent one.
    let lease_ttl = store::lease_ttl_secs(cfg.narrative_timeout_secs);

    let Some(_lease) = store::acquire_lease(&lock_key, &run_id, lease_ttl) else {
        // Another live run holds the lease. Don't drop the candidate window —
        // record a deferred-capture marker the holder will drain on its next
        // cycle. (Phase 0 only runs Light, so this fires only on genuine
        // multi-process contention; Deep adds richer source payloads.)
        if let Some(s) = store::store() {
            let _ = s.enqueue_pending(
                scope_key,
                "light_rescan",
                &run_id,
                None,
                &json!({ "trigger": trigger.as_str() }).to_string(),
            );
        }
        return skipped(
            trigger,
            started,
            "another instance holds the dreaming lease",
        );
    };

    // 4. Durable run row (best-effort). From here every terminal path
    //    finalises the row; the `_lease` guard releases the lease on drop.
    let scope_json = json!({
        "scopeDays": cfg.scope_days,
        "candidateLimit": cfg.candidate_limit,
    })
    .to_string();
    if let Some(s) = store::store() {
        if let Err(e) = s.create_run(
            &run_id,
            trigger.as_str(),
            phase.as_str(),
            &scope_json,
            lease_ttl,
        ) {
            app_warn!(
                "memory",
                "dreaming::store",
                "failed to persist run row: {}",
                e
            );
        }
    }

    app_info!(
        "memory",
        "dreaming::run_cycle",
        "dreaming cycle started (run={}, trigger={}, scope_days={})",
        run_id,
        trigger.as_str(),
        cfg.scope_days
    );
    emit_cycle_started(&run_id, trigger);

    // 5. Drain deferred-capture markers for this scope (reclaim abandoned
    //    claims first, then claim + acknowledge — the fresh scan below covers
    //    the same recent window).
    drain_pending(scope_key);

    // 6. Build an agent capable of side_query. Cheap — reuses cached
    //    prompt prefix when possible via the existing recap helper.
    let agent = match build_dreaming_agent(&cfg).await {
        Ok(a) => a,
        Err(e) => {
            let report = DreamReport {
                run_id: Some(run_id.clone()),
                trigger,
                candidates_scanned: 0,
                candidates_nominated: 0,
                promoted: Vec::new(),
                diary_path: None,
                duration_ms: started.elapsed().as_millis() as u64,
                note: Some(format!("could not build analysis agent: {}", e)),
            };
            finalize_failed(&run_id, &report);
            return report;
        }
    };

    // 7. Scan candidates off the async runtime.
    let scan_cfg = cfg.clone();
    let candidates = tokio::task::spawn_blocking(move || {
        scanner::collect_candidates(scan_cfg.scope_days, scan_cfg.candidate_limit)
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default();

    if candidates.is_empty() {
        let duration_ms = started.elapsed().as_millis() as u64;
        emit_cycle_event(&run_id, trigger, 0, 0, None, duration_ms);
        let report = DreamReport {
            run_id: Some(run_id.clone()),
            trigger,
            candidates_scanned: 0,
            candidates_nominated: 0,
            promoted: Vec::new(),
            diary_path: None,
            duration_ms,
            note: Some("no candidates in scan window".to_string()),
        };
        // A completed cycle that found nothing to do.
        if let Some(s) = store::store() {
            if let Err(e) = s.finish_run(&run_id, DreamRunStatus::Completed, &report) {
                app_warn!("memory", "dreaming::store", "failed to finalise run: {}", e);
            }
        }
        return report;
    }

    // 8. Run the narrative side_query.
    let narrative_out = match narrative::run_side_query(&agent, &candidates, &cfg).await {
        Ok(out) => out,
        Err(e) => {
            app_warn!(
                "memory",
                "dreaming::run_cycle",
                "narrative side_query failed: {}",
                e
            );
            let report = DreamReport {
                run_id: Some(run_id.clone()),
                trigger,
                candidates_scanned: candidates.len(),
                candidates_nominated: 0,
                promoted: Vec::new(),
                diary_path: None,
                duration_ms: started.elapsed().as_millis() as u64,
                note: Some(format!("side_query failed: {}", e)),
            };
            finalize_failed(&run_id, &report);
            return report;
        }
    };

    // 9. Apply promotions (flip pinned=true on each). Render the diary
    //    before moving `narrative_out` so we only hold one copy of the
    //    promotion records across the closure boundary.
    let diary_md = narrative::render_diary_markdown(&narrative_out);
    let promotions = narrative_out.promotions;
    let nominated_count = narrative_out.promotions_nominated;
    let promoted_count = promotions.len();
    let promotions_for_blocking = promotions.clone();
    let pinned = tokio::task::spawn_blocking(move || {
        promotion::apply_promotions(&promotions_for_blocking).unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    if pinned.len() < promoted_count {
        app_warn!(
            "memory",
            "dreaming::run_cycle",
            "promotions partial: {} pinned of {} nominated",
            pinned.len(),
            promoted_count
        );
    }

    // 10. Write the diary markdown.
    let diary_path = match narrative::write_diary(&diary_md) {
        Ok(path) => Some(path.to_string_lossy().to_string()),
        Err(e) => {
            app_warn!(
                "memory",
                "dreaming::run_cycle",
                "failed to write diary markdown: {}",
                e
            );
            None
        }
    };

    let duration_ms = started.elapsed().as_millis() as u64;
    emit_cycle_event(
        &run_id,
        trigger,
        candidates.len(),
        promoted_count,
        diary_path.clone(),
        duration_ms,
    );
    let report = DreamReport {
        run_id: Some(run_id.clone()),
        trigger,
        candidates_scanned: candidates.len(),
        candidates_nominated: nominated_count,
        promoted: promotions,
        diary_path,
        duration_ms,
        note: None,
    };

    // 11. Finalise: durable run + decision log + watermark (best-effort —
    //     a store failure must never lose the cycle; the diary is on disk).
    if let Some(s) = store::store() {
        if let Err(e) = s.finish_run(&run_id, DreamRunStatus::Completed, &report) {
            app_warn!("memory", "dreaming::store", "failed to finalise run: {}", e);
        }
        if let Err(e) = s.insert_decisions(&run_id, &report.promoted) {
            app_warn!(
                "memory",
                "dreaming::store",
                "failed to persist decisions: {}",
                e
            );
        }
        // Newest scanned candidate as the watermark for this scope. Foundation
        // only: the Light scanner still uses a time window, so this is written
        // but not yet read (the watermark-aware scanner lands in Phase 1+).
        if let Some(newest) = candidates
            .iter()
            .max_by(|a, b| a.created_at.cmp(&b.created_at))
        {
            let _ = s.set_watermark(
                "global",
                "memories",
                Some(&newest.id.to_string()),
                Some(&newest.created_at),
            );
        }
    }

    app_info!(
        "memory",
        "dreaming::run_cycle",
        "cycle done (run={}, trigger={}, scanned={}, nominated={}, promoted={}, duration={}ms)",
        run_id,
        trigger.as_str(),
        report.candidates_scanned,
        report.candidates_nominated,
        report.promoted.len(),
        duration_ms
    );

    report
}

/// Mark a durable run row as failed (best-effort). Used by the agent-build
/// and side_query failure paths.
fn finalize_failed(run_id: &str, report: &DreamReport) {
    if let Some(s) = store::store() {
        if let Err(e) = s.finish_run(run_id, DreamRunStatus::Failed, report) {
            app_warn!("memory", "dreaming::store", "failed to finalise run: {}", e);
        }
    }
}

/// Reclaim abandoned claims, then claim + acknowledge any deferred-capture
/// markers for `scope_key`. Phase 0's fresh scan covers the same recent
/// window, so draining = acknowledging; Deep will consume real payloads.
fn drain_pending(scope_key: &str) {
    let Some(s) = store::store() else { return };
    let _ = s.recover_stale_claimed();
    match s.claim_pending(scope_key, 256) {
        Ok(ids) if !ids.is_empty() => {
            let n = s.mark_pending_processed(&ids).unwrap_or(0);
            app_info!(
                "memory",
                "dreaming::pending",
                "drained {} deferred source(s) for scope {}",
                n,
                scope_key
            );
        }
        Ok(_) => {}
        Err(e) => app_warn!(
            "memory",
            "dreaming::pending",
            "failed to drain pending sources: {}",
            e
        ),
    }
}

fn skipped(trigger: DreamTrigger, started: Instant, note: &str) -> DreamReport {
    app_info!(
        "memory",
        "dreaming::run_cycle",
        "dreaming cycle skipped (trigger={}, reason={})",
        trigger.as_str(),
        note
    );
    DreamReport {
        run_id: None,
        trigger,
        candidates_scanned: 0,
        candidates_nominated: 0,
        promoted: Vec::new(),
        diary_path: None,
        duration_ms: started.elapsed().as_millis() as u64,
        note: Some(note.to_string()),
    }
}

/// Build an `AssistantAgent` for the narrative side_query.
/// Honours `DreamingConfig.narrative_model` when set (format:
/// `providerId:modelId`), falls back to the same heuristic as /recap.
/// `pub(super)` so the Deep resolver reuses the same model resolution.
pub(super) async fn build_dreaming_agent(cfg: &DreamingConfig) -> anyhow::Result<AssistantAgent> {
    let app_cfg = crate::config::cached_config();

    // Explicit dedicated model.
    if let Some(ref target) = cfg.narrative_model {
        if let Some((prov_id, model_id)) = target.split_once(':') {
            if let Some(prov) = app_cfg
                .providers
                .iter()
                .find(|p| p.id == prov_id && p.enabled)
            {
                return Ok(AssistantAgent::try_new_from_provider(prov, model_id)
                    .await?
                    .with_failover_context(prov));
            }
        }
    }

    // Fall back to the existing recap builder, which already has the
    // active-model / first-enabled-provider fallbacks wired up.
    let (agent, _model_id) = crate::recap::report::build_analysis_agent(&app_cfg).await?;
    Ok(agent)
}

/// Emit `dreaming:cycle_started` once a run has claimed its lease and a
/// durable row exists, so the Dashboard can show "running" with the run id.
fn emit_cycle_started(run_id: &str, trigger: DreamTrigger) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "dreaming:cycle_started",
            json!({
                "runId": run_id,
                "trigger": trigger.as_str(),
            }),
        );
    }
}

fn emit_cycle_event(
    run_id: &str,
    trigger: DreamTrigger,
    scanned: usize,
    promoted: usize,
    diary_path: Option<String>,
    duration_ms: u64,
) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "dreaming:cycle_complete",
            json!({
                "runId": run_id,
                "trigger": trigger.as_str(),
                "scanned": scanned,
                "promoted": promoted,
                "diaryPath": diary_path,
                "durationMs": duration_ms,
            }),
        );
    }
}
