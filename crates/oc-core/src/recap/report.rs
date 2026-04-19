use std::sync::Arc;

use anyhow::{anyhow, Result};
use tokio_util::sync::CancellationToken;
use crate::agent::AssistantAgent;
use crate::config::AppConfig;
use crate::dashboard::{
    query_activity_heatmap, query_cost_trend, query_health_score, query_hourly_distribution,
    query_model_efficiency, query_overview_with_delta, query_top_sessions,
};
use crate::globals::AppState;
use crate::logging::LogDB;
use crate::provider::find_provider;
use crate::session::SessionDB;
use crate::cron;

use super::aggregate::roll_up;
use super::db::RecapDb;
use super::facets::{extract_facets_for_candidates, resolve_candidates};
use super::sections::generate_all_sections;
use super::types::{
    GenerateMode, QuantitativeStats, RecapFilters, RecapProgress, RecapReport, ReportMeta,
    RECAP_SCHEMA_VERSION,
};

/// Bundle of dependencies needed to generate a recap.
pub struct RecapContext {
    pub session_db: Arc<SessionDB>,
    pub log_db: Arc<LogDB>,
    pub cron_db: Arc<cron::CronDB>,
    pub recap_db: Arc<RecapDb>,
    pub agent: AssistantAgent,
    pub analysis_model: String,
    pub config_snapshot: AppConfig,
    pub cancel: CancellationToken,
}

impl RecapContext {
    /// Build a context using the configured analysis agent (or a sensible
    /// fallback when none is configured).
    pub async fn from_app_state(
        state: &AppState,
        cancel: CancellationToken,
    ) -> Result<Self> {
        let config = state.config.lock().await.clone();
        let recap_db = super::api::recap_db()?;
        let (agent, analysis_model) = build_analysis_agent(&config)?;
        Ok(Self {
            session_db: state.session_db.clone(),
            log_db: state.log_db.clone(),
            cron_db: state.cron_db.clone(),
            recap_db,
            agent,
            analysis_model,
            config_snapshot: config,
            cancel,
        })
    }
}

/// Build an `AssistantAgent` for recap analysis, preferring the
/// configured `recap.analysisAgent` provider, falling back to the active
/// model and finally the first enabled provider.
pub fn build_analysis_agent(config: &AppConfig) -> Result<(AssistantAgent, String)> {
    // Honour explicit recap.analysisAgent if set.
    if let Some(target) = config.recap.analysis_agent.as_ref() {
        if let Some(prov) = find_provider(&config.providers, target) {
            if let Some(model) = prov.models.first() {
                return Ok((
                    AssistantAgent::new_from_provider(prov, &model.id)
                        .with_failover_context(prov),
                    model.id.clone(),
                ));
            }
        }
    }
    if let Some(active) = config.active_model.as_ref() {
        if let Some(prov) = find_provider(&config.providers, &active.provider_id) {
            return Ok((
                AssistantAgent::new_from_provider(prov, &active.model_id)
                    .with_failover_context(prov),
                active.model_id.clone(),
            ));
        }
    }
    for prov in &config.providers {
        if !prov.enabled {
            continue;
        }
        if let Some(model) = prov.models.first() {
            return Ok((
                AssistantAgent::new_from_provider(prov, &model.id)
                    .with_failover_context(prov),
                model.id.clone(),
            ));
        }
    }
    Err(anyhow!(
        "no LLM provider available — configure a provider before running /recap"
    ))
}

