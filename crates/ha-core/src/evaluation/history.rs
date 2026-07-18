use super::types::{
    EvalCampaignRecord, EvalExperimentDetail, EvalExperimentRecord, EvalExperimentStatus,
    EvalHistoryKind, EvalHistoryQuery, EvalIntegrity, EvalTrialRecord,
};
use crate::coding_improvement::{CodingBenchmarkCampaign, CodingBenchmarkCampaignListInput};
use crate::domain_eval::{DomainEvalCampaign, ListDomainEvalCampaignsInput};
use crate::session::SessionDB;
use anyhow::Result;
use ha_eval_spec::digest_serializable;
use ha_eval_spec::model::{ModelCampaignOutcome, ModelCampaignSource};

/// Read-only adapter boundary used by the Evaluation Center. Legacy sources
/// remain authoritative in sessions.db; this interface only projects them
/// into the common history DTO and never migrates or mutates their rows.
pub trait EvalHistorySource {
    fn list(&self, query: &EvalHistoryQuery) -> Result<Vec<EvalExperimentRecord>>;
}

pub struct CodingHistorySource<'a> {
    db: &'a SessionDB,
}

impl<'a> CodingHistorySource<'a> {
    pub fn new(db: &'a SessionDB) -> Self {
        Self { db }
    }
}

impl EvalHistorySource for CodingHistorySource<'_> {
    fn list(&self, query: &EvalHistoryQuery) -> Result<Vec<EvalExperimentRecord>> {
        if !allows_legacy(query, EvalHistoryKind::Coding) {
            return Ok(Vec::new());
        }
        let campaigns =
            self.db
                .list_coding_benchmark_campaigns(CodingBenchmarkCampaignListInput {
                    limit: Some(query.limit.clamp(1, 200) as usize),
                    ..Default::default()
                })?;
        Ok(campaigns
            .iter()
            .map(coding_record)
            .filter(|record| query.status.is_none_or(|status| status == record.status))
            .collect())
    }
}

pub struct DomainHistorySource<'a> {
    db: &'a SessionDB,
}

impl<'a> DomainHistorySource<'a> {
    pub fn new(db: &'a SessionDB) -> Self {
        Self { db }
    }
}

impl EvalHistorySource for DomainHistorySource<'_> {
    fn list(&self, query: &EvalHistoryQuery) -> Result<Vec<EvalExperimentRecord>> {
        if !allows_legacy(query, EvalHistoryKind::Domain) {
            return Ok(Vec::new());
        }
        let campaigns = self
            .db
            .list_domain_eval_campaigns(ListDomainEvalCampaignsInput {
                limit: Some(query.limit.clamp(1, 200) as usize),
                ..Default::default()
            })?;
        Ok(campaigns
            .iter()
            .map(domain_record)
            .filter(|record| query.status.is_none_or(|status| status == record.status))
            .collect())
    }
}

pub fn coding_detail(campaign: &CodingBenchmarkCampaign) -> EvalExperimentDetail {
    let experiment = coding_record(campaign);
    let campaign_id = format!("coding-campaign:{}", campaign.id);
    let trials = campaign
        .items
        .iter()
        .map(|item| EvalTrialRecord {
            id: format!("coding-item:{}", item.id),
            campaign_id: campaign_id.clone(),
            suite_id: campaign.task_pack_id.clone(),
            case_id: item.pack_run_id.clone().unwrap_or_else(|| item.id.clone()),
            arm: campaign.baseline_kind.clone(),
            outcome: legacy_outcome(&item.status),
            attempt: u8::try_from(item.attempt).unwrap_or(u8::MAX),
            duration_ms: elapsed_ms(item.started_at.as_deref(), item.finished_at.as_deref()),
            model_calls: 0,
            tool_calls: 0,
            input_tokens: None,
            output_tokens: None,
            cost_usd: None,
            trace_artifact_sha256: None,
            failure_class: item.error.clone(),
        })
        .collect::<Vec<_>>();
    let campaigns = vec![legacy_campaign_record(
        &campaign_id,
        &experiment,
        &campaign.model_matrix,
        &trials,
    )];
    EvalExperimentDetail {
        experiment,
        campaigns,
        trials,
    }
}

pub fn domain_detail(campaign: &DomainEvalCampaign) -> EvalExperimentDetail {
    let experiment = domain_record(campaign);
    let campaign_id = format!("domain-campaign:{}", campaign.id);
    let trials = campaign
        .items
        .iter()
        .map(|item| EvalTrialRecord {
            id: format!("domain-item:{}", item.id),
            campaign_id: campaign_id.clone(),
            suite_id: item.domain.clone(),
            case_id: item.task_id.clone(),
            arm: item.execution_mode.clone(),
            outcome: legacy_outcome(&item.status),
            attempt: u8::try_from(item.attempt).unwrap_or(u8::MAX),
            duration_ms: elapsed_ms(item.started_at.as_deref(), item.finished_at.as_deref()),
            model_calls: 0,
            tool_calls: 0,
            input_tokens: None,
            output_tokens: None,
            cost_usd: None,
            trace_artifact_sha256: None,
            failure_class: item.error.clone(),
        })
        .collect::<Vec<_>>();
    let campaigns = vec![legacy_campaign_record(
        &campaign_id,
        &experiment,
        &campaign.model_matrix,
        &trials,
    )];
    EvalExperimentDetail {
        experiment,
        campaigns,
        trials,
    }
}

