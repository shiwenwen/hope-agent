use std::sync::Arc;

use crate::agent::AssistantAgent;
use crate::config::AppConfig;
use crate::cron;
use crate::dashboard::{
    query_activity_heatmap, query_cost_trend, query_health_score, query_hourly_distribution,
    query_model_efficiency, query_overview_with_delta, query_top_sessions,
};
use crate::logging::LogDB;
use crate::provider::{find_provider, resolve_model_chain, ActiveModel};
use crate::session::SessionDB;
use anyhow::{anyhow, Context, Result};
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
    pub agent: AssistantAgent,
    pub analysis_model: String,
    pub config_snapshot: AppConfig,
    pub cancel: CancellationToken,
}

impl RecapContext {
    /// Build a context using the configured analysis agent (or a sensible
    /// fallback when none is configured). Pulls DB handles from the global
    /// OnceLock singletons — recap is driven from slash commands / HTTP
    /// routes that all share the same process-level state.
    pub async fn from_globals(cancel: CancellationToken) -> Result<Self> {
        let config = (*crate::config::cached_config()).clone();
        let recap_db = super::api::recap_db()?;
        let (agent, analysis_model) = build_analysis_agent(&config).await?;
        let session_db = crate::require_session_db()?.clone();
        let log_db = crate::require_log_db()?.clone();
        let cron_db = crate::require_cron_db()?.clone();
        Ok(Self {
            session_db,
            log_db,
            cron_db,
            recap_db,
            agent,
            analysis_model,
            config_snapshot: config,
            cancel,
        })
    }
}

/// Build an `AssistantAgent` for recap analysis, preferring the configured
/// `recap.analysisAgent`. When unset, inherit the global default agent, then
/// resolve that agent's model chain in the same order as regular chat.
pub async fn build_analysis_agent(config: &AppConfig) -> Result<(AssistantAgent, String)> {
    let explicit = normalize_agent_id(config.recap.analysis_agent.as_deref());
    let inherited = normalize_agent_id(config.default_agent_id.as_deref())
        .unwrap_or_else(|| crate::agent::resolver::HARDCODED_DEFAULT_AGENT_ID.to_string());

    let mut candidate_agent_ids = Vec::new();
    candidate_agent_ids.push(explicit.clone().unwrap_or_else(|| inherited.clone()));
    if explicit.is_some() && !candidate_agent_ids.iter().any(|id| id == &inherited) {
        candidate_agent_ids.push(inherited.clone());
    }
    if inherited != crate::agent::resolver::HARDCODED_DEFAULT_AGENT_ID
        && !candidate_agent_ids
            .iter()
            .any(|id| id == crate::agent::resolver::HARDCODED_DEFAULT_AGENT_ID)
    {
        candidate_agent_ids.push(crate::agent::resolver::HARDCODED_DEFAULT_AGENT_ID.to_string());
    }

    let mut last_error: Option<anyhow::Error> = None;
    for agent_id in candidate_agent_ids {
        match build_analysis_agent_for_id(config, &agent_id).await {
            Ok(built) => return Ok(built),
            Err(err) => {
                app_warn!(
                    "recap",
                    "report",
                    "analysis agent '{}' unavailable, trying fallback: {}",
                    agent_id,
                    err
                );
                last_error = Some(err);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        anyhow!("no LLM provider available — configure a provider before running /recap")
    }))
}

fn normalize_agent_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

async fn build_analysis_agent_for_id(
    config: &AppConfig,
    agent_id: &str,
) -> Result<(AssistantAgent, String)> {
    let agent_def = crate::agent_loader::load_agent(agent_id)
        .with_context(|| format!("failed to load agent '{}'", agent_id))?;
    let model_chain = analysis_model_chain(config, &agent_def.config.model);

    for model_ref in model_chain {
        let Some(prov) = find_provider(&config.providers, &model_ref.provider_id) else {
            continue;
        };
        let mut agent = AssistantAgent::try_new_from_provider(prov, &model_ref.model_id)
            .await?
            .with_failover_context(prov);
        agent.set_agent_id(agent_id);
        agent.set_temperature(agent_def.config.model.temperature.or(config.temperature));
        return Ok((agent, format!("{} / {}", agent_id, model_ref)));
    }

    Err(anyhow!(
        "no usable model configured for analysis agent '{}'",
        agent_id
    ))
}

fn analysis_model_chain(
    config: &AppConfig,
    agent_model: &crate::agent_config::AgentModelConfig,
) -> Vec<ActiveModel> {
    let (primary, fallbacks) = resolve_model_chain(agent_model, config);
    let mut chain = Vec::new();

    if let Some(primary) = primary {
        push_model_dedup(&mut chain, primary);
    }
    for fallback in fallbacks {
        push_model_dedup(&mut chain, fallback);
    }

    if let Some((provider_id, model_id)) = config.providers.iter().find_map(|provider| {
        if !provider.enabled {
            return None;
        }
        provider
            .models
            .first()
            .map(|model| (provider.id.clone(), model.id.clone()))
    }) {
        push_model_dedup(
            &mut chain,
            ActiveModel {
                provider_id,
                model_id,
            },
        );
    }

    chain
}

fn push_model_dedup(chain: &mut Vec<ActiveModel>, model: ActiveModel) {
    if !chain
        .iter()
        .any(|item| item.provider_id == model.provider_id && item.model_id == model.model_id)
    {
        chain.push(model);
    }
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
