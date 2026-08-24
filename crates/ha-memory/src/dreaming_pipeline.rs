//! Feature-owned Light Dreaming cycle orchestration.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde_json::json;

use crate::dreaming_narrative as narrative;
use crate::dreaming_promotion as promotion;
use crate::dreaming_scanner as scanner;
use ha_core::memory::dreaming::{self, DreamPhase, DreamReport, DreamRunStatus, DreamTrigger};

use crate::dreaming_triggers::try_claim;
use dreaming::store;

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
    let cfg = ha_core::config::cached_config().dreaming.clone();
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
    //    ACP multi-process overlap. Normal exits explicitly await lease release
    //    before `_guard` drops the in-process AtomicBool flag.
    let phase = DreamPhase::Light;
    let scope_key = "global";
    let lock_key = format!("{}:{}", phase.as_str(), scope_key);
    let run_id = uuid::Uuid::new_v4().to_string();
    // Size the lease from the configurable narrative timeout (which has no
    // upper bound) so a healthy run can't lose its lease mid-cycle and let
    // another process start a concurrent one.
    let lease_ttl = store::lease_ttl_secs(cfg.narrative_timeout_secs);

    let lock_key_for_acquire = lock_key.clone();
    let run_id_for_acquire = run_id.clone();
    let Some(lease) = ha_core::blocking::run_blocking(move || {
        store::acquire_lease(&lock_key_for_acquire, &run_id_for_acquire, lease_ttl)
    })
    .await
    else {
        // Another live run holds the lease. Don't drop the candidate window —
        // record a deferred-capture marker the holder will drain on its next
        // cycle. (Phase 0 only runs Light, so this fires only on genuine
        // multi-process contention; Deep adds richer source payloads.)
        let run_id_for_pending = run_id.clone();
        let trigger_name = trigger.as_str().to_string();
        ha_core::blocking::run_blocking(move || {
            if let Some(s) = store::store() {
                let _ = s.enqueue_pending(
                    "global",
                    "light_rescan",
                    &run_id_for_pending,
                    None,
                    &json!({ "trigger": trigger_name }).to_string(),
                );
            }
        })
        .await;
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
    let run_id_for_create = run_id.clone();
    let trigger_name = trigger.as_str().to_string();
    let phase_name = phase.as_str().to_string();
    ha_core::blocking::run_blocking(move || {
        if let Some(s) = store::store() {
            if let Err(e) = s.create_run(
                &run_id_for_create,
                &trigger_name,
                &phase_name,
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
    })
    .await;

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
    ha_core::blocking::run_blocking(move || drain_pending(scope_key)).await;

    // 6. Conservative Deep resolver automation: deterministic expiry plus a
    //    bounded graph-informed LLM pass. Automatic conflict handling only
    //    routes to Review Inbox; it never silently supersedes competing facts.
    if cfg.deep_resolver.auto_expire_on_light_cycle || cfg.deep_resolver.auto_resolve_on_light_cycle
    {
        let sweep = crate::dreaming_resolver::run_auto_resolver_sweep(trigger).await;
        if sweep.expired + sweep.merged + sweep.needs_review > 0 {
            app_info!(
                "memory",
                "dreaming::run_cycle",
                "automatic resolver changed {} claim(s) before light cycle {}",
                sweep.expired + sweep.merged + sweep.needs_review,
                run_id
            );
        }
    }

    // 7. Resolve the shared automation model chain. A missing chain becomes a
    // bounded narrative failure below and never weakens the resolver sweep.
    let chain = ha_core::memory::dreaming::resolve_dreaming_chain(&cfg);

    // 8. Scan candidates off the async runtime.
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
        let run_id_for_finish = run_id.clone();
        let report_for_finish = report.clone();
        ha_core::blocking::run_blocking(move || {
            if let Some(s) = store::store() {
                if let Err(e) = s.finish_run(
                    &run_id_for_finish,
                    DreamRunStatus::Completed,
                    &report_for_finish,
                ) {
                    app_warn!("memory", "dreaming::store", "failed to finalise run: {}", e);
                }
            }
        })
        .await;
        lease.release().await;
        return report;
    }

    // 9. Run the narrative side_query.
    let narrative_out = match narrative::run_side_query(chain, &candidates, &cfg).await {
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
            finalize_failed(&run_id, &report).await;
            lease.release().await;
            return report;
        }
    };

    // 10. Apply promotions (flip pinned=true on each). Render the diary
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

    // 11. Write the diary markdown.
    let diary_path =
        match ha_core::blocking::run_blocking(move || narrative::write_diary(&diary_md)).await {
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

    // 12. Finalise: durable run + decision log + watermark (best-effort —
    //     a store failure must never lose the cycle; the diary is on disk).
    let run_id_for_finish = run_id.clone();
    let report_for_finish = report.clone();
    let newest = candidates
        .iter()
        .max_by(|a, b| a.created_at.cmp(&b.created_at))
        .map(|entry| (entry.id.to_string(), entry.created_at.clone()));
    ha_core::blocking::run_blocking(move || {
        if let Some(s) = store::store() {
            if let Err(e) = s.finish_run(
                &run_id_for_finish,
                DreamRunStatus::Completed,
                &report_for_finish,
            ) {
                app_warn!("memory", "dreaming::store", "failed to finalise run: {}", e);
            }
            if let Err(e) = s.insert_decisions(&run_id_for_finish, &report_for_finish.promoted) {
                app_warn!(
                    "memory",
                    "dreaming::store",
                    "failed to persist decisions: {}",
                    e
                );
            }
            if let Some((id, created_at)) = newest {
                let _ = s.set_watermark("global", "memories", Some(&id), Some(&created_at));
            }
        }
    })
    .await;

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

    lease.release().await;
    report
}

/// Mark a durable run row as failed (best-effort). Used by the agent-build
/// and side_query failure paths.
async fn finalize_failed(run_id: &str, report: &DreamReport) {
    let run_id = run_id.to_string();
    let report = report.clone();
    ha_core::blocking::run_blocking(move || {
        if let Some(s) = store::store() {
            if let Err(e) = s.finish_run(&run_id, DreamRunStatus::Failed, &report) {
                app_warn!("memory", "dreaming::store", "failed to finalise run: {}", e);
            }
        }
    })
    .await;
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

/// Resolve the automation model chain for Dreaming. The legacy single-model
/// field remains a compatibility fallback behind the typed override.
fn emit_cycle_started(run_id: &str, trigger: DreamTrigger) {
    if let Some(bus) = ha_core::get_event_bus() {
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
    if let Some(bus) = ha_core::get_event_bus() {
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

pub fn run_cycle_boxed(trigger: DreamTrigger) -> ha_core::memory::dreaming::DreamingPipelineFuture {
    Box::pin(run_cycle(trigger))
}