fn allows_legacy(query: &EvalHistoryQuery, kind: EvalHistoryKind) -> bool {
    query.kind.is_none_or(|selected| selected == kind)
        && query
            .source
            .is_none_or(|source| source == ModelCampaignSource::LocalApp)
}

fn coding_record(campaign: &CodingBenchmarkCampaign) -> EvalExperimentRecord {
    let summary = &campaign.summary;
    let completed = summary
        .total_items
        .saturating_sub(summary.queued_items + summary.running_items);
    legacy_record(
        format!("coding:{}", campaign.id),
        EvalHistoryKind::Coding,
        campaign.name.clone(),
        &campaign.status,
        campaign.created_at.clone(),
        campaign.started_at.clone(),
        campaign.finished_at.clone(),
        summary.total_items,
        completed,
        summary.passed_items,
        summary.failed_items + summary.skipped_items,
        summary.interrupted_items,
        campaign.max_budget_usd,
        campaign.error.clone(),
    )
}

fn domain_record(campaign: &DomainEvalCampaign) -> EvalExperimentRecord {
    let summary = &campaign.summary;
    let completed = summary
        .total_items
        .saturating_sub(summary.queued_items + summary.running_items);
    legacy_record(
        format!("domain:{}", campaign.id),
        EvalHistoryKind::Domain,
        campaign.name.clone(),
        &campaign.status,
        campaign.created_at.clone(),
        campaign.started_at.clone(),
        campaign.finished_at.clone(),
        summary.total_items,
        completed,
        summary.passed_items,
        summary.failed_items,
        summary.interrupted_items,
        campaign.max_budget_usd,
        campaign.error.clone(),
    )
}

#[allow(clippy::too_many_arguments)]
fn legacy_record(
    id: String,
    kind: EvalHistoryKind,
    profile_id: String,
    status: &str,
    created_at: String,
    started_at: Option<String>,
    completed_at: Option<String>,
    total: usize,
    completed: usize,
    passed: usize,
    failed: usize,
    infra: usize,
    max_cost_usd: Option<f64>,
    error: Option<String>,
) -> EvalExperimentRecord {
    EvalExperimentRecord {
        id,
        kind,
        profile_id,
        source: ModelCampaignSource::LocalApp,
        integrity: EvalIntegrity::LegacyLocal,
        status: legacy_status(status),
        reference: "legacy-sessions-db".to_string(),
        // The old tables do not record exact git provenance. Dirty=true keeps
        // them permanently ineligible for protected or regression evidence.
        dirty: true,
        app_version: "legacy".to_string(),
        plan_digest: None,
        parent_experiment_id: None,
        created_at,
        started_at,
        completed_at,
        total_trials: saturating_u32(total),
        completed_trials: saturating_u32(completed),
        passed_trials: saturating_u32(passed),
        failed_trials: saturating_u32(failed),
        infra_error_trials: saturating_u32(infra),
        max_cost_usd,
        observed_cost_usd: None,
        pinned: false,
        signature_status: None,
        error,
    }
}

fn legacy_campaign_record<T: serde::Serialize>(
    id: &str,
    experiment: &EvalExperimentRecord,
    models: &[T],
    trials: &[EvalTrialRecord],
) -> EvalCampaignRecord {
    let passed = trials
        .iter()
        .filter(|trial| trial.outcome == ModelCampaignOutcome::Passed)
        .count();
    let infra = trials
        .iter()
        .filter(|trial| trial.outcome == ModelCampaignOutcome::InfraError)
        .count();
    EvalCampaignRecord {
        id: id.to_string(),
        experiment_id: experiment.id.clone(),
        model_digest: digest_serializable(&models).unwrap_or_else(|_| "0".repeat(64)),
        provider_config_digest: "0".repeat(64),
        status: experiment.status,
        evidence_artifact_sha256: None,
        aggregate_status: None,
        total_trials: saturating_u32(trials.len()),
        passed_trials: saturating_u32(passed),
        failed_trials: saturating_u32(trials.len().saturating_sub(passed + infra)),
        infra_error_trials: saturating_u32(infra),
        duration_ms: Some(trials.iter().map(|trial| trial.duration_ms).sum()),
        cost_usd: None,
    }
}

fn legacy_status(status: &str) -> EvalExperimentStatus {
    match status {
        "queued" | "pending" => EvalExperimentStatus::Queued,
        "planning" => EvalExperimentStatus::Planning,
        "running" => EvalExperimentStatus::Running,
        "cancelling" => EvalExperimentStatus::Cancelling,
        "completed" | "passed" | "succeeded" | "success" => EvalExperimentStatus::Completed,
        "cancelled" => EvalExperimentStatus::Cancelled,
        "interrupted" => EvalExperimentStatus::Interrupted,
        _ => EvalExperimentStatus::Failed,
    }
}

fn legacy_outcome(status: &str) -> ModelCampaignOutcome {
    match status {
        "completed" | "passed" | "succeeded" | "success" => ModelCampaignOutcome::Passed,
        "interrupted" => ModelCampaignOutcome::InfraError,
        "cancelled" => ModelCampaignOutcome::Cancelled,
        "queued" | "pending" | "planning" | "running" | "cancelling" => {
            ModelCampaignOutcome::Cancelled
        }
        _ => ModelCampaignOutcome::TaskFailed,
    }
}

fn elapsed_ms(started: Option<&str>, completed: Option<&str>) -> u64 {
    let Some(started) = started.and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
    else {
        return 0;
    };
    let Some(completed) =
        completed.and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
    else {
        return 0;
    };
    u64::try_from((completed - started).num_milliseconds().max(0)).unwrap_or(u64::MAX)
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
