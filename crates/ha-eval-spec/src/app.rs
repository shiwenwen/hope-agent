//! Product-independent protocol for the desktop evaluation center.
//!
//! App plans deliberately wrap, rather than replace, the release model
//! campaign protocol. A local plan may narrow registered assets and bind one
//! concrete model per child campaign, but it is never accepted as release
//! evidence.

use super::model::{
    validate_campaign_budget, CampaignBudget, ModelCampaignOutcome, ModelCampaignPlan,
    ModelCampaignSource, ModelCampaignTier, ModelProfile,
};
use super::{digest_serializable, validate_identifier, ArtifactDigest};
use anyhow::{bail, Result};
use base64::Engine;
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Component, Path};

pub const APP_PROFILE_SCHEMA_VERSION: &str = "eval-app-profile.v1";
pub const APP_REQUEST_SCHEMA_VERSION: &str = "eval-app-run-request.v1";
pub const APP_PLAN_SCHEMA_VERSION: &str = "eval-app-plan.v1";
pub const EVIDENCE_BUNDLE_SCHEMA_VERSION: &str = "eval-evidence-bundle.v1";
pub const EVIDENCE_TRUST_SCHEMA_VERSION: &str = "eval-evidence-trust.v1";
pub const APP_CONTROL_PROTOCOL_VERSION: &str = "eval-app-control.v1";

pub const APP_MAX_MODELS: usize = 4;
pub const APP_MAX_TRIALS: usize = 500;
/// App concurrency is bounded only by the largest supported local plan. A
/// profile may narrow this to its own maximum trial count, but must not impose
/// an unrelated low ceiling on a user-controlled diagnostic run.
pub const APP_MAX_CONCURRENCY: u32 = APP_MAX_TRIALS as u32;
/// A protocol sanity bound, not a recommended spend. Explicit user consent and
/// the value entered in the App remain the effective cost guardrail.
pub const APP_MAX_COST_USD: f64 = 1_000_000.0;

