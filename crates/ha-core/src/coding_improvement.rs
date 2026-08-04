//! Coding trend report and improvement-loop proposal queue.
//!
//! Phase 3.11 turns the durable coding control-plane traces (Goal, Workflow,
//! Review, Verification, Repair Loop, and eval records) into a deterministic
//! trend report plus improvement proposals.
//!
//! Phase 4.1 keeps the same owner-plane safety boundary and adds a
//! proposal-to-action layer: every proposal can be previewed as a deterministic
//! action plan, then explicitly applied into reviewable draft artifacts. Phase
//! 4.2 adds terminal workflow retros and explicit draft promotion into formal
//! eval fixtures, project guidance includes, or active managed skills. Phase
//! 4.4 adds deterministic transcript distillation and failure feedback
//! proposals. Phase 6.1 adds a read-only Benchmark Run Center on top of the
//! durable pack history. Phase 7.5 routes general-domain quality signals into
//! the same draft-first improvement queue. Generation, distillation, apply,
//! promotion, benchmark execution, and domain campaign learning all remain
//! explicit owner-plane actions.

use anyhow::{anyhow, bail, Result};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::coding_eval_defs::{GoldTaskPackReport, GoldTaskPackRunInput, StrategyEffectReport};
// `RecordCodingEvalRunInput` 是 `CodingEvalFixture.seed_eval_runs` 的字段类型，
// 已随契约层下沉 `coding_eval_defs`（否则 defs ↔ 业务层成源码环，会卡住后续
// improve 域上浮）。此处再导出保住 `coding_improvement::RecordCodingEvalRunInput`
// 既有路径。
pub use crate::coding_eval_defs::RecordCodingEvalRunInput;
use crate::session::SessionDB;
use crate::util::now_rfc3339;

