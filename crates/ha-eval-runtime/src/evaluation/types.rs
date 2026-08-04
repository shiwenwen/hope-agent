use ha_eval_spec::app::{
    AppControlHello, AppResolvedModelBinding, CompatibilityAssessment, CompatibilityMetric,
    EvalAppPlan, EvalAppProfile, EvalAppRunRequest, RuntimeEnvironmentSnapshot,
};
use ha_eval_spec::model::{
    CampaignBudget, ModelCampaignEvidence, ModelCampaignOutcome, ModelCampaignSource,
    ModelCampaignTier,
};
use serde::{Deserialize, Serialize};

pub const EVALUATION_EVENT: &str = "evaluation:changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalExperimentStatus {
    Queued,
    Planning,
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl EvalExperimentStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Planning => "planning",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalHistoryKind {
    HopeCore,
    Coding,
    Domain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalIntegrity {
    LocalDiagnostic,
    ProtectedVerified,
    ProtectedUnknownAssets,
    UnverifiedImport,
    LegacyLocal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalReadiness {
    pub available: bool,
    pub can_run: bool,
    pub remote_run_enabled: bool,
    pub signed_import_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hello: Option<AppControlHello>,
    #[serde(default)]
    pub issues: Vec<String>,
    #[serde(default)]
    pub signed_import_issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalModelOption {
    pub provider_id: String,
    pub model_id: String,
    pub label: String,
    pub provider_label: String,
    pub credential_profile_label: Option<String>,
    #[serde(default)]
    pub credential_profiles: Vec<EvalCredentialOption>,
    pub supports_isolated_eval: bool,
    pub cost_known: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalCredentialOption {
    pub credential_profile_ref: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalCatalog {
    pub readiness: EvalReadiness,
    pub profiles: Vec<EvalAppProfile>,
    pub suites: Vec<ha_eval_spec::app::AppEvalSuiteCatalog>,
    pub models: Vec<EvalModelOption>,
}

/// Secrets are intentionally not serializable. Owner-plane adapters resolve
/// this immediately before starting the Sidecar and must never persist it.
#[derive(Clone)]
pub struct EvalResolvedLaunch {
    pub request: EvalAppRunRequest,
    pub models: Vec<AppResolvedModelBinding>,
    pub runtime_environment: RuntimeEnvironmentSnapshot,
    pub reference: String,
    pub dirty: bool,
    pub app_version: String,
    pub credential_free_config: serde_json::Value,
    pub provider_secrets_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalPreview {
    pub plan: EvalAppPlan,
    pub estimated_trials: usize,
    pub max_cost_usd: Option<f64>,
    pub max_wall_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalLocalExportResult {
    pub experiment_id: String,
    pub output_path: String,
    pub bundle_sha256: String,
    pub campaign_count: u32,
    pub signed: bool,
    pub release_eligible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalExperimentRecord {
    pub id: String,
    pub kind: EvalHistoryKind,
    pub profile_id: String,
    pub source: ModelCampaignSource,
    pub integrity: EvalIntegrity,
    pub status: EvalExperimentStatus,
    pub reference: String,
    pub dirty: bool,
    pub app_version: String,
    pub plan_digest: Option<String>,
    pub parent_experiment_id: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub total_trials: u32,
    pub completed_trials: u32,
    pub passed_trials: u32,
    pub failed_trials: u32,
    pub infra_error_trials: u32,
    pub max_cost_usd: Option<f64>,
    pub observed_cost_usd: Option<f64>,
    pub pinned: bool,
    /// Current status of the key that signed imported evidence. This is kept
    /// separate from `integrity`: later revocation must not rewrite the fact
    /// that a bundle was validly verified when it was imported.
    pub signature_status: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalCampaignRecord {
    pub id: String,
    pub experiment_id: String,
    pub model_digest: String,
    pub provider_config_digest: String,
    pub status: EvalExperimentStatus,
    pub evidence_artifact_sha256: Option<String>,
    pub aggregate_status: Option<String>,
    pub total_trials: u32,
    pub passed_trials: u32,
    pub failed_trials: u32,
    pub infra_error_trials: u32,
    pub duration_ms: Option<u64>,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalTrialRecord {
    pub id: String,
    pub campaign_id: String,
    pub suite_id: String,
    pub case_id: String,
    pub arm: String,
    pub outcome: ModelCampaignOutcome,
    pub attempt: u8,
    pub duration_ms: u64,
    pub model_calls: u32,
    pub tool_calls: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub trace_artifact_sha256: Option<String>,
    pub failure_class: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalExperimentDetail {
    pub experiment: EvalExperimentRecord,
    pub campaigns: Vec<EvalCampaignRecord>,
    pub trials: Vec<EvalTrialRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalTrialDetail {
    pub record: EvalTrialRecord,
    /// Present for Hope Core evidence. The protocol guarantees this is a
    /// redacted causal/verifier summary with no prompt or tool payloads.
    pub result: Option<ha_eval_spec::model::ModelTrialResult>,
    /// Immutable per-trial limits resolved into the campaign evidence. These
    /// let owner UIs explain a budget outcome without exposing prompt or tool
    /// payloads.
    pub budget: Option<CampaignBudget>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalHistoryQuery {
    pub limit: u32,
    pub offset: u32,
    pub kind: Option<EvalHistoryKind>,
    pub source: Option<ModelCampaignSource>,
    pub status: Option<EvalExperimentStatus>,
}

impl Default for EvalHistoryQuery {
    fn default() -> Self {
        Self {
            limit: 50,
            offset: 0,
            kind: None,
            source: None,
            status: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalTrendPoint {
    pub experiment_id: String,
    pub campaign_id: String,
    pub model_digest: String,
    pub reference: String,
    pub completed_at: String,
    pub metric: EvalTrendMetric,
    pub metric_value: Option<f64>,
    pub compatibility: CompatibilityAssessment,
    pub success_rate: f64,
    pub end_to_end_yield: f64,
    pub infra_error_rate: f64,
    pub policy_failure_rate: f64,
    pub budget_exhausted_rate: f64,
    pub false_completion_rate: f64,
    pub any_pass_rate: Option<f64>,
    pub all_pass_rate: Option<f64>,
    pub multi_agent_uplift_pp: Option<f64>,
    pub median_wall_ms: Option<u64>,
    pub total_tool_calls: u64,
    pub total_input_tokens: Option<u64>,
    pub total_output_tokens: Option<u64>,
    pub total_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalTrendMetric {
    TaskSuccess,
    EndToEndYield,
    AnyPassAtK,
    AllPassAtK,
    InfraError,
    PolicyFailure,
    BudgetExhausted,
    FalseCompletion,
    WallTime,
    ToolCalls,
    Tokens,
    UsdCost,
    MultiAgentUplift,
}

impl EvalTrendMetric {
    pub fn compatibility_metric(self) -> CompatibilityMetric {
        match self {
            Self::TaskSuccess
            | Self::EndToEndYield
            | Self::AnyPassAtK
            | Self::AllPassAtK
            | Self::InfraError
            | Self::PolicyFailure
            | Self::BudgetExhausted
            | Self::FalseCompletion => CompatibilityMetric::Functional,
            Self::WallTime => CompatibilityMetric::WallTime,
            Self::ToolCalls => CompatibilityMetric::ToolCalls,
            Self::Tokens => CompatibilityMetric::Tokens,
            Self::UsdCost => CompatibilityMetric::UsdCost,
            Self::MultiAgentUplift => CompatibilityMetric::MultiAgent,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalTrendQuery {
    pub baseline_experiment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_campaign_id: Option<String>,
    pub metric: EvalTrendMetric,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalCompareQuery {
    pub baseline_experiment_id: String,
    pub candidate_experiment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_campaign_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_campaign_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalMetricComparison {
    pub metric: CompatibilityMetric,
    pub compatibility: CompatibilityAssessment,
    pub baseline_value: Option<f64>,
    pub candidate_value: Option<f64>,
    pub delta: Option<f64>,
    pub delta_percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalCompareResult {
    pub baseline_experiment_id: String,
    pub candidate_experiment_id: String,
    pub comparisons: Vec<EvalCampaignComparison>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalCampaignComparison {
    pub baseline_campaign_id: String,
    pub candidate_campaign_id: String,
    pub baseline_model_digest: String,
    pub candidate_model_digest: String,
    pub metrics: Vec<EvalMetricComparison>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalImportResult {
    pub import_id: String,
    pub experiment_id: String,
    pub integrity: EvalIntegrity,
    pub key_id: Option<String>,
    pub evidence_sha256: String,
    pub already_imported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalBaselineRecord {
    pub id: String,
    pub experiment_id: String,
    pub tier: ModelCampaignTier,
    pub approved_by: String,
    pub approved_at: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalAnnotationRecord {
    pub id: String,
    pub experiment_id: String,
    pub campaign_id: Option<String>,
    pub trial_id: Option<String>,
    pub text: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub enum EvalWorkerEvent {
    Phase {
        experiment_id: String,
        phase: String,
        completed: u32,
        total: u32,
    },
    TrialStarted {
        experiment_id: String,
        campaign_id: String,
        trial_id: String,
        completed: u32,
        total: u32,
    },
    TrialProgress {
        experiment_id: String,
        campaign_id: String,
        trial_id: String,
        wall_ms: u64,
        model_calls: u64,
        tool_calls: u64,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cost_usd: Option<f64>,
        loop_iterations: u64,
        spawned_agents: u64,
        async_jobs: u64,
        active_children: u64,
        attribution: String,
        last_event: Option<String>,
        last_event_status: Option<String>,
    },
    TrialCompleted {
        experiment_id: String,
        campaign_id: String,
        trial_id: String,
        completed: u32,
        total: u32,
        outcome: ModelCampaignOutcome,
        wall_ms: u64,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cost_usd: Option<f64>,
        model_calls: u64,
        tool_calls: u64,
        suite_id: String,
        case_id: String,
        arm: String,
        attempt: u8,
        failure_class: Option<String>,
    },
    BudgetWarning {
        experiment_id: String,
        dimension: String,
        observed: f64,
        limit: f64,
        ratio: f64,
    },
    ArtifactWritten {
        experiment_id: String,
        campaign_id: String,
        path: String,
        sha256: String,
    },
    Evidence {
        experiment_id: String,
        campaign_id: String,
        evidence_path: String,
    },
    Completed {
        experiment_id: String,
        evidence_paths: Vec<String>,
    },
    Cancelled {
        experiment_id: String,
    },
    Failed {
        experiment_id: String,
        code: String,
        message: String,
    },
}

pub fn evidence_summary(evidence: &ModelCampaignEvidence) -> (u32, u32, u32, u32) {
    let total = evidence.trial_results.len() as u32;
    let passed = evidence
        .trial_results
        .iter()
        .filter(|trial| trial.outcome == ModelCampaignOutcome::Passed)
        .count() as u32;
    let infra = evidence
        .trial_results
        .iter()
        .filter(|trial| {
            matches!(
                trial.outcome,
                ModelCampaignOutcome::InfraError | ModelCampaignOutcome::SimulatorError
            )
        })
        .count() as u32;
    (total, passed, total.saturating_sub(passed + infra), infra)
}
