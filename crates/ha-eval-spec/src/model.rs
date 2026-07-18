//! Product-independent protocol for real-model evaluation campaigns.
//!
//! These types deliberately do not share the deterministic adapter enum or
//! evidence schema. A real-model result can therefore never deserialize as
//! release `eval-evidence.v1` by accident.

use super::{
    canonical_json, digest_file, digest_serializable, resolve_contained, sha256_bytes,
    validate_identifier, ArtifactDigest, EvalStatus, PolicyMode,
};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

pub const SCENARIO_SCHEMA_VERSION: &str = "live-agent-scenario.v1";
pub const SUITE_SCHEMA_VERSION: &str = "model-campaign-suite.v1";
pub const POLICY_SCHEMA_VERSION: &str = "model-campaign-policy.v1";
pub const PLAN_SCHEMA_VERSION: &str = "model-campaign-plan.v1";
pub const TRIAL_SCHEMA_VERSION: &str = "model-campaign-trial.v1";
pub const SHARD_SCHEMA_VERSION: &str = "model-campaign-shard.v1";
pub const EVIDENCE_SCHEMA_VERSION: &str = "eval-model-campaign.v1";
pub const WAIVER_SCHEMA_VERSION: &str = "model-campaign-waiver.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCampaignTier {
    Nightly,
    Weekly,
    Release,
    Monthly,
}

impl ModelCampaignTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nightly => "nightly",
            Self::Weekly => "weekly",
            Self::Release => "release",
            Self::Monthly => "monthly",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCampaignSource {
    LocalApp,
    LocalCli,
    DedicatedRunner,
    GithubActions,
}