pub const DEFAULT_WINDOW_DAYS: u32 = 30;
pub const MAX_WINDOW_DAYS: u32 = 180;
const DEFAULT_RELEASE_GATE_MIN_PACK_RUNS: usize = 1;
const DEFAULT_RELEASE_GATE_MIN_STRATEGY_EFFECT_RUNS: usize = 0;
const DEFAULT_RELEASE_GATE_MIN_PACK_PASS_RATE: f64 = 1.0;
pub const DEFAULT_RELEASE_GATE_MAX_REGRESSED_STRATEGY_EFFECTS: usize = 0;
const DEFAULT_RELEASE_GATE_MAX_MIXED_STRATEGY_EFFECTS: usize = 0;
const DEFAULT_RELEASE_GATE_MAX_MISSING_TOOL_CALL_RUNS: usize = 0;
const DEFAULT_RELEASE_GATE_MAX_VALIDATION_VIOLATION_DELTA: isize = 0;
const DEFAULT_RELEASE_GATE_MAX_SCOPE_CREEP_DELTA: isize = 0;
const DEFAULT_GENERALIZATION_MIN_PROJECTS: usize = 2;
const DEFAULT_GENERALIZATION_MIN_PROJECT_PACK_RUNS: usize = 1;
const DEFAULT_GENERALIZATION_MIN_PROJECT_PACK_PASS_RATE: f64 = 1.0;
const DEFAULT_GENERALIZATION_MIN_STRATEGY_EFFECT_RUNS_PER_PROJECT: usize = 0;
const DEFAULT_GENERALIZATION_MAX_REGRESSED_PROJECTS: usize = 0;
const DEFAULT_GENERALIZATION_MAX_MIXED_PROJECTS: usize = 0;
const DEFAULT_GENERALIZATION_MAX_VALIDATION_VIOLATION_DELTA_PER_PROJECT: isize = 0;
const DEFAULT_GENERALIZATION_MAX_SCOPE_CREEP_DELTA_PER_PROJECT: isize = 0;
const DEFAULT_BENCHMARK_CENTER_LIMIT: usize = 12;
const MAX_BENCHMARK_CENTER_LIMIT: usize = 50;
const DEFAULT_BENCHMARK_CAMPAIGN_LIMIT: usize = 20;
pub const MAX_BENCHMARK_CAMPAIGN_LIMIT: usize = 100;
const MAX_BENCHMARK_CAMPAIGN_MODELS: usize = 16;
pub const DEFAULT_BENCHMARK_LEADERBOARD_LIMIT: usize = 12;
const MAX_BENCHMARK_LEADERBOARD_LIMIT: usize = 50;
const DEFAULT_BENCHMARK_LEADERBOARD_MIN_ITEMS: usize = 1;
const DEFAULT_BENCHMARK_CORPUS_LIMIT: usize = 30;
const MAX_BENCHMARK_CORPUS_LIMIT: usize = 100;
const MAX_BENCHMARK_CORPUS_TASKS: usize = 500;
const DEFAULT_BENCHMARK_CORPUS_STALE_DAYS: u32 = 90;
const MAX_BENCHMARK_CORPUS_STALE_DAYS: u32 = 365;
const DEFAULT_BENCHMARK_REPORT_LIMIT: usize = 20;
const MAX_BENCHMARK_REPORT_LIMIT: usize = 100;
const DEFAULT_CONTINUOUS_GATE_MAX_EVIDENCE_AGE_DAYS: u32 = 14;
const MAX_CONTINUOUS_GATE_MAX_EVIDENCE_AGE_DAYS: u32 = 180;
const DEFAULT_CONTINUOUS_GATE_MIN_CAMPAIGN_ITEMS: usize = 1;
const DEFAULT_CONTINUOUS_GATE_MIN_CASE_PASS_RATE: f64 = 1.0;
const DEFAULT_BENCHMARK_BACKLOG_LIMIT: usize = 20;
pub const MAX_BENCHMARK_BACKLOG_LIMIT: usize = 100;
const MAX_SCOPE_SESSIONS: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingTrendReport {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub scope: String,
    pub window_days: u32,
    pub generated_at: String,
    pub overview: CodingTrendOverview,
    pub eval: CodingEvalTrend,
    pub review: CodingReviewTrend,
    pub verification: CodingVerificationTrend,
    pub repair_loop: CodingRepairLoopTrend,
    pub retro: CodingRetroTrend,
    pub failures: Vec<CodingFailureBucket>,
    pub recent_runs: Vec<CodingRunSummary>,
    #[serde(default)]
    pub retros: Vec<CodingWorkflowRetro>,
    pub proposals: Vec<CodingImprovementProposal>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingTrendOverview {
    pub sessions: usize,
    pub goals: usize,
    pub completed_goals: usize,
    pub blocked_goals: usize,
    pub workflow_runs: usize,
    pub completed_workflows: usize,
    pub blocked_workflows: usize,
    pub failed_workflows: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_completion_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_completion_rate: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingEvalTrend {
    pub runs: usize,
    pub passed: usize,
    pub failed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_rate: Option<f64>,
    pub backlog_candidates: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingReviewTrend {
    pub runs: usize,
    pub findings: usize,
    pub blocking_findings: usize,
    pub resolved_findings: usize,
    pub false_positive_findings: usize,
    pub by_category: Vec<CodingMetricBucket>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingVerificationTrend {
    pub runs: usize,
    pub steps: usize,
    pub passed_steps: usize,
    pub failed_steps: usize,
    pub timed_out_steps: usize,
    pub planned_only_runs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executed_success_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommendation_coverage: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingRepairLoopTrend {
    pub runs: usize,
    pub completed: usize,
    pub blocked: usize,
    pub exhausted: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_rate: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingRetroTrend {
    pub total: usize,
    pub completed: usize,
    pub blocked: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub recommendations: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingMetricBucket {
    pub key: String,
    pub label: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingFailureBucket {
    pub category: String,
    pub label: String,
    pub count: usize,
    pub severity: String,
    #[serde(default)]
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingRunSummary {
    pub run_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    pub kind: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_category: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingWorkflowRetro {
    pub id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub workflow_run_id: String,
    pub run_state: String,
    pub summary: String,
    #[serde(default)]
    pub signals: Vec<CodingRetroSignal>,
    #[serde(default)]
    pub recommendations: Vec<CodingRetroRecommendation>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingRetroSignal {
    pub kind: String,
    pub label: String,
    pub severity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingRetroRecommendation {
    pub kind: String,
    pub title: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementProposal {
    pub id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub source_type: String,
    pub source_id: String,
    pub title: String,
    pub body: String,
    pub payload: Value,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<CodingImprovementActionRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promotion: Option<CodingImprovementPromotionRecord>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementActionRecord {
    pub applied: bool,
    #[serde(default)]
    pub artifacts: Vec<CodingImprovementActionArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementActionArtifact {
    pub kind: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementActionPlan {
    pub proposal: CodingImprovementProposal,
    pub target_kind: String,
    pub summary: String,
    pub requires_confirmation: bool,
    pub steps: Vec<CodingImprovementActionStep>,
    #[serde(default)]
    pub preview: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementActionStep {
    pub action: String,
    pub label: String,
    pub target_path: String,
    pub target_exists: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_preview: Option<String>,
    #[serde(skip)]
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyCodingImprovementProposalResult {
    pub proposal: CodingImprovementProposal,
    pub plan: CodingImprovementActionPlan,
    pub applied: bool,
    #[serde(default)]
    pub artifacts: Vec<CodingImprovementActionArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementPromotionRecord {
    pub promoted: bool,
    #[serde(default)]
    pub artifacts: Vec<CodingImprovementActionArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementPromotionPlan {
    pub proposal: CodingImprovementProposal,
    pub target_kind: String,
    pub summary: String,
    pub requires_confirmation: bool,
    pub steps: Vec<CodingImprovementPromotionStep>,
    #[serde(default)]
    pub preview: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementPromotionStep {
    pub action: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    pub target_path: String,
    pub target_exists: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_preview: Option<String>,
    #[serde(skip)]
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromoteCodingImprovementProposalResult {
    pub proposal: CodingImprovementProposal,
    pub plan: CodingImprovementPromotionPlan,
    pub promoted: bool,
    #[serde(default)]
    pub artifacts: Vec<CodingImprovementActionArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateCodingImprovementProposalsResult {
    pub inserted: usize,
    pub proposals: Vec<CodingImprovementProposal>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateCodingImprovementProposalsInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default)]
    pub proposal_kinds: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistillCodingImprovementResult {
    pub inserted: usize,
    pub distillation: CodingImprovementDistillation,
    pub proposals: Vec<CodingImprovementProposal>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementDistillation {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub scope: String,
    pub generated_at: String,
    pub transcript: CodingTranscriptDistillation,
    pub workflow_patterns: Vec<CodingWorkflowPatternDistillation>,
    pub failure_feedback: Vec<CodingFailureFeedback>,
    pub candidates: Vec<CodingDistilledCandidate>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingTranscriptDistillation {
    pub sessions_scanned: usize,
    pub messages_scanned: usize,
    pub user_messages: usize,
    pub assistant_messages: usize,
    pub tool_calls: usize,
    pub tool_errors: usize,
    pub top_tools: Vec<CodingToolUsageDistillation>,
    pub objective_snippets: Vec<String>,
    pub error_snippets: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingToolUsageDistillation {
    pub tool_name: String,
    pub calls: usize,
    pub errors: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_duration_ms: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingWorkflowPatternDistillation {
    pub run_id: String,
    pub session_id: String,
    pub kind: String,
    pub state: String,
    pub execution_mode: String,
    pub op_count: usize,
    pub completed_ops: usize,
    pub failed_ops: usize,
    pub has_review: bool,
    pub has_verification: bool,
    pub has_diff: bool,
    pub tool_ops: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingFailureFeedback {
    pub category: String,
    pub label: String,
    pub severity: String,
    pub count: usize,
    pub rule: String,
    pub expected_signals: Vec<String>,
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingDistilledCandidate {
    pub kind: String,
    pub source_type: String,
    pub source_id: String,
    pub title: String,
    pub rationale: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingEvalRunRecord {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub suite: String,
    pub name: String,
    pub status: String,
    pub metrics: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordCodingEvalPackRunInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub report: GoldTaskPackReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingEvalPackRunRecord {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub pack_id: String,
    pub source_doc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub baseline_kind: String,
    pub status: String,
    pub selected_cases: usize,
    pub automated_cases: usize,
    pub skipped_cases: usize,
    pub passed_cases: usize,
    pub failed_cases: usize,
    pub total_checks: usize,
    pub report: GoldTaskPackReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordCodingStrategyEffectRunInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_pack_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_pack_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub report: StrategyEffectReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingStrategyEffectRunRecord {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub strategy_type: String,
    pub baseline_label: String,
    pub candidate_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_pack_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_pack_run_id: Option<String>,
    pub verdict: String,
    pub compared_cases: usize,
    pub pass_rate_delta: f64,
    pub average_score_delta: f64,
    pub context_recall_delta: f64,
    pub validation_violation_delta: isize,
    pub scope_creep_delta: isize,
    pub execution_failure_delta: isize,
    pub report: StrategyEffectReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingEvalReleaseGateInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_pack_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_strategy_effect_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_pack_pass_rate: Option<f64>,
    #[serde(default)]
    pub require_external_model_pack: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_regressed_strategy_effects: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_mixed_strategy_effects: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_missing_tool_call_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_validation_violation_delta: Option<isize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_scope_creep_delta: Option<isize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingEvalReleaseGateThresholds {
    pub min_pack_runs: usize,
    pub min_strategy_effect_runs: usize,
    pub min_pack_pass_rate: f64,
    pub require_external_model_pack: bool,
    pub max_regressed_strategy_effects: usize,
    pub max_mixed_strategy_effects: usize,
    pub max_missing_tool_call_runs: usize,
    pub max_validation_violation_delta: isize,
    pub max_scope_creep_delta: isize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingEvalReleaseGateSummary {
    pub pack_runs: usize,
    pub passed_pack_runs: usize,
    pub failed_pack_runs: usize,
    pub skipped_pack_runs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pack_pass_rate: Option<f64>,
    pub deterministic_pack_runs: usize,
    pub mock_provider_pack_runs: usize,
    pub external_model_pack_runs: usize,
    pub passed_cases: usize,
    pub failed_cases: usize,
    pub skipped_cases: usize,
    pub total_checks: usize,
    pub strategy_effect_runs: usize,
    pub improved_strategy_effects: usize,
    pub regressed_strategy_effects: usize,
    pub mixed_strategy_effects: usize,
    pub inconclusive_strategy_effects: usize,
    pub validation_violation_delta: isize,
    pub scope_creep_delta: isize,
    pub execution_failure_delta: isize,
    pub missing_tool_call_runs: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingEvalReleaseGateCheck {
    pub name: String,
    pub status: String,
    pub severity: String,
    pub expected: String,
    pub actual: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingEvalReleaseGateReport {
    pub generated_at: String,
    pub status: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub window_days: u32,
    pub since: String,
    pub thresholds: CodingEvalReleaseGateThresholds,
    pub summary: CodingEvalReleaseGateSummary,
    pub checks: Vec<CodingEvalReleaseGateCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingLearningGeneralizationInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default)]
    pub proposal_kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_projects: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_project_pack_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_project_pack_pass_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_strategy_effect_runs_per_project: Option<usize>,
    #[serde(default = "crate::default_true")]
    pub require_promoted_learning: bool,
    #[serde(default)]
    pub require_external_model_pack: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_regressed_projects: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_mixed_projects: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_validation_violation_delta_per_project: Option<isize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_scope_creep_delta_per_project: Option<isize>,
}

impl Default for CodingLearningGeneralizationInput {
    fn default() -> Self {
        Self {
            session_id: None,
            project_id: None,
            window_days: None,
            source_type: None,
            source_id: None,
            proposal_kinds: Vec::new(),
            min_projects: None,
            min_project_pack_runs: None,
            min_project_pack_pass_rate: None,
            min_strategy_effect_runs_per_project: None,
            require_promoted_learning: true,
            require_external_model_pack: false,
            max_regressed_projects: None,
            max_mixed_projects: None,
            max_validation_violation_delta_per_project: None,
            max_scope_creep_delta_per_project: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingLearningGeneralizationThresholds {
    pub min_projects: usize,
    pub min_project_pack_runs: usize,
    pub min_project_pack_pass_rate: f64,
    pub min_strategy_effect_runs_per_project: usize,
    pub require_promoted_learning: bool,
    pub require_external_model_pack: bool,
    pub max_regressed_projects: usize,
    pub max_mixed_projects: usize,
    pub max_validation_violation_delta_per_project: isize,
    pub max_scope_creep_delta_per_project: isize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingLearningGeneralizationSummary {
    pub projects_evaluated: usize,
    pub projects_with_promoted_learning: usize,
    pub projects_with_pack_runs: usize,
    pub projects_with_strategy_effects: usize,
    pub projects_with_external_model_pack: usize,
    pub passed_projects: usize,
    pub failed_projects: usize,
    pub insufficient_projects: usize,
    pub total_promoted_learning: usize,
    pub total_pack_runs: usize,
    pub total_strategy_effect_runs: usize,
    pub regressed_projects: usize,
    pub mixed_projects: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingLearningGeneralizationItem {
    pub proposal_id: String,
    pub project_id: String,
    pub kind: String,
    pub title: String,
    pub source_type: String,
    pub source_id: String,
    pub promoted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingLearningGeneralizationProject {
    pub project_id: String,
    pub status: String,
    pub promoted_learning: usize,
    pub pack_runs: usize,
    pub passed_pack_runs: usize,
    pub failed_pack_runs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pack_pass_rate: Option<f64>,
    pub external_model_pack_runs: usize,
    pub strategy_effect_runs: usize,
    pub improved_strategy_effects: usize,
    pub regressed_strategy_effects: usize,
    pub mixed_strategy_effects: usize,
    pub validation_violation_delta: isize,
    pub scope_creep_delta: isize,
    pub execution_failure_delta: isize,
    pub reasons: Vec<String>,
    pub learning_items: Vec<CodingLearningGeneralizationItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingLearningGeneralizationCheck {
    pub name: String,
    pub status: String,
    pub severity: String,
    pub expected: String,
    pub actual: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingLearningGeneralizationReport {
    pub generated_at: String,
    pub status: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub window_days: u32,
    pub since: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub proposal_kinds: Vec<String>,
    pub thresholds: CodingLearningGeneralizationThresholds,
    pub summary: CodingLearningGeneralizationSummary,
    pub projects: Vec<CodingLearningGeneralizationProject>,
    pub checks: Vec<CodingLearningGeneralizationCheck>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkCenterInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default)]
    pub require_external_model_baseline: bool,
    #[serde(default)]
    pub require_learning_generalization: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkCenterSummary {
    pub total_runs: usize,
    pub passed_runs: usize,
    pub failed_runs: usize,
    pub skipped_runs: usize,
    pub deterministic_runs: usize,
    pub external_model_runs: usize,
    pub selected_cases: usize,
    pub automated_cases: usize,
    pub passed_cases: usize,
    pub failed_cases: usize,
    pub skipped_cases: usize,
    pub total_checks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_pass_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_pass_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_case_pass_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_run_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_run_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkRunItem {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub pack_id: String,
    pub source_doc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub baseline_kind: String,
    pub status: String,
    pub selected_cases: usize,
    pub automated_cases: usize,
    pub skipped_cases: usize,
    pub passed_cases: usize,
    pub failed_cases: usize,
    pub total_checks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_pass_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub created_at: String,
    pub failed_cases_summary: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkBaselineBucket {
    pub baseline_kind: String,
    pub runs: usize,
    pub passed_runs: usize,
    pub failed_runs: usize,
    pub skipped_runs: usize,
    pub passed_cases: usize,
    pub failed_cases: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_pass_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_pass_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_run_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkCenterCheck {
    pub name: String,
    pub status: String,
    pub severity: String,
    pub expected: String,
    pub actual: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkCenterReport {
    pub generated_at: String,
    pub status: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub window_days: u32,
    pub since: String,
    pub summary: CodingBenchmarkCenterSummary,
    pub baselines: Vec<CodingBenchmarkBaselineBucket>,
    pub runs: Vec<CodingBenchmarkRunItem>,
    pub checks: Vec<CodingBenchmarkCenterCheck>,
    pub release_gate: CodingEvalReleaseGateReport,
    pub generalization_gate: CodingLearningGeneralizationReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkCampaignModel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// Owner-plane reference accepted on create/run requests only. Campaign
    /// normalization clears it before persistence and responses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_profile_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkCampaignCreateInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub gold_task_input: GoldTaskPackRunInput,
    #[serde(default)]
    pub models: Vec<CodingBenchmarkCampaignModel>,
    #[serde(default)]
    pub run_now: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_budget_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkCampaignListInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkCampaignRunInput {
    pub campaign_id: String,
    /// Deprecated compatibility field. Owner adapters resolve `models` from
    /// backend configuration and clear this field before persistence.
    #[serde(default)]
    pub providers: Vec<crate::provider::ProviderConfig>,
    #[serde(default)]
    pub retry_failed_only: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkCampaignSummary {
    pub total_items: usize,
    pub queued_items: usize,
    pub running_items: usize,
    pub passed_items: usize,
    pub failed_items: usize,
    pub skipped_items: usize,
    pub cancelled_items: usize,
    pub interrupted_items: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_pass_rate: Option<f64>,
    pub selected_cases: usize,
    pub passed_cases: usize,
    pub failed_cases: usize,
    pub skipped_cases: usize,
    pub total_checks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_pass_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkCampaignItem {
    pub id: String,
    pub campaign_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub status: String,
    pub attempt: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pack_run_id: Option<String>,
    pub selected_cases: usize,
    pub passed_cases: usize,
    pub failed_cases: usize,
    pub skipped_cases: usize,
    pub total_checks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkCampaign {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub name: String,
    pub status: String,
    pub task_pack_id: String,
    pub source_doc: String,
    pub execution_mode: String,
    pub baseline_kind: String,
    pub task_filter: Value,
    pub model_matrix: Vec<CodingBenchmarkCampaignModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_budget_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    pub summary: CodingBenchmarkCampaignSummary,
    pub items: Vec<CodingBenchmarkCampaignItem>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkLeaderboardInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub campaign_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_items: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkComparisonInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub campaign_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_items: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkLeaderboardEvidence {
    pub campaign_id: String,
    pub campaign_name: String,
    pub item_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pack_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub status: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkLeaderboardRow {
    pub rank: usize,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub task_pack_id: String,
    pub source_doc: String,
    pub execution_mode: String,
    pub baseline_kind: String,
    pub campaigns: usize,
    pub items: usize,
    pub passed_items: usize,
    pub failed_items: usize,
    pub skipped_items: usize,
    pub cancelled_items: usize,
    pub interrupted_items: usize,
    pub attempts: usize,
    pub selected_cases: usize,
    pub passed_cases: usize,
    pub failed_cases: usize,
    pub skipped_cases: usize,
    pub total_checks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_pass_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_pass_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub evidence: Vec<CodingBenchmarkLeaderboardEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkLeaderboardReport {
    pub generated_at: String,
    pub status: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub window_days: u32,
    pub since: String,
    pub min_items: usize,
    pub rows: Vec<CodingBenchmarkLeaderboardRow>,
    pub checks: Vec<CodingBenchmarkCenterCheck>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkTaskPackTaskManifest {
    pub task_id: String,
    pub version: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub task_type: String,
    pub difficulty: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framework: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_template: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub success_criteria: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation_commands: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbidden_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calibration_notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibrated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redaction_status: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkTaskPackManifest {
    pub pack_id: String,
    pub version: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub source_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_template: Option<String>,
    pub license_note: String,
    pub privacy_note: String,
    pub redaction_status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<CodingBenchmarkTaskPackTaskManifest>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkTaskPackImportInput {
    pub manifest: CodingBenchmarkTaskPackManifest,
    #[serde(default)]
    pub explicit_import_consent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_from: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkTaskPackListInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default)]
    pub include_archived: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkTaskPackStatusInput {
    pub pack_id: String,
    pub version: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkTaskPackValidateInput {
    pub pack_id: String,
    pub version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkCorpusHealthInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_after_days: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkTaskPackTask {
    pub id: String,
    pub pack_id: String,
    pub pack_version: String,
    pub task_id: String,
    pub version: String,
    pub title: String,
    pub status: String,
    pub task_type: String,
    pub difficulty: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framework: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_template: Option<String>,
    pub tags: Vec<String>,
    pub success_criteria: Vec<String>,
    pub validation_commands: Vec<String>,
    pub allowed_paths: Vec<String>,
    pub forbidden_paths: Vec<String>,
    pub calibration_notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calibrated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privacy_note: Option<String>,
    pub redaction_status: String,
    pub risk_flags: Vec<String>,
    pub fingerprint: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkTaskPack {
    pub id: String,
    pub pack_id: String,
    pub version: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: String,
    pub source_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_template: Option<String>,
    pub license_note: String,
    pub privacy_note: String,
    pub redaction_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imported_from: Option<String>,
    pub tasks: Vec<CodingBenchmarkTaskPackTask>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkTaskPackValidationReport {
    pub generated_at: String,
    pub status: String,
    pub pack_id: String,
    pub version: String,
    pub checks: Vec<CodingBenchmarkCenterCheck>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkCorpusDuplicate {
    pub fingerprint: String,
    pub tasks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkCorpusHealthReport {
    pub generated_at: String,
    pub status: String,
    pub stale_after_days: u32,
    pub packs: usize,
    pub active_packs: usize,
    pub draft_packs: usize,
    pub archived_packs: usize,
    pub tasks: usize,
    pub active_tasks: usize,
    pub draft_tasks: usize,
    pub archived_tasks: usize,
    pub by_difficulty: Vec<CodingMetricBucket>,
    pub by_task_type: Vec<CodingMetricBucket>,
    pub by_language: Vec<CodingMetricBucket>,
    pub stale_tasks: Vec<String>,
    pub duplicate_tasks: Vec<CodingBenchmarkCorpusDuplicate>,
    pub gaming_risk_tasks: Vec<String>,
    pub checks: Vec<CodingBenchmarkCenterCheck>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkReportGenerateInput {
    pub report_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub campaign_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    #[serde(default)]
    pub mark_release_evidence: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkReportListInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default)]
    pub release_evidence_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkReportMarkInput {
    pub report_id: String,
    pub release_evidence: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkReport {
    pub id: String,
    pub report_type: String,
    pub title: String,
    pub status: String,
    pub summary: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub source_type: String,
    pub source_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub campaign_ids: Vec<String>,
    pub snapshot: Value,
    pub markdown_path: String,
    pub json_path: String,
    pub html_path: String,
    pub release_evidence: bool,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marked_release_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingContinuousBenchmarkGateInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_evidence_age_days: Option<u32>,
    #[serde(default = "serde_default_true")]
    pub require_release_report_evidence: bool,
    #[serde(default = "serde_default_true")]
    pub require_recent_campaign: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_task_pack_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_baseline_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_model_id: Option<String>,
    #[serde(default)]
    pub require_external_model: bool,
    #[serde(default)]
    pub external_model_policy_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_campaign_items: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_case_pass_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_open_backlog_items: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_interrupted_campaigns: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_provider_error_items: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_budget_exhausted_items: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_budget_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingContinuousBenchmarkGateThresholds {
    pub trigger_kind: String,
    pub window_days: u32,
    pub max_evidence_age_days: u32,
    pub require_release_report_evidence: bool,
    pub require_recent_campaign: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_task_pack_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_baseline_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_model_id: Option<String>,
    pub require_external_model: bool,
    pub external_model_policy_enabled: bool,
    pub min_campaign_items: usize,
    pub min_case_pass_rate: f64,
    pub max_open_backlog_items: usize,
    pub max_interrupted_campaigns: usize,
    pub max_provider_error_items: usize,
    pub max_budget_exhausted_items: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_budget_usd: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingContinuousBenchmarkReliability {
    pub campaigns: usize,
    pub passed_campaigns: usize,
    pub failed_campaigns: usize,
    pub partial_campaigns: usize,
    pub interrupted_campaigns: usize,
    pub cancelled_campaigns: usize,
    pub retry_attempts: usize,
    pub retry_passed_items: usize,
    pub provider_error_items: usize,
    pub budget_exhausted_items: usize,
    pub approval_wait_items: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign_success_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_success_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_error_rate: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingContinuousBenchmarkGateSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_release_report_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_release_evidence_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_passed_at: Option<String>,
    pub fresh_release_evidence: bool,
    pub fresh_campaigns: usize,
    pub total_campaign_items: usize,
    pub passed_campaign_items: usize,
    pub failed_campaign_items: usize,
    pub interrupted_campaign_items: usize,
    pub cancelled_campaign_items: usize,
    pub selected_cases: usize,
    pub passed_cases: usize,
    pub failed_cases: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_pass_rate: Option<f64>,
    pub open_backlog_items: usize,
    pub pending_failure_items: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_campaign_budget_usd: Option<f64>,
    pub retention_days: u32,
    pub raw_artifact_retention_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingContinuousBenchmarkGateReport {
    pub generated_at: String,
    pub status: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub since: String,
    pub stale_before: String,
    pub thresholds: CodingContinuousBenchmarkGateThresholds,
    pub summary: CodingContinuousBenchmarkGateSummary,
    pub reliability: CodingContinuousBenchmarkReliability,
    pub checks: Vec<CodingBenchmarkCenterCheck>,
    pub release_gate: CodingEvalReleaseGateReport,
    pub leaderboard: CodingBenchmarkLeaderboardReport,
    pub corpus_health: CodingBenchmarkCorpusHealthReport,
    pub blockers: Vec<String>,
    pub recommended_next_steps: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkBacklogListInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkBacklogMaterializeInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub campaign_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkBacklogStatusInput {
    pub item_id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkBacklogItem {
    pub id: String,
    pub status: String,
    pub severity: String,
    pub title: String,
    pub failure_category: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub campaign_id: String,
    pub campaign_item_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pack_run_id: Option<String>,
    pub task_pack_id: String,
    pub task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub baseline_kind: String,
    pub execution_mode: String,
    pub evidence: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBenchmarkBacklogMaterializeResult {
    pub inserted: usize,
    pub existing: usize,
    pub items: Vec<CodingBenchmarkBacklogItem>,
}

pub struct ReportScope {
    pub session_id: String,
    pub project_id: Option<String>,
    pub session_ids: Vec<String>,
    pub window_days: u32,
    pub since: String,
}

struct ReleaseGateScope {
    session_id: Option<String>,
    project_id: Option<String>,
    scope: String,
    window_days: u32,
    since: String,
}

struct LearningGeneralizationScope {
    session_id: Option<String>,
    project_id: Option<String>,
    scope: String,
    window_days: u32,
    since: String,
    source_type: Option<String>,
    source_id: Option<String>,
    proposal_kinds: Vec<String>,
}

struct BenchmarkCenterScope {
    session_id: Option<String>,
    project_id: Option<String>,
    scope: String,
    window_days: u32,
    since: String,
    limit: usize,
}

pub struct ContinuousBenchmarkGateScope {
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub scope: String,
    pub since: String,
    pub stale_before: String,
}

#[derive(Default)]
pub struct ContinuousBenchmarkFailureCandidate {
    pub campaign_id: String,
    pub campaign_item_id: String,
    pub pack_run_id: Option<String>,
    pub task_pack_id: String,
    pub task_id: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub label: Option<String>,
    pub baseline_kind: String,
    pub execution_mode: String,
    pub status: String,
    pub failure_category: String,
    pub title: String,
    pub evidence: Value,
}

struct BenchmarkLeaderboardScope {
    session_id: Option<String>,
    project_id: Option<String>,
    scope: String,
    window_days: u32,
    since: String,
    limit: usize,
    min_items: usize,
    campaign_ids: Vec<String>,
}

#[derive(Default)]
struct LearningProjectAccumulator {
    learning_items: Vec<CodingLearningGeneralizationItem>,
    pack_runs: usize,
    passed_pack_runs: usize,
    failed_pack_runs: usize,
    external_model_pack_runs: usize,
    strategy_effect_runs: usize,
    improved_strategy_effects: usize,
    regressed_strategy_effects: usize,
    mixed_strategy_effects: usize,
    validation_violation_delta: isize,
    scope_creep_delta: isize,
    execution_failure_delta: isize,
}

impl LearningProjectAccumulator {
    fn into_report(
        mut self,
        project_id: String,
        thresholds: &CodingLearningGeneralizationThresholds,
    ) -> CodingLearningGeneralizationProject {
        let promoted_learning = self.learning_items.len();
        self.learning_items.truncate(8);
        let pack_pass_rate = ratio(
            self.passed_pack_runs,
            self.passed_pack_runs + self.failed_pack_runs,
        );
        let mut insufficient = Vec::new();
        let mut failures = Vec::new();

        if thresholds.require_promoted_learning && promoted_learning == 0 {
            insufficient.push("no promoted learning artifact in this project".to_string());
        }
        if self.pack_runs < thresholds.min_project_pack_runs {
            insufficient.push(format!(
                "{} pack run(s), need {}",
                self.pack_runs, thresholds.min_project_pack_runs
            ));
        }
        if thresholds.require_external_model_pack && self.external_model_pack_runs == 0 {
            insufficient.push("no external_model pack run".to_string());
        }
        if self.strategy_effect_runs < thresholds.min_strategy_effect_runs_per_project {
            insufficient.push(format!(
                "{} strategy effect run(s), need {}",
                self.strategy_effect_runs, thresholds.min_strategy_effect_runs_per_project
            ));
        }
        if self.pack_runs >= thresholds.min_project_pack_runs {
            match pack_pass_rate {
                Some(rate) if rate + f64::EPSILON < thresholds.min_project_pack_pass_rate => {
                    failures.push(format!(
                        "pack pass rate {rate:.3} below {:.3}",
                        thresholds.min_project_pack_pass_rate
                    ));
                }
                None if thresholds.min_project_pack_pass_rate > 0.0 => {
                    insufficient.push("pack history has no passed/failed denominator".to_string());
                }
                _ => {}
            }
        }
        if self.regressed_strategy_effects > 0 {
            failures.push(format!(
                "{} regressed strategy effect(s)",
                self.regressed_strategy_effects
            ));
        }
        if self.mixed_strategy_effects > 0 && thresholds.max_mixed_projects == 0 {
            failures.push(format!(
                "{} mixed strategy effect(s)",
                self.mixed_strategy_effects
            ));
        }
        if self.validation_violation_delta > thresholds.max_validation_violation_delta_per_project {
            failures.push(format!(
                "validation violation delta {} exceeds {}",
                self.validation_violation_delta,
                thresholds.max_validation_violation_delta_per_project
            ));
        }
        if self.scope_creep_delta > thresholds.max_scope_creep_delta_per_project {
            failures.push(format!(
                "scope creep delta {} exceeds {}",
                self.scope_creep_delta, thresholds.max_scope_creep_delta_per_project
            ));
        }

        let status = if !failures.is_empty() {
            "failed"
        } else if !insufficient.is_empty() {
            "insufficient_data"
        } else {
            "passed"
        };
        let mut reasons = failures;
        reasons.extend(insufficient);

        CodingLearningGeneralizationProject {
            project_id,
            status: status.to_string(),
            promoted_learning,
            pack_runs: self.pack_runs,
            passed_pack_runs: self.passed_pack_runs,
            failed_pack_runs: self.failed_pack_runs,
            pack_pass_rate,
            external_model_pack_runs: self.external_model_pack_runs,
            strategy_effect_runs: self.strategy_effect_runs,
            improved_strategy_effects: self.improved_strategy_effects,
            regressed_strategy_effects: self.regressed_strategy_effects,
            mixed_strategy_effects: self.mixed_strategy_effects,
            validation_violation_delta: self.validation_violation_delta,
            scope_creep_delta: self.scope_creep_delta,
            execution_failure_delta: self.execution_failure_delta,
            reasons,
            learning_items: self.learning_items,
        }
    }
}

pub(crate) fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS coding_eval_runs (
            id TEXT PRIMARY KEY,
            session_id TEXT,
            project_id TEXT,
            suite TEXT NOT NULL,
            name TEXT NOT NULL,
            status TEXT NOT NULL,
            metrics_json TEXT NOT NULL DEFAULT '{}',
            source_type TEXT,
            source_id TEXT,
            created_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_coding_eval_runs_scope
            ON coding_eval_runs(project_id, session_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_coding_eval_runs_status
            ON coding_eval_runs(status, created_at DESC);

        CREATE TABLE IF NOT EXISTS coding_eval_pack_runs (
            id TEXT PRIMARY KEY,
            session_id TEXT,
            project_id TEXT,
            pack_id TEXT NOT NULL,
            source_doc TEXT NOT NULL,
            label TEXT,
            baseline_kind TEXT NOT NULL,
            status TEXT NOT NULL,
            selected_cases INTEGER NOT NULL,
            automated_cases INTEGER NOT NULL,
            skipped_cases INTEGER NOT NULL,
            passed_cases INTEGER NOT NULL,
            failed_cases INTEGER NOT NULL,
            total_checks INTEGER NOT NULL,
            report_json TEXT NOT NULL DEFAULT '{}',
            source_type TEXT,
            source_id TEXT,
            created_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_coding_eval_pack_runs_scope
            ON coding_eval_pack_runs(project_id, session_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_coding_eval_pack_runs_status
            ON coding_eval_pack_runs(status, baseline_kind, created_at DESC);

        CREATE TABLE IF NOT EXISTS coding_strategy_effect_runs (
            id TEXT PRIMARY KEY,
            session_id TEXT,
            project_id TEXT,
            strategy_type TEXT NOT NULL,
            baseline_label TEXT NOT NULL,
            candidate_label TEXT NOT NULL,
            baseline_pack_run_id TEXT,
            candidate_pack_run_id TEXT,
            verdict TEXT NOT NULL,
            compared_cases INTEGER NOT NULL,
            pass_rate_delta REAL NOT NULL,
            average_score_delta REAL NOT NULL,
            context_recall_delta REAL NOT NULL,
            validation_violation_delta INTEGER NOT NULL,
            scope_creep_delta INTEGER NOT NULL,
            execution_failure_delta INTEGER NOT NULL,
            report_json TEXT NOT NULL DEFAULT '{}',
            source_type TEXT,
            source_id TEXT,
            created_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (baseline_pack_run_id) REFERENCES coding_eval_pack_runs(id) ON DELETE SET NULL,
            FOREIGN KEY (candidate_pack_run_id) REFERENCES coding_eval_pack_runs(id) ON DELETE SET NULL
        );

        CREATE INDEX IF NOT EXISTS idx_coding_strategy_effect_runs_scope
            ON coding_strategy_effect_runs(project_id, session_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_coding_strategy_effect_runs_verdict
            ON coding_strategy_effect_runs(verdict, created_at DESC);

        CREATE TABLE IF NOT EXISTS coding_benchmark_campaigns (
            id TEXT PRIMARY KEY,
            session_id TEXT,
            project_id TEXT,
            name TEXT NOT NULL,
            status TEXT NOT NULL,
            task_pack_id TEXT NOT NULL,
            source_doc TEXT NOT NULL,
            execution_mode TEXT NOT NULL,
            baseline_kind TEXT NOT NULL,
            task_filter_json TEXT NOT NULL DEFAULT '{}',
            model_matrix_json TEXT NOT NULL DEFAULT '[]',
            max_budget_usd REAL,
            timeout_secs INTEGER,
            error TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            started_at TEXT,
            finished_at TEXT,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_coding_benchmark_campaigns_scope
            ON coding_benchmark_campaigns(project_id, session_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_coding_benchmark_campaigns_status
            ON coding_benchmark_campaigns(status, updated_at DESC);

        CREATE TABLE IF NOT EXISTS coding_benchmark_campaign_items (
            id TEXT PRIMARY KEY,
            campaign_id TEXT NOT NULL,
            provider_id TEXT,
            model_id TEXT,
            label TEXT,
            status TEXT NOT NULL,
            attempt INTEGER NOT NULL DEFAULT 0,
            pack_run_id TEXT,
            selected_cases INTEGER NOT NULL DEFAULT 0,
            passed_cases INTEGER NOT NULL DEFAULT 0,
            failed_cases INTEGER NOT NULL DEFAULT 0,
            skipped_cases INTEGER NOT NULL DEFAULT 0,
            total_checks INTEGER NOT NULL DEFAULT 0,
            report_json TEXT NOT NULL DEFAULT '{}',
            error TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            started_at TEXT,
            finished_at TEXT,
            FOREIGN KEY (campaign_id) REFERENCES coding_benchmark_campaigns(id) ON DELETE CASCADE,
            FOREIGN KEY (pack_run_id) REFERENCES coding_eval_pack_runs(id) ON DELETE SET NULL
        );

        CREATE INDEX IF NOT EXISTS idx_coding_benchmark_campaign_items_campaign
            ON coding_benchmark_campaign_items(campaign_id, status, updated_at DESC);

        CREATE TABLE IF NOT EXISTS coding_benchmark_task_packs (
            id TEXT PRIMARY KEY,
            pack_id TEXT NOT NULL,
            pack_version TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT,
            status TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            source_uri TEXT,
            repo_template TEXT,
            license_note TEXT NOT NULL,
            privacy_note TEXT NOT NULL,
            redaction_status TEXT NOT NULL,
            imported_from TEXT,
            manifest_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            activated_at TEXT,
            archived_at TEXT,
            UNIQUE(pack_id, pack_version)
        );

        CREATE INDEX IF NOT EXISTS idx_coding_benchmark_task_packs_status
            ON coding_benchmark_task_packs(status, updated_at DESC);

        CREATE TABLE IF NOT EXISTS coding_benchmark_task_pack_tasks (
            id TEXT PRIMARY KEY,
            pack_row_id TEXT NOT NULL,
            pack_id TEXT NOT NULL,
            pack_version TEXT NOT NULL,
            task_id TEXT NOT NULL,
            task_version TEXT NOT NULL,
            title TEXT NOT NULL,
            status TEXT NOT NULL,
            task_type TEXT NOT NULL,
            difficulty TEXT NOT NULL,
            language TEXT,
            framework TEXT,
            source_uri TEXT,
            repo_template TEXT,
            tags_json TEXT NOT NULL DEFAULT '[]',
            success_criteria_json TEXT NOT NULL DEFAULT '[]',
            validation_commands_json TEXT NOT NULL DEFAULT '[]',
            allowed_paths_json TEXT NOT NULL DEFAULT '[]',
            forbidden_paths_json TEXT NOT NULL DEFAULT '[]',
            calibration_notes_json TEXT NOT NULL DEFAULT '[]',
            calibrated_at TEXT,
            license_note TEXT,
            privacy_note TEXT,
            redaction_status TEXT NOT NULL,
            risk_flags_json TEXT NOT NULL DEFAULT '[]',
            fingerprint TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (pack_row_id) REFERENCES coding_benchmark_task_packs(id) ON DELETE CASCADE,
            UNIQUE(pack_id, pack_version, task_id, task_version)
        );

        CREATE INDEX IF NOT EXISTS idx_coding_benchmark_task_pack_tasks_pack
            ON coding_benchmark_task_pack_tasks(pack_row_id, status, task_type);
        CREATE INDEX IF NOT EXISTS idx_coding_benchmark_task_pack_tasks_fingerprint
            ON coding_benchmark_task_pack_tasks(fingerprint);

        CREATE TABLE IF NOT EXISTS coding_benchmark_reports (
            id TEXT PRIMARY KEY,
            report_type TEXT NOT NULL,
            title TEXT NOT NULL,
            status TEXT NOT NULL,
            summary TEXT NOT NULL,
            scope TEXT NOT NULL,
            session_id TEXT,
            project_id TEXT,
            source_type TEXT NOT NULL,
            source_id TEXT NOT NULL,
            campaign_id TEXT,
            campaign_ids_json TEXT NOT NULL DEFAULT '[]',
            snapshot_json TEXT NOT NULL DEFAULT '{}',
            markdown_path TEXT NOT NULL,
            json_path TEXT NOT NULL,
            html_path TEXT NOT NULL,
            release_evidence INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            marked_release_at TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_coding_benchmark_reports_scope
            ON coding_benchmark_reports(project_id, session_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_coding_benchmark_reports_release
            ON coding_benchmark_reports(release_evidence, created_at DESC);

        CREATE TABLE IF NOT EXISTS coding_benchmark_backlog_items (
            id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            severity TEXT NOT NULL,
            title TEXT NOT NULL,
            failure_category TEXT NOT NULL,
            scope TEXT NOT NULL,
            session_id TEXT,
            project_id TEXT,
            campaign_id TEXT NOT NULL,
            campaign_item_id TEXT NOT NULL,
            pack_run_id TEXT,
            task_pack_id TEXT NOT NULL,
            task_id TEXT NOT NULL DEFAULT '',
            provider_id TEXT,
            model_id TEXT,
            label TEXT,
            baseline_kind TEXT NOT NULL,
            execution_mode TEXT NOT NULL,
            evidence_json TEXT NOT NULL DEFAULT '{}',
            proposal_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            resolved_at TEXT,
            UNIQUE(campaign_item_id, task_id)
        );

        CREATE INDEX IF NOT EXISTS idx_coding_benchmark_backlog_scope
            ON coding_benchmark_backlog_items(project_id, session_id, status, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_coding_benchmark_backlog_campaign
            ON coding_benchmark_backlog_items(campaign_id, campaign_item_id);

        CREATE TABLE IF NOT EXISTS coding_improvement_proposals (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            project_id TEXT,
            kind TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'draft',
            source_type TEXT NOT NULL,
            source_id TEXT NOT NULL,
            title TEXT NOT NULL,
            body TEXT NOT NULL,
            payload_json TEXT NOT NULL DEFAULT '{}',
            fingerprint TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            decided_at TEXT,
            apply_result_json TEXT,
            applied_at TEXT,
            promotion_result_json TEXT,
            promoted_at TEXT,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            UNIQUE(session_id, fingerprint)
        );

        CREATE TABLE IF NOT EXISTS coding_workflow_retros (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            project_id TEXT,
            workflow_run_id TEXT NOT NULL UNIQUE,
            run_state TEXT NOT NULL,
            summary TEXT NOT NULL,
            signals_json TEXT NOT NULL DEFAULT '[]',
            recommendations_json TEXT NOT NULL DEFAULT '[]',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (workflow_run_id) REFERENCES workflow_runs(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_coding_improvement_session
            ON coding_improvement_proposals(session_id, status, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_coding_improvement_project
            ON coding_improvement_proposals(project_id, status, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_coding_workflow_retros_session
            ON coding_workflow_retros(session_id, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_coding_workflow_retros_project
            ON coding_workflow_retros(project_id, updated_at DESC);",
    )?;
    ensure_column(
        conn,
        "coding_improvement_proposals",
        "apply_result_json",
        "ALTER TABLE coding_improvement_proposals ADD COLUMN apply_result_json TEXT;",
    )?;
    ensure_column(
        conn,
        "coding_improvement_proposals",
        "applied_at",
        "ALTER TABLE coding_improvement_proposals ADD COLUMN applied_at TEXT;",
    )?;
    ensure_column(
        conn,
        "coding_improvement_proposals",
        "promotion_result_json",
        "ALTER TABLE coding_improvement_proposals ADD COLUMN promotion_result_json TEXT;",
    )?;
    ensure_column(
        conn,
        "coding_improvement_proposals",
        "promoted_at",
        "ALTER TABLE coding_improvement_proposals ADD COLUMN promoted_at TEXT;",
    )?;
    Ok(())
}

impl SessionDB {
    pub fn update_coding_improvement_proposal_status(
        &self,
        proposal_id: &str,
        status: &str,
    ) -> Result<CodingImprovementProposal> {
        let status = normalize_manual_proposal_status(status)?;
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let current_status = conn
            .query_row(
                "SELECT status FROM coding_improvement_proposals WHERE id = ?1",
                params![proposal_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| anyhow!("coding improvement proposal not found: {proposal_id}"))?;
        match current_status.as_str() {
            "applied" => bail!(
                "coding improvement proposal {proposal_id} is already applied and cannot be manually changed"
            ),
            "promoting" => bail!(
                "coding improvement proposal {proposal_id} is currently promoting and cannot be manually changed"
            ),
            "promoted" => bail!(
                "coding improvement proposal {proposal_id} is already promoted and cannot be manually changed"
            ),
            "promotion_failed" => bail!(
                "coding improvement proposal {proposal_id} has a promotion failure; retry with promote_coding_improvement_proposal"
            ),
            "applying" => bail!(
                "coding improvement proposal {proposal_id} is currently applying and cannot be manually changed"
            ),
            "draft" | "rejected" | "failed" => {}
            other => bail!(
                "coding improvement proposal {proposal_id} has unsupported status: {other}"
            ),
        }
        let changed = conn.execute(
            "UPDATE coding_improvement_proposals
             SET status = ?1,
                updated_at = ?2,
                decided_at = CASE WHEN ?1 = 'draft' THEN NULL ELSE ?2 END,
                apply_result_json = CASE WHEN ?1 = 'draft' THEN NULL ELSE apply_result_json END,
                applied_at = CASE WHEN ?1 = 'draft' THEN NULL ELSE applied_at END,
                promotion_result_json = CASE WHEN ?1 = 'draft' THEN NULL ELSE promotion_result_json END,
                promoted_at = CASE WHEN ?1 = 'draft' THEN NULL ELSE promoted_at END
             WHERE id = ?3 AND status = ?4",
            params![status, now, proposal_id, current_status],
        )?;
        if changed == 0 {
            bail!("coding improvement proposal {proposal_id} changed while updating status");
        }
        drop(conn);
        self.get_coding_improvement_proposal(proposal_id)?
            .ok_or_else(|| anyhow!("coding improvement proposal vanished after update"))
    }

    pub fn record_coding_eval_run(
        &self,
        input: RecordCodingEvalRunInput,
    ) -> Result<CodingEvalRunRecord> {
        let status = normalize_eval_status(&input.status)?;
        let (session_id, project_id) =
            self.resolve_durable_coding_record_scope(input.session_id, input.project_id, "eval")?;
        let suite = input.suite.trim();
        let name = input.name.trim();
        if suite.is_empty() || name.is_empty() {
            bail!("coding eval run suite and name must not be empty");
        }
        let id = format!("cer_{}", uuid::Uuid::new_v4().simple());
        let now = now_rfc3339();
        let metrics_json = stable_json(&input.metrics)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO coding_eval_runs (
                id, session_id, project_id, suite, name, status, metrics_json,
                source_type, source_id, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                session_id,
                project_id,
                suite,
                name,
                status,
                metrics_json,
                input.source_type,
                input.source_id,
                now
            ],
        )?;
        drop(conn);
        self.get_coding_eval_run(&id)?
            .ok_or_else(|| anyhow!("coding eval run vanished after insert"))
    }

    pub fn record_coding_eval_pack_run(
        &self,
        input: RecordCodingEvalPackRunInput,
    ) -> Result<CodingEvalPackRunRecord> {
        let (session_id, project_id) = self.resolve_durable_coding_record_scope(
            input.session_id,
            input.project_id,
            "eval pack",
        )?;
        let baseline_kind = normalize_baseline_kind(input.baseline_kind.as_deref());
        let label = input
            .label
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let status = if input.report.passed {
            "passed"
        } else if input.report.automated_cases == 0 {
            "skipped"
        } else {
            "failed"
        };
        let id = format!("cepr_{}", uuid::Uuid::new_v4().simple());
        let now = now_rfc3339();
        let report_json = stable_json(&serde_json::to_value(&input.report)?)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO coding_eval_pack_runs (
                id, session_id, project_id, pack_id, source_doc, label, baseline_kind, status,
                selected_cases, automated_cases, skipped_cases, passed_cases, failed_cases,
                total_checks, report_json, source_type, source_id, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                id,
                session_id,
                project_id,
                input.report.pack_id,
                input.report.source_doc,
                label,
                baseline_kind,
                status,
                input.report.selected_cases as i64,
                input.report.automated_cases as i64,
                input.report.skipped_cases as i64,
                input.report.passed_cases as i64,
                input.report.failed_cases as i64,
                input.report.total_checks as i64,
                report_json,
                input.source_type,
                input.source_id,
                now,
            ],
        )?;
        drop(conn);
        self.get_coding_eval_pack_run(&id)?
            .ok_or_else(|| anyhow!("coding eval pack run vanished after insert"))
    }

    pub fn record_coding_strategy_effect_run(
        &self,
        input: RecordCodingStrategyEffectRunInput,
    ) -> Result<CodingStrategyEffectRunRecord> {
        let (session_id, project_id) = self.resolve_durable_coding_record_scope(
            input.session_id,
            input.project_id,
            "strategy effect",
        )?;
        let report = input.report;
        let id = format!("cser_{}", uuid::Uuid::new_v4().simple());
        let now = now_rfc3339();
        let report_json = stable_json(&serde_json::to_value(&report)?)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO coding_strategy_effect_runs (
                id, session_id, project_id, strategy_type, baseline_label, candidate_label,
                baseline_pack_run_id, candidate_pack_run_id, verdict, compared_cases,
                pass_rate_delta, average_score_delta, context_recall_delta,
                validation_violation_delta, scope_creep_delta, execution_failure_delta,
                report_json, source_type, source_id, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            params![
                id,
                session_id,
                project_id,
                report.strategy_type,
                report.baseline_label,
                report.candidate_label,
                input.baseline_pack_run_id,
                input.candidate_pack_run_id,
                report.verdict,
                report.compared_cases as i64,
                report.summary.pass_rate_delta,
                report.summary.average_score_delta,
                report.summary.context_recall_delta,
                report.summary.validation_violation_delta as i64,
                report.summary.scope_creep_delta as i64,
                report.summary.execution_failure_delta as i64,
                report_json,
                input.source_type,
                input.source_id,
                now,
            ],
        )?;
        drop(conn);
        self.get_coding_strategy_effect_run(&id)?
            .ok_or_else(|| anyhow!("coding strategy effect run vanished after insert"))
    }

    pub fn evaluate_coding_eval_release_gate(
        &self,
        input: CodingEvalReleaseGateInput,
    ) -> Result<CodingEvalReleaseGateReport> {
        let thresholds = release_gate_thresholds(&input);
        let scope = self.resolve_coding_eval_release_gate_scope(&input)?;
        let summary = self.coding_eval_release_gate_summary(&scope)?;
        let mut checks = Vec::new();

        push_gate_check(
            &mut checks,
            "pack_run_sample",
            if summary.pack_runs < thresholds.min_pack_runs {
                "insufficient_data"
            } else {
                "passed"
            },
            "required",
            format!("at least {} pack run(s)", thresholds.min_pack_runs),
            format!("{} pack run(s)", summary.pack_runs),
            "Gold Task Pack history proves the gate is judging recent product behavior.",
        );

        if thresholds.require_external_model_pack {
            push_gate_check(
                &mut checks,
                "external_model_baseline",
                if summary.external_model_pack_runs == 0 {
                    "insufficient_data"
                } else {
                    "passed"
                },
                "required",
                "at least 1 external_model pack run",
                format!("{} external_model pack run(s)", summary.external_model_pack_runs),
                "External provider baselines stay separate from fixture and mock-provider baselines.",
            );
        }

        push_gate_check(
            &mut checks,
            "strategy_effect_sample",
            if summary.strategy_effect_runs < thresholds.min_strategy_effect_runs {
                "insufficient_data"
            } else {
                "passed"
            },
            "required",
            format!(
                "at least {} strategy effect run(s)",
                thresholds.min_strategy_effect_runs
            ),
            format!("{} strategy effect run(s)", summary.strategy_effect_runs),
            "Strategy history is optional by default, but release profiles can require it.",
        );

        let pack_pass_rate_status = match summary.pack_pass_rate {
            Some(rate) if rate + f64::EPSILON >= thresholds.min_pack_pass_rate => "passed",
            Some(_) => "failed",
            None if thresholds.min_pack_runs == 0 => "passed",
            None => "insufficient_data",
        };
        push_gate_check(
            &mut checks,
            "pack_pass_rate",
            pack_pass_rate_status,
            "blocking",
            format!("pack pass rate >= {:.3}", thresholds.min_pack_pass_rate),
            summary
                .pack_pass_rate
                .map(|rate| format!("{rate:.3}"))
                .unwrap_or_else(|| "no passed/failed pack runs".to_string()),
            "Pack-level pass rate is the primary release quality signal.",
        );

        push_gate_check(
            &mut checks,
            "strategy_regressions",
            if summary.regressed_strategy_effects > thresholds.max_regressed_strategy_effects {
                "failed"
            } else {
                "passed"
            },
            "blocking",
            format!(
                "<= {} regressed strategy effect(s)",
                thresholds.max_regressed_strategy_effects
            ),
            format!("{} regressed", summary.regressed_strategy_effects),
            "A candidate strategy should not make the gold pack worse.",
        );

        push_gate_check(
            &mut checks,
            "mixed_strategy_effects",
            if summary.mixed_strategy_effects > thresholds.max_mixed_strategy_effects {
                "failed"
            } else {
                "passed"
            },
            "blocking",
            format!(
                "<= {} mixed strategy effect(s)",
                thresholds.max_mixed_strategy_effects
            ),
            format!("{} mixed", summary.mixed_strategy_effects),
            "Mixed strategy outcomes require explicit review before promotion.",
        );

        push_gate_check(
            &mut checks,
            "missing_tool_calls",
            if summary.missing_tool_call_runs > thresholds.max_missing_tool_call_runs {
                "failed"
            } else {
                "passed"
            },
            "blocking",
            format!(
                "<= {} agent eval run(s) with no tool calls",
                thresholds.max_missing_tool_call_runs
            ),
            format!(
                "{} missing tool-call run(s)",
                summary.missing_tool_call_runs
            ),
            "Agent-mode evals must prove the model can drive the tool loop, not only emit text.",
        );

        push_gate_check(
            &mut checks,
            "validation_violation_delta",
            if summary.validation_violation_delta > thresholds.max_validation_violation_delta {
                "failed"
            } else {
                "passed"
            },
            "blocking",
            format!(
                "<= {} validation violation delta",
                thresholds.max_validation_violation_delta
            ),
            summary.validation_violation_delta.to_string(),
            "Strategy changes should not increase validation violations.",
        );

        push_gate_check(
            &mut checks,
            "scope_creep_delta",
            if summary.scope_creep_delta > thresholds.max_scope_creep_delta {
                "failed"
            } else {
                "passed"
            },
            "blocking",
            format!("<= {} scope creep delta", thresholds.max_scope_creep_delta),
            summary.scope_creep_delta.to_string(),
            "Strategy changes should not expand edits beyond the intended task scope.",
        );

        let has_failed = checks.iter().any(|check| check.status == "failed");
        let has_insufficient_data = checks
            .iter()
            .any(|check| check.status == "insufficient_data");
        let status = if has_failed {
            "failed"
        } else if has_insufficient_data {
            "insufficient_data"
        } else {
            "passed"
        };

        Ok(CodingEvalReleaseGateReport {
            generated_at: now_rfc3339(),
            status: status.to_string(),
            scope: scope.scope,
            session_id: scope.session_id,
            project_id: scope.project_id,
            window_days: scope.window_days,
            since: scope.since,
            thresholds,
            summary,
            checks,
        })
    }

    pub fn evaluate_coding_learning_generalization(
        &self,
        input: CodingLearningGeneralizationInput,
    ) -> Result<CodingLearningGeneralizationReport> {
        let thresholds = learning_generalization_thresholds(&input);
        let scope = self.resolve_coding_learning_generalization_scope(&input)?;
        let mut projects = self.coding_learning_generalization_projects(&scope, &thresholds)?;
        let mut summary = CodingLearningGeneralizationSummary::default();

        for project in &projects {
            summary.projects_evaluated += 1;
            summary.total_promoted_learning += project.promoted_learning;
            summary.total_pack_runs += project.pack_runs;
            summary.total_strategy_effect_runs += project.strategy_effect_runs;
            if project.promoted_learning > 0 {
                summary.projects_with_promoted_learning += 1;
            }
            if project.pack_runs > 0 {
                summary.projects_with_pack_runs += 1;
            }
            if project.strategy_effect_runs > 0 {
                summary.projects_with_strategy_effects += 1;
            }
            if project.external_model_pack_runs > 0 {
                summary.projects_with_external_model_pack += 1;
            }
            if project.regressed_strategy_effects > 0 {
                summary.regressed_projects += 1;
            }
            if project.mixed_strategy_effects > 0 {
                summary.mixed_projects += 1;
            }
            match project.status.as_str() {
                "passed" => summary.passed_projects += 1,
                "failed" => summary.failed_projects += 1,
                _ => summary.insufficient_projects += 1,
            }
        }

        let mut checks = Vec::new();
        push_generalization_check(
            &mut checks,
            "project_sample",
            if summary.projects_evaluated < thresholds.min_projects {
                "insufficient_data"
            } else {
                "passed"
            },
            "required",
            format!("at least {} project(s)", thresholds.min_projects),
            format!("{} project(s)", summary.projects_evaluated),
            "Cross-project learning needs evidence outside a single project.",
        );

        if thresholds.require_promoted_learning {
            push_generalization_check(
                &mut checks,
                "promoted_learning_sample",
                if summary.projects_with_promoted_learning < thresholds.min_projects {
                    "insufficient_data"
                } else {
                    "passed"
                },
                "required",
                format!(
                    "promoted learning in at least {} project(s)",
                    thresholds.min_projects
                ),
                format!(
                    "{} project(s), {} promoted artifact(s)",
                    summary.projects_with_promoted_learning, summary.total_promoted_learning
                ),
                "Only promoted guidance, workflow, or skill artifacts count as durable learning.",
            );
        }

        push_generalization_check(
            &mut checks,
            "pack_history_sample",
            if summary.projects_with_pack_runs < thresholds.min_projects {
                "insufficient_data"
            } else {
                "passed"
            },
            "required",
            format!(
                "pack history in at least {} project(s)",
                thresholds.min_projects
            ),
            format!(
                "{} project(s), {} pack run(s)",
                summary.projects_with_pack_runs, summary.total_pack_runs
            ),
            "Gold Task Pack history is the comparable quality signal across projects.",
        );

        if thresholds.require_external_model_pack {
            push_generalization_check(
                &mut checks,
                "external_model_project_sample",
                if summary.projects_with_external_model_pack < thresholds.min_projects {
                    "insufficient_data"
                } else {
                    "passed"
                },
                "required",
                format!(
                    "external_model pack history in at least {} project(s)",
                    thresholds.min_projects
                ),
                format!("{} project(s)", summary.projects_with_external_model_pack),
                "External provider evidence stays separate from deterministic and mock baselines.",
            );
        }

        push_generalization_check(
            &mut checks,
            "project_quality",
            if summary.failed_projects > 0 {
                "failed"
            } else if summary.passed_projects < thresholds.min_projects {
                "insufficient_data"
            } else {
                "passed"
            },
            "blocking",
            format!(
                "at least {} passed project(s), 0 failed project(s)",
                thresholds.min_projects
            ),
            format!(
                "{} passed, {} failed, {} insufficient",
                summary.passed_projects, summary.failed_projects, summary.insufficient_projects
            ),
            "Learning should generalize without dragging any measured project below its quality bar.",
        );

        push_generalization_check(
            &mut checks,
            "strategy_regression_projects",
            if summary.regressed_projects > thresholds.max_regressed_projects {
                "failed"
            } else {
                "passed"
            },
            "blocking",
            format!(
                "<= {} project(s) with strategy regression",
                thresholds.max_regressed_projects
            ),
            format!("{} project(s)", summary.regressed_projects),
            "A cross-project learning artifact should not regress any project strategy evidence.",
        );

        push_generalization_check(
            &mut checks,
            "mixed_strategy_projects",
            if summary.mixed_projects > thresholds.max_mixed_projects {
                "failed"
            } else {
                "passed"
            },
            "blocking",
            format!(
                "<= {} project(s) with mixed strategy effects",
                thresholds.max_mixed_projects
            ),
            format!("{} project(s)", summary.mixed_projects),
            "Mixed outcomes require human review before claiming broad generalization.",
        );

        let has_failed = checks.iter().any(|check| check.status == "failed");
        let has_insufficient_data = checks
            .iter()
            .any(|check| check.status == "insufficient_data");
        let status = if has_failed {
            "failed"
        } else if has_insufficient_data {
            "insufficient_data"
        } else {
            "passed"
        };

        projects.sort_by(|a, b| {
            project_status_rank(&a.status)
                .cmp(&project_status_rank(&b.status))
                .then_with(|| b.promoted_learning.cmp(&a.promoted_learning))
                .then_with(|| b.pack_runs.cmp(&a.pack_runs))
                .then_with(|| a.project_id.cmp(&b.project_id))
        });

        Ok(CodingLearningGeneralizationReport {
            generated_at: now_rfc3339(),
            status: status.to_string(),
            scope: scope.scope,
            session_id: scope.session_id,
            project_id: scope.project_id,
            window_days: scope.window_days,
            since: scope.since,
            source_type: scope.source_type,
            source_id: scope.source_id,
            proposal_kinds: scope.proposal_kinds,
            thresholds,
            summary,
            projects,
            checks,
        })
    }

    pub fn get_coding_benchmark_center(
        &self,
        input: CodingBenchmarkCenterInput,
    ) -> Result<CodingBenchmarkCenterReport> {
        let scope = self.resolve_coding_benchmark_center_scope(&input)?;
        let summary = self.coding_benchmark_center_summary(&scope)?;
        let mut baselines = self.coding_benchmark_center_baselines(&scope)?;
        let runs = self.coding_benchmark_center_runs(&scope)?;
        let release_gate = self.evaluate_coding_eval_release_gate(CodingEvalReleaseGateInput {
            session_id: scope.session_id.clone(),
            project_id: scope.project_id.clone(),
            window_days: Some(scope.window_days),
            require_external_model_pack: input.require_external_model_baseline,
            ..Default::default()
        })?;
        let generalization_gate =
            self.evaluate_coding_learning_generalization(CodingLearningGeneralizationInput {
                session_id: scope.session_id.clone(),
                project_id: scope.project_id.clone(),
                window_days: Some(scope.window_days),
                require_external_model_pack: input.require_external_model_baseline,
                ..Default::default()
            })?;

        baselines.sort_by(|a, b| {
            b.runs
                .cmp(&a.runs)
                .then_with(|| a.baseline_kind.cmp(&b.baseline_kind))
        });

        let mut checks = Vec::new();
        push_benchmark_check(
            &mut checks,
            "benchmark_history",
            if summary.total_runs == 0 {
                "insufficient_data"
            } else {
                "passed"
            },
            "required",
            "at least 1 recorded benchmark run",
            format!("{} run(s)", summary.total_runs),
            "Benchmark Run Center is backed by durable Gold Task Pack history.",
        );
        push_benchmark_check(
            &mut checks,
            "latest_pack_run",
            match summary.latest_run_status.as_deref() {
                Some("passed") => "passed",
                Some("failed") => "failed",
                Some(_) => "insufficient_data",
                None => "insufficient_data",
            },
            "required",
            "latest recorded pack run passed",
            summary
                .latest_run_status
                .clone()
                .unwrap_or_else(|| "none".to_string()),
            "The newest benchmark run is the first signal users see in the run center.",
        );
        push_benchmark_check(
            &mut checks,
            "release_gate",
            release_gate.status.clone(),
            "required",
            "release gate passed",
            release_gate.status.clone(),
            "Release Gate combines pack quality, strategy regressions, and missing tool-call evidence.",
        );
        push_benchmark_check(
            &mut checks,
            "external_model_baseline",
            if summary.external_model_runs > 0 {
                "passed"
            } else {
                "insufficient_data"
            },
            if input.require_external_model_baseline {
                "required"
            } else {
                "advisory"
            },
            "at least 1 external_model pack run",
            format!("{} run(s)", summary.external_model_runs),
            "External baselines are never inferred from deterministic or mock runs.",
        );
        push_benchmark_check(
            &mut checks,
            "learning_generalization",
            generalization_gate.status.clone(),
            if input.require_learning_generalization {
                "required"
            } else {
                "advisory"
            },
            "learning generalization gate passed",
            generalization_gate.status.clone(),
            "Cross-project promoted learning evidence is kept visible next to benchmark results.",
        );

        let has_failed = checks.iter().any(|check| check.status == "failed");
        let has_required_insufficient = checks
            .iter()
            .any(|check| check.severity == "required" && check.status == "insufficient_data");
        let status = if has_failed {
            "failed"
        } else if has_required_insufficient {
            "insufficient_data"
        } else {
            "passed"
        };

        Ok(CodingBenchmarkCenterReport {
            generated_at: now_rfc3339(),
            status: status.to_string(),
            scope: scope.scope,
            session_id: scope.session_id,
            project_id: scope.project_id,
            window_days: scope.window_days,
            since: scope.since,
            summary,
            baselines,
            runs,
            checks,
            release_gate,
            generalization_gate,
        })
    }

    pub fn create_coding_benchmark_campaign(
        &self,
        input: CodingBenchmarkCampaignCreateInput,
    ) -> Result<CodingBenchmarkCampaign> {
        let (session_id, project_id) = self.resolve_durable_coding_record_scope(
            input
                .session_id
                .or_else(|| input.gold_task_input.session_id.clone()),
            input
                .project_id
                .or_else(|| input.gold_task_input.project_id.clone()),
            "benchmark campaign",
        )?;
        let models = normalize_benchmark_campaign_models(input.models)?;
        let has_external_model = models
            .iter()
            .any(|model| model.provider_id.is_some() || model.model_id.is_some());
        let execution_mode = input
            .gold_task_input
            .execution_mode
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if has_external_model {
                    "agent".to_string()
                } else {
                    "fixture_patch".to_string()
                }
            });
        let baseline_kind = input
            .gold_task_input
            .baseline_kind
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if has_external_model {
                    "external_model".to_string()
                } else {
                    "deterministic_mock".to_string()
                }
            });
        if baseline_kind == "external_model" && !has_external_model {
            bail!("external_model benchmark campaign requires at least one provider/model");
        }
        let mut sanitized_input = input.gold_task_input.clone();
        sanitized_input.session_id = session_id.clone();
        sanitized_input.project_id = project_id.clone();
        sanitized_input.providers.clear();
        sanitized_input.model_chain.clear();
        sanitized_input.execution_mode = Some(execution_mode.clone());
        sanitized_input.baseline_kind = Some(baseline_kind.clone());
        sanitized_input.source_type = Some("benchmark_campaign".to_string());
        sanitized_input.source_id = None;
        let task_filter_json = stable_json(&serde_json::to_value(&sanitized_input)?)?;
        let model_matrix_json = stable_json(&serde_json::to_value(&models)?)?;
        let name = input
            .name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if has_external_model {
                    "External model benchmark campaign".to_string()
                } else {
                    "Deterministic benchmark campaign".to_string()
                }
            });
        let id = format!("cbc_{}", uuid::Uuid::new_v4().simple());
        let now = now_rfc3339();
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO coding_benchmark_campaigns (
                id, session_id, project_id, name, status, task_pack_id, source_doc,
                execution_mode, baseline_kind, task_filter_json, model_matrix_json,
                max_budget_usd, timeout_secs, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 'queued', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)",
            params![
                id,
                session_id,
                project_id,
                name,
                "phase5-gold-task-pack",
                "docs/roadmap/coding-eval-tasks.md",
                execution_mode,
                baseline_kind,
                task_filter_json,
                model_matrix_json,
                input.max_budget_usd,
                input.timeout_secs.map(|value| value as i64),
                now,
            ],
        )?;
        for model in &models {
            let item_id = format!("cbci_{}", uuid::Uuid::new_v4().simple());
            tx.execute(
                "INSERT INTO coding_benchmark_campaign_items (
                    id, campaign_id, provider_id, model_id, label, status,
                    created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6, ?6)",
                params![
                    item_id,
                    id,
                    model.provider_id,
                    model.model_id,
                    model.label,
                    now,
                ],
            )?;
        }
        tx.commit()?;
        drop(conn);
        self.get_coding_benchmark_campaign(&id)?
            .ok_or_else(|| anyhow!("benchmark campaign vanished after insert"))
    }

    pub fn list_coding_benchmark_campaigns(
        &self,
        input: CodingBenchmarkCampaignListInput,
    ) -> Result<Vec<CodingBenchmarkCampaign>> {
        let (session_id, project_id) = self.resolve_durable_coding_record_scope(
            input.session_id,
            input.project_id,
            "benchmark campaign",
        )?;
        let limit = input
            .limit
            .unwrap_or(DEFAULT_BENCHMARK_CAMPAIGN_LIMIT)
            .clamp(1, MAX_BENCHMARK_CAMPAIGN_LIMIT);
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut clauses = Vec::new();
        let mut params = Vec::new();
        if let Some(project_id) = project_id.as_ref() {
            clauses.push("project_id = ?".to_string());
            params.push(project_id.clone());
        } else if let Some(session_id) = session_id.as_ref() {
            clauses.push("session_id = ?".to_string());
            params.push(session_id.clone());
        }
        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        params.push(limit.to_string());
        let mut stmt = conn.prepare(&format!(
            "SELECT id FROM coding_benchmark_campaigns
             {where_sql}
             ORDER BY created_at DESC, id DESC
             LIMIT ?"
        ))?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
            row.get::<_, String>(0)
        })?;
        let ids = collect_rows(rows)?;
        drop(stmt);
        drop(conn);
        ids.into_iter()
            .filter_map(|id| self.get_coding_benchmark_campaign(&id).transpose())
            .collect()
    }

    pub fn get_coding_benchmark_campaign(
        &self,
        campaign_id: &str,
    ) -> Result<Option<CodingBenchmarkCampaign>> {
        let campaign_id = campaign_id.trim();
        if campaign_id.is_empty() {
            return Ok(None);
        }
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let row = conn
            .query_row(
                "SELECT id, session_id, project_id, name, status, task_pack_id, source_doc,
                        execution_mode, baseline_kind, task_filter_json, model_matrix_json,
                        max_budget_usd, timeout_secs, created_at, updated_at, started_at,
                        finished_at, error
                 FROM coding_benchmark_campaigns
                 WHERE id = ?1",
                params![campaign_id],
                coding_benchmark_campaign_from_row,
            )
            .optional()?;
        let Some(mut campaign) = row else {
            return Ok(None);
        };
        campaign.items = self.coding_benchmark_campaign_items_locked(&conn, campaign_id)?;
        campaign.summary = benchmark_campaign_summary(&campaign.items);
        Ok(Some(campaign))
    }

    pub fn cancel_coding_benchmark_campaign(
        &self,
        campaign_id: &str,
    ) -> Result<Option<CodingBenchmarkCampaign>> {
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE coding_benchmark_campaigns
             SET status = CASE WHEN status IN ('passed','failed','partial','cancelled','interrupted') THEN status ELSE 'cancel_requested' END,
                 updated_at = ?2,
                 error = CASE WHEN status IN ('passed','failed','partial','cancelled','interrupted') THEN error ELSE 'Cancellation requested' END
             WHERE id = ?1",
            params![campaign_id, now],
        )?;
        if changed > 0 {
            conn.execute(
                "UPDATE coding_benchmark_campaign_items
                 SET status = 'cancelled', updated_at = ?2, finished_at = ?2, error = 'Cancelled before run'
                 WHERE campaign_id = ?1 AND status = 'queued'",
                params![campaign_id, now],
            )?;
        }
        drop(conn);
        self.get_coding_benchmark_campaign(campaign_id)
    }

    pub fn prepare_coding_benchmark_campaign_run(
        &self,
        campaign_id: &str,
        retry_failed_only: bool,
    ) -> Result<Vec<CodingBenchmarkCampaignItem>> {
        let now = now_rfc3339();
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        if retry_failed_only {
            tx.execute(
                "UPDATE coding_benchmark_campaign_items
                 SET status = 'queued', updated_at = ?2, error = NULL
                 WHERE campaign_id = ?1 AND status IN ('failed','interrupted','cancelled')",
                params![campaign_id, now],
            )?;
        }
        tx.execute(
            "UPDATE coding_benchmark_campaigns
             SET status = 'running', started_at = COALESCE(started_at, ?2), updated_at = ?2,
                 finished_at = NULL, error = NULL
             WHERE id = ?1 AND status NOT IN ('cancel_requested','passed','failed','partial','cancelled')",
            params![campaign_id, now],
        )?;
        tx.commit()?;
        drop(conn);
        let campaign = self
            .get_coding_benchmark_campaign(campaign_id)?
            .ok_or_else(|| anyhow!("benchmark campaign not found: {campaign_id}"))?;
        Ok(campaign
            .items
            .into_iter()
            .filter(|item| item.status == "queued")
            .collect())
    }

    pub fn is_coding_benchmark_campaign_cancel_requested(&self, campaign_id: &str) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let status = conn
            .query_row(
                "SELECT status FROM coding_benchmark_campaigns WHERE id = ?1",
                params![campaign_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(matches!(
            status.as_deref(),
            Some("cancel_requested" | "cancelled")
        ))
    }

    pub fn mark_coding_benchmark_campaign_item_running(
        &self,
        item_id: &str,
    ) -> Result<Option<CodingBenchmarkCampaignItem>> {
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE coding_benchmark_campaign_items
             SET status = 'running', attempt = attempt + 1, started_at = ?2,
                 updated_at = ?2, error = NULL
             WHERE id = ?1 AND status = 'queued'",
            params![item_id, now],
        )?;
        let item = conn
            .query_row(
                "SELECT id, campaign_id, provider_id, model_id, label, status, attempt,
                        pack_run_id, selected_cases, passed_cases, failed_cases, skipped_cases,
                        total_checks, started_at, finished_at, error
                 FROM coding_benchmark_campaign_items WHERE id = ?1",
                params![item_id],
                coding_benchmark_campaign_item_from_row,
            )
            .optional()?;
        Ok(item)
    }

    pub fn finish_coding_benchmark_campaign_item(
        &self,
        item_id: &str,
        report: &GoldTaskPackReport,
    ) -> Result<()> {
        let now = now_rfc3339();
        let status = if report.passed {
            "passed"
        } else if report.automated_cases == 0 {
            "skipped"
        } else {
            "failed"
        };
        let report_json = stable_json(&serde_json::to_value(report)?)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE coding_benchmark_campaign_items
             SET status = ?2, pack_run_id = ?3, selected_cases = ?4, passed_cases = ?5,
                 failed_cases = ?6, skipped_cases = ?7, total_checks = ?8,
                 report_json = ?9, error = NULL, updated_at = ?10, finished_at = ?10
             WHERE id = ?1",
            params![
                item_id,
                status,
                report.pack_run_id,
                report.selected_cases as i64,
                report.passed_cases as i64,
                report.failed_cases as i64,
                report.skipped_cases as i64,
                report.total_checks as i64,
                report_json,
                now,
            ],
        )?;
        Ok(())
    }

    pub fn fail_coding_benchmark_campaign_item(&self, item_id: &str, error: &str) -> Result<()> {
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE coding_benchmark_campaign_items
             SET status = 'failed', error = ?2, updated_at = ?3, finished_at = ?3
             WHERE id = ?1",
            params![item_id, truncate_for_storage(error, 2000), now],
        )?;
        Ok(())
    }

    pub fn complete_coding_benchmark_campaign(&self, campaign_id: &str) -> Result<()> {
        let now = now_rfc3339();
        let campaign = self
            .get_coding_benchmark_campaign(campaign_id)?
            .ok_or_else(|| anyhow!("benchmark campaign not found: {campaign_id}"))?;
        let summary = benchmark_campaign_summary(&campaign.items);
        let status = if campaign.status == "cancel_requested" || summary.cancelled_items > 0 {
            "cancelled"
        } else if summary.running_items > 0 || summary.queued_items > 0 {
            "interrupted"
        } else if summary.failed_items > 0 || summary.interrupted_items > 0 {
            if summary.passed_items > 0 || summary.skipped_items > 0 {
                "partial"
            } else {
                "failed"
            }
        } else if summary.passed_items > 0 && summary.failed_items == 0 {
            "passed"
        } else if summary.skipped_items > 0 {
            "partial"
        } else {
            "failed"
        };
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE coding_benchmark_campaigns
             SET status = ?2, updated_at = ?3, finished_at = ?3,
                 error = CASE WHEN ?2 = 'passed' THEN NULL ELSE error END
             WHERE id = ?1",
            params![campaign_id, status, now],
        )?;
        Ok(())
    }

    pub fn get_benchmark_leaderboard(
        &self,
        input: CodingBenchmarkLeaderboardInput,
    ) -> Result<CodingBenchmarkLeaderboardReport> {
        let scope = self.resolve_benchmark_leaderboard_scope(
            input.session_id,
            input.project_id,
            input.window_days,
            input.campaign_ids,
            input.limit,
            input.min_items,
        )?;
        self.build_benchmark_leaderboard(scope)
    }

    pub fn compare_benchmark_models(
        &self,
        input: CodingBenchmarkComparisonInput,
    ) -> Result<CodingBenchmarkLeaderboardReport> {
        let scope = self.resolve_benchmark_leaderboard_scope(
            input.session_id,
            input.project_id,
            input.window_days,
            input.campaign_ids,
            input.limit,
            input.min_items,
        )?;
        self.build_benchmark_leaderboard(scope)
    }

    pub fn import_benchmark_task_pack(
        &self,
        input: CodingBenchmarkTaskPackImportInput,
    ) -> Result<CodingBenchmarkTaskPack> {
        if !input.explicit_import_consent {
            bail!(
                "benchmark task pack import requires explicitImportConsent=true; Hope will not implicitly scan or upload private repositories"
            );
        }
        let manifest = normalize_benchmark_task_pack_manifest(input.manifest)?;
        let validation = validate_benchmark_task_pack_manifest(&manifest);
        if validation.status == "failed" {
            let failed = validation
                .checks
                .iter()
                .filter(|check| check.status == "failed")
                .map(|check| check.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("benchmark task pack manifest failed validation: {failed}");
        }

        let now = now_rfc3339();
        let pack_row_id = format!("cbtp_{}", uuid::Uuid::new_v4().simple());
        let status = normalize_benchmark_pack_status(manifest.status.as_deref())?;
        let imported_from = input
            .imported_from
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let manifest_json = serde_json::to_string(&manifest)?;
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO coding_benchmark_task_packs (
                id, pack_id, pack_version, name, description, status, source_kind,
                source_uri, repo_template, license_note, privacy_note, redaction_status,
                imported_from, manifest_json, created_at, updated_at, activated_at, archived_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                ?13, ?14, ?15, ?15, ?16, ?17
             )",
            params![
                pack_row_id,
                manifest.pack_id,
                manifest.version,
                manifest.name,
                manifest.description,
                status,
                manifest.source_kind,
                manifest.source_uri,
                manifest.repo_template,
                manifest.license_note,
                manifest.privacy_note,
                manifest.redaction_status,
                imported_from,
                manifest_json,
                now,
                if status == "active" {
                    Some(now.clone())
                } else {
                    None
                },
                if status == "archived" {
                    Some(now.clone())
                } else {
                    None
                },
            ],
        )
        .map_err(|err| {
            anyhow!(
                "failed to import benchmark task pack {}@{}: {err}",
                manifest.pack_id,
                manifest.version
            )
        })?;

        for task in &manifest.tasks {
            let task_status = normalize_benchmark_task_status(task.status.as_deref())?;
            let risk_flags = benchmark_task_risk_flags(task);
            let fingerprint = benchmark_task_fingerprint(task)?;
            tx.execute(
                "INSERT INTO coding_benchmark_task_pack_tasks (
                    id, pack_row_id, pack_id, pack_version, task_id, task_version,
                    title, status, task_type, difficulty, language, framework,
                    source_uri, repo_template, tags_json, success_criteria_json,
                    validation_commands_json, allowed_paths_json, forbidden_paths_json,
                    calibration_notes_json, calibrated_at, license_note, privacy_note,
                    redaction_status, risk_flags_json, fingerprint, created_at, updated_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22,
                    ?23, ?24, ?25, ?26, ?27, ?27
                 )",
                params![
                    format!("cbtpt_{}", uuid::Uuid::new_v4().simple()),
                    pack_row_id,
                    manifest.pack_id,
                    manifest.version,
                    task.task_id,
                    task.version,
                    task.title,
                    task_status,
                    task.task_type,
                    task.difficulty,
                    task.language,
                    task.framework,
                    task.source_uri,
                    task.repo_template,
                    serde_json::to_string(&task.tags)?,
                    serde_json::to_string(&task.success_criteria)?,
                    serde_json::to_string(&task.validation_commands)?,
                    serde_json::to_string(&task.allowed_paths)?,
                    serde_json::to_string(&task.forbidden_paths)?,
                    serde_json::to_string(&task.calibration_notes)?,
                    task.calibrated_at,
                    task.license_note,
                    task.privacy_note,
                    task.redaction_status
                        .clone()
                        .unwrap_or_else(|| manifest.redaction_status.clone()),
                    serde_json::to_string(&risk_flags)?,
                    fingerprint,
                    now,
                ],
            )?;
        }
        tx.commit()?;
        drop(conn);

        self.get_benchmark_task_pack(&manifest.pack_id, &manifest.version)?
            .ok_or_else(|| anyhow!("benchmark task pack vanished after import"))
    }

    pub fn list_benchmark_task_packs(
        &self,
        input: CodingBenchmarkTaskPackListInput,
    ) -> Result<Vec<CodingBenchmarkTaskPack>> {
        let status = input
            .status
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase);
        if let Some(status) = status.as_deref() {
            normalize_benchmark_pack_status(Some(status))?;
        }
        let limit = input
            .limit
            .unwrap_or(DEFAULT_BENCHMARK_CORPUS_LIMIT)
            .clamp(1, MAX_BENCHMARK_CORPUS_LIMIT);
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut clauses = Vec::new();
        let mut params = Vec::new();
        if let Some(status) = status.as_ref() {
            clauses.push("status = ?".to_string());
            params.push(status.clone());
        } else if !input.include_archived {
            clauses.push("status != 'archived'".to_string());
        }
        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        let sql = format!(
            "SELECT id FROM coding_benchmark_task_packs
             {where_sql}
             ORDER BY updated_at DESC, pack_id ASC, pack_version DESC
             LIMIT {limit}"
        );
        let mut stmt = conn.prepare(&sql)?;
        let ids = stmt
            .query_map(params_from_iter(params.iter()), |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        drop(conn);

        ids.into_iter()
            .filter_map(|id| self.get_benchmark_task_pack_by_row_id(&id).transpose())
            .collect()
    }

    pub fn get_benchmark_task_pack(
        &self,
        pack_id: &str,
        version: &str,
    ) -> Result<Option<CodingBenchmarkTaskPack>> {
        let pack_id = pack_id.trim();
        let version = version.trim();
        if pack_id.is_empty() || version.is_empty() {
            bail!("benchmark task pack id and version must not be empty");
        }
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let row_id = conn
            .query_row(
                "SELECT id FROM coding_benchmark_task_packs
                 WHERE pack_id = ?1 AND pack_version = ?2",
                params![pack_id, version],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        drop(conn);
        match row_id {
            Some(id) => self.get_benchmark_task_pack_by_row_id(&id),
            None => Ok(None),
        }
    }

    pub fn update_benchmark_task_pack_status(
        &self,
        input: CodingBenchmarkTaskPackStatusInput,
    ) -> Result<CodingBenchmarkTaskPack> {
        let pack_id = input.pack_id.trim().to_string();
        let version = input.version.trim().to_string();
        if pack_id.is_empty() || version.is_empty() {
            bail!("benchmark task pack id and version must not be empty");
        }
        let status = normalize_benchmark_pack_status(Some(&input.status))?;
        if status == "active" {
            let validation =
                self.validate_benchmark_task_pack(CodingBenchmarkTaskPackValidateInput {
                    pack_id: pack_id.clone(),
                    version: version.clone(),
                })?;
            if validation.status == "failed" {
                bail!("cannot activate benchmark task pack with failed validation");
            }
        }
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE coding_benchmark_task_packs
             SET status = ?3,
                 updated_at = ?4,
                 activated_at = CASE WHEN ?3 = 'active' THEN ?4 ELSE activated_at END,
                 archived_at = CASE WHEN ?3 = 'archived' THEN ?4 ELSE NULL END
             WHERE pack_id = ?1 AND pack_version = ?2",
            params![pack_id, version, status, now],
        )?;
        drop(conn);
        if changed == 0 {
            bail!("benchmark task pack not found: {pack_id}@{version}");
        }
        self.get_benchmark_task_pack(&pack_id, &version)?
            .ok_or_else(|| anyhow!("benchmark task pack not found after status update"))
    }

    pub fn validate_benchmark_task_pack(
        &self,
        input: CodingBenchmarkTaskPackValidateInput,
    ) -> Result<CodingBenchmarkTaskPackValidationReport> {
        let pack = self
            .get_benchmark_task_pack(&input.pack_id, &input.version)?
            .ok_or_else(|| {
                anyhow!(
                    "benchmark task pack not found: {}@{}",
                    input.pack_id,
                    input.version
                )
            })?;
        Ok(validate_benchmark_task_pack(&pack))
    }

    pub fn get_benchmark_corpus_health(
        &self,
        input: CodingBenchmarkCorpusHealthInput,
    ) -> Result<CodingBenchmarkCorpusHealthReport> {
        let stale_after_days = input
            .stale_after_days
            .unwrap_or(DEFAULT_BENCHMARK_CORPUS_STALE_DAYS)
            .clamp(1, MAX_BENCHMARK_CORPUS_STALE_DAYS);
        let packs = self.list_benchmark_task_packs(CodingBenchmarkTaskPackListInput {
            include_archived: true,
            limit: Some(MAX_BENCHMARK_CORPUS_LIMIT),
            ..Default::default()
        })?;
        let mut active_packs = 0usize;
        let mut draft_packs = 0usize;
        let mut archived_packs = 0usize;
        let mut active_tasks = 0usize;
        let mut draft_tasks = 0usize;
        let mut archived_tasks = 0usize;
        let mut difficulty_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut type_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut language_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut fingerprint_tasks: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut stale_tasks = Vec::new();
        let mut gaming_risk_tasks = Vec::new();
        let stale_cutoff = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(stale_after_days as i64))
            .unwrap_or_else(chrono::Utc::now);

        for pack in &packs {
            match pack.status.as_str() {
                "active" => active_packs += 1,
                "archived" => archived_packs += 1,
                _ => draft_packs += 1,
            }
            for task in &pack.tasks {
                let effective_active = pack.status == "active" && task.status == "active";
                let effective_archived = pack.status == "archived" || task.status == "archived";
                if effective_active {
                    active_tasks += 1;
                } else if effective_archived {
                    archived_tasks += 1;
                } else {
                    draft_tasks += 1;
                }
                *difficulty_counts
                    .entry(task.difficulty.clone())
                    .or_default() += 1;
                *type_counts.entry(task.task_type.clone()).or_default() += 1;
                *language_counts
                    .entry(
                        task.language
                            .clone()
                            .unwrap_or_else(|| "unspecified".to_string()),
                    )
                    .or_default() += 1;
                if effective_active {
                    fingerprint_tasks
                        .entry(task.fingerprint.clone())
                        .or_default()
                        .push(format!(
                            "{}@{}:{}@{}",
                            pack.pack_id, pack.version, task.task_id, task.version
                        ));
                    if task.risk_flags.iter().any(|flag| {
                        matches!(
                            flag.as_str(),
                            "missing_validation" | "thin_success_criteria" | "wide_write_surface"
                        )
                    }) {
                        gaming_risk_tasks.push(format!(
                            "{}@{}:{}@{}",
                            pack.pack_id, pack.version, task.task_id, task.version
                        ));
                    }
                    let stale = task
                        .calibrated_at
                        .as_deref()
                        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                        .map(|value| value.with_timezone(&chrono::Utc) < stale_cutoff)
                        .unwrap_or(true);
                    if stale {
                        stale_tasks.push(format!(
                            "{}@{}:{}@{}",
                            pack.pack_id, pack.version, task.task_id, task.version
                        ));
                    }
                }
            }
        }

        let duplicate_tasks = fingerprint_tasks
            .into_iter()
            .filter(|(_, tasks)| tasks.len() > 1)
            .map(|(fingerprint, tasks)| CodingBenchmarkCorpusDuplicate { fingerprint, tasks })
            .collect::<Vec<_>>();
        let mut checks = Vec::new();
        push_benchmark_check(
            &mut checks,
            "task_pack_count",
            if packs.is_empty() {
                "insufficient_data"
            } else {
                "passed"
            },
            if packs.is_empty() { "advisory" } else { "info" },
            "at least 1 imported task pack",
            packs.len().to_string(),
            "The corpus registry must contain explicit owner-imported packs before it can drive benchmark policy.",
        );
        push_benchmark_check(
            &mut checks,
            "active_task_count",
            if active_tasks == 0 {
                "insufficient_data"
            } else {
                "passed"
            },
            if active_tasks == 0 {
                "advisory"
            } else {
                "info"
            },
            "at least 1 active task",
            active_tasks.to_string(),
            "Draft tasks stay visible for curation but do not count as active benchmark coverage.",
        );
        push_benchmark_check(
            &mut checks,
            "duplicate_tasks",
            if duplicate_tasks.is_empty() {
                "passed"
            } else {
                "failed"
            },
            if duplicate_tasks.is_empty() {
                "info"
            } else {
                "warning"
            },
            "0 active duplicate task fingerprints",
            duplicate_tasks.len().to_string(),
            "Duplicate active tasks can make the benchmark easier to overfit.",
        );
        push_benchmark_check(
            &mut checks,
            "gaming_risk",
            if gaming_risk_tasks.is_empty() {
                "passed"
            } else {
                "failed"
            },
            if gaming_risk_tasks.is_empty() { "info" } else { "warning" },
            "0 active tasks with fixture-gaming risk flags",
            gaming_risk_tasks.len().to_string(),
            "Active tasks need clear success criteria, validation commands, and bounded write surfaces.",
        );
        push_benchmark_check(
            &mut checks,
            "calibration_freshness",
            if stale_tasks.is_empty() {
                "passed"
            } else {
                "insufficient_data"
            },
            if stale_tasks.is_empty() { "info" } else { "advisory" },
            format!("all active tasks calibrated within {stale_after_days} days"),
            stale_tasks.len().to_string(),
            "Stale or never-calibrated active tasks should be manually reviewed before strict release gating.",
        );
        let status = if packs.is_empty() || active_tasks == 0 {
            "insufficient_data"
        } else if duplicate_tasks.is_empty() && gaming_risk_tasks.is_empty() {
            "passed"
        } else {
            "failed"
        }
        .to_string();

        Ok(CodingBenchmarkCorpusHealthReport {
            generated_at: now_rfc3339(),
            status,
            stale_after_days,
            packs: packs.len(),
            active_packs,
            draft_packs,
            archived_packs,
            tasks: active_tasks + draft_tasks + archived_tasks,
            active_tasks,
            draft_tasks,
            archived_tasks,
            by_difficulty: metric_buckets_from_counts(difficulty_counts),
            by_task_type: metric_buckets_from_counts(type_counts),
            by_language: metric_buckets_from_counts(language_counts),
            stale_tasks,
            duplicate_tasks,
            gaming_risk_tasks,
            checks,
        })
    }

    pub fn generate_benchmark_report(
        &self,
        input: CodingBenchmarkReportGenerateInput,
    ) -> Result<CodingBenchmarkReport> {
        let report_type = normalize_benchmark_report_type(&input.report_type)?;
        let report_id = format!("cbr_{}", uuid::Uuid::new_v4().simple());
        let generated_at = now_rfc3339();
        let mut title = input
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let campaign_id = input
            .campaign_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mut campaign_ids = input
            .campaign_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .take(MAX_BENCHMARK_CAMPAIGN_LIMIT)
            .collect::<Vec<_>>();
        if let Some(campaign_id) = campaign_id.as_ref() {
            if !campaign_ids.iter().any(|id| id == campaign_id) {
                campaign_ids.push(campaign_id.clone());
            }
        }

        let window_days = input
            .window_days
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS);
        let mut session_id = input
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mut project_id = input
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mut source_type = report_type.clone();
        let mut source_id = report_id.clone();
        let (status, scope, summary, snapshot) = match report_type.as_str() {
            "campaign" => {
                let campaign_id_value = campaign_id
                    .clone()
                    .ok_or_else(|| anyhow!("campaign benchmark report requires campaignId"))?;
                let campaign = self
                    .get_coding_benchmark_campaign(&campaign_id_value)?
                    .ok_or_else(|| anyhow!("benchmark campaign not found: {campaign_id_value}"))?;
                session_id = campaign.session_id.clone().or(session_id);
                project_id = campaign.project_id.clone().or(project_id);
                source_type = "campaign".to_string();
                source_id = campaign.id.clone();
                title
                    .get_or_insert_with(|| format!("Benchmark campaign report: {}", campaign.name));
                let leaderboard =
                    self.get_benchmark_leaderboard(CodingBenchmarkLeaderboardInput {
                        session_id: session_id.clone(),
                        project_id: project_id.clone(),
                        window_days: Some(window_days),
                        campaign_ids: vec![campaign.id.clone()],
                        limit: Some(DEFAULT_BENCHMARK_LEADERBOARD_LIMIT),
                        min_items: Some(DEFAULT_BENCHMARK_LEADERBOARD_MIN_ITEMS),
                    })?;
                let scope = benchmark_scope_label(session_id.as_ref(), project_id.as_ref());
                let status = benchmark_report_status_from_campaign(&campaign);
                let summary = format!(
                    "Campaign {} has {}/{} passed item(s), {} failed item(s), and {} total check(s).",
                    campaign.name,
                    campaign.summary.passed_items,
                    campaign.summary.total_items,
                    campaign.summary.failed_items,
                    campaign.summary.total_checks
                );
                let snapshot = json!({
                    "reportId": report_id,
                    "reportType": report_type,
                    "generatedAt": generated_at,
                    "campaign": campaign,
                    "leaderboard": leaderboard,
                });
                (status, scope, summary, snapshot)
            }
            "comparison" => {
                title.get_or_insert_with(|| "Benchmark comparison report".to_string());
                let leaderboard =
                    self.compare_benchmark_models(CodingBenchmarkComparisonInput {
                        session_id: session_id.clone(),
                        project_id: project_id.clone(),
                        window_days: Some(window_days),
                        campaign_ids: campaign_ids.clone(),
                        limit: Some(MAX_BENCHMARK_LEADERBOARD_LIMIT.min(20)),
                        min_items: Some(DEFAULT_BENCHMARK_LEADERBOARD_MIN_ITEMS),
                    })?;
                let corpus_health =
                    self.get_benchmark_corpus_health(CodingBenchmarkCorpusHealthInput::default())?;
                session_id = leaderboard.session_id.clone().or(session_id);
                project_id = leaderboard.project_id.clone().or(project_id);
                let scope = leaderboard.scope.clone();
                let status = leaderboard.status.clone();
                let summary = format!(
                    "Comparison includes {} model/baseline row(s) across a {} day window.",
                    leaderboard.rows.len(),
                    leaderboard.window_days
                );
                let snapshot = json!({
                    "reportId": report_id,
                    "reportType": report_type,
                    "generatedAt": generated_at,
                    "leaderboard": leaderboard,
                    "corpusHealth": corpus_health,
                });
                (status, scope, summary, snapshot)
            }
            "release" => {
                title.get_or_insert_with(|| "Benchmark release report".to_string());
                let center = self.get_coding_benchmark_center(CodingBenchmarkCenterInput {
                    session_id: session_id.clone(),
                    project_id: project_id.clone(),
                    window_days: Some(window_days),
                    limit: Some(DEFAULT_BENCHMARK_CENTER_LIMIT),
                    ..Default::default()
                })?;
                let release_gate =
                    self.evaluate_coding_eval_release_gate(CodingEvalReleaseGateInput {
                        session_id: session_id.clone(),
                        project_id: project_id.clone(),
                        window_days: Some(window_days),
                        ..Default::default()
                    })?;
                let leaderboard =
                    self.get_benchmark_leaderboard(CodingBenchmarkLeaderboardInput {
                        session_id: session_id.clone(),
                        project_id: project_id.clone(),
                        window_days: Some(window_days),
                        campaign_ids: campaign_ids.clone(),
                        limit: Some(DEFAULT_BENCHMARK_LEADERBOARD_LIMIT),
                        min_items: Some(DEFAULT_BENCHMARK_LEADERBOARD_MIN_ITEMS),
                    })?;
                let corpus_health =
                    self.get_benchmark_corpus_health(CodingBenchmarkCorpusHealthInput::default())?;
                session_id = center.session_id.clone().or(session_id);
                project_id = center.project_id.clone().or(project_id);
                source_type = "release_gate".to_string();
                source_id = release_gate.generated_at.clone();
                let scope = center.scope.clone();
                let status = project_status_rank(&center.status)
                    .min(project_status_rank(&release_gate.status))
                    .min(project_status_rank(&corpus_health.status));
                let status = match status {
                    0 => "failed",
                    1 => "insufficient_data",
                    _ => "passed",
                }
                .to_string();
                let summary = format!(
                    "Release gate is {}; benchmark center is {}; corpus health is {}.",
                    release_gate.status, center.status, corpus_health.status
                );
                let snapshot = json!({
                    "reportId": report_id,
                    "reportType": report_type,
                    "generatedAt": generated_at,
                    "benchmarkCenter": center,
                    "releaseGate": release_gate,
                    "leaderboard": leaderboard,
                    "corpusHealth": corpus_health,
                });
                (status, scope, summary, snapshot)
            }
            _ => unreachable!(),
        };

        let title = title.unwrap_or_else(|| "Benchmark report".to_string());
        let output_root = if let Some(path) = input
            .output_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            PathBuf::from(path)
        } else {
            crate::paths::reports_dir()?.join("benchmark")
        };
        let report_dir = output_root.join(&report_id);
        std::fs::create_dir_all(&report_dir)?;
        let markdown_path = report_dir.join("report.md");
        let json_path = report_dir.join("snapshot.json");
        let html_path = report_dir.join("report.html");
        let markdown = benchmark_report_markdown(&title, &status, &scope, &summary, &snapshot)?;
        let snapshot_json = serde_json::to_string_pretty(&snapshot)?;
        let html = benchmark_report_html(&title, &markdown);
        crate::platform::write_atomic(&markdown_path, markdown.as_bytes())?;
        crate::platform::write_atomic(&json_path, snapshot_json.as_bytes())?;
        crate::platform::write_atomic(&html_path, html.as_bytes())?;

        let release_evidence = input.mark_release_evidence || report_type == "release";
        let marked_release_at = if release_evidence {
            Some(generated_at.clone())
        } else {
            None
        };
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO coding_benchmark_reports (
                id, report_type, title, status, summary, scope, session_id, project_id,
                source_type, source_id, campaign_id, campaign_ids_json, snapshot_json,
                markdown_path, json_path, html_path, release_evidence, created_at,
                updated_at, marked_release_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?18, ?19
             )",
            params![
                report_id,
                report_type,
                title,
                status,
                summary,
                scope,
                session_id,
                project_id,
                source_type,
                source_id,
                campaign_id,
                serde_json::to_string(&campaign_ids)?,
                snapshot_json,
                markdown_path.to_string_lossy().to_string(),
                json_path.to_string_lossy().to_string(),
                html_path.to_string_lossy().to_string(),
                if release_evidence { 1i64 } else { 0i64 },
                generated_at,
                marked_release_at,
            ],
        )?;
        drop(conn);
        self.get_benchmark_report(&report_id)?
            .ok_or_else(|| anyhow!("benchmark report vanished after insert"))
    }

    pub fn list_benchmark_reports(
        &self,
        input: CodingBenchmarkReportListInput,
    ) -> Result<Vec<CodingBenchmarkReport>> {
        let limit = input
            .limit
            .unwrap_or(DEFAULT_BENCHMARK_REPORT_LIMIT)
            .clamp(1, MAX_BENCHMARK_REPORT_LIMIT);
        let session_id = input
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let project_id = input
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut clauses = Vec::new();
        let mut params = Vec::new();
        if let Some(project_id) = project_id.as_ref() {
            clauses.push("project_id = ?".to_string());
            params.push(project_id.clone());
        } else if let Some(session_id) = session_id.as_ref() {
            clauses.push("session_id = ?".to_string());
            params.push(session_id.clone());
        }
        if input.release_evidence_only {
            clauses.push("release_evidence = 1".to_string());
        }
        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        let sql = format!(
            "SELECT id, report_type, title, status, summary, scope, session_id,
                    project_id, source_type, source_id, campaign_id, campaign_ids_json,
                    snapshot_json, markdown_path, json_path, html_path, release_evidence,
                    created_at, updated_at, marked_release_at
             FROM coding_benchmark_reports
             {where_sql}
             ORDER BY created_at DESC
             LIMIT {limit}"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params_from_iter(params.iter()),
            coding_benchmark_report_from_row,
        )?;
        collect_rows(rows)
    }

    pub fn get_benchmark_report(&self, report_id: &str) -> Result<Option<CodingBenchmarkReport>> {
        let report_id = report_id.trim();
        if report_id.is_empty() {
            bail!("benchmark report id must not be empty");
        }
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, report_type, title, status, summary, scope, session_id,
                    project_id, source_type, source_id, campaign_id, campaign_ids_json,
                    snapshot_json, markdown_path, json_path, html_path, release_evidence,
                    created_at, updated_at, marked_release_at
             FROM coding_benchmark_reports
             WHERE id = ?1",
            params![report_id],
            coding_benchmark_report_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn mark_benchmark_report_release_evidence(
        &self,
        input: CodingBenchmarkReportMarkInput,
    ) -> Result<CodingBenchmarkReport> {
        let report_id = input.report_id.trim().to_string();
        if report_id.is_empty() {
            bail!("benchmark report id must not be empty");
        }
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE coding_benchmark_reports
             SET release_evidence = ?2, updated_at = ?3,
                 marked_release_at = CASE WHEN ?2 = 1 THEN COALESCE(marked_release_at, ?3) ELSE NULL END
             WHERE id = ?1",
            params![
                report_id,
                if input.release_evidence { 1i64 } else { 0i64 },
                now
            ],
        )?;
        drop(conn);
        if changed == 0 {
            bail!("benchmark report not found: {report_id}");
        }
        self.get_benchmark_report(&report_id)?
            .ok_or_else(|| anyhow!("benchmark report not found after mark"))
    }

    pub fn materialize_benchmark_backlog(
        &self,
        input: CodingBenchmarkBacklogMaterializeInput,
    ) -> Result<CodingBenchmarkBacklogMaterializeResult> {
        let limit = input
            .limit
            .unwrap_or(DEFAULT_BENCHMARK_BACKLOG_LIMIT)
            .clamp(1, MAX_BENCHMARK_BACKLOG_LIMIT);
        let gate_input = CodingContinuousBenchmarkGateInput {
            session_id: input.session_id,
            project_id: input.project_id,
            window_days: input.window_days,
            ..Default::default()
        };
        let thresholds = continuous_benchmark_gate_thresholds(&gate_input)?;
        let scope = self.resolve_continuous_benchmark_gate_scope(
            &gate_input,
            thresholds.window_days,
            thresholds.max_evidence_age_days,
        )?;
        let candidates = self.collect_continuous_benchmark_failure_candidates(
            &scope,
            &input.campaign_ids,
            limit,
        )?;
        let now = now_rfc3339();
        let mut inserted = 0usize;
        let mut existing = 0usize;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        for candidate in candidates {
            let id = format!("cbbi_{}", uuid::Uuid::new_v4().simple());
            let changed = conn.execute(
                "INSERT OR IGNORE INTO coding_benchmark_backlog_items (
                    id, status, severity, title, failure_category, scope, session_id,
                    project_id, campaign_id, campaign_item_id, pack_run_id, task_pack_id,
                    task_id, provider_id, model_id, label, baseline_kind, execution_mode,
                    evidence_json, created_at, updated_at
                 ) VALUES (
                    ?1, 'open', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                    ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?19
                 )",
                params![
                    id,
                    if candidate.status == "failed" {
                        "high"
                    } else {
                        "medium"
                    },
                    candidate.title,
                    candidate.failure_category,
                    scope.scope,
                    scope.session_id,
                    scope.project_id,
                    candidate.campaign_id,
                    candidate.campaign_item_id,
                    candidate.pack_run_id,
                    candidate.task_pack_id,
                    candidate.task_id,
                    candidate.provider_id,
                    candidate.model_id,
                    candidate.label,
                    candidate.baseline_kind,
                    candidate.execution_mode,
                    stable_json(&candidate.evidence)?,
                    now,
                ],
            )?;
            if changed == 0 {
                existing += 1;
            } else {
                inserted += 1;
            }
        }
        drop(conn);
        let items = self.list_benchmark_backlog(CodingBenchmarkBacklogListInput {
            session_id: scope.session_id,
            project_id: scope.project_id,
            status: Some("open".to_string()),
            limit: Some(limit),
        })?;
        Ok(CodingBenchmarkBacklogMaterializeResult {
            inserted,
            existing,
            items,
        })
    }

    pub fn list_benchmark_backlog(
        &self,
        input: CodingBenchmarkBacklogListInput,
    ) -> Result<Vec<CodingBenchmarkBacklogItem>> {
        let limit = input
            .limit
            .unwrap_or(DEFAULT_BENCHMARK_BACKLOG_LIMIT)
            .clamp(1, MAX_BENCHMARK_BACKLOG_LIMIT);
        let (session_id, project_id) = self.resolve_durable_coding_record_scope(
            input.session_id,
            input.project_id,
            "benchmark backlog",
        )?;
        let status = input
            .status
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(normalize_benchmark_backlog_status)
            .transpose()?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut clauses = Vec::new();
        let mut params = Vec::new();
        if let Some(project_id) = project_id.as_ref() {
            clauses.push("project_id = ?".to_string());
            params.push(project_id.clone());
        } else if let Some(session_id) = session_id.as_ref() {
            clauses.push("session_id = ?".to_string());
            params.push(session_id.clone());
        }
        if let Some(status) = status.as_ref() {
            clauses.push("status = ?".to_string());
            params.push(status.clone());
        }
        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        params.push(limit.to_string());
        let mut stmt = conn.prepare(&format!(
            "SELECT id, status, severity, title, failure_category, scope, session_id,
                    project_id, campaign_id, campaign_item_id, pack_run_id, task_pack_id,
                    task_id, provider_id, model_id, label, baseline_kind, execution_mode,
                    evidence_json, proposal_id, created_at, updated_at, resolved_at
             FROM coding_benchmark_backlog_items
             {where_sql}
             ORDER BY updated_at DESC, id DESC
             LIMIT ?"
        ))?;
        let rows = stmt.query_map(
            params_from_iter(params.iter()),
            coding_benchmark_backlog_item_from_row,
        )?;
        collect_rows(rows)
    }

    pub fn update_benchmark_backlog_status(
        &self,
        input: CodingBenchmarkBacklogStatusInput,
    ) -> Result<CodingBenchmarkBacklogItem> {
        let item_id = input.item_id.trim();
        if item_id.is_empty() {
            bail!("benchmark backlog item id must not be empty");
        }
        let status = normalize_benchmark_backlog_status(&input.status)?;
        let now = now_rfc3339();
        let resolved_at = if matches!(status.as_str(), "resolved" | "wont_fix") {
            Some(now.clone())
        } else {
            None
        };
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE coding_benchmark_backlog_items
             SET status = ?2, proposal_id = COALESCE(?3, proposal_id),
                 updated_at = ?4, resolved_at = ?5
             WHERE id = ?1",
            params![item_id, status, input.proposal_id, now, resolved_at],
        )?;
        drop(conn);
        if changed == 0 {
            bail!("benchmark backlog item not found: {item_id}");
        }
        self.get_benchmark_backlog_item(item_id)?
            .ok_or_else(|| anyhow!("benchmark backlog item not found after update"))
    }

    fn get_benchmark_backlog_item(
        &self,
        item_id: &str,
    ) -> Result<Option<CodingBenchmarkBacklogItem>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, status, severity, title, failure_category, scope, session_id,
                    project_id, campaign_id, campaign_item_id, pack_run_id, task_pack_id,
                    task_id, provider_id, model_id, label, baseline_kind, execution_mode,
                    evidence_json, proposal_id, created_at, updated_at, resolved_at
             FROM coding_benchmark_backlog_items
             WHERE id = ?1",
            params![item_id],
            coding_benchmark_backlog_item_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    fn get_benchmark_task_pack_by_row_id(
        &self,
        row_id: &str,
    ) -> Result<Option<CodingBenchmarkTaskPack>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let pack = conn
            .query_row(
                "SELECT id, pack_id, pack_version, name, description, status, source_kind,
                        source_uri, repo_template, license_note, privacy_note, redaction_status,
                        imported_from, created_at, updated_at, activated_at, archived_at
                 FROM coding_benchmark_task_packs
                 WHERE id = ?1",
                params![row_id],
                coding_benchmark_task_pack_from_row,
            )
            .optional()?;
        let Some(mut pack) = pack else {
            return Ok(None);
        };
        pack.tasks = self.coding_benchmark_task_pack_tasks_locked(&conn, row_id)?;
        Ok(Some(pack))
    }

    fn coding_benchmark_task_pack_tasks_locked(
        &self,
        conn: &Connection,
        pack_row_id: &str,
    ) -> Result<Vec<CodingBenchmarkTaskPackTask>> {
        let mut stmt = conn.prepare(
            "SELECT id, pack_id, pack_version, task_id, task_version, title, status,
                    task_type, difficulty, language, framework, source_uri, repo_template,
                    tags_json, success_criteria_json, validation_commands_json,
                    allowed_paths_json, forbidden_paths_json, calibration_notes_json,
                    calibrated_at, license_note, privacy_note, redaction_status,
                    risk_flags_json, fingerprint, created_at, updated_at
             FROM coding_benchmark_task_pack_tasks
             WHERE pack_row_id = ?1
             ORDER BY task_id ASC, task_version DESC",
        )?;
        let rows = stmt.query_map(
            params![pack_row_id],
            coding_benchmark_task_pack_task_from_row,
        )?;
        collect_rows(rows)
    }

    fn coding_benchmark_campaign_items_locked(
        &self,
        conn: &Connection,
        campaign_id: &str,
    ) -> Result<Vec<CodingBenchmarkCampaignItem>> {
        let mut stmt = conn.prepare(
            "SELECT id, campaign_id, provider_id, model_id, label, status, attempt,
                    pack_run_id, selected_cases, passed_cases, failed_cases, skipped_cases,
                    total_checks, started_at, finished_at, error
             FROM coding_benchmark_campaign_items
             WHERE campaign_id = ?1
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(
            params![campaign_id],
            coding_benchmark_campaign_item_from_row,
        )?;
        collect_rows(rows)
    }

    fn resolve_durable_coding_record_scope(
        &self,
        session_id: Option<String>,
        project_id: Option<String>,
        kind: &str,
    ) -> Result<(Option<String>, Option<String>)> {
        let session_id = session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let session_project_id = if let Some(session_id) = session_id.as_deref() {
            let meta = self
                .get_session(session_id)?
                .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
            if meta.incognito {
                bail!("Cannot record coding {kind} run for incognito session {session_id}");
            }
            meta.project_id
        } else {
            None
        };
        let project_id = project_id
            .or(session_project_id)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        Ok((session_id, project_id))
    }

    fn resolve_coding_eval_release_gate_scope(
        &self,
        input: &CodingEvalReleaseGateInput,
    ) -> Result<ReleaseGateScope> {
        let window_days = input
            .window_days
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS);
        let since = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(window_days as i64))
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let session_id = input
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let explicit_project_id = input
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let session_project_id = if let Some(session_id) = session_id.as_deref() {
            let meta = self
                .get_session(session_id)?
                .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
            if meta.incognito {
                bail!("Cannot evaluate coding release gate for incognito session {session_id}");
            }
            meta.project_id
        } else {
            None
        };
        let project_id = explicit_project_id.or(session_project_id);
        let scope = if project_id.is_some() {
            "project"
        } else if session_id.is_some() {
            "session"
        } else {
            "global"
        }
        .to_string();
        Ok(ReleaseGateScope {
            session_id,
            project_id,
            scope,
            window_days,
            since,
        })
    }

    pub fn resolve_continuous_benchmark_gate_scope(
        &self,
        input: &CodingContinuousBenchmarkGateInput,
        window_days: u32,
        max_evidence_age_days: u32,
    ) -> Result<ContinuousBenchmarkGateScope> {
        let since = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(window_days as i64))
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let stale_before = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(max_evidence_age_days as i64))
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let session_id = input
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let explicit_project_id = input
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let session_project_id = if let Some(session_id) = session_id.as_deref() {
            let meta = self
                .get_session(session_id)?
                .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
            if meta.incognito {
                bail!(
                    "Cannot evaluate continuous benchmark gate for incognito session {session_id}"
                );
            }
            meta.project_id
        } else {
            None
        };
        let project_id = explicit_project_id.or(session_project_id);
        let scope = if project_id.is_some() {
            "project"
        } else if session_id.is_some() {
            "session"
        } else {
            "global"
        }
        .to_string();
        Ok(ContinuousBenchmarkGateScope {
            session_id,
            project_id,
            scope,
            since,
            stale_before,
        })
    }

    pub fn latest_release_evidence_report(
        &self,
        scope: &ContinuousBenchmarkGateScope,
    ) -> Result<Option<(String, String, String)>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut clauses = vec!["release_evidence = 1".to_string()];
        let mut params = Vec::new();
        if let Some(project_id) = scope.project_id.as_ref() {
            clauses.push("project_id = ?".to_string());
            params.push(project_id.clone());
        } else if let Some(session_id) = scope.session_id.as_ref() {
            clauses.push("session_id = ?".to_string());
            params.push(session_id.clone());
        }
        let sql = format!(
            "SELECT id, status, created_at
             FROM coding_benchmark_reports
             WHERE {}
             ORDER BY created_at DESC
             LIMIT 1",
            clauses.join(" AND ")
        );
        conn.query_row(&sql, params_from_iter(params.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .optional()
        .map_err(Into::into)
    }

    pub fn count_open_benchmark_backlog_items(
        &self,
        scope: &ContinuousBenchmarkGateScope,
    ) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut clauses = vec!["status IN ('open','in_progress')".to_string()];
        let mut params = Vec::new();
        if let Some(project_id) = scope.project_id.as_ref() {
            clauses.push("project_id = ?".to_string());
            params.push(project_id.clone());
        } else if let Some(session_id) = scope.session_id.as_ref() {
            clauses.push("session_id = ?".to_string());
            params.push(session_id.clone());
        }
        conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM coding_benchmark_backlog_items WHERE {}",
                clauses.join(" AND ")
            ),
            params_from_iter(params.iter()),
            |row| Ok(nonnegative_usize(row.get::<_, i64>(0)?)),
        )
        .map_err(Into::into)
    }

    pub fn count_unmaterialized_backlog_candidates(
        &self,
        candidates: &[ContinuousBenchmarkFailureCandidate],
    ) -> Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut pending = 0usize;
        for candidate in candidates {
            let exists = conn
                .query_row(
                    "SELECT 1 FROM coding_benchmark_backlog_items
                 WHERE campaign_item_id = ?1 AND task_id = ?2
                 LIMIT 1",
                    params![candidate.campaign_item_id, candidate.task_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !exists {
                pending += 1;
            }
        }
        Ok(pending)
    }

    pub fn collect_continuous_benchmark_failure_candidates(
        &self,
        scope: &ContinuousBenchmarkGateScope,
        campaign_ids: &[String],
        limit: usize,
    ) -> Result<Vec<ContinuousBenchmarkFailureCandidate>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut clauses = vec![
            "c.updated_at >= ?".to_string(),
            "i.status IN ('failed','interrupted','cancelled')".to_string(),
        ];
        let mut params = vec![scope.since.clone()];
        if let Some(project_id) = scope.project_id.as_ref() {
            clauses.push("c.project_id = ?".to_string());
            params.push(project_id.clone());
        } else if let Some(session_id) = scope.session_id.as_ref() {
            clauses.push("c.session_id = ?".to_string());
            params.push(session_id.clone());
        }
        let campaign_ids = campaign_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .take(MAX_BENCHMARK_CAMPAIGN_LIMIT)
            .collect::<Vec<_>>();
        if !campaign_ids.is_empty() {
            let placeholders = std::iter::repeat_n("?", campaign_ids.len())
                .collect::<Vec<_>>()
                .join(", ");
            clauses.push(format!("c.id IN ({placeholders})"));
            params.extend(campaign_ids);
        }
        params.push(limit.to_string());
        let sql = format!(
            "SELECT c.id, c.name, c.task_pack_id, c.execution_mode, c.baseline_kind,
                    i.id, i.provider_id, i.model_id, i.label, i.status, i.pack_run_id,
                    i.report_json, i.error, i.updated_at
             FROM coding_benchmark_campaign_items i
             JOIN coding_benchmark_campaigns c ON c.id = i.campaign_id
             WHERE {}
             ORDER BY i.updated_at DESC, i.id DESC
             LIMIT ?",
            clauses.join(" AND ")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, String>(13)?,
            ))
        })?;
        let mut candidates = Vec::new();
        for row in rows {
            let (
                campaign_id,
                campaign_name,
                task_pack_id,
                execution_mode,
                baseline_kind,
                item_id,
                provider_id,
                model_id,
                label,
                status,
                pack_run_id,
                report_json,
                error,
                updated_at,
            ) = row?;
            let failures = benchmark_backlog_failures_from_report(&report_json);
            if failures.is_empty() {
                let category = classify_benchmark_item_failure(&status, error.as_deref())
                    .unwrap_or_else(|| "benchmark_failed".to_string());
                candidates.push(ContinuousBenchmarkFailureCandidate {
                    campaign_id: campaign_id.clone(),
                    campaign_item_id: item_id.clone(),
                    pack_run_id: pack_run_id.clone(),
                    task_pack_id: task_pack_id.clone(),
                    task_id: String::new(),
                    provider_id: provider_id.clone(),
                    model_id: model_id.clone(),
                    label: label.clone(),
                    baseline_kind: baseline_kind.clone(),
                    execution_mode: execution_mode.clone(),
                    status: status.clone(),
                    failure_category: category.clone(),
                    title: format!("{} benchmark item {}", campaign_name, status),
                    evidence: json!({
                        "campaignId": &campaign_id,
                        "campaignName": &campaign_name,
                        "itemId": &item_id,
                        "status": &status,
                        "packRunId": &pack_run_id,
                        "providerId": &provider_id,
                        "modelId": &model_id,
                        "label": &label,
                        "error": &error,
                        "updatedAt": &updated_at,
                        "failureCategory": &category,
                    }),
                });
            } else {
                for failure in failures {
                    candidates.push(ContinuousBenchmarkFailureCandidate {
                        campaign_id: campaign_id.clone(),
                        campaign_item_id: item_id.clone(),
                        pack_run_id: pack_run_id.clone(),
                        task_pack_id: task_pack_id.clone(),
                        task_id: failure.0.clone(),
                        provider_id: provider_id.clone(),
                        model_id: model_id.clone(),
                        label: label.clone(),
                        baseline_kind: baseline_kind.clone(),
                        execution_mode: execution_mode.clone(),
                        status: status.clone(),
                        failure_category: failure.2.clone(),
                        title: failure.1.clone(),
                        evidence: json!({
                            "campaignId": &campaign_id,
                            "campaignName": &campaign_name,
                            "itemId": &item_id,
                            "status": &status,
                            "packRunId": &pack_run_id,
                            "taskPackId": &task_pack_id,
                            "taskId": &failure.0,
                            "providerId": &provider_id,
                            "modelId": &model_id,
                            "label": &label,
                            "error": &error,
                            "updatedAt": &updated_at,
                            "failureCategory": &failure.2,
                            "case": &failure.3,
                        }),
                    });
                }
            }
        }
        Ok(candidates)
    }

    fn resolve_coding_learning_generalization_scope(
        &self,
        input: &CodingLearningGeneralizationInput,
    ) -> Result<LearningGeneralizationScope> {
        let window_days = input
            .window_days
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS);
        let since = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(window_days as i64))
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let session_id = input
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let explicit_project_id = input
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let session_project_id = if let Some(session_id) = session_id.as_deref() {
            let meta = self
                .get_session(session_id)?
                .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
            if meta.incognito {
                bail!(
                    "Cannot evaluate coding learning generalization for incognito session {session_id}"
                );
            }
            meta.project_id
        } else {
            None
        };
        let project_id = explicit_project_id.or(session_project_id);
        let scope = if project_id.is_some() {
            "project"
        } else if session_id.is_some() {
            "session"
        } else {
            "global"
        }
        .to_string();
        let source_type = input
            .source_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let source_id = input
            .source_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let proposal_kinds = normalize_generalization_proposal_kinds(&input.proposal_kinds);
        Ok(LearningGeneralizationScope {
            session_id,
            project_id,
            scope,
            window_days,
            since,
            source_type,
            source_id,
            proposal_kinds,
        })
    }

    fn resolve_coding_benchmark_center_scope(
        &self,
        input: &CodingBenchmarkCenterInput,
    ) -> Result<BenchmarkCenterScope> {
        let window_days = input
            .window_days
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS);
        let since = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(window_days as i64))
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let session_id = input
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let explicit_project_id = input
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let session_project_id = if let Some(session_id) = session_id.as_deref() {
            let meta = self
                .get_session(session_id)?
                .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
            if meta.incognito {
                bail!("Cannot build coding benchmark center for incognito session {session_id}");
            }
            meta.project_id
        } else {
            None
        };
        let project_id = explicit_project_id.or(session_project_id);
        let scope = if project_id.is_some() {
            "project"
        } else if session_id.is_some() {
            "session"
        } else {
            "global"
        }
        .to_string();
        let limit = input
            .limit
            .unwrap_or(DEFAULT_BENCHMARK_CENTER_LIMIT)
            .clamp(1, MAX_BENCHMARK_CENTER_LIMIT);

        Ok(BenchmarkCenterScope {
            session_id,
            project_id,
            scope,
            window_days,
            since,
            limit,
        })
    }

    fn resolve_benchmark_leaderboard_scope(
        &self,
        session_id: Option<String>,
        project_id: Option<String>,
        window_days: Option<u32>,
        campaign_ids: Vec<String>,
        limit: Option<usize>,
        min_items: Option<usize>,
    ) -> Result<BenchmarkLeaderboardScope> {
        let window_days = window_days
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS);
        let since = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(window_days as i64))
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let session_id = session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let explicit_project_id = project_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let session_project_id = if let Some(session_id) = session_id.as_deref() {
            let meta = self
                .get_session(session_id)?
                .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
            if meta.incognito {
                bail!("Cannot build benchmark leaderboard for incognito session {session_id}");
            }
            meta.project_id
        } else {
            None
        };
        let project_id = explicit_project_id.or(session_project_id);
        let scope = if project_id.is_some() {
            "project"
        } else if session_id.is_some() {
            "session"
        } else {
            "global"
        }
        .to_string();
        let campaign_ids = campaign_ids
            .into_iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .take(MAX_BENCHMARK_CAMPAIGN_LIMIT)
            .collect::<Vec<_>>();
        let limit = limit
            .unwrap_or(DEFAULT_BENCHMARK_LEADERBOARD_LIMIT)
            .clamp(1, MAX_BENCHMARK_LEADERBOARD_LIMIT);
        let min_items = min_items
            .unwrap_or(DEFAULT_BENCHMARK_LEADERBOARD_MIN_ITEMS)
            .clamp(1, MAX_BENCHMARK_CAMPAIGN_LIMIT);

        Ok(BenchmarkLeaderboardScope {
            session_id,
            project_id,
            scope,
            window_days,
            since,
            limit,
            min_items,
            campaign_ids,
        })
    }

    fn build_benchmark_leaderboard(
        &self,
        scope: BenchmarkLeaderboardScope,
    ) -> Result<CodingBenchmarkLeaderboardReport> {
        let item_rows = self.list_benchmark_leaderboard_item_rows(&scope)?;
        let mut grouped: BTreeMap<BenchmarkLeaderboardKey, BenchmarkLeaderboardAccumulator> =
            BTreeMap::new();
        for row in item_rows {
            let key = BenchmarkLeaderboardKey::from(&row);
            grouped.entry(key).or_default().add(row);
        }
        let mut rows = grouped
            .into_iter()
            .map(|(key, acc)| acc.into_row(key, scope.min_items))
            .collect::<Vec<_>>();
        rows.sort_by(compare_benchmark_leaderboard_rows);
        rows.truncate(scope.limit);
        for (idx, row) in rows.iter_mut().enumerate() {
            row.rank = idx + 1;
        }

        let mut checks = Vec::new();
        push_benchmark_check(
            &mut checks,
            "model_count",
            if rows.len() >= 2 {
                "passed"
            } else {
                "insufficient_data"
            },
            if rows.len() >= 2 { "info" } else { "advisory" },
            "at least 2 comparable model rows",
            rows.len().to_string(),
            "Cross-model comparison needs at least two model/baseline rows in the selected window.",
        );
        let under_sampled = rows
            .iter()
            .filter(|row| row.items < scope.min_items)
            .count();
        push_benchmark_check(
            &mut checks,
            "sample_size",
            if under_sampled == 0 {
                "passed"
            } else {
                "insufficient_data"
            },
            if under_sampled == 0 { "info" } else { "advisory" },
            format!("each row has >= {} items", scope.min_items),
            format!("{under_sampled} under-sampled rows"),
            "Rows with too few campaign items remain visible but are marked with a sample-size warning.",
        );
        let status = if rows.len() >= 2 {
            "passed"
        } else {
            "insufficient_data"
        }
        .to_string();

        Ok(CodingBenchmarkLeaderboardReport {
            generated_at: now_rfc3339(),
            status,
            scope: scope.scope,
            session_id: scope.session_id,
            project_id: scope.project_id,
            window_days: scope.window_days,
            since: scope.since,
            min_items: scope.min_items,
            rows,
            checks,
        })
    }

    fn list_benchmark_leaderboard_item_rows(
        &self,
        scope: &BenchmarkLeaderboardScope,
    ) -> Result<Vec<BenchmarkLeaderboardItemRow>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut clauses = vec!["c.updated_at >= ?".to_string()];
        let mut params = vec![scope.since.clone()];
        if let Some(project_id) = scope.project_id.as_ref() {
            clauses.push("c.project_id = ?".to_string());
            params.push(project_id.clone());
        } else if let Some(session_id) = scope.session_id.as_ref() {
            clauses.push("c.session_id = ?".to_string());
            params.push(session_id.clone());
        }
        if !scope.campaign_ids.is_empty() {
            let placeholders = std::iter::repeat_n("?", scope.campaign_ids.len())
                .collect::<Vec<_>>()
                .join(", ");
            clauses.push(format!("c.id IN ({placeholders})"));
            params.extend(scope.campaign_ids.iter().cloned());
        }
        let where_sql = clauses.join(" AND ");
        let sql = format!(
            "SELECT c.id, c.name, c.task_pack_id, c.source_doc, c.execution_mode,
                    c.baseline_kind, i.id, i.provider_id, i.model_id, i.label,
                    i.status, i.attempt, i.pack_run_id, i.selected_cases,
                    i.passed_cases, i.failed_cases, i.skipped_cases, i.total_checks,
                    i.updated_at, i.error
             FROM coding_benchmark_campaign_items i
             JOIN coding_benchmark_campaigns c ON c.id = i.campaign_id
             WHERE {where_sql}
             ORDER BY c.updated_at DESC, i.updated_at DESC, i.id DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
            Ok(BenchmarkLeaderboardItemRow {
                campaign_id: row.get(0)?,
                campaign_name: row.get(1)?,
                task_pack_id: row.get(2)?,
                source_doc: row.get(3)?,
                execution_mode: row.get(4)?,
                baseline_kind: row.get(5)?,
                item_id: row.get(6)?,
                provider_id: row.get(7)?,
                model_id: row.get(8)?,
                label: row.get(9)?,
                status: row.get(10)?,
                attempt: nonnegative_usize(row.get::<_, i64>(11)?),
                pack_run_id: row.get(12)?,
                selected_cases: nonnegative_usize(row.get::<_, i64>(13)?),
                passed_cases: nonnegative_usize(row.get::<_, i64>(14)?),
                failed_cases: nonnegative_usize(row.get::<_, i64>(15)?),
                skipped_cases: nonnegative_usize(row.get::<_, i64>(16)?),
                total_checks: nonnegative_usize(row.get::<_, i64>(17)?),
                updated_at: row.get(18)?,
                error: row.get(19)?,
            })
        })?;
        collect_rows(rows)
    }

    fn coding_benchmark_center_summary(
        &self,
        scope: &BenchmarkCenterScope,
    ) -> Result<CodingBenchmarkCenterSummary> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let (where_sql, params) = benchmark_center_filter(scope, "cepr", "cepr.created_at");
        let mut summary = conn.query_row(
            &format!(
                "SELECT COUNT(*),
                        COALESCE(SUM(CASE WHEN cepr.status = 'passed' THEN 1 ELSE 0 END), 0),
                        COALESCE(SUM(CASE WHEN cepr.status = 'failed' THEN 1 ELSE 0 END), 0),
                        COALESCE(SUM(CASE WHEN cepr.status = 'skipped' THEN 1 ELSE 0 END), 0),
                        COALESCE(SUM(CASE WHEN cepr.baseline_kind = 'external_model' THEN 1 ELSE 0 END), 0),
                        COALESCE(SUM(CASE WHEN cepr.baseline_kind <> 'external_model' THEN 1 ELSE 0 END), 0),
                        COALESCE(SUM(cepr.selected_cases), 0),
                        COALESCE(SUM(cepr.automated_cases), 0),
                        COALESCE(SUM(cepr.passed_cases), 0),
                        COALESCE(SUM(cepr.failed_cases), 0),
                        COALESCE(SUM(cepr.skipped_cases), 0),
                        COALESCE(SUM(cepr.total_checks), 0),
                        MAX(CASE
                            WHEN (cepr.passed_cases + cepr.failed_cases) > 0
                            THEN CAST(cepr.passed_cases AS REAL) / CAST(cepr.passed_cases + cepr.failed_cases AS REAL)
                            ELSE NULL
                        END)
                 FROM coding_eval_pack_runs cepr
                 LEFT JOIN sessions s ON s.id = cepr.session_id
                 {}",
                where_sql
            ),
            params_from_iter(params.iter()),
            |row| {
                Ok(CodingBenchmarkCenterSummary {
                    total_runs: nonnegative_usize(row.get::<_, i64>(0)?),
                    passed_runs: nonnegative_usize(row.get::<_, i64>(1)?),
                    failed_runs: nonnegative_usize(row.get::<_, i64>(2)?),
                    skipped_runs: nonnegative_usize(row.get::<_, i64>(3)?),
                    external_model_runs: nonnegative_usize(row.get::<_, i64>(4)?),
                    deterministic_runs: nonnegative_usize(row.get::<_, i64>(5)?),
                    selected_cases: nonnegative_usize(row.get::<_, i64>(6)?),
                    automated_cases: nonnegative_usize(row.get::<_, i64>(7)?),
                    passed_cases: nonnegative_usize(row.get::<_, i64>(8)?),
                    failed_cases: nonnegative_usize(row.get::<_, i64>(9)?),
                    skipped_cases: nonnegative_usize(row.get::<_, i64>(10)?),
                    total_checks: nonnegative_usize(row.get::<_, i64>(11)?),
                    best_case_pass_rate: row
                        .get::<_, Option<f64>>(12)?
                        .map(|value| (value * 1000.0).round() / 1000.0),
                    ..CodingBenchmarkCenterSummary::default()
                })
            },
        )?;
        summary.run_pass_rate = ratio(
            summary.passed_runs,
            summary.passed_runs + summary.failed_runs,
        );
        summary.case_pass_rate = ratio(
            summary.passed_cases,
            summary.passed_cases + summary.failed_cases,
        );

        let latest = conn
            .query_row(
                &format!(
                    "SELECT cepr.id, cepr.status, cepr.created_at
                     FROM coding_eval_pack_runs cepr
                     LEFT JOIN sessions s ON s.id = cepr.session_id
                     {}
                     ORDER BY cepr.created_at DESC, cepr.id DESC
                     LIMIT 1",
                    where_sql
                ),
                params_from_iter(params.iter()),
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        if let Some((id, status, created_at)) = latest {
            summary.latest_run_id = Some(id);
            summary.latest_run_status = Some(status);
            summary.latest_run_at = Some(created_at);
        }

        Ok(summary)
    }

    fn coding_benchmark_center_baselines(
        &self,
        scope: &BenchmarkCenterScope,
    ) -> Result<Vec<CodingBenchmarkBaselineBucket>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let (where_sql, params) = benchmark_center_filter(scope, "cepr", "cepr.created_at");
        let mut stmt = conn.prepare(&format!(
            "SELECT cepr.baseline_kind,
                    COUNT(*),
                    COALESCE(SUM(CASE WHEN cepr.status = 'passed' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN cepr.status = 'failed' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN cepr.status = 'skipped' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(cepr.passed_cases), 0),
                    COALESCE(SUM(cepr.failed_cases), 0),
                    MAX(cepr.created_at)
             FROM coding_eval_pack_runs cepr
             LEFT JOIN sessions s ON s.id = cepr.session_id
             {}
             GROUP BY cepr.baseline_kind",
            where_sql
        ))?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
            let runs = nonnegative_usize(row.get::<_, i64>(1)?);
            let passed_runs = nonnegative_usize(row.get::<_, i64>(2)?);
            let failed_runs = nonnegative_usize(row.get::<_, i64>(3)?);
            let passed_cases = nonnegative_usize(row.get::<_, i64>(5)?);
            let failed_cases = nonnegative_usize(row.get::<_, i64>(6)?);
            Ok(CodingBenchmarkBaselineBucket {
                baseline_kind: row.get(0)?,
                runs,
                passed_runs,
                failed_runs,
                skipped_runs: nonnegative_usize(row.get::<_, i64>(4)?),
                passed_cases,
                failed_cases,
                run_pass_rate: ratio(passed_runs, passed_runs + failed_runs),
                case_pass_rate: ratio(passed_cases, passed_cases + failed_cases),
                latest_run_at: row.get(7)?,
            })
        })?;
        collect_rows(rows)
    }

    fn coding_benchmark_center_runs(
        &self,
        scope: &BenchmarkCenterScope,
    ) -> Result<Vec<CodingBenchmarkRunItem>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let (where_sql, mut params) = benchmark_center_filter(scope, "cepr", "cepr.created_at");
        params.push(scope.limit.to_string());
        let mut stmt = conn.prepare(&format!(
            "SELECT cepr.id, cepr.session_id, COALESCE(cepr.project_id, s.project_id),
                    cepr.pack_id, cepr.source_doc, cepr.label, cepr.baseline_kind,
                    cepr.status, cepr.selected_cases, cepr.automated_cases,
                    cepr.skipped_cases, cepr.passed_cases, cepr.failed_cases,
                    cepr.total_checks, cepr.report_json, cepr.source_type,
                    cepr.source_id, cepr.created_at
             FROM coding_eval_pack_runs cepr
             LEFT JOIN sessions s ON s.id = cepr.session_id
             {}
             ORDER BY cepr.created_at DESC, cepr.id DESC
             LIMIT ?",
            where_sql
        ))?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
            let passed_cases = nonnegative_usize(row.get::<_, i64>(11)?);
            let failed_cases = nonnegative_usize(row.get::<_, i64>(12)?);
            let report_json: String = row.get(14)?;
            Ok(CodingBenchmarkRunItem {
                id: row.get(0)?,
                session_id: row.get(1)?,
                project_id: row.get(2)?,
                pack_id: row.get(3)?,
                source_doc: row.get(4)?,
                label: row.get(5)?,
                baseline_kind: row.get(6)?,
                status: row.get(7)?,
                selected_cases: nonnegative_usize(row.get::<_, i64>(8)?),
                automated_cases: nonnegative_usize(row.get::<_, i64>(9)?),
                skipped_cases: nonnegative_usize(row.get::<_, i64>(10)?),
                passed_cases,
                failed_cases,
                total_checks: nonnegative_usize(row.get::<_, i64>(13)?),
                case_pass_rate: ratio(passed_cases, passed_cases + failed_cases),
                source_type: row.get(15)?,
                source_id: row.get(16)?,
                created_at: row.get(17)?,
                failed_cases_summary: benchmark_failed_cases_summary(&report_json),
            })
        })?;
        collect_rows(rows)
    }

    fn coding_eval_release_gate_summary(
        &self,
        scope: &ReleaseGateScope,
    ) -> Result<CodingEvalReleaseGateSummary> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut summary = CodingEvalReleaseGateSummary::default();

        let (pack_where, pack_params) = release_gate_filter(scope, "cepr", "cepr.created_at");
        let mut stmt = conn.prepare(&format!(
            "SELECT cepr.status, cepr.baseline_kind, COUNT(*),
                    COALESCE(SUM(cepr.passed_cases), 0),
                    COALESCE(SUM(cepr.failed_cases), 0),
                    COALESCE(SUM(cepr.skipped_cases), 0),
                    COALESCE(SUM(cepr.total_checks), 0)
             FROM coding_eval_pack_runs cepr
             LEFT JOIN sessions s ON s.id = cepr.session_id
             {}
             GROUP BY cepr.status, cepr.baseline_kind",
            pack_where
        ))?;
        let pack_rows = stmt.query_map(params_from_iter(pack_params.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                nonnegative_usize(row.get::<_, i64>(2)?),
                nonnegative_usize(row.get::<_, i64>(3)?),
                nonnegative_usize(row.get::<_, i64>(4)?),
                nonnegative_usize(row.get::<_, i64>(5)?),
                nonnegative_usize(row.get::<_, i64>(6)?),
            ))
        })?;
        for row in pack_rows {
            let (status, baseline_kind, count, passed_cases, failed_cases, skipped_cases, checks) =
                row?;
            summary.pack_runs += count;
            summary.passed_cases += passed_cases;
            summary.failed_cases += failed_cases;
            summary.skipped_cases += skipped_cases;
            summary.total_checks += checks;
            match status.as_str() {
                "passed" => summary.passed_pack_runs += count,
                "failed" => summary.failed_pack_runs += count,
                "skipped" => summary.skipped_pack_runs += count,
                _ => {}
            }
            match baseline_kind.as_str() {
                "external_model" => summary.external_model_pack_runs += count,
                "mock_provider" => summary.mock_provider_pack_runs += count,
                _ => summary.deterministic_pack_runs += count,
            }
        }
        summary.pack_pass_rate = ratio(
            summary.passed_pack_runs,
            summary.passed_pack_runs + summary.failed_pack_runs,
        );

        let (strategy_where, strategy_params) =
            release_gate_filter(scope, "cser", "cser.created_at");
        let mut stmt = conn.prepare(&format!(
            "SELECT cser.verdict, COUNT(*),
                    COALESCE(SUM(cser.validation_violation_delta), 0),
                    COALESCE(SUM(cser.scope_creep_delta), 0),
                    COALESCE(SUM(cser.execution_failure_delta), 0)
             FROM coding_strategy_effect_runs cser
             LEFT JOIN sessions s ON s.id = cser.session_id
             {}
             GROUP BY cser.verdict",
            strategy_where
        ))?;
        let strategy_rows = stmt.query_map(params_from_iter(strategy_params.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                nonnegative_usize(row.get::<_, i64>(1)?),
                row.get::<_, i64>(2)? as isize,
                row.get::<_, i64>(3)? as isize,
                row.get::<_, i64>(4)? as isize,
            ))
        })?;
        for row in strategy_rows {
            let (verdict, count, validation_delta, scope_delta, execution_delta) = row?;
            summary.strategy_effect_runs += count;
            summary.validation_violation_delta += validation_delta;
            summary.scope_creep_delta += scope_delta;
            summary.execution_failure_delta += execution_delta;
            match verdict.as_str() {
                "improved" => summary.improved_strategy_effects += count,
                "regressed" => summary.regressed_strategy_effects += count,
                "mixed" => summary.mixed_strategy_effects += count,
                _ => summary.inconclusive_strategy_effects += count,
            }
        }

        let (eval_where, eval_params) = release_gate_filter(scope, "cer", "cer.created_at");
        summary.missing_tool_call_runs = conn.query_row(
            &format!(
                "SELECT COUNT(*)
                 FROM coding_eval_runs cer
                 LEFT JOIN sessions s ON s.id = cer.session_id
                 {}
                   AND cer.source_type = 'coding_task_eval'
                   AND COALESCE(
                        CAST(json_extract(cer.metrics_json, '$.metrics.executionMode') AS TEXT),
                        CAST(json_extract(cer.metrics_json, '$.metrics.execution_mode') AS TEXT),
                        CAST(json_extract(cer.metrics_json, '$.executionMode') AS TEXT),
                        CAST(json_extract(cer.metrics_json, '$.execution_mode') AS TEXT),
                        ''
                   ) = 'agent'
                   AND COALESCE(
                        json_array_length(json_extract(cer.metrics_json, '$.metrics.agentExecution.toolCalls')),
                        json_array_length(json_extract(cer.metrics_json, '$.metrics.agent_execution.tool_calls')),
                        json_array_length(json_extract(cer.metrics_json, '$.metrics.execution_tool_calls')),
                        json_array_length(json_extract(cer.metrics_json, '$.execution_tool_calls')),
                        0
                   ) = 0",
                eval_where
            ),
            params_from_iter(eval_params.iter()),
            |row| Ok(nonnegative_usize(row.get::<_, i64>(0)?)),
        )?;

        Ok(summary)
    }

    fn coding_learning_generalization_projects(
        &self,
        scope: &LearningGeneralizationScope,
        thresholds: &CodingLearningGeneralizationThresholds,
    ) -> Result<Vec<CodingLearningGeneralizationProject>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut projects: BTreeMap<String, LearningProjectAccumulator> = BTreeMap::new();

        let (proposal_where, proposal_params) = learning_generalization_filter(
            scope,
            "cip",
            "COALESCE(cip.promoted_at, cip.updated_at)",
            true,
            true,
        );
        let mut stmt = conn.prepare(&format!(
            "SELECT COALESCE(cip.project_id, s.project_id), cip.id, cip.kind, cip.title,
                    cip.source_type, cip.source_id, COALESCE(cip.promoted_at, cip.updated_at)
             FROM coding_improvement_proposals cip
             LEFT JOIN sessions s ON s.id = cip.session_id
             {}
             ORDER BY COALESCE(cip.promoted_at, cip.updated_at) DESC",
            proposal_where
        ))?;
        let proposal_rows = stmt.query_map(params_from_iter(proposal_params.iter()), |row| {
            Ok(CodingLearningGeneralizationItem {
                project_id: row.get(0)?,
                proposal_id: row.get(1)?,
                kind: row.get(2)?,
                title: row.get(3)?,
                source_type: row.get(4)?,
                source_id: row.get(5)?,
                promoted_at: row.get(6)?,
            })
        })?;
        for item in collect_rows(proposal_rows)? {
            let project = projects.entry(item.project_id.clone()).or_default();
            project.learning_items.push(item);
        }

        let (pack_where, pack_params) =
            learning_generalization_filter(scope, "cepr", "cepr.created_at", false, false);
        let mut stmt = conn.prepare(&format!(
            "SELECT COALESCE(cepr.project_id, s.project_id), cepr.status, cepr.baseline_kind, COUNT(*)
             FROM coding_eval_pack_runs cepr
             LEFT JOIN sessions s ON s.id = cepr.session_id
             {}
             GROUP BY COALESCE(cepr.project_id, s.project_id), cepr.status, cepr.baseline_kind",
            pack_where
        ))?;
        let pack_rows = stmt.query_map(params_from_iter(pack_params.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                nonnegative_usize(row.get::<_, i64>(3)?),
            ))
        })?;
        for row in pack_rows {
            let (project_id, status, baseline_kind, count) = row?;
            let project = projects.entry(project_id).or_default();
            project.pack_runs += count;
            match status.as_str() {
                "passed" => project.passed_pack_runs += count,
                "failed" => project.failed_pack_runs += count,
                _ => {}
            }
            if baseline_kind == "external_model" {
                project.external_model_pack_runs += count;
            }
        }

        let (strategy_where, strategy_params) =
            learning_generalization_filter(scope, "cser", "cser.created_at", false, true);
        let mut stmt = conn.prepare(&format!(
            "SELECT COALESCE(cser.project_id, s.project_id), cser.verdict, COUNT(*),
                    COALESCE(SUM(cser.validation_violation_delta), 0),
                    COALESCE(SUM(cser.scope_creep_delta), 0),
                    COALESCE(SUM(cser.execution_failure_delta), 0)
             FROM coding_strategy_effect_runs cser
             LEFT JOIN sessions s ON s.id = cser.session_id
             {}
             GROUP BY COALESCE(cser.project_id, s.project_id), cser.verdict",
            strategy_where
        ))?;
        let strategy_rows = stmt.query_map(params_from_iter(strategy_params.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                nonnegative_usize(row.get::<_, i64>(2)?),
                row.get::<_, i64>(3)? as isize,
                row.get::<_, i64>(4)? as isize,
                row.get::<_, i64>(5)? as isize,
            ))
        })?;
        for row in strategy_rows {
            let (project_id, verdict, count, validation_delta, scope_delta, execution_delta) = row?;
            let project = projects.entry(project_id).or_default();
            project.strategy_effect_runs += count;
            project.validation_violation_delta += validation_delta;
            project.scope_creep_delta += scope_delta;
            project.execution_failure_delta += execution_delta;
            match verdict.as_str() {
                "improved" => project.improved_strategy_effects += count,
                "regressed" => project.regressed_strategy_effects += count,
                "mixed" => project.mixed_strategy_effects += count,
                _ => {}
            }
        }

        let mut out = Vec::new();
        for (project_id, project) in projects {
            out.push(project.into_report(project_id, thresholds));
        }
        Ok(out)
    }

    pub fn resolve_coding_report_scope(
        &self,
        session_id: &str,
        window_days: Option<u32>,
    ) -> Result<ReportScope> {
        let window_days = window_days
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS);
        let since = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(window_days as i64))
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let meta = self
            .get_session(session_id)?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        if meta.incognito {
            bail!("Cannot build durable coding trend report for incognito session {session_id}");
        }
        let session_ids = if let Some(project_id) = meta.project_id.as_deref() {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let mut stmt = conn.prepare(
                "SELECT id FROM sessions
                 WHERE project_id = ?1
                   AND incognito = 0
                   AND (updated_at >= ?2 OR id = ?3)
                 ORDER BY updated_at DESC
                 LIMIT ?4",
            )?;
            let rows = stmt.query_map(
                params![project_id, since, session_id, MAX_SCOPE_SESSIONS as i64],
                |row| row.get::<_, String>(0),
            )?;
            collect_rows(rows)?
        } else {
            vec![session_id.to_string()]
        };
        Ok(ReportScope {
            session_id: session_id.to_string(),
            project_id: meta.project_id,
            session_ids,
            window_days,
            since,
        })
    }

    pub fn list_goal_rows_for_session(
        &self,
        session_id: &str,
        since: &str,
    ) -> Result<Vec<GoalRow>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, state, blocked_reason, updated_at
             FROM goals
             WHERE session_id = ?1 AND updated_at >= ?2
             ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map(params![session_id, since], |row| {
            Ok(GoalRow {
                id: row.get(0)?,
                state: row.get(1)?,
                blocked_reason: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })?;
        collect_rows(rows)
    }

    pub fn list_coding_eval_runs_for_scope(
        &self,
        scope: &ReportScope,
    ) -> Result<Vec<CodingEvalRunRecord>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut out = Vec::new();
        if let Some(project_id) = scope.project_id.as_deref() {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, project_id, suite, name, status, metrics_json,
                        source_type, source_id, created_at
                 FROM coding_eval_runs
                 WHERE project_id = ?1 AND created_at >= ?2
                 ORDER BY created_at DESC
                 LIMIT 200",
            )?;
            let rows = stmt.query_map(params![project_id, scope.since], row_to_eval_run)?;
            out.extend(collect_rows(rows)?);
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, project_id, suite, name, status, metrics_json,
                        source_type, source_id, created_at
                 FROM coding_eval_runs
                 WHERE session_id = ?1 AND created_at >= ?2
                 ORDER BY created_at DESC
                 LIMIT 200",
            )?;
            let rows = stmt.query_map(params![scope.session_id, scope.since], row_to_eval_run)?;
            out.extend(collect_rows(rows)?);
        }
        Ok(out)
    }

    pub fn list_coding_workflow_retros_for_scope(
        &self,
        scope: &ReportScope,
    ) -> Result<Vec<CodingWorkflowRetro>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        if let Some(project_id) = scope.project_id.as_deref() {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, project_id, workflow_run_id, run_state, summary,
                        signals_json, recommendations_json, created_at, updated_at
                 FROM coding_workflow_retros
                 WHERE project_id = ?1 AND updated_at >= ?2
                 ORDER BY updated_at DESC
                 LIMIT 100",
            )?;
            let rows = stmt.query_map(params![project_id, scope.since], row_to_retro)?;
            collect_rows(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, project_id, workflow_run_id, run_state, summary,
                        signals_json, recommendations_json, created_at, updated_at
                 FROM coding_workflow_retros
                 WHERE session_id = ?1 AND updated_at >= ?2
                 ORDER BY updated_at DESC
                 LIMIT 100",
            )?;
            let rows = stmt.query_map(params![scope.session_id, scope.since], row_to_retro)?;
            collect_rows(rows)
        }
    }

    pub fn list_domain_eval_campaign_learning_items(
        &self,
        scope: &ReportScope,
        limit: usize,
    ) -> Result<Vec<DomainCampaignLearningItem>> {
        let limit = limit.clamp(1, 100) as i64;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        if let Some(project_id) = scope.project_id.as_deref() {
            let mut stmt = conn.prepare(
                "SELECT c.id, c.name, c.status, c.domain, c.execution_mode,
                        i.id, i.task_id, i.task_title, i.domain, i.execution_mode,
                        i.provider_id, i.model_id, i.label, i.status, i.attempt,
                        i.fixture_run_id, i.eval_run_id, i.score, i.total_checks,
                        i.passed_checks, i.failed_checks, i.report_json, i.error, i.updated_at
                 FROM domain_eval_campaign_items i
                 JOIN domain_eval_campaigns c ON c.id = i.campaign_id
                 WHERE c.project_id = ?1
                   AND i.updated_at >= ?2
                   AND i.status IN ('failed', 'cancelled', 'interrupted')
                 ORDER BY i.updated_at DESC, i.id DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![project_id, scope.since, limit], |row| {
                row_to_domain_campaign_learning_item(row)
            })?;
            collect_rows(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT c.id, c.name, c.status, c.domain, c.execution_mode,
                        i.id, i.task_id, i.task_title, i.domain, i.execution_mode,
                        i.provider_id, i.model_id, i.label, i.status, i.attempt,
                        i.fixture_run_id, i.eval_run_id, i.score, i.total_checks,
                        i.passed_checks, i.failed_checks, i.report_json, i.error, i.updated_at
                 FROM domain_eval_campaign_items i
                 JOIN domain_eval_campaigns c ON c.id = i.campaign_id
                 WHERE c.session_id = ?1
                   AND i.updated_at >= ?2
                   AND i.status IN ('failed', 'cancelled', 'interrupted')
                 ORDER BY i.updated_at DESC, i.id DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![scope.session_id, scope.since, limit], |row| {
                row_to_domain_campaign_learning_item(row)
            })?;
            collect_rows(rows)
        }
    }

    pub fn get_coding_workflow_retro_for_run(
        &self,
        workflow_run_id: &str,
    ) -> Result<Option<CodingWorkflowRetro>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, session_id, project_id, workflow_run_id, run_state, summary,
                    signals_json, recommendations_json, created_at, updated_at
             FROM coding_workflow_retros
             WHERE workflow_run_id = ?1",
            params![workflow_run_id],
            row_to_retro,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn upsert_coding_workflow_retro(&self, retro: CodingWorkflowRetro) -> Result<()> {
        let signals_json = serde_json::to_string(&retro.signals)?;
        let recommendations_json = serde_json::to_string(&retro.recommendations)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO coding_workflow_retros (
                id, session_id, project_id, workflow_run_id, run_state, summary,
                signals_json, recommendations_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(workflow_run_id) DO UPDATE SET
                session_id = excluded.session_id,
                project_id = excluded.project_id,
                run_state = excluded.run_state,
                summary = excluded.summary,
                signals_json = excluded.signals_json,
                recommendations_json = excluded.recommendations_json,
                updated_at = excluded.updated_at",
            params![
                retro.id,
                retro.session_id,
                retro.project_id,
                retro.workflow_run_id,
                retro.run_state,
                retro.summary,
                signals_json,
                recommendations_json,
                retro.created_at,
                retro.updated_at,
            ],
        )?;
        Ok(())
    }

    fn get_coding_eval_run(&self, id: &str) -> Result<Option<CodingEvalRunRecord>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, session_id, project_id, suite, name, status, metrics_json,
                    source_type, source_id, created_at
             FROM coding_eval_runs
             WHERE id = ?1",
            params![id],
            row_to_eval_run,
        )
        .optional()
        .map_err(Into::into)
    }

    fn get_coding_eval_pack_run(&self, id: &str) -> Result<Option<CodingEvalPackRunRecord>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, session_id, project_id, pack_id, source_doc, label, baseline_kind,
                    status, selected_cases, automated_cases, skipped_cases, passed_cases,
                    failed_cases, total_checks, report_json, source_type, source_id, created_at
             FROM coding_eval_pack_runs
             WHERE id = ?1",
            params![id],
            row_to_eval_pack_run,
        )
        .optional()
        .map_err(Into::into)
    }

    fn get_coding_strategy_effect_run(
        &self,
        id: &str,
    ) -> Result<Option<CodingStrategyEffectRunRecord>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, session_id, project_id, strategy_type, baseline_label, candidate_label,
                    baseline_pack_run_id, candidate_pack_run_id, verdict, compared_cases,
                    pass_rate_delta, average_score_delta, context_recall_delta,
                    validation_violation_delta, scope_creep_delta, execution_failure_delta,
                    report_json, source_type, source_id, created_at
             FROM coding_strategy_effect_runs
             WHERE id = ?1",
            params![id],
            row_to_strategy_effect_run,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn get_coding_improvement_proposal(
        &self,
        id: &str,
    ) -> Result<Option<CodingImprovementProposal>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, session_id, project_id, kind, status, source_type, source_id,
                    title, body, payload_json, fingerprint, apply_result_json,
                    promotion_result_json,
                    created_at, updated_at, decided_at
             FROM coding_improvement_proposals
             WHERE id = ?1",
            params![id],
            row_to_proposal,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_coding_improvement_proposals_for_scope(
        &self,
        scope: &ReportScope,
    ) -> Result<Vec<CodingImprovementProposal>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        if let Some(project_id) = scope.project_id.as_deref() {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, project_id, kind, status, source_type, source_id,
                        title, body, payload_json, fingerprint, apply_result_json,
                        promotion_result_json,
                        created_at, updated_at, decided_at
                 FROM coding_improvement_proposals
                 WHERE project_id = ?1
                 ORDER BY CASE status WHEN 'draft' THEN 0 WHEN 'applied' THEN 1 WHEN 'promotion_failed' THEN 2 ELSE 3 END, updated_at DESC
                 LIMIT 100",
            )?;
            let rows = stmt.query_map(params![project_id], row_to_proposal)?;
            collect_rows(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, project_id, kind, status, source_type, source_id,
                        title, body, payload_json, fingerprint, apply_result_json,
                        promotion_result_json,
                        created_at, updated_at, decided_at
                 FROM coding_improvement_proposals
                 WHERE session_id = ?1
                 ORDER BY CASE status WHEN 'draft' THEN 0 WHEN 'applied' THEN 1 WHEN 'promotion_failed' THEN 2 ELSE 3 END, updated_at DESC
                 LIMIT 100",
            )?;
            let rows = stmt.query_map(params![scope.session_id], row_to_proposal)?;
            collect_rows(rows)
        }
    }

    pub fn insert_coding_improvement_proposal(
        &self,
        scope: &ReportScope,
        candidate: NewProposal,
    ) -> Result<bool> {
        let id = format!("cip_{}", uuid::Uuid::new_v4().simple());
        let now = now_rfc3339();
        let payload_json = stable_json(&candidate.payload)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "INSERT OR IGNORE INTO coding_improvement_proposals (
                id, session_id, project_id, kind, status, source_type, source_id,
                title, body, payload_json, fingerprint, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 'draft', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            params![
                id,
                scope.session_id,
                scope.project_id,
                candidate.kind,
                candidate.source_type,
                candidate.source_id,
                candidate.title,
                candidate.body,
                payload_json,
                candidate.fingerprint,
                now
            ],
        )?;
        Ok(changed > 0)
    }

    pub fn claim_coding_improvement_proposal_apply(
        &self,
        proposal_id: &str,
    ) -> Result<CodingImprovementProposal> {
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE coding_improvement_proposals
             SET status = 'applying',
                 updated_at = ?1
             WHERE id = ?2 AND status = 'draft'",
            params![now, proposal_id],
        )?;
        if changed == 0 {
            let status = conn
                .query_row(
                    "SELECT status FROM coding_improvement_proposals WHERE id = ?1",
                    params![proposal_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            match status {
                Some(status) => bail!(
                    "coding improvement proposal {proposal_id} is not draft (status: {status})"
                ),
                None => bail!("coding improvement proposal not found: {proposal_id}"),
            }
        }
        conn.query_row(
            "SELECT id, session_id, project_id, kind, status, source_type, source_id,
                    title, body, payload_json, fingerprint, apply_result_json,
                    promotion_result_json,
                    created_at, updated_at, decided_at
             FROM coding_improvement_proposals
             WHERE id = ?1",
            params![proposal_id],
            row_to_proposal,
        )
        .optional()?
        .ok_or_else(|| anyhow!("coding improvement proposal vanished after claim"))
    }

    pub fn claim_coding_improvement_proposal_promotion(
        &self,
        proposal_id: &str,
    ) -> Result<CodingImprovementProposal> {
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE coding_improvement_proposals
             SET status = 'promoting',
                 updated_at = ?1
             WHERE id = ?2 AND status IN ('applied','promotion_failed')",
            params![now, proposal_id],
        )?;
        if changed == 0 {
            let status = conn
                .query_row(
                    "SELECT status FROM coding_improvement_proposals WHERE id = ?1",
                    params![proposal_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            match status {
                Some(status) => bail!(
                    "coding improvement proposal {proposal_id} is not ready for promotion (status: {status})"
                ),
                None => bail!("coding improvement proposal not found: {proposal_id}"),
            }
        }
        conn.query_row(
            "SELECT id, session_id, project_id, kind, status, source_type, source_id,
                    title, body, payload_json, fingerprint, apply_result_json,
                    promotion_result_json,
                    created_at, updated_at, decided_at
             FROM coding_improvement_proposals
             WHERE id = ?1",
            params![proposal_id],
            row_to_proposal,
        )
        .optional()?
        .ok_or_else(|| anyhow!("coding improvement proposal vanished after promotion claim"))
    }

    pub fn set_coding_improvement_apply_result(
        &self,
        proposal_id: &str,
        status: &str,
        record: &CodingImprovementActionRecord,
    ) -> Result<()> {
        let now = now_rfc3339();
        let applied_at = record.applied_at.clone();
        let action_json = serde_json::to_string(record)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE coding_improvement_proposals
             SET status = ?1,
                 updated_at = ?2,
                 decided_at = ?2,
                 apply_result_json = ?3,
                 applied_at = ?4
             WHERE id = ?5 AND status = 'applying'",
            params![status, now, action_json, applied_at, proposal_id],
        )?;
        if changed == 0 {
            bail!("coding improvement proposal {proposal_id} is no longer applying");
        }
        Ok(())
    }

    pub fn set_coding_improvement_promotion_result(
        &self,
        proposal_id: &str,
        status: &str,
        record: &CodingImprovementPromotionRecord,
    ) -> Result<()> {
        let now = now_rfc3339();
        let promoted_at = record.promoted_at.clone();
        let promotion_json = serde_json::to_string(record)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE coding_improvement_proposals
             SET status = ?1,
                 updated_at = ?2,
                 promotion_result_json = ?3,
                 promoted_at = ?4
             WHERE id = ?5 AND status = 'promoting'",
            params![status, now, promotion_json, promoted_at, proposal_id],
        )?;
        if changed == 0 {
            bail!("coding improvement proposal {proposal_id} is no longer promoting");
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct GoalRow {
    #[allow(dead_code)]
    pub id: String,
    pub state: String,
    pub blocked_reason: Option<String>,
    #[allow(dead_code)]
    pub updated_at: String,
}

#[derive(Debug)]
pub struct DomainCampaignLearningItem {
    pub campaign_id: String,
    pub campaign_name: String,
    pub campaign_status: String,
    pub campaign_domain: Option<String>,
    pub campaign_execution_mode: String,
    pub item_id: String,
    pub task_id: String,
    pub task_title: String,
    pub domain: String,
    pub execution_mode: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub label: Option<String>,
    pub item_status: String,
    pub attempt: usize,
    pub fixture_run_id: Option<String>,
    pub eval_run_id: Option<String>,
    pub score: Option<f64>,
    pub total_checks: usize,
    pub passed_checks: usize,
    pub failed_checks: usize,
    pub report_json: Value,
    pub error: Option<String>,
    pub updated_at: String,
}

pub struct NewProposal {
    pub kind: String,
    pub source_type: String,
    pub source_id: String,
    pub title: String,
    pub body: String,
    pub payload: Value,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Default)]
pub struct ProposalGenerationFilter {
    pub source_type: Option<String>,
    pub source_id: Option<String>,
    pub proposal_kinds: BTreeSet<String>,
}

impl ProposalGenerationFilter {
    pub fn from_input(input: &GenerateCodingImprovementProposalsInput) -> Self {
        Self {
            source_type: normalize_optional_filter(input.source_type.as_deref()),
            source_id: normalize_optional_filter(input.source_id.as_deref()),
            proposal_kinds: input
                .proposal_kinds
                .iter()
                .filter_map(|kind| normalize_optional_filter(Some(kind)))
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.source_type.is_none() && self.source_id.is_none() && self.proposal_kinds.is_empty()
    }

    pub fn matches_candidate(&self, candidate: &NewProposal) -> bool {
        self.matches_parts(
            &candidate.source_type,
            &candidate.source_id,
            &candidate.kind,
        )
    }

    pub fn matches_proposal(&self, proposal: &CodingImprovementProposal) -> bool {
        self.matches_parts(&proposal.source_type, &proposal.source_id, &proposal.kind)
    }

    fn matches_parts(&self, source_type: &str, source_id: &str, kind: &str) -> bool {
        if let Some(expected) = self.source_type.as_deref() {
            if source_type != expected {
                return false;
            }
        }
        if let Some(expected) = self.source_id.as_deref() {
            if source_id != expected {
                return false;
            }
        }
        if !self.proposal_kinds.is_empty() && !self.proposal_kinds.contains(kind) {
            return false;
        }
        true
    }
}

fn normalize_optional_filter(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn short_hash(content: &str) -> String {
    let mut hash = blake3::hash(content.as_bytes()).to_hex().to_string();
    hash.truncate(16);
    hash
}

pub trait ReportScopeKey {
    fn scope_key(&self) -> String;
}

impl ReportScopeKey for CodingTrendReport {
    fn scope_key(&self) -> String {
        self.project_id
            .clone()
            .unwrap_or_else(|| self.session_id.clone())
    }
}

impl ReportScopeKey for ReportScope {
    fn scope_key(&self) -> String {
        self.project_id
            .clone()
            .unwrap_or_else(|| self.session_id.clone())
    }
}

pub fn failure_label(category: &str) -> Option<&'static str> {
    Some(match category {
        "validation_failed" => "Validation failed",
        "eval_failed" => "Coding eval failed",
        "review_blocker" => "Review blocker",
        "repair_loop_exhausted" => "Repair loop exhausted",
        "no_effective_diff_progress" => "No effective diff progress",
        "permission_stall" => "Permission stall",
        "context_miss" => "Context miss",
        "verification_selection_gap" => "Verification selection gap",
        "workflow_failed" => "Workflow failed",
        "workflow_blocked" => "Workflow blocked",
        "goal_failed" => "Goal failed",
        "correctness" => "Correctness",
        "security" => "Security",
        "maintainability" => "Maintainability",
        "tests" => "Tests",
        "frontend" => "Frontend",
        "accessibility" => "Accessibility",
        "concurrency" => "Concurrency",
        _ => return None,
    })
}

fn normalize_manual_proposal_status(status: &str) -> Result<&'static str> {
    match status.trim() {
        "draft" | "open" | "reopen" => Ok("draft"),
        "rejected" | "dismissed" | "reject" => Ok("rejected"),
        "accepted" | "approve" | "approved" | "applied" | "apply" => {
            bail!("use apply_coding_improvement_proposal to apply a proposal")
        }
        "applying" => bail!("applying status is managed by apply_coding_improvement_proposal"),
        "promoting" | "promoted" | "promotion_failed" => {
            bail!("promotion status is managed by promote_coding_improvement_proposal")
        }
        "failed" => bail!("failed status is reserved for apply errors"),
        other => bail!("unsupported coding improvement proposal status: {other}"),
    }
}

fn normalize_eval_status(status: &str) -> Result<&'static str> {
    match status.trim() {
        "passed" | "pass" | "ok" => Ok("passed"),
        "failed" | "fail" | "error" => Ok("failed"),
        "blocked" => Ok("blocked"),
        other => bail!("unsupported coding eval status: {other}"),
    }
}

fn normalize_baseline_kind(value: Option<&str>) -> String {
    let normalized = value
        .unwrap_or("deterministic_mock")
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_");
    match normalized.as_str() {
        "" | "deterministic" | "fixture" | "fixture_patch" | "mock" => {
            "deterministic_mock".to_string()
        }
        "mock_provider" | "provider_mock" => "mock_provider".to_string(),
        "external" | "external_provider" | "real_model" | "model" => "external_model".to_string(),
        other => other.to_string(),
    }
}

fn release_gate_thresholds(input: &CodingEvalReleaseGateInput) -> CodingEvalReleaseGateThresholds {
    CodingEvalReleaseGateThresholds {
        min_pack_runs: input
            .min_pack_runs
            .unwrap_or(DEFAULT_RELEASE_GATE_MIN_PACK_RUNS),
        min_strategy_effect_runs: input
            .min_strategy_effect_runs
            .unwrap_or(DEFAULT_RELEASE_GATE_MIN_STRATEGY_EFFECT_RUNS),
        min_pack_pass_rate: input
            .min_pack_pass_rate
            .unwrap_or(DEFAULT_RELEASE_GATE_MIN_PACK_PASS_RATE)
            .clamp(0.0, 1.0),
        require_external_model_pack: input.require_external_model_pack,
        max_regressed_strategy_effects: input
            .max_regressed_strategy_effects
            .unwrap_or(DEFAULT_RELEASE_GATE_MAX_REGRESSED_STRATEGY_EFFECTS),
        max_mixed_strategy_effects: input
            .max_mixed_strategy_effects
            .unwrap_or(DEFAULT_RELEASE_GATE_MAX_MIXED_STRATEGY_EFFECTS),
        max_missing_tool_call_runs: input
            .max_missing_tool_call_runs
            .unwrap_or(DEFAULT_RELEASE_GATE_MAX_MISSING_TOOL_CALL_RUNS),
        max_validation_violation_delta: input
            .max_validation_violation_delta
            .unwrap_or(DEFAULT_RELEASE_GATE_MAX_VALIDATION_VIOLATION_DELTA),
        max_scope_creep_delta: input
            .max_scope_creep_delta
            .unwrap_or(DEFAULT_RELEASE_GATE_MAX_SCOPE_CREEP_DELTA),
    }
}

fn learning_generalization_thresholds(
    input: &CodingLearningGeneralizationInput,
) -> CodingLearningGeneralizationThresholds {
    CodingLearningGeneralizationThresholds {
        min_projects: input
            .min_projects
            .unwrap_or(DEFAULT_GENERALIZATION_MIN_PROJECTS)
            .max(1),
        min_project_pack_runs: input
            .min_project_pack_runs
            .unwrap_or(DEFAULT_GENERALIZATION_MIN_PROJECT_PACK_RUNS),
        min_project_pack_pass_rate: input
            .min_project_pack_pass_rate
            .unwrap_or(DEFAULT_GENERALIZATION_MIN_PROJECT_PACK_PASS_RATE)
            .clamp(0.0, 1.0),
        min_strategy_effect_runs_per_project: input
            .min_strategy_effect_runs_per_project
            .unwrap_or(DEFAULT_GENERALIZATION_MIN_STRATEGY_EFFECT_RUNS_PER_PROJECT),
        require_promoted_learning: input.require_promoted_learning,
        require_external_model_pack: input.require_external_model_pack,
        max_regressed_projects: input
            .max_regressed_projects
            .unwrap_or(DEFAULT_GENERALIZATION_MAX_REGRESSED_PROJECTS),
        max_mixed_projects: input
            .max_mixed_projects
            .unwrap_or(DEFAULT_GENERALIZATION_MAX_MIXED_PROJECTS),
        max_validation_violation_delta_per_project: input
            .max_validation_violation_delta_per_project
            .unwrap_or(DEFAULT_GENERALIZATION_MAX_VALIDATION_VIOLATION_DELTA_PER_PROJECT),
        max_scope_creep_delta_per_project: input
            .max_scope_creep_delta_per_project
            .unwrap_or(DEFAULT_GENERALIZATION_MAX_SCOPE_CREEP_DELTA_PER_PROJECT),
    }
}

fn normalize_generalization_proposal_kinds(values: &[String]) -> Vec<String> {
    let mut kinds = values
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if kinds.is_empty() {
        kinds = vec![
            "guidance_candidate".to_string(),
            "skill_candidate".to_string(),
            "workflow_template".to_string(),
        ];
    }
    kinds.sort();
    kinds.dedup();
    kinds
}

fn push_gate_check(
    checks: &mut Vec<CodingEvalReleaseGateCheck>,
    name: &str,
    status: &str,
    severity: &str,
    expected: impl Into<String>,
    actual: impl Into<String>,
    detail: impl Into<String>,
) {
    checks.push(CodingEvalReleaseGateCheck {
        name: name.to_string(),
        status: status.to_string(),
        severity: severity.to_string(),
        expected: expected.into(),
        actual: actual.into(),
        detail: detail.into(),
    });
}

fn push_generalization_check(
    checks: &mut Vec<CodingLearningGeneralizationCheck>,
    name: &str,
    status: &str,
    severity: &str,
    expected: impl Into<String>,
    actual: impl Into<String>,
    detail: impl Into<String>,
) {
    checks.push(CodingLearningGeneralizationCheck {
        name: name.to_string(),
        status: status.to_string(),
        severity: severity.to_string(),
        expected: expected.into(),
        actual: actual.into(),
        detail: detail.into(),
    });
}

pub fn push_benchmark_check(
    checks: &mut Vec<CodingBenchmarkCenterCheck>,
    name: &str,
    status: impl Into<String>,
    severity: &str,
    expected: impl Into<String>,
    actual: impl Into<String>,
    detail: impl Into<String>,
) {
    checks.push(CodingBenchmarkCenterCheck {
        name: name.to_string(),
        status: status.into(),
        severity: severity.to_string(),
        expected: expected.into(),
        actual: actual.into(),
        detail: detail.into(),
    });
}

#[derive(Debug, Clone)]
struct BenchmarkLeaderboardItemRow {
    campaign_id: String,
    campaign_name: String,
    task_pack_id: String,
    source_doc: String,
    execution_mode: String,
    baseline_kind: String,
    item_id: String,
    provider_id: Option<String>,
    model_id: Option<String>,
    label: Option<String>,
    status: String,
    attempt: usize,
    pack_run_id: Option<String>,
    selected_cases: usize,
    passed_cases: usize,
    failed_cases: usize,
    skipped_cases: usize,
    total_checks: usize,
    updated_at: String,
    error: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct BenchmarkLeaderboardKey {
    task_pack_id: String,
    source_doc: String,
    execution_mode: String,
    baseline_kind: String,
    provider_id: Option<String>,
    model_id: Option<String>,
}

impl From<&BenchmarkLeaderboardItemRow> for BenchmarkLeaderboardKey {
    fn from(row: &BenchmarkLeaderboardItemRow) -> Self {
        Self {
            task_pack_id: row.task_pack_id.clone(),
            source_doc: row.source_doc.clone(),
            execution_mode: row.execution_mode.clone(),
            baseline_kind: row.baseline_kind.clone(),
            provider_id: row.provider_id.clone(),
            model_id: row.model_id.clone(),
        }
    }
}

#[derive(Default)]
struct BenchmarkLeaderboardAccumulator {
    label: Option<String>,
    campaign_ids: BTreeSet<String>,
    items: usize,
    passed_items: usize,
    failed_items: usize,
    skipped_items: usize,
    cancelled_items: usize,
    interrupted_items: usize,
    running_items: usize,
    queued_items: usize,
    attempts: usize,
    selected_cases: usize,
    passed_cases: usize,
    failed_cases: usize,
    skipped_cases: usize,
    total_checks: usize,
    evidence: Vec<CodingBenchmarkLeaderboardEvidence>,
}

impl BenchmarkLeaderboardAccumulator {
    fn add(&mut self, row: BenchmarkLeaderboardItemRow) {
        if self.label.is_none() {
            self.label = row.label.clone();
        }
        self.campaign_ids.insert(row.campaign_id.clone());
        self.items += 1;
        match row.status.as_str() {
            "passed" => self.passed_items += 1,
            "failed" => self.failed_items += 1,
            "skipped" => self.skipped_items += 1,
            "cancelled" => self.cancelled_items += 1,
            "interrupted" => self.interrupted_items += 1,
            "running" => self.running_items += 1,
            "queued" => self.queued_items += 1,
            _ => {}
        }
        self.attempts += row.attempt;
        self.selected_cases += row.selected_cases;
        self.passed_cases += row.passed_cases;
        self.failed_cases += row.failed_cases;
        self.skipped_cases += row.skipped_cases;
        self.total_checks += row.total_checks;
        self.evidence.push(CodingBenchmarkLeaderboardEvidence {
            campaign_id: row.campaign_id,
            campaign_name: row.campaign_name,
            item_id: row.item_id,
            pack_run_id: row.pack_run_id,
            provider_id: row.provider_id,
            model_id: row.model_id,
            label: row.label,
            status: row.status,
            updated_at: row.updated_at,
            error: row.error,
        });
    }

    fn into_row(
        mut self,
        key: BenchmarkLeaderboardKey,
        min_items: usize,
    ) -> CodingBenchmarkLeaderboardRow {
        self.evidence.truncate(6);
        let mut warnings = Vec::new();
        if self.items < min_items {
            warnings.push(format!("sample_size_below_{min_items}"));
        }
        if self.running_items > 0 || self.queued_items > 0 {
            warnings.push("campaign_incomplete".to_string());
        }
        if self.cancelled_items > 0 || self.interrupted_items > 0 {
            warnings.push("contains_cancelled_or_interrupted_items".to_string());
        }
        let label = self.label.unwrap_or_else(|| {
            key.provider_id
                .as_ref()
                .zip(key.model_id.as_ref())
                .map(|(provider, model)| format!("{provider}/{model}"))
                .unwrap_or_else(|| key.baseline_kind.clone())
        });
        CodingBenchmarkLeaderboardRow {
            rank: 0,
            label,
            provider_id: key.provider_id,
            model_id: key.model_id,
            task_pack_id: key.task_pack_id,
            source_doc: key.source_doc,
            execution_mode: key.execution_mode,
            baseline_kind: key.baseline_kind,
            campaigns: self.campaign_ids.len(),
            items: self.items,
            passed_items: self.passed_items,
            failed_items: self.failed_items,
            skipped_items: self.skipped_items,
            cancelled_items: self.cancelled_items,
            interrupted_items: self.interrupted_items,
            attempts: self.attempts,
            selected_cases: self.selected_cases,
            passed_cases: self.passed_cases,
            failed_cases: self.failed_cases,
            skipped_cases: self.skipped_cases,
            total_checks: self.total_checks,
            item_pass_rate: ratio(self.passed_items, self.passed_items + self.failed_items),
            case_pass_rate: ratio(self.passed_cases, self.passed_cases + self.failed_cases),
            warnings,
            evidence: self.evidence,
        }
    }
}

fn compare_benchmark_leaderboard_rows(
    left: &CodingBenchmarkLeaderboardRow,
    right: &CodingBenchmarkLeaderboardRow,
) -> std::cmp::Ordering {
    f64_sort_key(right.case_pass_rate)
        .cmp(&f64_sort_key(left.case_pass_rate))
        .then_with(|| f64_sort_key(right.item_pass_rate).cmp(&f64_sort_key(left.item_pass_rate)))
        .then_with(|| right.total_checks.cmp(&left.total_checks))
        .then_with(|| right.items.cmp(&left.items))
        .then_with(|| left.label.cmp(&right.label))
}

fn f64_sort_key(value: Option<f64>) -> i64 {
    value
        .map(|value| (value.clamp(0.0, 1.0) * 1_000_000.0).round() as i64)
        .unwrap_or(-1)
}

fn normalize_benchmark_task_pack_manifest(
    manifest: CodingBenchmarkTaskPackManifest,
) -> Result<CodingBenchmarkTaskPackManifest> {
    let pack_id = normalized_required_field(&manifest.pack_id, "task pack id")?;
    let version = normalized_required_field(&manifest.version, "task pack version")?;
    let name = normalized_required_field(&manifest.name, "task pack name")?;
    let source_kind = normalized_required_field(&manifest.source_kind, "task pack sourceKind")?;
    let license_note = normalized_required_field(&manifest.license_note, "task pack licenseNote")?;
    let privacy_note = normalized_required_field(&manifest.privacy_note, "task pack privacyNote")?;
    let redaction_status = normalize_redaction_status(
        Some(&manifest.redaction_status),
        "task pack redactionStatus",
    )?;
    if manifest.tasks.len() > MAX_BENCHMARK_CORPUS_TASKS {
        bail!(
            "benchmark task pack has too many tasks: {} > {}",
            manifest.tasks.len(),
            MAX_BENCHMARK_CORPUS_TASKS
        );
    }
    let mut tasks = Vec::with_capacity(manifest.tasks.len());
    for task in manifest.tasks {
        tasks.push(normalize_benchmark_task_manifest(task, &redaction_status)?);
    }
    Ok(CodingBenchmarkTaskPackManifest {
        pack_id,
        version,
        name,
        description: normalize_optional_string(manifest.description),
        status: Some(normalize_benchmark_pack_status(manifest.status.as_deref())?),
        source_kind,
        source_uri: normalize_optional_string(manifest.source_uri),
        repo_template: normalize_optional_string(manifest.repo_template),
        license_note,
        privacy_note,
        redaction_status,
        tasks,
    })
}

fn normalize_benchmark_task_manifest(
    task: CodingBenchmarkTaskPackTaskManifest,
    default_redaction_status: &str,
) -> Result<CodingBenchmarkTaskPackTaskManifest> {
    let task_id = normalized_required_field(&task.task_id, "task id")?;
    let version = normalized_required_field(&task.version, "task version")?;
    let title = normalized_required_field(&task.title, "task title")?;
    let task_type = normalized_required_field(&task.task_type, "task type")?;
    let difficulty = normalized_required_field(&task.difficulty, "task difficulty")?;
    let redaction_status = match task.redaction_status.as_deref() {
        Some(value) if !value.trim().is_empty() => {
            normalize_redaction_status(Some(value), "task redactionStatus")?
        }
        _ => default_redaction_status.to_string(),
    };
    Ok(CodingBenchmarkTaskPackTaskManifest {
        task_id,
        version,
        title,
        status: Some(normalize_benchmark_task_status(task.status.as_deref())?),
        task_type,
        difficulty,
        language: normalize_optional_string(task.language),
        framework: normalize_optional_string(task.framework),
        source_uri: normalize_optional_string(task.source_uri),
        repo_template: normalize_optional_string(task.repo_template),
        tags: normalize_string_vec(task.tags),
        success_criteria: normalize_string_vec(task.success_criteria),
        validation_commands: normalize_string_vec(task.validation_commands),
        allowed_paths: normalize_string_vec(task.allowed_paths),
        forbidden_paths: normalize_string_vec(task.forbidden_paths),
        calibration_notes: normalize_string_vec(task.calibration_notes),
        calibrated_at: normalize_optional_string(task.calibrated_at),
        license_note: normalize_optional_string(task.license_note),
        privacy_note: normalize_optional_string(task.privacy_note),
        redaction_status: Some(redaction_status),
    })
}

fn normalize_benchmark_pack_status(status: Option<&str>) -> Result<String> {
    let status = status.unwrap_or("draft").trim().to_ascii_lowercase();
    match status.as_str() {
        "" => Ok("draft".to_string()),
        "draft" | "active" | "archived" => Ok(status),
        other => bail!("unsupported benchmark task pack status: {other}"),
    }
}

fn normalize_benchmark_task_status(status: Option<&str>) -> Result<String> {
    let status = status.unwrap_or("draft").trim().to_ascii_lowercase();
    match status.as_str() {
        "" => Ok("draft".to_string()),
        "draft" | "active" | "archived" => Ok(status),
        other => bail!("unsupported benchmark task status: {other}"),
    }
}

fn normalize_redaction_status(status: Option<&str>, field: &str) -> Result<String> {
    let status = normalized_required_field(status.unwrap_or_default(), field)?.to_ascii_lowercase();
    match status.as_str() {
        "redacted" | "not_required" | "pending" => Ok(status),
        other => bail!("unsupported {field}: {other}"),
    }
}

fn normalized_required_field(value: &str, field: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{field} must not be empty");
    }
    Ok(value.to_string())
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalize_string_vec(values: Vec<String>) -> Vec<String> {
    let mut out = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

fn validate_benchmark_task_pack_manifest(
    manifest: &CodingBenchmarkTaskPackManifest,
) -> CodingBenchmarkTaskPackValidationReport {
    let mut checks = Vec::new();
    let mut warnings = Vec::new();
    push_benchmark_check(
        &mut checks,
        "pack_identity",
        if manifest.pack_id.trim().is_empty()
            || manifest.version.trim().is_empty()
            || manifest.name.trim().is_empty()
        {
            "failed"
        } else {
            "passed"
        },
        "error",
        "packId, version and name are present",
        format!("{}@{}", manifest.pack_id, manifest.version),
        "Task pack versions are immutable; a changed prompt, fixture, expected diff, or scorer schema must use a new version.",
    );
    let has_source = !manifest.source_kind.trim().is_empty()
        && (manifest.source_uri.is_some() || manifest.repo_template.is_some());
    push_benchmark_check(
        &mut checks,
        "source_traceability",
        if has_source { "passed" } else { "failed" },
        "error",
        "sourceKind plus sourceUri or repoTemplate",
        format!(
            "sourceKind={}, sourceUri={}, repoTemplate={}",
            manifest.source_kind,
            manifest.source_uri.is_some(),
            manifest.repo_template.is_some()
        ),
        "Imported real tasks must keep their origin visible for license, privacy and reproducibility review.",
    );
    let import_safe = !manifest.license_note.trim().is_empty()
        && !manifest.privacy_note.trim().is_empty()
        && matches!(
            manifest.redaction_status.as_str(),
            "redacted" | "not_required" | "pending"
        );
    push_benchmark_check(
        &mut checks,
        "import_safety",
        if import_safe { "passed" } else { "failed" },
        "error",
        "licenseNote, privacyNote and redactionStatus recorded",
        format!(
            "license={}, privacy={}, redactionStatus={}",
            !manifest.license_note.trim().is_empty(),
            !manifest.privacy_note.trim().is_empty(),
            manifest.redaction_status
        ),
        "Owner import records what is safe to store before any task can become benchmark evidence.",
    );
    let has_tasks = !manifest.tasks.is_empty();
    push_benchmark_check(
        &mut checks,
        "task_count",
        if has_tasks { "passed" } else { "failed" },
        "error",
        "at least 1 task",
        manifest.tasks.len().to_string(),
        "Empty task packs cannot improve benchmark coverage.",
    );

    let mut versions = BTreeSet::new();
    let mut duplicate_versions = Vec::new();
    let mut active_tasks = 0usize;
    let mut active_quality_failures = Vec::new();
    let mut risk_flags = Vec::new();
    for task in &manifest.tasks {
        let key = format!("{}@{}", task.task_id, task.version);
        if !versions.insert(key.clone()) {
            duplicate_versions.push(key.clone());
        }
        if task.status.as_deref().unwrap_or("draft") == "active" {
            active_tasks += 1;
            if task.success_criteria.is_empty()
                || task.validation_commands.is_empty()
                || (task.source_uri.is_none() && task.repo_template.is_none())
                || task.redaction_status.as_deref().unwrap_or_default() == "pending"
            {
                active_quality_failures.push(key.clone());
            }
        }
        let flags = benchmark_task_risk_flags(task);
        if !flags.is_empty() {
            if task.status.as_deref().unwrap_or("draft") == "active" {
                risk_flags.push(format!("{key}:{}", flags.join("|")));
            } else {
                warnings.push(format!("draft_task_risk:{key}:{}", flags.join("|")));
            }
        }
    }
    push_benchmark_check(
        &mut checks,
        "task_version_uniqueness",
        if duplicate_versions.is_empty() {
            "passed"
        } else {
            "failed"
        },
        if duplicate_versions.is_empty() { "info" } else { "error" },
        "no duplicate taskId@version inside pack",
        duplicate_versions.len().to_string(),
        "Task versioning must be explicit; importing the same task id/version twice would make history ambiguous.",
    );
    let pack_status = manifest.status.as_deref().unwrap_or("draft");
    let needs_active_tasks = pack_status == "active";
    push_benchmark_check(
        &mut checks,
        "active_task_presence",
        if !needs_active_tasks || active_tasks > 0 {
            "passed"
        } else {
            "failed"
        },
        if needs_active_tasks { "error" } else { "info" },
        "active packs contain at least 1 active task",
        active_tasks.to_string(),
        "Draft tasks are useful for curation but do not count as active benchmark coverage.",
    );
    push_benchmark_check(
        &mut checks,
        "active_task_quality",
        if active_quality_failures.is_empty() {
            "passed"
        } else {
            "failed"
        },
        if active_quality_failures.is_empty() {
            "info"
        } else {
            "error"
        },
        "every active task has source, criteria, validation and non-pending redaction",
        active_quality_failures.len().to_string(),
        "Active tasks must be reviewable and reproducible before they are allowed into gates or leaderboards.",
    );
    push_benchmark_check(
        &mut checks,
        "fixture_gaming_risk",
        if risk_flags.is_empty() {
            "passed"
        } else {
            "failed"
        },
        if risk_flags.is_empty() { "info" } else { "warning" },
        "0 active task risk flags",
        risk_flags.len().to_string(),
        "Tasks with thin criteria, missing validation, or overly broad write surface are easy to overfit.",
    );
    warnings.extend(active_quality_failures);
    warnings.extend(risk_flags);
    let status = if checks.iter().any(|check| check.status == "failed") {
        "failed"
    } else {
        "passed"
    }
    .to_string();
    CodingBenchmarkTaskPackValidationReport {
        generated_at: now_rfc3339(),
        status,
        pack_id: manifest.pack_id.clone(),
        version: manifest.version.clone(),
        checks,
        warnings,
    }
}

fn validate_benchmark_task_pack(
    pack: &CodingBenchmarkTaskPack,
) -> CodingBenchmarkTaskPackValidationReport {
    let manifest = CodingBenchmarkTaskPackManifest {
        pack_id: pack.pack_id.clone(),
        version: pack.version.clone(),
        name: pack.name.clone(),
        description: pack.description.clone(),
        status: Some(pack.status.clone()),
        source_kind: pack.source_kind.clone(),
        source_uri: pack.source_uri.clone(),
        repo_template: pack.repo_template.clone(),
        license_note: pack.license_note.clone(),
        privacy_note: pack.privacy_note.clone(),
        redaction_status: pack.redaction_status.clone(),
        tasks: pack
            .tasks
            .iter()
            .map(|task| CodingBenchmarkTaskPackTaskManifest {
                task_id: task.task_id.clone(),
                version: task.version.clone(),
                title: task.title.clone(),
                status: Some(task.status.clone()),
                task_type: task.task_type.clone(),
                difficulty: task.difficulty.clone(),
                language: task.language.clone(),
                framework: task.framework.clone(),
                source_uri: task.source_uri.clone(),
                repo_template: task.repo_template.clone(),
                tags: task.tags.clone(),
                success_criteria: task.success_criteria.clone(),
                validation_commands: task.validation_commands.clone(),
                allowed_paths: task.allowed_paths.clone(),
                forbidden_paths: task.forbidden_paths.clone(),
                calibration_notes: task.calibration_notes.clone(),
                calibrated_at: task.calibrated_at.clone(),
                license_note: task.license_note.clone(),
                privacy_note: task.privacy_note.clone(),
                redaction_status: Some(task.redaction_status.clone()),
            })
            .collect(),
    };
    validate_benchmark_task_pack_manifest(&manifest)
}

fn benchmark_task_risk_flags(task: &CodingBenchmarkTaskPackTaskManifest) -> Vec<String> {
    let mut flags = Vec::new();
    if task.success_criteria.len() < 2 {
        flags.push("thin_success_criteria".to_string());
    }
    if task.validation_commands.is_empty() {
        flags.push("missing_validation".to_string());
    }
    if task.allowed_paths.is_empty() && task.forbidden_paths.is_empty() {
        flags.push("wide_write_surface".to_string());
    }
    if task.calibration_notes.is_empty() {
        flags.push("missing_calibration_note".to_string());
    }
    flags
}

fn benchmark_task_fingerprint(task: &CodingBenchmarkTaskPackTaskManifest) -> Result<String> {
    Ok(short_hash(&serde_json::to_string(&json!({
        "title": &task.title,
        "taskType": &task.task_type,
        "difficulty": &task.difficulty,
        "language": &task.language,
        "framework": &task.framework,
        "successCriteria": &task.success_criteria,
        "validationCommands": &task.validation_commands,
        "allowedPaths": &task.allowed_paths,
        "forbiddenPaths": &task.forbidden_paths,
    }))?))
}

fn metric_buckets_from_counts(counts: BTreeMap<String, usize>) -> Vec<CodingMetricBucket> {
    counts
        .into_iter()
        .map(|(key, count)| CodingMetricBucket {
            label: failure_label(&key).unwrap_or(&key).to_string(),
            key,
            count,
        })
        .collect()
}

fn normalize_benchmark_report_type(report_type: &str) -> Result<String> {
    let report_type = report_type.trim().to_ascii_lowercase();
    match report_type.as_str() {
        "campaign" | "comparison" | "release" => Ok(report_type),
        other => bail!("unsupported benchmark report type: {other}"),
    }
}

fn benchmark_scope_label(session_id: Option<&String>, project_id: Option<&String>) -> String {
    if project_id.is_some() {
        "project"
    } else if session_id.is_some() {
        "session"
    } else {
        "global"
    }
    .to_string()
}

fn benchmark_report_status_from_campaign(campaign: &CodingBenchmarkCampaign) -> String {
    match campaign.status.as_str() {
        "passed" => "passed",
        "failed" | "partial" | "interrupted" => "failed",
        "cancelled" | "cancel_requested" | "queued" | "running" => "insufficient_data",
        _ => "insufficient_data",
    }
    .to_string()
}

fn benchmark_report_markdown(
    title: &str,
    status: &str,
    scope: &str,
    summary: &str,
    snapshot: &Value,
) -> Result<String> {
    let report_id = snapshot
        .get("reportId")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let report_type = snapshot
        .get("reportType")
        .and_then(Value::as_str)
        .unwrap_or("benchmark");
    let generated_at = snapshot
        .get("generatedAt")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut evidence = Vec::new();
    if let Some(campaign) = snapshot.get("campaign") {
        if let Some(id) = campaign.get("id").and_then(Value::as_str) {
            evidence.push(format!("- Campaign: `{id}`"));
        }
        if let Some(items) = campaign.get("items").and_then(Value::as_array) {
            for item in items.iter().take(6) {
                if let Some(pack_run_id) = item.get("packRunId").and_then(Value::as_str) {
                    evidence.push(format!("- Pack run: `{pack_run_id}`"));
                }
            }
        }
    }
    if let Some(leaderboard) = snapshot.get("leaderboard") {
        if let Some(rows) = leaderboard.get("rows").and_then(Value::as_array) {
            for row in rows.iter().take(6) {
                let label = row.get("label").and_then(Value::as_str).unwrap_or("row");
                let case_rate = row
                    .get("casePassRate")
                    .and_then(Value::as_f64)
                    .map(|value| format!("{:.0}%", value * 100.0))
                    .unwrap_or_else(|| "n/a".to_string());
                evidence.push(format!(
                    "- Leaderboard row `{label}` case pass rate: {case_rate}"
                ));
            }
        }
    }
    if let Some(release_gate) = snapshot.get("releaseGate") {
        if let Some(status) = release_gate.get("status").and_then(Value::as_str) {
            evidence.push(format!("- Release gate status: `{status}`"));
        }
    }
    if evidence.is_empty() {
        evidence.push("- No linked benchmark evidence in snapshot.".to_string());
    }

    Ok(format!(
        "# {title}\n\n## Executive Summary\n\n- Report id: `{report_id}`\n- Type: `{report_type}`\n- Status: `{status}`\n- Scope: `{scope}`\n- Generated at: `{generated_at}`\n\n{summary}\n\n## Evidence Links\n\n{}\n\n## Snapshot\n\nThe full immutable JSON snapshot is stored next to this report as `snapshot.json`.\n",
        evidence.join("\n")
    ))
}

fn benchmark_report_html(title: &str, markdown: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title><style>body{{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;max-width:960px;margin:40px auto;padding:0 24px;line-height:1.55}}pre{{white-space:pre-wrap;background:#f6f8fa;padding:16px;border-radius:8px}}</style></head><body><pre>{}</pre></body></html>",
        escape_html(title),
        escape_html(markdown)
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn serde_default_true() -> bool {
    true
}

fn normalized_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub fn continuous_benchmark_gate_thresholds(
    input: &CodingContinuousBenchmarkGateInput,
) -> Result<CodingContinuousBenchmarkGateThresholds> {
    let trigger_kind = normalize_benchmark_trigger_kind(input.trigger_kind.as_deref())?;
    let window_days = input
        .window_days
        .unwrap_or(DEFAULT_WINDOW_DAYS)
        .clamp(1, MAX_WINDOW_DAYS);
    let max_evidence_age_days = input
        .max_evidence_age_days
        .unwrap_or(DEFAULT_CONTINUOUS_GATE_MAX_EVIDENCE_AGE_DAYS)
        .clamp(1, MAX_CONTINUOUS_GATE_MAX_EVIDENCE_AGE_DAYS);
    let required_task_pack_id = normalized_optional(input.required_task_pack_id.as_deref());
    let mut required_baseline_kind = normalized_optional(input.required_baseline_kind.as_deref());
    if input.require_external_model && required_baseline_kind.is_none() {
        required_baseline_kind = Some("external_model".to_string());
    }
    let min_case_pass_rate = input
        .min_case_pass_rate
        .unwrap_or(DEFAULT_CONTINUOUS_GATE_MIN_CASE_PASS_RATE)
        .clamp(0.0, 1.0);
    let max_budget_usd = input.max_budget_usd.map(|value| value.max(0.0));
    Ok(CodingContinuousBenchmarkGateThresholds {
        trigger_kind,
        window_days,
        max_evidence_age_days,
        require_release_report_evidence: input.require_release_report_evidence,
        require_recent_campaign: input.require_recent_campaign,
        required_task_pack_id,
        required_baseline_kind,
        required_provider_id: normalized_optional(input.required_provider_id.as_deref()),
        required_model_id: normalized_optional(input.required_model_id.as_deref()),
        require_external_model: input.require_external_model,
        external_model_policy_enabled: input.external_model_policy_enabled,
        min_campaign_items: input
            .min_campaign_items
            .unwrap_or(DEFAULT_CONTINUOUS_GATE_MIN_CAMPAIGN_ITEMS)
            .clamp(1, MAX_BENCHMARK_CAMPAIGN_MODELS),
        min_case_pass_rate,
        max_open_backlog_items: input.max_open_backlog_items.unwrap_or(0),
        max_interrupted_campaigns: input.max_interrupted_campaigns.unwrap_or(0),
        max_provider_error_items: input.max_provider_error_items.unwrap_or(0),
        max_budget_exhausted_items: input.max_budget_exhausted_items.unwrap_or(0),
        max_budget_usd,
    })
}

fn normalize_benchmark_trigger_kind(value: Option<&str>) -> Result<String> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("manual")
        .to_ascii_lowercase();
    match value.as_str() {
        "manual" | "pre_release" | "strategy_changed" | "task_pack_updated" | "periodic" => {
            Ok(value)
        }
        _ => bail!("unsupported benchmark trigger kind: {value}"),
    }
}

fn normalize_benchmark_backlog_status(value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "open" | "in_progress" | "resolved" | "wont_fix" => Ok(value),
        _ => bail!("unsupported benchmark backlog status: {value}"),
    }
}

pub fn classify_benchmark_item_failure(status: &str, error: Option<&str>) -> Option<String> {
    if !matches!(status, "failed" | "interrupted" | "cancelled") {
        return None;
    }
    let text = error.unwrap_or_default().to_ascii_lowercase();
    if text.contains("budget") || text.contains("cost") || text.contains("quota") {
        Some("budget_exhausted".to_string())
    } else if text.contains("provider")
        || text.contains("api")
        || text.contains("rate limit")
        || text.contains("network")
        || text.contains("timeout")
    {
        Some("provider_error".to_string())
    } else if text.contains("approval") || text.contains("permission") {
        Some("approval_wait".to_string())
    } else if status == "interrupted" || status == "cancelled" {
        Some("benchmark_interrupted".to_string())
    } else {
        Some("benchmark_failed".to_string())
    }
}

fn benchmark_backlog_failures_from_report(
    report_json: &str,
) -> Vec<(String, String, String, Value)> {
    serde_json::from_str::<GoldTaskPackReport>(report_json)
        .map(|report| {
            report
                .cases
                .into_iter()
                .filter(|case_report| case_report.status != "passed")
                .take(8)
                .map(|case_report| {
                    let task_id = case_report.case.id.clone();
                    let title = if case_report.case.title.trim().is_empty() {
                        task_id.clone()
                    } else {
                        format!("{}: {}", task_id, case_report.case.title.trim())
                    };
                    let failure_category = case_report
                        .report
                        .as_ref()
                        .and_then(|report| report.task.as_ref())
                        .and_then(|task| task.failure_category.clone())
                        .or_else(|| {
                            case_report.error.as_deref().and_then(|error| {
                                classify_benchmark_item_failure("failed", Some(error))
                            })
                        })
                        .unwrap_or_else(|| "benchmark_failed".to_string());
                    let evidence = json!({
                        "caseId": task_id,
                        "caseTitle": case_report.case.title,
                        "status": case_report.status,
                        "fixtureName": case_report.fixture_name,
                        "error": case_report.error,
                        "taskType": case_report.case.task_type,
                        "successCriteria": case_report.case.success_criteria,
                    });
                    (task_id, title, failure_category, evidence)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn normalize_benchmark_campaign_models(
    models: Vec<CodingBenchmarkCampaignModel>,
) -> Result<Vec<CodingBenchmarkCampaignModel>> {
    let mut out = models
        .into_iter()
        .filter_map(|model| {
            let provider_id = model
                .provider_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let model_id = model
                .model_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let label = model
                .label
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            if provider_id.is_none() && model_id.is_none() && label.is_none() {
                None
            } else {
                Some(CodingBenchmarkCampaignModel {
                    provider_id,
                    model_id,
                    label,
                    credential_profile_ref: None,
                })
            }
        })
        .collect::<Vec<_>>();
    if out.is_empty() {
        out.push(CodingBenchmarkCampaignModel {
            provider_id: None,
            model_id: None,
            label: Some("deterministic".to_string()),
            credential_profile_ref: None,
        });
    }
    if out.len() > MAX_BENCHMARK_CAMPAIGN_MODELS {
        bail!(
            "benchmark campaign model matrix too large: {} > {}",
            out.len(),
            MAX_BENCHMARK_CAMPAIGN_MODELS
        );
    }
    for model in &out {
        if model.provider_id.is_some() != model.model_id.is_some() {
            bail!("benchmark campaign external model entries require both providerId and modelId");
        }
    }
    Ok(out)
}

fn benchmark_campaign_summary(
    items: &[CodingBenchmarkCampaignItem],
) -> CodingBenchmarkCampaignSummary {
    let mut summary = CodingBenchmarkCampaignSummary {
        total_items: items.len(),
        ..Default::default()
    };
    for item in items {
        match item.status.as_str() {
            "queued" => summary.queued_items += 1,
            "running" => summary.running_items += 1,
            "passed" => summary.passed_items += 1,
            "failed" => summary.failed_items += 1,
            "skipped" => summary.skipped_items += 1,
            "cancelled" => summary.cancelled_items += 1,
            "interrupted" => summary.interrupted_items += 1,
            _ => {}
        }
        summary.selected_cases += item.selected_cases;
        summary.passed_cases += item.passed_cases;
        summary.failed_cases += item.failed_cases;
        summary.skipped_cases += item.skipped_cases;
        summary.total_checks += item.total_checks;
    }
    summary.item_pass_rate = ratio(
        summary.passed_items,
        summary.passed_items + summary.failed_items,
    );
    summary.case_pass_rate = ratio(
        summary.passed_cases,
        summary.passed_cases + summary.failed_cases,
    );
    summary
}

fn coding_benchmark_campaign_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CodingBenchmarkCampaign> {
    let task_filter_json: String = row.get(9)?;
    let model_matrix_json: String = row.get(10)?;
    let model_matrix = serde_json::from_str(&model_matrix_json).unwrap_or_default();
    Ok(CodingBenchmarkCampaign {
        id: row.get(0)?,
        session_id: row.get(1)?,
        project_id: row.get(2)?,
        name: row.get(3)?,
        status: row.get(4)?,
        task_pack_id: row.get(5)?,
        source_doc: row.get(6)?,
        execution_mode: row.get(7)?,
        baseline_kind: row.get(8)?,
        task_filter: serde_json::from_str(&task_filter_json).unwrap_or_else(|_| json!({})),
        model_matrix,
        max_budget_usd: row.get(11)?,
        timeout_secs: row
            .get::<_, Option<i64>>(12)?
            .map(|value| value.max(0) as u64),
        summary: CodingBenchmarkCampaignSummary::default(),
        items: Vec::new(),
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        started_at: row.get(15)?,
        finished_at: row.get(16)?,
        error: row.get(17)?,
    })
}

fn coding_benchmark_campaign_item_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CodingBenchmarkCampaignItem> {
    Ok(CodingBenchmarkCampaignItem {
        id: row.get(0)?,
        campaign_id: row.get(1)?,
        provider_id: row.get(2)?,
        model_id: row.get(3)?,
        label: row.get(4)?,
        status: row.get(5)?,
        attempt: nonnegative_usize(row.get::<_, i64>(6)?),
        pack_run_id: row.get(7)?,
        selected_cases: nonnegative_usize(row.get::<_, i64>(8)?),
        passed_cases: nonnegative_usize(row.get::<_, i64>(9)?),
        failed_cases: nonnegative_usize(row.get::<_, i64>(10)?),
        skipped_cases: nonnegative_usize(row.get::<_, i64>(11)?),
        total_checks: nonnegative_usize(row.get::<_, i64>(12)?),
        started_at: row.get(13)?,
        finished_at: row.get(14)?,
        error: row.get(15)?,
    })
}

fn coding_benchmark_task_pack_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CodingBenchmarkTaskPack> {
    Ok(CodingBenchmarkTaskPack {
        id: row.get(0)?,
        pack_id: row.get(1)?,
        version: row.get(2)?,
        name: row.get(3)?,
        description: row.get(4)?,
        status: row.get(5)?,
        source_kind: row.get(6)?,
        source_uri: row.get(7)?,
        repo_template: row.get(8)?,
        license_note: row.get(9)?,
        privacy_note: row.get(10)?,
        redaction_status: row.get(11)?,
        imported_from: row.get(12)?,
        tasks: Vec::new(),
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        activated_at: row.get(15)?,
        archived_at: row.get(16)?,
    })
}

fn coding_benchmark_task_pack_task_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CodingBenchmarkTaskPackTask> {
    let tags_json: String = row.get(13)?;
    let success_criteria_json: String = row.get(14)?;
    let validation_commands_json: String = row.get(15)?;
    let allowed_paths_json: String = row.get(16)?;
    let forbidden_paths_json: String = row.get(17)?;
    let calibration_notes_json: String = row.get(18)?;
    let risk_flags_json: String = row.get(23)?;
    Ok(CodingBenchmarkTaskPackTask {
        id: row.get(0)?,
        pack_id: row.get(1)?,
        pack_version: row.get(2)?,
        task_id: row.get(3)?,
        version: row.get(4)?,
        title: row.get(5)?,
        status: row.get(6)?,
        task_type: row.get(7)?,
        difficulty: row.get(8)?,
        language: row.get(9)?,
        framework: row.get(10)?,
        source_uri: row.get(11)?,
        repo_template: row.get(12)?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        success_criteria: serde_json::from_str(&success_criteria_json).unwrap_or_default(),
        validation_commands: serde_json::from_str(&validation_commands_json).unwrap_or_default(),
        allowed_paths: serde_json::from_str(&allowed_paths_json).unwrap_or_default(),
        forbidden_paths: serde_json::from_str(&forbidden_paths_json).unwrap_or_default(),
        calibration_notes: serde_json::from_str(&calibration_notes_json).unwrap_or_default(),
        calibrated_at: row.get(19)?,
        license_note: row.get(20)?,
        privacy_note: row.get(21)?,
        redaction_status: row.get(22)?,
        risk_flags: serde_json::from_str(&risk_flags_json).unwrap_or_default(),
        fingerprint: row.get(24)?,
        created_at: row.get(25)?,
        updated_at: row.get(26)?,
    })
}

fn coding_benchmark_report_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CodingBenchmarkReport> {
    let campaign_ids_json: String = row.get(11)?;
    let snapshot_json: String = row.get(12)?;
    Ok(CodingBenchmarkReport {
        id: row.get(0)?,
        report_type: row.get(1)?,
        title: row.get(2)?,
        status: row.get(3)?,
        summary: row.get(4)?,
        scope: row.get(5)?,
        session_id: row.get(6)?,
        project_id: row.get(7)?,
        source_type: row.get(8)?,
        source_id: row.get(9)?,
        campaign_id: row.get(10)?,
        campaign_ids: serde_json::from_str(&campaign_ids_json).unwrap_or_default(),
        snapshot: serde_json::from_str(&snapshot_json).unwrap_or_else(|_| json!({})),
        markdown_path: row.get(13)?,
        json_path: row.get(14)?,
        html_path: row.get(15)?,
        release_evidence: row.get::<_, i64>(16)? != 0,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
        marked_release_at: row.get(19)?,
    })
}

fn coding_benchmark_backlog_item_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CodingBenchmarkBacklogItem> {
    let evidence_json: String = row.get(18)?;
    Ok(CodingBenchmarkBacklogItem {
        id: row.get(0)?,
        status: row.get(1)?,
        severity: row.get(2)?,
        title: row.get(3)?,
        failure_category: row.get(4)?,
        scope: row.get(5)?,
        session_id: row.get(6)?,
        project_id: row.get(7)?,
        campaign_id: row.get(8)?,
        campaign_item_id: row.get(9)?,
        pack_run_id: row.get(10)?,
        task_pack_id: row.get(11)?,
        task_id: row.get(12)?,
        provider_id: row.get(13)?,
        model_id: row.get(14)?,
        label: row.get(15)?,
        baseline_kind: row.get(16)?,
        execution_mode: row.get(17)?,
        evidence: serde_json::from_str(&evidence_json).unwrap_or_else(|_| json!({})),
        proposal_id: row.get(19)?,
        created_at: row.get(20)?,
        updated_at: row.get(21)?,
        resolved_at: row.get(22)?,
    })
}

fn release_gate_filter(
    scope: &ReleaseGateScope,
    fact_alias: &str,
    time_expr: &str,
) -> (String, Vec<String>) {
    let mut clauses = vec![
        format!("{time_expr} >= ?"),
        format!(
            "({fact_alias}.session_id IS NULL OR (s.is_cron = 0 AND s.parent_session_id IS NULL AND s.incognito = 0))"
        ),
    ];
    let mut params = vec![scope.since.clone()];
    if let Some(project_id) = scope.project_id.as_ref() {
        clauses.push(format!(
            "COALESCE({fact_alias}.project_id, s.project_id) = ?"
        ));
        params.push(project_id.clone());
    } else if let Some(session_id) = scope.session_id.as_ref() {
        clauses.push(format!("{fact_alias}.session_id = ?"));
        params.push(session_id.clone());
    }
    (format!("WHERE {}", clauses.join(" AND ")), params)
}

fn benchmark_center_filter(
    scope: &BenchmarkCenterScope,
    fact_alias: &str,
    time_expr: &str,
) -> (String, Vec<String>) {
    let mut clauses = vec![
        format!("{time_expr} >= ?"),
        format!(
            "({fact_alias}.session_id IS NULL OR (s.is_cron = 0 AND s.parent_session_id IS NULL AND s.incognito = 0))"
        ),
    ];
    let mut params = vec![scope.since.clone()];
    if let Some(project_id) = scope.project_id.as_ref() {
        clauses.push(format!(
            "COALESCE({fact_alias}.project_id, s.project_id) = ?"
        ));
        params.push(project_id.clone());
    } else if let Some(session_id) = scope.session_id.as_ref() {
        clauses.push(format!("{fact_alias}.session_id = ?"));
        params.push(session_id.clone());
    }
    (format!("WHERE {}", clauses.join(" AND ")), params)
}

fn learning_generalization_filter(
    scope: &LearningGeneralizationScope,
    fact_alias: &str,
    time_expr: &str,
    proposal_only: bool,
    source_scoped: bool,
) -> (String, Vec<String>) {
    let project_expr = format!("COALESCE({fact_alias}.project_id, s.project_id)");
    let mut clauses = vec![
        format!("{time_expr} >= ?"),
        format!(
            "({fact_alias}.session_id IS NULL OR (s.is_cron = 0 AND s.parent_session_id IS NULL AND s.incognito = 0))"
        ),
        format!("{project_expr} IS NOT NULL"),
        format!("TRIM({project_expr}) <> ''"),
    ];
    let mut params = vec![scope.since.clone()];

    if let Some(project_id) = scope.project_id.as_ref() {
        clauses.push(format!("{project_expr} = ?"));
        params.push(project_id.clone());
    } else if let Some(session_id) = scope.session_id.as_ref() {
        clauses.push(format!("{fact_alias}.session_id = ?"));
        params.push(session_id.clone());
    }

    if proposal_only {
        clauses.push(format!("{fact_alias}.status = 'promoted'"));
        if !scope.proposal_kinds.is_empty() {
            let placeholders = std::iter::repeat_n("?", scope.proposal_kinds.len())
                .collect::<Vec<_>>()
                .join(", ");
            clauses.push(format!("{fact_alias}.kind IN ({placeholders})"));
            params.extend(scope.proposal_kinds.iter().cloned());
        }
    }

    if source_scoped {
        if let Some(source_type) = scope.source_type.as_ref() {
            clauses.push(format!("{fact_alias}.source_type = ?"));
            params.push(source_type.clone());
        }
        if let Some(source_id) = scope.source_id.as_ref() {
            clauses.push(format!("{fact_alias}.source_id = ?"));
            params.push(source_id.clone());
        }
    }

    (format!("WHERE {}", clauses.join(" AND ")), params)
}

fn benchmark_failed_cases_summary(report_json: &str) -> Vec<String> {
    serde_json::from_str::<GoldTaskPackReport>(report_json)
        .map(|report| {
            report
                .cases
                .into_iter()
                .filter(|case_report| case_report.status != "passed")
                .take(4)
                .map(|case_report| {
                    let title = case_report.case.title.trim();
                    if title.is_empty() {
                        case_report.case.id
                    } else {
                        format!("{}: {}", case_report.case.id, title)
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn project_status_rank(status: &str) -> usize {
    match status {
        "failed" => 0,
        "insufficient_data" => 1,
        "passed" => 2,
        _ => 3,
    }
}

pub fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    if denominator == 0 {
        None
    } else {
        Some((numerator as f64 / denominator as f64 * 1000.0).round() / 1000.0)
    }
}

fn nonnegative_usize(value: i64) -> usize {
    value.max(0) as usize
}

fn truncate_for_storage(value: &str, max_bytes: usize) -> String {
    crate::truncate_utf8(value, max_bytes).to_string()
}

fn stable_json(value: &Value) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>> {
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn ensure_column(conn: &Connection, table: &str, column: &str, alter_sql: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let columns = collect_rows(rows)?;
    if !columns.iter().any(|name| name == column) {
        conn.execute_batch(alter_sql)?;
    }
    Ok(())
}

fn row_to_eval_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<CodingEvalRunRecord> {
    let metrics_json: String = row.get(6)?;
    Ok(CodingEvalRunRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        project_id: row.get(2)?,
        suite: row.get(3)?,
        name: row.get(4)?,
        status: row.get(5)?,
        metrics: serde_json::from_str(&metrics_json).unwrap_or_else(|_| json!({})),
        source_type: row.get(7)?,
        source_id: row.get(8)?,
        created_at: row.get(9)?,
    })
}

fn row_to_eval_pack_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<CodingEvalPackRunRecord> {
    let report_json: String = row.get(14)?;
    let mut report =
        serde_json::from_str::<GoldTaskPackReport>(&report_json).unwrap_or_else(|_| {
            GoldTaskPackReport {
                pack_id: row.get(3).unwrap_or_default(),
                source_doc: row.get(4).unwrap_or_default(),
                pack_run_id: None,
                selected_cases: row.get::<_, i64>(8).unwrap_or_default().max(0) as usize,
                automated_cases: row.get::<_, i64>(9).unwrap_or_default().max(0) as usize,
                skipped_cases: row.get::<_, i64>(10).unwrap_or_default().max(0) as usize,
                passed_cases: row.get::<_, i64>(11).unwrap_or_default().max(0) as usize,
                failed_cases: row.get::<_, i64>(12).unwrap_or_default().max(0) as usize,
                total_checks: row.get::<_, i64>(13).unwrap_or_default().max(0) as usize,
                passed: row
                    .get::<_, String>(7)
                    .map(|status| status == "passed")
                    .unwrap_or(false),
                cases: Vec::new(),
            }
        });
    let id: String = row.get(0)?;
    report.pack_run_id = Some(id.clone());
    Ok(CodingEvalPackRunRecord {
        id,
        session_id: row.get(1)?,
        project_id: row.get(2)?,
        pack_id: row.get(3)?,
        source_doc: row.get(4)?,
        label: row.get(5)?,
        baseline_kind: row.get(6)?,
        status: row.get(7)?,
        selected_cases: row.get::<_, i64>(8)?.max(0) as usize,
        automated_cases: row.get::<_, i64>(9)?.max(0) as usize,
        skipped_cases: row.get::<_, i64>(10)?.max(0) as usize,
        passed_cases: row.get::<_, i64>(11)?.max(0) as usize,
        failed_cases: row.get::<_, i64>(12)?.max(0) as usize,
        total_checks: row.get::<_, i64>(13)?.max(0) as usize,
        report,
        source_type: row.get(15)?,
        source_id: row.get(16)?,
        created_at: row.get(17)?,
    })
}

fn row_to_domain_campaign_learning_item(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<DomainCampaignLearningItem> {
    let report_json: String = row.get(21)?;
    Ok(DomainCampaignLearningItem {
        campaign_id: row.get(0)?,
        campaign_name: row.get(1)?,
        campaign_status: row.get(2)?,
        campaign_domain: row.get(3)?,
        campaign_execution_mode: row.get(4)?,
        item_id: row.get(5)?,
        task_id: row.get(6)?,
        task_title: row.get(7)?,
        domain: row.get(8)?,
        execution_mode: row.get(9)?,
        provider_id: row.get(10)?,
        model_id: row.get(11)?,
        label: row.get(12)?,
        item_status: row.get(13)?,
        attempt: row.get::<_, i64>(14)?.max(0) as usize,
        fixture_run_id: row.get(15)?,
        eval_run_id: row.get(16)?,
        score: row.get(17)?,
        total_checks: row.get::<_, i64>(18)?.max(0) as usize,
        passed_checks: row.get::<_, i64>(19)?.max(0) as usize,
        failed_checks: row.get::<_, i64>(20)?.max(0) as usize,
        report_json: serde_json::from_str(&report_json).unwrap_or_else(|_| json!({})),
        error: row.get(22)?,
        updated_at: row.get(23)?,
    })
}

fn row_to_strategy_effect_run(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CodingStrategyEffectRunRecord> {
    let report_json: String = row.get(16)?;
    let mut report =
        serde_json::from_str::<StrategyEffectReport>(&report_json).unwrap_or_else(|_| {
            StrategyEffectReport {
                run_id: None,
                strategy_type: row.get(3).unwrap_or_else(|_| "strategy".to_string()),
                baseline_label: row.get(4).unwrap_or_else(|_| "baseline".to_string()),
                candidate_label: row.get(5).unwrap_or_else(|_| "candidate".to_string()),
                verdict: row.get(8).unwrap_or_else(|_| "inconclusive".to_string()),
                compared_cases: row.get::<_, i64>(9).unwrap_or_default().max(0) as usize,
                baseline_only_cases: Vec::new(),
                candidate_only_cases: Vec::new(),
                summary: Default::default(),
                dimensions: Vec::new(),
                cases: Vec::new(),
                regressions: Vec::new(),
                improvements: Vec::new(),
            }
        });
    let id: String = row.get(0)?;
    report.run_id = Some(id.clone());
    Ok(CodingStrategyEffectRunRecord {
        id,
        session_id: row.get(1)?,
        project_id: row.get(2)?,
        strategy_type: row.get(3)?,
        baseline_label: row.get(4)?,
        candidate_label: row.get(5)?,
        baseline_pack_run_id: row.get(6)?,
        candidate_pack_run_id: row.get(7)?,
        verdict: row.get(8)?,
        compared_cases: row.get::<_, i64>(9)?.max(0) as usize,
        pass_rate_delta: row.get(10)?,
        average_score_delta: row.get(11)?,
        context_recall_delta: row.get(12)?,
        validation_violation_delta: row.get::<_, i64>(13)? as isize,
        scope_creep_delta: row.get::<_, i64>(14)? as isize,
        execution_failure_delta: row.get::<_, i64>(15)? as isize,
        report,
        source_type: row.get(17)?,
        source_id: row.get(18)?,
        created_at: row.get(19)?,
    })
}

fn row_to_proposal(row: &rusqlite::Row<'_>) -> rusqlite::Result<CodingImprovementProposal> {
    let payload_json: String = row.get(9)?;
    let action_json: Option<String> = row.get(11)?;
    let promotion_json: Option<String> = row.get(12)?;
    Ok(CodingImprovementProposal {
        id: row.get(0)?,
        session_id: row.get(1)?,
        project_id: row.get(2)?,
        kind: row.get(3)?,
        status: row.get(4)?,
        source_type: row.get(5)?,
        source_id: row.get(6)?,
        title: row.get(7)?,
        body: row.get(8)?,
        payload: serde_json::from_str(&payload_json).unwrap_or_else(|_| json!({})),
        fingerprint: row.get(10)?,
        action: action_json
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok()),
        promotion: promotion_json
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok()),
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        decided_at: row.get(15)?,
    })
}

fn row_to_retro(row: &rusqlite::Row<'_>) -> rusqlite::Result<CodingWorkflowRetro> {
    let signals_json: String = row.get(6)?;
    let recommendations_json: String = row.get(7)?;
    Ok(CodingWorkflowRetro {
        id: row.get(0)?,
        session_id: row.get(1)?,
        project_id: row.get(2)?,
        workflow_run_id: row.get(3)?,
        run_state: row.get(4)?,
        summary: row.get(5)?,
        signals: serde_json::from_str(&signals_json).unwrap_or_default(),
        recommendations: serde_json::from_str(&recommendations_json).unwrap_or_default(),
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

#[cfg(all(test, feature = "eval-internal-tests"))]
mod tests {
    use super::*;

    fn test_db() -> (tempfile::TempDir, SessionDB) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open_ephemeral_for_test(&dir.path().join("sessions.db"))
            .expect("session db");
        ensure_channel_conversations_table(&db);
        (dir, db)
    }

    fn ensure_channel_conversations_table(db: &SessionDB) {
        let conn = db.conn.lock().expect("lock connection");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS channel_conversations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                chat_id TEXT NOT NULL,
                thread_id TEXT,
                session_id TEXT NOT NULL,
                sender_id TEXT,
                sender_name TEXT,
                chat_type TEXT NOT NULL DEFAULT 'dm',
                source TEXT NOT NULL DEFAULT 'inbound',
                attached_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );",
        )
        .expect("create channel conversations table");
    }

    fn sample_task_pack_manifest(status: &str, version: &str) -> CodingBenchmarkTaskPackManifest {
        CodingBenchmarkTaskPackManifest {
            pack_id: "sample-real-project-pack".to_string(),
            version: version.to_string(),
            name: "Sample real project pack".to_string(),
            description: Some("Synthetic manifest for corpus tests".to_string()),
            status: Some(status.to_string()),
            source_kind: "fixture_repo".to_string(),
            source_uri: Some("local://fixtures/sample-real-project-pack".to_string()),
            repo_template: Some("fixture://react-rust-desktop-app".to_string()),
            license_note: "Synthetic local fixture".to_string(),
            privacy_note: "No private source content".to_string(),
            redaction_status: "not_required".to_string(),
            tasks: vec![
                CodingBenchmarkTaskPackTaskManifest {
                    task_id: "REAL-BUGFIX-001".to_string(),
                    version: "v1".to_string(),
                    title: "Repair benchmark status rendering".to_string(),
                    status: Some("active".to_string()),
                    task_type: "bugfix".to_string(),
                    difficulty: "medium".to_string(),
                    language: Some("typescript".to_string()),
                    framework: Some("react".to_string()),
                    source_uri: Some("local://fixtures/sample/issues/bugfix-001".to_string()),
                    repo_template: Some("fixture://react-rust-desktop-app".to_string()),
                    tags: vec!["dashboard".to_string()],
                    success_criteria: vec![
                        "Campaign status stays in sync after reload.".to_string(),
                        "Retry action only appears for failed terminal states.".to_string(),
                    ],
                    validation_commands: vec!["pnpm typecheck".to_string()],
                    allowed_paths: vec!["src/components/dashboard/**".to_string()],
                    forbidden_paths: vec!["crates/**".to_string()],
                    calibration_notes: vec!["Manual calibration completed".to_string()],
                    calibrated_at: Some(now_rfc3339()),
                    license_note: Some("Synthetic local fixture".to_string()),
                    privacy_note: Some("No private source content".to_string()),
                    redaction_status: Some("not_required".to_string()),
                },
                CodingBenchmarkTaskPackTaskManifest {
                    task_id: "REAL-REFACTOR-002".to_string(),
                    version: "v1".to_string(),
                    title: "Separate corpus validation from runner state".to_string(),
                    status: Some("active".to_string()),
                    task_type: "refactor".to_string(),
                    difficulty: "hard".to_string(),
                    language: Some("rust".to_string()),
                    framework: Some("ha-core".to_string()),
                    source_uri: Some("local://fixtures/sample/issues/refactor-002".to_string()),
                    repo_template: Some("fixture://react-rust-desktop-app".to_string()),
                    tags: vec!["benchmark".to_string()],
                    success_criteria: vec![
                        "Validation is deterministic.".to_string(),
                        "Activation fails closed on missing active task metadata.".to_string(),
                    ],
                    validation_commands: vec!["cargo check -p ha-core --locked".to_string()],
                    allowed_paths: vec!["crates/ha-core/src/coding_improvement.rs".to_string()],
                    forbidden_paths: vec!["src/**".to_string()],
                    calibration_notes: vec!["Manual calibration completed".to_string()],
                    calibrated_at: Some(now_rfc3339()),
                    license_note: Some("Synthetic local fixture".to_string()),
                    privacy_note: Some("No private source content".to_string()),
                    redaction_status: Some("not_required".to_string()),
                },
            ],
        }
    }

    fn insert_promoted_learning(
        db: &SessionDB,
        session_id: &str,
        project_id: &str,
        proposal_id: &str,
        source_id: &str,
    ) {
        let now = now_rfc3339();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO coding_improvement_proposals (
                id, session_id, project_id, kind, status, source_type, source_id,
                title, body, payload_json, fingerprint, created_at, updated_at,
                decided_at, apply_result_json, applied_at, promotion_result_json, promoted_at
             ) VALUES (
                ?1, ?2, ?3, 'guidance_candidate', 'promoted', 'failure_feedback', ?4,
                'Cross project validation guidance', 'Use targeted verification evidence.',
                '{}', ?5, ?6, ?6, ?6, ?7, ?6, ?8, ?6
             )",
            params![
                proposal_id,
                session_id,
                project_id,
                source_id,
                format!("generalization:{project_id}:{source_id}"),
                now,
                json!({"applied":true,"artifacts":[{"kind":"create_file","path":"draft.md"}],"error":null,"appliedAt":now}).to_string(),
                json!({"promoted":true,"artifacts":[{"kind":"create_promoted_file","path":"guidance.md"}],"error":null,"promotedAt":now}).to_string(),
            ],
        )
        .unwrap();
    }

    fn insert_generalization_pack(
        db: &SessionDB,
        session_id: &str,
        project_id: &str,
        pack_id: &str,
        status: &str,
    ) {
        let now = now_rfc3339();
        let (passed_cases, failed_cases) = if status == "passed" { (2, 0) } else { (1, 1) };
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO coding_eval_pack_runs (
                id, session_id, project_id, pack_id, source_doc, label,
                baseline_kind, status, selected_cases, automated_cases,
                skipped_cases, passed_cases, failed_cases, total_checks,
                report_json, source_type, source_id, created_at
             ) VALUES (
                ?1, ?2, ?3, 'phase5-gold-task-pack',
                'docs/roadmap/coding-eval.md', 'generalization evidence',
                'deterministic_mock', ?4, 2, 2, 0, ?5, ?6, 8,
                '{}', 'gold_task_pack', 'phase5-gold-task-pack', ?7
             )",
            params![
                pack_id,
                session_id,
                project_id,
                status,
                passed_cases,
                failed_cases,
                now
            ],
        )
        .unwrap();
    }

    fn insert_generalization_strategy_effect(
        db: &SessionDB,
        session_id: &str,
        project_id: &str,
        run_id: &str,
        source_id: &str,
        verdict: &str,
    ) {
        let now = now_rfc3339();
        let (pass_delta, score_delta, validation_delta, scope_delta, execution_delta) =
            if verdict == "regressed" {
                (-0.25, -0.2, 1, 1, 0)
            } else {
                (0.25, 0.2, 0, 0, 0)
            };
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO coding_strategy_effect_runs (
                id, session_id, project_id, strategy_type, baseline_label,
                candidate_label, baseline_pack_run_id, candidate_pack_run_id,
                verdict, compared_cases, pass_rate_delta, average_score_delta,
                context_recall_delta, validation_violation_delta, scope_creep_delta,
                execution_failure_delta, report_json, source_type, source_id, created_at
             ) VALUES (
                ?1, ?2, ?3, 'guidance_candidate', 'before', 'after',
                NULL, NULL, ?4, 2, ?5, ?6, 0.1, ?7, ?8, ?9, '{}',
                'failure_feedback', ?10, ?11
             )",
            params![
                run_id,
                session_id,
                project_id,
                verdict,
                pass_delta,
                score_delta,
                validation_delta,
                scope_delta,
                execution_delta,
                source_id,
                now
            ],
        )
        .unwrap();
    }

    #[test]
    fn release_gate_passes_clean_pack_and_strategy_history() {
        let (_dir, db) = test_db();
        let project_id = "proj-release-gate-pass";
        let session = db
            .create_session_with_project(
                crate::agent_loader::DEFAULT_AGENT_ID,
                Some(project_id),
                None,
            )
            .unwrap();
        let now = now_rfc3339();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO coding_eval_pack_runs (
                    id, session_id, project_id, pack_id, source_doc, label,
                    baseline_kind, status, selected_cases, automated_cases,
                    skipped_cases, passed_cases, failed_cases, total_checks,
                    report_json, source_type, source_id, created_at
                 ) VALUES (
                    'cepr_release_pass', ?1, ?2, 'phase5-gold-task-pack',
                    'docs/roadmap/coding-eval.md', 'clean candidate',
                    'deterministic_mock', 'passed', 2, 2, 0, 2, 0, 8,
                    '{}', 'gold_task_pack', 'phase5-gold-task-pack', ?3
                 )",
                params![session.id, project_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO coding_strategy_effect_runs (
                    id, session_id, project_id, strategy_type, baseline_label,
                    candidate_label, baseline_pack_run_id, candidate_pack_run_id,
                    verdict, compared_cases, pass_rate_delta, average_score_delta,
                    context_recall_delta, validation_violation_delta, scope_creep_delta,
                    execution_failure_delta, report_json, source_type, source_id, created_at
                 ) VALUES (
                    'cser_release_pass', ?1, ?2, 'workflow_policy', 'before',
                    'after', NULL, 'cepr_release_pass', 'improved', 2, 0.5, 0.25,
                    0.1, 0, 0, 0, '{}', 'strategy_effect', 'workflow_policy', ?3
                 )",
                params![session.id, project_id, now],
            )
            .unwrap();
        }

        let report = db
            .evaluate_coding_eval_release_gate(CodingEvalReleaseGateInput {
                session_id: Some(session.id.clone()),
                min_strategy_effect_runs: Some(1),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.status, "passed");
        assert_eq!(report.scope, "project");
        assert_eq!(report.project_id.as_deref(), Some(project_id));
        assert_eq!(report.summary.pack_runs, 1);
        assert_eq!(report.summary.strategy_effect_runs, 1);
        assert_eq!(report.summary.missing_tool_call_runs, 0);
        assert!(report.checks.iter().all(|check| check.status == "passed"));
    }

    #[test]
    fn release_gate_fails_on_strategy_regression_and_missing_tool_call() {
        let (_dir, db) = test_db();
        let project_id = "proj-release-gate-fail";
        let session = db
            .create_session_with_project(
                crate::agent_loader::DEFAULT_AGENT_ID,
                Some(project_id),
                None,
            )
            .unwrap();
        let now = now_rfc3339();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO coding_eval_pack_runs (
                    id, session_id, project_id, pack_id, source_doc, label,
                    baseline_kind, status, selected_cases, automated_cases,
                    skipped_cases, passed_cases, failed_cases, total_checks,
                    report_json, source_type, source_id, created_at
                 ) VALUES (
                    'cepr_release_regressed', ?1, ?2, 'phase5-gold-task-pack',
                    'docs/roadmap/coding-eval.md', 'regressed candidate',
                    'mock_provider', 'passed', 2, 2, 0, 2, 0, 8,
                    '{}', 'gold_task_pack', 'phase5-gold-task-pack', ?3
                 )",
                params![session.id, project_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO coding_strategy_effect_runs (
                    id, session_id, project_id, strategy_type, baseline_label,
                    candidate_label, baseline_pack_run_id, candidate_pack_run_id,
                    verdict, compared_cases, pass_rate_delta, average_score_delta,
                    context_recall_delta, validation_violation_delta, scope_creep_delta,
                    execution_failure_delta, report_json, source_type, source_id, created_at
                 ) VALUES (
                    'cser_release_regressed', ?1, ?2, 'workflow_policy', 'before',
                    'after', NULL, 'cepr_release_regressed', 'regressed', 2, -0.5,
                    -0.25, -0.1, 1, 2, 1, '{}', 'strategy_effect',
                    'workflow_policy', ?3
                 )",
                params![session.id, project_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO coding_eval_runs (
                    id, session_id, project_id, suite, name, status,
                    metrics_json, source_type, source_id, created_at
                 ) VALUES (
                    'cer_release_missing_tool', ?1, ?2, 'task_level_coding_eval',
                    'agent tool calls', 'failed', ?3, 'coding_task_eval',
                    'agent-tool-calls', ?4
                 )",
                params![
                    session.id,
                    project_id,
                    json!({"metrics":{"executionMode":"agent","agentExecution":{"toolCalls":[]}}})
                        .to_string(),
                    now
                ],
            )
            .unwrap();
        }

        let report = db
            .evaluate_coding_eval_release_gate(CodingEvalReleaseGateInput {
                session_id: Some(session.id),
                min_strategy_effect_runs: Some(1),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.status, "failed");
        assert_eq!(report.summary.regressed_strategy_effects, 1);
        assert_eq!(report.summary.validation_violation_delta, 1);
        assert_eq!(report.summary.scope_creep_delta, 2);
        assert_eq!(report.summary.missing_tool_call_runs, 1);
        for name in [
            "strategy_regressions",
            "missing_tool_calls",
            "validation_violation_delta",
            "scope_creep_delta",
        ] {
            assert!(report
                .checks
                .iter()
                .any(|check| check.name == name && check.status == "failed"));
        }
    }

    #[test]
    fn release_gate_requires_external_model_when_configured() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let now = now_rfc3339();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO coding_eval_pack_runs (
                    id, session_id, project_id, pack_id, source_doc, label,
                    baseline_kind, status, selected_cases, automated_cases,
                    skipped_cases, passed_cases, failed_cases, total_checks,
                    report_json, source_type, source_id, created_at
                 ) VALUES (
                    'cepr_release_deterministic_only', ?1, NULL,
                    'phase5-gold-task-pack', 'docs/roadmap/coding-eval.md',
                    'deterministic only', 'deterministic_mock', 'passed',
                    1, 1, 0, 1, 0, 4, '{}', 'gold_task_pack',
                    'phase5-gold-task-pack', ?2
                 )",
                params![session.id, now],
            )
            .unwrap();
        }

        let report = db
            .evaluate_coding_eval_release_gate(CodingEvalReleaseGateInput {
                session_id: Some(session.id),
                require_external_model_pack: true,
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.status, "insufficient_data");
        assert_eq!(report.summary.external_model_pack_runs, 0);
        assert!(report.checks.iter().any(|check| {
            check.name == "external_model_baseline" && check.status == "insufficient_data"
        }));
    }

    #[test]
    fn learning_generalization_passes_two_clean_projects() {
        let (_dir, db) = test_db();
        let source_id = "validation_failed";
        let session_a = db
            .create_session_with_project(
                crate::agent_loader::DEFAULT_AGENT_ID,
                Some("project-generalization-a"),
                None,
            )
            .unwrap();
        let session_b = db
            .create_session_with_project(
                crate::agent_loader::DEFAULT_AGENT_ID,
                Some("project-generalization-b"),
                None,
            )
            .unwrap();
        insert_promoted_learning(
            &db,
            &session_a.id,
            "project-generalization-a",
            "cip_generalization_a",
            source_id,
        );
        insert_promoted_learning(
            &db,
            &session_b.id,
            "project-generalization-b",
            "cip_generalization_b",
            source_id,
        );
        insert_generalization_pack(
            &db,
            &session_a.id,
            "project-generalization-a",
            "cepr_gen_a",
            "passed",
        );
        insert_generalization_pack(
            &db,
            &session_b.id,
            "project-generalization-b",
            "cepr_gen_b",
            "passed",
        );
        insert_generalization_strategy_effect(
            &db,
            &session_a.id,
            "project-generalization-a",
            "cser_gen_a",
            source_id,
            "improved",
        );
        insert_generalization_strategy_effect(
            &db,
            &session_b.id,
            "project-generalization-b",
            "cser_gen_b",
            source_id,
            "improved",
        );

        let report = db
            .evaluate_coding_learning_generalization(CodingLearningGeneralizationInput {
                source_type: Some("failure_feedback".to_string()),
                source_id: Some(source_id.to_string()),
                min_strategy_effect_runs_per_project: Some(1),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.status, "passed");
        assert_eq!(report.scope, "global");
        assert_eq!(report.summary.projects_evaluated, 2);
        assert_eq!(report.summary.passed_projects, 2);
        assert_eq!(report.summary.total_promoted_learning, 2);
        assert_eq!(report.summary.total_strategy_effect_runs, 2);
        assert!(report.checks.iter().all(|check| check.status == "passed"));
    }

    #[test]
    fn learning_generalization_fails_regressed_project() {
        let (_dir, db) = test_db();
        let source_id = "review_blocker";
        let session_a = db
            .create_session_with_project(
                crate::agent_loader::DEFAULT_AGENT_ID,
                Some("project-generalization-pass"),
                None,
            )
            .unwrap();
        let session_b = db
            .create_session_with_project(
                crate::agent_loader::DEFAULT_AGENT_ID,
                Some("project-generalization-regress"),
                None,
            )
            .unwrap();
        for (session, project, proposal, pack, strategy, verdict) in [
            (
                &session_a.id,
                "project-generalization-pass",
                "cip_generalization_pass",
                "cepr_gen_pass",
                "cser_gen_pass",
                "improved",
            ),
            (
                &session_b.id,
                "project-generalization-regress",
                "cip_generalization_regress",
                "cepr_gen_regress",
                "cser_gen_regress",
                "regressed",
            ),
        ] {
            insert_promoted_learning(&db, session, project, proposal, source_id);
            insert_generalization_pack(&db, session, project, pack, "passed");
            insert_generalization_strategy_effect(
                &db, session, project, strategy, source_id, verdict,
            );
        }

        let report = db
            .evaluate_coding_learning_generalization(CodingLearningGeneralizationInput {
                source_type: Some("failure_feedback".to_string()),
                source_id: Some(source_id.to_string()),
                min_strategy_effect_runs_per_project: Some(1),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.status, "failed");
        assert_eq!(report.summary.failed_projects, 1);
        assert_eq!(report.summary.regressed_projects, 1);
        assert!(report.projects.iter().any(|project| {
            project.project_id == "project-generalization-regress"
                && project.status == "failed"
                && project
                    .reasons
                    .iter()
                    .any(|reason| reason.contains("regressed"))
        }));
        assert!(report.checks.iter().any(|check| {
            check.name == "strategy_regression_projects" && check.status == "failed"
        }));
    }

    #[test]
    fn benchmark_center_passes_clean_deterministic_history() {
        let (_dir, db) = test_db();
        let project_id = "project-benchmark-clean";
        let session = db
            .create_session_with_project(
                crate::agent_loader::DEFAULT_AGENT_ID,
                Some(project_id),
                None,
            )
            .unwrap();
        insert_generalization_pack(
            &db,
            &session.id,
            project_id,
            "cepr_benchmark_clean",
            "passed",
        );

        let report = db
            .get_coding_benchmark_center(CodingBenchmarkCenterInput {
                session_id: Some(session.id),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.status, "passed");
        assert_eq!(report.scope, "project");
        assert_eq!(report.summary.total_runs, 1);
        assert_eq!(report.summary.passed_runs, 1);
        assert_eq!(report.summary.run_pass_rate, Some(1.0));
        assert_eq!(report.summary.case_pass_rate, Some(1.0));
        assert_eq!(report.summary.latest_run_status.as_deref(), Some("passed"));
        assert_eq!(report.release_gate.status, "passed");
        assert_eq!(report.runs.len(), 1);
        assert!(report
            .baselines
            .iter()
            .any(|baseline| baseline.baseline_kind == "deterministic_mock" && baseline.runs == 1));
        assert!(report.checks.iter().any(|check| {
            check.name == "external_model_baseline"
                && check.status == "insufficient_data"
                && check.severity == "advisory"
        }));
    }

    #[test]
    fn benchmark_center_fails_latest_failed_pack_run() {
        let (_dir, db) = test_db();
        let project_id = "project-benchmark-failed";
        let session = db
            .create_session_with_project(
                crate::agent_loader::DEFAULT_AGENT_ID,
                Some(project_id),
                None,
            )
            .unwrap();
        insert_generalization_pack(
            &db,
            &session.id,
            project_id,
            "cepr_benchmark_failed",
            "failed",
        );

        let report = db
            .get_coding_benchmark_center(CodingBenchmarkCenterInput {
                session_id: Some(session.id),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.status, "failed");
        assert_eq!(report.summary.failed_runs, 1);
        assert_eq!(report.summary.latest_run_status.as_deref(), Some("failed"));
        assert_eq!(report.release_gate.status, "failed");
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "latest_pack_run" && check.status == "failed"));
    }

    #[test]
    fn benchmark_center_requires_external_model_when_configured() {
        let (_dir, db) = test_db();
        let project_id = "project-benchmark-external-required";
        let session = db
            .create_session_with_project(
                crate::agent_loader::DEFAULT_AGENT_ID,
                Some(project_id),
                None,
            )
            .unwrap();
        insert_generalization_pack(
            &db,
            &session.id,
            project_id,
            "cepr_benchmark_external_required",
            "passed",
        );

        let report = db
            .get_coding_benchmark_center(CodingBenchmarkCenterInput {
                session_id: Some(session.id),
                require_external_model_baseline: true,
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.status, "insufficient_data");
        assert_eq!(report.summary.external_model_runs, 0);
        assert_eq!(report.release_gate.status, "insufficient_data");
        assert!(report.checks.iter().any(|check| {
            check.name == "external_model_baseline"
                && check.status == "insufficient_data"
                && check.severity == "required"
        }));
    }

    #[test]
    fn benchmark_corpus_imports_versions_and_health_after_activation() {
        let (_dir, db) = test_db();
        let pack = db
            .import_benchmark_task_pack(CodingBenchmarkTaskPackImportInput {
                manifest: sample_task_pack_manifest("draft", "v1"),
                explicit_import_consent: true,
                imported_from: Some("unit-test".to_string()),
            })
            .unwrap();

        assert_eq!(pack.status, "draft");
        assert_eq!(pack.tasks.len(), 2);
        assert!(db
            .import_benchmark_task_pack(CodingBenchmarkTaskPackImportInput {
                manifest: sample_task_pack_manifest("draft", "v1"),
                explicit_import_consent: true,
                imported_from: Some("unit-test".to_string()),
            })
            .is_err());

        let health_before = db
            .get_benchmark_corpus_health(CodingBenchmarkCorpusHealthInput::default())
            .unwrap();
        assert_eq!(health_before.status, "insufficient_data");
        assert_eq!(health_before.active_tasks, 0);
        assert_eq!(health_before.draft_tasks, 2);

        let validation = db
            .validate_benchmark_task_pack(CodingBenchmarkTaskPackValidateInput {
                pack_id: pack.pack_id.clone(),
                version: pack.version.clone(),
            })
            .unwrap();
        assert_eq!(validation.status, "passed");

        let active = db
            .update_benchmark_task_pack_status(CodingBenchmarkTaskPackStatusInput {
                pack_id: pack.pack_id,
                version: pack.version,
                status: "active".to_string(),
            })
            .unwrap();
        assert_eq!(active.status, "active");

        let health_after = db
            .get_benchmark_corpus_health(CodingBenchmarkCorpusHealthInput::default())
            .unwrap();
        assert_eq!(health_after.status, "passed");
        assert_eq!(health_after.active_packs, 1);
        assert_eq!(health_after.active_tasks, 2);
        assert!(health_after
            .by_task_type
            .iter()
            .any(|bucket| bucket.key == "bugfix" && bucket.count == 1));
    }

    #[test]
    fn benchmark_corpus_rejects_implicit_import_and_bad_active_tasks() {
        let (_dir, db) = test_db();
        assert!(db
            .import_benchmark_task_pack(CodingBenchmarkTaskPackImportInput {
                manifest: sample_task_pack_manifest("draft", "v1"),
                explicit_import_consent: false,
                imported_from: Some("unit-test".to_string()),
            })
            .is_err());

        let mut bad = sample_task_pack_manifest("active", "v2");
        bad.tasks[0].validation_commands.clear();
        bad.tasks[0].success_criteria.truncate(1);
        let err = db
            .import_benchmark_task_pack(CodingBenchmarkTaskPackImportInput {
                manifest: bad,
                explicit_import_consent: true,
                imported_from: Some("unit-test".to_string()),
            })
            .unwrap_err()
            .to_string();
        assert!(err.contains("active_task_quality") || err.contains("fixture_gaming_risk"));
    }

    #[test]
    fn benchmark_report_exports_release_snapshot_and_marks_evidence() {
        let (dir, db) = test_db();
        let project_id = "project-benchmark-report";
        let session = db
            .create_session_with_project(
                crate::agent_loader::DEFAULT_AGENT_ID,
                Some(project_id),
                None,
            )
            .unwrap();
        insert_generalization_pack(
            &db,
            &session.id,
            project_id,
            "cepr_benchmark_report",
            "passed",
        );
        db.import_benchmark_task_pack(CodingBenchmarkTaskPackImportInput {
            manifest: sample_task_pack_manifest("active", "v-report"),
            explicit_import_consent: true,
            imported_from: Some("unit-test".to_string()),
        })
        .unwrap();

        let output_dir = dir.path().join("benchmark-reports");
        let report = db
            .generate_benchmark_report(CodingBenchmarkReportGenerateInput {
                report_type: "release".to_string(),
                session_id: Some(session.id.clone()),
                output_dir: Some(output_dir.to_string_lossy().into_owned()),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.report_type, "release");
        assert_eq!(report.status, "passed");
        assert_eq!(report.project_id.as_deref(), Some(project_id));
        assert!(report.release_evidence);
        assert!(report.marked_release_at.is_some());
        assert!(report.snapshot.get("benchmarkCenter").is_some());
        assert!(report.snapshot.get("releaseGate").is_some());
        assert!(report.snapshot.get("leaderboard").is_some());
        assert!(report.snapshot.get("corpusHealth").is_some());
        assert!(std::path::Path::new(&report.markdown_path).exists());
        assert!(std::path::Path::new(&report.json_path).exists());
        assert!(std::path::Path::new(&report.html_path).exists());

        let markdown = std::fs::read_to_string(&report.markdown_path).unwrap();
        assert!(markdown.contains("## Executive Summary"));
        assert!(markdown.contains(&report.id));

        let listed = db
            .list_benchmark_reports(CodingBenchmarkReportListInput {
                session_id: Some(session.id),
                release_evidence_only: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, report.id);

        let unmarked = db
            .mark_benchmark_report_release_evidence(CodingBenchmarkReportMarkInput {
                report_id: report.id.clone(),
                release_evidence: false,
            })
            .unwrap();
        assert!(!unmarked.release_evidence);
        assert!(unmarked.marked_release_at.is_none());

        let fetched = db.get_benchmark_report(&report.id).unwrap().unwrap();
        assert_eq!(fetched.id, report.id);
        assert!(!fetched.release_evidence);
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    fn contract_db() -> (tempfile::TempDir, SessionDB) {
        let dir = tempfile::tempdir().unwrap();
        let db = SessionDB::open_ephemeral_for_test(&dir.path().join("sessions.db")).unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS channel_conversations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                chat_id TEXT NOT NULL,
                thread_id TEXT,
                session_id TEXT NOT NULL,
                sender_id TEXT,
                sender_name TEXT,
                chat_type TEXT NOT NULL DEFAULT 'dm',
                source TEXT NOT NULL DEFAULT 'inbound',
                attached_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );",
        )
        .unwrap();
        drop(conn);
        (dir, db)
    }

    fn contract_task_pack(status: &str) -> CodingBenchmarkTaskPackManifest {
        CodingBenchmarkTaskPackManifest {
            pack_id: "contract-pack".to_string(),
            version: "v1".to_string(),
            name: "Contract pack".to_string(),
            description: None,
            status: Some(status.to_string()),
            source_kind: "fixture_repo".to_string(),
            source_uri: Some("local://contract-pack".to_string()),
            repo_template: Some("fixture://contract-repo".to_string()),
            license_note: "Synthetic fixture".to_string(),
            privacy_note: "No private content".to_string(),
            redaction_status: "not_required".to_string(),
            tasks: vec![CodingBenchmarkTaskPackTaskManifest {
                task_id: "CONTRACT-001".to_string(),
                version: "v1".to_string(),
                title: "Protect active task validation".to_string(),
                status: Some("active".to_string()),
                task_type: "bugfix".to_string(),
                difficulty: "medium".to_string(),
                language: Some("rust".to_string()),
                framework: Some("ha-core".to_string()),
                source_uri: Some("local://contract-pack/001".to_string()),
                repo_template: Some("fixture://contract-repo".to_string()),
                tags: vec!["contract".to_string()],
                success_criteria: vec![
                    "The behavior is corrected.".to_string(),
                    "A focused regression remains.".to_string(),
                ],
                validation_commands: vec!["cargo check -p ha-core --locked".to_string()],
                allowed_paths: vec!["crates/ha-core/**".to_string()],
                forbidden_paths: vec!["src/**".to_string()],
                calibration_notes: vec!["Reviewed deterministic fixture".to_string()],
                calibrated_at: Some(now_rfc3339()),
                license_note: Some("Synthetic fixture".to_string()),
                privacy_note: Some("No private content".to_string()),
                redaction_status: Some("not_required".to_string()),
            }],
        }
    }

    #[test]
    fn benchmark_report_type_and_trigger_kind_fail_closed() {
        assert_eq!(
            normalize_benchmark_report_type("campaign").unwrap(),
            "campaign"
        );
        assert!(normalize_benchmark_report_type("external_model").is_err());
        assert_eq!(
            normalize_benchmark_trigger_kind(Some("pre_release")).unwrap(),
            "pre_release"
        );
        assert!(normalize_benchmark_trigger_kind(Some("provider")).is_err());
    }

    #[test]
    fn infrastructure_failures_are_not_scored_as_model_regressions() {
        assert_eq!(
            classify_benchmark_item_failure("failed", Some("Provider config was not supplied")),
            Some("provider_error".to_string())
        );
        assert_eq!(classify_benchmark_item_failure("passed", None), None);
    }

    #[test]
    fn benchmark_corpus_requires_consent_and_active_task_quality() {
        let (_dir, db) = contract_db();
        assert!(db
            .import_benchmark_task_pack(CodingBenchmarkTaskPackImportInput {
                manifest: contract_task_pack("draft"),
                explicit_import_consent: false,
                imported_from: Some("contract-test".to_string()),
            })
            .is_err());

        let mut invalid = contract_task_pack("active");
        invalid.tasks[0].validation_commands.clear();
        invalid.tasks[0].success_criteria.truncate(1);
        assert!(db
            .import_benchmark_task_pack(CodingBenchmarkTaskPackImportInput {
                manifest: invalid,
                explicit_import_consent: true,
                imported_from: Some("contract-test".to_string()),
            })
            .is_err());
    }

    #[test]
    fn release_gate_without_evidence_fails_closed() {
        let (_dir, db) = contract_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();

        let report = db
            .evaluate_coding_eval_release_gate(CodingEvalReleaseGateInput {
                session_id: Some(session.id),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.status, "insufficient_data");
        assert_eq!(report.summary.pack_runs, 0);
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "pack_run_sample" && check.status == "insufficient_data"));
    }
}
