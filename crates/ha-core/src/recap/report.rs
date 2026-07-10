use std::sync::Arc;

use crate::config::AppConfig;
use crate::cron;
use crate::dashboard::{
    query_activity_heatmap, query_cost_trend, query_health_score, query_hourly_distribution,
    query_model_efficiency, query_overview_with_delta, query_top_sessions,
};
use crate::logging::LogDB;
use crate::provider::ActiveModel;
use crate::session::SessionDB;
use anyhow::{anyhow, Result};
use tokio_util::sync::CancellationToken;

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
    /// Resolved model chain for facet extraction + section generation. See
    /// `crate::automation`.
    pub chain: Arc<Vec<ActiveModel>>,
    pub analysis_model: String,
    /// Resolved output language (locale code) for this report run.
    pub locale: String,
    pub config_snapshot: AppConfig,
    pub cancel: CancellationToken,
}

impl RecapContext {
    /// Build a context using the configured analysis model (or a sensible
    /// fallback when none is configured). Pulls DB handles from the global
    /// OnceLock singletons — recap is driven from slash commands / HTTP
    /// routes that all share the same process-level state.
    pub async fn from_globals(cancel: CancellationToken) -> Result<Self> {
        let config = (*crate::config::cached_config()).clone();
        let recap_db = super::api::recap_db()?;
        let (chain, analysis_model) = resolve_recap_chain(&config)?;
        let session_db = crate::require_session_db()?.clone();
        let log_db = crate::require_log_db()?.clone();
        let cron_db = crate::require_cron_db()?.clone();
        let locale = super::i18n::effective_recap_locale(&config);
        Ok(Self {
            session_db,
            log_db,
            cron_db,
            recap_db,
            chain: Arc::new(chain),
            analysis_model,
            locale,
            config_snapshot: config,
            cancel,
        })
    }
}

/// Stable key for `PROFILE_STICKY`/`PROFILE_COOLDOWNS` bookkeeping and the
/// usage ledger's `session_id` column — recap runs aren't tied to one chat
/// session, so this is a fixed synthetic key shared across all recap calls.
pub(super) const RECAP_SESSION_KEY: &str = "automation:recap";

/// Resolve recap's model chain: `recap.model_override` (new) → the
/// deprecated `recap.analysisAgent` (resolved to an equivalent chain via that
/// agent's own model config, unchanged from the old behavior) →
/// `automation::effective_chain`'s own fallback (`function_models.automation`
/// → chat default). Also returns a display label for `RecapReport.meta.analysis_model`.
fn resolve_recap_chain(config: &AppConfig) -> Result<(Vec<ActiveModel>, String)> {
    let override_chain = config.recap.model_override.clone().or_else(|| {
        config
            .recap
            .analysis_agent
            .as_deref()
            .and_then(|id| crate::automation::resolve_legacy_agent_chain(config, id))
    });
    let chain = crate::automation::effective_chain(config, override_chain);
    if chain.is_empty() {
        return Err(anyhow!(
            "no LLM provider available — configure a provider before running analysis tasks"
        ));
    }
    // Label the whole configured chain, not just the primary: `automation::run`
    // genuinely fails over per-call (facet extraction is one call per
    // session, potentially concurrent), so any individual fact in the
    // report may actually have been produced by a fallback if the primary
    // was transiently unavailable — a label naming only `chain[0]` would
    // claim a certainty the chain's whole design contract doesn't provide.
    let primary_label = crate::automation::model_label(config, &chain[0]);
    let label = if chain.len() > 1 {
        format!(
            "{primary_label} (+{} fallback{})",
            chain.len() - 1,
            if chain.len() > 2 { "s" } else { "" }
        )
    } else {
        primary_label
    };
    Ok((chain, label))
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
        &ctx.chain,
        &ctx.analysis_model,
        &ctx.locale,
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
    let sections = generate_all_sections(
        &ctx.chain,
        &facet_summary,
        &quantitative,
        &ctx.locale,
        &progress,
    )
    .await?;

    progress(RecapProgress::Persisting);
    let now = chrono::Utc::now().to_rfc3339();
    let title = report_title(&filters, total_sessions, &ctx.locale);
    let report = RecapReport {
        meta: ReportMeta {
            id: report_id.clone(),
            title,
            range_start: filters.start_date.clone().unwrap_or_default(),
            range_end: filters.end_date.clone().unwrap_or_else(|| now.clone()),
            session_count: total_sessions,
            generated_at: now,
            analysis_model: ctx.analysis_model.clone(),
            locale: ctx.locale.clone(),
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

fn report_title(filters: &RecapFilters, sessions: u32, locale: &str) -> String {
    let start = filters.start_date.as_deref().unwrap_or("…");
    let end = filters.end_date.as_deref().unwrap_or("…");
    super::i18n::report_title(locale, start, end, sessions)
}
