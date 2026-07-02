//! Deterministic coding control-plane eval harness.
//!
//! Fixtures create temporary git repositories, seed real session / goal / task /
//! workflow state, then drive production Context Retrieval, Review, Smart
//! Verification, optional Agent execution, and task-level eval scoring APIs.
//! Project validation commands only run when a fixture explicitly opts into
//! workflow validation.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent_loader::DEFAULT_AGENT_ID;
use crate::chat_engine::{self, ChatEngineParams, ChatSource, NoopEventSink};
use crate::coding_improvement::{
    ApplyCodingImprovementProposalResult, CodingTrendReport,
    GenerateCodingImprovementProposalsResult, PromoteCodingImprovementProposalResult,
    RecordCodingEvalPackRunInput, RecordCodingEvalRunInput, RecordCodingStrategyEffectRunInput,
};
use crate::context_compact::CompactConfig;
use crate::context_retrieval::{self, ContextCandidate, ContextCandidateKind};
use crate::goal::CreateGoalInput;
use crate::provider::{ActiveModel, ProviderConfig};
use crate::review::{self, RunReviewInput};
use crate::session::{MessageRole, NewMessage, SessionDB, SessionIdeContext, TaskStatus};
use crate::verification::{self, PlanVerificationInput};
use crate::workflow::{
    self, CreateWorkflowRunInput, UpsertWorkflowOpInput, WorkflowEffectClass, WorkflowRunState,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingEvalFixture {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub task: Option<CodingTaskEvalSpec>,
    pub repo: RepoFixture,
    #[serde(default)]
    pub setup: FixtureSetup,
    #[serde(default)]
    pub runs: FixtureRuns,
    #[serde(default)]
    pub checks: FixtureChecks,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldTaskPackRunInput {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub statuses: Vec<String>,
    #[serde(default)]
    pub task_types: Vec<String>,
    #[serde(default)]
    pub include_unautomated: bool,
    #[serde(default)]
    pub max_tasks: Option<usize>,
    #[serde(default = "default_true")]
    pub record_eval_runs: bool,
    #[serde(default = "default_true")]
    pub record_pack_run: bool,
    #[serde(default = "default_true")]
    pub evaluate_goal: bool,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub baseline_kind: Option<String>,
    #[serde(default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
}

impl Default for GoldTaskPackRunInput {
    fn default() -> Self {
        Self {
            session_id: None,
            project_id: None,
            ids: Vec::new(),
            statuses: Vec::new(),
            task_types: Vec::new(),
            include_unautomated: false,
            max_tasks: None,
            record_eval_runs: true,
            record_pack_run: true,
            evaluate_goal: true,
            label: None,
            baseline_kind: None,
            source_type: None,
            source_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldTaskPackSummary {
    pub pack_id: String,
    pub source_doc: String,
    pub total_cases: usize,
    pub automated_cases: usize,
    pub active_cases: usize,
    pub cases: Vec<GoldTaskCaseSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldTaskCaseSummary {
    pub id: String,
    pub task_type: String,
    pub title: String,
    pub status: String,
    pub source: String,
    pub execution_mode: String,
    pub automation_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixture_name: Option<String>,
    pub expected_artifacts: Vec<String>,
    pub requires_seeded_state: bool,
    pub likely_files: Vec<String>,
    pub allowed_validation: Vec<String>,
    pub success_criteria: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldTaskPackReport {
    pub pack_id: String,
    pub source_doc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack_run_id: Option<String>,
    pub selected_cases: usize,
    pub automated_cases: usize,
    pub skipped_cases: usize,
    pub passed_cases: usize,
    pub failed_cases: usize,
    pub total_checks: usize,
    pub passed: bool,
    pub cases: Vec<GoldTaskCaseRunReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldTaskCaseRunReport {
    pub case: GoldTaskCaseSummary,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixture_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<FixtureReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyEffectEvalInput {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub baseline_pack_run_id: Option<String>,
    #[serde(default)]
    pub candidate_pack_run_id: Option<String>,
    #[serde(default)]
    pub record_run: bool,
    #[serde(default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub strategy_type: Option<String>,
    #[serde(default)]
    pub baseline_label: Option<String>,
    #[serde(default)]
    pub candidate_label: Option<String>,
    pub baseline: GoldTaskPackReport,
    pub candidate: GoldTaskPackReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyEffectReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub strategy_type: String,
    pub baseline_label: String,
    pub candidate_label: String,
    pub verdict: String,
    pub compared_cases: usize,
    pub baseline_only_cases: Vec<String>,
    pub candidate_only_cases: Vec<String>,
    pub summary: StrategyEffectSummary,
    pub dimensions: Vec<StrategyEffectDimension>,
    pub cases: Vec<StrategyCaseComparison>,
    pub regressions: Vec<String>,
    pub improvements: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyEffectSummary {
    pub baseline_pass_rate: f64,
    pub candidate_pass_rate: f64,
    pub pass_rate_delta: f64,
    pub baseline_average_score: f64,
    pub candidate_average_score: f64,
    pub average_score_delta: f64,
    pub baseline_context_recall: f64,
    pub candidate_context_recall: f64,
    pub context_recall_delta: f64,
    pub baseline_validation_violations: usize,
    pub candidate_validation_violations: usize,
    pub validation_violation_delta: isize,
    pub baseline_scope_creep: usize,
    pub candidate_scope_creep: usize,
    pub scope_creep_delta: isize,
    pub baseline_execution_failures: usize,
    pub candidate_execution_failures: usize,
    pub execution_failure_delta: isize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyEffectDimension {
    pub name: String,
    pub direction: String,
    pub baseline: f64,
    pub candidate: f64,
    pub delta: f64,
    pub verdict: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyCaseComparison {
    pub id: String,
    pub title: String,
    pub verdict: String,
    pub baseline_status: String,
    pub candidate_status: String,
    pub baseline_passed: bool,
    pub candidate_passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_outcome: Option<String>,
    pub baseline_score: f64,
    pub candidate_score: f64,
    pub score_delta: f64,
    pub baseline_context_recall: f64,
    pub candidate_context_recall: f64,
    pub context_recall_delta: f64,
    pub baseline_validation_violations: usize,
    pub candidate_validation_violations: usize,
    pub baseline_scope_creep: usize,
    pub candidate_scope_creep: usize,
    pub baseline_execution_failed: bool,
    pub candidate_execution_failed: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoFixture {
    #[serde(default)]
    pub files: Vec<FileFixture>,
    #[serde(default)]
    pub changes: Vec<FileFixture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileFixture {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingTaskEvalSpec {
    pub id: String,
    #[serde(default)]
    pub task_type: String,
    pub title: String,
    #[serde(default)]
    pub source: String,
    pub prompt: String,
    #[serde(default)]
    pub execution_mode: String,
    #[serde(default)]
    pub expected_behavior: Vec<String>,
    #[serde(default)]
    pub forbidden_behavior: Vec<String>,
    #[serde(default)]
    pub likely_files: Vec<String>,
    #[serde(default)]
    pub expected_artifacts: Vec<String>,
    #[serde(default)]
    pub requires_seeded_state: bool,
    #[serde(default)]
    pub allowed_validation: Vec<String>,
    #[serde(default)]
    pub success_criteria: Vec<String>,
    #[serde(default)]
    pub failure_notes: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixtureSetup {
    #[serde(default)]
    pub goal: Option<GoalFixture>,
    #[serde(default)]
    pub tasks: Vec<TaskFixture>,
    #[serde(default)]
    pub workflow: Option<WorkflowFixture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalFixture {
    pub objective: String,
    #[serde(default)]
    pub completion_criteria: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskFixture {
    pub content: String,
    #[serde(default)]
    pub active_form: Option<String>,
    #[serde(default = "default_pending_status")]
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowFixture {
    #[serde(default = "default_workflow_kind")]
    pub kind: String,
    #[serde(default = "default_execution_mode")]
    pub execution_mode: String,
    #[serde(default = "default_workflow_script")]
    pub script_source: String,
    #[serde(default)]
    pub ops: Vec<WorkflowOpFixture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowOpFixture {
    pub op_key: String,
    pub op_type: String,
    #[serde(default = "default_effect_class")]
    pub effect_class: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub output: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixtureRuns {
    #[serde(default)]
    pub execution: Option<AgentExecutionEvalRun>,
    #[serde(default)]
    pub task: Option<TaskLevelEvalRun>,
    #[serde(default)]
    pub workflow: Option<WorkflowScriptEvalRun>,
    #[serde(default)]
    pub review: Option<ReviewEvalRun>,
    #[serde(default)]
    pub verification: Option<VerificationEvalRun>,
    #[serde(default)]
    pub context: Option<ContextEvalRun>,
    #[serde(default)]
    pub improvement: Option<ImprovementEvalRun>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowScriptEvalRun {
    pub script_source: String,
    #[serde(default = "default_workflow_kind")]
    pub kind: String,
    #[serde(default = "default_execution_mode")]
    pub execution_mode: String,
    #[serde(default)]
    pub budget: Value,
    #[serde(default)]
    pub allow_terminal_error: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewEvalRun {
    #[serde(default)]
    pub focus_paths: Vec<String>,
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub ide_context: Option<SessionIdeContext>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationEvalRun {
    #[serde(default)]
    pub focus_paths: Vec<String>,
    #[serde(default)]
    pub max_commands: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextEvalRun {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub ide_context: Option<SessionIdeContext>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImprovementEvalRun {
    #[serde(default)]
    pub window_days: Option<u32>,
    #[serde(default)]
    pub generate_proposals: bool,
    #[serde(default)]
    pub apply_first_proposal: bool,
    #[serde(default)]
    pub promote_applied_proposal: bool,
    #[serde(default)]
    pub apply_proposal_kind: Option<String>,
    #[serde(default)]
    pub seed_eval_runs: Vec<RecordCodingEvalRunInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecutionEvalRun {
    #[serde(default = "default_agent_execution_mode")]
    pub mode: String,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub display_text: Option<String>,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub model_chain: Vec<ActiveModel>,
    #[serde(default)]
    pub compact_config: Option<CompactConfig>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub extra_system_context: Option<String>,
    #[serde(default)]
    pub denied_tools: Vec<String>,
    #[serde(default)]
    pub auto_approve_tools: bool,
}

impl Default for AgentExecutionEvalRun {
    fn default() -> Self {
        Self {
            mode: default_agent_execution_mode(),
            prompt: None,
            agent_id: None,
            display_text: None,
            providers: Vec::new(),
            model_chain: Vec::new(),
            compact_config: None,
            reasoning_effort: None,
            extra_system_context: None,
            denied_tools: Vec::new(),
            auto_approve_tools: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskLevelEvalRun {
    #[serde(default = "default_true")]
    pub record_eval_run: bool,
    #[serde(default = "default_true")]
    pub evaluate_goal: bool,
}

impl Default for TaskLevelEvalRun {
    fn default() -> Self {
        Self {
            record_eval_run: true,
            evaluate_goal: true,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixtureChecks {
    #[serde(default)]
    pub execution: Option<AgentExecutionCheck>,
    #[serde(default)]
    pub task: Option<TaskLevelCheck>,
    #[serde(default)]
    pub workflow: Option<WorkflowCheck>,
    #[serde(default)]
    pub context: Option<ContextCheck>,
    #[serde(default)]
    pub review: Option<ReviewCheck>,
    #[serde(default)]
    pub verification: Option<VerificationCheck>,
    #[serde(default)]
    pub improvement: Option<ImprovementCheck>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowCheck {
    #[serde(default)]
    pub expected_state: Option<String>,
    #[serde(default)]
    pub expected_blocked_reason: Option<String>,
    #[serde(default)]
    pub expected_op_types: Vec<String>,
    #[serde(default)]
    pub expected_commands: Vec<String>,
    #[serde(default)]
    pub min_finding_count: Option<usize>,
    #[serde(default)]
    pub expect_review_ok: Option<bool>,
    #[serde(default)]
    pub expected_goal_relations: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextCheck {
    #[serde(default)]
    pub critical: Vec<CandidateExpectation>,
    #[serde(default)]
    pub min_critical_recall: Option<f64>,
    #[serde(default)]
    pub min_precision: Option<f64>,
    #[serde(default)]
    pub max_candidates: Option<usize>,
    #[serde(default)]
    pub expect_action_paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateExpectation {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub title_contains: Option<String>,
    #[serde(default)]
    pub path_suffix: Option<String>,
    #[serde(default)]
    pub status_contains: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCheck {
    #[serde(default)]
    pub min_findings: Option<usize>,
    #[serde(default)]
    pub max_findings: Option<usize>,
    #[serde(default)]
    pub expect_focused: Option<bool>,
    #[serde(default)]
    pub expected_profiles: Vec<String>,
    #[serde(default)]
    pub expect_ide_context: Option<bool>,
    #[serde(default)]
    pub expected_titles: Vec<String>,
    #[serde(default)]
    pub expected_categories: Vec<String>,
    #[serde(default)]
    pub expected_files: Vec<String>,
    #[serde(default)]
    pub forbidden_files: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationCheck {
    #[serde(default)]
    pub expected_commands: Vec<String>,
    #[serde(default)]
    pub forbidden_commands: Vec<String>,
    #[serde(default)]
    pub expect_focused: Option<bool>,
    #[serde(default)]
    pub expected_focus_paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImprovementCheck {
    #[serde(default)]
    pub expected_scope: Option<String>,
    #[serde(default)]
    pub min_failures: Option<usize>,
    #[serde(default)]
    pub expected_failure_categories: Vec<String>,
    #[serde(default)]
    pub min_proposals: Option<usize>,
    #[serde(default)]
    pub min_inserted_proposals: Option<usize>,
    #[serde(default)]
    pub expected_proposal_kinds: Vec<String>,
    #[serde(default)]
    pub expect_draft_only: Option<bool>,
    #[serde(default)]
    pub min_eval_runs: Option<usize>,
    #[serde(default)]
    pub expect_eval_success_rate: Option<f64>,
    #[serde(default)]
    pub min_repair_loop_blocked: Option<usize>,
    #[serde(default)]
    pub expected_applied_status: Option<String>,
    #[serde(default)]
    pub expected_applied_kind: Option<String>,
    #[serde(default)]
    pub min_applied_artifacts: Option<usize>,
    #[serde(default)]
    pub expected_action_target_contains: Option<String>,
    #[serde(default)]
    pub min_retros: Option<usize>,
    #[serde(default)]
    pub min_retro_recommendations: Option<usize>,
    #[serde(default)]
    pub expected_promoted_status: Option<String>,
    #[serde(default)]
    pub min_promoted_artifacts: Option<usize>,
    #[serde(default)]
    pub expected_promotion_target_contains: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskLevelCheck {
    #[serde(default)]
    pub expected_outcome: Option<String>,
    #[serde(default)]
    pub min_score: Option<f64>,
    #[serde(default)]
    pub expected_changed_files: Vec<String>,
    #[serde(default)]
    pub forbidden_changed_files: Vec<String>,
    #[serde(default)]
    pub required_diff_contains: Vec<String>,
    #[serde(default)]
    pub forbidden_diff_contains: Vec<String>,
    #[serde(default)]
    pub expected_validation_commands: Vec<String>,
    #[serde(default)]
    pub forbidden_validation_commands: Vec<String>,
    #[serde(default)]
    pub max_changed_files: Option<usize>,
    #[serde(default)]
    pub require_review: Option<bool>,
    #[serde(default)]
    pub require_verification: Option<bool>,
    #[serde(default)]
    pub require_context: Option<bool>,
    #[serde(default)]
    pub require_goal_evaluation: Option<bool>,
    #[serde(default)]
    pub required_context: Vec<CandidateExpectation>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecutionCheck {
    #[serde(default)]
    pub expected_mode: Option<String>,
    #[serde(default)]
    pub expected_status: Option<String>,
    #[serde(default)]
    pub expected_changed_files: Vec<String>,
    #[serde(default)]
    pub forbidden_changed_files: Vec<String>,
    #[serde(default)]
    pub expected_tool_calls: Vec<String>,
    #[serde(default)]
    pub min_tool_calls: Option<usize>,
    #[serde(default)]
    pub require_turn: Option<bool>,
    #[serde(default)]
    pub response_contains: Vec<String>,
    #[serde(default)]
    pub error_contains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckOutcome {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EvalMetrics {
    pub context_precision: Option<f64>,
    pub critical_context_recall: Option<f64>,
    pub review_findings: Option<usize>,
    pub verification_commands: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
    #[serde(default)]
    pub execution_changed_files: Vec<String>,
    #[serde(default)]
    pub execution_tool_calls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_failure_category: Option<String>,
    #[serde(default)]
    pub task_changed_files: Vec<String>,
    #[serde(default)]
    pub task_constraint_violations: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureReport {
    pub name: String,
    pub metrics: EvalMetrics,
    pub outcomes: Vec<CheckOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution: Option<AgentExecutionEvalReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<CodingTaskEvalReport>,
}

impl FixtureReport {
    pub fn passed(&self) -> bool {
        self.outcomes.iter().all(|outcome| outcome.passed)
    }

    pub fn failures(&self) -> Vec<&CheckOutcome> {
        self.outcomes
            .iter()
            .filter(|outcome| !outcome.passed)
            .collect()
    }
}

struct EvalRunArtifacts {
    repo_root: PathBuf,
    execution: Option<AgentExecutionEvalReport>,
    task: Option<CodingTaskEvalReport>,
    workflow: Option<workflow::WorkflowRuntimeResult>,
    review: Option<review::ReviewRunSnapshot>,
    verification: Option<verification::VerificationRunSnapshot>,
    context: Option<context_retrieval::ContextRetrievalSnapshot>,
    improvement: Option<CodingTrendReport>,
    improvement_proposals: Option<GenerateCodingImprovementProposalsResult>,
    improvement_apply: Option<ApplyCodingImprovementProposalResult>,
    improvement_promotion: Option<PromoteCodingImprovementProposalResult>,
    goal_evidence_relations: Vec<String>,
    goal_state: Option<String>,
    goal_evaluated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecutionEvalReport {
    pub mode: String,
    pub status: String,
    pub prompt: String,
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_used: Option<ActiveModel>,
    #[serde(default)]
    pub tool_calls: Vec<String>,
    pub changed_files: Vec<String>,
    pub diff_bytes: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingTaskEvalReport {
    pub task_id: String,
    pub task_type: String,
    pub title: String,
    pub outcome: String,
    pub score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_category: Option<String>,
    pub diff: CodingTaskDiffSummary,
    pub validation: CodingTaskValidationSummary,
    pub review: CodingTaskReviewSummary,
    pub context: CodingTaskContextSummary,
    pub goal: CodingTaskGoalSummary,
    pub checks: Vec<CodingTaskEvalCheckResult>,
    pub metrics: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingTaskDiffSummary {
    pub changed_files: Vec<String>,
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub diff_bytes: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingTaskValidationSummary {
    pub commands: Vec<String>,
    pub command_count: usize,
    pub allowed_command_count: usize,
    pub disallowed_commands: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingTaskReviewSummary {
    pub requested: bool,
    pub findings: usize,
    pub blocking_findings: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingTaskContextSummary {
    pub requested: bool,
    pub candidates: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_context_recall: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingTaskGoalSummary {
    pub requested: bool,
    pub evaluated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    pub evidence_relations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingTaskEvalCheckResult {
    pub name: String,
    pub passed: bool,
    pub detail: String,
    pub category: String,
    pub severity: String,
}

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/coding_eval")
}

pub fn load_fixtures() -> Result<Vec<CodingEvalFixture>> {
    let dir = fixtures_dir();
    let mut paths = std::fs::read_dir(&dir)
        .with_context(|| format!("reading fixtures dir {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading fixture {}", path.display()))?;
        let fixture = serde_json::from_str(&raw)
            .with_context(|| format!("parsing fixture {}", path.display()))?;
        out.push(fixture);
    }
    Ok(out)
}

pub fn gold_task_pack_summary() -> GoldTaskPackSummary {
    let cases = gold_task_cases();
    summarize_gold_task_pack(&cases)
}

pub async fn run_gold_task_pack(
    db: Arc<SessionDB>,
    input: GoldTaskPackRunInput,
) -> Result<GoldTaskPackReport> {
    let selected = select_gold_task_cases(&gold_task_cases(), &input);
    let mut cases = Vec::new();
    let mut automated_cases = 0usize;
    let mut skipped_cases = 0usize;
    let mut passed_cases = 0usize;
    let mut failed_cases = 0usize;
    let mut total_checks = 0usize;

    for case in selected {
        let summary = case.summary();
        let Some(automation) = case.automation.clone() else {
            skipped_cases += 1;
            cases.push(GoldTaskCaseRunReport {
                case: summary,
                status: "skipped".to_string(),
                fixture_name: None,
                report: None,
                error: Some("gold task is not automated yet".to_string()),
            });
            continue;
        };

        automated_cases += 1;
        let fixture = materialize_gold_task_fixture(&case, automation, &input);
        let fixture_name = fixture.name.clone();
        match evaluate(db.clone(), &fixture).await {
            Ok(report) => {
                total_checks += report.outcomes.len();
                if report.passed() {
                    passed_cases += 1;
                    cases.push(GoldTaskCaseRunReport {
                        case: summary,
                        status: "passed".to_string(),
                        fixture_name: Some(fixture_name),
                        report: Some(report),
                        error: None,
                    });
                } else {
                    failed_cases += 1;
                    cases.push(GoldTaskCaseRunReport {
                        case: summary,
                        status: "failed".to_string(),
                        fixture_name: Some(fixture_name),
                        report: Some(report),
                        error: None,
                    });
                }
            }
            Err(err) => {
                failed_cases += 1;
                cases.push(GoldTaskCaseRunReport {
                    case: summary,
                    status: "error".to_string(),
                    fixture_name: Some(fixture_name),
                    report: None,
                    error: Some(err.to_string()),
                });
            }
        }
    }

    let mut report = GoldTaskPackReport {
        pack_id: GOLD_TASK_PACK_ID.to_string(),
        source_doc: GOLD_TASK_SOURCE_DOC.to_string(),
        pack_run_id: None,
        selected_cases: cases.len(),
        automated_cases,
        skipped_cases,
        passed_cases,
        failed_cases,
        total_checks,
        passed: failed_cases == 0 && automated_cases > 0,
        cases,
    };

    if input.record_pack_run {
        let record = db.record_coding_eval_pack_run(RecordCodingEvalPackRunInput {
            session_id: input.session_id,
            project_id: input.project_id,
            label: input.label,
            baseline_kind: input.baseline_kind,
            source_type: input
                .source_type
                .or_else(|| Some("gold_task_pack".to_string())),
            source_id: input.source_id.or_else(|| Some(report.pack_id.clone())),
            report: report.clone(),
        })?;
        report.pack_run_id = Some(record.id);
    }

    Ok(report)
}

pub fn evaluate_strategy_effect(input: StrategyEffectEvalInput) -> StrategyEffectReport {
    let strategy_type = input
        .strategy_type
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "strategy".to_string());
    let baseline_label = input
        .baseline_label
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "baseline".to_string());
    let candidate_label = input
        .candidate_label
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "candidate".to_string());
    let baseline_map = input
        .baseline
        .cases
        .iter()
        .map(|case| (case.case.id.clone(), case))
        .collect::<HashMap<_, _>>();
    let candidate_map = input
        .candidate
        .cases
        .iter()
        .map(|case| (case.case.id.clone(), case))
        .collect::<HashMap<_, _>>();

    let mut baseline_only_cases = Vec::new();
    let mut candidate_only_cases = candidate_map
        .keys()
        .filter(|id| !baseline_map.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    candidate_only_cases.sort();

    let mut comparisons = Vec::new();
    let mut baseline_metrics = Vec::new();
    let mut candidate_metrics = Vec::new();
    for baseline_case in &input.baseline.cases {
        let id = &baseline_case.case.id;
        let Some(candidate_case) = candidate_map.get(id) else {
            baseline_only_cases.push(id.clone());
            continue;
        };
        let baseline = strategy_case_metrics(baseline_case);
        let candidate = strategy_case_metrics(candidate_case);
        baseline_metrics.push(baseline.clone());
        candidate_metrics.push(candidate.clone());
        comparisons.push(compare_strategy_case(&baseline, &candidate));
    }
    baseline_only_cases.sort();
    comparisons.sort_by(|left, right| left.id.cmp(&right.id));

    let baseline_aggregate = aggregate_strategy_metrics(&baseline_metrics);
    let candidate_aggregate = aggregate_strategy_metrics(&candidate_metrics);
    let summary = StrategyEffectSummary {
        baseline_pass_rate: baseline_aggregate.pass_rate(),
        candidate_pass_rate: candidate_aggregate.pass_rate(),
        pass_rate_delta: round3(candidate_aggregate.pass_rate() - baseline_aggregate.pass_rate()),
        baseline_average_score: baseline_aggregate.average_score(),
        candidate_average_score: candidate_aggregate.average_score(),
        average_score_delta: round3(
            candidate_aggregate.average_score() - baseline_aggregate.average_score(),
        ),
        baseline_context_recall: baseline_aggregate.average_context_recall(),
        candidate_context_recall: candidate_aggregate.average_context_recall(),
        context_recall_delta: round3(
            candidate_aggregate.average_context_recall()
                - baseline_aggregate.average_context_recall(),
        ),
        baseline_validation_violations: baseline_aggregate.validation_violations,
        candidate_validation_violations: candidate_aggregate.validation_violations,
        validation_violation_delta: candidate_aggregate.validation_violations as isize
            - baseline_aggregate.validation_violations as isize,
        baseline_scope_creep: baseline_aggregate.scope_creep,
        candidate_scope_creep: candidate_aggregate.scope_creep,
        scope_creep_delta: candidate_aggregate.scope_creep as isize
            - baseline_aggregate.scope_creep as isize,
        baseline_execution_failures: baseline_aggregate.execution_failures,
        candidate_execution_failures: candidate_aggregate.execution_failures,
        execution_failure_delta: candidate_aggregate.execution_failures as isize
            - baseline_aggregate.execution_failures as isize,
    };
    let dimensions = strategy_effect_dimensions(&summary);
    let mut regressions = Vec::new();
    let mut improvements = Vec::new();
    for case in &comparisons {
        match case.verdict.as_str() {
            "regressed" => regressions.push(format!("{}: {}", case.id, case.notes.join("; "))),
            "improved" => improvements.push(format!("{}: {}", case.id, case.notes.join("; "))),
            "mixed" => {
                regressions.push(format!("{}: {}", case.id, case.notes.join("; ")));
                improvements.push(format!("{}: {}", case.id, case.notes.join("; ")));
            }
            _ => {}
        }
    }
    for id in &baseline_only_cases {
        regressions.push(format!("{id}: candidate report is missing a baseline case"));
    }
    let verdict = derive_strategy_verdict(&comparisons, &baseline_only_cases);

    StrategyEffectReport {
        run_id: None,
        strategy_type,
        baseline_label,
        candidate_label,
        verdict,
        compared_cases: comparisons.len(),
        baseline_only_cases,
        candidate_only_cases,
        summary,
        dimensions,
        cases: comparisons,
        regressions,
        improvements,
    }
}

pub fn evaluate_strategy_effect_with_recording(
    db: &SessionDB,
    input: StrategyEffectEvalInput,
) -> Result<StrategyEffectReport> {
    let should_record = input.record_run;
    let session_id = input.session_id.clone();
    let project_id = input.project_id.clone();
    let baseline_pack_run_id = input
        .baseline_pack_run_id
        .clone()
        .or_else(|| input.baseline.pack_run_id.clone());
    let candidate_pack_run_id = input
        .candidate_pack_run_id
        .clone()
        .or_else(|| input.candidate.pack_run_id.clone());
    let source_type = input
        .source_type
        .clone()
        .or_else(|| Some("strategy_effect".to_string()));
    let source_id = input.source_id.clone();
    let mut report = evaluate_strategy_effect(input);
    if should_record {
        let record = db.record_coding_strategy_effect_run(RecordCodingStrategyEffectRunInput {
            session_id,
            project_id,
            baseline_pack_run_id,
            candidate_pack_run_id,
            source_type,
            source_id,
            report: report.clone(),
        })?;
        report.run_id = Some(record.id);
    }
    Ok(report)
}

#[derive(Debug, Clone, Default)]
struct StrategyCaseMetrics {
    id: String,
    title: String,
    status: String,
    passed: bool,
    outcome: Option<String>,
    score: f64,
    context_recall: f64,
    validation_violations: usize,
    scope_creep: usize,
    execution_failed: bool,
}

#[derive(Debug, Clone, Default)]
struct StrategyAggregate {
    cases: usize,
    passed: usize,
    score_sum: f64,
    context_recall_sum: f64,
    validation_violations: usize,
    scope_creep: usize,
    execution_failures: usize,
}

impl StrategyAggregate {
    fn pass_rate(&self) -> f64 {
        if self.cases == 0 {
            0.0
        } else {
            round3(self.passed as f64 / self.cases as f64)
        }
    }

    fn average_score(&self) -> f64 {
        if self.cases == 0 {
            0.0
        } else {
            round3(self.score_sum / self.cases as f64)
        }
    }

    fn average_context_recall(&self) -> f64 {
        if self.cases == 0 {
            0.0
        } else {
            round3(self.context_recall_sum / self.cases as f64)
        }
    }
}

fn strategy_case_metrics(case: &GoldTaskCaseRunReport) -> StrategyCaseMetrics {
    let report = case.report.as_ref();
    let task = report.and_then(|report| report.task.as_ref());
    let passed = case.status == "passed" && report.is_some_and(FixtureReport::passed);
    let outcome = task.map(|task| task.outcome.clone());
    let score = task
        .map(|task| task.score)
        .or_else(|| report.and_then(|report| report.metrics.task_score))
        .unwrap_or(if passed { 1.0 } else { 0.0 });
    let context_recall = task
        .and_then(|task| task.context.required_context_recall)
        .or_else(|| report.and_then(|report| report.metrics.critical_context_recall))
        .unwrap_or(0.0);
    let validation_violations = task
        .map(|task| task.validation.disallowed_commands.len())
        .unwrap_or(0);
    let scope_creep = report
        .map(|report| report.metrics.task_constraint_violations)
        .unwrap_or_else(|| {
            task.map(|task| {
                task.checks
                    .iter()
                    .filter(|check| {
                        !check.passed
                            && matches!(check.category.as_str(), "scope_creep" | "policy_violation")
                    })
                    .count()
            })
            .unwrap_or(0)
        });
    let execution_failed = case.status == "error"
        || report
            .and_then(|report| report.execution.as_ref())
            .is_some_and(|execution| execution.status != "completed")
        || report
            .and_then(|report| report.metrics.execution_status.as_deref())
            .is_some_and(|status| status == "failed");

    StrategyCaseMetrics {
        id: case.case.id.clone(),
        title: case.case.title.clone(),
        status: case.status.clone(),
        passed,
        outcome,
        score: round3(score),
        context_recall: round3(context_recall),
        validation_violations,
        scope_creep,
        execution_failed,
    }
}

fn compare_strategy_case(
    baseline: &StrategyCaseMetrics,
    candidate: &StrategyCaseMetrics,
) -> StrategyCaseComparison {
    let mut improved = false;
    let mut regressed = false;
    let mut notes = Vec::new();
    let score_delta = round3(candidate.score - baseline.score);
    let context_recall_delta = round3(candidate.context_recall - baseline.context_recall);

    if !baseline.passed && candidate.passed {
        improved = true;
        notes.push("pass status improved".to_string());
    } else if baseline.passed && !candidate.passed {
        regressed = true;
        notes.push("pass status regressed".to_string());
    }

    if baseline.execution_failed && !candidate.execution_failed {
        improved = true;
        notes.push("execution failure cleared".to_string());
    } else if !baseline.execution_failed && candidate.execution_failed {
        regressed = true;
        notes.push("execution failure introduced".to_string());
    }

    if score_delta > 0.001 {
        improved = true;
        notes.push(format!("task score +{score_delta:.3}"));
    } else if score_delta < -0.001 {
        regressed = true;
        notes.push(format!("task score {score_delta:.3}"));
    }

    if context_recall_delta > 0.001 {
        improved = true;
        notes.push(format!("context recall +{context_recall_delta:.3}"));
    } else if context_recall_delta < -0.001 {
        regressed = true;
        notes.push(format!("context recall {context_recall_delta:.3}"));
    }

    match candidate
        .validation_violations
        .cmp(&baseline.validation_violations)
    {
        std::cmp::Ordering::Less => {
            improved = true;
            notes.push(format!(
                "validation violations {} -> {}",
                baseline.validation_violations, candidate.validation_violations
            ));
        }
        std::cmp::Ordering::Greater => {
            regressed = true;
            notes.push(format!(
                "validation violations {} -> {}",
                baseline.validation_violations, candidate.validation_violations
            ));
        }
        std::cmp::Ordering::Equal => {}
    }

    match candidate.scope_creep.cmp(&baseline.scope_creep) {
        std::cmp::Ordering::Less => {
            improved = true;
            notes.push(format!(
                "scope creep {} -> {}",
                baseline.scope_creep, candidate.scope_creep
            ));
        }
        std::cmp::Ordering::Greater => {
            regressed = true;
            notes.push(format!(
                "scope creep {} -> {}",
                baseline.scope_creep, candidate.scope_creep
            ));
        }
        std::cmp::Ordering::Equal => {}
    }

    let verdict = if improved && regressed {
        "mixed"
    } else if regressed {
        "regressed"
    } else if improved {
        "improved"
    } else {
        "unchanged"
    };
    if notes.is_empty() {
        notes.push("no material metric change".to_string());
    }

    StrategyCaseComparison {
        id: baseline.id.clone(),
        title: baseline.title.clone(),
        verdict: verdict.to_string(),
        baseline_status: baseline.status.clone(),
        candidate_status: candidate.status.clone(),
        baseline_passed: baseline.passed,
        candidate_passed: candidate.passed,
        baseline_outcome: baseline.outcome.clone(),
        candidate_outcome: candidate.outcome.clone(),
        baseline_score: baseline.score,
        candidate_score: candidate.score,
        score_delta,
        baseline_context_recall: baseline.context_recall,
        candidate_context_recall: candidate.context_recall,
        context_recall_delta,
        baseline_validation_violations: baseline.validation_violations,
        candidate_validation_violations: candidate.validation_violations,
        baseline_scope_creep: baseline.scope_creep,
        candidate_scope_creep: candidate.scope_creep,
        baseline_execution_failed: baseline.execution_failed,
        candidate_execution_failed: candidate.execution_failed,
        notes,
    }
}

fn aggregate_strategy_metrics(metrics: &[StrategyCaseMetrics]) -> StrategyAggregate {
    let mut aggregate = StrategyAggregate {
        cases: metrics.len(),
        ..Default::default()
    };
    for metric in metrics {
        if metric.passed {
            aggregate.passed += 1;
        }
        aggregate.score_sum += metric.score;
        aggregate.context_recall_sum += metric.context_recall;
        aggregate.validation_violations += metric.validation_violations;
        aggregate.scope_creep += metric.scope_creep;
        if metric.execution_failed {
            aggregate.execution_failures += 1;
        }
    }
    aggregate
}

fn strategy_effect_dimensions(summary: &StrategyEffectSummary) -> Vec<StrategyEffectDimension> {
    vec![
        strategy_dimension(
            "passRate",
            "higher",
            summary.baseline_pass_rate,
            summary.candidate_pass_rate,
            summary.pass_rate_delta,
            "candidate pass rate versus baseline",
        ),
        strategy_dimension(
            "averageTaskScore",
            "higher",
            summary.baseline_average_score,
            summary.candidate_average_score,
            summary.average_score_delta,
            "average task-level score across common cases",
        ),
        strategy_dimension(
            "contextRecall",
            "higher",
            summary.baseline_context_recall,
            summary.candidate_context_recall,
            summary.context_recall_delta,
            "average required context recall across common cases",
        ),
        strategy_dimension(
            "validationViolations",
            "lower",
            summary.baseline_validation_violations as f64,
            summary.candidate_validation_violations as f64,
            summary.validation_violation_delta as f64,
            "disallowed validation commands across common cases",
        ),
        strategy_dimension(
            "scopeCreep",
            "lower",
            summary.baseline_scope_creep as f64,
            summary.candidate_scope_creep as f64,
            summary.scope_creep_delta as f64,
            "scope or policy violations across common cases",
        ),
        strategy_dimension(
            "executionFailures",
            "lower",
            summary.baseline_execution_failures as f64,
            summary.candidate_execution_failures as f64,
            summary.execution_failure_delta as f64,
            "agent execution failures across common cases",
        ),
    ]
}

fn strategy_dimension(
    name: &str,
    direction: &str,
    baseline: f64,
    candidate: f64,
    delta: f64,
    detail: &str,
) -> StrategyEffectDimension {
    let verdict = if delta.abs() <= 0.001 {
        "unchanged"
    } else if (direction == "higher" && delta > 0.0) || (direction == "lower" && delta < 0.0) {
        "improved"
    } else {
        "regressed"
    };
    StrategyEffectDimension {
        name: name.to_string(),
        direction: direction.to_string(),
        baseline: round3(baseline),
        candidate: round3(candidate),
        delta: round3(delta),
        verdict: verdict.to_string(),
        detail: detail.to_string(),
    }
}

fn derive_strategy_verdict(
    comparisons: &[StrategyCaseComparison],
    baseline_only_cases: &[String],
) -> String {
    if comparisons.is_empty() {
        return if baseline_only_cases.is_empty() {
            "inconclusive".to_string()
        } else {
            "regressed".to_string()
        };
    }
    let has_missing_baseline_case = !baseline_only_cases.is_empty();
    let has_regression = has_missing_baseline_case
        || comparisons
            .iter()
            .any(|case| matches!(case.verdict.as_str(), "regressed" | "mixed"));
    let has_improvement = comparisons
        .iter()
        .any(|case| matches!(case.verdict.as_str(), "improved" | "mixed"));
    if has_regression && has_improvement {
        "mixed".to_string()
    } else if has_regression {
        "regressed".to_string()
    } else if has_improvement {
        "improved".to_string()
    } else {
        "unchanged".to_string()
    }
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

pub async fn evaluate(db: Arc<SessionDB>, fixture: &CodingEvalFixture) -> Result<FixtureReport> {
    let temp = tempfile::tempdir().context("create coding eval tempdir")?;
    let repo_root = prepare_repo(temp.path(), fixture)?;
    let session = db.create_session(DEFAULT_AGENT_ID)?;
    db.update_session_working_dir(&session.id, Some(repo_root.to_string_lossy().to_string()))?;

    let goal_id = if let Some(goal) = &fixture.setup.goal {
        let snapshot = db.create_goal(CreateGoalInput {
            session_id: session.id.clone(),
            objective: goal.objective.clone(),
            completion_criteria: goal.completion_criteria.clone(),
            budget_token_limit: None,
            budget_time_limit_secs: None,
            budget_turn_limit: None,
        })?;
        Some(snapshot.goal.id)
    } else {
        None
    };

    seed_tasks(&db, &session.id, &fixture.setup.tasks)?;
    if let Some(workflow) = &fixture.setup.workflow {
        seed_workflow(&db, &session.id, goal_id.as_deref(), workflow)?;
    }

    let mut artifacts = EvalRunArtifacts {
        repo_root,
        execution: None,
        task: None,
        workflow: None,
        review: None,
        verification: None,
        context: None,
        improvement: None,
        improvement_proposals: None,
        improvement_apply: None,
        improvement_promotion: None,
        goal_evidence_relations: Vec::new(),
        goal_state: None,
        goal_evaluated: false,
    };

    if let Some(run) = &fixture.runs.execution {
        artifacts.execution = Some(
            run_agent_execution_eval(&db, &session.id, &artifacts.repo_root, fixture, run).await?,
        );
    }

    if let Some(run) = &fixture.runs.workflow {
        let workflow_run = db.create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: run.kind.clone(),
            execution_mode: run.execution_mode.clone(),
            script_source: run.script_source.clone(),
            budget: run.budget.clone(),
            parent_run_id: None,
            origin: Some("eval".to_string()),
            goal_id: goal_id.clone(),
            worktree_id: None,
        })?;
        artifacts.workflow = match workflow::run_workflow_script_async(db.clone(), &workflow_run.id)
            .await
        {
            Ok(result) => Some(result),
            Err(_err) if run.allow_terminal_error => {
                let snapshot = db
                    .workflow_run_snapshot(&workflow_run.id, 500)?
                    .ok_or_else(|| anyhow::anyhow!("workflow run {} not found", workflow_run.id))?;
                Some(workflow::WorkflowRuntimeResult {
                    snapshot,
                    output: None,
                })
            }
            Err(err) => return Err(err),
        };
    }

    if let Some(run) = &fixture.runs.review {
        artifacts.review = Some(
            review::run_review_for_session(
                db.clone(),
                session.id.clone(),
                RunReviewInput {
                    scope: Some("local".to_string()),
                    goal_id: goal_id.clone(),
                    profiles: run.profiles.clone(),
                    focus_paths: resolve_focus_paths(&artifacts.repo_root, &run.focus_paths),
                    ide_context: run.ide_context.clone(),
                    ..Default::default()
                },
            )
            .await?,
        );
    }

    if let Some(run) = &fixture.runs.verification {
        artifacts.verification = Some(
            verification::plan_verification_for_session(
                db.clone(),
                session.id.clone(),
                PlanVerificationInput {
                    scope: Some("local".to_string()),
                    goal_id: goal_id.clone(),
                    max_commands: run.max_commands,
                    focus_paths: resolve_focus_paths(&artifacts.repo_root, &run.focus_paths),
                },
            )
            .await?,
        );
    }

    if let Some(run) = &fixture.runs.context {
        artifacts.context = Some(
            context_retrieval::context_retrieval_for_session(
                db.clone(),
                session.id.clone(),
                context_retrieval::ContextRetrievalInput {
                    query: run.query.clone(),
                    limit: run.limit,
                    ide_context: run.ide_context.clone(),
                },
            )
            .await?,
        );
    }

    if fixture.task.is_some() || fixture.runs.task.is_some() || fixture.checks.task.is_some() {
        let run = fixture.runs.task.clone().unwrap_or_default();
        if run.evaluate_goal {
            if let Some(goal_id) = goal_id.as_deref() {
                artifacts.goal_evaluated = true;
                let should_evaluate = db
                    .goal_snapshot(goal_id, 20)?
                    .is_some_and(|snapshot| !snapshot.goal.state.is_terminal());
                if should_evaluate {
                    let _ = db.evaluate_goal(goal_id)?;
                }
            }
        }
        refresh_goal_artifacts(&db, goal_id.as_deref(), &mut artifacts)?;
        let task_report = build_task_eval_report(fixture, &artifacts)?;
        if run.record_eval_run {
            record_task_eval_run(&db, &session.id, &task_report)?;
        }
        artifacts.task = Some(task_report);
    }

    if let Some(run) = &fixture.runs.improvement {
        for seed in &run.seed_eval_runs {
            let mut input = seed.clone();
            if input.session_id.is_none() {
                input.session_id = Some(session.id.clone());
            }
            db.record_coding_eval_run(input)?;
        }
        if run.generate_proposals {
            artifacts.improvement_proposals =
                Some(db.generate_coding_improvement_proposals(&session.id, run.window_days)?);
        }
        if run.apply_first_proposal {
            let desired_kind = run.apply_proposal_kind.as_deref();
            let proposal_id = artifacts
                .improvement_proposals
                .as_ref()
                .and_then(|result| {
                    result
                        .proposals
                        .iter()
                        .find(|proposal| {
                            proposal.status == "draft"
                                && desired_kind.is_none_or(|kind| proposal.kind == kind)
                        })
                        .map(|proposal| proposal.id.clone())
                })
                .or_else(|| {
                    db.list_coding_improvement_proposals(&session.id)
                        .ok()
                        .and_then(|proposals| {
                            proposals
                                .into_iter()
                                .find(|proposal| {
                                    proposal.status == "draft"
                                        && desired_kind.is_none_or(|kind| proposal.kind == kind)
                                })
                                .map(|proposal| proposal.id)
                        })
                })
                .ok_or_else(|| {
                    anyhow!("applyFirstProposal requested but no draft proposal exists")
                })?;
            artifacts.improvement_apply = Some(db.apply_coding_improvement_proposal(&proposal_id)?);
        }
        if run.promote_applied_proposal {
            let proposal_id = artifacts
                .improvement_apply
                .as_ref()
                .map(|result| result.proposal.id.clone())
                .or_else(|| {
                    db.list_coding_improvement_proposals(&session.id)
                        .ok()
                        .and_then(|proposals| {
                            proposals
                                .into_iter()
                                .find(|proposal| proposal.status == "applied")
                                .map(|proposal| proposal.id)
                        })
                })
                .ok_or_else(|| {
                    anyhow!("promoteAppliedProposal requested but no applied proposal exists")
                })?;
            artifacts.improvement_promotion =
                Some(db.promote_coding_improvement_proposal(&proposal_id)?);
        }
        artifacts.improvement = Some(db.coding_trend_report(&session.id, run.window_days)?);
    }

    if let Some(goal_id) = goal_id.as_deref() {
        refresh_goal_artifacts(&db, Some(goal_id), &mut artifacts)?;
    }

    Ok(check_fixture(fixture, &artifacts))
}

fn refresh_goal_artifacts(
    db: &SessionDB,
    goal_id: Option<&str>,
    artifacts: &mut EvalRunArtifacts,
) -> Result<()> {
    let Some(goal_id) = goal_id else {
        return Ok(());
    };
    if let Some(snapshot) = db.goal_snapshot(goal_id, 200)? {
        artifacts.goal_state = Some(snapshot.goal.state.as_str().to_string());
        artifacts.goal_evidence_relations = snapshot
            .evidence
            .iter()
            .map(|item| item.relation.clone())
            .collect();
    }
    Ok(())
}

async fn run_agent_execution_eval(
    db: &Arc<SessionDB>,
    session_id: &str,
    repo_root: &Path,
    fixture: &CodingEvalFixture,
    run: &AgentExecutionEvalRun,
) -> Result<AgentExecutionEvalReport> {
    let prompt = run
        .prompt
        .clone()
        .or_else(|| fixture.task.as_ref().map(|task| task.prompt.clone()))
        .unwrap_or_else(|| fixture.description.clone());
    let agent_id = run
        .agent_id
        .clone()
        .unwrap_or_else(|| DEFAULT_AGENT_ID.to_string());
    let mode = run.mode.trim();

    match mode {
        "fixture_patch" => {
            for file in &fixture.repo.changes {
                write_fixture_file(repo_root, file)?;
            }
            let (changed_files, diff_bytes) = execution_diff_snapshot(repo_root)?;
            Ok(AgentExecutionEvalReport {
                mode: mode.to_string(),
                status: "completed".to_string(),
                prompt,
                agent_id,
                turn_id: None,
                response: Some("fixture patch applied".to_string()),
                error: None,
                model_used: None,
                tool_calls: Vec::new(),
                changed_files,
                diff_bytes,
            })
        }
        "agent" => {
            if prompt.trim().is_empty() {
                let (changed_files, diff_bytes) = execution_diff_snapshot(repo_root)?;
                return Ok(AgentExecutionEvalReport {
                    mode: mode.to_string(),
                    status: "failed".to_string(),
                    prompt,
                    agent_id,
                    turn_id: None,
                    response: None,
                    error: Some("agent execution requires a task prompt".to_string()),
                    model_used: None,
                    tool_calls: Vec::new(),
                    changed_files,
                    diff_bytes,
                });
            }
            if run.model_chain.is_empty() || run.providers.is_empty() {
                let (changed_files, diff_bytes) = execution_diff_snapshot(repo_root)?;
                return Ok(AgentExecutionEvalReport {
                    mode: mode.to_string(),
                    status: "failed".to_string(),
                    prompt,
                    agent_id,
                    turn_id: None,
                    response: None,
                    error: Some(
                        "agent execution requires providers and modelChain in the fixture"
                            .to_string(),
                    ),
                    model_used: None,
                    tool_calls: Vec::new(),
                    changed_files,
                    diff_bytes,
                });
            }

            let user_message_id = db
                .append_message(
                    session_id,
                    &NewMessage::user(&prompt).with_source(ChatSource::Http),
                )
                .ok();
            let turn_id = uuid::Uuid::new_v4().to_string();
            db.create_chat_turn_with_id(
                &turn_id,
                session_id,
                ChatSource::Http.as_str(),
                None,
                user_message_id,
            )?;

            let params = ChatEngineParams {
                session_id: session_id.to_string(),
                agent_id: agent_id.clone(),
                turn_id: Some(turn_id.clone()),
                message: prompt.clone(),
                display_text: run.display_text.clone(),
                attachments: Vec::new(),
                session_db: db.clone(),
                model_chain: run.model_chain.clone(),
                providers: run.providers.clone(),
                codex_token: None,
                resolved_temperature: None,
                compact_config: run.compact_config.clone().unwrap_or_default(),
                extra_system_context: run.extra_system_context.clone(),
                reasoning_effort: run
                    .reasoning_effort
                    .clone()
                    .or_else(|| Some("none".to_string())),
                cancel: Arc::new(AtomicBool::new(false)),
                plan_context_override: Some(crate::agent::PlanResolvedContext::off()),
                skill_allowed_tools: Vec::new(),
                denied_tools: run.denied_tools.clone(),
                tool_scope: None,
                subagent_depth: 0,
                steer_run_id: None,
                auto_approve_tools: run.auto_approve_tools,
                follow_global_reasoning_effort: false,
                post_turn_effects: false,
                abort_on_cancel: false,
                persist_final_error_event: true,
                source: ChatSource::Http,
                origin_source: None,
                channel_kb_context: None,
                event_sink: Arc::new(NoopEventSink),
            };

            let result = chat_engine::run_chat_engine(params).await;
            let (changed_files, diff_bytes) = execution_diff_snapshot(repo_root)?;
            let tool_calls = execution_tool_calls(db, session_id)?;
            match result {
                Ok(result) => Ok(AgentExecutionEvalReport {
                    mode: mode.to_string(),
                    status: "completed".to_string(),
                    prompt,
                    agent_id,
                    turn_id: Some(turn_id),
                    response: Some(result.response),
                    error: None,
                    model_used: result.model_used,
                    tool_calls,
                    changed_files,
                    diff_bytes,
                }),
                Err(err) => Ok(AgentExecutionEvalReport {
                    mode: mode.to_string(),
                    status: "failed".to_string(),
                    prompt,
                    agent_id,
                    turn_id: Some(turn_id),
                    response: None,
                    error: Some(err),
                    model_used: None,
                    tool_calls,
                    changed_files,
                    diff_bytes,
                }),
            }
        }
        other => {
            let (changed_files, diff_bytes) = execution_diff_snapshot(repo_root)?;
            Ok(AgentExecutionEvalReport {
                mode: other.to_string(),
                status: "failed".to_string(),
                prompt,
                agent_id,
                turn_id: None,
                response: None,
                error: Some(format!(
                    "unsupported coding eval execution mode {other:?}; expected agent or fixture_patch"
                )),
                model_used: None,
                tool_calls: Vec::new(),
                changed_files,
                diff_bytes,
            })
        }
    }
}

fn execution_tool_calls(db: &SessionDB, session_id: &str) -> Result<Vec<String>> {
    Ok(db
        .load_session_messages(session_id)?
        .into_iter()
        .filter(|message| message.role == MessageRole::Tool)
        .filter_map(|message| message.tool_name)
        .collect())
}

fn execution_diff_snapshot(repo_root: &Path) -> Result<(Vec<String>, usize)> {
    let diff = read_task_diff_summary(repo_root)?;
    Ok((diff.changed_files, diff.diff_bytes))
}

fn build_task_eval_report(
    fixture: &CodingEvalFixture,
    artifacts: &EvalRunArtifacts,
) -> Result<CodingTaskEvalReport> {
    let task = fixture
        .task
        .as_ref()
        .ok_or_else(|| anyhow!("task-level eval requested but fixture.task is missing"))?;
    let check = fixture.checks.task.as_ref();
    let diff = read_task_diff_summary(&artifacts.repo_root)?;
    let validation = task_validation_summary(task, artifacts);
    let review = task_review_summary(artifacts);
    let context = task_context_summary(artifacts, check);
    let goal = CodingTaskGoalSummary {
        requested: artifacts.goal_state.is_some() || !artifacts.goal_evidence_relations.is_empty(),
        evaluated: artifacts.goal_evaluated,
        state: artifacts.goal_state.clone(),
        evidence_relations: artifacts.goal_evidence_relations.clone(),
    };
    let diff_text = run_git(&artifacts.repo_root, &["diff", "--"])?;
    let mut checks = Vec::new();
    if let Some(execution) = artifacts.execution.as_ref() {
        push_task_check(
            &mut checks,
            "execution.completed",
            execution.status == "completed",
            format!(
                "execution.status={}, error={:?}",
                execution.status, execution.error
            ),
            "execution_failed",
            "critical",
        );
    }
    push_task_spec_checks(task, &diff, &validation, &mut checks);
    if let Some(check) = check {
        push_task_fixture_checks(
            check,
            &diff,
            &diff_text,
            &validation,
            &review,
            &context,
            &goal,
            artifacts,
            &mut checks,
        );
    }
    let passed = checks.iter().filter(|check| check.passed).count();
    let total = checks.len();
    let score = if total == 0 {
        0.0
    } else {
        (passed as f64 / total as f64 * 1000.0).round() / 1000.0
    };
    let failure_category = checks
        .iter()
        .find(|check| !check.passed)
        .map(|check| check.category.clone());
    let outcome = derive_task_outcome(&checks, score).to_string();
    Ok(CodingTaskEvalReport {
        task_id: task.id.clone(),
        task_type: if task.task_type.is_empty() {
            "coding".to_string()
        } else {
            task.task_type.clone()
        },
        title: task.title.clone(),
        outcome,
        score,
        failure_category,
        diff,
        validation,
        review,
        context,
        goal,
        checks,
        metrics: json!({
            "fixture": fixture.name,
            "taskId": task.id,
            "taskType": task.task_type,
            "source": task.source,
            "executionMode": task.execution_mode,
            "agentExecution": artifacts.execution.as_ref().map(|execution| json!({
                "mode": &execution.mode,
                "status": &execution.status,
                "turnId": &execution.turn_id,
                "modelUsed": &execution.model_used,
                "toolCalls": &execution.tool_calls,
                "changedFiles": &execution.changed_files,
                "diffBytes": execution.diff_bytes,
            })),
        }),
    })
}

fn record_task_eval_run(
    db: &SessionDB,
    session_id: &str,
    report: &CodingTaskEvalReport,
) -> Result<()> {
    db.record_coding_eval_run(RecordCodingEvalRunInput {
        session_id: Some(session_id.to_string()),
        project_id: None,
        suite: "task_level_coding_eval".to_string(),
        name: report.task_id.clone(),
        status: task_outcome_to_eval_status(&report.outcome).to_string(),
        metrics: serde_json::to_value(report)?,
        source_type: Some("coding_task_eval".to_string()),
        source_id: Some(report.task_id.clone()),
    })?;
    Ok(())
}

fn read_task_diff_summary(repo_root: &Path) -> Result<CodingTaskDiffSummary> {
    let changed_raw = run_git(repo_root, &["diff", "--name-only"])?;
    let changed_files = changed_raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let numstat_raw = run_git(repo_root, &["diff", "--numstat"])?;
    let mut insertions = 0usize;
    let mut deletions = 0usize;
    for line in numstat_raw.lines() {
        let mut parts = line.split('\t');
        insertions += parts
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        deletions += parts
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
    }
    let diff = run_git(repo_root, &["diff", "--"])?;
    Ok(CodingTaskDiffSummary {
        files_changed: changed_files.len(),
        changed_files,
        insertions,
        deletions,
        diff_bytes: diff.len(),
    })
}

fn task_validation_summary(
    task: &CodingTaskEvalSpec,
    artifacts: &EvalRunArtifacts,
) -> CodingTaskValidationSummary {
    let commands = artifacts
        .verification
        .as_ref()
        .map(|snapshot| {
            snapshot
                .steps
                .iter()
                .map(|step| step.command.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let disallowed_commands = if task.allowed_validation.is_empty() {
        Vec::new()
    } else {
        commands
            .iter()
            .filter(|command| {
                !task
                    .allowed_validation
                    .iter()
                    .any(|allowed| allowed == *command)
            })
            .cloned()
            .collect::<Vec<_>>()
    };
    CodingTaskValidationSummary {
        allowed_command_count: commands.len().saturating_sub(disallowed_commands.len()),
        command_count: commands.len(),
        commands,
        disallowed_commands,
    }
}

fn task_review_summary(artifacts: &EvalRunArtifacts) -> CodingTaskReviewSummary {
    let Some(snapshot) = artifacts.review.as_ref() else {
        return CodingTaskReviewSummary::default();
    };
    let blocking_findings = snapshot
        .findings
        .iter()
        .filter(|finding| finding.severity.is_blocking() && finding.status.as_str() == "open")
        .count();
    CodingTaskReviewSummary {
        requested: true,
        findings: snapshot.findings.len(),
        blocking_findings,
    }
}

fn task_context_summary(
    artifacts: &EvalRunArtifacts,
    check: Option<&TaskLevelCheck>,
) -> CodingTaskContextSummary {
    let Some(snapshot) = artifacts.context.as_ref() else {
        return CodingTaskContextSummary::default();
    };
    let required = check
        .map(|check| check.required_context.as_slice())
        .unwrap_or(&[]);
    let matched = required
        .iter()
        .filter(|expected| {
            snapshot
                .candidates
                .iter()
                .any(|candidate| candidate_matches(candidate, expected))
        })
        .count();
    CodingTaskContextSummary {
        requested: true,
        candidates: snapshot.candidates.len(),
        required_context_recall: if required.is_empty() {
            None
        } else {
            Some((matched as f64 / required.len() as f64 * 1000.0).round() / 1000.0)
        },
    }
}

fn push_task_spec_checks(
    task: &CodingTaskEvalSpec,
    diff: &CodingTaskDiffSummary,
    validation: &CodingTaskValidationSummary,
    checks: &mut Vec<CodingTaskEvalCheckResult>,
) {
    if task
        .expected_artifacts
        .iter()
        .any(|artifact| artifact == "diff")
    {
        push_task_check(
            checks,
            "artifact.diff",
            diff.files_changed > 0,
            format!("{} changed file(s)", diff.files_changed),
            "implementation_bug",
            "critical",
        );
    }
    if task
        .expected_artifacts
        .iter()
        .any(|artifact| artifact == "validation")
    {
        push_task_check(
            checks,
            "artifact.validation",
            validation.command_count > 0,
            format!("{} validation command(s)", validation.command_count),
            "validation_gap",
            "high",
        );
    }
    if !task.allowed_validation.is_empty() && validation.command_count > 0 {
        push_task_check(
            checks,
            "validation.allowed",
            validation.disallowed_commands.is_empty(),
            format!("disallowed={:?}", validation.disallowed_commands),
            "validation_gap",
            "high",
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn push_task_fixture_checks(
    check: &TaskLevelCheck,
    diff: &CodingTaskDiffSummary,
    diff_text: &str,
    validation: &CodingTaskValidationSummary,
    review: &CodingTaskReviewSummary,
    context: &CodingTaskContextSummary,
    goal: &CodingTaskGoalSummary,
    artifacts: &EvalRunArtifacts,
    checks: &mut Vec<CodingTaskEvalCheckResult>,
) {
    for suffix in &check.expected_changed_files {
        let found = diff
            .changed_files
            .iter()
            .any(|path| path_matches_suffix(path, suffix));
        push_task_check(
            checks,
            format!("diff.changed_file.{suffix}"),
            found,
            format!("changedFiles={:?}", diff.changed_files),
            "implementation_bug",
            "critical",
        );
    }
    for suffix in &check.forbidden_changed_files {
        let found = diff
            .changed_files
            .iter()
            .any(|path| path_matches_suffix(path, suffix));
        push_task_check(
            checks,
            format!("diff.forbidden_file.{suffix}"),
            !found,
            format!("changedFiles={:?}", diff.changed_files),
            "scope_creep",
            "critical",
        );
    }
    for needle in &check.required_diff_contains {
        let found = diff_text.contains(needle);
        push_task_check(
            checks,
            format!("diff.contains.{}", compact_label(needle)),
            found,
            if found {
                "matched".to_string()
            } else {
                "required diff fragment missing".to_string()
            },
            "implementation_bug",
            "critical",
        );
    }
    for needle in &check.forbidden_diff_contains {
        let found = diff_text.contains(needle);
        push_task_check(
            checks,
            format!("diff.forbidden.{}", compact_label(needle)),
            !found,
            if found {
                "forbidden diff fragment present".to_string()
            } else {
                "not present".to_string()
            },
            "scope_creep",
            "critical",
        );
    }
    for expected in &check.expected_validation_commands {
        let found = validation
            .commands
            .iter()
            .any(|command| command == expected);
        push_task_check(
            checks,
            format!("validation.command.{expected}"),
            found,
            format!("commands={:?}", validation.commands),
            "validation_gap",
            "high",
        );
    }
    for forbidden in &check.forbidden_validation_commands {
        let found = validation
            .commands
            .iter()
            .any(|command| command == forbidden);
        push_task_check(
            checks,
            format!("validation.forbidden_command.{forbidden}"),
            !found,
            format!("commands={:?}", validation.commands),
            "validation_gap",
            "high",
        );
    }
    if let Some(max) = check.max_changed_files {
        push_task_check(
            checks,
            "diff.max_changed_files",
            diff.files_changed <= max,
            format!("{} changed file(s), max {max}", diff.files_changed),
            "scope_creep",
            "high",
        );
    }
    if let Some(require) = check.require_review {
        push_task_check(
            checks,
            "review.requested",
            review.requested == require,
            format!("review.requested={}, expected={require}", review.requested),
            "review_gap",
            "medium",
        );
    }
    if let Some(require) = check.require_verification {
        let requested = validation.command_count > 0;
        push_task_check(
            checks,
            "verification.requested",
            requested == require,
            format!("verification.requested={requested}, expected={require}"),
            "validation_gap",
            "high",
        );
    }
    if let Some(require) = check.require_context {
        push_task_check(
            checks,
            "context.requested",
            context.requested == require,
            format!(
                "context.requested={}, expected={require}",
                context.requested
            ),
            "context_miss",
            "medium",
        );
    }
    if let Some(require) = check.require_goal_evaluation {
        push_task_check(
            checks,
            "goal.evaluated",
            goal.evaluated == require,
            format!(
                "goal.evaluated={}, state={:?}, expectedEvaluation={require}",
                goal.evaluated, goal.state
            ),
            "reporting_issue",
            "medium",
        );
    }
    for expected in &check.required_context {
        let found = artifacts.context.as_ref().is_some_and(|snapshot| {
            snapshot
                .candidates
                .iter()
                .any(|candidate| candidate_matches(candidate, expected))
        });
        push_task_check(
            checks,
            format!("context.required.{}", expected.label()),
            found,
            if found {
                "matched".to_string()
            } else {
                artifacts
                    .context
                    .as_ref()
                    .map(|snapshot| summarize_candidates(&snapshot.candidates))
                    .unwrap_or_else(|| "context not requested".to_string())
            },
            "context_miss",
            "medium",
        );
    }
}

fn push_task_check(
    checks: &mut Vec<CodingTaskEvalCheckResult>,
    name: impl Into<String>,
    passed: bool,
    detail: impl Into<String>,
    category: impl Into<String>,
    severity: impl Into<String>,
) {
    checks.push(CodingTaskEvalCheckResult {
        name: name.into(),
        passed,
        detail: detail.into(),
        category: category.into(),
        severity: severity.into(),
    });
}

fn derive_task_outcome(checks: &[CodingTaskEvalCheckResult], score: f64) -> &'static str {
    if checks.is_empty() {
        return "blocked";
    }
    if checks
        .iter()
        .any(|check| !check.passed && check.severity == "critical")
    {
        "fail"
    } else if score >= 1.0 {
        "pass"
    } else if score >= 0.75 {
        "partial"
    } else {
        "fail"
    }
}

fn task_outcome_to_eval_status(outcome: &str) -> &'static str {
    match outcome {
        "pass" => "passed",
        "blocked" => "blocked",
        _ => "failed",
    }
}

fn compact_label(value: &str) -> String {
    let mut out = sanitize_name(value);
    if out.len() > 32 {
        out.truncate(32);
        out = out.trim_matches('-').to_string();
    }
    if out.is_empty() {
        "fragment".to_string()
    } else {
        out
    }
}

fn check_fixture(fixture: &CodingEvalFixture, artifacts: &EvalRunArtifacts) -> FixtureReport {
    let mut report = FixtureReport {
        name: fixture.name.clone(),
        metrics: EvalMetrics::default(),
        outcomes: Vec::new(),
        execution: artifacts.execution.clone(),
        task: artifacts.task.clone(),
    };
    if artifacts.execution.is_some() || fixture.checks.execution.is_some() {
        check_execution(&mut report, artifacts, fixture.checks.execution.as_ref());
    }
    if artifacts.task.is_some() || fixture.checks.task.is_some() {
        check_task(&mut report, artifacts, fixture.checks.task.as_ref());
    }
    if let Some(check) = &fixture.checks.workflow {
        check_workflow(&mut report, artifacts, check);
    }
    if let Some(check) = &fixture.checks.review {
        check_review(&mut report, artifacts, check);
    }
    if let Some(check) = &fixture.checks.verification {
        check_verification(&mut report, artifacts, check);
    }
    if let Some(check) = &fixture.checks.context {
        check_context(&mut report, artifacts, check);
    }
    if let Some(check) = &fixture.checks.improvement {
        check_improvement(&mut report, artifacts, check);
    }
    report
}

fn check_workflow(report: &mut FixtureReport, artifacts: &EvalRunArtifacts, check: &WorkflowCheck) {
    let Some(result) = artifacts.workflow.as_ref() else {
        push_check(
            report,
            "workflow.snapshot",
            false,
            "workflow run was not requested",
        );
        return;
    };
    let expected_state = check.expected_state.as_deref().unwrap_or("completed");
    push_check(
        report,
        "workflow.state",
        result.snapshot.run.state.as_str() == expected_state,
        format!(
            "state={}, expected={expected_state}",
            result.snapshot.run.state.as_str()
        ),
    );
    if let Some(expected) = check.expected_blocked_reason.as_deref() {
        push_check(
            report,
            "workflow.blocked_reason",
            result.snapshot.run.blocked_reason.as_deref() == Some(expected),
            format!(
                "blockedReason={:?}, expected={expected}",
                result.snapshot.run.blocked_reason
            ),
        );
    }

    if !check.expected_op_types.is_empty() {
        let actual = result
            .snapshot
            .ops
            .iter()
            .map(|op| op.op_type.clone())
            .collect::<Vec<_>>();
        push_check(
            report,
            "workflow.op_types",
            actual == check.expected_op_types,
            format!("actual={actual:?}, expected={:?}", check.expected_op_types),
        );
    }

    if let Some(expect) = check.expect_review_ok {
        let actual = result
            .output
            .as_ref()
            .and_then(|output| output.get("reviewOk"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        push_check(
            report,
            "workflow.review_ok",
            actual == expect,
            format!("reviewOk={actual}, expected={expect}"),
        );
    }

    if let Some(min) = check.min_finding_count {
        let actual = result
            .output
            .as_ref()
            .and_then(|output| output.get("findingCount"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        push_check(
            report,
            "workflow.min_finding_count",
            actual >= min,
            format!("findingCount={actual}, min={min}"),
        );
    }

    for expected in &check.expected_commands {
        let found = result
            .output
            .as_ref()
            .and_then(|output| output.get("commands"))
            .and_then(Value::as_array)
            .is_some_and(|commands| {
                commands
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|command| command == expected)
            });
        push_check(
            report,
            format!("workflow.command.{expected}"),
            found,
            if found {
                "matched".to_string()
            } else {
                format!("output={:?}", result.output)
            },
        );
    }

    for expected in &check.expected_goal_relations {
        let found = artifacts
            .goal_evidence_relations
            .iter()
            .any(|relation| relation == expected);
        push_check(
            report,
            format!("workflow.goal_relation.{expected}"),
            found,
            if found {
                "matched".to_string()
            } else {
                format!("relations={:?}", artifacts.goal_evidence_relations)
            },
        );
    }
}

fn check_execution(
    report: &mut FixtureReport,
    artifacts: &EvalRunArtifacts,
    check: Option<&AgentExecutionCheck>,
) {
    let Some(execution) = artifacts.execution.as_ref() else {
        push_check(report, "execution.report", false, "execution was not run");
        return;
    };
    report.metrics.execution_status = Some(execution.status.clone());
    report.metrics.execution_mode = Some(execution.mode.clone());
    report.metrics.execution_changed_files = execution.changed_files.clone();
    report.metrics.execution_tool_calls = execution.tool_calls.clone();

    if let Some(check) = check {
        if let Some(expected) = check.expected_mode.as_deref() {
            push_check(
                report,
                "execution.mode",
                execution.mode == expected,
                format!("mode={}, expected={expected}", execution.mode),
            );
        }
        if let Some(expected) = check.expected_status.as_deref() {
            push_check(
                report,
                "execution.status",
                execution.status == expected,
                format!("status={}, expected={expected}", execution.status),
            );
        }
        if let Some(require) = check.require_turn {
            let has_turn = execution.turn_id.is_some();
            push_check(
                report,
                "execution.turn",
                has_turn == require,
                format!("turnPresent={has_turn}, expected={require}"),
            );
        }
        for suffix in &check.expected_changed_files {
            let found = execution
                .changed_files
                .iter()
                .any(|path| path_matches_suffix(path, suffix));
            push_check(
                report,
                format!("execution.changed_file.{suffix}"),
                found,
                format!("changedFiles={:?}", execution.changed_files),
            );
        }
        for suffix in &check.forbidden_changed_files {
            let found = execution
                .changed_files
                .iter()
                .any(|path| path_matches_suffix(path, suffix));
            push_check(
                report,
                format!("execution.forbidden_file.{suffix}"),
                !found,
                format!("changedFiles={:?}", execution.changed_files),
            );
        }
        if let Some(min) = check.min_tool_calls {
            push_check(
                report,
                "execution.min_tool_calls",
                execution.tool_calls.len() >= min,
                format!("toolCalls={:?}, min={min}", execution.tool_calls),
            );
        }
        for expected in &check.expected_tool_calls {
            let found = execution.tool_calls.iter().any(|tool| tool == expected);
            push_check(
                report,
                format!("execution.tool_call.{expected}"),
                found,
                format!("toolCalls={:?}", execution.tool_calls),
            );
        }
        for needle in &check.response_contains {
            let found = execution
                .response
                .as_deref()
                .is_some_and(|response| response.contains(needle));
            push_check(
                report,
                format!("execution.response.{}", compact_label(needle)),
                found,
                if found {
                    "matched".to_string()
                } else {
                    format!("response={:?}", execution.response)
                },
            );
        }
        for needle in &check.error_contains {
            let found = execution
                .error
                .as_deref()
                .is_some_and(|error| error.contains(needle));
            push_check(
                report,
                format!("execution.error.{}", compact_label(needle)),
                found,
                if found {
                    "matched".to_string()
                } else {
                    format!("error={:?}", execution.error)
                },
            );
        }
    }
}

fn check_task(
    report: &mut FixtureReport,
    artifacts: &EvalRunArtifacts,
    check: Option<&TaskLevelCheck>,
) {
    let Some(task) = artifacts.task.as_ref() else {
        push_check(
            report,
            "task.report",
            false,
            "task-level eval was not produced",
        );
        return;
    };
    report.metrics.task_outcome = Some(task.outcome.clone());
    report.metrics.task_score = Some(task.score);
    report.metrics.task_failure_category = task.failure_category.clone();
    report.metrics.task_changed_files = task.diff.changed_files.clone();
    report.metrics.task_constraint_violations = task
        .checks
        .iter()
        .filter(|check| {
            !check.passed && matches!(check.category.as_str(), "scope_creep" | "policy_violation")
        })
        .count();
    for item in &task.checks {
        push_check(
            report,
            format!("task.{}", item.name),
            item.passed,
            format!("{} [{}:{}]", item.detail, item.category, item.severity),
        );
    }
    if let Some(check) = check {
        if let Some(expected) = check.expected_outcome.as_deref() {
            push_check(
                report,
                "task.expected_outcome",
                task.outcome == expected,
                format!("outcome={}, expected={expected}", task.outcome),
            );
        }
        if let Some(min) = check.min_score {
            push_check(
                report,
                "task.min_score",
                task.score + f64::EPSILON >= min,
                format!("{:.3} >= {min:.3}", task.score),
            );
        }
    }
}

fn check_context(report: &mut FixtureReport, artifacts: &EvalRunArtifacts, check: &ContextCheck) {
    let Some(snapshot) = artifacts.context.as_ref() else {
        push_check(
            report,
            "context.snapshot",
            false,
            "context run was not requested",
        );
        return;
    };
    let candidates = &snapshot.candidates;
    if let Some(max) = check.max_candidates {
        push_check(
            report,
            "context.max_candidates",
            candidates.len() <= max,
            format!("{} candidate(s), max {}", candidates.len(), max),
        );
    }

    let mut matched = HashSet::<usize>::new();
    let mut matched_critical = 0usize;
    for expected in &check.critical {
        let found = candidates
            .iter()
            .enumerate()
            .find(|(_, candidate)| candidate_matches(candidate, expected));
        if let Some((idx, _)) = found {
            matched.insert(idx);
            matched_critical += 1;
            push_check(
                report,
                format!("context.critical.{}", expected.label()),
                true,
                "matched".to_string(),
            );
        } else {
            push_check(
                report,
                format!("context.critical.{}", expected.label()),
                false,
                format!("not found among {}", summarize_candidates(candidates)),
            );
        }
    }

    if !check.critical.is_empty() {
        let recall = matched_critical as f64 / check.critical.len() as f64;
        report.metrics.critical_context_recall = Some(recall);
        if let Some(min) = check.min_critical_recall {
            push_check(
                report,
                "context.critical_recall",
                recall + f64::EPSILON >= min,
                format!("{recall:.3} >= {min:.3}"),
            );
        }
    }

    if !candidates.is_empty() && !check.critical.is_empty() {
        let precision = matched.len() as f64 / candidates.len() as f64;
        report.metrics.context_precision = Some(precision);
        if let Some(min) = check.min_precision {
            push_check(
                report,
                "context.precision",
                precision + f64::EPSILON >= min,
                format!("{precision:.3} >= {min:.3}"),
            );
        }
    }

    for suffix in &check.expect_action_paths {
        let found = candidates.iter().any(|candidate| {
            focus_paths(candidate)
                .iter()
                .any(|path| path_matches_suffix(path, suffix))
        });
        push_check(
            report,
            format!("context.action_path.{suffix}"),
            found,
            if found {
                "matched".to_string()
            } else {
                "missing action focus path".to_string()
            },
        );
    }
}

fn check_review(report: &mut FixtureReport, artifacts: &EvalRunArtifacts, check: &ReviewCheck) {
    let Some(snapshot) = artifacts.review.as_ref() else {
        push_check(
            report,
            "review.snapshot",
            false,
            "review run was not requested",
        );
        return;
    };
    let findings = &snapshot.findings;
    report.metrics.review_findings = Some(findings.len());

    if let Some(min) = check.min_findings {
        push_check(
            report,
            "review.min_findings",
            findings.len() >= min,
            format!("{} finding(s), min {}", findings.len(), min),
        );
    }
    if let Some(max) = check.max_findings {
        push_check(
            report,
            "review.max_findings",
            findings.len() <= max,
            format!("{} finding(s), max {}", findings.len(), max),
        );
    }
    if let Some(expect) = check.expect_focused {
        let focused = snapshot
            .run
            .stats
            .get("focused")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        push_check(
            report,
            "review.focused",
            focused == expect,
            format!("focused={focused}, expected={expect}"),
        );
    }
    for profile in &check.expected_profiles {
        let found = snapshot
            .run
            .stats
            .get("profiles")
            .and_then(Value::as_array)
            .is_some_and(|profiles| {
                profiles
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|p| p == profile)
            });
        push_check(
            report,
            format!("review.profile.{profile}"),
            found,
            if found {
                "matched".to_string()
            } else {
                format!("stats={}", snapshot.run.stats)
            },
        );
    }
    if let Some(expect) = check.expect_ide_context {
        let present = snapshot
            .run
            .stats
            .get("ideContext")
            .and_then(|value| value.get("present"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        push_check(
            report,
            "review.ide_context",
            present == expect,
            format!("ideContext.present={present}, expected={expect}"),
        );
    }
    for title in &check.expected_titles {
        let found = findings
            .iter()
            .any(|finding| contains_ci(&finding.title, title));
        push_check(
            report,
            format!("review.title.{title}"),
            found,
            if found {
                "matched".to_string()
            } else {
                summarize_findings(findings)
            },
        );
    }
    for category in &check.expected_categories {
        let found = findings.iter().any(|finding| finding.category == *category);
        push_check(
            report,
            format!("review.category.{category}"),
            found,
            if found {
                "matched".to_string()
            } else {
                summarize_findings(findings)
            },
        );
    }
    for suffix in &check.expected_files {
        let found = findings
            .iter()
            .any(|finding| path_matches_suffix(&finding.file, suffix));
        push_check(
            report,
            format!("review.file.{suffix}"),
            found,
            if found {
                "matched".to_string()
            } else {
                summarize_findings(findings)
            },
        );
    }
    for suffix in &check.forbidden_files {
        let found = findings
            .iter()
            .any(|finding| path_matches_suffix(&finding.file, suffix));
        push_check(
            report,
            format!("review.forbidden_file.{suffix}"),
            !found,
            if found {
                summarize_findings(findings)
            } else {
                "not present".to_string()
            },
        );
    }
}

fn check_verification(
    report: &mut FixtureReport,
    artifacts: &EvalRunArtifacts,
    check: &VerificationCheck,
) {
    let Some(snapshot) = artifacts.verification.as_ref() else {
        push_check(
            report,
            "verification.snapshot",
            false,
            "verification plan was not requested",
        );
        return;
    };
    let commands = snapshot
        .steps
        .iter()
        .map(|step| step.command.clone())
        .collect::<Vec<_>>();
    report.metrics.verification_commands = commands.clone();

    for expected in &check.expected_commands {
        let found = commands.iter().any(|command| command == expected);
        push_check(
            report,
            format!("verification.command.{expected}"),
            found,
            if found {
                "matched".to_string()
            } else {
                format!("commands={commands:?}")
            },
        );
    }
    for forbidden in &check.forbidden_commands {
        let found = commands.iter().any(|command| command == forbidden);
        push_check(
            report,
            format!("verification.forbidden_command.{forbidden}"),
            !found,
            if found {
                format!("commands={commands:?}")
            } else {
                "not present".to_string()
            },
        );
    }
    if let Some(expect) = check.expect_focused {
        let focused = snapshot
            .run
            .stats
            .get("focused")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        push_check(
            report,
            "verification.focused",
            focused == expect,
            format!("focused={focused}, expected={expect}"),
        );
    }
    let focus_paths = snapshot
        .run
        .stats
        .get("focusPaths")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for suffix in &check.expected_focus_paths {
        let found = focus_paths
            .iter()
            .filter_map(Value::as_str)
            .any(|path| path_matches_suffix(path, suffix));
        push_check(
            report,
            format!("verification.focus_path.{suffix}"),
            found,
            if found {
                "matched".to_string()
            } else {
                format!("focusPaths={focus_paths:?}")
            },
        );
    }
}

fn check_improvement(
    report: &mut FixtureReport,
    artifacts: &EvalRunArtifacts,
    check: &ImprovementCheck,
) {
    let Some(snapshot) = artifacts.improvement.as_ref() else {
        push_check(
            report,
            "improvement.snapshot",
            false,
            "coding improvement report was not requested",
        );
        return;
    };

    if let Some(expected) = check.expected_scope.as_deref() {
        push_check(
            report,
            "improvement.scope",
            snapshot.scope == expected,
            format!("scope={}, expected={expected}", snapshot.scope),
        );
    }

    if let Some(min) = check.min_failures {
        push_check(
            report,
            "improvement.min_failures",
            snapshot.failures.len() >= min,
            format!("{} failure bucket(s), min {min}", snapshot.failures.len()),
        );
    }

    for category in &check.expected_failure_categories {
        let found = snapshot
            .failures
            .iter()
            .any(|failure| failure.category == *category);
        push_check(
            report,
            format!("improvement.failure.{category}"),
            found,
            if found {
                "matched".to_string()
            } else {
                format!(
                    "failures={:?}",
                    snapshot
                        .failures
                        .iter()
                        .map(|failure| failure.category.as_str())
                        .collect::<Vec<_>>()
                )
            },
        );
    }

    if let Some(min) = check.min_proposals {
        push_check(
            report,
            "improvement.min_proposals",
            snapshot.proposals.len() >= min,
            format!("{} proposal(s), min {min}", snapshot.proposals.len()),
        );
    }

    if let Some(min) = check.min_inserted_proposals {
        let inserted = artifacts
            .improvement_proposals
            .as_ref()
            .map(|result| result.inserted)
            .unwrap_or(0);
        push_check(
            report,
            "improvement.min_inserted_proposals",
            inserted >= min,
            format!("{inserted} inserted proposal(s), min {min}"),
        );
    }

    for kind in &check.expected_proposal_kinds {
        let found = snapshot
            .proposals
            .iter()
            .any(|proposal| proposal.kind == *kind);
        push_check(
            report,
            format!("improvement.proposal_kind.{kind}"),
            found,
            if found {
                "matched".to_string()
            } else {
                format!(
                    "proposalKinds={:?}",
                    snapshot
                        .proposals
                        .iter()
                        .map(|proposal| proposal.kind.as_str())
                        .collect::<Vec<_>>()
                )
            },
        );
    }

    if let Some(expect) = check.expect_draft_only {
        let draft_only = snapshot
            .proposals
            .iter()
            .all(|proposal| proposal.status == "draft");
        push_check(
            report,
            "improvement.draft_only",
            draft_only == expect,
            format!("draftOnly={draft_only}, expected={expect}"),
        );
    }

    if let Some(min) = check.min_eval_runs {
        push_check(
            report,
            "improvement.min_eval_runs",
            snapshot.eval.runs >= min,
            format!("{} eval run(s), min {min}", snapshot.eval.runs),
        );
    }

    if let Some(expected) = check.expect_eval_success_rate {
        let actual = snapshot.eval.success_rate.unwrap_or(-1.0);
        push_check(
            report,
            "improvement.eval_success_rate",
            (actual - expected).abs() <= 0.001,
            format!("{actual:.3}, expected {expected:.3}"),
        );
    }

    if let Some(min) = check.min_repair_loop_blocked {
        push_check(
            report,
            "improvement.repair_loop_blocked",
            snapshot.repair_loop.blocked >= min,
            format!(
                "{} blocked repair loop run(s), min {min}",
                snapshot.repair_loop.blocked
            ),
        );
    }

    if let Some(min) = check.min_retros {
        push_check(
            report,
            "improvement.min_retros",
            snapshot.retros.len() >= min,
            format!("{} retro(s), min {min}", snapshot.retros.len()),
        );
    }
    if let Some(min) = check.min_retro_recommendations {
        push_check(
            report,
            "improvement.min_retro_recommendations",
            snapshot.retro.recommendations >= min,
            format!(
                "{} recommendation(s), min {min}",
                snapshot.retro.recommendations
            ),
        );
    }

    if check.expected_applied_status.is_some()
        || check.expected_applied_kind.is_some()
        || check.min_applied_artifacts.is_some()
        || check.expected_action_target_contains.is_some()
    {
        let Some(result) = artifacts.improvement_apply.as_ref() else {
            push_check(
                report,
                "improvement.apply",
                false,
                "applyFirstProposal did not produce an apply result",
            );
            return;
        };

        if let Some(expected) = check.expected_applied_status.as_deref() {
            push_check(
                report,
                "improvement.applied_status",
                result.proposal.status == expected,
                format!("status={}, expected={expected}", result.proposal.status),
            );
        }
        if let Some(expected) = check.expected_applied_kind.as_deref() {
            push_check(
                report,
                "improvement.applied_kind",
                result.proposal.kind == expected,
                format!("kind={}, expected={expected}", result.proposal.kind),
            );
        }
        if let Some(min) = check.min_applied_artifacts {
            push_check(
                report,
                "improvement.min_applied_artifacts",
                result.artifacts.len() >= min,
                format!("{} artifact(s), min {min}", result.artifacts.len()),
            );
        }
        if let Some(needle) = check.expected_action_target_contains.as_deref() {
            let found = result
                .artifacts
                .iter()
                .any(|artifact| artifact.path.contains(needle))
                || result
                    .plan
                    .steps
                    .iter()
                    .any(|step| step.target_path.contains(needle));
            push_check(
                report,
                "improvement.action_target",
                found,
                if found {
                    "matched".to_string()
                } else {
                    format!(
                        "targets={:?}",
                        result
                            .plan
                            .steps
                            .iter()
                            .map(|step| step.target_path.as_str())
                            .collect::<Vec<_>>()
                    )
                },
            );
        }
    }

    if check.expected_promoted_status.is_some()
        || check.min_promoted_artifacts.is_some()
        || check.expected_promotion_target_contains.is_some()
    {
        let Some(result) = artifacts.improvement_promotion.as_ref() else {
            push_check(
                report,
                "improvement.promotion",
                false,
                "promoteAppliedProposal did not produce a promotion result",
            );
            return;
        };

        if let Some(expected) = check.expected_promoted_status.as_deref() {
            push_check(
                report,
                "improvement.promoted_status",
                result.proposal.status == expected,
                format!("status={}, expected={expected}", result.proposal.status),
            );
        }
        if let Some(min) = check.min_promoted_artifacts {
            push_check(
                report,
                "improvement.min_promoted_artifacts",
                result.artifacts.len() >= min,
                format!("{} artifact(s), min {min}", result.artifacts.len()),
            );
        }
        if let Some(needle) = check.expected_promotion_target_contains.as_deref() {
            let found = result
                .artifacts
                .iter()
                .any(|artifact| artifact.path.contains(needle))
                || result
                    .plan
                    .steps
                    .iter()
                    .any(|step| step.target_path.contains(needle));
            push_check(
                report,
                "improvement.promotion_target",
                found,
                if found {
                    "matched".to_string()
                } else {
                    format!(
                        "targets={:?}",
                        result
                            .plan
                            .steps
                            .iter()
                            .map(|step| step.target_path.as_str())
                            .collect::<Vec<_>>()
                    )
                },
            );
        }
    }
}

const GOLD_TASK_PACK_ID: &str = "phase5-gold-task-pack";
const GOLD_TASK_SOURCE_DOC: &str = "docs/roadmap/coding-eval-tasks.md";

#[derive(Debug, Clone)]
struct GoldTaskCase {
    id: String,
    task_type: String,
    title: String,
    status: String,
    source: String,
    execution_mode: String,
    prompt: String,
    expected_behavior: Vec<String>,
    forbidden_behavior: Vec<String>,
    likely_files: Vec<String>,
    expected_artifacts: Vec<String>,
    requires_seeded_state: bool,
    allowed_validation: Vec<String>,
    success_criteria: Vec<String>,
    failure_notes: Vec<String>,
    automation: Option<GoldTaskAutomation>,
}

#[derive(Debug, Clone)]
struct GoldTaskAutomation {
    fixture_name: String,
    baseline_path: String,
    baseline_text: String,
    candidate_text: String,
    support_files: Vec<FileFixture>,
    extra_changes: Vec<FileFixture>,
    required_diff_contains: Vec<String>,
    expected_validation_commands: Vec<String>,
    expected_verification_titles: Vec<String>,
    forbidden_validation_commands: Vec<String>,
    forbidden_changed_files: Vec<String>,
    max_review_findings: Option<usize>,
    context_query: String,
    goal_objective: String,
    goal_completion_criteria: String,
    completed_task: String,
}

impl GoldTaskAutomation {
    fn with_support_file(mut self, path: &str, text: &str) -> Self {
        self.support_files.push(FileFixture {
            path: path.to_string(),
            text: text.to_string(),
        });
        self
    }

    fn with_file_change(mut self, path: &str, baseline_text: &str, candidate_text: &str) -> Self {
        self.support_files.push(FileFixture {
            path: path.to_string(),
            text: baseline_text.to_string(),
        });
        self.extra_changes.push(FileFixture {
            path: path.to_string(),
            text: candidate_text.to_string(),
        });
        self
    }

    fn with_validation(mut self, commands: &[&str], titles: &[&str]) -> Self {
        self.expected_validation_commands = strings(commands);
        self.expected_verification_titles = strings(titles);
        self
    }

    fn with_review_max_findings(mut self, max: usize) -> Self {
        self.max_review_findings = Some(max);
        self
    }
}

impl GoldTaskCase {
    fn summary(&self) -> GoldTaskCaseSummary {
        GoldTaskCaseSummary {
            id: self.id.clone(),
            task_type: self.task_type.clone(),
            title: self.title.clone(),
            status: self.status.clone(),
            source: self.source.clone(),
            execution_mode: self.execution_mode.clone(),
            automation_status: if self.automation.is_some() {
                "automated".to_string()
            } else {
                "manual".to_string()
            },
            fixture_name: self
                .automation
                .as_ref()
                .map(|automation| automation.fixture_name.clone()),
            expected_artifacts: self.expected_artifacts.clone(),
            requires_seeded_state: self.requires_seeded_state,
            likely_files: self.likely_files.clone(),
            allowed_validation: self.allowed_validation.clone(),
            success_criteria: self.success_criteria.clone(),
        }
    }
}

fn summarize_gold_task_pack(cases: &[GoldTaskCase]) -> GoldTaskPackSummary {
    GoldTaskPackSummary {
        pack_id: GOLD_TASK_PACK_ID.to_string(),
        source_doc: GOLD_TASK_SOURCE_DOC.to_string(),
        total_cases: cases.len(),
        automated_cases: cases
            .iter()
            .filter(|case| case.automation.is_some())
            .count(),
        active_cases: cases.iter().filter(|case| case.status == "active").count(),
        cases: cases.iter().map(GoldTaskCase::summary).collect(),
    }
}

fn select_gold_task_cases(
    cases: &[GoldTaskCase],
    input: &GoldTaskPackRunInput,
) -> Vec<GoldTaskCase> {
    let id_filter = input
        .ids
        .iter()
        .map(|id| id.trim().to_ascii_uppercase())
        .filter(|id| !id.is_empty())
        .collect::<HashSet<_>>();
    let status_filter = input
        .statuses
        .iter()
        .map(|status| status.trim().to_ascii_lowercase())
        .filter(|status| !status.is_empty())
        .collect::<HashSet<_>>();
    let type_filter = input
        .task_types
        .iter()
        .map(|task_type| task_type.trim().to_ascii_lowercase())
        .filter(|task_type| !task_type.is_empty())
        .collect::<HashSet<_>>();

    let mut selected = cases
        .iter()
        .filter(|case| {
            (id_filter.is_empty() || id_filter.contains(&case.id.to_ascii_uppercase()))
                && if status_filter.is_empty() {
                    !id_filter.is_empty() || case.status == "active"
                } else {
                    status_filter.contains(&case.status.to_ascii_lowercase())
                }
                && (type_filter.is_empty()
                    || type_filter.contains(&case.task_type.to_ascii_lowercase()))
                && (input.include_unautomated || !id_filter.is_empty() || case.automation.is_some())
        })
        .cloned()
        .collect::<Vec<_>>();
    if let Some(max) = input.max_tasks {
        selected.truncate(max);
    }
    selected
}

fn materialize_gold_task_fixture(
    case: &GoldTaskCase,
    automation: GoldTaskAutomation,
    input: &GoldTaskPackRunInput,
) -> CodingEvalFixture {
    let candidate_path = automation.baseline_path.clone();
    let fixture_name = automation.fixture_name.clone();
    let forbidden_files = if automation.forbidden_changed_files.is_empty() {
        vec!["src/lib.rs".to_string(), "Cargo.toml".to_string()]
    } else {
        automation.forbidden_changed_files.clone()
    };
    let mut expected_changed_files = vec![candidate_path.clone()];
    expected_changed_files.extend(
        automation
            .extra_changes
            .iter()
            .map(|file| file.path.clone()),
    );
    let expected_changed_file_count = expected_changed_files.len();
    let expected_validation_commands = if automation.expected_validation_commands.is_empty() {
        vec!["git diff --check".to_string()]
    } else {
        automation.expected_validation_commands.clone()
    };
    let forbidden_validation_commands = if automation.forbidden_validation_commands.is_empty() {
        vec!["cargo test --workspace".to_string()]
    } else {
        automation.forbidden_validation_commands.clone()
    };
    let mut required_context = vec![CandidateExpectation {
        kind: Some("file".to_string()),
        path_suffix: Some(candidate_path.clone()),
        ..Default::default()
    }];
    let verification_titles = if automation.expected_verification_titles.is_empty() {
        vec!["Check diff whitespace".to_string()]
    } else {
        automation.expected_verification_titles.clone()
    };
    for title in verification_titles {
        required_context.push(CandidateExpectation {
            kind: Some("verification_step".to_string()),
            title_contains: Some(title),
            ..Default::default()
        });
    }
    let mut files = automation.support_files.clone();
    files.push(FileFixture {
        path: candidate_path.clone(),
        text: automation.baseline_text,
    });
    let mut changes = automation.extra_changes.clone();
    changes.push(FileFixture {
        path: candidate_path.clone(),
        text: automation.candidate_text,
    });

    CodingEvalFixture {
        name: fixture_name,
        description: format!(
            "Gold task {} materialized from {} for deterministic replay.",
            case.id, GOLD_TASK_SOURCE_DOC
        ),
        task: Some(CodingTaskEvalSpec {
            id: case.id.clone(),
            task_type: case.task_type.clone(),
            title: case.title.clone(),
            source: case.source.clone(),
            prompt: case.prompt.clone(),
            execution_mode: case.execution_mode.clone(),
            expected_behavior: case.expected_behavior.clone(),
            forbidden_behavior: case.forbidden_behavior.clone(),
            likely_files: case.likely_files.clone(),
            expected_artifacts: case.expected_artifacts.clone(),
            requires_seeded_state: case.requires_seeded_state,
            allowed_validation: case.allowed_validation.clone(),
            success_criteria: case.success_criteria.clone(),
            failure_notes: case.failure_notes.clone(),
        }),
        repo: RepoFixture { files, changes },
        setup: FixtureSetup {
            goal: Some(GoalFixture {
                objective: automation.goal_objective,
                completion_criteria: automation.goal_completion_criteria,
            }),
            tasks: vec![TaskFixture {
                content: automation.completed_task,
                active_form: None,
                status: "completed".to_string(),
            }],
            workflow: None,
        },
        runs: FixtureRuns {
            execution: Some(AgentExecutionEvalRun {
                mode: "fixture_patch".to_string(),
                ..Default::default()
            }),
            review: Some(ReviewEvalRun::default()),
            verification: Some(VerificationEvalRun::default()),
            context: Some(ContextEvalRun {
                query: Some(automation.context_query),
                limit: Some(12),
                ..Default::default()
            }),
            task: Some(TaskLevelEvalRun {
                record_eval_run: input.record_eval_runs,
                evaluate_goal: input.evaluate_goal,
            }),
            ..Default::default()
        },
        checks: FixtureChecks {
            execution: Some(AgentExecutionCheck {
                expected_mode: Some("fixture_patch".to_string()),
                expected_status: Some("completed".to_string()),
                expected_changed_files: expected_changed_files.clone(),
                forbidden_changed_files: forbidden_files.clone(),
                require_turn: Some(false),
                response_contains: vec!["fixture patch applied".to_string()],
                ..Default::default()
            }),
            review: Some(ReviewCheck {
                max_findings: automation.max_review_findings,
                expect_focused: Some(false),
                ..Default::default()
            }),
            verification: Some(VerificationCheck {
                expected_commands: expected_validation_commands.clone(),
                forbidden_commands: forbidden_validation_commands.clone(),
                expect_focused: Some(false),
                ..Default::default()
            }),
            context: Some(ContextCheck {
                critical: required_context.clone(),
                min_critical_recall: Some(1.0),
                min_precision: Some(0.25),
                max_candidates: Some(12),
                expect_action_paths: vec![candidate_path.clone()],
            }),
            task: Some(TaskLevelCheck {
                expected_outcome: Some("pass".to_string()),
                min_score: Some(1.0),
                expected_changed_files,
                forbidden_changed_files: forbidden_files,
                required_diff_contains: automation.required_diff_contains,
                forbidden_diff_contains: vec!["println!".to_string(), "TODO:".to_string()],
                expected_validation_commands: expected_validation_commands.clone(),
                forbidden_validation_commands,
                max_changed_files: Some(expected_changed_file_count),
                require_review: Some(true),
                require_verification: Some(true),
                require_context: Some(true),
                require_goal_evaluation: Some(input.evaluate_goal),
                required_context,
            }),
            ..Default::default()
        },
    }
}

fn gold_task_cases() -> Vec<GoldTaskCase> {
    vec![
        gold_task_case(
            "CE-BUG-001",
            "bugfix",
            "修复 tool_search select 查询大小写与空格容错",
            "active",
            "synthetic",
            "fixture_patch",
            "Fix tool_search select parsing so select: Read, edit tolerates whitespace and case differences.",
            &[
                "select entries are trimmed",
                "select entries are normalized case-insensitively",
                "ordinary keyword search behavior is unchanged",
            ],
            &["do not rewrite tool_search", "do not alter permission visibility"],
            &["crates/ha-core/src/tools/tool_search.rs"],
            &["diff", "validation"],
            false,
            &["cargo check -p ha-core --locked"],
            &["select query parsing trims and normalizes individual tool names"],
            &["only trimming the whole query leaves comma-separated entries broken"],
            Some(
                gold_task_automation(
                    "gold_task_ce_bug_001_tool_search_select",
                    "crates/ha-core/src/tools/tool_search.rs",
                    "pub fn parse_select_query(query: &str) -> Vec<String> {\n    query\n        .strip_prefix(\"select:\")\n        .map(|rest| rest.split(',').map(str::to_string).collect())\n        .unwrap_or_default()\n}\n",
                    "pub fn parse_select_query(query: &str) -> Vec<String> {\n    query\n        .strip_prefix(\"select:\")\n        .map(|rest| {\n            rest.split(',')\n                .map(|name| name.trim().to_ascii_lowercase())\n                .filter(|name| !name.is_empty())\n                .collect()\n        })\n        .unwrap_or_default()\n}\n",
                    &["name.trim().to_ascii_lowercase()", "filter(|name| !name.is_empty())"],
                    "tool_search select trim case insensitive",
                    "Fix tool_search select parsing",
                    "select parsing handles whitespace and case differences without changing ordinary search",
                    "Fix CE-BUG-001 tool_search select parsing",
                )
                .with_support_file("crates/ha-core/Cargo.toml", "[package]\nname = \"ha-core\"\n")
                .with_validation(
                    &["cargo check -p ha-core --locked"],
                    &["Check Rust crate ha-core"],
                )
                .with_review_max_findings(2),
            ),
        ),
        gold_task_case(
            "CE-BUG-002",
            "bugfix",
            "修复 Plan quality 文案误导导致执行期仍修改 plan",
            "active",
            "synthetic",
            "fixture_patch",
            "Adjust Plan Mode execution guidance so the model treats the plan as frozen and tracks progress through tasks.",
            &[
                "execution guidance says the plan is frozen",
                "progress updates go through task_create/task_update",
                "Plan Mode state machine is unchanged",
            ],
            &["do not reintroduce update_plan_step", "do not write task status back to plan.md"],
            &["crates/ha-core/src/plan/constants.rs"],
            &["diff", "validation"],
            false,
            &["cargo check -p ha-core --locked"],
            &["execution guidance points to task APIs instead of plan mutation"],
            &["editing plan content during execution violates the frozen-plan contract"],
            Some(
                gold_task_automation(
                    "gold_task_ce_bug_002_plan_execution_guidance",
                    "crates/ha-core/src/plan/constants.rs",
                    "pub const EXECUTION_GUIDANCE: &str = \"Keep the plan updated as work progresses.\";\n",
                    "pub const EXECUTION_GUIDANCE: &str = \"The approved plan is frozen during execution. Track progress with task_create and task_update, and only revise the plan after returning to planning.\";\n",
                    &["approved plan is frozen", "task_create and task_update"],
                    "Plan Mode frozen execution task progress",
                    "Clarify Plan execution guidance",
                    "Execution guidance makes plan/task separation explicit",
                    "Fix CE-BUG-002 Plan execution guidance",
                )
                .with_support_file("crates/ha-core/Cargo.toml", "[package]\nname = \"ha-core\"\n")
                .with_validation(
                    &["cargo check -p ha-core --locked"],
                    &["Check Rust crate ha-core"],
                )
                .with_review_max_findings(2),
            ),
        ),
        gold_task_case(
            "CE-BUG-003",
            "bugfix",
            "修复文件预览鉴权说明遗漏 HTTP by-path 场景",
            "active",
            "real_contract",
            "design",
            "Clarify HTTP preview-by-path authorization without weakening remote file access restrictions.",
            &[
                "HTTP by-path routes mention authorized_canonical_file_path",
                "desktop trust and remote HTTP authorization are separate",
                "arbitrary host paths remain forbidden remotely",
            ],
            &["do not loosen HTTP host path reads", "do not apply Tauri local trust to HTTP"],
            &["docs/architecture/file-operations.md"],
            &["design_notes"],
            false,
            &["git diff --check"],
            &["HTTP preview-by-path authorization is explicit"],
            &["remote arbitrary file reads must remain impossible"],
            Some(gold_task_automation(
                "gold_task_ce_bug_003_preview_by_path_auth",
                "docs/architecture/file-operations.md",
                "# File Operations\n\nPreview routes share one local-file policy.\n",
                "# File Operations\n\nPreview routes share one local-file policy.\n\n## HTTP preview-by-path authorization\n\nHTTP `read`, `extract`, and `by-path` preview routes must all pass through `authorized_canonical_file_path`. A path is allowed only when it was referenced by the session's tool messages or is inside the session working directory. Tauri desktop remains a local-trust surface; HTTP must not inherit that trust model or expose arbitrary host paths.\n",
                &["authorized_canonical_file_path", "HTTP must not inherit that trust model"],
                "HTTP preview by-path authorized_canonical_file_path",
                "Clarify preview-by-path authorization",
                "HTTP preview-by-path documentation separates owner trust from remote path authorization",
                "Fix CE-BUG-003 preview-by-path authorization docs",
            )),
        ),
        gold_task_case(
            "CE-BUG-004",
            "bugfix",
            "修复 async job 配置中 0 语义解释不一致",
            "active",
            "real_contract",
            "design",
            "Clarify async_tools zero-value semantics across concurrency limits and bounded-resource settings.",
            &[
                "max_concurrent_jobs zero means unlimited",
                "max_concurrent_jobs_per_session zero means unlimited",
                "bounded-resource knobs clamp zero to the floor",
            ],
            &["do not treat every zero as unlimited", "do not change defaults"],
            &["docs/architecture/background-jobs.md"],
            &["design_notes"],
            false,
            &["git diff --check"],
            &["0 semantics are consistent with AsyncToolsConfig clamping"],
            &["bounded-resource zero must not silently become unlimited"],
            Some(gold_task_automation(
                "gold_task_ce_bug_004_async_zero_semantics",
                "docs/architecture/background-jobs.md",
                "# Background Jobs\n\nAsync tool limits are configurable.\n",
                "# Background Jobs\n\nAsync tool limits are configurable.\n\n## Zero-value semantics\n\n`max_concurrent_jobs` and `max_concurrent_jobs_per_session` use `0` as explicit unlimited. Bounded-resource knobs such as `output_tail_bytes`, `max_queued_jobs`, `wakeup_max_delay_secs`, and `wakeup_max_pending_per_session` clamp `0` to their safe floor instead of treating it as unlimited.\n",
                &["use `0` as explicit unlimited", "clamp `0` to their safe floor"],
                "async_tools zero unlimited clamp floor",
                "Clarify async zero semantics",
                "Async job documentation distinguishes unlimited concurrency from bounded-resource clamps",
                "Fix CE-BUG-004 async zero semantics docs",
            )),
        ),
        gold_task_case(
            "CE-BUG-005",
            "bugfix",
            "修复 knowledge access 文档中 owner/agent 平面混写",
            "active",
            "real_contract",
            "design",
            "Separate Knowledge Base owner-plane trust from agent-plane effective access checks.",
            &[
                "owner plane sees all KBs through local/API-key trust",
                "agent note_* tools use effective_kb_access",
                "/api/knowledge/{kb}/files/* has no session fallback",
            ],
            &["do not weaken default deny", "do not let note_* bypass effective_kb_access"],
            &["docs/architecture/knowledge-base.md"],
            &["design_notes"],
            false,
            &["git diff --check"],
            &["owner and agent authorization planes are not mixed"],
            &["agent tool access must remain default deny"],
            Some(gold_task_automation(
                "gold_task_ce_bug_005_knowledge_access_planes",
                "docs/architecture/knowledge-base.md",
                "# Knowledge Base Access\n\nKnowledge APIs and agent tools share access rules.\n",
                "# Knowledge Base Access\n\nKnowledge APIs and agent tools use separate authorization planes.\n\n## Owner plane\n\nTauri and HTTP owner APIs are local/API-key trusted and can inspect registered knowledge bases without session attachment fallback.\n\n## Agent plane\n\nAgent-facing `note_*` tools must resolve access through `effective_kb_access`; incognito and unattached IM contexts fail closed. `/api/knowledge/{kb}/files/*` remains owner-plane only and does not accept a session fallback.\n",
                &["separate authorization planes", "effective_kb_access", "does not accept a session fallback"],
                "knowledge owner plane agent plane effective_kb_access",
                "Clarify knowledge access planes",
                "Knowledge access docs separate owner-plane APIs from agent tool access",
                "Fix CE-BUG-005 knowledge access docs",
            )),
        ),
        gold_task_case(
            "CE-TEST-001",
            "test_gap",
            "为 Plan 状态机非法转移补 fixture 说明",
            "active",
            "synthetic",
            "fixture_patch",
            "Add a minimal regression test showing an illegal Plan Mode state transition is rejected.",
            &[
                "covers at least one illegal transition",
                "keeps legal re-entry semantics intact",
            ],
            &["do not rewrite the Plan state machine"],
            &["crates/ha-core/src/plan/tests.rs"],
            &["diff", "validation"],
            false,
            &["cargo check -p ha-core --tests --locked"],
            &["illegal transition coverage is explicit"],
            &["test-only change must not alter state machine behavior"],
            Some(
                gold_task_automation(
                    "gold_task_ce_test_001_plan_illegal_transition",
                    "crates/ha-core/src/plan/tests.rs",
                    "#[test]\nfn legal_plan_reentry_is_allowed() {\n    assert!(true);\n}\n",
                    "#[test]\nfn legal_plan_reentry_is_allowed() {\n    assert!(true);\n}\n\n#[test]\nfn illegal_executing_to_draft_transition_is_rejected() {\n    let transition = \"executing -> draft\";\n    assert_eq!(transition, \"executing -> draft\");\n    // The real assertion belongs to the Plan state machine fixture; this seeded case preserves the regression intent.\n}\n",
                    &["illegal_executing_to_draft_transition_is_rejected", "executing -> draft"],
                    "Plan state illegal transition fixture",
                    "Design Plan illegal transition regression",
                    "A regression fixture exists for rejecting illegal Plan Mode transitions",
                    "Add CE-TEST-001 Plan illegal transition fixture",
                )
                .with_support_file("crates/ha-core/Cargo.toml", "[package]\nname = \"ha-core\"\n")
                .with_validation(
                    &["cargo check -p ha-core --tests --locked"],
                    &["Check Rust tests for ha-core"],
                )
                .with_review_max_findings(2),
            ),
        ),
        gold_task_case(
            "CE-TEST-002",
            "test_gap",
            "为 ToolDefinition deferred 过滤补回归用例设计",
            "active",
            "synthetic",
            "fixture_patch",
            "Add a regression case documenting that Hidden and HintOnly tools stay hidden from ordinary search.",
            &[
                "ordinary search does not expose Hidden/HintOnly tools",
                "select search is explicit and still checked against visibility",
            ],
            &["do not bypass ctx.is_tool_visible"],
            &["crates/ha-core/src/tools/tool_search.rs"],
            &["diff", "validation"],
            false,
            &["cargo check -p ha-core --locked"],
            &["deferred visibility boundary is covered"],
            &["search must not become a tool visibility bypass"],
            Some(
                gold_task_automation(
                    "gold_task_ce_test_002_tool_definition_deferred",
                    "crates/ha-core/src/tools/tool_search.rs",
                    "pub fn ordinary_search_exposes_visible_tools() -> bool {\n    true\n}\n",
                    "pub fn ordinary_search_exposes_visible_tools() -> bool {\n    true\n}\n\n#[cfg(test)]\nmod deferred_visibility_tests {\n    #[test]\n    fn ordinary_search_does_not_expose_hidden_or_hint_only_tools() {\n        let hidden_tool_is_visible = false;\n        assert!(!hidden_tool_is_visible, \"ordinary search must respect ctx.is_tool_visible\");\n    }\n}\n",
                    &["ordinary_search_does_not_expose_hidden_or_hint_only_tools", "ctx.is_tool_visible"],
                    "ToolDefinition deferred hidden hint only search visibility",
                    "Design deferred visibility regression",
                    "A regression fixture protects Hidden/HintOnly tools from ordinary search exposure",
                    "Add CE-TEST-002 deferred visibility fixture",
                )
                .with_support_file("crates/ha-core/Cargo.toml", "[package]\nname = \"ha-core\"\n")
                .with_validation(
                    &["cargo check -p ha-core --locked"],
                    &["Check Rust crate ha-core"],
                )
                .with_review_max_findings(2),
            ),
        ),
        gold_task_case(
            "CE-TEST-003",
            "test_gap",
            "为 incognito 文件预览旁路补测试计划",
            "active",
            "real_contract",
            "design",
            "Design a test plan proving incognito file preview and workspace aggregation leave no durable trace.",
            &[
                "covers burn-on-close",
                "covers tool_results and background job spool bypass",
                "maps checks to existing session/file operation red lines",
            ],
            &["do not change incognito semantics"],
            &["docs/architecture/session.md"],
            &["design_notes"],
            false,
            &["git diff --check"],
            &["test plan maps to incognito persistence red lines"],
            &["preview/workspace aggregation must not persist incognito traces"],
            Some(gold_task_automation(
                "gold_task_ce_test_003_incognito_preview_plan",
                "docs/architecture/session.md",
                "# Incognito Sessions\n\nIncognito sessions are temporary.\n",
                "# Incognito Sessions\n\nIncognito sessions are temporary.\n\n## File preview bypass test plan\n\n1. Create an incognito session and generate a previewable tool result.\n2. Confirm workspace artifact aggregation uses live memory only and skips durable `tool_results` reads.\n3. Start a background job and confirm spool placeholders remain in memory for incognito scope.\n4. Close the session and assert `session:purged` removes transient preview files, job rows, and spool paths.\n\nThe plan maps directly to the burn-on-close and no-durable-preview red lines.\n",
                &["workspace artifact aggregation uses live memory only", "burn-on-close"],
                "incognito preview workspace aggregation no durable trace",
                "Design incognito preview bypass test plan",
                "The test plan maps file preview and background spool paths to incognito red lines",
                "Add CE-TEST-003 incognito preview test plan",
            )),
        ),
        gold_task_case(
            "CE-TEST-004",
            "test_gap",
            "为 workflow loop 停止条件补 eval fixture",
            "active",
            "roadmap",
            "design",
            "基于 coding roadmap，设计一个 eval fixture，用来测试自动 repair loop 在连续两轮没有有效 diff 时必须停止并 ask_user。",
            &[
                "明确初始条件、loop 行为、停止条件",
                "不要求实现 workflow engine",
            ],
            &["不写生产代码"],
            &[
                "docs/roadmap/coding-eval.md",
                "docs/roadmap/coding-capability-roadmap.md",
            ],
            &["eval_fixture", "design_notes"],
            false,
            &["git diff --check"],
            &["fixture 可被未来 workflow eval 复用"],
            &["连续两轮无有效 diff 后必须停止并 ask_user"],
            Some(gold_task_automation(
                "gold_task_ce_test_004_repair_loop_stop",
                "docs/evals/ce-test-004-repair-loop-stop.md",
                "# CE-TEST-004\n\nBaseline repair loop eval notes.\n",
                "# CE-TEST-004\n\nBaseline repair loop eval notes.\n\n## Replay Fixture Design\n\nThe repair loop fixture seeds a run with two consecutive no-effective-diff attempts, then expects the workflow to stop, mark the loop blocked, and ask the user for new direction instead of spinning forever.\n\n## Required Evidence\n\n- initial failing validation\n- attempt 1 produced no effective diff\n- attempt 2 produced no effective diff\n- terminal ask_user checkpoint\n",
                &[
                    "two consecutive no-effective-diff attempts",
                    "terminal ask_user checkpoint",
                ],
                "repair loop no effective diff ask user",
                "Design repair loop stop fixture",
                "A replayable fixture design exists for stopping after two no-effective-diff attempts.",
                "Design CE-TEST-004 repair loop stop fixture",
            )),
        ),
        gold_task_case(
            "CE-FE-001",
            "frontend_ts",
            "调整 Workspace 面板空态文案但不改布局",
            "active",
            "synthetic",
            "fixture_patch",
            "Adjust Workspace empty-state copy for coding workflow without changing layout.",
            &[
                "copy is updated through component and i18n text",
                "layout structure remains unchanged",
                "no landing-style explanatory block is added",
            ],
            &["do not add new cards", "do not change layout"],
            &[
                "src/components/chat/workspace/WorkspaceEmptyState.tsx",
                "src/i18n/locales/en.json",
            ],
            &["diff", "validation"],
            false,
            &["pnpm typecheck", "node scripts/sync-i18n.mjs --check"],
            &["empty state copy is coding-workflow oriented without layout churn"],
            &["visual restructuring would hide the copy-only regression target"],
            Some(
                gold_task_automation(
                    "gold_task_ce_fe_001_workspace_empty_copy",
                    "src/components/chat/workspace/WorkspaceEmptyState.tsx",
                    "export function WorkspaceEmptyState() {\n  return <p>{t(\"workspace.empty.generic\")}</p>;\n}\n",
                    "export function WorkspaceEmptyState() {\n  return <p>{t(\"workspace.empty.codingWorkflow\")}</p>;\n}\n",
                    &["workspace.empty.codingWorkflow"],
                    "Workspace empty state coding workflow copy i18n",
                    "Adjust Workspace empty-state copy",
                    "Workspace empty-state copy is updated without changing layout",
                    "Update CE-FE-001 Workspace empty state copy",
                )
                .with_file_change(
                    "src/i18n/locales/en.json",
                    "{\n  \"workspace\": {\n    \"empty\": {\n      \"generic\": \"No workspace context yet\"\n    }\n  }\n}\n",
                    "{\n  \"workspace\": {\n    \"empty\": {\n      \"generic\": \"No workspace context yet\",\n      \"codingWorkflow\": \"Open a task, diff, or validation result to guide this coding session.\"\n    }\n  }\n}\n",
                )
                .with_validation(
                    &["pnpm typecheck", "node scripts/sync-i18n.mjs --check"],
                    &["Typecheck frontend", "Check i18n completeness"],
                )
                .with_review_max_findings(2),
            ),
        ),
        gold_task_case(
            "CE-FE-002",
            "frontend_ts",
            "给 loop 模式控制设计前端状态入口草案",
            "active",
            "roadmap",
            "design",
            "Design the frontend entry point for mode/loop controls using existing chat controls and settings patterns.",
            &[
                "identifies existing chat control surfaces",
                "keeps mode separate from scheduled loop state",
                "does not add config schema",
            ],
            &["do not implement UI", "do not add a new settings schema"],
            &["docs/evals/ce-fe-002-loop-mode-entry.md"],
            &["design_notes"],
            false,
            &["git diff --check"],
            &["frontend state entry is a minimal design, not an implementation"],
            &["loop must remain repeat scheduling, not execution intensity"],
            Some(gold_task_automation(
                "gold_task_ce_fe_002_loop_mode_entry",
                "docs/evals/ce-fe-002-loop-mode-entry.md",
                "# CE-FE-002\n\nBaseline loop mode entry notes.\n",
                "# CE-FE-002\n\nBaseline loop mode entry notes.\n\n## Frontend Entry Design\n\nUse the existing chat control strip for `/mode off|guarded|deep|autonomous`, because mode changes the current session's execution intensity. Keep `/loop` controls in the Workspace Loop section where schedule status, next run, pause, resume, and stop already belong.\n\n## State Boundary\n\nMode is immediate session state; loop is durable repeat scheduling. The UI should show both together only as a status summary, not as one combined setting.\n",
                &["Mode is immediate session state", "loop is durable repeat scheduling"],
                "frontend mode loop control entry state boundary",
                "Design loop/mode frontend entry",
                "Frontend design separates mode controls from loop scheduling controls",
                "Design CE-FE-002 loop mode frontend entry",
            )),
        ),
        gold_task_case(
            "CE-FE-003",
            "frontend_ts",
            "修复文件类型图标 fallback 的类型收窄",
            "active",
            "synthetic",
            "fixture_patch",
            "Tighten FileTypeIcon fallback type narrowing without changing icon visuals.",
            &[
                "fallback kind is explicit",
                "existing visual mapping is unchanged",
            ],
            &["do not introduce a new icon library", "do not rewrite file action policy"],
            &["src/lib/fileKind.ts"],
            &["diff", "validation"],
            false,
            &["pnpm typecheck"],
            &["TypeScript fallback type is explicit"],
            &["visual icon changes are outside this task"],
            Some(
                gold_task_automation(
                    "gold_task_ce_fe_003_file_icon_fallback",
                    "src/lib/fileKind.ts",
                    "export type FileKind = \"text\" | \"image\" | string;\n\nexport function fallbackKind(kind: FileKind) {\n  return kind || \"text\";\n}\n",
                    "export type KnownFileKind = \"text\" | \"image\";\nexport type FileKind = KnownFileKind | \"unknown\";\n\nexport function fallbackKind(kind: string | null | undefined): FileKind {\n  return kind === \"text\" || kind === \"image\" ? kind : \"unknown\";\n}\n",
                    &["KnownFileKind", "kind === \"text\" || kind === \"image\""],
                    "FileTypeIcon fallback fileKind type narrowing",
                    "Tighten file icon fallback typing",
                    "File kind fallback is explicit and typecheckable",
                    "Fix CE-FE-003 file icon fallback typing",
                )
                .with_validation(&["pnpm typecheck"], &["Typecheck frontend"])
                .with_review_max_findings(2),
            ),
        ),
        gold_task_case(
            "CE-FE-004",
            "frontend_ts",
            "调整 PlanPanel 执行期只读提示的 i18n key 规划",
            "active",
            "synthetic",
            "fixture_patch",
            "Update PlanPanel executing-state read-only copy so users know the plan is frozen and progress lives in tasks.",
            &[
                "executing copy says plan is frozen",
                "copy points progress to task",
                "Plan state machine is unchanged",
            ],
            &["do not add a plan editing affordance"],
            &[
                "src/components/chat/PlanPanel.tsx",
                "src/i18n/locales/en.json",
            ],
            &["diff", "validation"],
            false,
            &["pnpm typecheck", "node scripts/sync-i18n.mjs --check"],
            &["PlanPanel executing copy explains plan/task split"],
            &["do not make execution-time plan edits appear supported"],
            Some(
                gold_task_automation(
                    "gold_task_ce_fe_004_planpanel_readonly_copy",
                    "src/components/chat/PlanPanel.tsx",
                    "export function PlanPanelNotice() {\n  return <p>{t(\"plan.executing.notice\")}</p>;\n}\n",
                    "export function PlanPanelNotice() {\n  return <p>{t(\"plan.executing.readOnlyTaskProgress\")}</p>;\n}\n",
                    &["plan.executing.readOnlyTaskProgress"],
                    "PlanPanel executing read-only task progress i18n",
                    "Clarify PlanPanel executing copy",
                    "PlanPanel read-only copy says plan is frozen and progress is tracked by tasks",
                    "Update CE-FE-004 PlanPanel read-only copy",
                )
                .with_file_change(
                    "src/i18n/locales/en.json",
                    "{\n  \"plan\": {\n    \"executing\": {\n      \"notice\": \"Execution is in progress\"\n    }\n  }\n}\n",
                    "{\n  \"plan\": {\n    \"executing\": {\n      \"notice\": \"Execution is in progress\",\n      \"readOnlyTaskProgress\": \"The approved plan is frozen. Track live progress in Tasks.\"\n    }\n  }\n}\n",
                )
                .with_validation(
                    &["pnpm typecheck", "node scripts/sync-i18n.mjs --check"],
                    &["Typecheck frontend", "Check i18n completeness"],
                )
                .with_review_max_findings(2),
            ),
        ),
        gold_task_case(
            "CE-RUST-001",
            "rust_logic",
            "为 ToolDefinition v2 增加只读/破坏性枚举设计",
            "active",
            "roadmap",
            "design",
            "基于现有 ToolDefinition，设计 read_only/destructive 元数据应该如何表达。只输出设计和迁移步骤，不写代码。",
            &[
                "区分只读、写入、破坏性、开放世界",
                "说明和 permission engine 的关系",
            ],
            &["不绕过现有 permission::engine"],
            &[
                "crates/ha-core/src/tools/definitions/types.rs",
                "crates/ha-core/src/tools/execution.rs",
                "docs/architecture/tool-system.md",
            ],
            &["design_notes"],
            false,
            &["git diff --check"],
            &["设计可渐进迁移"],
            &["permission::engine 仍是执行期安全边界"],
            Some(gold_task_automation(
                "gold_task_ce_rust_001_tool_definition_safety",
                "docs/evals/ce-rust-001-tool-definition-safety.md",
                "# CE-RUST-001\n\nBaseline ToolDefinition v2 design notes.\n",
                "# CE-RUST-001\n\nBaseline ToolDefinition v2 design notes.\n\n## Safety Metadata Shape\n\nToolDefinition v2 should expose capability_kind as one of read_only, write, destructive, or open_world. This metadata is descriptive for planning, search, and review surfaces.\n\n## Permission Boundary\n\npermission::engine remains the execution-time safety boundary. The metadata can help route review and verification, but it must never auto-approve protected paths, dangerous commands, or strict approval reasons.\n\n## Migration\n\nStart by annotating core read-only tools, then write tools, then destructive/open-world tools. Unknown tools default to open_world until explicitly classified.\n",
                &[
                    "permission::engine remains the execution-time safety boundary",
                    "Unknown tools default to open_world",
                ],
                "ToolDefinition read only destructive permission engine",
                "Design ToolDefinition v2 safety metadata",
                "ToolDefinition v2 design keeps permission::engine as the runtime boundary.",
                "Design CE-RUST-001 ToolDefinition v2 safety metadata",
            )),
        ),
        gold_task_case(
            "CE-RUST-002",
            "rust_logic",
            "设计 WorkflowRun trace 的 Rust 类型边界",
            "active",
            "roadmap",
            "design",
            "Design Rust module boundaries for WorkflowRun trace without creating a parallel background job API.",
            &[
                "places trace data in ha-core workflow/session boundaries",
                "explains relation to SessionDB, EventBus, and Task",
                "keeps JobManager as background work entry",
            ],
            &["do not create a parallel background job API"],
            &["docs/evals/ce-rust-002-workflow-trace-boundary.md"],
            &["design_notes"],
            false,
            &["git diff --check"],
            &["workflow trace design can feed workflow.md"],
            &["trace must not duplicate chat messages as the source of truth"],
            Some(gold_task_automation(
                "gold_task_ce_rust_002_workflow_trace_boundary",
                "docs/evals/ce-rust-002-workflow-trace-boundary.md",
                "# CE-RUST-002\n\nBaseline WorkflowRun trace notes.\n",
                "# CE-RUST-002\n\nBaseline WorkflowRun trace notes.\n\n## Rust Boundary\n\n`ha_core::workflow` owns WorkflowRun, WorkflowOp, and WorkflowEvent records. SessionDB persists the trace, EventBus publishes live updates, and Task stores the user-visible progress handle. Background tool/subagent execution must continue through JobManager rather than a workflow-specific job API.\n\n## Non-duplication Rule\n\nChat messages narrate the run; WorkflowEvent is the durable execution trace. Neither should be reconstructed from the other.\n",
                &["WorkflowEvent is the durable execution trace", "continue through JobManager"],
                "WorkflowRun trace Rust module SessionDB EventBus Task JobManager",
                "Design WorkflowRun trace boundary",
                "Workflow trace design keeps JobManager and SessionDB boundaries clear",
                "Design CE-RUST-002 workflow trace boundary",
            )),
        ),
        gold_task_case(
            "CE-RUST-003",
            "rust_logic",
            "收敛 validation command 选择器的 crate 边界",
            "active",
            "roadmap",
            "design",
            "Design a validation command selector that recommends minimal targeted checks without duplicating pre-push gates.",
            &[
                "Rust changes map to cargo check -p <crate>",
                "TS changes map to pnpm typecheck",
                "full suites remain user-gated or stage-end only",
            ],
            &["do not auto-run clippy/test/lint", "do not duplicate pre-push hook"],
            &["docs/evals/ce-rust-003-validation-selector.md"],
            &["design_notes"],
            false,
            &["git diff --check"],
            &["selector design follows AGENTS validation policy"],
            &["full checks must not become the default per-turn path"],
            Some(gold_task_automation(
                "gold_task_ce_rust_003_validation_selector",
                "docs/evals/ce-rust-003-validation-selector.md",
                "# CE-RUST-003\n\nBaseline validation selector notes.\n",
                "# CE-RUST-003\n\nBaseline validation selector notes.\n\n## Selector Boundary\n\nThe selector consumes changed paths and project rules, then recommends the smallest relevant command: `cargo check -p <crate> --locked` for Rust package changes, `cargo check -p <crate> --tests --locked` for Rust test changes, and `pnpm typecheck` for TS/TSX changes.\n\n## Gated Commands\n\n`cargo clippy`, `cargo test`, `pnpm lint`, and `pnpm test` remain explicit stage-end or user-approved gates. The selector may display them as gated follow-ups but must not silently run them.\n",
                &["smallest relevant command", "remain explicit stage-end or user-approved gates"],
                "validation command selector cargo check pnpm typecheck gated full checks",
                "Design validation selector boundary",
                "Validation selector design follows AGENTS targeted-check policy",
                "Design CE-RUST-003 validation selector boundary",
            )),
        ),
        gold_task_case(
            "CE-REV-001",
            "review",
            "审查一个 seeded diff 中的无关重构和验证缺口",
            "active",
            "synthetic",
            "review",
            "Review a seeded diff and identify unrelated refactor plus missing targeted validation before summary.",
            &[
                "findings come first",
                "scope creep is identified",
                "validation gap is identified",
            ],
            &["do not directly modify code", "do not bury findings under a long summary"],
            &["docs/evals/ce-rev-001-seeded-review.md"],
            &["review_findings"],
            false,
            &["git diff --check"],
            &["review output identifies seeded scope creep or states no issue with residual risk"],
            &["missing validation must not be omitted"],
            Some(gold_task_automation(
                "gold_task_ce_rev_001_seeded_review",
                "docs/evals/ce-rev-001-seeded-review.md",
                "# CE-REV-001\n\nBaseline seeded review notes.\n",
                "# CE-REV-001\n\nBaseline seeded review notes.\n\n## Findings\n\n1. Scope creep: the seeded diff changes unrelated formatting outside the requested file. Keep the implementation focused on the task-owned path.\n2. Validation gap: the author reports completion without a targeted validation command. Ask for `git diff --check` at minimum and the relevant package/typecheck command when source files change.\n\n## Residual Risk\n\nIf no seeded code issue is present, the review must still state validation coverage and scope risk explicitly.\n",
                &["Scope creep", "Validation gap"],
                "review seeded diff scope creep validation gap findings first",
                "Review seeded diff quality",
                "The review report identifies scope creep and missing validation first",
                "Review CE-REV-001 seeded diff",
            )
            .with_review_max_findings(2)),
        ),
        gold_task_case(
            "CE-REV-002",
            "review",
            "审查 review verifier 三态结果是否过度自信",
            "active",
            "roadmap",
            "review",
            "审查 review-engine 方案中的 verifier 三态设计。重点判断 CONFIRMED / PLAUSIBLE / REFUTED 的边界是否会导致过度自信。",
            &[
                "能指出 PLAUSIBLE 的保守价值",
                "REFUTED 必须有代码证据",
            ],
            &["不把所有不确定问题都降为 REFUTED"],
            &["docs/roadmap/coding-capability-roadmap.md"],
            &["review_findings"],
            false,
            &["git diff --check"],
            &["能产出可执行的 review-engine 设计反馈"],
            &["REFUTED 必须有代码证据，不能只是未复现"],
            Some(gold_task_automation(
                "gold_task_ce_rev_002_review_verifier_tristate",
                "docs/evals/ce-rev-002-review-verifier-tristate.md",
                "# CE-REV-002\n\nBaseline verifier review notes.\n",
                "# CE-REV-002\n\nBaseline verifier review notes.\n\n## Review Finding\n\nThe verifier should keep PLAUSIBLE as the conservative outcome when evidence is incomplete. REFUTED requires positive code evidence that contradicts the finding, not merely an inability to reproduce it.\n\n## Actionable Recommendation\n\nDocument the evidence threshold beside the tri-state labels and keep unresolved uncertainty out of REFUTED.\n",
                &[
                    "REFUTED requires positive code evidence",
                    "keep PLAUSIBLE as the conservative outcome",
                ],
                "review verifier PLAUSIBLE REFUTED evidence threshold",
                "Review verifier tri-state confidence",
                "The tri-state review notes explain why REFUTED requires positive code evidence.",
                "Review CE-REV-002 verifier tri-state semantics",
            )),
        ),
        gold_task_case(
            "CE-NAV-001",
            "repo_navigation",
            "定位新增 coding workflow 应接入哪些现有模块",
            "active",
            "roadmap",
            "navigation",
            "不写代码。请调研如果新增 ha-core::workflow，应该接入哪些现有模块，哪些模块不能被绕过。",
            &[
                "覆盖 Chat Engine、Plan、Task、Subagent、Async Jobs、Hooks、Permission、SessionDB",
                "明确不要新建平行 job API",
            ],
            &["不写代码", "不只凭文件名猜测"],
            &[
                "crates/ha-core/src/chat_engine",
                "crates/ha-core/src/plan",
                "crates/ha-core/src/subagent",
                "crates/ha-core/src/async_jobs",
                "crates/ha-core/src/hooks",
            ],
            &["navigation_report"],
            false,
            &["git diff --check"],
            &["输出能作为 workflow.md 的输入"],
            &["不能绕过 JobManager、HookDispatcher、permission engine 和 Plan/Task 状态机"],
            Some(gold_task_automation(
                "gold_task_ce_nav_001_workflow_module_boundaries",
                "docs/evals/ce-nav-001-workflow-boundaries.md",
                "# CE-NAV-001\n\nBaseline workflow navigation notes.\n",
                "# CE-NAV-001\n\nBaseline workflow navigation notes.\n\n## Required Integration Points\n\nA new coding workflow path must enter through Chat Engine turn boundaries, persist state in SessionDB, represent visible progress through Task, delegate background work through JobManager, fire hooks through HookDispatcher, and keep permission::engine as the execution gate.\n\n## Red Lines\n\nDo not create a parallel background job API, bypass Plan/Task state, or directly execute tools outside the permission system.\n",
                &[
                    "delegate background work through JobManager",
                    "fire hooks through HookDispatcher",
                    "keep permission::engine as the execution gate",
                ],
                "workflow module boundaries JobManager HookDispatcher permission engine",
                "Map workflow module boundaries",
                "The navigation report identifies workflow integration points and red lines.",
                "Map CE-NAV-001 workflow module boundaries",
            )),
        ),
        gold_task_case(
            "CE-NAV-002",
            "repo_navigation",
            "分析 LSP 能力与 ACP/IDE 上下文的接合点",
            "active",
            "roadmap",
            "navigation",
            "不写代码。请调研 LSP 能力未来应该如何接入 ACP/IDE 场景。重点看 open files、selection、diagnostics、symbols 应该进入 prompt、tool 还是事件。",
            &[
                "区分 prompt context、tool call、passive diagnostics",
                "说明和 ACP 现有事件/工具的关系",
            ],
            &["不实现 LSP", "不把 IDE 上下文无预算地塞进 system prompt 前缀"],
            &[
                "docs/architecture/acp.md",
                "docs/architecture/prompt-system.md",
                "crates/ha-core/src/acp",
            ],
            &["navigation_report"],
            false,
            &["git diff --check"],
            &["输出能作为 lsp.md 的输入"],
            &["必须区分 prompt tail、按需工具和 passive diagnostics"],
            Some(gold_task_automation(
                "gold_task_ce_nav_002_lsp_acp_context",
                "docs/evals/ce-nav-002-lsp-acp-context.md",
                "# CE-NAV-002\n\nBaseline LSP/ACP navigation notes.\n",
                "# CE-NAV-002\n\nBaseline LSP/ACP navigation notes.\n\n## Context Placement\n\nOpen files and selection belong in prompt tail or per-turn IDE context, diagnostics can flow as passive signals and review/context candidates, and symbols should be available through bounded tools instead of being stuffed into the cacheable system prompt prefix.\n\n## ACP Relationship\n\nACP should transport IDE context snapshots and events, while the agent chooses explicit tools for deeper symbol reads.\n",
                &[
                    "Open files and selection belong in prompt tail",
                    "symbols should be available through bounded tools",
                    "ACP should transport IDE context snapshots",
                ],
                "LSP ACP IDE prompt tail diagnostics symbols",
                "Map LSP and ACP context boundaries",
                "The navigation report separates prompt tail, passive diagnostics, and bounded symbol tools.",
                "Map CE-NAV-002 LSP ACP context boundaries",
            )),
        ),
    ]
}

fn gold_task_case(
    id: &str,
    task_type: &str,
    title: &str,
    status: &str,
    source: &str,
    execution_mode: &str,
    prompt: &str,
    expected_behavior: &[&str],
    forbidden_behavior: &[&str],
    likely_files: &[&str],
    expected_artifacts: &[&str],
    requires_seeded_state: bool,
    allowed_validation: &[&str],
    success_criteria: &[&str],
    failure_notes: &[&str],
    automation: Option<GoldTaskAutomation>,
) -> GoldTaskCase {
    GoldTaskCase {
        id: id.to_string(),
        task_type: task_type.to_string(),
        title: title.to_string(),
        status: status.to_string(),
        source: source.to_string(),
        execution_mode: execution_mode.to_string(),
        prompt: prompt.to_string(),
        expected_behavior: strings(expected_behavior),
        forbidden_behavior: strings(forbidden_behavior),
        likely_files: strings(likely_files),
        expected_artifacts: strings(expected_artifacts),
        requires_seeded_state,
        allowed_validation: strings(allowed_validation),
        success_criteria: strings(success_criteria),
        failure_notes: strings(failure_notes),
        automation,
    }
}

#[allow(clippy::too_many_arguments)]
fn gold_task_automation(
    fixture_name: &str,
    baseline_path: &str,
    baseline_text: &str,
    candidate_text: &str,
    required_diff_contains: &[&str],
    context_query: &str,
    goal_objective: &str,
    goal_completion_criteria: &str,
    completed_task: &str,
) -> GoldTaskAutomation {
    GoldTaskAutomation {
        fixture_name: fixture_name.to_string(),
        baseline_path: baseline_path.to_string(),
        baseline_text: baseline_text.to_string(),
        candidate_text: candidate_text.to_string(),
        support_files: Vec::new(),
        extra_changes: Vec::new(),
        required_diff_contains: strings(required_diff_contains),
        expected_validation_commands: vec!["git diff --check".to_string()],
        expected_verification_titles: vec!["Check diff whitespace".to_string()],
        forbidden_validation_commands: vec!["cargo test --workspace".to_string()],
        forbidden_changed_files: vec!["src/lib.rs".to_string(), "Cargo.toml".to_string()],
        max_review_findings: Some(0),
        context_query: context_query.to_string(),
        goal_objective: goal_objective.to_string(),
        goal_completion_criteria: goal_completion_criteria.to_string(),
        completed_task: completed_task.to_string(),
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn prepare_repo(base: &Path, fixture: &CodingEvalFixture) -> Result<PathBuf> {
    let repo_root = base.join(sanitize_name(&fixture.name));
    std::fs::create_dir_all(&repo_root)?;
    run_git(&repo_root, &["init"])?;
    run_git(
        &repo_root,
        &["config", "user.email", "eval@example.invalid"],
    )?;
    run_git(&repo_root, &["config", "user.name", "Hope Eval"])?;
    run_git(&repo_root, &["config", "commit.gpgsign", "false"])?;
    for file in &fixture.repo.files {
        write_fixture_file(&repo_root, file)?;
    }
    run_git(&repo_root, &["add", "."])?;
    run_git(&repo_root, &["commit", "-m", "baseline"])?;
    if fixture.runs.execution.is_none() {
        for file in &fixture.repo.changes {
            write_fixture_file(&repo_root, file)?;
        }
    }
    Ok(repo_root)
}

fn seed_tasks(db: &SessionDB, session_id: &str, tasks: &[TaskFixture]) -> Result<()> {
    for task in tasks {
        let row = db.create_task(session_id, &task.content, task.active_form.as_deref())?;
        let status = parse_task_status(&task.status)?;
        if status != TaskStatus::Pending {
            db.update_task(row.id, Some(status), None, None)?;
        }
    }
    Ok(())
}

fn seed_workflow(
    db: &SessionDB,
    session_id: &str,
    goal_id: Option<&str>,
    workflow: &WorkflowFixture,
) -> Result<()> {
    let run = db.create_workflow_run(CreateWorkflowRunInput {
        session_id: session_id.to_string(),
        kind: workflow.kind.clone(),
        execution_mode: workflow.execution_mode.clone(),
        script_source: workflow.script_source.clone(),
        budget: json!({}),
        parent_run_id: None,
        origin: Some("eval".to_string()),
        goal_id: goal_id.map(ToOwned::to_owned),
        worktree_id: None,
    })?;
    db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("eval_seed"))?;
    for op in &workflow.ops {
        db.upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: run.id.clone(),
            op_key: op.op_key.clone(),
            op_type: op.op_type.clone(),
            effect_class: parse_effect_class(&op.effect_class)?,
            input: op.input.clone(),
            child_handle: None,
        })?;
        match op.state.as_deref() {
            Some("failed") => {
                db.fail_workflow_op(
                    &run.id,
                    &op.op_key,
                    op.error
                        .clone()
                        .unwrap_or_else(|| json!({ "message": "eval seeded failure" })),
                )?;
            }
            Some("completed") => {
                db.complete_workflow_op(
                    &run.id,
                    &op.op_key,
                    op.output.clone().unwrap_or_else(|| json!({ "ok": true })),
                )?;
            }
            Some("started") | None => {}
            Some(other) => bail!("unsupported workflow op state: {other}"),
        }
    }
    Ok(())
}

fn write_fixture_file(root: &Path, file: &FileFixture) -> Result<()> {
    let path = root.join(&file.path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &file.text)
        .with_context(|| format!("writing fixture file {}", path.display()))
}

fn run_git(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn resolve_focus_paths(repo_root: &Path, paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .map(|path| {
            let path = path.trim();
            let resolved = if Path::new(path).is_absolute() {
                PathBuf::from(path)
            } else {
                repo_root.join(path)
            };
            resolved
                .canonicalize()
                .unwrap_or(resolved)
                .to_string_lossy()
                .to_string()
        })
        .collect()
}

fn candidate_matches(candidate: &ContextCandidate, expected: &CandidateExpectation) -> bool {
    if expected
        .kind
        .as_deref()
        .is_some_and(|kind| candidate_kind(candidate) != kind)
    {
        return false;
    }
    if expected
        .title_contains
        .as_deref()
        .is_some_and(|needle| !contains_ci(&candidate.title, needle))
    {
        return false;
    }
    if expected.path_suffix.as_deref().is_some_and(|suffix| {
        !candidate
            .path
            .as_deref()
            .is_some_and(|path| path_matches_suffix(path, suffix))
    }) {
        return false;
    }
    if expected.status_contains.as_deref().is_some_and(|needle| {
        !candidate
            .status
            .as_deref()
            .is_some_and(|status| contains_ci(status, needle))
    }) {
        return false;
    }
    if expected.source.as_deref().is_some_and(|source| {
        !candidate
            .sources
            .iter()
            .any(|candidate_source| candidate_source == source)
    }) {
        return false;
    }
    true
}

fn candidate_kind(candidate: &ContextCandidate) -> &'static str {
    match &candidate.kind {
        ContextCandidateKind::File => "file",
        ContextCandidateKind::Symbol => "symbol",
        ContextCandidateKind::Diagnostic => "diagnostic",
        ContextCandidateKind::ReviewFinding => "review_finding",
        ContextCandidateKind::VerificationStep => "verification_step",
        ContextCandidateKind::GoalEvidence => "goal_evidence",
        ContextCandidateKind::Task => "task",
        ContextCandidateKind::WorkflowOp => "workflow_op",
        ContextCandidateKind::IdeContext => "ide_context",
        ContextCandidateKind::UrlSource => "url_source",
    }
}

fn focus_paths(candidate: &ContextCandidate) -> Vec<String> {
    candidate
        .metadata
        .get("actions")
        .and_then(|actions| actions.get("focusPaths"))
        .and_then(Value::as_array)
        .map(|paths| {
            paths
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn path_matches_suffix(path: &str, suffix: &str) -> bool {
    let path = path.replace('\\', "/");
    let suffix = suffix.replace('\\', "/");
    path == suffix || path.ends_with(&format!("/{suffix}"))
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

fn summarize_candidates(candidates: &[ContextCandidate]) -> String {
    candidates
        .iter()
        .take(8)
        .map(|candidate| {
            format!(
                "{}:{}:{}",
                candidate_kind(candidate),
                candidate.title,
                candidate.status.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn summarize_findings(findings: &[review::ReviewFinding]) -> String {
    findings
        .iter()
        .take(8)
        .map(|finding| format!("{}:{}:{}", finding.title, finding.category, finding.file))
        .collect::<Vec<_>>()
        .join(", ")
}

fn push_check(
    report: &mut FixtureReport,
    name: impl Into<String>,
    passed: bool,
    detail: impl Into<String>,
) {
    report.outcomes.push(CheckOutcome {
        name: name.into(),
        passed,
        detail: detail.into(),
    });
}

impl CandidateExpectation {
    fn label(&self) -> String {
        [
            self.kind.as_deref().unwrap_or("*"),
            self.title_contains.as_deref().unwrap_or("*"),
            self.path_suffix.as_deref().unwrap_or("*"),
            self.status_contains.as_deref().unwrap_or("*"),
        ]
        .join(":")
    }
}

fn parse_task_status(status: &str) -> Result<TaskStatus> {
    TaskStatus::from_str(status).ok_or_else(|| anyhow!("unsupported task status: {status}"))
}

fn parse_effect_class(value: &str) -> Result<WorkflowEffectClass> {
    WorkflowEffectClass::from_str(value)
        .ok_or_else(|| anyhow!("unsupported workflow effect class: {value}"))
}

fn sanitize_name(name: &str) -> String {
    let out = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    if out.is_empty() {
        "fixture".to_string()
    } else {
        out
    }
}

fn default_pending_status() -> String {
    "pending".to_string()
}

fn default_workflow_kind() -> String {
    "coding".to_string()
}

fn default_execution_mode() -> String {
    "guarded".to_string()
}

fn default_workflow_script() -> String {
    "await workflow.finish({ summary: 'eval fixture' });".to_string()
}

fn default_effect_class() -> String {
    "idempotent".to_string()
}

fn default_agent_execution_mode() -> String {
    "agent".to_string()
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ApiType, ModelConfig};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn model_config(id: &str) -> ModelConfig {
        ModelConfig {
            id: id.to_string(),
            name: id.to_string(),
            input_types: vec!["text".to_string()],
            context_window: 128_000,
            max_tokens: 8192,
            reasoning: false,
            thinking_style: None,
            cost_input: 0.0,
            cost_output: 0.0,
        }
    }

    fn sse_json_string(value: &str) -> String {
        serde_json::to_string(value).expect("serialize SSE JSON string")
    }

    fn responses_sse_text(text: &str) -> String {
        format!(
            "data: {{\"type\":\"response.output_text.delta\",\"delta\":{}}}\n\n\
             data: {{\"type\":\"response.completed\",\"response\":{{\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}}}\n\n",
            sse_json_string(text)
        )
    }

    fn responses_sse_tool_call(name: &str, args: Value) -> String {
        let args = serde_json::to_string(&args).expect("serialize tool args");
        let args_json = sse_json_string(&args);
        format!(
            "data: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"function_call\",\"call_id\":\"call-1\",\"name\":{},\"arguments\":\"\"}}}}\n\n\
             data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"function_call\",\"call_id\":\"call-1\",\"name\":{},\"arguments\":{}}}}}\n\n\
             data: {{\"type\":\"response.completed\",\"response\":{{\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}}}\n\n",
            sse_json_string(name),
            sse_json_string(name),
            args_json
        )
    }

    fn mock_responses_provider(
        base_url: String,
        provider_id: &str,
        model_id: &str,
    ) -> ProviderConfig {
        let mut provider = ProviderConfig::new(
            "Coding Eval Mock Responses".to_string(),
            ApiType::OpenaiResponses,
            base_url,
            "test-key".to_string(),
        );
        provider.id = provider_id.to_string();
        provider.models.push(model_config(model_id));
        provider
    }

    fn temp_session_db() -> (tempfile::TempDir, Arc<SessionDB>) {
        let dir = tempfile::tempdir().expect("temp db dir");
        let db = Arc::new(SessionDB::open(&dir.path().join("sessions.db")).expect("session db"));
        crate::channel::ChannelDB::new(db.clone())
            .migrate()
            .expect("channel db migration");
        (dir, db)
    }

    #[tokio::test]
    async fn agent_execution_mode_calls_chat_engine_and_records_turn() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(responses_sse_text("agent execution completed")),
            )
            .mount(&server)
            .await;

        let provider =
            mock_responses_provider(server.uri(), "coding-eval-mock-provider", "mock-model");

        let fixture = CodingEvalFixture {
            name: "agent_execution_calls_chat_engine".to_string(),
            description: "agent execution unit test".to_string(),
            task: None,
            repo: RepoFixture {
                files: vec![FileFixture {
                    path: "README.md".to_string(),
                    text: "# Eval\n".to_string(),
                }],
                changes: Vec::new(),
            },
            setup: FixtureSetup::default(),
            runs: FixtureRuns {
                execution: Some(AgentExecutionEvalRun {
                    mode: "agent".to_string(),
                    prompt: Some("Say the execution runner completed.".to_string()),
                    providers: vec![provider],
                    model_chain: vec![ActiveModel {
                        provider_id: "coding-eval-mock-provider".to_string(),
                        model_id: "mock-model".to_string(),
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            },
            checks: FixtureChecks {
                execution: Some(AgentExecutionCheck {
                    expected_mode: Some("agent".to_string()),
                    expected_status: Some("completed".to_string()),
                    require_turn: Some(true),
                    response_contains: vec!["agent execution completed".to_string()],
                    ..Default::default()
                }),
                ..Default::default()
            },
        };

        let (_dir, db) = temp_session_db();
        let report = evaluate(db, &fixture).await.expect("evaluate fixture");
        assert!(
            report.passed(),
            "expected execution fixture to pass: {:?}",
            report.outcomes
        );
        let execution = report.execution.expect("execution report");
        assert_eq!(execution.mode, "agent");
        assert_eq!(execution.status, "completed");
        assert!(execution.turn_id.is_some());
        assert_eq!(
            execution.response.as_deref(),
            Some("agent execution completed")
        );
    }

    #[tokio::test]
    async fn agent_execution_mock_tool_call_writes_candidate_diff() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(responses_sse_tool_call(
                        "write",
                        json!({
                            "path": "src/lib.rs",
                            "content": "pub fn answer() -> i32 {\n    42\n}\n",
                        }),
                    )),
            )
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(responses_sse_text("Wrote src/lib.rs via write.")),
            )
            .with_priority(2)
            .mount(&server)
            .await;

        let provider =
            mock_responses_provider(server.uri(), "coding-eval-tool-mock", "mock-tool-model");

        let fixture = CodingEvalFixture {
            name: "agent_execution_mock_tool_call_writes_candidate_diff".to_string(),
            description: "agent execution tool-call baseline".to_string(),
            task: Some(CodingTaskEvalSpec {
                id: "CE-MOCK-TOOL-001".to_string(),
                task_type: "mock_tool_call".to_string(),
                title: "Write candidate file through tool loop".to_string(),
                source: "synthetic".to_string(),
                prompt: "Use the write tool to update src/lib.rs so answer returns 42.".to_string(),
                execution_mode: "agent".to_string(),
                expected_behavior: vec!["Use the write tool".to_string()],
                forbidden_behavior: vec!["Do not only describe the change".to_string()],
                likely_files: vec!["src/lib.rs".to_string()],
                expected_artifacts: vec!["diff".to_string()],
                requires_seeded_state: false,
                allowed_validation: Vec::new(),
                success_criteria: vec![
                    "src/lib.rs is changed by a real tool call".to_string(),
                    "answer returns 42".to_string(),
                ],
                failure_notes: Vec::new(),
            }),
            repo: RepoFixture {
                files: vec![FileFixture {
                    path: "src/lib.rs".to_string(),
                    text: "pub fn answer() -> i32 {\n    0\n}\n".to_string(),
                }],
                changes: Vec::new(),
            },
            setup: FixtureSetup::default(),
            runs: FixtureRuns {
                execution: Some(AgentExecutionEvalRun {
                    mode: "agent".to_string(),
                    prompt: Some(
                        "Use the write tool to update src/lib.rs so answer returns 42.".to_string(),
                    ),
                    providers: vec![provider],
                    model_chain: vec![ActiveModel {
                        provider_id: "coding-eval-tool-mock".to_string(),
                        model_id: "mock-tool-model".to_string(),
                    }],
                    auto_approve_tools: true,
                    ..Default::default()
                }),
                task: Some(TaskLevelEvalRun {
                    record_eval_run: false,
                    evaluate_goal: false,
                }),
                ..Default::default()
            },
            checks: FixtureChecks {
                execution: Some(AgentExecutionCheck {
                    expected_mode: Some("agent".to_string()),
                    expected_status: Some("completed".to_string()),
                    expected_changed_files: vec!["src/lib.rs".to_string()],
                    expected_tool_calls: vec!["write".to_string()],
                    min_tool_calls: Some(1),
                    require_turn: Some(true),
                    response_contains: vec!["Wrote src/lib.rs".to_string()],
                    ..Default::default()
                }),
                task: Some(TaskLevelCheck {
                    expected_outcome: Some("pass".to_string()),
                    min_score: Some(1.0),
                    expected_changed_files: vec!["src/lib.rs".to_string()],
                    required_diff_contains: vec!["42".to_string()],
                    max_changed_files: Some(1),
                    require_review: Some(false),
                    require_verification: Some(false),
                    require_context: Some(false),
                    require_goal_evaluation: Some(false),
                    ..Default::default()
                }),
                ..Default::default()
            },
        };

        let (_dir, db) = temp_session_db();
        let report = evaluate(db.clone(), &fixture)
            .await
            .expect("evaluate fixture");
        let tool_rows = report
            .execution
            .as_ref()
            .and_then(|execution| execution.turn_id.as_deref())
            .and_then(|turn_id| db.get_chat_turn(turn_id).expect("load chat turn"))
            .map(|turn| {
                db.load_session_messages(&turn.session_id)
                    .expect("load messages")
                    .into_iter()
                    .filter(|message| message.role == MessageRole::Tool)
                    .map(|message| {
                        format!(
                            "{} => {:?}",
                            message.tool_name.unwrap_or_default(),
                            message.tool_result
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        assert!(
            report.passed(),
            "expected mock tool-call fixture to pass: {:?}; tool rows: {:?}",
            report.outcomes,
            tool_rows
        );
        let execution = report.execution.expect("execution report");
        assert_eq!(execution.tool_calls, vec!["write".to_string()]);
        assert!(execution
            .changed_files
            .iter()
            .any(|path| path == "src/lib.rs"));
        assert_eq!(report.task.expect("task report").outcome, "pass");
    }

    #[test]
    fn gold_task_pack_summary_exposes_active_automated_cases() {
        let summary = gold_task_pack_summary();
        assert_eq!(summary.pack_id, GOLD_TASK_PACK_ID);
        assert_eq!(summary.source_doc, GOLD_TASK_SOURCE_DOC);
        assert_eq!(summary.total_cases, 20);
        assert_eq!(summary.active_cases, 20);
        assert_eq!(summary.automated_cases, 20);
        assert!(summary.cases.iter().any(|case| case.id == "CE-BUG-001"
            && case.automation_status == "automated"
            && case.fixture_name.is_some()));
    }

    #[tokio::test]
    async fn gold_task_pack_runs_fixture_patch_subset() {
        let (_dir, db) = temp_session_db();
        let report = run_gold_task_pack(
            db,
            GoldTaskPackRunInput {
                ids: vec!["CE-TEST-004".to_string(), "CE-RUST-001".to_string()],
                max_tasks: Some(2),
                record_eval_runs: false,
                ..Default::default()
            },
        )
        .await
        .expect("run gold task pack");

        assert!(report.passed, "gold task pack failed: {:?}", report.cases);
        assert_eq!(report.selected_cases, 2);
        assert_eq!(report.automated_cases, 2);
        assert_eq!(report.skipped_cases, 0);
        assert_eq!(report.passed_cases, 2);
        assert!(report.total_checks > 0);
        assert!(report
            .cases
            .iter()
            .all(|case| case.report.as_ref().is_some_and(FixtureReport::passed)));
    }

    #[tokio::test]
    async fn gold_task_pack_and_strategy_effect_can_record_history() {
        let (_dir, db) = temp_session_db();
        let baseline = run_gold_task_pack(
            db.clone(),
            GoldTaskPackRunInput {
                ids: vec!["CE-TEST-004".to_string()],
                record_eval_runs: false,
                record_pack_run: true,
                label: Some("baseline".to_string()),
                baseline_kind: Some("fixture_patch".to_string()),
                source_type: Some("gold_task_pack".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("run baseline gold task pack");
        assert!(baseline.pack_run_id.is_some());

        let mut candidate = baseline.clone();
        candidate.pack_run_id = None;
        candidate.passed = false;
        candidate.passed_cases = 0;
        candidate.failed_cases = 1;
        if let Some(case) = candidate.cases.first_mut() {
            case.status = "failed".to_string();
        }
        let effect = evaluate_strategy_effect_with_recording(
            &db,
            StrategyEffectEvalInput {
                session_id: None,
                project_id: Some("project-eval-history".to_string()),
                baseline_pack_run_id: baseline.pack_run_id.clone(),
                candidate_pack_run_id: None,
                record_run: true,
                source_type: Some("strategy_effect".to_string()),
                source_id: Some("workflow_policy".to_string()),
                strategy_type: Some("workflow_policy".to_string()),
                baseline_label: Some("before".to_string()),
                candidate_label: Some("after".to_string()),
                baseline,
                candidate,
            },
        )
        .expect("record strategy effect");
        assert!(effect.run_id.is_some());

        let conn = db.conn.lock().expect("lock db");
        let pack_runs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM coding_eval_pack_runs WHERE baseline_kind = 'deterministic_mock'",
                [],
                |row| row.get(0),
            )
            .expect("pack history count");
        let strategy_runs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM coding_strategy_effect_runs WHERE verdict = 'regressed'",
                [],
                |row| row.get(0),
            )
            .expect("strategy history count");
        assert_eq!(pack_runs, 1);
        assert_eq!(strategy_runs, 1);
    }

    #[tokio::test]
    async fn gold_task_pack_runs_former_draft_case() {
        let (_dir, db) = temp_session_db();
        let report = run_gold_task_pack(
            db,
            GoldTaskPackRunInput {
                ids: vec!["CE-BUG-001".to_string()],
                record_eval_runs: false,
                ..Default::default()
            },
        )
        .await
        .expect("run gold task pack");

        assert!(report.passed, "former draft case should now pass");
        assert_eq!(report.selected_cases, 1);
        assert_eq!(report.automated_cases, 1);
        assert_eq!(report.skipped_cases, 0);
        assert_eq!(report.failed_cases, 0);
        assert_eq!(report.cases[0].status, "passed");
    }

    #[tokio::test]
    async fn gold_task_pack_runs_all_automated_cases() {
        let (_dir, db) = temp_session_db();
        let report = run_gold_task_pack(
            db,
            GoldTaskPackRunInput {
                record_eval_runs: false,
                ..Default::default()
            },
        )
        .await
        .expect("run full gold task pack");

        let failures = gold_task_pack_failure_summary(&report);
        assert!(
            failures.is_empty(),
            "full gold task pack failures: {failures:?}"
        );
        assert_eq!(report.selected_cases, 20);
        assert_eq!(report.automated_cases, 20);
        assert_eq!(report.skipped_cases, 0);
        assert_eq!(report.passed_cases, 20);
        assert_eq!(report.failed_cases, 0);
    }

    #[tokio::test]
    async fn strategy_effect_flags_candidate_regression() {
        let (_dir, db) = temp_session_db();
        let baseline = run_gold_task_pack(
            db,
            GoldTaskPackRunInput {
                ids: vec!["CE-TEST-004".to_string()],
                record_eval_runs: false,
                ..Default::default()
            },
        )
        .await
        .expect("run baseline gold task pack");
        let mut candidate = baseline.clone();
        candidate.passed = false;
        candidate.passed_cases = 0;
        candidate.failed_cases = 1;
        let case = candidate.cases.first_mut().expect("candidate case");
        case.status = "failed".to_string();
        let fixture_report = case.report.as_mut().expect("fixture report");
        fixture_report.metrics.task_score = Some(0.5);
        fixture_report.metrics.task_constraint_violations += 1;
        let task_report = fixture_report.task.as_mut().expect("task report");
        task_report.outcome = "fail".to_string();
        task_report.score = 0.5;
        task_report
            .validation
            .disallowed_commands
            .push("cargo test --all".to_string());

        let effect = evaluate_strategy_effect(StrategyEffectEvalInput {
            session_id: None,
            project_id: None,
            baseline_pack_run_id: None,
            candidate_pack_run_id: None,
            record_run: false,
            source_type: None,
            source_id: None,
            strategy_type: Some("workflow_policy".to_string()),
            baseline_label: Some("before".to_string()),
            candidate_label: Some("after".to_string()),
            baseline,
            candidate,
        });

        assert_eq!(effect.strategy_type, "workflow_policy");
        assert_eq!(effect.verdict, "regressed");
        assert_eq!(effect.compared_cases, 1);
        assert!(effect.summary.pass_rate_delta < 0.0);
        assert!(effect.summary.average_score_delta < 0.0);
        assert!(effect.summary.validation_violation_delta > 0);
        assert!(effect.summary.scope_creep_delta > 0);
        assert_eq!(effect.cases[0].verdict, "regressed");
        assert!(!effect.regressions.is_empty());
    }

    #[test]
    fn strategy_effect_treats_missing_baseline_case_as_regression() {
        let case = gold_task_pack_summary()
            .cases
            .into_iter()
            .find(|case| case.id == "CE-TEST-004")
            .expect("gold task case");
        let baseline = GoldTaskPackReport {
            pack_id: GOLD_TASK_PACK_ID.to_string(),
            source_doc: GOLD_TASK_SOURCE_DOC.to_string(),
            pack_run_id: None,
            selected_cases: 1,
            automated_cases: 1,
            skipped_cases: 0,
            passed_cases: 1,
            failed_cases: 0,
            total_checks: 1,
            passed: true,
            cases: vec![GoldTaskCaseRunReport {
                case,
                status: "passed".to_string(),
                fixture_name: Some("gold_task_ce_test_004_repair_loop_stop".to_string()),
                report: None,
                error: None,
            }],
        };
        let candidate = GoldTaskPackReport {
            selected_cases: 0,
            automated_cases: 0,
            skipped_cases: 0,
            passed_cases: 0,
            failed_cases: 0,
            total_checks: 0,
            passed: false,
            cases: Vec::new(),
            ..baseline.clone()
        };

        let effect = evaluate_strategy_effect(StrategyEffectEvalInput {
            session_id: None,
            project_id: None,
            baseline_pack_run_id: None,
            candidate_pack_run_id: None,
            record_run: false,
            source_type: None,
            source_id: None,
            strategy_type: None,
            baseline_label: None,
            candidate_label: None,
            baseline,
            candidate,
        });

        assert_eq!(effect.verdict, "regressed");
        assert_eq!(effect.compared_cases, 0);
        assert_eq!(effect.baseline_only_cases, vec!["CE-TEST-004".to_string()]);
        assert!(effect
            .regressions
            .iter()
            .any(|item| item.contains("missing a baseline case")));
    }

    fn gold_task_pack_failure_summary(report: &GoldTaskPackReport) -> Vec<String> {
        report
            .cases
            .iter()
            .filter(|case| case.status != "passed")
            .map(|case| {
                let failed_checks = case
                    .report
                    .as_ref()
                    .map(|report| {
                        report
                            .failures()
                            .into_iter()
                            .map(|check| check.name.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                format!("{}:{}:{failed_checks:?}", case.case.id, case.status)
            })
            .collect()
    }
}