/// Top-level entry: extract facets, run dashboard queries, generate
/// AI sections, persist the report.
///
/// `report_id` is provided by the caller so progress events emitted on the
/// EventBus can be keyed to the same id the frontend subscribed to BEFORE
/// the pipeline started.
pub async fn generate_report<F>(
    ctx: &RecapContext,
    mode: GenerateMode,
    report_id: String,
    progress: F,
) -> Result<RecapReport>
where
    F: Fn(RecapProgress) + Send + Sync,
{
    let (candidates, filters) = resolve_candidates(
        &ctx.session_db,
        &ctx.recap_db,
        &mode,
        ctx.config_snapshot.recap.default_range_days,
        ctx.config_snapshot.recap.max_sessions_per_report,
    )?;
    let total_sessions = candidates.len() as u32;
    progress(RecapProgress::Started {
        report_id: report_id.clone(),
        total_sessions,
    });
    app_info!(
        "recap",
        "report",
        "starting report {} ({} sessions, model={})",
        report_id,
        total_sessions,
        ctx.analysis_model
    );

    let facets = extract_facets_for_candidates(
        &ctx.session_db,
        &ctx.recap_db,
        &ctx.agent,
        &ctx.analysis_model,
        candidates,
        ctx.config_snapshot.recap.facet_concurrency,
        &progress,
        ctx.cancel.clone(),
    )
    .await?;

    if ctx.cancel.is_cancelled() {
        anyhow::bail!("cancelled");
    }

    // Dashboard queries acquire SessionDB's Mutex<Connection>; run on a
    // blocking thread so we don't stall the async runtime.
    progress(RecapProgress::AggregatingDashboard);
    let session_db = ctx.session_db.clone();
    let log_db = ctx.log_db.clone();
    let cron_db = ctx.cron_db.clone();
    let dash_filter = filters.clone();
    let quantitative = tokio::task::spawn_blocking(move || {
        compute_quantitative(&session_db, &log_db, &cron_db, &dash_filter)
    })
    .await
    .map_err(|e| anyhow!("dashboard query join error: {}", e))??;

    let facet_summary = roll_up(&facets);
    let sections =
        generate_all_sections(&ctx.agent, &facet_summary, &quantitative, &progress).await?;

    progress(RecapProgress::Persisting);
    let now = chrono::Utc::now().to_rfc3339();
    let title = report_title(&filters, total_sessions);
    let report = RecapReport {
        meta: ReportMeta {
            id: report_id.clone(),
            title,
            range_start: filters.start_date.clone().unwrap_or_default(),
            range_end: filters.end_date.clone().unwrap_or_else(|| now.clone()),
            session_count: total_sessions,
            generated_at: now,
            analysis_model: ctx.analysis_model.clone(),
            filters,
            schema_version: RECAP_SCHEMA_VERSION,
        },
        quantitative,
        facet_summary,
        sections,
    };

    if let Err(e) = ctx.recap_db.save_report(&report) {
        app_warn!("recap", "report", "save_report failed: {}", e);
    }

    progress(RecapProgress::Done {
        report_id: report.meta.id.clone(),
    });
    Ok(report)
}

fn compute_quantitative(
    session_db: &Arc<SessionDB>,
    log_db: &Arc<LogDB>,
    cron_db: &Arc<cron::CronDB>,
    filter: &RecapFilters,
) -> Result<QuantitativeStats> {
    let overview = query_overview_with_delta(session_db, log_db, cron_db, filter)?;
    let health = query_health_score(session_db, log_db, cron_db, filter)?;
    let cost_trend = query_cost_trend(session_db, filter)?;
    let heatmap = query_activity_heatmap(session_db, filter)?;
    let hourly = query_hourly_distribution(session_db, filter)?;
    let top_sessions = query_top_sessions(session_db, filter, 10)?;
    let model_efficiency = query_model_efficiency(session_db, filter)?;
    Ok(QuantitativeStats {
        overview,
        health,
        cost_trend,
        heatmap,
        hourly,
        top_sessions,
        model_efficiency,
    })
}

fn report_title(filters: &RecapFilters, sessions: u32) -> String {
    let start = filters.start_date.as_deref().unwrap_or("…");
    let end = filters.end_date.as_deref().unwrap_or("…");
    format!("Recap {} → {} ({} sessions)", start, end, sessions)
}