impl ModelCampaignSource {
    pub fn is_release_eligible(self) -> bool {
        matches!(self, Self::DedicatedRunner | Self::GithubActions)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCampaignAdapter {
    HopeCoreScenario,
    BfclV4,
    #[serde(rename = "appworld")]
    AppWorld,
    Gaia2,
    TerminalBench,
    Tau3,
    #[serde(rename = "teambench")]
    TeamBench,
    #[serde(rename = "cooperbench")]
    CooperBench,
    #[serde(rename = "mcpmark")]
    McpMark,
    #[serde(rename = "osworld")]
    OsWorld,
}

impl ModelCampaignAdapter {
    pub const REGISTERED_V1: [Self; 10] = [
        Self::HopeCoreScenario,
        Self::BfclV4,
        Self::AppWorld,
        Self::Gaia2,
        Self::TerminalBench,
        Self::Tau3,
        Self::TeamBench,
        Self::CooperBench,
        Self::McpMark,
        Self::OsWorld,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerClass {
    HostedLinux,
    DockerLinux,
    DedicatedLinux,
    DesktopVm,
    IsolatedExternalService,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    ProviderOnly,
    Allowlist,
    SandboxLocal,
    LiveWeb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    NativeProvider,
    BridgedProvider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioEntrypoint {
    Server,
    Acp,
    Desktop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioDriver {
    ChatTask,
    CodingCampaign,
    DomainCampaign,
    GoalLoop,
    Workflow,
    AsyncJobs,
    SubagentTeam,
    MixedE2e,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserSimulatorKind {
    ScriptedFsm,
    Replay,
    Llm,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifierKind {
    FileAssertion,
    GitAssertion,
    RegisteredCommand,
    SqliteAssertion,
    HttpAssertion,
    BrowserAssertion,
    EventAssertion,
    TraceAssertion,
    HumanOrLlmRubric,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvariantKind {
    Never,
    Before,
    After,
    AtMostOnce,
    ExactlyOnce,
    Eventually,
    EventuallyWithin,
    MaxConcurrent,
    NoOverlap,
    ParentChildClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    ProviderResponse,
    ToolResponse,
    SchedulerOrder,
    ProcessRestart,
    StorageState,
    UserEvent,
    EnvironmentEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultProfile {
    Clean,
    Chaos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCampaignOutcome {
    Passed,
    TaskFailed,
    PolicyFailed,
    BudgetExhausted,
    InfraError,
    BenchmarkDefect,
    SimulatorError,
    Cancelled,
}

impl ModelCampaignOutcome {
    pub fn aggregate_status(self) -> Option<EvalStatus> {
        match self {
            Self::Passed => Some(EvalStatus::Passed),
            Self::TaskFailed | Self::PolicyFailed | Self::BudgetExhausted => {
                Some(EvalStatus::Failed)
            }
            Self::InfraError => Some(EvalStatus::InfraError),
            Self::BenchmarkDefect | Self::SimulatorError | Self::Cancelled => None,
        }
    }

    pub fn is_valid_trial(self) -> bool {
        matches!(
            self,
            Self::Passed | Self::TaskFailed | Self::PolicyFailed | Self::BudgetExhausted
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionCompleteness {
    Complete,
    Partial,
    Missing,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CampaignBudget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wall_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_model_calls: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_calls: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_agents: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CampaignRepetitions {
    #[serde(default = "one")]
    pub nightly: u16,
    #[serde(default = "three")]
    pub weekly: u16,
    #[serde(default = "three")]
    pub release: u16,
    #[serde(default = "one")]
    pub monthly: u16,
}

impl Default for CampaignRepetitions {
    fn default() -> Self {
        Self {
            nightly: 1,
            weekly: 3,
            release: 3,
            monthly: 1,
        }
    }
}

impl CampaignRepetitions {
    pub fn for_tier(self, tier: ModelCampaignTier) -> u16 {
        match tier {
            ModelCampaignTier::Nightly => self.nightly,
            ModelCampaignTier::Weekly => self.weekly,
            ModelCampaignTier::Release => self.release,
            ModelCampaignTier::Monthly => self.monthly,
        }
    }
}

const fn one() -> u16 {
    1
}

const fn three() -> u16 {
    3
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelProfile {
    pub role: String,
    pub provider_id: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCampaignScorer {
    pub hard_verifier: String,
    #[serde(default)]
    pub milestones: bool,
    #[serde(default)]
    pub trajectory_rules: bool,
    #[serde(default)]
    pub llm_judge: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCampaignCaseSpec {
    pub id: String,
    pub scenario_path: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub model_roles: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repetitions: Option<u16>,
    /// Experiment arms are registered identifiers such as `control`,
    /// `faulted`, `team_full`, or `single_agent_compute_matched`.
    #[serde(default = "default_control_arm")]
    pub arms: Vec<String>,
}

fn default_control_arm() -> Vec<String> {
    vec!["control".to_string()]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCampaignSuite {
    pub schema_version: String,
    pub id: String,
    pub version: String,
    pub capability: String,
    pub adapter: ModelCampaignAdapter,
    pub tiers: Vec<ModelCampaignTier>,
    pub runner_class: RunnerClass,
    pub network_policy: NetworkPolicy,
    pub execution_mode: ExecutionMode,
    #[serde(default)]
    pub repetitions: CampaignRepetitions,
    #[serde(default = "default_live_timeout")]
    pub timeout_seconds: u64,
    #[serde(default = "one")]
    pub shards: u16,
    #[serde(default)]
    pub budget: CampaignBudget,
    pub scorer: ModelCampaignScorer,
    pub cases: Vec<ModelCampaignCaseSpec>,
}

const fn default_live_timeout() -> u64 {
    1_800
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelPolicySuite {
    pub id: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub case_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repetitions: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCampaignPolicy {
    pub schema_version: String,
    pub id: String,
    pub version: String,
    pub tier: ModelCampaignTier,
    pub mode: PolicyMode,
    pub allowed_adapters: Vec<ModelCampaignAdapter>,
    pub allowed_runner_classes: Vec<RunnerClass>,
    pub allowed_network_policies: Vec<NetworkPolicy>,
    pub allowed_execution_modes: Vec<ExecutionMode>,
    pub allowed_sources: Vec<ModelCampaignSource>,
    pub models: Vec<ModelProfile>,
    pub suites: Vec<ModelPolicySuite>,
    #[serde(default)]
    pub budget: CampaignBudget,
    /// Aggregate ceiling for the complete campaign. `budget` is the strict
    /// per-trial ceiling; keeping the two explicit prevents a large shard
    /// matrix from multiplying a per-trial allowance into an unbounded bill.
    #[serde(default)]
    pub campaign_budget: CampaignBudget,
    #[serde(default)]
    pub allow_llm_judge: bool,
    #[serde(default)]
    pub performance_blocking: bool,
    #[serde(default)]
    pub require_model_snapshot: bool,
    #[serde(default = "default_retention_days")]
    pub artifact_retention_days: u16,
    #[serde(default)]
    pub max_infra_error_rate: f64,
}

const fn default_retention_days() -> u16 {
    30
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScenarioSubject {
    pub entrypoint: ScenarioEntrypoint,
    pub driver: ScenarioDriver,
    pub config_profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_schema_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScenarioService {
    pub id: String,
    pub adapter: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixture_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScenarioEnvironment {
    pub runner_class: RunnerClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default)]
    pub assets: Vec<String>,
    #[serde(default)]
    pub services: Vec<ScenarioService>,
    #[serde(default)]
    pub controlled_clock: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScenarioTask {
    pub prompt_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden_truth_path: Option<String>,
    pub success_summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserSimulatorSpec {
    pub kind: UserSimulatorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_role: Option<String>,
    #[serde(default = "default_user_turns")]
    pub max_turns: u16,
}

const fn default_user_turns() -> u16 {
    12
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MilestoneSpec {
    pub id: String,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub any_of: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier: Option<String>,
    pub weight: f64,
    #[serde(default)]
    pub blocking: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FaultSpec {
    pub id: String,
    pub kind: FaultKind,
    pub trigger: String,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    #[serde(default = "one_u32")]
    pub max_activations: u32,
}

const fn one_u32() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerifierSpec {
    pub id: String,
    pub kind: VerifierKind,
    pub handler: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default = "default_verifier_timeout")]
    pub timeout_seconds: u64,
}

const fn default_verifier_timeout() -> u64 {
    60
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvariantSpec {
    pub id: String,
    pub kind: InvariantKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub then: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<u32>,
    #[serde(default = "default_true")]
    pub blocking: bool,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScenarioComparison {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_mode: Option<String>,
    #[serde(default)]
    pub ablations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScenarioNetwork {
    pub policy: NetworkPolicy,
    #[serde(default)]
    pub allow: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScenarioPrivacy {
    pub classification: String,
    #[serde(default)]
    pub redact: Vec<String>,
    #[serde(default = "default_retention_days")]
    pub raw_trace_retention_days: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveAgentScenario {
    pub schema_version: String,
    pub id: String,
    pub version: String,
    pub title: String,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub business_domains: Vec<String>,
    pub subject: ScenarioSubject,
    pub environment: ScenarioEnvironment,
    pub task: ScenarioTask,
    pub user_simulator: UserSimulatorSpec,
    #[serde(default)]
    pub milestones: Vec<MilestoneSpec>,
    #[serde(default)]
    pub faults: Vec<FaultSpec>,
    pub verifiers: Vec<VerifierSpec>,
    #[serde(default)]
    pub invariants: Vec<InvariantSpec>,
    /// Relative output paths whose content digest may be published in trial
    /// evidence. Files are never uploaded by the Rust runner.
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub budgets: CampaignBudget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison: Option<ScenarioComparison>,
    pub network: ScenarioNetwork,
    pub privacy: ScenarioPrivacy,
    pub cadence: Vec<ModelCampaignTier>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlannedModelCase {
    pub id: String,
    pub digest: String,
    pub scenario_path: String,
    pub scenario_id: String,
    pub scenario_version: String,
    pub scenario_digest: String,
    pub environment_digest: String,
    pub verifier_digest: String,
    pub prompt_digest: String,
    pub tool_schema_digest: String,
    pub timeout_seconds: u64,
    pub repetitions: u16,
    pub budget: CampaignBudget,
    pub model_roles: Vec<String>,
    pub arms: Vec<String>,
    pub driver: ScenarioDriver,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlannedModelSuite {
    pub id: String,
    pub version: String,
    pub capability: String,
    pub adapter: ModelCampaignAdapter,
    pub digest: String,
    pub runner_class: RunnerClass,
    pub network_policy: NetworkPolicy,
    pub execution_mode: ExecutionMode,
    pub shards: u16,
    pub cases: Vec<PlannedModelCase>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlannedModelTrial {
    pub id: String,
    pub campaign_id: String,
    pub plan_digest: String,
    pub commit_sha: String,
    pub app_version: String,
    pub suite_id: String,
    pub suite_version: String,
    pub suite_digest: String,
    pub case_id: String,
    pub case_digest: String,
    pub trial_index: u16,
    pub seed: u64,
    pub model_role: String,
    pub arm: String,
    pub fault_profile: FaultProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orchestration_profile: Option<String>,
    pub model_digest: String,
    pub scenario_digest: String,
    pub environment_digest: String,
    pub verifier_digest: String,
    pub prompt_digest: String,
    pub tool_schema_digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCampaignPlan {
    pub schema_version: String,
    pub campaign_id: String,
    pub plan_digest: String,
    pub reference: String,
    pub tier: ModelCampaignTier,
    pub policy_id: String,
    pub policy_version: String,
    pub policy_digest: String,
    pub runner_digest: String,
    pub app_version: String,
    pub campaign_budget: CampaignBudget,
    pub models: Vec<ModelProfile>,
    pub suites: Vec<PlannedModelSuite>,
    pub trials: Vec<PlannedModelTrial>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimingMetrics {
    pub wall_ms: u64,
    pub environment_setup_ms: u64,
    pub environment_cleanup_ms: u64,
    pub model_active_ms: u64,
    pub tool_active_ms: u64,
    pub queue_wait_ms: u64,
    pub approval_wait_ms: u64,
    pub environment_wait_ms: u64,
    pub critical_path_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttft_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenMetrics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_source: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CostMetrics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_usd: Option<f64>,
    #[serde(default)]
    pub agent_usd: Option<f64>,
    #[serde(default)]
    pub simulator_usd: Option<f64>,
    #[serde(default)]
    pub judge_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_snapshot_digest: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolMetrics {
    pub attempted: u64,
    pub logical_calls: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub retries: u64,
    pub parse_errors: u64,
    pub invalid: u64,
    pub duplicate: u64,
    pub unused_results: u64,
    pub effective: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationMetrics {
    pub model_calls: u64,
    #[serde(default)]
    pub model_retries: u64,
    pub failovers: u64,
    pub loop_iterations: u64,
    pub replans: u64,
    pub checkpoints: u64,
    pub resumes: u64,
    pub spawned_agents: u64,
    pub max_agent_depth: u32,
    pub max_concurrency: u32,
    pub handoffs: u64,
    /// Coordination-only tokens require message-level attribution. `None`
    /// means the Provider/runtime could not separate them from task tokens;
    /// it must not be serialized as a misleading measured zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordination_tokens: Option<u64>,
    #[serde(default)]
    pub child_active_ms: u64,
    pub async_jobs: u64,
    pub duplicate_injections: u64,
    pub orphaned_children: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCampaignCheck {
    pub id: String,
    pub passed: bool,
    pub blocking: bool,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<f64>,
    #[serde(default)]
    pub artifact_hashes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TraceSummary {
    pub trace_id: String,
    pub root_span_id: String,
    pub span_count: u64,
    pub orphan_span_count: u64,
    pub closed: bool,
}

/// Payload-free causal event safe to embed in the redacted JSONL trace
/// artifact. Free-form prompts, tool arguments/results, and Provider bodies
/// are intentionally absent from this protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCampaignEvent {
    pub seq: u64,
    pub event: String,
    pub timestamp_ms: u64,
    pub span_id: String,
    pub parent_span_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub status: String,
    pub duration_ms: u64,
    /// Registered, payload-free scalar attributes. Prompt text, tool
    /// arguments/results and Provider bodies are forbidden; large/sensitive
    /// values are referenced only by SHA-256 digest.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, Value>,
}

/// Validate that the redacted event stream is a closed causal graph rooted in
/// the declared trial span. This is deliberately part of the product-free
/// protocol crate so evidence cannot claim a closed trace merely by setting a
/// boolean while carrying orphaned or identity-reused events.
pub fn validate_model_trace(
    trace: &TraceSummary,
    events: &[ModelCampaignEvent],
    label: &str,
) -> Result<()> {
    if events.len() > 4_096
        || !valid_trace_identifier(&trace.trace_id, "trace_")
        || !valid_trace_identifier(&trace.root_span_id, "span_")
        || trace.orphan_span_count > 0
        || !trace.closed
    {
        bail!("{label} contains an invalid or open trace");
    }
    let mut previous_seq = 0;
    let mut previous_timestamp = 0;
    let mut spans =
        BTreeMap::<&str, (&str, Option<&str>, &str, u64, Vec<&ModelCampaignEvent>)>::new();
    for event in events {
        if event.seq == 0
            || event.seq <= previous_seq
            || event.timestamp_ms < previous_timestamp
            || event.event.is_empty()
            || event.event.len() > 128
            || event.status.is_empty()
            || event.status.len() > 64
            || event.event.contains(['\r', '\n'])
            || event.status.contains(['\r', '\n'])
            || event
                .key
                .as_deref()
                .is_some_and(|key| key.len() > 256 || key.contains(['\r', '\n']))
            || !valid_trace_identifier(&event.span_id, "span_")
            || !valid_trace_identifier(&event.parent_span_id, "span_")
            || event.span_id == trace.root_span_id
            || event.span_id == event.parent_span_id
            || !valid_trace_attributes(&event.attributes)
        {
            bail!("{label} contains an invalid trace event");
        }
        previous_seq = event.seq;
        previous_timestamp = event.timestamp_ms;
        match spans.get_mut(event.span_id.as_str()) {
            Some((event_name, key, parent, _, records)) => {
                if *event_name != event.event
                    || *key != event.key.as_deref()
                    || *parent != event.parent_span_id
                {
                    bail!("{label} reuses a span with different identity");
                }
                records.push(event);
            }
            None => {
                spans.insert(
                    event.span_id.as_str(),
                    (
                        event.event.as_str(),
                        event.key.as_deref(),
                        event.parent_span_id.as_str(),
                        event.seq,
                        vec![event],
                    ),
                );
            }
        }
    }
    if trace.span_count != 1u64.saturating_add(spans.len() as u64) {
        bail!("{label} trace span count is inconsistent");
    }
    for (span_id, (_, _, parent, first_seq, records)) in &spans {
        if *parent != trace.root_span_id {
            let Some((_, _, _, parent_seq, _)) = spans.get(parent) else {
                bail!("{label} trace contains an orphan parent span");
            };
            if parent_seq >= first_seq {
                bail!("{label} trace parent must precede its child");
            }
        }
        if records.len() > 2 {
            bail!("{label} span {span_id} has too many lifecycle events");
        }
        if records.len() == 1 && records[0].status == "started" {
            bail!("{label} span {span_id} has no terminal lifecycle event");
        }
        if records.len() == 2 {
            let terminal = records[1].status.as_str();
            if records[0].status != "started"
                || !matches!(terminal, "completed" | "succeeded" | "failed" | "cancelled")
            {
                bail!("{label} span {span_id} is not start/terminal paired");
            }
        }
    }
    Ok(())
}

/// Compute the wall-clock length of the causal frontier directly below the
/// declared trial root. Top-level Session turns, process restarts and other
/// registered root work may be sequential or overlap; taking their interval
/// union counts sequential work once and parallel work concurrently. Child
/// spans are already enclosed by their top-level causal owner and therefore
/// must not be added again.
///
/// `timestamp_ms` is the observation/end time of an event. A leaf span starts
/// `duration_ms` earlier; a paired lifecycle span starts at its `started`
/// event and ends at its terminal event. Invalid/open traces are rejected by
/// [`validate_model_trace`] before this function is used for evidence.
pub fn causal_critical_path_ms(trace: &TraceSummary, events: &[ModelCampaignEvent]) -> u64 {
    let mut spans = BTreeMap::<&str, (&str, Option<u64>, Option<u64>)>::new();
    for event in events {
        let entry = spans.entry(event.span_id.as_str()).or_insert((
            event.parent_span_id.as_str(),
            None,
            None,
        ));
        if event.status == "started" {
            entry.1 = Some(
                entry
                    .1
                    .map_or(event.timestamp_ms, |value| value.min(event.timestamp_ms)),
            );
        } else {
            let inferred_start = event.timestamp_ms.saturating_sub(event.duration_ms);
            entry.1 = Some(
                entry
                    .1
                    .map_or(inferred_start, |value| value.min(inferred_start)),
            );
            entry.2 = Some(
                entry
                    .2
                    .map_or(event.timestamp_ms, |value| value.max(event.timestamp_ms)),
            );
        }
    }
    let mut intervals = spans
        .into_values()
        .filter_map(|(parent, start, end)| (parent == trace.root_span_id).then_some((start?, end?)))
        .filter(|(start, end)| end >= start)
        .collect::<Vec<_>>();
    intervals.sort_unstable();
    let Some((mut start, mut end)) = intervals.first().copied() else {
        return 0;
    };
    let mut total = 0u64;
    for (next_start, next_end) in intervals.into_iter().skip(1) {
        if next_start <= end {
            end = end.max(next_end);
        } else {
            total = total.saturating_add(end.saturating_sub(start));
            start = next_start;
            end = next_end;
        }
    }
    total.saturating_add(end.saturating_sub(start))
}

fn valid_trace_identifier(value: &str, prefix: &str) -> bool {
    value.starts_with(prefix)
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn valid_trace_attributes(attributes: &BTreeMap<String, Value>) -> bool {
    attributes.len() <= 24
        && attributes.iter().all(|(key, value)| {
            valid_eval_scalar_key(key)
                && match value {
                    Value::Null | Value::Bool(_) => true,
                    Value::Number(number) => {
                        number.as_u64().is_some()
                            || number
                                .as_f64()
                                .is_some_and(|value| value.is_finite() && value >= 0.0)
                    }
                    Value::String(text) => {
                        text.len() <= 256
                            && !text.contains(['\r', '\n'])
                            && (!key.ends_with("Digest") || is_sha256(text))
                    }
                    Value::Array(_) | Value::Object(_) => false,
                }
        })
}

fn valid_eval_scalar_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Full, immutable record of an earlier infrastructure/simulator attempt.
/// A retry remains the same planned trial; retaining this record prevents the
/// successful retry from erasing the original failure and its usage/cost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelTrialAttemptRecord {
    pub attempt: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_config_digest: Option<String>,
    pub started_at: String,
    pub completed_at: String,
    pub outcome: ModelCampaignOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<String>,
    pub attribution: AttributionCompleteness,
    pub timings: TimingMetrics,
    pub tokens: TokenMetrics,
    pub cost: CostMetrics,
    pub tools: ToolMetrics,
    pub orchestration: OrchestrationMetrics,
    #[serde(default)]
    pub milestones: Vec<ModelCampaignCheck>,
    #[serde(default)]
    pub invariants: Vec<ModelCampaignCheck>,
    #[serde(default)]
    pub judge_checks: Vec<ModelCampaignCheck>,
    pub trace: TraceSummary,
    #[serde(default)]
    pub trace_events: Vec<ModelCampaignEvent>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactDigest>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelTrialResult {
    pub schema_version: String,
    pub trial_id: String,
    pub campaign_id: String,
    pub plan_digest: String,
    pub commit_sha: String,
    pub app_version: String,
    pub suite_id: String,
    pub suite_version: String,
    pub suite_digest: String,
    pub case_id: String,
    pub case_digest: String,
    pub trial_index: u16,
    pub seed: u64,
    pub model_role: String,
    pub arm: String,
    pub fault_profile: FaultProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orchestration_profile: Option<String>,
    pub model_digest: String,
    pub scenario_digest: String,
    pub environment_digest: String,
    pub verifier_digest: String,
    pub prompt_digest: String,
    pub tool_schema_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_config_digest: Option<String>,
    pub attempt: u8,
    #[serde(default)]
    pub prior_attempts: Vec<ModelTrialAttemptRecord>,
    pub started_at: String,
    pub completed_at: String,
    pub outcome: ModelCampaignOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<String>,
    pub attribution: AttributionCompleteness,
    pub timings: TimingMetrics,
    pub tokens: TokenMetrics,
    pub cost: CostMetrics,
    pub tools: ToolMetrics,
    pub orchestration: OrchestrationMetrics,
    #[serde(default)]
    pub milestones: Vec<ModelCampaignCheck>,
    #[serde(default)]
    pub invariants: Vec<ModelCampaignCheck>,
    #[serde(default)]
    pub judge_checks: Vec<ModelCampaignCheck>,
    pub trace: TraceSummary,
    #[serde(default)]
    pub trace_events: Vec<ModelCampaignEvent>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactDigest>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ModelTrialResult {
    pub fn as_attempt_record(&self) -> ModelTrialAttemptRecord {
        ModelTrialAttemptRecord {
            attempt: self.attempt,
            runtime_config_digest: self.runtime_config_digest.clone(),
            started_at: self.started_at.clone(),
            completed_at: self.completed_at.clone(),
            outcome: self.outcome,
            failure_class: self.failure_class.clone(),
            attribution: self.attribution,
            timings: self.timings.clone(),
            tokens: self.tokens.clone(),
            cost: self.cost.clone(),
            tools: self.tools.clone(),
            orchestration: self.orchestration.clone(),
            milestones: self.milestones.clone(),
            invariants: self.invariants.clone(),
            judge_checks: self.judge_checks.clone(),
            trace: self.trace.clone(),
            trace_events: self.trace_events.clone(),
            artifacts: self.artifacts.clone(),
            warnings: self.warnings.clone(),
            error: self.error.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelShardResult {
    pub schema_version: String,
    pub reference: String,
    pub runner_digest: String,
    pub suite_id: String,
    pub suite_digest: String,
    pub shard_index: u16,
    pub shard_total: u16,
    pub started_at: String,
    pub completed_at: String,
    pub duration_ms: u64,
    pub trace_artifact: ArtifactDigest,
    pub trials: Vec<ModelTrialResult>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CampaignCounts {
    pub scheduled: u64,
    pub retried: u64,
    pub recovered_after_retry: u64,
    pub valid: u64,
    pub passed: u64,
    pub failed: u64,
    pub policy_failed: u64,
    pub budget_exhausted: u64,
    pub infra_error: u64,
    pub benchmark_defect: u64,
    pub simulator_error: u64,
    pub cancelled: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DistributionSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p50: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CampaignMetricsSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard_success_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard_success_wilson_low: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard_success_wilson_high: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_to_end_yield: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infra_error_rate: Option<f64>,
    /// All-pass reliability grouped by the actual repetition count. A
    /// campaign may intentionally mix k=3 normal cases with k=5 critical
    /// cases, so a single global k would silently discard one group.
    #[serde(default)]
    pub reliability: Vec<ReliabilitySummary>,
    #[serde(default)]
    pub comparisons: Vec<OrchestrationComparisonSummary>,
    pub successful_wall_ms: DistributionSummary,
    pub successful_tokens: DistributionSummary,
    pub successful_cost_usd: DistributionSummary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationComparisonSummary {
    pub suite_id: String,
    pub case_id: String,
    pub model_role: String,
    pub fault_profile: FaultProfile,
    pub baseline_profile: String,
    pub candidate_profile: String,
    pub paired_valid_trials: u64,
    pub baseline_passed: u64,
    pub candidate_passed: u64,
    pub candidate_only_passed: u64,
    pub baseline_only_passed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_success_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_success_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_uplift_pp: Option<f64>,
    pub successful_wall_speedup: DistributionSummary,
    pub successful_token_amplification: DistributionSummary,
    pub successful_cost_amplification: DistributionSummary,
    pub successful_parallel_efficiency: DistributionSummary,
    pub successful_coordination_overhead: DistributionSummary,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReliabilitySummary {
    pub repetitions: u16,
    pub groups: u64,
    pub any_pass_groups: u64,
    pub all_pass_groups: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub any_pass_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_pass_rate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCampaignWaiver {
    pub schema_version: String,
    pub commit_sha: String,
    pub tag: String,
    pub reason: String,
    pub suites: Vec<String>,
    #[serde(default)]
    pub cases: Vec<String>,
    #[serde(default)]
    pub model_roles: Vec<String>,
    pub approved_by: String,
    pub approved_at: String,
    pub workflow_run_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCampaignEvidence {
    pub schema_version: String,
    pub evidence_kind: String,
    pub campaign_id: String,
    pub commit_sha: String,
    pub dirty: bool,
    pub source: ModelCampaignSource,
    pub app_version: String,
    pub tier: ModelCampaignTier,
    pub policy_id: String,
    pub policy_version: String,
    pub policy_mode: PolicyMode,
    pub policy_digest: String,
    pub runner_digest: String,
    pub runner_os: String,
    pub runner_arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_profile: Option<crate::app::AppExecutionProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_plan_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_environment: Option<crate::app::RuntimeEnvironmentSnapshot>,
    pub aggregate_status: EvalStatus,
    pub started_at: String,
    pub completed_at: String,
    pub duration_ms: u64,
    pub counts: CampaignCounts,
    pub metrics: CampaignMetricsSummary,
    pub models: Vec<ModelProfile>,
    pub campaign_budget: CampaignBudget,
    pub suites: Vec<PlannedModelSuite>,
    pub planned_trials: Vec<PlannedModelTrial>,
    pub trial_results: Vec<ModelTrialResult>,
    #[serde(default)]
    pub campaign_budget_violations: Vec<String>,
    #[serde(default)]
    pub guardrail_violations: Vec<String>,
    #[serde(default)]
    pub regressions: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactDigest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiver: Option<ModelCampaignWaiver>,
}

pub fn read_json_or_yaml(path: &Path) -> Result<Value> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading structured asset {}", path.display()))?;
    match path.extension().and_then(|value| value.to_str()) {
        Some("json") => {
            serde_json::from_str(&raw).with_context(|| format!("parsing JSON {}", path.display()))
        }
        Some("yaml" | "yml") => {
            reject_yaml_extensions(&raw)?;
            let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&raw)
                .with_context(|| format!("parsing restricted YAML {}", path.display()))?;
            reject_tagged_or_merged_yaml(&value, "$")?;
            serde_json::to_value(value).context("converting restricted YAML to JSON")
        }
        _ => bail!("structured asset must use .json, .yaml, or .yml"),
    }
}

fn reject_yaml_extensions(raw: &str) -> Result<()> {
    for (line_index, line) in raw.lines().enumerate() {
        let mut single = false;
        let mut double = false;
        let mut escaped = false;
        for ch in line.chars() {
            if escaped {
                escaped = false;
                continue;
            }
            if double && ch == '\\' {
                escaped = true;
                continue;
            }
            if !double && ch == '\'' {
                single = !single;
                continue;
            }
            if !single && ch == '"' {
                double = !double;
                continue;
            }
            if !single && !double {
                if ch == '#' {
                    break;
                }
                if matches!(ch, '&' | '*' | '!') {
                    bail!(
                        "restricted YAML rejects anchors, aliases, and tags at line {}",
                        line_index + 1
                    );
                }
            }
        }
        if line.trim_start().starts_with("<<:") {
            bail!(
                "restricted YAML rejects merge keys at line {}",
                line_index + 1
            );
        }
    }
    Ok(())
}

fn reject_tagged_or_merged_yaml(value: &serde_yaml_ng::Value, location: &str) -> Result<()> {
    match value {
        serde_yaml_ng::Value::Tagged(_) => bail!("restricted YAML rejects tags at {location}"),
        serde_yaml_ng::Value::Sequence(items) => {
            for (index, child) in items.iter().enumerate() {
                reject_tagged_or_merged_yaml(child, &format!("{location}[{index}]"))?;
            }
        }
        serde_yaml_ng::Value::Mapping(mapping) => {
            for (key, child) in mapping {
                if key.as_str() == Some("<<") {
                    bail!("restricted YAML rejects merge keys at {location}");
                }
                reject_tagged_or_merged_yaml(child, location)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn validate_campaign_budget(budget: &CampaignBudget, label: &str) -> Result<()> {
    if budget.max_wall_seconds == Some(0)
        || budget.max_model_calls == Some(0)
        || budget.max_input_tokens == Some(0)
        || budget.max_output_tokens == Some(0)
        || budget.max_tool_calls == Some(0)
        || budget.max_agents == Some(0)
        || budget.max_concurrency == Some(0)
        || budget
            .max_cost_usd
            .is_some_and(|value| !value.is_finite() || value <= 0.0)
    {
        bail!("{label} budget values must be positive and finite");
    }
    if let (Some(agents), Some(concurrency)) = (budget.max_agents, budget.max_concurrency) {
        if concurrency > agents {
            bail!("{label} maxConcurrency may not exceed maxAgents");
        }
    }
    Ok(())
}

pub fn strictest_budget(budgets: &[&CampaignBudget]) -> CampaignBudget {
    fn min_u64(values: impl Iterator<Item = Option<u64>>) -> Option<u64> {
        values.flatten().min()
    }
    fn min_u32(values: impl Iterator<Item = Option<u32>>) -> Option<u32> {
        values.flatten().min()
    }
    fn min_f64(values: impl Iterator<Item = Option<f64>>) -> Option<f64> {
        values.flatten().reduce(f64::min)
    }
    CampaignBudget {
        max_wall_seconds: min_u64(budgets.iter().map(|budget| budget.max_wall_seconds)),
        max_model_calls: min_u64(budgets.iter().map(|budget| budget.max_model_calls)),
        max_input_tokens: min_u64(budgets.iter().map(|budget| budget.max_input_tokens)),
        max_output_tokens: min_u64(budgets.iter().map(|budget| budget.max_output_tokens)),
        max_cost_usd: min_f64(budgets.iter().map(|budget| budget.max_cost_usd)),
        max_tool_calls: min_u64(budgets.iter().map(|budget| budget.max_tool_calls)),
        max_agents: min_u32(budgets.iter().map(|budget| budget.max_agents)),
        max_concurrency: min_u32(budgets.iter().map(|budget| budget.max_concurrency)),
    }
}

pub fn reject_embedded_secrets(value: &Value, location: &str) -> Result<()> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let normalized = key.to_ascii_lowercase().replace(['_', '-'], "");
                if matches!(
                    normalized.as_str(),
                    "apikey"
                        | "apitoken"
                        | "accesstoken"
                        | "refreshtoken"
                        | "authorization"
                        | "cookie"
                        | "credentials"
                        | "clientsecret"
                        | "secretkey"
                        | "privatekey"
                        | "authprofiles"
                ) && match child {
                    Value::Null => false,
                    Value::String(value) => !value.is_empty(),
                    Value::Array(value) => !value.is_empty(),
                    Value::Object(value) => !value.is_empty(),
                    _ => true,
                } {
                    bail!("model campaign asset embeds a secret field at {location}.{key}");
                }
                reject_embedded_secrets(child, &format!("{location}.{key}"))?;
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                reject_embedded_secrets(child, &format!("{location}[{index}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn validate_model_profile(model: &ModelProfile) -> Result<()> {
    validate_identifier("model role", &model.role)?;
    validate_identifier("provider id", &model.provider_id)?;
    if model.model_id.trim().is_empty() {
        bail!("model profile {} has an empty modelId", model.role);
    }
    if model.temperature.is_some_and(|value| !value.is_finite()) {
        bail!("model profile {} temperature must be finite", model.role);
    }
    if model.max_output_tokens == Some(0) {
        bail!(
            "model profile {} maxOutputTokens must be positive",
            model.role
        );
    }
    Ok(())
}

pub fn validate_model_policy(policy: &ModelCampaignPolicy) -> Result<()> {
    if policy.schema_version != POLICY_SCHEMA_VERSION {
        bail!("model policy {} has unsupported schemaVersion", policy.id);
    }
    validate_identifier("model policy id", &policy.id)?;
    if policy.version.trim().is_empty() {
        bail!("model policy {} has an empty version", policy.id);
    }
    if policy.allowed_adapters.is_empty()
        || policy.allowed_runner_classes.is_empty()
        || policy.allowed_network_policies.is_empty()
        || policy.allowed_execution_modes.is_empty()
        || policy.allowed_sources.is_empty()
        || policy.models.is_empty()
        || policy.suites.is_empty()
    {
        bail!("model policy {} has an empty allowlist", policy.id);
    }
    if policy
        .allowed_adapters
        .iter()
        .any(|adapter| !ModelCampaignAdapter::REGISTERED_V1.contains(adapter))
    {
        bail!("model policy {} allows an unregistered adapter", policy.id);
    }
    if policy.performance_blocking {
        bail!("model campaign performance remains advisory in v1");
    }
    if !(1..=365).contains(&policy.artifact_retention_days) {
        bail!("artifactRetentionDays must be 1..=365");
    }
    if !policy.max_infra_error_rate.is_finite()
        || !(0.0..=1.0).contains(&policy.max_infra_error_rate)
    {
        bail!("maxInfraErrorRate must be 0..=1");
    }
    validate_campaign_budget(&policy.budget, "policy per-trial")?;
    validate_campaign_budget(&policy.campaign_budget, "policy campaign")?;
    let mut roles = BTreeSet::new();
    for model in &policy.models {
        validate_model_profile(model)?;
        if !roles.insert(model.role.as_str()) {
            bail!("model policy contains duplicate role {}", model.role);
        }
        if policy.require_model_snapshot && model.snapshot.as_deref().is_none_or(str::is_empty) {
            bail!("model policy requires a snapshot for role {}", model.role);
        }
    }
    let mut suites = BTreeSet::new();
    for suite in &policy.suites {
        validate_identifier("model policy suite id", &suite.id)?;
        if !suites.insert(suite.id.as_str()) {
            bail!("model policy contains duplicate suite {}", suite.id);
        }
        if suite.repetitions == Some(0) {
            bail!(
                "model policy suite {} repetitions must be positive",
                suite.id
            );
        }
    }
    if policy.tier == ModelCampaignTier::Release {
        if policy.allowed_sources.iter().any(|source| {
            matches!(
                source,
                ModelCampaignSource::LocalApp | ModelCampaignSource::LocalCli
            )
        }) {
            bail!("release model policy may not allow local evidence sources");
        }
        if policy
            .allowed_execution_modes
            .contains(&ExecutionMode::BridgedProvider)
        {
            bail!("release model policy may not allow bridged_provider in v1");
        }
    }
    Ok(())
}

pub fn validate_model_suite(suite: &ModelCampaignSuite, live_root: &Path) -> Result<()> {
    if suite.schema_version != SUITE_SCHEMA_VERSION {
        bail!("model suite {} has unsupported schemaVersion", suite.id);
    }
    validate_identifier("model suite id", &suite.id)?;
    validate_identifier("model capability", &suite.capability)?;
    if suite.version.trim().is_empty() || suite.tiers.is_empty() || suite.cases.is_empty() {
        bail!(
            "model suite {} must declare version, tiers, and cases",
            suite.id
        );
    }
    if !ModelCampaignAdapter::REGISTERED_V1.contains(&suite.adapter) {
        bail!("model suite {} uses an unregistered adapter", suite.id);
    }
    if !(1..=64).contains(&suite.shards) {
        bail!("model suite {} shards must be 1..=64", suite.id);
    }
    if !(1..=7_200).contains(&suite.timeout_seconds) {
        bail!("model suite {} timeoutSeconds must be 1..=7200", suite.id);
    }
    for repetitions in [
        suite.repetitions.nightly,
        suite.repetitions.weekly,
        suite.repetitions.release,
        suite.repetitions.monthly,
    ] {
        if !(1..=20).contains(&repetitions) {
            bail!("model suite repetitions must be 1..=20");
        }
    }
    validate_campaign_budget(&suite.budget, "suite")?;
    validate_identifier("hard verifier", &suite.scorer.hard_verifier)?;
    let mut ids = BTreeSet::new();
    for case in &suite.cases {
        validate_identifier("model case id", &case.id)?;
        if !ids.insert(case.id.as_str()) {
            bail!(
                "model suite {} contains duplicate case {}",
                suite.id,
                case.id
            );
        }
        if case
            .timeout_seconds
            .is_some_and(|value| !(1..=7_200).contains(&value))
        {
            bail!("model case {} timeoutSeconds must be 1..=7200", case.id);
        }
        if case
            .repetitions
            .is_some_and(|value| !(1..=20).contains(&value))
        {
            bail!("model case {} repetitions must be 1..=20", case.id);
        }
        resolve_contained(live_root, &case.scenario_path)?;
        for role in &case.model_roles {
            validate_identifier("model role", role)?;
        }
        if case.arms.is_empty() || case.arms.len() > 8 {
            bail!("model case {} arms must contain 1..=8 values", case.id);
        }
        let mut arms = BTreeSet::new();
        for arm in &case.arms {
            validate_identifier("model experiment arm", arm)?;
            if !arms.insert(arm.as_str()) {
                bail!("model case {} contains duplicate arm {arm}", case.id);
            }
        }
        if !arms
            .iter()
            .any(|arm| *arm == "control" || arm.ends_with("_control"))
        {
            bail!(
                "model case {} must declare at least one clean control arm",
                case.id
            );
        }
    }
    Ok(())
}

pub fn validate_scenario(scenario: &LiveAgentScenario, scenario_dir: &Path) -> Result<()> {
    if scenario.schema_version != SCENARIO_SCHEMA_VERSION {
        bail!("scenario {} has unsupported schemaVersion", scenario.id);
    }
    validate_identifier("scenario id", &scenario.id)?;
    if scenario.version.trim().is_empty()
        || scenario.title.trim().is_empty()
        || scenario.capabilities.is_empty()
        || scenario.verifiers.is_empty()
        || scenario.cadence.is_empty()
    {
        bail!(
            "scenario {} is missing identity or verifier fields",
            scenario.id
        );
    }
    validate_identifier("scenario config profile", &scenario.subject.config_profile)?;
    if scenario.environment.runner_class == RunnerClass::DesktopVm
        && scenario.subject.entrypoint != ScenarioEntrypoint::Desktop
    {
        bail!("desktop_vm scenario must use the desktop entrypoint");
    }
    if scenario.network.policy != NetworkPolicy::Allowlist && !scenario.network.allow.is_empty() {
        bail!("scenario network allow entries require allowlist policy");
    }
    if scenario.network.allow.iter().any(|host| {
        host.trim().is_empty()
            || host.contains('/')
            || host.contains(':')
            || host.parse::<std::net::IpAddr>().is_ok()
    }) {
        bail!("scenario network allow entries must be DNS hostnames without ports or paths");
    }
    validate_campaign_budget(&scenario.budgets, "scenario")?;
    let all_assets = scenario_asset_paths(scenario);
    let mut assets = BTreeSet::new();
    for path in all_assets {
        if !assets.insert(path.clone()) {
            continue;
        }
        resolve_contained(scenario_dir, &path)?;
    }
    let mut verifier_ids = BTreeSet::new();
    for verifier in &scenario.verifiers {
        validate_identifier("verifier id", &verifier.id)?;
        validate_identifier("verifier handler", &verifier.handler)?;
        if !verifier_ids.insert(verifier.id.as_str()) {
            bail!(
                "scenario {} contains duplicate verifier {}",
                scenario.id,
                verifier.id
            );
        }
        if !(1..=900).contains(&verifier.timeout_seconds) {
            bail!("verifier {} timeoutSeconds must be 1..=900", verifier.id);
        }
        if verifier.kind == VerifierKind::HumanOrLlmRubric && verifier.blocking {
            bail!("human_or_llm_rubric may not be blocking in v1");
        }
    }
    let mut milestones = BTreeSet::new();
    let mut total_weight = 0.0;
    for milestone in &scenario.milestones {
        validate_identifier("milestone id", &milestone.id)?;
        if !milestones.insert(milestone.id.as_str()) {
            bail!(
                "scenario {} contains duplicate milestone {}",
                scenario.id,
                milestone.id
            );
        }
        if !milestone.weight.is_finite() || !(0.0..=1.0).contains(&milestone.weight) {
            bail!("milestone {} weight must be 0..=1", milestone.id);
        }
        total_weight += milestone.weight;
        if let Some(verifier) = &milestone.verifier {
            if !verifier_ids.contains(verifier.as_str()) {
                bail!(
                    "milestone {} references unknown verifier {}",
                    milestone.id,
                    verifier
                );
            }
        }
    }
    if !scenario.milestones.is_empty() && (total_weight - 1.0).abs() > 1e-6 {
        bail!("scenario milestone weights must sum to 1.0");
    }
    for milestone in &scenario.milestones {
        for dependency in milestone.requires.iter().chain(&milestone.any_of) {
            if !milestones.contains(dependency.as_str()) {
                bail!(
                    "milestone {} references unknown dependency {}",
                    milestone.id,
                    dependency
                );
            }
        }
    }
    ensure_milestone_dag(&scenario.milestones)?;
    let mut invariant_ids = BTreeSet::new();
    for invariant in &scenario.invariants {
        validate_identifier("invariant id", &invariant.id)?;
        if !invariant_ids.insert(invariant.id.as_str()) {
            bail!("scenario contains duplicate invariant {}", invariant.id);
        }
        validate_invariant(invariant)?;
    }
    let mut fault_ids = BTreeSet::new();
    for fault in &scenario.faults {
        validate_identifier("fault id", &fault.id)?;
        validate_identifier("fault trigger", &fault.trigger)?;
        if fault.max_activations == 0 || !fault_ids.insert(fault.id.as_str()) {
            bail!("scenario fault IDs must be unique and maxActivations positive");
        }
        reject_unsafe_manifest_keys(&serde_json::to_value(&fault.params)?, "fault.params")?;
    }
    let mut artifact_paths = BTreeSet::new();
    for artifact in &scenario.artifacts {
        let path = Path::new(artifact);
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
            || !artifact_paths.insert(artifact.as_str())
        {
            bail!("scenario artifacts must be unique safe relative paths");
        }
    }
    match scenario.user_simulator.kind {
        UserSimulatorKind::ScriptedFsm | UserSimulatorKind::Replay
            if scenario.user_simulator.script_path.is_none() =>
        {
            bail!("scripted/replay user simulator requires scriptPath")
        }
        UserSimulatorKind::Llm if scenario.user_simulator.model_role.is_none() => {
            bail!("llm user simulator requires modelRole")
        }
        UserSimulatorKind::None
            if scenario.user_simulator.script_path.is_some()
                || scenario.user_simulator.model_role.is_some() =>
        {
            bail!("none user simulator may not declare scriptPath/modelRole")
        }
        _ => {}
    }
    if scenario.user_simulator.max_turns == 0 {
        bail!("user simulator maxTurns must be positive");
    }
    if let Some(comparison) = &scenario.comparison {
        let baseline = comparison
            .baseline
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("comparison requires baseline"))?;
        validate_identifier("comparison baseline", baseline)?;
        if comparison.ablations.len() < 2 || comparison.ablations.len() > 4 {
            bail!("comparison requires 2..=4 registered ablation profiles");
        }
        let mut profiles = BTreeSet::new();
        for profile in &comparison.ablations {
            validate_identifier("comparison ablation", profile)?;
            if !profiles.insert(profile.as_str()) {
                bail!("comparison contains duplicate ablation {profile}");
            }
        }
        if !profiles.contains(baseline) {
            bail!("comparison baseline must be present in ablations");
        }
        if comparison.budget_mode.as_deref().is_none_or(str::is_empty) {
            bail!("comparison requires budgetMode");
        }
    }
    Ok(())
}

fn validate_invariant(invariant: &InvariantSpec) -> Result<()> {
    match invariant.kind {
        InvariantKind::Never
        | InvariantKind::AtMostOnce
        | InvariantKind::ExactlyOnce
        | InvariantKind::Eventually => {
            if invariant.event.as_deref().is_none_or(str::is_empty) {
                bail!("invariant {} requires event", invariant.id);
            }
        }
        InvariantKind::Before | InvariantKind::After | InvariantKind::NoOverlap => {
            if invariant.first.as_deref().is_none_or(str::is_empty)
                || invariant.then.as_deref().is_none_or(str::is_empty)
            {
                bail!("invariant {} requires first and then", invariant.id);
            }
        }
        InvariantKind::EventuallyWithin => {
            if invariant.event.as_deref().is_none_or(str::is_empty)
                || invariant.within_ms.is_none_or(|value| value == 0)
            {
                bail!(
                    "invariant {} requires event and positive withinMs",
                    invariant.id
                );
            }
        }
        InvariantKind::MaxConcurrent => {
            if invariant.event.as_deref().is_none_or(str::is_empty)
                || invariant.maximum.is_none_or(|value| value == 0)
            {
                bail!(
                    "invariant {} requires event and positive maximum",
                    invariant.id
                );
            }
        }
        InvariantKind::ParentChildClosed => {}
    }
    Ok(())
}

fn ensure_milestone_dag(milestones: &[MilestoneSpec]) -> Result<()> {
    let graph = milestones
        .iter()
        .map(|milestone| {
            (
                milestone.id.as_str(),
                milestone
                    .requires
                    .iter()
                    .chain(&milestone.any_of)
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    fn visit<'a>(
        id: &'a str,
        graph: &BTreeMap<&'a str, Vec<&'a str>>,
        visiting: &mut BTreeSet<&'a str>,
        visited: &mut BTreeSet<&'a str>,
    ) -> Result<()> {
        if visited.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id) {
            bail!("milestone graph contains a cycle at {id}");
        }
        if let Some(dependencies) = graph.get(id) {
            for dependency in dependencies {
                visit(dependency, graph, visiting, visited)?;
            }
        }
        visiting.remove(id);
        visited.insert(id);
        Ok(())
    }
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for id in graph.keys() {
        visit(id, &graph, &mut visiting, &mut visited)?;
    }
    Ok(())
}

fn reject_unsafe_manifest_keys(value: &Value, location: &str) -> Result<()> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let normalized = key.to_ascii_lowercase().replace(['_', '-'], "");
                if matches!(
                    normalized.as_str(),
                    "shell" | "command" | "commands" | "argv" | "executable" | "program"
                ) {
                    bail!("model campaign manifest may not execute arbitrary code at {location}.{key}");
                }
                reject_unsafe_manifest_keys(child, &format!("{location}.{key}"))?;
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                reject_unsafe_manifest_keys(child, &format!("{location}[{index}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn scenario_asset_paths(scenario: &LiveAgentScenario) -> Vec<String> {
    let mut paths = scenario.environment.assets.clone();
    paths.push(scenario.task.prompt_path.clone());
    if let Some(path) = &scenario.task.hidden_truth_path {
        paths.push(path.clone());
    }
    if let Some(path) = &scenario.subject.tool_schema_path {
        paths.push(path.clone());
    }
    if let Some(path) = &scenario.user_simulator.script_path {
        paths.push(path.clone());
    }
    paths.extend(
        scenario
            .environment
            .services
            .iter()
            .filter_map(|service| service.fixture_path.clone()),
    );
    paths.extend(
        scenario
            .verifiers
            .iter()
            .filter_map(|verifier| verifier.config_path.clone()),
    );
    paths.sort();
    paths.dedup();
    paths
}

fn digest_asset_map<'a>(
    scenario_dir: &Path,
    paths: impl IntoIterator<Item = &'a str>,
) -> Result<BTreeMap<String, String>> {
    paths
        .into_iter()
        .map(|path| {
            let resolved = resolve_contained(scenario_dir, path)?;
            Ok((path.to_string(), digest_file(&resolved)?))
        })
        .collect()
}

pub fn scenario_digest(scenario: &LiveAgentScenario, scenario_dir: &Path) -> Result<String> {
    let mut value = serde_json::to_value(scenario)?;
    value
        .as_object_mut()
        .ok_or_else(|| anyhow!("scenario did not serialize as an object"))?
        .insert(
            "assetDigests".to_string(),
            serde_json::to_value(digest_asset_map(
                scenario_dir,
                scenario_asset_paths(scenario).iter().map(String::as_str),
            )?)?,
        );
    Ok(sha256_bytes(&canonical_json(&value)?))
}

pub fn scenario_component_digests(
    scenario: &LiveAgentScenario,
    scenario_dir: &Path,
) -> Result<(String, String, String, String)> {
    let environment_paths = scenario
        .environment
        .assets
        .iter()
        .map(String::as_str)
        .chain(
            scenario
                .environment
                .services
                .iter()
                .filter_map(|service| service.fixture_path.as_deref()),
        );
    let environment_value = serde_json::json!({
        "spec": scenario.environment,
        "assets": digest_asset_map(scenario_dir, environment_paths)?,
    });
    let verifier_paths = scenario
        .verifiers
        .iter()
        .filter_map(|verifier| verifier.config_path.as_deref());
    let verifier_value = serde_json::json!({
        "spec": scenario.verifiers,
        "assets": digest_asset_map(scenario_dir, verifier_paths)?,
    });
    let prompt_digest = digest_file(&resolve_contained(
        scenario_dir,
        &scenario.task.prompt_path,
    )?)?;
    let tool_schema_digest = match &scenario.subject.tool_schema_path {
        Some(path) => digest_file(&resolve_contained(scenario_dir, path)?)?,
        None => sha256_bytes(b"no-tool-schema-asset"),
    };
    Ok((
        sha256_bytes(&canonical_json(&environment_value)?),
        sha256_bytes(&canonical_json(&verifier_value)?),
        prompt_digest,
        tool_schema_digest,
    ))
}

pub fn model_case_digest(
    case: &ModelCampaignCaseSpec,
    scenario: &LiveAgentScenario,
    scenario_dir: &Path,
) -> Result<String> {
    let value = serde_json::json!({
        "case": case,
        "scenarioDigest": scenario_digest(scenario, scenario_dir)?,
    });
    Ok(sha256_bytes(&canonical_json(&value)?))
}

pub fn model_suite_digest(suite: &ModelCampaignSuite, live_root: &Path) -> Result<String> {
    let cases = suite
        .cases
        .iter()
        .map(|case| {
            let path = resolve_contained(live_root, &case.scenario_path)?;
            let value = read_json_or_yaml(&path)?;
            let scenario: LiveAgentScenario = serde_json::from_value(value)?;
            let scenario_dir = path
                .parent()
                .ok_or_else(|| anyhow!("scenario path has no parent"))?;
            validate_scenario(&scenario, scenario_dir)?;
            Ok((
                case.id.clone(),
                model_case_digest(case, &scenario, scenario_dir)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let value = serde_json::json!({"suite": suite, "caseDigests": cases});
    Ok(sha256_bytes(&canonical_json(&value)?))
}

pub fn stable_trial_id(
    reference: &str,
    suite_id: &str,
    case_id: &str,
    model_role: &str,
    arm: &str,
    trial_index: u16,
) -> String {
    let digest = sha256_bytes(
        format!("{reference}:{suite_id}:{case_id}:{model_role}:{arm}:{trial_index}").as_bytes(),
    );
    format!("mtrial_{}", &digest[..24])
}

pub fn stable_trial_seed(
    reference: &str,
    suite_id: &str,
    case_id: &str,
    model_role: &str,
    arm: &str,
    trial_index: u16,
) -> u64 {
    let digest = sha2::Sha256::digest(
        format!("seed:{reference}:{suite_id}:{case_id}:{model_role}:{arm}:{trial_index}")
            .as_bytes(),
    );
    u64::from_be_bytes(digest[..8].try_into().expect("SHA-256 has eight bytes"))
}

pub fn aggregate_counts(scheduled: usize, results: &[ModelTrialResult]) -> CampaignCounts {
    let mut counts = CampaignCounts {
        scheduled: scheduled as u64,
        ..Default::default()
    };
    for result in results {
        if !result.prior_attempts.is_empty() {
            counts.retried += 1;
            if result.outcome.is_valid_trial() {
                counts.recovered_after_retry += 1;
            }
        }
        if result.outcome.is_valid_trial() {
            counts.valid += 1;
        }
        match result.outcome {
            ModelCampaignOutcome::Passed => counts.passed += 1,
            ModelCampaignOutcome::TaskFailed => counts.failed += 1,
            ModelCampaignOutcome::PolicyFailed => {
                counts.failed += 1;
                counts.policy_failed += 1;
            }
            ModelCampaignOutcome::BudgetExhausted => {
                counts.failed += 1;
                counts.budget_exhausted += 1;
            }
            ModelCampaignOutcome::InfraError => counts.infra_error += 1,
            ModelCampaignOutcome::BenchmarkDefect => counts.benchmark_defect += 1,
            ModelCampaignOutcome::SimulatorError => counts.simulator_error += 1,
            ModelCampaignOutcome::Cancelled => counts.cancelled += 1,
        }
    }
    counts
}

pub fn aggregate_model_status(results: &[ModelTrialResult]) -> EvalStatus {
    if results.iter().any(|result| {
        matches!(
            result.outcome,
            ModelCampaignOutcome::TaskFailed
                | ModelCampaignOutcome::PolicyFailed
                | ModelCampaignOutcome::BudgetExhausted
        )
    }) {
        EvalStatus::Failed
    } else if results
        .iter()
        .any(|result| !matches!(result.outcome, ModelCampaignOutcome::Passed))
    {
        EvalStatus::InfraError
    } else {
        EvalStatus::Passed
    }
}

pub fn aggregate_metrics(
    planned: &[PlannedModelTrial],
    results: &[ModelTrialResult],
) -> CampaignMetricsSummary {
    let counts = aggregate_counts(planned.len(), results);
    let successful = results
        .iter()
        .filter(|result| result.outcome == ModelCampaignOutcome::Passed)
        .collect::<Vec<_>>();
    let wall = successful
        .iter()
        .map(|result| result.timings.wall_ms as f64)
        .collect::<Vec<_>>();
    let tokens = successful
        .iter()
        .filter_map(|result| Some((result.tokens.input? + result.tokens.output?) as f64))
        .collect::<Vec<_>>();
    let costs = successful
        .iter()
        .filter_map(|result| result.cost.total_usd)
        .collect::<Vec<_>>();
    let mut cases =
        BTreeMap::<(&str, &str, &str, &str), BTreeMap<u16, ModelCampaignOutcome>>::new();
    for result in results {
        cases
            .entry((
                &result.suite_id,
                &result.case_id,
                &result.model_role,
                &result.arm,
            ))
            .or_default()
            .insert(result.trial_index, result.outcome);
    }
    let planned_repetitions = planned.iter().fold(
        BTreeMap::<(&str, &str, &str, &str), u16>::new(),
        |mut grouped, trial| {
            grouped
                .entry((
                    &trial.suite_id,
                    &trial.case_id,
                    &trial.model_role,
                    &trial.arm,
                ))
                .and_modify(|count| *count = (*count).max(trial.trial_index + 1))
                .or_insert(trial.trial_index + 1);
            grouped
        },
    );
    let mut reliability_groups = BTreeMap::<u16, Vec<Vec<ModelCampaignOutcome>>>::new();
    for (key, repetitions) in planned_repetitions {
        let Some(outcomes) = cases.get(&key) else {
            continue;
        };
        if outcomes.len() != usize::from(repetitions)
            || (0..repetitions).any(|index| !outcomes.contains_key(&index))
        {
            continue;
        }
        reliability_groups
            .entry(repetitions)
            .or_default()
            .push(outcomes.values().copied().collect());
    }
    let reliability = reliability_groups
        .into_iter()
        .map(|(repetitions, groups)| {
            let total = groups.len() as u64;
            let any_pass_groups = groups
                .iter()
                .filter(|outcomes| {
                    outcomes
                        .iter()
                        .any(|outcome| *outcome == ModelCampaignOutcome::Passed)
                })
                .count() as u64;
            let all_pass_groups = groups
                .iter()
                .filter(|outcomes| {
                    outcomes
                        .iter()
                        .all(|outcome| *outcome == ModelCampaignOutcome::Passed)
                })
                .count() as u64;
            ReliabilitySummary {
                repetitions,
                groups: total,
                any_pass_groups,
                all_pass_groups,
                any_pass_rate: (total > 0).then(|| any_pass_groups as f64 / total as f64),
                all_pass_rate: (total > 0).then(|| all_pass_groups as f64 / total as f64),
            }
        })
        .collect();
    let success_interval = wilson_interval(counts.passed, counts.valid, 1.959_963_984_540_054);
    let comparisons = aggregate_orchestration_comparisons(planned, results);
    CampaignMetricsSummary {
        hard_success_rate: (counts.valid > 0).then(|| counts.passed as f64 / counts.valid as f64),
        hard_success_wilson_low: success_interval.map(|interval| interval.0),
        hard_success_wilson_high: success_interval.map(|interval| interval.1),
        end_to_end_yield: (counts.scheduled > 0)
            .then(|| counts.passed as f64 / counts.scheduled as f64),
        infra_error_rate: (counts.scheduled > 0)
            .then(|| counts.infra_error as f64 / counts.scheduled as f64),
        reliability,
        comparisons,
        successful_wall_ms: distribution(&wall),
        successful_tokens: distribution(&tokens),
        successful_cost_usd: distribution(&costs),
    }
}

fn aggregate_orchestration_comparisons(
    planned: &[PlannedModelTrial],
    results: &[ModelTrialResult],
) -> Vec<OrchestrationComparisonSummary> {
    type GroupKey = (String, String, String, FaultProfile);
    type TrialKey = (String, String, String, FaultProfile, String, u16);
    let mut profiles = BTreeMap::<GroupKey, BTreeSet<String>>::new();
    let mut planned_indices = BTreeMap::<(GroupKey, String), BTreeSet<u16>>::new();
    for trial in planned {
        let Some(profile) = trial.orchestration_profile.as_ref() else {
            continue;
        };
        let group = (
            trial.suite_id.clone(),
            trial.case_id.clone(),
            trial.model_role.clone(),
            trial.fault_profile,
        );
        profiles
            .entry(group.clone())
            .or_default()
            .insert(profile.clone());
        planned_indices
            .entry((group, profile.clone()))
            .or_default()
            .insert(trial.trial_index);
    }
    let result_map = results
        .iter()
        .filter_map(|result| {
            Some((
                (
                    result.suite_id.clone(),
                    result.case_id.clone(),
                    result.model_role.clone(),
                    result.fault_profile,
                    result.orchestration_profile.clone()?,
                    result.trial_index,
                ),
                result,
            ))
        })
        .collect::<BTreeMap<TrialKey, _>>();
    let mut summaries = Vec::new();
    for (group, group_profiles) in profiles {
        let Some(baseline) = group_profiles
            .iter()
            .find(|profile| profile.starts_with("single_agent"))
            .cloned()
        else {
            continue;
        };
        let Some(candidate) = group_profiles
            .iter()
            .find(|profile| profile.as_str() == "team_full")
            .cloned()
        else {
            continue;
        };
        let Some(baseline_indices) = planned_indices.get(&(group.clone(), baseline.clone())) else {
            continue;
        };
        let Some(candidate_indices) = planned_indices.get(&(group.clone(), candidate.clone()))
        else {
            continue;
        };
        let mut paired_valid = 0u64;
        let mut baseline_passed = 0u64;
        let mut candidate_passed = 0u64;
        let mut candidate_only = 0u64;
        let mut baseline_only = 0u64;
        let mut wall_speedup = Vec::new();
        let mut token_amplification = Vec::new();
        let mut cost_amplification = Vec::new();
        let mut parallel_efficiency = Vec::new();
        let mut coordination_overhead = Vec::new();
        for trial_index in baseline_indices.intersection(candidate_indices) {
            let baseline_key = (
                group.0.clone(),
                group.1.clone(),
                group.2.clone(),
                group.3,
                baseline.clone(),
                *trial_index,
            );
            let candidate_key = (
                group.0.clone(),
                group.1.clone(),
                group.2.clone(),
                group.3,
                candidate.clone(),
                *trial_index,
            );
            let (Some(baseline_result), Some(candidate_result)) = (
                result_map.get(&baseline_key),
                result_map.get(&candidate_key),
            ) else {
                continue;
            };
            if !baseline_result.outcome.is_valid_trial()
                || !candidate_result.outcome.is_valid_trial()
            {
                continue;
            }
            paired_valid = paired_valid.saturating_add(1);
            let baseline_ok = baseline_result.outcome == ModelCampaignOutcome::Passed;
            let candidate_ok = candidate_result.outcome == ModelCampaignOutcome::Passed;
            baseline_passed = baseline_passed.saturating_add(u64::from(baseline_ok));
            candidate_passed = candidate_passed.saturating_add(u64::from(candidate_ok));
            candidate_only = candidate_only.saturating_add(u64::from(candidate_ok && !baseline_ok));
            baseline_only = baseline_only.saturating_add(u64::from(baseline_ok && !candidate_ok));
            if !(baseline_ok && candidate_ok) {
                continue;
            }
            if candidate_result.timings.wall_ms > 0 {
                wall_speedup.push(
                    baseline_result.timings.wall_ms as f64
                        / candidate_result.timings.wall_ms as f64,
                );
            }
            let baseline_tokens = baseline_result
                .tokens
                .input
                .zip(baseline_result.tokens.output)
                .map(|(input, output)| input.saturating_add(output));
            let candidate_tokens = candidate_result
                .tokens
                .input
                .zip(candidate_result.tokens.output)
                .map(|(input, output)| input.saturating_add(output));
            if let (Some(baseline_tokens), Some(candidate_tokens)) =
                (baseline_tokens.filter(|value| *value > 0), candidate_tokens)
            {
                token_amplification.push(candidate_tokens as f64 / baseline_tokens as f64);
            }
            if let (Some(baseline_cost), Some(candidate_cost)) = (
                baseline_result.cost.total_usd.filter(|cost| *cost > 0.0),
                candidate_result.cost.total_usd,
            ) {
                cost_amplification.push(candidate_cost / baseline_cost);
            }
            if candidate_result.timings.wall_ms > 0
                && candidate_result.orchestration.max_concurrency > 0
            {
                parallel_efficiency.push(
                    candidate_result.orchestration.child_active_ms as f64
                        / (candidate_result.timings.wall_ms as f64
                            * f64::from(candidate_result.orchestration.max_concurrency)),
                );
            }
            if let (Some(coordination), Some(total_tokens)) = (
                candidate_result.orchestration.coordination_tokens,
                candidate_tokens.filter(|value| *value > 0),
            ) {
                coordination_overhead.push(coordination as f64 / total_tokens as f64);
            }
        }
        let baseline_rate =
            (paired_valid > 0).then(|| baseline_passed as f64 / paired_valid as f64);
        let candidate_rate =
            (paired_valid > 0).then(|| candidate_passed as f64 / paired_valid as f64);
        summaries.push(OrchestrationComparisonSummary {
            suite_id: group.0,
            case_id: group.1,
            model_role: group.2,
            fault_profile: group.3,
            baseline_profile: baseline,
            candidate_profile: candidate,
            paired_valid_trials: paired_valid,
            baseline_passed,
            candidate_passed,
            candidate_only_passed: candidate_only,
            baseline_only_passed: baseline_only,
            baseline_success_rate: baseline_rate,
            candidate_success_rate: candidate_rate,
            candidate_uplift_pp: baseline_rate
                .zip(candidate_rate)
                .map(|(baseline, candidate)| (candidate - baseline) * 100.0),
            successful_wall_speedup: distribution(&wall_speedup),
            successful_token_amplification: distribution(&token_amplification),
            successful_cost_amplification: distribution(&cost_amplification),
            successful_parallel_efficiency: distribution(&parallel_efficiency),
            successful_coordination_overhead: distribution(&coordination_overhead),
        });
    }
    summaries
}

fn wilson_interval(successes: u64, total: u64, z: f64) -> Option<(f64, f64)> {
    if total == 0 || !z.is_finite() || z <= 0.0 {
        return None;
    }
    let n = total as f64;
    let p = successes.min(total) as f64 / n;
    let z2 = z * z;
    let denominator = 1.0 + z2 / n;
    let center = (p + z2 / (2.0 * n)) / denominator;
    let margin = z * ((p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt()) / denominator;
    Some(((center - margin).max(0.0), (center + margin).min(1.0)))
}

fn distribution(values: &[f64]) -> DistributionSummary {
    DistributionSummary {
        p50: percentile(values, 0.50),
        p95: percentile(values, 0.95),
    }
}

fn percentile(values: &[f64], quantile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f64::total_cmp);
    let rank = ((sorted.len() - 1) as f64 * quantile).ceil() as usize;
    sorted.get(rank).copied()
}

pub fn model_runner_digest(reference: &str) -> String {
    sha256_bytes(format!("hope-agent-eval:model:v1:{reference}").as_bytes())
}

pub fn validate_evidence_shape(evidence: &ModelCampaignEvidence) -> Result<()> {
    if evidence.schema_version != EVIDENCE_SCHEMA_VERSION
        || evidence.evidence_kind != "model_campaign"
    {
        bail!("unsupported model campaign evidence schema/kind");
    }
    if evidence.runner_os.trim().is_empty() || evidence.runner_arch.trim().is_empty() {
        bail!("model evidence must record the runner OS and architecture");
    }
    if evidence.source == ModelCampaignSource::LocalApp {
        if evidence.execution_profile
            != Some(crate::app::AppExecutionProfile::LocalNativeDiagnostic)
            || evidence
                .app_plan_digest
                .as_deref()
                .is_none_or(|digest| !is_sha256(digest))
            || evidence.runtime_environment.is_none()
        {
            bail!("local_app evidence must record its app plan and runtime environment");
        }
    }
    if let Some(runtime) = &evidence.runtime_environment {
        crate::app::validate_runtime_environment(runtime)?;
        if runtime.os != evidence.runner_os || runtime.arch != evidence.runner_arch {
            bail!("model evidence runner and runtime environment identity differ");
        }
    }
    if evidence.counts != aggregate_counts(evidence.planned_trials.len(), &evidence.trial_results) {
        bail!("model evidence counts cannot be recomputed from trial results");
    }
    let expected_status = if evidence.campaign_budget_violations.is_empty() {
        aggregate_model_status(&evidence.trial_results)
    } else {
        EvalStatus::Failed
    };
    if evidence.aggregate_status != expected_status {
        bail!("model evidence aggregate status is inconsistent");
    }
    if evidence.metrics != aggregate_metrics(&evidence.planned_trials, &evidence.trial_results) {
        bail!("model evidence metrics cannot be recomputed from trial results");
    }
    let expected = evidence
        .planned_trials
        .iter()
        .map(|trial| trial.id.as_str())
        .collect::<BTreeSet<_>>();
    let actual = evidence
        .trial_results
        .iter()
        .map(|trial| trial.trial_id.as_str())
        .collect::<BTreeSet<_>>();
    if expected.len() != evidence.planned_trials.len()
        || actual.len() != evidence.trial_results.len()
        || expected != actual
    {
        bail!("model evidence must contain every planned trial exactly once");
    }
    for result in &evidence.trial_results {
        if result.schema_version != TRIAL_SCHEMA_VERSION {
            bail!(
                "model trial {} has unsupported schemaVersion",
                result.trial_id
            );
        }
        if result.trace.orphan_span_count > 0 || !result.trace.closed {
            bail!(
                "model trial {} has an open or orphaned trace",
                result.trial_id
            );
        }
        if result
            .runtime_config_digest
            .as_deref()
            .is_some_and(|digest| !is_sha256(digest))
        {
            bail!(
                "model trial {} has an invalid runtime config digest",
                result.trial_id
            );
        }
        validate_trial_metric_invariants(
            &result.timings,
            &result.tokens,
            &result.tools,
            &result.orchestration,
            &format!("model trial {}", result.trial_id),
        )?;
        validate_model_trace(
            &result.trace,
            &result.trace_events,
            &format!("model trial {}", result.trial_id),
        )?;
        for attempt in &result.prior_attempts {
            if attempt
                .runtime_config_digest
                .as_deref()
                .is_some_and(|digest| !is_sha256(digest))
            {
                bail!(
                    "model trial {} has an invalid prior runtime config digest",
                    result.trial_id
                );
            }
            validate_trial_metric_invariants(
                &attempt.timings,
                &attempt.tokens,
                &attempt.tools,
                &attempt.orchestration,
                &format!(
                    "model trial {} prior attempt {}",
                    result.trial_id, attempt.attempt
                ),
            )?;
            validate_model_trace(
                &attempt.trace,
                &attempt.trace_events,
                &format!(
                    "model trial {} prior attempt {}",
                    result.trial_id, attempt.attempt
                ),
            )?;
        }
    }
    Ok(())
}

fn validate_trial_metric_invariants(
    timings: &TimingMetrics,
    tokens: &TokenMetrics,
    tools: &ToolMetrics,
    orchestration: &OrchestrationMetrics,
    label: &str,
) -> Result<()> {
    if timings.critical_path_ms > timings.wall_ms {
        bail!("{label} critical path exceeds wall time");
    }
    if tools.succeeded + tools.failed + tools.cancelled != tools.attempted
        || tools.logical_calls > tools.attempted
        || tools.retries != tools.attempted.saturating_sub(tools.logical_calls)
        || tools.parse_errors > tools.failed
        || tools.invalid > tools.failed
        || tools.duplicate > tools.attempted
        || tools.unused_results > tools.succeeded
        || tools.effective > tools.succeeded
        || tools.effective.saturating_add(tools.unused_results) > tools.succeeded
    {
        bail!("{label} tool metrics are inconsistent");
    }
    if orchestration.failovers > orchestration.model_retries {
        bail!("{label} orchestration metrics are inconsistent");
    }
    if let Some(coordination_tokens) = orchestration.coordination_tokens {
        let total_tokens = tokens
            .input
            .unwrap_or(0)
            .saturating_add(tokens.output.unwrap_or(0))
            .saturating_add(tokens.reasoning.unwrap_or(0));
        if coordination_tokens > total_tokens {
            bail!("{label} coordination tokens exceed total attributed tokens");
        }
    }
    Ok(())
}

pub fn digest_model_profile(profile: &ModelProfile) -> Result<String> {
    digest_serializable(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricted_yaml_rejects_anchors_aliases_and_tags() {
        for value in ["a: &x 1\nb: *x\n", "a: !secret value\n", "<<: {a: 1}\n"] {
            assert!(reject_yaml_extensions(value).is_err());
        }
        assert!(reject_yaml_extensions("a: \"literal * and ! and &\"\n").is_ok());
    }

    #[test]
    fn outcome_mapping_keeps_invalid_trials_out_of_valid_denominator() {
        assert_eq!(
            ModelCampaignOutcome::BudgetExhausted.aggregate_status(),
            Some(EvalStatus::Failed)
        );
        assert!(!ModelCampaignOutcome::SimulatorError.is_valid_trial());
        assert_eq!(ModelCampaignOutcome::Cancelled.aggregate_status(), None);
    }

    #[test]
    fn stable_trials_are_repeatable_and_distinct() {
        let a = stable_trial_id("a", "suite", "case", "anchor", "control", 0);
        assert_eq!(
            a,
            stable_trial_id("a", "suite", "case", "anchor", "control", 0)
        );
        assert_ne!(
            a,
            stable_trial_id("a", "suite", "case", "anchor", "faulted", 0)
        );
        assert_ne!(
            a,
            stable_trial_id("a", "suite", "case", "anchor", "control", 1)
        );
        assert_ne!(
            stable_trial_seed("a", "suite", "case", "anchor", "control", 0),
            stable_trial_seed("a", "suite", "case", "anchor", "faulted", 0)
        );
    }

    #[test]
    fn percentile_uses_nearest_rank_upper_bound() {
        assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0], 0.5), Some(3.0));
        assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0], 0.95), Some(4.0));
    }

    #[test]
    fn causal_trace_rejects_orphans_and_unpaired_reuse() {
        let trace = TraceSummary {
            trace_id: "trace_test".to_string(),
            root_span_id: "span_root".to_string(),
            span_count: 2,
            orphan_span_count: 0,
            closed: true,
        };
        let orphan = vec![ModelCampaignEvent {
            seq: 1,
            event: "tool.read".to_string(),
            timestamp_ms: 1,
            span_id: "span_tool".to_string(),
            parent_span_id: "span_missing".to_string(),
            key: Some("call-1".to_string()),
            status: "succeeded".to_string(),
            duration_ms: 1,
            attributes: BTreeMap::new(),
        }];
        assert!(validate_model_trace(&trace, &orphan, "orphan").is_err());

        let mismatched = vec![
            ModelCampaignEvent {
                seq: 1,
                event: "session.turn".to_string(),
                timestamp_ms: 1,
                span_id: "span_turn".to_string(),
                parent_span_id: "span_root".to_string(),
                key: Some("session".to_string()),
                status: "started".to_string(),
                duration_ms: 0,
                attributes: BTreeMap::new(),
            },
            ModelCampaignEvent {
                seq: 2,
                event: "session.turn".to_string(),
                timestamp_ms: 2,
                span_id: "span_turn".to_string(),
                parent_span_id: "span_root".to_string(),
                key: Some("session".to_string()),
                status: "observed".to_string(),
                duration_ms: 1,
                attributes: BTreeMap::new(),
            },
        ];
        assert!(validate_model_trace(&trace, &mismatched, "mismatch").is_err());
        assert!(validate_model_trace(&trace, &mismatched[..1], "open lifecycle").is_err());
    }

    #[test]
    fn causal_critical_path_unions_root_work_without_double_counting_children() {
        let trace = TraceSummary {
            trace_id: "trace_critical".to_string(),
            root_span_id: "span_root".to_string(),
            span_count: 5,
            orphan_span_count: 0,
            closed: true,
        };
        let events = vec![
            ModelCampaignEvent {
                seq: 1,
                event: "session.turn".to_string(),
                timestamp_ms: 10,
                span_id: "span_turn_one".to_string(),
                parent_span_id: "span_root".to_string(),
                key: Some("session".to_string()),
                status: "started".to_string(),
                duration_ms: 0,
                attributes: BTreeMap::new(),
            },
            ModelCampaignEvent {
                seq: 2,
                event: "model.call".to_string(),
                timestamp_ms: 70,
                span_id: "span_model".to_string(),
                parent_span_id: "span_turn_one".to_string(),
                key: Some("round-1".to_string()),
                status: "succeeded".to_string(),
                duration_ms: 40,
                attributes: BTreeMap::new(),
            },
            ModelCampaignEvent {
                seq: 3,
                event: "session.turn".to_string(),
                timestamp_ms: 100,
                span_id: "span_turn_one".to_string(),
                parent_span_id: "span_root".to_string(),
                key: Some("session".to_string()),
                status: "completed".to_string(),
                duration_ms: 90,
                attributes: BTreeMap::new(),
            },
            ModelCampaignEvent {
                seq: 4,
                event: "session.turn".to_string(),
                timestamp_ms: 125,
                span_id: "span_turn_two".to_string(),
                parent_span_id: "span_root".to_string(),
                key: Some("session".to_string()),
                status: "started".to_string(),
                duration_ms: 0,
                attributes: BTreeMap::new(),
            },
            ModelCampaignEvent {
                seq: 5,
                event: "process.restart".to_string(),
                timestamp_ms: 130,
                span_id: "span_restart".to_string(),
                parent_span_id: "span_root".to_string(),
                key: Some("server".to_string()),
                status: "completed".to_string(),
                duration_ms: 20,
                attributes: BTreeMap::new(),
            },
            ModelCampaignEvent {
                seq: 6,
                event: "session.turn".to_string(),
                timestamp_ms: 175,
                span_id: "span_turn_two".to_string(),
                parent_span_id: "span_root".to_string(),
                key: Some("session".to_string()),
                status: "completed".to_string(),
                duration_ms: 50,
                attributes: BTreeMap::new(),
            },
        ];
        validate_model_trace(&trace, &events, "critical path").unwrap();
        // [10,100] + union([110,130], [125,175]) = 90 + 65.
        assert_eq!(causal_critical_path_ms(&trace, &events), 155);
    }

    #[test]
    fn causal_trace_attributes_accept_only_redacted_scalars_and_sha256_digests() {
        let trace = TraceSummary {
            trace_id: "trace_attributes".to_string(),
            root_span_id: "span_root".to_string(),
            span_count: 2,
            orphan_span_count: 0,
            closed: true,
        };
        let mut attributes = BTreeMap::from([
            ("toolName".to_string(), Value::String("read".to_string())),
            ("inputTokens".to_string(), Value::from(12)),
            ("success".to_string(), Value::Bool(true)),
            ("argumentsDigest".to_string(), Value::String("a".repeat(64))),
        ]);
        let event = |attributes| {
            vec![ModelCampaignEvent {
                seq: 1,
                event: "tool.read".to_string(),
                timestamp_ms: 1,
                span_id: "span_tool".to_string(),
                parent_span_id: "span_root".to_string(),
                key: Some("call-1".to_string()),
                status: "succeeded".to_string(),
                duration_ms: 1,
                attributes,
            }]
        };

        validate_model_trace(&trace, &event(attributes.clone()), "safe attributes").unwrap();
        attributes.insert("nested".to_string(), serde_json::json!({"raw": "secret"}));
        assert!(validate_model_trace(&trace, &event(attributes.clone()), "nested").is_err());
        attributes.remove("nested");
        attributes.insert(
            "providerName".to_string(),
            Value::String("unsafe\nvalue".to_string()),
        );
        assert!(validate_model_trace(&trace, &event(attributes.clone()), "newline").is_err());
        attributes.remove("providerName");
        attributes.insert(
            "argumentsDigest".to_string(),
            Value::String("not-a-digest".to_string()),
        );
        assert!(validate_model_trace(&trace, &event(attributes), "digest").is_err());
    }

    #[test]
    fn metric_invariants_reject_impossible_critical_path_and_tool_counts() {
        let mut timings = TimingMetrics {
            wall_ms: 10,
            critical_path_ms: 11,
            ..Default::default()
        };
        assert!(validate_trial_metric_invariants(
            &timings,
            &TokenMetrics::default(),
            &ToolMetrics::default(),
            &OrchestrationMetrics::default(),
            "trial",
        )
        .is_err());

        timings.critical_path_ms = 10;
        let tools = ToolMetrics {
            attempted: 1,
            logical_calls: 1,
            succeeded: 1,
            effective: 1,
            unused_results: 1,
            ..Default::default()
        };
        assert!(validate_trial_metric_invariants(
            &timings,
            &TokenMetrics::default(),
            &tools,
            &OrchestrationMetrics::default(),
            "trial",
        )
        .is_err());
    }
}