fn default_trial_attempt() -> u8 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppExecutionProfile {
    LocalNativeDiagnostic,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppDebugRetention {
    MetricsOnly,
    #[default]
    Redacted,
    FullLocal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppArmMode {
    OneControlPerCase,
    AllAllowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkEnforcement {
    Unverified,
    Enforced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeEnvironmentSnapshot {
    pub actual_runner_class: String,
    pub os: String,
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    pub network_enforcement: NetworkEnforcement,
    pub product_binary_digest: String,
    pub runner_binary_digest: String,
    pub asset_root_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardware_class_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_image_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppProfileSuiteSelection {
    pub suite_id: String,
    #[serde(default)]
    pub case_tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalAppProfile {
    pub schema_version: String,
    pub id: String,
    pub version: String,
    pub title: String,
    pub description: String,
    pub base_tier: ModelCampaignTier,
    pub suites: Vec<AppProfileSuiteSelection>,
    pub allowed_arms: Vec<String>,
    pub arm_mode: AppArmMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_repetitions: Option<u16>,
    #[serde(default)]
    pub use_suite_repetitions: bool,
    pub max_trials: u16,
    pub max_models: u8,
    pub max_concurrency: u16,
    pub max_cost_usd: f64,
    /// Optional App-only ceiling for one trial. Registered suite/scenario
    /// limits remain authoritative when they are stricter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_trial_seconds: Option<u64>,
    #[serde(default)]
    pub allow_custom: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppSuiteRequest {
    pub suite_id: String,
    #[serde(default)]
    pub case_ids: Vec<String>,
    #[serde(default)]
    pub arms: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repetitions: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppModelSelection {
    pub provider_id: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_profile_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
}

impl AppModelSelection {
    /// Return the form that may be persisted. The backend-only credential
    /// reference is useful only while resolving the selected Provider.
    pub fn redacted(&self) -> Self {
        Self {
            credential_profile_ref: None,
            ..self.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppEvalConsent {
    pub model_costs: bool,
    pub synthetic_tool_execution: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalAppRunRequest {
    pub schema_version: String,
    pub profile_id: String,
    #[serde(default)]
    pub suite_selections: Vec<AppSuiteRequest>,
    pub models: Vec<AppModelSelection>,
    pub campaign_budget: CampaignBudget,
    #[serde(default)]
    pub debug_retention: AppDebugRetention,
    pub consent: AppEvalConsent,
}

impl EvalAppRunRequest {
    pub fn redacted(&self) -> Self {
        Self {
            models: self
                .models
                .iter()
                .map(AppModelSelection::redacted)
                .collect(),
            ..self.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppResolvedCampaign {
    pub campaign_id: String,
    pub plan_digest: String,
    pub model: ModelProfile,
    pub model_digest: String,
    pub provider_config_digest: String,
    pub credential_config_digest: String,
    pub resolved_plan: ModelCampaignPlan,
}

/// Backend-resolved model binding used only to build an immutable App plan.
/// The digests describe the credential-free Provider configuration and the
/// selected credential profile metadata; no secret value is carried here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppResolvedModelBinding {
    pub model: ModelProfile,
    pub provider_config_digest: String,
    pub credential_config_digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalAppPlan {
    pub schema_version: String,
    pub experiment_id: String,
    pub plan_digest: String,
    pub reference: String,
    pub dirty: bool,
    pub app_version: String,
    pub source: ModelCampaignSource,
    pub execution_profile: AppExecutionProfile,
    pub profile_id: String,
    pub profile_version: String,
    pub profile_digest: String,
    pub selection_digest: String,
    pub asset_root_digest: String,
    pub runtime_environment: RuntimeEnvironmentSnapshot,
    pub debug_retention: AppDebugRetention,
    pub campaign_budget: CampaignBudget,
    pub campaigns: Vec<AppResolvedCampaign>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityMetric {
    Functional,
    Tokens,
    WallTime,
    ToolCalls,
    UsdCost,
    MultiAgent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonCompatibility {
    Incompatible,
    DiagnosticOnly,
    Functional,
    Exact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompatibilityFingerprint {
    pub suite_id: String,
    pub case_id: String,
    pub scenario_digest: String,
    pub verifier_digest: String,
    pub prompt_digest: String,
    pub tool_schema_digest: String,
    pub model_digest: String,
    pub runtime_config_digest: String,
    pub execution_mode: String,
    pub arm: String,
    pub source: ModelCampaignSource,
    pub runner_class: String,
    pub network_enforcement: NetworkEnforcement,
    pub runner_os: String,
    pub runner_arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardware_class_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokenizer_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_snapshot_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compute_budget_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trial_seed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompatibilityAssessment {
    pub compatibility: ComparisonCompatibility,
    #[serde(default)]
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKeyStatus {
    Active,
    Revoked,
    Retired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceTrustKey {
    pub id: String,
    pub algorithm: String,
    pub public_key: String,
    pub status: EvidenceKeyStatus,
    pub valid_from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceTrustRegistry {
    pub schema_version: String,
    pub version: String,
    pub keys: Vec<EvidenceTrustKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceBundleManifest {
    pub schema_version: String,
    pub repository: String,
    pub workflow: String,
    pub workflow_run_id: String,
    pub environment: String,
    pub commit_sha: String,
    pub tier: ModelCampaignTier,
    pub created_at: String,
    pub key_id: String,
    pub evidence: ArtifactDigest,
    #[serde(default)]
    pub artifacts: Vec<ArtifactDigest>,
}

/// Credential-free readiness information emitted before the desktop sends
/// any command. The binary and asset digests make version skew explicit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppControlHello {
    pub protocol_version: String,
    pub product_version: String,
    pub runner_digest: String,
    pub asset_root_digest: String,
    pub version_lock_digest: String,
    pub os: String,
    pub arch: String,
    pub adapters: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppEvalCaseCatalog {
    pub id: String,
    pub title: String,
    pub tags: Vec<String>,
    pub arms: Vec<String>,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppEvalSuiteCatalog {
    pub id: String,
    pub version: String,
    pub capability: String,
    pub cases: Vec<AppEvalCaseCatalog>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AppControlCommand {
    HelloAck {
        protocol_version: String,
        product_version: String,
        asset_root_digest: String,
    },
    ListProfiles,
    ListCatalog,
    Preview {
        request: EvalAppRunRequest,
        resolved_models: Vec<AppResolvedModelBinding>,
        reference: String,
        dirty: bool,
        app_version: String,
        runtime_environment: RuntimeEnvironmentSnapshot,
    },
    Start {
        request: EvalAppRunRequest,
        resolved_models: Vec<AppResolvedModelBinding>,
        reference: String,
        dirty: bool,
        app_version: String,
        runtime_environment: RuntimeEnvironmentSnapshot,
        product_binary: String,
        product_binary_digest: String,
        output_root: String,
        /// Credential-free isolated Hope config. Provider secrets are sent in
        /// the separate opaque field below and are never written to disk.
        config: Value,
        provider_secrets_b64: String,
    },
    Cancel {
        experiment_id: String,
    },
    Shutdown,
}

impl AppControlCommand {
    pub fn correlation_id(&self) -> Option<&str> {
        match self {
            Self::Cancel { experiment_id } => Some(experiment_id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AppControlEvent {
    Hello {
        hello: AppControlHello,
    },
    Ready,
    Profiles {
        profiles: Vec<EvalAppProfile>,
    },
    Catalog {
        suites: Vec<AppEvalSuiteCatalog>,
    },
    Preview {
        plan: EvalAppPlan,
    },
    Started {
        experiment_id: String,
        plan_digest: String,
    },
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
        loop_iterations: u64,
        spawned_agents: u64,
        async_jobs: u64,
        active_children: u64,
        attribution: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_event: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
        #[serde(default)]
        model_calls: u64,
        #[serde(default)]
        tool_calls: u64,
        #[serde(default)]
        suite_id: String,
        #[serde(default)]
        case_id: String,
        #[serde(default)]
        arm: String,
        #[serde(default = "default_trial_attempt")]
        attempt: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
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
    CampaignCompleted {
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
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        experiment_id: Option<String>,
        code: String,
        message: String,
        recoverable: bool,
    },
    Bye,
}

impl AppControlEvent {
    pub fn correlation_id(&self) -> Option<&str> {
        match self {
            Self::Started { experiment_id, .. }
            | Self::Phase { experiment_id, .. }
            | Self::Completed { experiment_id, .. }
            | Self::Cancelled { experiment_id }
            | Self::BudgetWarning { experiment_id, .. } => Some(experiment_id),
            Self::TrialStarted { campaign_id, .. }
            | Self::TrialProgress { campaign_id, .. }
            | Self::TrialCompleted { campaign_id, .. }
            | Self::ArtifactWritten { campaign_id, .. }
            | Self::CampaignCompleted { campaign_id, .. } => Some(campaign_id),
            Self::Error { experiment_id, .. } => experiment_id.as_deref(),
            Self::Hello { .. }
            | Self::Ready
            | Self::Profiles { .. }
            | Self::Catalog { .. }
            | Self::Preview { .. }
            | Self::Bye => None,
        }
    }
}

/// Versioned JSONL wire envelope. Sequence numbers are monotonic per stream;
/// receivers reject gaps and ignore duplicates before applying payloads to the
/// database-backed state machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppControlEnvelope<T> {
    pub protocol_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<String>,
    pub seq: u64,
    pub timestamp: String,
    pub payload: T,
}

pub fn validate_app_control_envelope<T>(envelope: &AppControlEnvelope<T>) -> Result<()> {
    if envelope.protocol_version != APP_CONTROL_PROTOCOL_VERSION
        || envelope.seq == 0
        || envelope.timestamp.trim().is_empty()
        || envelope.timestamp.len() > 64
        || envelope
            .campaign_id
            .as_deref()
            .is_some_and(|value| value.is_empty() || value.len() > 256)
    {
        bail!("invalid App control protocol envelope");
    }
    Ok(())
}

pub fn validate_app_profile(profile: &EvalAppProfile) -> Result<()> {
    if profile.schema_version != APP_PROFILE_SCHEMA_VERSION {
        bail!("app profile {} has unsupported schemaVersion", profile.id);
    }
    validate_identifier("app profile id", &profile.id)?;
    if profile.version.trim().is_empty()
        || profile.title.trim().is_empty()
        || profile.description.trim().is_empty()
        || profile.suites.is_empty()
        || profile.allowed_arms.is_empty()
    {
        bail!("app profile {} is incomplete", profile.id);
    }
    if profile.base_tier == ModelCampaignTier::Release {
        bail!("app profile may not use the release tier");
    }
    if !(1..=APP_MAX_TRIALS as u16).contains(&profile.max_trials)
        || !(1..=APP_MAX_MODELS as u8).contains(&profile.max_models)
        || !(1..=APP_MAX_CONCURRENCY as u16).contains(&profile.max_concurrency)
        || !profile.max_cost_usd.is_finite()
        || !(0.0..=APP_MAX_COST_USD).contains(&profile.max_cost_usd)
        || profile.max_cost_usd == 0.0
    {
        bail!("app profile {} exceeds local safety limits", profile.id);
    }
    if profile
        .default_repetitions
        .is_some_and(|value| !(1..=5).contains(&value))
    {
        bail!("app profile repetitions must be 1..=5");
    }
    if profile
        .max_trial_seconds
        .is_some_and(|value| !(30..=900).contains(&value))
    {
        bail!("app profile maxTrialSeconds must be 30..=900");
    }
    let mut suites = BTreeSet::new();
    for suite in &profile.suites {
        validate_identifier("app profile suite id", &suite.suite_id)?;
        if !suites.insert(suite.suite_id.as_str()) {
            bail!("app profile contains duplicate suite {}", suite.suite_id);
        }
        for tag in &suite.case_tags {
            validate_identifier("app profile case tag", tag)?;
        }
    }
    let mut arms = BTreeSet::new();
    for arm in &profile.allowed_arms {
        validate_identifier("app profile arm", arm)?;
        if !arms.insert(arm.as_str()) {
            bail!("app profile contains duplicate arm {arm}");
        }
    }
    if !profile
        .allowed_arms
        .iter()
        .any(|arm| arm == "control" || arm.ends_with("_control"))
    {
        bail!("app profile must allow a clean control arm");
    }
    Ok(())
}

pub fn validate_app_request(request: &EvalAppRunRequest) -> Result<()> {
    if request.schema_version != APP_REQUEST_SCHEMA_VERSION {
        bail!("unsupported app run request schemaVersion");
    }
    validate_identifier("app profile id", &request.profile_id)?;
    if !request.consent.model_costs || !request.consent.synthetic_tool_execution {
        bail!("app evaluation requires explicit model-cost and synthetic-tool consent");
    }
    if request.models.is_empty() || request.models.len() > APP_MAX_MODELS {
        bail!("app evaluation requires 1..={APP_MAX_MODELS} models");
    }
    let mut models = BTreeSet::new();
    for model in &request.models {
        validate_identifier("app model provider id", &model.provider_id)?;
        if model.model_id.trim().is_empty() || model.max_output_tokens == Some(0) {
            bail!("app model selection is invalid");
        }
        if !models.insert((model.provider_id.as_str(), model.model_id.as_str())) {
            bail!("app evaluation contains a duplicate model selection");
        }
    }
    let mut suites = BTreeSet::new();
    for suite in &request.suite_selections {
        validate_identifier("app request suite id", &suite.suite_id)?;
        if !suites.insert(suite.suite_id.as_str()) {
            bail!("app request contains duplicate suite {}", suite.suite_id);
        }
        if suite
            .repetitions
            .is_some_and(|value| !(1..=5).contains(&value))
        {
            bail!("app request repetitions must be 1..=5");
        }
        for id in &suite.case_ids {
            validate_identifier("app request case id", id)?;
        }
        for arm in &suite.arms {
            validate_identifier("app request arm", arm)?;
        }
    }
    validate_campaign_budget(&request.campaign_budget, "app campaign")?;
    if request
        .campaign_budget
        .max_cost_usd
        .is_some_and(|value| value > APP_MAX_COST_USD)
        || request
            .campaign_budget
            .max_concurrency
            .is_some_and(|value| value > APP_MAX_CONCURRENCY)
    {
        bail!("app campaign budget exceeds local safety limits");
    }
    if request.campaign_budget.max_cost_usd.is_none()
        && request.campaign_budget.max_input_tokens.is_none()
        && request.campaign_budget.max_model_calls.is_none()
    {
        bail!("app campaign requires a cost, token, or model-call hard limit");
    }
    Ok(())
}

pub fn validate_runtime_environment(runtime: &RuntimeEnvironmentSnapshot) -> Result<()> {
    if runtime.actual_runner_class.trim().is_empty()
        || runtime.os.trim().is_empty()
        || runtime.arch.trim().is_empty()
    {
        bail!("runtime environment identity is incomplete");
    }
    for (label, digest) in [
        ("product binary", runtime.product_binary_digest.as_str()),
        ("runner binary", runtime.runner_binary_digest.as_str()),
        ("asset root", runtime.asset_root_digest.as_str()),
    ] {
        validate_sha256(digest, label)?;
    }
    for (label, digest) in [
        ("hardware class", runtime.hardware_class_digest.as_deref()),
        ("container image", runtime.container_image_digest.as_deref()),
    ] {
        if let Some(digest) = digest {
            validate_sha256(digest, label)?;
        }
    }
    Ok(())
}

pub fn validate_app_plan(plan: &EvalAppPlan) -> Result<()> {
    if plan.schema_version != APP_PLAN_SCHEMA_VERSION
        || plan.source != ModelCampaignSource::LocalApp
        || plan.execution_profile != AppExecutionProfile::LocalNativeDiagnostic
    {
        bail!("unsupported app plan schema/source/execution profile");
    }
    validate_identifier("app plan profile id", &plan.profile_id)?;
    if plan.reference.trim().is_empty() || plan.app_version.trim().is_empty() {
        bail!("app plan is missing build identity");
    }
    for (label, digest) in [
        ("app plan", plan.plan_digest.as_str()),
        ("profile", plan.profile_digest.as_str()),
        ("selection", plan.selection_digest.as_str()),
        ("asset root", plan.asset_root_digest.as_str()),
    ] {
        validate_sha256(digest, label)?;
    }
    validate_runtime_environment(&plan.runtime_environment)?;
    if plan.runtime_environment.asset_root_digest != plan.asset_root_digest {
        bail!("app plan runtime and asset digests differ");
    }
    validate_campaign_budget(&plan.campaign_budget, "app plan campaign")?;
    if plan.campaigns.is_empty() || plan.campaigns.len() > APP_MAX_MODELS {
        bail!("app plan requires 1..={APP_MAX_MODELS} child campaigns");
    }
    let mut campaign_ids = BTreeSet::new();
    let mut total_trials = 0usize;
    for campaign in &plan.campaigns {
        if campaign.campaign_id.trim().is_empty()
            || !campaign_ids.insert(campaign.campaign_id.as_str())
        {
            bail!("app plan contains an invalid or duplicate campaign id");
        }
        for (label, digest) in [
            ("child plan", campaign.plan_digest.as_str()),
            ("model", campaign.model_digest.as_str()),
            ("provider config", campaign.provider_config_digest.as_str()),
            (
                "credential config",
                campaign.credential_config_digest.as_str(),
            ),
        ] {
            validate_sha256(digest, label)?;
        }
        super::model::validate_model_profile(&campaign.model)?;
        if campaign.resolved_plan.suites.is_empty() || campaign.resolved_plan.trials.is_empty() {
            bail!("app child campaign must contain suites and trials");
        }
        if campaign.resolved_plan.campaign_id != campaign.campaign_id
            || campaign.resolved_plan.plan_digest != campaign.plan_digest
            || campaign.resolved_plan.models.as_slice() != [campaign.model.clone()]
            || campaign
                .resolved_plan
                .trials
                .iter()
                .any(|trial| trial.campaign_id != campaign.campaign_id)
        {
            bail!("app child campaign wrapper and resolved plan differ");
        }
        total_trials += campaign.resolved_plan.trials.len();
    }
    if total_trials > APP_MAX_TRIALS {
        bail!("app plan exceeds the {APP_MAX_TRIALS}-trial safety limit");
    }
    Ok(())
}

pub fn app_profile_digest(profile: &EvalAppProfile) -> Result<String> {
    validate_app_profile(profile)?;
    digest_serializable(profile)
}

pub fn compatibility_for(
    left: &CompatibilityFingerprint,
    right: &CompatibilityFingerprint,
    metric: CompatibilityMetric,
) -> CompatibilityAssessment {
    let mut incompatible = Vec::new();
    let mut diagnostic = Vec::new();
    for (label, matches) in [
        ("suite", left.suite_id == right.suite_id),
        ("case", left.case_id == right.case_id),
        ("scenario", left.scenario_digest == right.scenario_digest),
        ("verifier", left.verifier_digest == right.verifier_digest),
        ("prompt", left.prompt_digest == right.prompt_digest),
        (
            "tool_schema",
            left.tool_schema_digest == right.tool_schema_digest,
        ),
        ("model", left.model_digest == right.model_digest),
        (
            "runtime_config",
            !left.runtime_config_digest.is_empty()
                && left.runtime_config_digest == right.runtime_config_digest,
        ),
        (
            "execution_mode",
            left.execution_mode == right.execution_mode,
        ),
        ("arm", left.arm == right.arm),
    ] {
        if !matches {
            incompatible.push(format!("{label}_mismatch"));
        }
    }
    match metric {
        CompatibilityMetric::Tokens => {
            require_optional_equal_or_diagnostic(
                "tokenizer",
                &left.tokenizer_digest,
                &right.tokenizer_digest,
                &mut incompatible,
                &mut diagnostic,
            );
            require_optional_equal_or_diagnostic(
                "usage_source",
                &left.usage_source,
                &right.usage_source,
                &mut incompatible,
                &mut diagnostic,
            );
        }
        CompatibilityMetric::UsdCost => require_optional_equal(
            "price_snapshot",
            &left.price_snapshot_digest,
            &right.price_snapshot_digest,
            &mut incompatible,
        ),
        CompatibilityMetric::MultiAgent => {
            require_optional_equal(
                "compute_budget",
                &left.compute_budget_digest,
                &right.compute_budget_digest,
                &mut incompatible,
            );
            if left.trial_seed.is_none() || left.trial_seed != right.trial_seed {
                incompatible.push("trial_seed_mismatch".to_string());
            }
        }
        CompatibilityMetric::Functional
        | CompatibilityMetric::WallTime
        | CompatibilityMetric::ToolCalls => {}
    }
    if !incompatible.is_empty() {
        return CompatibilityAssessment {
            compatibility: ComparisonCompatibility::Incompatible,
            reasons: incompatible,
        };
    }

    if left.source.is_release_eligible() != right.source.is_release_eligible() {
        diagnostic.push("evidence_source_class_mismatch".to_string());
    }
    if left.network_enforcement != right.network_enforcement {
        diagnostic.push("network_enforcement_mismatch".to_string());
    }
    if left.runner_class != right.runner_class {
        diagnostic.push("runner_class_mismatch".to_string());
    }
    if metric == CompatibilityMetric::WallTime
        && (left.runner_os != right.runner_os
            || left.runner_arch != right.runner_arch
            || left.hardware_class_digest.is_none()
            || left.hardware_class_digest != right.hardware_class_digest)
    {
        diagnostic.push("wall_time_environment_mismatch".to_string());
    }
    if !diagnostic.is_empty() {
        return CompatibilityAssessment {
            compatibility: ComparisonCompatibility::DiagnosticOnly,
            reasons: diagnostic,
        };
    }

    let exact = left == right;
    CompatibilityAssessment {
        compatibility: if exact {
            ComparisonCompatibility::Exact
        } else {
            ComparisonCompatibility::Functional
        },
        reasons: if exact {
            Vec::new()
        } else {
            vec!["non_blocking_runtime_difference".to_string()]
        },
    }
}

fn require_optional_equal_or_diagnostic<T: PartialEq>(
    label: &str,
    left: &Option<T>,
    right: &Option<T>,
    incompatible: &mut Vec<String>,
    diagnostic: &mut Vec<String>,
) {
    match (left, right) {
        (Some(left), Some(right)) if left == right => {}
        (None, None) => diagnostic.push(format!("{label}_unavailable")),
        _ => incompatible.push(format!("{label}_mismatch")),
    }
}

fn require_optional_equal<T: PartialEq>(
    label: &str,
    left: &Option<T>,
    right: &Option<T>,
    reasons: &mut Vec<String>,
) {
    if left.is_none() || left != right {
        reasons.push(format!("{label}_mismatch"));
    }
}

pub fn validate_trust_registry(registry: &EvidenceTrustRegistry) -> Result<()> {
    if registry.schema_version != EVIDENCE_TRUST_SCHEMA_VERSION
        || registry.version.trim().is_empty()
        || registry.keys.is_empty()
    {
        bail!("evidence trust registry is incomplete");
    }
    let mut ids = BTreeSet::new();
    for key in &registry.keys {
        validate_identifier("evidence key id", &key.id)?;
        let public_key = base64::engine::general_purpose::STANDARD
            .decode(key.public_key.trim())
            .map_err(|_| anyhow::anyhow!("evidence trust key {} is not valid base64", key.id))?;
        let valid_from = DateTime::parse_from_rfc3339(&key.valid_from)
            .map_err(|_| anyhow::anyhow!("evidence trust key {} has invalid validFrom", key.id))?;
        if !ids.insert(key.id.as_str()) || key.algorithm != "ed25519" || public_key.len() != 32 {
            bail!("evidence trust key {} is invalid", key.id);
        }
        let valid_until = key
            .valid_until
            .as_deref()
            .map(DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(|_| anyhow::anyhow!("evidence trust key {} has invalid validUntil", key.id))?;
        let revoked_at = key
            .revoked_at
            .as_deref()
            .map(DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(|_| anyhow::anyhow!("evidence trust key {} has invalid revokedAt", key.id))?;
        if valid_until.is_some_and(|until| until < valid_from)
            || revoked_at.is_some_and(|revoked| revoked < valid_from)
        {
            bail!(
                "evidence trust key {} has an invalid validity window",
                key.id
            );
        }
        if key.status == EvidenceKeyStatus::Revoked && key.revoked_at.is_none() {
            bail!("revoked evidence key {} must record revokedAt", key.id);
        }
        if key.status == EvidenceKeyStatus::Retired && key.valid_until.is_none() {
            bail!("retired evidence key {} must record validUntil", key.id);
        }
        if key.status != EvidenceKeyStatus::Revoked && key.revoked_at.is_some() {
            bail!(
                "non-revoked evidence key {} cannot record revokedAt",
                key.id
            );
        }
    }
    Ok(())
}

/// Stable identity for one trusted signing key. Key IDs are operator-friendly
/// labels and may never be sufficient on their own: trust refresh must also
/// prove that the registry still contains the exact Ed25519 public key that
/// verified the imported bundle.
pub fn evidence_trust_key_fingerprint(key: &EvidenceTrustKey) -> Result<String> {
    let public_key = base64::engine::general_purpose::STANDARD
        .decode(key.public_key.trim())
        .map_err(|_| anyhow::anyhow!("evidence trust key {} is not valid base64", key.id))?;
    if key.algorithm != "ed25519" || public_key.len() != 32 {
        bail!("evidence trust key {} is invalid", key.id);
    }
    Ok(crate::sha256_bytes(&public_key))
}

pub fn validate_evidence_bundle_manifest(manifest: &EvidenceBundleManifest) -> Result<()> {
    if manifest.schema_version != EVIDENCE_BUNDLE_SCHEMA_VERSION
        || manifest.repository.trim().is_empty()
        || manifest.workflow.trim().is_empty()
        || manifest.workflow_run_id.trim().is_empty()
        || manifest.environment.trim().is_empty()
        || manifest.created_at.trim().is_empty()
    {
        bail!("evidence bundle manifest is incomplete");
    }
    validate_identifier("evidence key id", &manifest.key_id)?;
    DateTime::parse_from_rfc3339(&manifest.created_at)
        .map_err(|_| anyhow::anyhow!("evidence bundle createdAt is invalid"))?;
    if manifest.commit_sha.len() != 40
        || !manifest
            .commit_sha
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("evidence bundle commitSha is invalid");
    }
    let mut paths = BTreeSet::from(["manifest.json", "manifest.sig"]);
    for artifact in std::iter::once(&manifest.evidence).chain(manifest.artifacts.iter()) {
        validate_sha256(&artifact.sha256, "evidence bundle artifact")?;
        if !safe_relative_path(&artifact.path) || !paths.insert(artifact.path.as_str()) {
            bail!("evidence bundle contains an unsafe or duplicate artifact path");
        }
    }
    Ok(())
}

fn safe_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !value.contains('\\')
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("{label} must be a lowercase SHA-256 digest");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(source: ModelCampaignSource) -> CompatibilityFingerprint {
        CompatibilityFingerprint {
            suite_id: "suite".into(),
            case_id: "case".into(),
            scenario_digest: "1".repeat(64),
            verifier_digest: "2".repeat(64),
            prompt_digest: "3".repeat(64),
            tool_schema_digest: "4".repeat(64),
            model_digest: "5".repeat(64),
            runtime_config_digest: "6".repeat(64),
            execution_mode: "native_provider".into(),
            arm: "control".into(),
            source,
            runner_class: "dedicated_linux".into(),
            network_enforcement: NetworkEnforcement::Enforced,
            runner_os: "linux".into(),
            runner_arch: "x86_64".into(),
            hardware_class_digest: Some("7".repeat(64)),
            tokenizer_digest: Some("8".repeat(64)),
            usage_source: Some("provider".into()),
            price_snapshot_digest: Some("9".repeat(64)),
            compute_budget_digest: Some("a".repeat(64)),
            trial_seed: Some(42),
        }
    }

    #[test]
    fn local_and_protected_results_are_diagnostic_only() {
        let local = fingerprint(ModelCampaignSource::LocalApp);
        let protected = fingerprint(ModelCampaignSource::GithubActions);
        let assessment = compatibility_for(&local, &protected, CompatibilityMetric::Functional);
        assert_eq!(
            assessment.compatibility,
            ComparisonCompatibility::DiagnosticOnly
        );
    }

    #[test]
    fn price_change_breaks_usd_comparison() {
        let left = fingerprint(ModelCampaignSource::GithubActions);
        let mut right = left.clone();
        right.price_snapshot_digest = Some("b".repeat(64));
        let assessment = compatibility_for(&left, &right, CompatibilityMetric::UsdCost);
        assert_eq!(
            assessment.compatibility,
            ComparisonCompatibility::Incompatible
        );
    }

    #[test]
    fn request_redaction_removes_credential_reference() {
        let request = EvalAppRunRequest {
            schema_version: APP_REQUEST_SCHEMA_VERSION.into(),
            profile_id: "quick".into(),
            suite_selections: vec![],
            models: vec![AppModelSelection {
                provider_id: "provider".into(),
                model_id: "model".into(),
                credential_profile_ref: Some("secret-backend-ref".into()),
                reasoning_effort: None,
                max_output_tokens: None,
            }],
            campaign_budget: CampaignBudget {
                max_model_calls: Some(10),
                ..CampaignBudget::default()
            },
            debug_retention: AppDebugRetention::Redacted,
            consent: AppEvalConsent {
                model_costs: true,
                synthetic_tool_execution: true,
            },
        };
        validate_app_request(&request).unwrap();
        assert!(request.redacted().models[0]
            .credential_profile_ref
            .is_none());
    }

    #[test]
    fn trial_completed_metrics_are_backward_compatible() {
        let event: AppControlEvent = serde_json::from_value(serde_json::json!({
            "type": "trial_completed",
            "experimentId": "experiment",
            "campaignId": "campaign",
            "trialId": "trial",
            "completed": 1,
            "total": 1,
            "outcome": "passed",
            "wallMs": 123
        }))
        .unwrap();
        match event {
            AppControlEvent::TrialCompleted {
                model_calls,
                tool_calls,
                ..
            } => {
                assert_eq!(model_calls, 0);
                assert_eq!(tool_calls, 0);
            }
            _ => panic!("expected trial_completed"),
        }
    }

    #[test]
    fn trust_registry_requires_a_real_ed25519_public_key() {
        let mut registry = EvidenceTrustRegistry {
            schema_version: EVIDENCE_TRUST_SCHEMA_VERSION.into(),
            version: "1.0.0".into(),
            keys: vec![EvidenceTrustKey {
                id: "model-eval-test".into(),
                algorithm: "ed25519".into(),
                public_key: base64::engine::general_purpose::STANDARD.encode([7u8; 32]),
                status: EvidenceKeyStatus::Active,
                valid_from: "2026-07-18T00:00:00Z".into(),
                valid_until: None,
                revoked_at: None,
            }],
        };
        validate_trust_registry(&registry).unwrap();
        assert_eq!(
            evidence_trust_key_fingerprint(&registry.keys[0]).unwrap(),
            crate::sha256_bytes(&[7u8; 32])
        );
        registry.keys[0].public_key = base64::engine::general_purpose::STANDARD.encode([7u8; 31]);
        assert!(validate_trust_registry(&registry).is_err());
        assert!(evidence_trust_key_fingerprint(&registry.keys[0]).is_err());
    }

    #[test]
    fn evidence_manifest_reserves_protocol_entry_names() {
        let manifest = EvidenceBundleManifest {
            schema_version: EVIDENCE_BUNDLE_SCHEMA_VERSION.into(),
            repository: "shiwenwen/hope-agent".into(),
            workflow: "workflow".into(),
            workflow_run_id: "1".into(),
            environment: "model-eval".into(),
            commit_sha: "a".repeat(40),
            tier: ModelCampaignTier::Weekly,
            created_at: "2026-07-18T00:00:00Z".into(),
            key_id: "model-eval-test".into(),
            evidence: ArtifactDigest {
                path: "manifest.json".into(),
                sha256: "b".repeat(64),
            },
            artifacts: Vec::new(),
        };
        assert!(validate_evidence_bundle_manifest(&manifest).is_err());
    }
}
