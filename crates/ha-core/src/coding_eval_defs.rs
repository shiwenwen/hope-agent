//! `coding_eval` 的**契约层**——kernel 侧仍需认识的评测 wire 类型。
//!
//! # 为什么在 kernel
//!
//! 评测机器（fixture runner / gold task pack / strategy 对照）随阶段 5 第四刀
//! 迁入 `ha-eval-runtime`，但 kernel 的 `coding_improvement`（提案队列 /
//! Benchmark Run Center）**存的就是这些报告的 JSON**：`coding_benchmark_*` 表
//! 里落 `GoldTaskPackReport`、`StrategyEffectReport`，排行榜再解回来；
//! 提案晋升成正式 eval fixture 时还要按 `CodingEvalFixture` 校验一遍。
//! 若类型跟着机器上浮，kernel 就得反向 `use ha_eval_runtime::…`，成环。
//!
//! 同 `tool_defs` / `slash_defs` / `cron_defs` 的分法：**契约下沉、机器上浮**。
//! `ha_eval_runtime::coding_eval` 对本模块做 glob 再导出，故
//! `…::coding_eval::CodingEvalFixture` 这类既有路径**逐字不变**。
//!
//! 本模块**零行为逻辑**：只有 serde 结构、`Default`、以及两个纯格式化
//! 的小 impl。任何需要跑评测的代码都在 ha-eval-runtime。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context_compact::CompactConfig;
use crate::provider::{ActiveModel, ProviderConfig};
use crate::session::SessionIdeContext;

// `CodingEvalFixture.seed_eval_runs` 的元素类型。它原本定义在
// `coding_improvement`，但那样一来契约层就反向依赖业务层，与
// `coding_improvement` 对本模块的正向依赖构成源码环——同 crate 内能编译，
// 却会在后续 improve 域上浮时变成真的 Cargo 反向依赖。故随契约一起下沉，
// `coding_improvement` 保留同名 re-export。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordCodingEvalRunInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub suite: String,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub metrics: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default)]
    pub execution_mode: Option<String>,
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
            execution_mode: None,
            providers: Vec::new(),
            model_chain: Vec::new(),
            compact_config: None,
            reasoning_effort: None,
            extra_system_context: None,
            denied_tools: Vec::new(),
            auto_approve_tools: false,
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

impl CandidateExpectation {
    pub fn label(&self) -> String {
        [
            self.kind.as_deref().unwrap_or("*"),
            self.title_contains.as_deref().unwrap_or("*"),
            self.path_suffix.as_deref().unwrap_or("*"),
            self.status_contains.as_deref().unwrap_or("*"),
        ]
        .join(":")
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
