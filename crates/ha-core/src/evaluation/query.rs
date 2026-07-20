use super::{
    EvalArtifactStore, EvalCampaignComparison, EvalCompareQuery, EvalCompareResult,
    EvalExperimentRecord, EvalExperimentStatus, EvalHistoryKind, EvalHistoryQuery, EvalIntegrity,
    EvalMetricComparison, EvalRepository, EvalTrendMetric, EvalTrendPoint, EvalTrendQuery,
    EvalTrialDetail,
};
use anyhow::{anyhow, bail, Result};
use ha_eval_spec::app::{
    compatibility_for, ComparisonCompatibility, CompatibilityAssessment, CompatibilityFingerprint,
    CompatibilityMetric, NetworkEnforcement,
};
use ha_eval_spec::digest_serializable;
use ha_eval_spec::model::{
    ExecutionMode, ModelCampaignEvidence, ModelCampaignOutcome, ModelTrialResult, NetworkPolicy,
    PlannedModelTrial, RunnerClass,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub struct EvalQueryService {
    repository: EvalRepository,
    artifacts: EvalArtifactStore,
}

impl EvalQueryService {
    pub fn new(repository: EvalRepository, artifacts: EvalArtifactStore) -> Self {
        Self {
            repository,
            artifacts,
        }
    }

    pub fn trends(&self, query: &EvalTrendQuery) -> Result<Vec<EvalTrendPoint>> {
        let baseline_record = self
            .repository
            .get_experiment(&query.baseline_experiment_id)?
            .ok_or_else(|| anyhow!("baseline evaluation experiment not found"))?;
        let baselines = self.select_evidences(
            &query.baseline_experiment_id,
            query.baseline_campaign_id.as_deref(),
        )?;
        let candidates = self.repository.list_experiments(&EvalHistoryQuery {
            limit: query.limit.clamp(1, 200),
            kind: Some(EvalHistoryKind::HopeCore),
            status: Some(EvalExperimentStatus::Completed),
            ..Default::default()
        })?;
        let mut points = Vec::new();
        for candidate in candidates {
            let evidences = match self.load_evidences(&candidate.id) {
                Ok(evidences) => evidences,
                Err(_) => continue,
            };
            for baseline in &baselines {
                let baseline_model = evidence_model_digest(baseline);
                for evidence in evidences
                    .iter()
                    .filter(|value| evidence_model_digest(value) == baseline_model)
                {
                    let mut point = trend_point(&candidate.id, baseline, evidence, query.metric)?;
                    downgrade_evidence_trust(
                        &mut point.compatibility,
                        &baseline_record,
                        &candidate,
                    );
                    points.push(point);
                }
            }
        }
        points.sort_by(|left, right| {
            left.model_digest
                .cmp(&right.model_digest)
                .then_with(|| left.completed_at.cmp(&right.completed_at))
                .then_with(|| left.campaign_id.cmp(&right.campaign_id))
        });
        Ok(points)
    }

    pub fn compare(&self, query: &EvalCompareQuery) -> Result<EvalCompareResult> {
        let baseline_record = self
            .repository
            .get_experiment(&query.baseline_experiment_id)?
            .ok_or_else(|| anyhow!("baseline evaluation experiment not found"))?;
        let candidate_record = self
            .repository
            .get_experiment(&query.candidate_experiment_id)?
            .ok_or_else(|| anyhow!("candidate evaluation experiment not found"))?;
        let baselines = self.select_evidences(
            &query.baseline_experiment_id,
            query.baseline_campaign_id.as_deref(),
        )?;
        let candidates = self.select_evidences(
            &query.candidate_experiment_id,
            query.candidate_campaign_id.as_deref(),
        )?;
        let pairs = comparison_pairs(
            &baselines,
            &candidates,
            query.baseline_experiment_id == query.candidate_experiment_id,
            query.baseline_campaign_id.is_some() || query.candidate_campaign_id.is_some(),
        );
        if pairs.is_empty() {
            bail!("no campaign pair is available for comparison");
        }
        let mut comparisons = Vec::with_capacity(pairs.len());
        for (baseline, candidate) in pairs {
            let mut metrics = Vec::new();
            for metric in [
                CompatibilityMetric::Functional,
                CompatibilityMetric::WallTime,
                CompatibilityMetric::ToolCalls,
                CompatibilityMetric::Tokens,
                CompatibilityMetric::UsdCost,
                CompatibilityMetric::MultiAgent,
            ] {
                let mut compatibility = campaign_compatibility(&baseline, &candidate, metric)?;
                downgrade_evidence_trust(&mut compatibility, &baseline_record, &candidate_record);
                let (baseline_value, candidate_value) =
                    metric_values(&baseline, &candidate, metric);
                let delta = baseline_value
                    .zip(candidate_value)
                    .map(|(left, right)| right - left);
                let delta_percent = baseline_value
                    .zip(candidate_value)
                    .filter(|(left, _)| *left != 0.0)
                    .map(|(left, right)| (right - left) / left * 100.0);
                metrics.push(EvalMetricComparison {
                    metric,
                    compatibility,
                    baseline_value,
                    candidate_value,
                    delta,
                    delta_percent,
                });
            }
            comparisons.push(EvalCampaignComparison {
                baseline_campaign_id: baseline.campaign_id.clone(),
                candidate_campaign_id: candidate.campaign_id.clone(),
                baseline_model_digest: evidence_model_digest(baseline),
                candidate_model_digest: evidence_model_digest(candidate),
                metrics,
            });
        }
        Ok(EvalCompareResult {
            baseline_experiment_id: query.baseline_experiment_id.clone(),
            candidate_experiment_id: query.candidate_experiment_id.clone(),
            comparisons,
        })
    }

    pub fn trial(
        &self,
        experiment_id: &str,
        campaign_id: &str,
        trial_id: &str,
    ) -> Result<EvalTrialDetail> {
        let detail = self
            .repository
            .detail(experiment_id)?
            .ok_or_else(|| anyhow!("evaluation experiment not found"))?;
        let record = detail
            .trials
            .into_iter()
            .find(|trial| trial.campaign_id == campaign_id && trial.id == trial_id)
            .ok_or_else(|| anyhow!("evaluation trial not found"))?;
        let evidence_detail = self
            .repository
            .campaign_evidence_sha256(experiment_id, campaign_id)?
            .map(|digest| -> Result<_> {
                let evidence: ModelCampaignEvidence =
                    serde_json::from_slice(&self.artifacts.read(&digest, 256 * 1024 * 1024)?)?;
                ha_eval_spec::model::validate_evidence_shape(&evidence)?;
                let planned_case = evidence
                    .suites
                    .iter()
                    .find(|suite| suite.id == record.suite_id)
                    .and_then(|suite| suite.cases.iter().find(|case| case.id == record.case_id));
                let budget = planned_case.map(|case| case.budget.clone());
                let timeout_seconds = planned_case.map(|case| case.timeout_seconds);
                let result = evidence
                    .trial_results
                    .into_iter()
                    .find(|trial| trial.trial_id == trial_id)
                    .ok_or_else(|| {
                        anyhow!("indexed evidence does not contain the selected trial")
                    })?;
                Ok((result, budget, timeout_seconds))
            })
            .transpose()?;
        let (result, budget, timeout_seconds) = evidence_detail
            .map(|(result, budget, timeout_seconds)| (Some(result), budget, timeout_seconds))
            .unwrap_or((None, None, None));
        Ok(EvalTrialDetail {
            record,
            result,
            budget,
            timeout_seconds,
        })
    }

    fn load_evidences(&self, experiment_id: &str) -> Result<Vec<ModelCampaignEvidence>> {
        let digests = self.repository.evidence_artifact_sha256s(experiment_id)?;
        if digests.is_empty() {
            bail!("experiment has no indexed campaign evidence");
        }
        digests
            .into_iter()
            .map(|digest| {
                let evidence: ModelCampaignEvidence =
                    serde_json::from_slice(&self.artifacts.read(&digest, 256 * 1024 * 1024)?)?;
                ha_eval_spec::model::validate_evidence_shape(&evidence)?;
                Ok(evidence)
            })
            .collect()
    }

    fn select_evidences(
        &self,
        experiment_id: &str,
        campaign_id: Option<&str>,
    ) -> Result<Vec<ModelCampaignEvidence>> {
        let mut evidences = self.load_evidences(experiment_id)?;
        if let Some(campaign_id) = campaign_id {
            evidences.retain(|evidence| evidence.campaign_id == campaign_id);
            if evidences.is_empty() {
                bail!("selected campaign has no indexed evidence");
            }
        }
        Ok(evidences)
    }
}

fn downgrade_evidence_trust(
    compatibility: &mut CompatibilityAssessment,
    baseline: &EvalExperimentRecord,
    candidate: &EvalExperimentRecord,
) {
    let mut reasons = Vec::new();
    if matches!(baseline.integrity, EvalIntegrity::ProtectedUnknownAssets)
        || matches!(candidate.integrity, EvalIntegrity::ProtectedUnknownAssets)
    {
        reasons.push("unknown_asset_version");
    }
    if matches!(baseline.integrity, EvalIntegrity::UnverifiedImport)
        || matches!(candidate.integrity, EvalIntegrity::UnverifiedImport)
    {
        reasons.push("unverified_evidence");
    }
    if protected_signature_is_untrusted(baseline) || protected_signature_is_untrusted(candidate) {
        reasons.push("signature_no_longer_trusted");
    }
    if !reasons.is_empty() {
        if compatibility.compatibility != ComparisonCompatibility::Incompatible {
            compatibility.compatibility = ComparisonCompatibility::DiagnosticOnly;
        }
        for reason in reasons {
            if !compatibility.reasons.iter().any(|value| value == reason) {
                compatibility.reasons.push(reason.to_string());
            }
        }
    }
}

fn protected_signature_is_untrusted(record: &EvalExperimentRecord) -> bool {
    matches!(
        record.integrity,
        EvalIntegrity::ProtectedVerified | EvalIntegrity::ProtectedUnknownAssets
    ) && !matches!(
        record.signature_status.as_deref(),
        Some("verified" | "verified_retired")
    )
}

fn comparison_pairs<'a>(
    baselines: &'a [ModelCampaignEvidence],
    candidates: &'a [ModelCampaignEvidence],
    same_experiment: bool,
    explicit_campaign: bool,
) -> Vec<(&'a ModelCampaignEvidence, &'a ModelCampaignEvidence)> {
    if explicit_campaign {
        return baselines
            .iter()
            .flat_map(|baseline| {
                candidates
                    .iter()
                    .map(move |candidate| (baseline, candidate))
            })
            .collect();
    }
    if same_experiment {
        return baselines
            .iter()
            .enumerate()
            .flat_map(|(index, baseline)| {
                candidates
                    .iter()
                    .skip(index + 1)
                    .map(move |candidate| (baseline, candidate))
            })
            .collect();
    }
    let matched = baselines
        .iter()
        .flat_map(|baseline| {
            let digest = evidence_model_digest(baseline);
            candidates
                .iter()
                .filter(move |candidate| evidence_model_digest(candidate) == digest)
                .map(move |candidate| (baseline, candidate))
        })
        .collect::<Vec<_>>();
    if !matched.is_empty() {
        matched
    } else if baselines.len() == 1 && candidates.len() == 1 {
        vec![(&baselines[0], &candidates[0])]
    } else {
        Vec::new()
    }
}

