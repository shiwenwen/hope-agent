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

use std::time::Instant;

use serde_json::json;

use super::config::DreamingConfig;
use super::narrative::{self};
use super::promotion;
use super::scanner;
use super::triggers::{try_claim, DreamTrigger};
use super::types::DreamReport;

use crate::agent::AssistantAgent;

/// Execute one dreaming cycle and return the report.
/// `report.note` carries a short reason when a cycle is skipped.
pub async fn run_cycle(trigger: DreamTrigger) -> DreamReport {
    let started = Instant::now();

    // 1. Load config. Bail fast if the feature is disabled.
    let cfg = crate::config::cached_config().dreaming.clone();
    if !cfg.enabled {
        return DreamReport {
            trigger,
            candidates_scanned: 0,
            candidates_nominated: 0,
            promoted: Vec::new(),
            diary_path: None,
            duration_ms: started.elapsed().as_millis() as u64,
            note: Some("dreaming disabled in config".to_string()),
        };
    }

    // Manual button gating — honour manual_enabled even when called from
    // a manual trigger so the UI switch actually works.
    if matches!(trigger, DreamTrigger::Manual) && !cfg.manual_enabled {
        return skipped(trigger, started, "manual trigger disabled in config");
    }

    // 2. Claim the running flag — refuse overlap.
    let Some(_guard) = try_claim() else {
        return skipped(
            trigger,
            started,
            "another dreaming cycle is already running",
        );
    };

    app_info!(
        "memory",
        "dreaming::run_cycle",
        "dreaming cycle started (trigger={}, scope_days={})",
        trigger.as_str(),
        cfg.scope_days
    );

    // 3. Build an agent capable of side_query. Cheap — reuses cached
    //    prompt prefix when possible via the existing recap helper.
    let agent = match build_dreaming_agent(&cfg) {
        Ok(a) => a,
        Err(e) => {
            return skipped(
                trigger,
                started,
                &format!("could not build analysis agent: {}", e),
            );
        }
    };

    // 4. Scan candidates off the async runtime.
    let scan_cfg = cfg.clone();
    let candidates = tokio::task::spawn_blocking(move || {
        scanner::collect_candidates(scan_cfg.scope_days, scan_cfg.candidate_limit)
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default();

    if candidates.is_empty() {
        emit_cycle_event(trigger, 0, 0, None, started.elapsed().as_millis() as u64);
        return DreamReport {
            trigger,
            candidates_scanned: 0,
            candidates_nominated: 0,
            promoted: Vec::new(),
            diary_path: None,
            duration_ms: started.elapsed().as_millis() as u64,
            note: Some("no candidates in scan window".to_string()),
        };
    }

    // 5. Run the narrative side_query.
    let narrative_out = match narrative::run_side_query(&agent, &candidates, &cfg).await {
        Ok(out) => out,
        Err(e) => {
            app_warn!(
                "memory",
                "dreaming::run_cycle",
                "narrative side_query failed: {}",
                e
            );
            return DreamReport {
                trigger,
                candidates_scanned: candidates.len(),
                candidates_nominated: 0,
                promoted: Vec::new(),
                diary_path: None,
                duration_ms: started.elapsed().as_millis() as u64,
                note: Some(format!("side_query failed: {}", e)),
            };
        }
    };

    // 6. Apply promotions (flip pinned=true on each). Render the diary
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

    // 7. Write the diary markdown.
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
        trigger,
        candidates.len(),
        promoted_count,
        diary_path.clone(),
        duration_ms,
    );
    let report = DreamReport {
        trigger,
        candidates_scanned: candidates.len(),
        candidates_nominated: nominated_count,
        promoted: promotions,
        diary_path,
        duration_ms,
        note: None,
    };

    app_info!(
        "memory",
        "dreaming::run_cycle",
        "cycle done (trigger={}, scanned={}, nominated={}, promoted={}, duration={}ms)",
        trigger.as_str(),
        report.candidates_scanned,
        report.candidates_nominated,
        report.promoted.len(),
        duration_ms
    );

    report
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
fn build_dreaming_agent(cfg: &DreamingConfig) -> anyhow::Result<AssistantAgent> {
    let app_cfg = crate::config::cached_config();

    // Explicit dedicated model.
    if let Some(ref target) = cfg.narrative_model {
        if let Some((prov_id, model_id)) = target.split_once(':') {
            if let Some(prov) = app_cfg
                .providers
                .iter()
                .find(|p| p.id == prov_id && p.enabled)
            {
                return Ok(AssistantAgent::new_from_provider(prov, model_id)
                    .with_failover_context(prov));
            }
        }
    }

    // Fall back to the existing recap builder, which already has the
    // active-model / first-enabled-provider fallbacks wired up.
    let (agent, _model_id) = crate::recap::report::build_analysis_agent(&app_cfg)?;
    Ok(agent)
}

fn emit_cycle_event(
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
                "trigger": trigger.as_str(),
                "scanned": scanned,
                "promoted": promoted,
                "diaryPath": diary_path,
                "durationMs": duration_ms,
            }),
        );
    }
}