fn evidence_model_digest(evidence: &ModelCampaignEvidence) -> String {
    evidence
        .trial_results
        .first()
        .map(|trial| trial.model_digest.clone())
        .or_else(|| {
            evidence
                .planned_trials
                .first()
                .map(|trial| trial.model_digest.clone())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn trend_point(
    experiment_id: &str,
    baseline: &ModelCampaignEvidence,
    evidence: &ModelCampaignEvidence,
    metric: EvalTrendMetric,
) -> Result<EvalTrendPoint> {
    let compatibility = campaign_compatibility(baseline, evidence, metric.compatibility_metric())?;
    let scheduled = evidence.trial_results.len() as f64;
    let infra = evidence
        .trial_results
        .iter()
        .filter(|trial| {
            matches!(
                trial.outcome,
                ModelCampaignOutcome::InfraError | ModelCampaignOutcome::SimulatorError
            )
        })
        .count() as f64;
    let policy_failed = evidence
        .trial_results
        .iter()
        .filter(|trial| trial.outcome == ModelCampaignOutcome::PolicyFailed)
        .count() as f64;
    let budget_exhausted = evidence
        .trial_results
        .iter()
        .filter(|trial| trial.outcome == ModelCampaignOutcome::BudgetExhausted)
        .count() as f64;
    let false_completion = evidence
        .trial_results
        .iter()
        .filter(|trial| {
            trial
                .milestones
                .iter()
                .chain(trial.invariants.iter())
                .any(|check| check.id == "false_completion" && !check.passed)
        })
        .count() as f64;
    let any_pass_rate = weighted_reliability_rate(evidence, false);
    let all_pass_rate = weighted_reliability_rate(evidence, true);
    let multi_agent_uplift_pp = team_uplift(evidence);
    let metric_value = match metric {
        EvalTrendMetric::TaskSuccess => evidence.metrics.hard_success_rate,
        EvalTrendMetric::EndToEndYield => evidence.metrics.end_to_end_yield,
        EvalTrendMetric::AnyPassAtK => any_pass_rate,
        EvalTrendMetric::AllPassAtK => all_pass_rate,
        EvalTrendMetric::InfraError => Some(if scheduled > 0.0 {
            infra / scheduled
        } else {
            0.0
        }),
        EvalTrendMetric::PolicyFailure => Some(if scheduled > 0.0 {
            policy_failed / scheduled
        } else {
            0.0
        }),
        EvalTrendMetric::BudgetExhausted => Some(if scheduled > 0.0 {
            budget_exhausted / scheduled
        } else {
            0.0
        }),
        EvalTrendMetric::FalseCompletion => Some(if scheduled > 0.0 {
            false_completion / scheduled
        } else {
            0.0
        }),
        EvalTrendMetric::WallTime => successful_wall_p50(evidence),
        EvalTrendMetric::ToolCalls => average_tools(evidence),
        EvalTrendMetric::Tokens => average_tokens(evidence),
        EvalTrendMetric::UsdCost => average_cost(evidence),
        EvalTrendMetric::MultiAgentUplift => multi_agent_uplift_pp,
    };
    let successful_wall = evidence
        .trial_results
        .iter()
        .filter(|trial| trial.outcome == ModelCampaignOutcome::Passed)
        .map(|trial| trial.timings.wall_ms)
        .collect::<Vec<_>>();
    Ok(EvalTrendPoint {
        experiment_id: experiment_id.to_string(),
        campaign_id: evidence.campaign_id.clone(),
        model_digest: evidence_model_digest(evidence),
        reference: evidence.commit_sha.clone(),
        completed_at: evidence.completed_at.clone(),
        metric,
        metric_value,
        compatibility,
        success_rate: evidence.metrics.hard_success_rate.unwrap_or(0.0),
        end_to_end_yield: evidence.metrics.end_to_end_yield.unwrap_or(0.0),
        infra_error_rate: if scheduled > 0.0 {
            infra / scheduled
        } else {
            0.0
        },
        policy_failure_rate: if scheduled > 0.0 {
            policy_failed / scheduled
        } else {
            0.0
        },
        budget_exhausted_rate: if scheduled > 0.0 {
            budget_exhausted / scheduled
        } else {
            0.0
        },
        false_completion_rate: if scheduled > 0.0 {
            false_completion / scheduled
        } else {
            0.0
        },
        any_pass_rate,
        all_pass_rate,
        multi_agent_uplift_pp,
        median_wall_ms: median_u64(&successful_wall),
        total_tool_calls: evidence
            .trial_results
            .iter()
            .map(|trial| trial.tools.attempted)
            .sum(),
        total_input_tokens: sum_optional(
            evidence
                .trial_results
                .iter()
                .map(|trial| trial.tokens.input),
        ),
        total_output_tokens: sum_optional(
            evidence
                .trial_results
                .iter()
                .map(|trial| trial.tokens.output),
        ),
        total_cost_usd: sum_optional_f64(
            evidence
                .trial_results
                .iter()
                .map(|trial| trial.cost.total_usd),
        ),
    })
}

fn weighted_reliability_rate(evidence: &ModelCampaignEvidence, require_all: bool) -> Option<f64> {
    let groups = evidence
        .metrics
        .reliability
        .iter()
        .map(|summary| summary.groups)
        .sum::<u64>();
    (groups > 0).then(|| {
        evidence
            .metrics
            .reliability
            .iter()
            .map(|summary| {
                if require_all {
                    summary.all_pass_groups
                } else {
                    summary.any_pass_groups
                }
            })
            .sum::<u64>() as f64
            / groups as f64
    })
}

fn campaign_compatibility(
    baseline: &ModelCampaignEvidence,
    candidate: &ModelCampaignEvidence,
    metric: CompatibilityMetric,
) -> Result<CompatibilityAssessment> {
    let baseline_map = fingerprints(baseline)?;
    let candidate_map = fingerprints(candidate)?;
    if baseline_map.keys().collect::<BTreeSet<_>>() != candidate_map.keys().collect::<BTreeSet<_>>()
    {
        return Ok(CompatibilityAssessment {
            compatibility: ComparisonCompatibility::Incompatible,
            reasons: vec!["trial_set_mismatch".to_string()],
        });
    }
    let mut compatibility = ComparisonCompatibility::Exact;
    let mut reasons = BTreeSet::new();
    for (key, baseline) in baseline_map {
        let candidate = candidate_map
            .get(&key)
            .ok_or_else(|| anyhow!("candidate comparison trial disappeared"))?;
        let assessment = compatibility_for(&baseline, candidate, metric);
        compatibility = compatibility.min(assessment.compatibility);
        reasons.extend(assessment.reasons);
    }
    Ok(CompatibilityAssessment {
        compatibility,
        reasons: reasons.into_iter().collect(),
    })
}

fn fingerprints(
    evidence: &ModelCampaignEvidence,
) -> Result<BTreeMap<String, CompatibilityFingerprint>> {
    let planned = evidence
        .planned_trials
        .iter()
        .map(|trial| (trial.id.as_str(), trial))
        .collect::<BTreeMap<_, _>>();
    evidence
        .trial_results
        .iter()
        .map(|trial| {
            let planned = planned
                .get(trial.trial_id.as_str())
                .ok_or_else(|| anyhow!("evidence result has no planned trial"))?;
            let suite = evidence
                .suites
                .iter()
                .find(|suite| suite.id == trial.suite_id)
                .ok_or_else(|| anyhow!("evidence result has no planned suite"))?;
            let key = comparison_trial_key(planned);
            let (runner_class, network_enforcement, hardware_class_digest) = evidence
                .runtime_environment
                .as_ref()
                .map(|runtime| {
                    (
                        runtime.actual_runner_class.clone(),
                        runtime.network_enforcement,
                        runtime.hardware_class_digest.clone(),
                    )
                })
                .unwrap_or_else(|| {
                    (
                        runner_class_name(suite.runner_class).to_string(),
                        if evidence.source.is_release_eligible()
                            && suite.network_policy == NetworkPolicy::ProviderOnly
                        {
                            NetworkEnforcement::Enforced
                        } else {
                            NetworkEnforcement::Unverified
                        },
                        None,
                    )
                });
            Ok((
                key,
                CompatibilityFingerprint {
                    suite_id: trial.suite_id.clone(),
                    case_id: trial.case_id.clone(),
                    scenario_digest: trial.scenario_digest.clone(),
                    verifier_digest: trial.verifier_digest.clone(),
                    prompt_digest: trial.prompt_digest.clone(),
                    tool_schema_digest: trial.tool_schema_digest.clone(),
                    model_digest: trial.model_digest.clone(),
                    runtime_config_digest: trial.runtime_config_digest.clone().unwrap_or_default(),
                    execution_mode: execution_mode_name(suite.execution_mode).to_string(),
                    arm: trial.arm.clone(),
                    source: evidence.source,
                    runner_class,
                    network_enforcement,
                    runner_os: evidence.runner_os.clone(),
                    runner_arch: evidence.runner_arch.clone(),
                    hardware_class_digest,
                    tokenizer_digest: None,
                    usage_source: trial.tokens.usage_source.clone(),
                    price_snapshot_digest: trial.cost.price_snapshot_digest.clone(),
                    compute_budget_digest: planned_budget_digest(evidence, trial)?,
                    trial_seed: Some(trial.seed),
                },
            ))
        })
        .collect()
}

fn planned_budget_digest(
    evidence: &ModelCampaignEvidence,
    trial: &ModelTrialResult,
) -> Result<Option<String>> {
    let budget = evidence
        .suites
        .iter()
        .find(|suite| suite.id == trial.suite_id)
        .and_then(|suite| suite.cases.iter().find(|case| case.id == trial.case_id))
        .map(|case| &case.budget);
    budget.map(digest_serializable).transpose()
}

fn comparison_trial_key(trial: &PlannedModelTrial) -> String {
    // The reference is deliberately part of the generated trial seed, so two
    // commits receive independent stochastic samples.  Seed therefore cannot
    // be part of the cross-commit join key: metric-specific compatibility
    // still checks it where paired seeds are required (multi-agent uplift),
    // while ordinary functional trends can compare the same logical trial.
    comparison_trial_identity(
        &trial.suite_id,
        &trial.case_id,
        &trial.model_role,
        &trial.arm,
        trial.trial_index,
    )
}

fn comparison_trial_identity(
    suite_id: &str,
    case_id: &str,
    model_role: &str,
    arm: &str,
    trial_index: u16,
) -> String {
    format!("{suite_id}:{case_id}:{model_role}:{arm}:{trial_index}")
}

fn metric_values(
    baseline: &ModelCampaignEvidence,
    candidate: &ModelCampaignEvidence,
    metric: CompatibilityMetric,
) -> (Option<f64>, Option<f64>) {
    match metric {
        CompatibilityMetric::Functional => (success_rate(baseline), success_rate(candidate)),
        CompatibilityMetric::WallTime => (
            successful_wall_p50(baseline),
            successful_wall_p50(candidate),
        ),
        CompatibilityMetric::ToolCalls => (average_tools(baseline), average_tools(candidate)),
        CompatibilityMetric::Tokens => (average_tokens(baseline), average_tokens(candidate)),
        CompatibilityMetric::UsdCost => (average_cost(baseline), average_cost(candidate)),
        CompatibilityMetric::MultiAgent => (team_uplift(baseline), team_uplift(candidate)),
    }
}

fn success_rate(evidence: &ModelCampaignEvidence) -> Option<f64> {
    // Functional quality uses the valid-trial denominator. Infra and other
    // non-valid outcomes are intentionally represented by end_to_end_yield
    // and their dedicated rates rather than being silently folded into model
    // task quality.
    evidence.metrics.hard_success_rate
}

fn successful_wall_p50(evidence: &ModelCampaignEvidence) -> Option<f64> {
    evidence.metrics.successful_wall_ms.p50
}

fn average_tools(evidence: &ModelCampaignEvidence) -> Option<f64> {
    let successful = evidence
        .trial_results
        .iter()
        .filter(|trial| trial.outcome == ModelCampaignOutcome::Passed)
        .collect::<Vec<_>>();
    (!successful.is_empty()).then(|| {
        successful
            .iter()
            .map(|trial| trial.tools.attempted)
            .sum::<u64>() as f64
            / successful.len() as f64
    })
}

fn average_tokens(evidence: &ModelCampaignEvidence) -> Option<f64> {
    let values = evidence
        .trial_results
        .iter()
        .filter(|trial| trial.outcome == ModelCampaignOutcome::Passed)
        .map(|trial| {
            trial
                .tokens
                .input
                .zip(trial.tokens.output)
                .map(|(a, b)| a + b)
        })
        .collect::<Option<Vec<_>>>()?;
    (!values.is_empty()).then(|| values.iter().sum::<u64>() as f64 / values.len() as f64)
}

fn average_cost(evidence: &ModelCampaignEvidence) -> Option<f64> {
    let values = evidence
        .trial_results
        .iter()
        .filter(|trial| trial.outcome == ModelCampaignOutcome::Passed)
        .map(|trial| trial.cost.total_usd)
        .collect::<Option<Vec<_>>>()?;
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn team_uplift(evidence: &ModelCampaignEvidence) -> Option<f64> {
    let values = evidence
        .metrics
        .comparisons
        .iter()
        .filter_map(|comparison| comparison.candidate_uplift_pp)
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn sum_optional(mut values: impl Iterator<Item = Option<u64>>) -> Option<u64> {
    values.try_fold(0u64, |sum, value| {
        value.map(|value| sum.saturating_add(value))
    })
}

fn sum_optional_f64(mut values: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    values.try_fold(0.0, |sum, value| value.map(|value| sum + value))
}

fn median_u64(values: &[u64]) -> Option<u64> {
    let mut values = values.to_vec();
    values.sort_unstable();
    values.get(values.len() / 2).copied()
}

fn runner_class_name(value: RunnerClass) -> &'static str {
    match value {
        RunnerClass::HostedLinux => "hosted_linux",
        RunnerClass::DockerLinux => "docker_linux",
        RunnerClass::DedicatedLinux => "dedicated_linux",
        RunnerClass::DesktopVm => "desktop_vm",
        RunnerClass::IsolatedExternalService => "isolated_external_service",
    }
}

fn execution_mode_name(value: ExecutionMode) -> &'static str {
    match value {
        ExecutionMode::NativeProvider => "native_provider",
        ExecutionMode::BridgedProvider => "bridged_provider",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_commit_trial_identity_does_not_include_reference_derived_seed() {
        let left_seed = ha_eval_spec::model::stable_trial_seed(
            &"a".repeat(40),
            "suite",
            "case",
            "anchor",
            "control",
            2,
        );
        let right_seed = ha_eval_spec::model::stable_trial_seed(
            &"b".repeat(40),
            "suite",
            "case",
            "anchor",
            "control",
            2,
        );
        assert_ne!(left_seed, right_seed);
        let left = comparison_trial_identity("suite", "case", "anchor", "control", 2);
        let right = comparison_trial_identity("suite", "case", "anchor", "control", 2);
        assert_eq!(left, right);
        assert_ne!(
            left,
            comparison_trial_identity("suite", "case", "anchor", "control", 3)
        );
    }
}
