//! General-domain eval and quality gate control plane.
//!
//! Coding eval remains coding-shaped and benchmark-oriented. This module keeps
//! non-coding eval separate: built-in domain tasks, deterministic trace scoring,
//! durable domain eval run history, and a domain quality gate that reads domain
//! eval + domain quality evidence without mixing it into coding benchmark score.

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::agent_loader::DEFAULT_AGENT_ID;
use crate::chat_engine::{self, ChatEngineParams, ChatSource, NoopEventSink};
use crate::context_compact::CompactConfig;
use crate::domain_quality::{
    DomainQualityCheckStatus, DomainQualityRunSnapshot, DomainQualityRunState,
    RunDomainQualityInput,
};
use crate::domain_workflow::{ListDomainEvidenceInput, RecordDomainEvidenceInput};
use crate::provider::{ActiveModel, ProviderConfig};
use crate::session::{MessageRole, NewMessage, SessionDB, SessionKind};
use crate::util::now_rfc3339;
use crate::workflow::CreateWorkflowRunInput;
use crate::workflow_mode::WorkflowMode;

const DEFAULT_WINDOW_DAYS: u32 = 30;
const MAX_WINDOW_DAYS: u32 = 180;
const DEFAULT_DOMAIN_EVAL_LIMIT: usize = 20;
const MAX_DOMAIN_EVAL_LIMIT: usize = 100;
const DEFAULT_DOMAIN_EVAL_CAMPAIGN_LIMIT: usize = 12;
const MAX_DOMAIN_EVAL_CAMPAIGN_LIMIT: usize = 50;
const DEFAULT_DOMAIN_EVAL_CAMPAIGN_TASKS: usize = 5;
const MAX_DOMAIN_EVAL_CAMPAIGN_TASKS: usize = 15;
const MAX_DOMAIN_EVAL_CAMPAIGN_MODELS: usize = 8;
const DEFAULT_MIN_EVAL_RUNS: usize = 1;
const DEFAULT_MIN_PASS_RATE: f64 = 1.0;
const DEFAULT_MIN_AVERAGE_SCORE: f64 = 0.8;
const DEFAULT_MIN_QUALITY_RUNS: usize = 1;
const DEFAULT_MAX_BLOCKED_QUALITY_RUNS: usize = 0;
const DEFAULT_MIN_DOMAIN_COVERAGE: usize = 1;
const DEFAULT_DOMAIN_READINESS_MIN_CAMPAIGN_ITEMS: usize = 1;
const DEFAULT_DOMAIN_READINESS_MIN_LEADERBOARD_ROWS: usize = 1;
const DEFAULT_DOMAIN_READINESS_MAX_FAILED_CAMPAIGN_ITEMS: usize = 0;
const DEFAULT_DOMAIN_READINESS_MAX_OPEN_LEARNING_PROPOSALS: usize = 0;
const DEFAULT_DOMAIN_OPERATIONAL_MIN_WORKFLOW_RUNS: usize = 1;
const DEFAULT_DOMAIN_OPERATIONAL_MAX_FAILED_WORKFLOW_RUNS: usize = 0;
const DEFAULT_DOMAIN_OPERATIONAL_MAX_BLOCKED_WORKFLOW_RUNS: usize = 0;
const DEFAULT_DOMAIN_OPERATIONAL_MAX_CANCELLED_WORKFLOW_RUNS: usize = 0;
const DEFAULT_DOMAIN_OPERATIONAL_MAX_ACTIVE_WORKFLOW_RUNS: usize = 0;
const DEFAULT_DOMAIN_OPERATIONAL_MIN_LOOP_RUNS: usize = 0;
const DEFAULT_DOMAIN_OPERATIONAL_MAX_FAILED_LOOP_RUNS: usize = 0;
const DEFAULT_DOMAIN_OPERATIONAL_MAX_ACTIVE_CAMPAIGNS: usize = 0;
const DEFAULT_DOMAIN_OPERATIONAL_MAX_FAILED_CAMPAIGN_ITEMS: usize = 0;
const DEFAULT_DOMAIN_SOAK_WINDOW_DAYS: u32 = 7;
const DEFAULT_DOMAIN_SOAK_MAX_ITEMS: usize = 12;
const MAX_DOMAIN_SOAK_MAX_ITEMS: usize = 50;
const DOMAIN_EVAL_SOURCE_LIVE: &str = "live";
const DOMAIN_EVAL_SOURCE_FIXTURE_TRACE: &str = "fixture_trace";
const DOMAIN_EVAL_SOURCE_FIXTURE_AGENT: &str = "fixture_agent";
const DOMAIN_EVAL_SOURCE_FIXTURE_UNSUPPORTED: &str = "fixture_unsupported";
const DOMAIN_EVAL_SOURCE_CAMPAIGN: &str = "fixture_campaign";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalTask {
    pub id: String,
    pub version: String,
    pub domain: String,
    pub title: String,
    pub task_type: String,
    pub input: DomainEvalTaskInput,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub required_evidence: Vec<DomainEvalEvidenceRequirement>,
    #[serde(default)]
    pub success_criteria: Vec<String>,
    #[serde(default)]
    pub prohibited_actions: Vec<String>,
    #[serde(default)]
    pub calibration: Vec<DomainEvalCalibrationRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalTaskInput {
    pub prompt: String,
    pub fixture_kind: String,
    #[serde(default)]
    pub source_requirements: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalEvidenceRequirement {
    pub evidence_type: String,
    pub title: String,
    pub required: bool,
    pub min_count: usize,
    #[serde(default)]
    pub metadata_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalCalibrationRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_run_id: Option<String>,
    pub calibrated_at: String,
    pub reviewer: String,
    pub note: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDomainEvalTasksInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordDomainEvalCalibrationInput {
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<String>,
    pub verdict: String,
    pub note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_run_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDomainEvalCalibrationsInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default)]
    pub include_user_scope: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDomainEvalCaseInput {
    pub proposal_id: String,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDomainEvalCaseResult {
    pub imported: bool,
    pub task: DomainEvalTask,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub source_path: String,
    pub imported_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDomainEvalTaskInput {
    pub session_id: String,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_quality_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDomainEvalFixtureInput {
    pub fixture: DomainEvalFixture,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalFixture {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default = "default_domain_eval_fixture_execution_mode")]
    pub execution_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default)]
    pub goal: DomainEvalFixtureGoal,
    #[serde(default)]
    pub evidence: Vec<DomainEvalFixtureEvidence>,
    #[serde(default)]
    pub workflow: Option<DomainEvalFixtureWorkflow>,
    #[serde(default)]
    pub quality: Option<DomainEvalFixtureQuality>,
    #[serde(default)]
    pub execution: DomainEvalFixtureExecution,
    #[serde(default)]
    pub checks: DomainEvalFixtureChecks,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalFixtureGoal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_criteria: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_template_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_task_type: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalFixtureEvidence {
    pub evidence_type: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub source_metadata: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalFixtureWorkflow {
    #[serde(default = "default_domain_workflow_kind")]
    pub kind: String,
    #[serde(default = "default_domain_workflow_script")]
    pub script_source: String,
    #[serde(default = "default_execution_mode")]
    pub execution_mode: String,
}

impl Default for DomainEvalFixtureWorkflow {
    fn default() -> Self {
        Self {
            kind: default_domain_workflow_kind(),
            script_source: default_domain_workflow_script(),
            execution_mode: default_execution_mode(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalFixtureQuality {
    #[serde(default = "default_true")]
    pub run: bool,
    #[serde(default)]
    pub source_metadata: Value,
    #[serde(default)]
    pub explicit_user_approval: bool,
}

impl Default for DomainEvalFixtureQuality {
    fn default() -> Self {
        Self {
            run: true,
            source_metadata: Value::Null,
            explicit_user_approval: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalFixtureExecution {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_text: Option<String>,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub model_chain: Vec<ActiveModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_config: Option<CompactConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_system_context: Option<String>,
    #[serde(default)]
    pub denied_tools: Vec<String>,
    #[serde(default)]
    pub auto_approve_tools: bool,
    #[serde(default = "default_agent_fixture_workflow_mode")]
    pub workflow_mode: String,
}

impl Default for DomainEvalFixtureExecution {
    fn default() -> Self {
        Self {
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
            workflow_mode: default_agent_fixture_workflow_mode(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalFixtureChecks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_score: Option<f64>,
    #[serde(default)]
    pub expected_passed_checks: Vec<String>,
    #[serde(default)]
    pub expected_failed_checks: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_execution_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_turn: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_tool_calls: Option<usize>,
    #[serde(default)]
    pub expected_tool_calls: Vec<String>,
    #[serde(default)]
    pub response_contains: Vec<String>,
    #[serde(default)]
    pub error_contains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalFixtureReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixture_run_id: Option<String>,
    pub name: String,
    pub execution_mode: String,
    pub source_type: String,
    pub status: String,
    pub passed: bool,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_run: Option<DomainEvalRunRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution: Option<DomainEvalFixtureExecutionReport>,
    #[serde(default)]
    pub checks: Vec<DomainEvalFixtureCheck>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalFixtureExecutionReport {
    pub mode: String,
    pub status: String,
    pub prompt: String,
    pub agent_id: String,
    pub workflow_mode: String,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalFixtureCheck {
    pub name: String,
    pub status: String,
    pub expected: String,
    pub actual: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDomainEvalFixtureRunsInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalFixtureRunRecord {
    pub id: String,
    pub name: String,
    pub execution_mode: String,
    pub source_type: String,
    pub status: String,
    pub passed: bool,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_run_id: Option<String>,
    pub report: DomainEvalFixtureReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalCampaignModel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// Owner-plane request reference; stripped before campaign persistence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_profile_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDomainEvalCampaignInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default)]
    pub task_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tasks: Option<usize>,
    #[serde(default)]
    pub models: Vec<DomainEvalCampaignModel>,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
    #[serde(default)]
    pub run_now: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_budget_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDomainEvalCampaignsInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalCampaignLeaderboardInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default)]
    pub campaign_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDomainEvalCampaignInput {
    pub campaign_id: String,
    /// Deprecated one-cycle compatibility field.
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub retry_failed_only: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalCampaignSummary {
    pub total_items: usize,
    pub queued_items: usize,
    pub running_items: usize,
    pub passed_items: usize,
    pub failed_items: usize,
    pub cancelled_items: usize,
    pub interrupted_items: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_pass_rate: Option<f64>,
    pub eval_runs: usize,
    pub passed_eval_runs: usize,
    pub failed_eval_runs: usize,
    pub insufficient_eval_runs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_score: Option<f64>,
    pub total_checks: usize,
    pub passed_checks: usize,
    pub failed_checks: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalCampaignItem {
    pub id: String,
    pub campaign_id: String,
    pub task_id: String,
    pub task_title: String,
    pub domain: String,
    pub execution_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub status: String,
    pub attempt: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixture_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    pub total_checks: usize,
    pub passed_checks: usize,
    pub failed_checks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalCampaign {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    pub task_filter: Value,
    pub model_matrix: Vec<DomainEvalCampaignModel>,
    pub execution_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_budget_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    pub summary: DomainEvalCampaignSummary,
    pub items: Vec<DomainEvalCampaignItem>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalCampaignLeaderboardEvidence {
    pub campaign_id: String,
    pub campaign_name: String,
    pub item_id: String,
    pub task_id: String,
    pub domain: String,
    pub execution_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalCampaignLeaderboardRow {
    pub rank: usize,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub execution_mode: String,
    pub campaigns: usize,
    pub items: usize,
    pub passed_items: usize,
    pub failed_items: usize,
    pub cancelled_items: usize,
    pub interrupted_items: usize,
    pub attempts: usize,
    pub eval_runs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_pass_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_score: Option<f64>,
    pub total_checks: usize,
    pub failed_checks: usize,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<DomainEvalCampaignLeaderboardEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalCampaignLeaderboardReport {
    pub generated_at: String,
    pub status: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    pub window_days: u32,
    pub rows: Vec<DomainEvalCampaignLeaderboardRow>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDomainEvalRunsInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(default)]
    pub include_synthetic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalRunRecord {
    pub id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub task_id: String,
    pub task_version: String,
    pub domain: String,
    pub label: String,
    pub status: String,
    pub score: f64,
    pub source_type: String,
    pub report: DomainEvalReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_quality_run_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalReport {
    pub task: DomainEvalTask,
    pub status: String,
    pub score: f64,
    pub summary: DomainEvalSummary,
    #[serde(default)]
    pub checks: Vec<DomainEvalCheck>,
    pub evidence: Value,
    pub goal: Value,
    pub quality: Value,
    pub workflow: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalSummary {
    pub required_evidence: usize,
    pub satisfied_required_evidence: usize,
    pub missing_required_evidence: usize,
    pub total_evidence: usize,
    pub source_count: usize,
    pub dated_source_count: usize,
    pub data_quality_count: usize,
    pub user_decision_count: usize,
    pub workflow_runs: usize,
    pub quality_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvalCheck {
    pub name: String,
    pub category: String,
    pub status: String,
    pub weight: f64,
    pub score: f64,
    pub expected: String,
    pub actual: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainQualityGateInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_eval_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_pass_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_average_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_quality_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_blocked_quality_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_domain_coverage: Option<usize>,
    #[serde(default)]
    pub require_approval_safety: bool,
    #[serde(default)]
    pub include_synthetic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainQualityGateThresholds {
    pub min_eval_runs: usize,
    pub min_pass_rate: f64,
    pub min_average_score: f64,
    pub min_quality_runs: usize,
    pub max_blocked_quality_runs: usize,
    pub min_domain_coverage: usize,
    pub require_approval_safety: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainQualityGateSummary {
    pub eval_runs: usize,
    pub passed_eval_runs: usize,
    pub failed_eval_runs: usize,
    pub insufficient_eval_runs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pass_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_score: Option<f64>,
    pub quality_runs: usize,
    pub completed_quality_runs: usize,
    pub blocked_quality_runs: usize,
    pub failed_quality_runs: usize,
    pub needs_user_quality_runs: usize,
    pub approval_blockers: usize,
    pub domains_covered: usize,
    pub evidence_items: usize,
    pub source_cited: usize,
    pub dated_sources: usize,
    pub data_quality_checked: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainQualityGateCheck {
    pub name: String,
    pub status: String,
    pub severity: String,
    pub expected: String,
    pub actual: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainQualityGateReport {
    pub generated_at: String,
    pub status: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    pub window_days: u32,
    pub since: String,
    pub thresholds: DomainQualityGateThresholds,
    pub summary: DomainQualityGateSummary,
    #[serde(default)]
    pub checks: Vec<DomainQualityGateCheck>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainReadinessGateInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_eval_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_pass_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_average_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_quality_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_blocked_quality_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_domain_coverage: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_campaign_items: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_leaderboard_rows: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_failed_campaign_items: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_open_learning_proposals: Option<usize>,
    #[serde(default)]
    pub require_approval_safety: bool,
    #[serde(default)]
    pub include_synthetic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainReadinessGateThresholds {
    pub window_days: u32,
    pub min_eval_runs: usize,
    pub min_pass_rate: f64,
    pub min_average_score: f64,
    pub min_quality_runs: usize,
    pub max_blocked_quality_runs: usize,
    pub min_domain_coverage: usize,
    pub min_campaign_items: usize,
    pub min_leaderboard_rows: usize,
    pub max_failed_campaign_items: usize,
    pub max_open_learning_proposals: usize,
    pub require_approval_safety: bool,
    pub include_synthetic: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainReadinessGateSummary {
    pub eval_runs: usize,
    pub quality_runs: usize,
    pub campaigns: usize,
    pub active_campaigns: usize,
    pub terminal_campaigns: usize,
    pub campaign_items: usize,
    pub terminal_campaign_items: usize,
    pub passed_campaign_items: usize,
    pub failed_campaign_items: usize,
    pub cancelled_campaign_items: usize,
    pub interrupted_campaign_items: usize,
    pub leaderboard_rows: usize,
    pub open_learning_proposals: usize,
    pub pending_learning_campaigns: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_campaign_at: Option<String>,
    pub quality_status: String,
    pub leaderboard_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainReadinessGateCheck {
    pub name: String,
    pub status: String,
    pub severity: String,
    pub expected: String,
    pub actual: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainReadinessGateReport {
    pub generated_at: String,
    pub status: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    pub since: String,
    pub thresholds: DomainReadinessGateThresholds,
    pub summary: DomainReadinessGateSummary,
    #[serde(default)]
    pub checks: Vec<DomainReadinessGateCheck>,
    pub quality_gate: DomainQualityGateReport,
    pub campaign_leaderboard: DomainEvalCampaignLeaderboardReport,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub recommended_next_steps: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainOperationalGateInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_workflow_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_failed_workflow_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_blocked_workflow_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cancelled_workflow_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_active_workflow_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_loop_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_failed_loop_runs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_active_campaigns: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_failed_campaign_items: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainOperationalGateThresholds {
    pub window_days: u32,
    pub min_workflow_runs: usize,
    pub max_failed_workflow_runs: usize,
    pub max_blocked_workflow_runs: usize,
    pub max_cancelled_workflow_runs: usize,
    pub max_active_workflow_runs: usize,
    pub min_loop_runs: usize,
    pub max_failed_loop_runs: usize,
    pub max_active_campaigns: usize,
    pub max_failed_campaign_items: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainOperationalGateSummary {
    pub workflow_runs: usize,
    pub completed_workflow_runs: usize,
    pub failed_workflow_runs: usize,
    pub blocked_workflow_runs: usize,
    pub cancelled_workflow_runs: usize,
    pub active_workflow_runs: usize,
    pub paused_workflow_runs: usize,
    pub awaiting_approval_workflow_runs: usize,
    pub loop_schedules: usize,
    pub active_loop_schedules: usize,
    pub loop_runs: usize,
    pub succeeded_loop_runs: usize,
    pub failed_loop_runs: usize,
    pub active_loop_runs: usize,
    pub campaigns: usize,
    pub active_campaigns: usize,
    pub campaign_items: usize,
    pub passed_campaign_items: usize,
    pub failed_campaign_items: usize,
    pub cancelled_campaign_items: usize,
    pub interrupted_campaign_items: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_activity_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_active_work_age_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainOperationalGateCheck {
    pub name: String,
    pub status: String,
    pub severity: String,
    pub expected: String,
    pub actual: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainOperationalGateReport {
    pub generated_at: String,
    pub status: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    pub since: String,
    pub thresholds: DomainOperationalGateThresholds,
    pub summary: DomainOperationalGateSummary,
    #[serde(default)]
    pub checks: Vec<DomainOperationalGateCheck>,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub recommended_next_steps: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainSoakReportInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainSoakReportSummary {
    pub workflow_runs: usize,
    pub completed_workflow_runs: usize,
    pub failed_workflow_runs: usize,
    pub blocked_workflow_runs: usize,
    pub cancelled_workflow_runs: usize,
    pub active_workflow_runs: usize,
    pub awaiting_approval_workflow_runs: usize,
    pub repair_workflow_runs: usize,
    pub approval_events: usize,
    pub approval_request_events: usize,
    pub approval_decision_events: usize,
    pub open_approval_waits: usize,
    pub pause_events: usize,
    pub resume_events: usize,
    pub cancel_events: usize,
    pub recovery_events: usize,
    pub workflow_control_intervention_events: usize,
    pub workflow_budget_usage_events: usize,
    pub workflow_budget_exhausted_events: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_workflow_output_tokens_spent: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_workflow_output_token_budget: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_approval_wait_secs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_approval_wait_secs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_open_approval_wait_secs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_workflow_drain_secs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_workflow_drain_secs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_activity_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_activity_age_secs: Option<i64>,
    pub sample_days: usize,
    pub required_sample_days: usize,
    pub loop_runs: usize,
    pub succeeded_loop_runs: usize,
    pub failed_loop_runs: usize,
    pub active_loop_runs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_loop_duration_secs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_loop_duration_secs: Option<i64>,
    pub campaigns: usize,
    pub active_campaigns: usize,
    pub campaign_items: usize,
    pub passed_campaign_items: usize,
    pub failed_campaign_items: usize,
    pub cancelled_campaign_items: usize,
    pub interrupted_campaign_items: usize,
    pub retried_campaign_items: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_campaign_item_duration_secs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_campaign_item_duration_secs: Option<i64>,
    pub connector_e2e_evidence: usize,
    pub connector_execution_evidence: usize,
    pub connector_verification_evidence: usize,
    pub incidents: usize,
    pub critical_incidents: usize,
    pub warning_incidents: usize,
    pub total_records: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainSoakIncident {
    pub source: String,
    pub id: String,
    pub title: String,
    pub status: String,
    pub severity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<i64>,
    pub reason: String,
    pub recommendation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainSoakTimelineItem {
    pub source: String,
    pub id: String,
    pub label: String,
    pub status: String,
    pub at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainSoakReport {
    pub generated_at: String,
    pub status: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    pub window_days: u32,
    pub since: String,
    pub until: String,
    pub summary: DomainSoakReportSummary,
    #[serde(default)]
    pub incidents: Vec<DomainSoakIncident>,
    #[serde(default)]
    pub timeline: Vec<DomainSoakTimelineItem>,
    #[serde(default)]
    pub recommended_next_steps: Vec<String>,
    pub markdown: String,
    pub operational_gate: DomainOperationalGateReport,
}

struct SoakWorkflowRow {
    id: String,
    kind: String,
    state: String,
    origin: Option<String>,
    created_at: String,
    updated_at: String,
    completed_at: Option<String>,
    blocked_reason: Option<String>,
}

struct SoakWorkflowEventRow {
    run_id: String,
    event_type: String,
    payload: Value,
    created_at: String,
}

struct SoakLoopRunRow {
    id: String,
    loop_id: String,
    state: String,
    trigger_reason: String,
    result_summary: Option<String>,
    error: Option<String>,
    started_at: String,
    finished_at: Option<String>,
}

struct SoakCampaignRow {
    campaign_id: String,
    campaign_name: String,
    campaign_status: String,
    campaign_updated_at: String,
    item_id: Option<String>,
    item_title: Option<String>,
    item_status: Option<String>,
    item_attempt: Option<usize>,
    item_error: Option<String>,
    item_started_at: Option<String>,
    item_finished_at: Option<String>,
    item_updated_at: Option<String>,
}

struct DomainGateScope {
    scope: String,
    session_id: Option<String>,
    project_id: Option<String>,
    domain: Option<String>,
    window_days: u32,
    since: String,
    include_synthetic: bool,
}

struct QualityGateRow {
    state: String,
    domain: String,
    checks: Vec<(String, String)>,
}

pub(crate) fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS domain_eval_runs (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            project_id TEXT,
            task_id TEXT NOT NULL,
            task_version TEXT NOT NULL,
            domain TEXT NOT NULL,
            label TEXT NOT NULL,
            status TEXT NOT NULL,
            score REAL NOT NULL,
            source_type TEXT NOT NULL DEFAULT 'live',
            report_json TEXT NOT NULL DEFAULT '{}',
            source_quality_run_id TEXT,
            created_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (source_quality_run_id) REFERENCES domain_quality_runs(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_domain_eval_runs_scope
            ON domain_eval_runs(project_id, session_id, domain, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_domain_eval_runs_task
            ON domain_eval_runs(task_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_domain_eval_runs_status
            ON domain_eval_runs(status, created_at DESC);

        CREATE TABLE IF NOT EXISTS domain_eval_fixture_runs (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            execution_mode TEXT NOT NULL,
            source_type TEXT NOT NULL,
            status TEXT NOT NULL,
            passed INTEGER NOT NULL DEFAULT 0,
            session_id TEXT NOT NULL,
            goal_id TEXT,
            workflow_run_id TEXT,
            quality_run_id TEXT,
            eval_run_id TEXT,
            report_json TEXT NOT NULL DEFAULT '{}',
            error TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (eval_run_id) REFERENCES domain_eval_runs(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_domain_eval_fixture_runs_recent
            ON domain_eval_fixture_runs(source_type, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_domain_eval_fixture_runs_status
            ON domain_eval_fixture_runs(status, created_at DESC);",
    )?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS domain_eval_campaigns (
            id TEXT PRIMARY KEY,
            session_id TEXT,
            project_id TEXT,
            name TEXT NOT NULL,
            status TEXT NOT NULL,
            domain TEXT,
            task_filter_json TEXT NOT NULL DEFAULT '{}',
            model_matrix_json TEXT NOT NULL DEFAULT '[]',
            execution_mode TEXT NOT NULL,
            max_budget_usd REAL,
            timeout_secs INTEGER,
            error TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            started_at TEXT,
            finished_at TEXT,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_domain_eval_campaigns_scope
            ON domain_eval_campaigns(project_id, session_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_domain_eval_campaigns_status
            ON domain_eval_campaigns(status, updated_at DESC);

        CREATE TABLE IF NOT EXISTS domain_eval_campaign_items (
            id TEXT PRIMARY KEY,
            campaign_id TEXT NOT NULL,
            task_id TEXT NOT NULL,
            task_title TEXT NOT NULL,
            domain TEXT NOT NULL,
            execution_mode TEXT NOT NULL,
            provider_id TEXT,
            model_id TEXT,
            label TEXT,
            status TEXT NOT NULL,
            attempt INTEGER NOT NULL DEFAULT 0,
            fixture_run_id TEXT,
            eval_run_id TEXT,
            score REAL,
            total_checks INTEGER NOT NULL DEFAULT 0,
            passed_checks INTEGER NOT NULL DEFAULT 0,
            failed_checks INTEGER NOT NULL DEFAULT 0,
            report_json TEXT NOT NULL DEFAULT '{}',
            error TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            started_at TEXT,
            finished_at TEXT,
            FOREIGN KEY (campaign_id) REFERENCES domain_eval_campaigns(id) ON DELETE CASCADE,
            FOREIGN KEY (fixture_run_id) REFERENCES domain_eval_fixture_runs(id) ON DELETE SET NULL,
            FOREIGN KEY (eval_run_id) REFERENCES domain_eval_runs(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_domain_eval_campaign_items_campaign
            ON domain_eval_campaign_items(campaign_id, status, updated_at DESC);",
    )?;
    ensure_domain_eval_column(
        conn,
        "domain_eval_runs",
        "source_type",
        "ALTER TABLE domain_eval_runs ADD COLUMN source_type TEXT NOT NULL DEFAULT 'live';",
    )?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_domain_eval_runs_source
            ON domain_eval_runs(source_type, created_at DESC);",
    )?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS domain_eval_tasks (
            id TEXT NOT NULL,
            version TEXT NOT NULL,
            project_id TEXT,
            status TEXT NOT NULL DEFAULT 'active',
            source_type TEXT NOT NULL,
            source_id TEXT NOT NULL,
            source_path TEXT NOT NULL,
            task_json TEXT NOT NULL DEFAULT '{}',
            imported_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (id, version)
        );
        CREATE INDEX IF NOT EXISTS idx_domain_eval_tasks_domain_status
            ON domain_eval_tasks(status, json_extract(task_json, '$.domain'));
        CREATE INDEX IF NOT EXISTS idx_domain_eval_tasks_source
            ON domain_eval_tasks(source_type, source_id);",
    )?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS domain_eval_calibrations (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL,
            task_version TEXT NOT NULL,
            domain TEXT NOT NULL,
            project_id TEXT,
            scope TEXT NOT NULL,
            reviewer TEXT NOT NULL,
            verdict TEXT NOT NULL,
            note TEXT NOT NULL,
            source_run_id TEXT,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_domain_eval_calibrations_task
            ON domain_eval_calibrations(task_id, task_version, project_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_domain_eval_calibrations_domain
            ON domain_eval_calibrations(domain, project_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_domain_eval_calibrations_source_run
            ON domain_eval_calibrations(source_run_id);",
    )?;
    Ok(())
}

impl SessionDB {
    pub fn list_domain_eval_tasks(
        &self,
        input: ListDomainEvalTasksInput,
    ) -> Result<Vec<DomainEvalTask>> {
        let domain = input.domain.as_deref().map(normalize_domain);
        let project_id = input.project_id.as_deref().and_then(non_empty);
        let limit = input
            .limit
            .unwrap_or(usize::MAX)
            .clamp(1, MAX_DOMAIN_EVAL_LIMIT);
        let mut tasks = built_in_domain_eval_tasks()
            .into_iter()
            .filter(|task| {
                domain
                    .as_deref()
                    .map(|domain| task.domain == domain)
                    .unwrap_or(true)
            })
            .collect::<Vec<_>>();
        tasks.extend(self.list_imported_domain_eval_tasks(domain.as_deref(), limit)?);
        tasks.sort_by(|a, b| {
            a.domain
                .cmp(&b.domain)
                .then_with(|| a.id.cmp(&b.id))
                .then_with(|| a.version.cmp(&b.version))
        });
        tasks.truncate(limit);
        self.attach_domain_eval_calibrations(&mut tasks, project_id)?;
        Ok(tasks)
    }

    pub fn run_domain_eval_task(
        &self,
        input: RunDomainEvalTaskInput,
    ) -> Result<DomainEvalRunRecord> {
        let session_id = non_empty(&input.session_id)
            .ok_or_else(|| anyhow!("session_id is required"))?
            .to_string();
        let task_id = non_empty(&input.task_id)
            .ok_or_else(|| anyhow!("task_id is required"))?
            .to_string();
        let session = self
            .get_session(&session_id)?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        if session.incognito {
            bail!("domain eval is disabled for incognito sessions");
        }
        let mut task = self
            .resolve_domain_eval_task(&task_id)?
            .ok_or_else(|| anyhow!("domain eval task not found: {task_id}"))?;
        self.attach_domain_eval_calibrations(
            std::slice::from_mut(&mut task),
            session.project_id.as_deref(),
        )?;
        let quality = self.resolve_eval_quality_snapshot(&session_id, &task.domain, &input)?;
        let report = self.build_domain_eval_report(&session_id, &task, quality.as_ref())?;
        let now = now_rfc3339();
        let id = format!("der_{}", uuid::Uuid::new_v4().simple());
        let label = input
            .label
            .as_deref()
            .and_then(non_empty)
            .unwrap_or(&task.title)
            .to_string();
        let source_quality_run_id = quality.as_ref().map(|snapshot| snapshot.run.id.clone());
        let source_type = normalized_eval_source_type(input.source_type.as_deref());
        let record = DomainEvalRunRecord {
            id: id.clone(),
            session_id: session_id.clone(),
            project_id: session.project_id.clone(),
            task_id: task.id.clone(),
            task_version: task.version.clone(),
            domain: task.domain.clone(),
            label,
            status: report.status.clone(),
            score: report.score,
            source_type: source_type.clone(),
            report,
            source_quality_run_id,
            created_at: now,
        };
        let report_json = serde_json::to_string(&record.report)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO domain_eval_runs (
                id, session_id, project_id, task_id, task_version, domain, label,
                status, score, source_type, report_json, source_quality_run_id, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                record.id,
                record.session_id,
                record.project_id,
                record.task_id,
                record.task_version,
                record.domain,
                record.label,
                record.status,
                record.score,
                record.source_type,
                report_json,
                record.source_quality_run_id,
                record.created_at,
            ],
        )?;
        drop(conn);
        self.get_domain_eval_run(&id)?
            .ok_or_else(|| anyhow!("domain eval run vanished after insert: {id}"))
    }

    pub async fn run_domain_eval_fixture(
        db: Arc<Self>,
        input: RunDomainEvalFixtureInput,
    ) -> Result<DomainEvalFixtureReport> {
        let fixture = input.fixture;
        let name = non_empty(&fixture.name)
            .ok_or_else(|| anyhow!("fixture.name is required"))?
            .to_string();
        let execution_mode = non_empty(&fixture.execution_mode)
            .unwrap_or("trace_fixture")
            .to_string();
        let source_type = fixture_source_type(&execution_mode);
        let task_id = non_empty(&fixture.task_id)
            .ok_or_else(|| anyhow!("fixture.task_id is required"))?
            .to_string();
        let task = db
            .resolve_domain_eval_task(&task_id)?
            .ok_or_else(|| anyhow!("domain eval task not found: {task_id}"))?;
        let session = db.create_session(DEFAULT_AGENT_ID)?;
        db.set_session_kind(&session.id, SessionKind::EvalFixture)?;
        let mut report = DomainEvalFixtureReport {
            fixture_run_id: None,
            name: name.clone(),
            execution_mode: execution_mode.clone(),
            source_type: source_type.clone(),
            status: "failed".to_string(),
            passed: false,
            session_id: session.id.clone(),
            goal_id: None,
            workflow_run_id: None,
            quality_run_id: None,
            eval_run: None,
            execution: None,
            checks: Vec::new(),
            error: None,
        };

        if !matches!(execution_mode.as_str(), "trace_fixture" | "agent") {
            report.error = Some(format!(
                "unsupported domain eval fixture execution mode {execution_mode:?}; expected trace_fixture or agent"
            ));
            report.checks.push(DomainEvalFixtureCheck {
                name: "execution_mode".to_string(),
                status: "failed".to_string(),
                expected: "trace_fixture or agent".to_string(),
                actual: execution_mode,
                detail: "Domain eval fixtures only support deterministic trace replay or explicit agent-backed execution.".to_string(),
            });
            persist_domain_eval_fixture_report(&db, &mut report)?;
            return Ok(report);
        }

        let domain = fixture
            .domain
            .as_deref()
            .and_then(non_empty)
            .map(normalize_domain)
            .unwrap_or_else(|| task.domain.clone());
        let goal = db.create_goal(crate::goal::CreateGoalInput {
            session_id: session.id.clone(),
            objective: fixture
                .goal
                .objective
                .as_deref()
                .and_then(non_empty)
                .unwrap_or(&task.input.prompt)
                .to_string(),
            completion_criteria: fixture
                .goal
                .completion_criteria
                .as_deref()
                .and_then(non_empty)
                .unwrap_or_else(|| {
                    task.success_criteria
                        .first()
                        .map(String::as_str)
                        .unwrap_or("Domain eval fixture criteria must pass.")
                })
                .to_string(),
            domain: Some(domain.clone()),
            workflow_template_id: fixture.goal.workflow_template_id.clone(),
            workflow_template_version: fixture.goal.workflow_template_version.clone(),
            workflow_task_type: fixture
                .goal
                .workflow_task_type
                .clone()
                .or_else(|| Some(task.task_type.clone())),
            budget_token_limit: None,
            budget_time_limit_secs: None,
            budget_turn_limit: None,
        })?;
        report.goal_id = Some(goal.goal.id.clone());

        if execution_mode == "agent" {
            let execution =
                run_domain_eval_agent_execution(db.clone(), &session.id, &task, &fixture).await?;
            let execution_failed = execution.status != "completed";
            report.execution = Some(execution);
            if execution_failed {
                report.checks =
                    domain_eval_fixture_checks(&fixture.checks, None, &report.execution);
                report.error = report
                    .execution
                    .as_ref()
                    .and_then(|execution| execution.error.clone())
                    .or_else(|| Some("agent execution failed".to_string()));
                report.status = "failed".to_string();
                report.passed = false;
                persist_domain_eval_fixture_report(&db, &mut report)?;
                return Ok(report);
            }
        }

        if execution_mode == "trace_fixture" {
            for evidence in &fixture.evidence {
                db.record_domain_evidence(RecordDomainEvidenceInput {
                    goal_id: Some(goal.goal.id.clone()),
                    session_id: Some(session.id.clone()),
                    project_id: session.project_id.clone(),
                    domain: domain.clone(),
                    evidence_type: evidence.evidence_type.clone(),
                    title: evidence.title.clone(),
                    summary: evidence.summary.clone(),
                    source_metadata: evidence.source_metadata.clone(),
                    confidence: evidence.confidence.or(Some(0.95)),
                    access_scope: Some("fixture".to_string()),
                    redaction_status: Some("not_required".to_string()),
                })?;
            }

            if let Some(workflow) = fixture.workflow.clone() {
                let run = db.create_workflow_run(CreateWorkflowRunInput {
                    session_id: session.id.clone(),
                    kind: workflow.kind,
                    execution_mode: workflow.execution_mode,
                    script_source: workflow.script_source,
                    budget: json!({ "fixture": name }),
                    parent_run_id: None,
                    origin: Some("domain_eval_fixture".to_string()),
                    goal_id: Some(goal.goal.id.clone()),
                    goal_criterion_id: None,
                    worktree_id: None,
                })?;
                report.workflow_run_id = Some(run.id);
            }
        }

        let source_quality_run_id = if fixture
            .quality
            .as_ref()
            .map(|quality| quality.run)
            .unwrap_or(true)
        {
            let quality = fixture.quality.clone().unwrap_or_default();
            let snapshot = db.run_domain_quality_for_session(RunDomainQualityInput {
                session_id: session.id.clone(),
                goal_id: Some(goal.goal.id.clone()),
                domain: Some(domain),
                template_id: fixture.goal.workflow_template_id.clone(),
                template_version: fixture.goal.workflow_template_version.clone(),
                profiles: Vec::new(),
                artifact_title: Some(task.title.clone()),
                artifact_kind: Some(task.task_type.clone()),
                source_metadata: fixture_quality_source_metadata(
                    quality.source_metadata,
                    &source_type,
                    &name,
                    &execution_mode,
                ),
                explicit_user_approval: quality.explicit_user_approval,
            })?;
            let quality_run_id = snapshot.run.id;
            report.quality_run_id = Some(quality_run_id.clone());
            Some(quality_run_id)
        } else {
            None
        };

        let eval_run = db.run_domain_eval_task(RunDomainEvalTaskInput {
            session_id: session.id.clone(),
            task_id: task.id,
            label: fixture.label.clone().or_else(|| Some(name.clone())),
            source_quality_run_id,
            source_type: Some(source_type),
        })?;
        report.checks =
            domain_eval_fixture_checks(&fixture.checks, Some(&eval_run), &report.execution);
        let passed = report.checks.iter().all(|check| check.status == "passed");
        report.status = if passed { "passed" } else { "failed" }.to_string();
        report.passed = passed;
        report.eval_run = Some(eval_run);
        persist_domain_eval_fixture_report(&db, &mut report)?;
        Ok(report)
    }

    pub fn import_domain_eval_case(
        &self,
        input: ImportDomainEvalCaseInput,
    ) -> Result<ImportDomainEvalCaseResult> {
        let proposal_id = non_empty(&input.proposal_id)
            .ok_or_else(|| anyhow!("proposal_id is required"))?
            .to_string();
        let proposal = self
            .get_coding_improvement_proposal(&proposal_id)?
            .ok_or_else(|| anyhow!("coding improvement proposal not found: {proposal_id}"))?;
        if proposal.kind != "domain_eval_case" {
            bail!(
                "proposal {} is {} not domain_eval_case",
                proposal.id,
                proposal.kind
            );
        }
        if proposal.status != "promoted" {
            bail!(
                "domain eval case proposal {} must be promoted before import (status: {})",
                proposal.id,
                proposal.status
            );
        }
        let promotion = proposal
            .promotion
            .as_ref()
            .filter(|record| record.promoted)
            .ok_or_else(|| anyhow!("proposal {} has no promoted artifact record", proposal.id))?;
        let source_path = promotion
            .artifacts
            .iter()
            .find(|artifact| {
                matches!(
                    artifact.kind.as_str(),
                    "create_promoted_file" | "existing_promoted_file"
                ) && artifact.path.ends_with(".json")
            })
            .or_else(|| {
                promotion
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.path.ends_with(".json"))
            })
            .map(|artifact| artifact.path.clone())
            .ok_or_else(|| anyhow!("proposal {} promotion has no JSON artifact", proposal.id))?;
        let source = PathBuf::from(&source_path);
        let source_content = std::fs::read_to_string(&source).map_err(|err| {
            anyhow!(
                "failed to read domain eval case {}: {}",
                source.display(),
                err
            )
        })?;
        let fixture: Value = serde_json::from_str(&source_content).map_err(|err| {
            anyhow!(
                "invalid domain eval case JSON {}: {}",
                source.display(),
                err
            )
        })?;
        let task = domain_eval_task_from_fixture(&proposal, &fixture)?;
        let task_json = serde_json::to_string(&task)?;
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let existing = conn
            .query_row(
                "SELECT task_json, project_id, source_path, imported_at
                 FROM domain_eval_tasks
                 WHERE id = ?1 AND version = ?2",
                params![task.id, task.version],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?;
        if let Some((existing_json, project_id, source_path, imported_at)) = existing {
            if !input.overwrite {
                let existing_task = serde_json::from_str(&existing_json).unwrap_or(task);
                return Ok(ImportDomainEvalCaseResult {
                    imported: false,
                    task: existing_task,
                    project_id,
                    source_path,
                    imported_at,
                });
            }
            conn.execute(
                "UPDATE domain_eval_tasks
                 SET project_id = ?1,
                     status = 'active',
                     source_type = 'coding_improvement_proposal',
                     source_id = ?2,
                     source_path = ?3,
                     task_json = ?4,
                     updated_at = ?5
                 WHERE id = ?6 AND version = ?7",
                params![
                    proposal.project_id.clone(),
                    proposal.id.clone(),
                    source_path.clone(),
                    task_json,
                    now,
                    task.id.clone(),
                    task.version.clone(),
                ],
            )?;
        } else {
            conn.execute(
                "INSERT INTO domain_eval_tasks (
                    id, version, project_id, status, source_type, source_id,
                    source_path, task_json, imported_at, updated_at
                 ) VALUES (
                    ?1, ?2, ?3, 'active', 'coding_improvement_proposal', ?4,
                    ?5, ?6, ?7, ?7
                 )",
                params![
                    task.id.clone(),
                    task.version.clone(),
                    proposal.project_id.clone(),
                    proposal.id.clone(),
                    source_path.clone(),
                    task_json,
                    now,
                ],
            )?;
        }
        Ok(ImportDomainEvalCaseResult {
            imported: true,
            task,
            project_id: proposal.project_id.clone(),
            source_path,
            imported_at: now,
        })
    }

    pub fn record_domain_eval_calibration(
        &self,
        input: RecordDomainEvalCalibrationInput,
    ) -> Result<DomainEvalCalibrationRecord> {
        let task_id = non_empty(&input.task_id)
            .ok_or_else(|| anyhow!("task_id is required"))?
            .to_string();
        let task = self
            .resolve_domain_eval_task(&task_id)?
            .ok_or_else(|| anyhow!("domain eval task not found: {task_id}"))?;
        let mut task_version = input
            .task_version
            .as_deref()
            .and_then(non_empty)
            .unwrap_or(&task.version)
            .to_string();
        if task_version != task.version {
            bail!(
                "domain eval task {} version mismatch: {} != {}",
                task.id,
                task_version,
                task.version
            );
        }
        let mut project_id = input
            .project_id
            .as_deref()
            .and_then(non_empty)
            .map(ToOwned::to_owned);
        let source_run_id = input
            .source_run_id
            .as_deref()
            .and_then(non_empty)
            .map(ToOwned::to_owned);
        if let Some(source_run_id) = source_run_id.as_deref() {
            let run = self
                .get_domain_eval_run(source_run_id)?
                .ok_or_else(|| anyhow!("domain eval run not found: {source_run_id}"))?;
            if run.task_id != task.id {
                bail!(
                    "domain eval run {} is for task {} not {}",
                    run.id,
                    run.task_id,
                    task.id
                );
            }
            task_version = run.task_version;
            if project_id.is_none() {
                project_id = run.project_id;
            }
        }
        let reviewer = input
            .reviewer
            .as_deref()
            .and_then(non_empty)
            .unwrap_or("user")
            .to_string();
        let verdict = normalize_calibration_verdict(&input.verdict)?;
        let note = non_empty(&input.note)
            .ok_or_else(|| anyhow!("calibration note is required"))?
            .to_string();
        let scope = if project_id.is_some() {
            "project"
        } else {
            "user"
        }
        .to_string();
        let id = format!("dec_{}", uuid::Uuid::new_v4().simple());
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        if let Some(source_run_id) = source_run_id.as_deref() {
            let existing = conn
                .query_row(
                    "SELECT id, task_id, task_version, domain, project_id, scope, reviewer,
                            verdict, note, source_run_id, created_at
                     FROM domain_eval_calibrations
                     WHERE source_run_id = ?1
                       AND reviewer = ?2
                       AND scope = ?3
                       AND COALESCE(project_id, '') = COALESCE(?4, '')
                     ORDER BY created_at DESC
                     LIMIT 1",
                    params![source_run_id, &reviewer, &scope, project_id.clone()],
                    row_to_domain_eval_calibration,
                )
                .optional()?;
            if let Some(existing) = existing {
                return Ok(existing);
            }
        }
        conn.execute(
            "INSERT INTO domain_eval_calibrations (
                id, task_id, task_version, domain, project_id, scope, reviewer,
                verdict, note, source_run_id, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id,
                task.id,
                task_version,
                task.domain,
                project_id,
                scope,
                reviewer,
                verdict,
                note,
                source_run_id,
                now,
            ],
        )?;
        drop(conn);
        self.get_domain_eval_calibration(&id)?
            .ok_or_else(|| anyhow!("domain eval calibration vanished after insert: {id}"))
    }

    pub fn list_domain_eval_calibrations(
        &self,
        input: ListDomainEvalCalibrationsInput,
    ) -> Result<Vec<DomainEvalCalibrationRecord>> {
        let limit = input
            .limit
            .unwrap_or(DEFAULT_DOMAIN_EVAL_LIMIT)
            .clamp(1, MAX_DOMAIN_EVAL_LIMIT);
        let mut clauses = Vec::new();
        let mut params = Vec::new();
        if let Some(task_id) = input.task_id.as_deref().and_then(non_empty) {
            clauses.push("task_id = ?".to_string());
            params.push(task_id.to_string());
        }
        if let Some(domain) = input.domain.as_deref().and_then(non_empty) {
            clauses.push("domain = ?".to_string());
            params.push(normalize_domain(domain));
        }
        if let Some(project_id) = input.project_id.as_deref().and_then(non_empty) {
            if input.include_user_scope {
                clauses.push("(project_id = ? OR project_id IS NULL)".to_string());
            } else {
                clauses.push("project_id = ?".to_string());
            }
            params.push(project_id.to_string());
        } else {
            clauses.push("project_id IS NULL".to_string());
        }
        params.push(limit.to_string());
        let where_sql = if clauses.is_empty() {
            "1 = 1".to_string()
        } else {
            clauses.join(" AND ")
        };
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&format!(
            "SELECT id, task_id, task_version, domain, project_id, scope, reviewer,
                    verdict, note, source_run_id, created_at
             FROM domain_eval_calibrations
             WHERE {where_sql}
             ORDER BY created_at DESC
             LIMIT ?"
        ))?;
        let rows = stmt.query_map(
            params_from_iter(params.iter()),
            row_to_domain_eval_calibration,
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn list_domain_eval_runs(
        &self,
        input: ListDomainEvalRunsInput,
    ) -> Result<Vec<DomainEvalRunRecord>> {
        let limit = input
            .limit
            .unwrap_or(DEFAULT_DOMAIN_EVAL_LIMIT)
            .clamp(1, MAX_DOMAIN_EVAL_LIMIT);
        let window_days = input
            .window_days
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS);
        let since = since_timestamp(window_days);
        let mut clauses = vec!["der.created_at >= ?".to_string()];
        let mut params = vec![since];
        if let Some(session_id) = input.session_id.as_deref().and_then(non_empty) {
            clauses.push("der.session_id = ?".to_string());
            params.push(session_id.to_string());
        }
        if let Some(project_id) = input.project_id.as_deref().and_then(non_empty) {
            clauses.push("der.project_id = ?".to_string());
            params.push(project_id.to_string());
        }
        if let Some(domain) = input.domain.as_deref().and_then(non_empty) {
            clauses.push("der.domain = ?".to_string());
            params.push(normalize_domain(domain));
        }
        if let Some(task_id) = input.task_id.as_deref().and_then(non_empty) {
            clauses.push("der.task_id = ?".to_string());
            params.push(task_id.to_string());
        }
        if let Some(source_type) = input.source_type.as_deref().and_then(non_empty) {
            let source_type = normalized_eval_source_type(Some(source_type));
            if source_type == "fixture" {
                clauses.push("der.source_type LIKE 'fixture_%'".to_string());
            } else {
                clauses.push("der.source_type = ?".to_string());
                params.push(source_type);
            }
        } else if !input.include_synthetic {
            clauses.push("der.source_type NOT LIKE 'fixture_%'".to_string());
            clauses.push("s.kind != 'eval_fixture'".to_string());
        }
        params.push(limit.to_string());
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&format!(
            "SELECT der.id, der.session_id, der.project_id, der.task_id, der.task_version,
                    der.domain, der.label, der.status, der.score, der.source_type, der.report_json,
                    der.source_quality_run_id, der.created_at
             FROM domain_eval_runs der
             JOIN sessions s ON s.id = der.session_id
             WHERE s.incognito = 0 AND {}
             ORDER BY der.created_at DESC
             LIMIT ?",
            clauses.join(" AND ")
        ))?;
        let rows = stmt.query_map(params_from_iter(params.iter()), row_to_domain_eval_run)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn list_domain_eval_fixture_runs(
        &self,
        input: ListDomainEvalFixtureRunsInput,
    ) -> Result<Vec<DomainEvalFixtureRunRecord>> {
        let limit = input
            .limit
            .unwrap_or(DEFAULT_DOMAIN_EVAL_LIMIT)
            .clamp(1, MAX_DOMAIN_EVAL_LIMIT);
        let window_days = input
            .window_days
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS);
        let since = since_timestamp(window_days);
        let mut clauses = vec!["created_at >= ?".to_string()];
        let mut params = vec![since];
        if let Some(source_type) = input.source_type.as_deref().and_then(non_empty) {
            let source_type = normalized_eval_source_type(Some(source_type));
            if source_type == "fixture" {
                clauses.push("source_type LIKE 'fixture_%'".to_string());
            } else {
                clauses.push("source_type = ?".to_string());
                params.push(source_type);
            }
        } else {
            clauses.push("source_type LIKE 'fixture_%'".to_string());
        }
        if let Some(mode) = input.execution_mode.as_deref().and_then(non_empty) {
            clauses.push("execution_mode = ?".to_string());
            params.push(mode.to_string());
        }
        if let Some(status) = input.status.as_deref().and_then(non_empty) {
            clauses.push("status = ?".to_string());
            params.push(status.to_string());
        }
        params.push(limit.to_string());
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&format!(
            "SELECT id, name, execution_mode, source_type, status, passed, session_id,
                    goal_id, workflow_run_id, quality_run_id, eval_run_id, report_json,
                    error, created_at, updated_at
             FROM domain_eval_fixture_runs
             WHERE {}
             ORDER BY created_at DESC
             LIMIT ?",
            clauses.join(" AND ")
        ))?;
        let rows = stmt.query_map(
            params_from_iter(params.iter()),
            row_to_domain_eval_fixture_run,
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn create_domain_eval_campaign(
        &self,
        input: CreateDomainEvalCampaignInput,
    ) -> Result<DomainEvalCampaign> {
        let (session_id, project_id) =
            self.resolve_domain_eval_campaign_scope(input.session_id, input.project_id)?;
        let domain = input
            .domain
            .as_deref()
            .and_then(non_empty)
            .map(normalize_domain);
        let max_tasks = input
            .max_tasks
            .unwrap_or(DEFAULT_DOMAIN_EVAL_CAMPAIGN_TASKS)
            .clamp(1, MAX_DOMAIN_EVAL_CAMPAIGN_TASKS);
        let requested_task_ids = input
            .task_ids
            .iter()
            .filter_map(|id| non_empty(id).map(str::to_string))
            .collect::<BTreeSet<_>>();
        let mut tasks = self.list_domain_eval_tasks(ListDomainEvalTasksInput {
            domain: domain.clone(),
            project_id: project_id.clone(),
            limit: Some(MAX_DOMAIN_EVAL_LIMIT),
        })?;
        if !requested_task_ids.is_empty() {
            tasks.retain(|task| requested_task_ids.contains(&task.id));
        }
        tasks.truncate(max_tasks);
        if tasks.is_empty() {
            bail!("domain eval campaign requires at least one matching task");
        }

        let requested_execution_mode =
            normalize_campaign_execution_mode(input.execution_mode.as_deref())?;
        let models = normalize_domain_eval_campaign_models(input.models)?;
        if requested_execution_mode.as_deref() == Some("agent")
            && models
                .iter()
                .all(|model| model.provider_id.is_none() && model.model_id.is_none())
        {
            bail!("agent domain eval campaign requires at least one provider/model entry");
        }
        let item_modes = models
            .iter()
            .map(|model| {
                if model.provider_id.is_some() || model.model_id.is_some() {
                    "agent".to_string()
                } else {
                    requested_execution_mode
                        .clone()
                        .unwrap_or_else(|| "trace_fixture".to_string())
                }
            })
            .collect::<Vec<_>>();
        let execution_mode = if item_modes.iter().all(|mode| mode == &item_modes[0]) {
            item_modes[0].clone()
        } else {
            "mixed".to_string()
        };
        let task_filter = json!({
            "domain": domain,
            "taskIds": tasks.iter().map(|task| task.id.clone()).collect::<Vec<_>>(),
            "maxTasks": max_tasks,
        });
        let task_filter_json = serde_json::to_string(&task_filter)?;
        let model_matrix_json = serde_json::to_string(&models)?;
        let name = input
            .name
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string)
            .unwrap_or_else(|| {
                if execution_mode == "agent" || execution_mode == "mixed" {
                    "Domain eval model campaign".to_string()
                } else {
                    "Domain eval trace campaign".to_string()
                }
            });
        let id = format!("dec_{}", uuid::Uuid::new_v4().simple());
        let now = now_rfc3339();
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO domain_eval_campaigns (
                id, session_id, project_id, name, status, domain, task_filter_json,
                model_matrix_json, execution_mode, max_budget_usd, timeout_secs,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 'queued', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            params![
                id,
                session_id,
                project_id,
                name,
                task_filter
                    .get("domain")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                task_filter_json,
                model_matrix_json,
                execution_mode,
                input.max_budget_usd,
                input.timeout_secs.map(|value| value as i64),
                now,
            ],
        )?;
        for task in &tasks {
            for (model, item_execution_mode) in models.iter().zip(item_modes.iter()) {
                let item_id = format!("deci_{}", uuid::Uuid::new_v4().simple());
                tx.execute(
                    "INSERT INTO domain_eval_campaign_items (
                        id, campaign_id, task_id, task_title, domain, execution_mode,
                        provider_id, model_id, label, status, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'queued', ?10, ?10)",
                    params![
                        item_id,
                        id,
                        task.id,
                        task.title,
                        task.domain,
                        item_execution_mode,
                        model.provider_id,
                        model.model_id,
                        model.label,
                        now,
                    ],
                )?;
            }
        }
        tx.commit()?;
        drop(conn);
        self.get_domain_eval_campaign(&id)?
            .ok_or_else(|| anyhow!("domain eval campaign vanished after insert: {id}"))
    }

    pub fn list_domain_eval_campaigns(
        &self,
        input: ListDomainEvalCampaignsInput,
    ) -> Result<Vec<DomainEvalCampaign>> {
        let (session_id, project_id) =
            self.resolve_domain_eval_campaign_scope(input.session_id, input.project_id)?;
        let limit = input
            .limit
            .unwrap_or(DEFAULT_DOMAIN_EVAL_CAMPAIGN_LIMIT)
            .clamp(1, MAX_DOMAIN_EVAL_CAMPAIGN_LIMIT);
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
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&format!(
            "SELECT id FROM domain_eval_campaigns
             {where_sql}
             ORDER BY created_at DESC, id DESC
             LIMIT ?"
        ))?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
            row.get::<_, String>(0)
        })?;
        let ids = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        drop(conn);
        ids.into_iter()
            .filter_map(|id| self.get_domain_eval_campaign(&id).transpose())
            .collect()
    }

    pub fn get_domain_eval_campaign(
        &self,
        campaign_id: &str,
    ) -> Result<Option<DomainEvalCampaign>> {
        let campaign_id = campaign_id.trim();
        if campaign_id.is_empty() {
            return Ok(None);
        }
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let row = conn
            .query_row(
                "SELECT id, session_id, project_id, name, status, domain, task_filter_json,
                        model_matrix_json, execution_mode, max_budget_usd, timeout_secs,
                        created_at, updated_at, started_at, finished_at, error
                 FROM domain_eval_campaigns
                 WHERE id = ?1",
                params![campaign_id],
                row_to_domain_eval_campaign,
            )
            .optional()?;
        let Some(mut campaign) = row else {
            return Ok(None);
        };
        campaign.items = self.domain_eval_campaign_items_locked(&conn, campaign_id)?;
        campaign.summary = domain_eval_campaign_summary(&campaign.items);
        Ok(Some(campaign))
    }

    pub fn get_domain_eval_campaign_leaderboard(
        &self,
        input: DomainEvalCampaignLeaderboardInput,
    ) -> Result<DomainEvalCampaignLeaderboardReport> {
        let (session_id, project_id) =
            self.resolve_domain_eval_campaign_scope(input.session_id, input.project_id)?;
        let window_days = input
            .window_days
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS);
        let limit = input
            .limit
            .unwrap_or(DEFAULT_DOMAIN_EVAL_CAMPAIGN_LIMIT)
            .clamp(1, MAX_DOMAIN_EVAL_CAMPAIGN_LIMIT);
        let domain = input
            .domain
            .as_deref()
            .and_then(non_empty)
            .map(normalize_domain);
        let since = since_timestamp(window_days);
        let mut clauses = vec!["c.created_at >= ?".to_string()];
        let mut params = vec![since];
        if let Some(project_id) = project_id.as_ref() {
            clauses.push("c.project_id = ?".to_string());
            params.push(project_id.clone());
        } else if let Some(session_id) = session_id.as_ref() {
            clauses.push("c.session_id = ?".to_string());
            params.push(session_id.clone());
        }
        if let Some(domain) = domain.as_ref() {
            clauses.push("i.domain = ?".to_string());
            params.push(domain.clone());
        }
        let campaign_ids = input
            .campaign_ids
            .iter()
            .filter_map(|id| non_empty(id).map(str::to_string))
            .collect::<Vec<_>>();
        if !campaign_ids.is_empty() {
            let placeholders = std::iter::repeat_n("?", campaign_ids.len())
                .collect::<Vec<_>>()
                .join(", ");
            clauses.push(format!("c.id IN ({placeholders})"));
            params.extend(campaign_ids);
        }
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&format!(
            "SELECT c.id, c.name, i.id, i.task_id, i.domain, i.execution_mode,
                    i.provider_id, i.model_id, i.label, i.status, i.attempt,
                    i.eval_run_id, i.score, i.total_checks, i.failed_checks,
                    i.updated_at, i.error
             FROM domain_eval_campaign_items i
             JOIN domain_eval_campaigns c ON c.id = i.campaign_id
             WHERE {}
             ORDER BY i.updated_at DESC, i.id DESC
             LIMIT 500",
            clauses.join(" AND ")
        ))?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
            Ok(DomainLeaderboardItemRow {
                campaign_id: row.get(0)?,
                campaign_name: row.get(1)?,
                item_id: row.get(2)?,
                task_id: row.get(3)?,
                domain: row.get(4)?,
                execution_mode: row.get(5)?,
                provider_id: row.get(6)?,
                model_id: row.get(7)?,
                label: row.get(8)?,
                status: row.get(9)?,
                attempt: row.get::<_, i64>(10)?.max(0) as usize,
                eval_run_id: row.get(11)?,
                score: row.get(12)?,
                total_checks: row.get::<_, i64>(13)?.max(0) as usize,
                failed_checks: row.get::<_, i64>(14)?.max(0) as usize,
                updated_at: row.get(15)?,
                error: row.get(16)?,
            })
        })?;
        let item_rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut grouped: BTreeMap<DomainLeaderboardKey, DomainLeaderboardAccumulator> =
            BTreeMap::new();
        for row in item_rows {
            grouped
                .entry(DomainLeaderboardKey::from(&row))
                .or_default()
                .add(row);
        }
        let mut rows = grouped
            .into_iter()
            .map(|(key, acc)| acc.into_row(key))
            .collect::<Vec<_>>();
        rows.sort_by(compare_domain_leaderboard_rows);
        rows.truncate(limit);
        for (index, row) in rows.iter_mut().enumerate() {
            row.rank = index + 1;
        }
        let has_terminal_items = rows.iter().any(|row| {
            row.passed_items + row.failed_items + row.cancelled_items + row.interrupted_items > 0
        });
        let has_failed_items = rows
            .iter()
            .any(|row| row.failed_items + row.cancelled_items + row.interrupted_items > 0);
        let status = if rows.is_empty() || !has_terminal_items {
            "insufficient_data"
        } else if has_failed_items {
            "failed"
        } else {
            "passed"
        }
        .to_string();
        let scope = if project_id.is_some() {
            "project"
        } else if session_id.is_some() {
            "session"
        } else {
            "global"
        }
        .to_string();
        Ok(DomainEvalCampaignLeaderboardReport {
            generated_at: now_rfc3339(),
            status,
            scope,
            session_id,
            project_id,
            domain,
            window_days,
            rows,
        })
    }

    pub fn cancel_domain_eval_campaign(
        &self,
        campaign_id: &str,
    ) -> Result<Option<DomainEvalCampaign>> {
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE domain_eval_campaigns
             SET status = CASE WHEN status IN ('passed','failed','partial','cancelled','interrupted') THEN status ELSE 'cancel_requested' END,
                 updated_at = ?2,
                 error = CASE WHEN status IN ('passed','failed','partial','cancelled','interrupted') THEN error ELSE 'Cancellation requested' END
             WHERE id = ?1",
            params![campaign_id, now],
        )?;
        if changed > 0 {
            conn.execute(
                "UPDATE domain_eval_campaign_items
                 SET status = 'cancelled', updated_at = ?2, finished_at = ?2, error = 'Cancelled before run'
                 WHERE campaign_id = ?1 AND status = 'queued'",
                params![campaign_id, now],
            )?;
        }
        drop(conn);
        self.get_domain_eval_campaign(campaign_id)
    }

    pub fn prepare_domain_eval_campaign_run(
        &self,
        campaign_id: &str,
        retry_failed_only: bool,
    ) -> Result<Vec<DomainEvalCampaignItem>> {
        let now = now_rfc3339();
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        if retry_failed_only {
            tx.execute(
                "UPDATE domain_eval_campaign_items
                 SET status = 'queued', fixture_run_id = NULL, eval_run_id = NULL,
                     score = NULL, total_checks = 0, passed_checks = 0, failed_checks = 0,
                     report_json = '{}', updated_at = ?2, started_at = NULL,
                     finished_at = NULL, error = NULL
                 WHERE campaign_id = ?1 AND status IN ('failed','interrupted','cancelled')",
                params![campaign_id, now],
            )?;
        }
        tx.execute(
            "UPDATE domain_eval_campaigns
             SET status = 'running', started_at = COALESCE(started_at, ?2),
                 updated_at = ?2, finished_at = NULL, error = NULL
             WHERE id = ?1 AND (?3 = 1 OR status != 'cancel_requested')",
            params![campaign_id, now, if retry_failed_only { 1 } else { 0 }],
        )?;
        tx.commit()?;
        drop(conn);
        let campaign = self
            .get_domain_eval_campaign(campaign_id)?
            .ok_or_else(|| anyhow!("domain eval campaign not found: {campaign_id}"))?;
        Ok(campaign
            .items
            .into_iter()
            .filter(|item| item.status == "queued")
            .collect())
    }

    pub fn is_domain_eval_campaign_cancel_requested(&self, campaign_id: &str) -> Result<bool> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let status = conn
            .query_row(
                "SELECT status FROM domain_eval_campaigns WHERE id = ?1",
                params![campaign_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(matches!(
            status.as_deref(),
            Some("cancel_requested" | "cancelled")
        ))
    }

    pub fn mark_domain_eval_campaign_item_running(
        &self,
        item_id: &str,
    ) -> Result<Option<DomainEvalCampaignItem>> {
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE domain_eval_campaign_items
             SET status = 'running', attempt = attempt + 1, started_at = ?2,
                 updated_at = ?2, error = NULL
             WHERE id = ?1 AND status = 'queued'",
            params![item_id, now],
        )?;
        conn.query_row(
            "SELECT id, campaign_id, task_id, task_title, domain, execution_mode,
                    provider_id, model_id, label, status, attempt, fixture_run_id,
                    eval_run_id, score, total_checks, passed_checks, failed_checks,
                    started_at, finished_at, error
             FROM domain_eval_campaign_items WHERE id = ?1",
            params![item_id],
            row_to_domain_eval_campaign_item,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn finish_domain_eval_campaign_item(
        &self,
        item_id: &str,
        report: &DomainEvalFixtureReport,
    ) -> Result<()> {
        let now = now_rfc3339();
        let status = if report.passed { "passed" } else { "failed" };
        let eval_run_id = report.eval_run.as_ref().map(|run| run.id.clone());
        let score = report.eval_run.as_ref().map(|run| run.score);
        let total_checks = report.checks.len();
        let passed_checks = report
            .checks
            .iter()
            .filter(|check| check.status == "passed")
            .count();
        let failed_checks = report
            .checks
            .iter()
            .filter(|check| check.status == "failed")
            .count();
        let report_json = serde_json::to_string(report)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE domain_eval_campaign_items
             SET status = ?2, fixture_run_id = ?3, eval_run_id = ?4, score = ?5,
                 total_checks = ?6, passed_checks = ?7, failed_checks = ?8,
                 report_json = ?9, error = ?10, updated_at = ?11, finished_at = ?11
             WHERE id = ?1",
            params![
                item_id,
                status,
                &report.fixture_run_id,
                eval_run_id,
                score,
                total_checks as i64,
                passed_checks as i64,
                failed_checks as i64,
                report_json,
                report
                    .error
                    .as_ref()
                    .map(|error| truncate_for_storage(error, 2000)),
                now,
            ],
        )?;
        Ok(())
    }

    pub fn fail_domain_eval_campaign_item(&self, item_id: &str, error: &str) -> Result<()> {
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE domain_eval_campaign_items
             SET status = 'failed', error = ?2, updated_at = ?3, finished_at = ?3
             WHERE id = ?1",
            params![item_id, truncate_for_storage(error, 2000), now],
        )?;
        Ok(())
    }

    pub fn complete_domain_eval_campaign(&self, campaign_id: &str) -> Result<()> {
        let now = now_rfc3339();
        let campaign = self
            .get_domain_eval_campaign(campaign_id)?
            .ok_or_else(|| anyhow!("domain eval campaign not found: {campaign_id}"))?;
        let summary = domain_eval_campaign_summary(&campaign.items);
        let status = if campaign.status == "cancel_requested" || summary.cancelled_items > 0 {
            "cancelled"
        } else if summary.running_items > 0 || summary.queued_items > 0 {
            "interrupted"
        } else if summary.failed_items > 0 || summary.interrupted_items > 0 {
            if summary.passed_items > 0 {
                "partial"
            } else {
                "failed"
            }
        } else if summary.passed_items > 0 {
            "passed"
        } else {
            "failed"
        };
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE domain_eval_campaigns
             SET status = ?2, updated_at = ?3, finished_at = ?3,
                 error = CASE WHEN ?2 = 'passed' THEN NULL ELSE error END
             WHERE id = ?1",
            params![campaign_id, status, now],
        )?;
        Ok(())
    }

    fn resolve_domain_eval_campaign_scope(
        &self,
        session_id: Option<String>,
        project_id: Option<String>,
    ) -> Result<(Option<String>, Option<String>)> {
        let session_id = session_id
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        let mut project_id = project_id
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        if let Some(session_id) = session_id.as_ref() {
            let session = self
                .get_session(session_id)?
                .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
            if session.incognito {
                bail!("domain eval campaign is disabled for incognito sessions");
            }
            if project_id.is_none() {
                project_id = session.project_id;
            }
        }
        Ok((session_id, project_id))
    }

    fn domain_eval_campaign_items_locked(
        &self,
        conn: &Connection,
        campaign_id: &str,
    ) -> Result<Vec<DomainEvalCampaignItem>> {
        let mut stmt = conn.prepare(
            "SELECT id, campaign_id, task_id, task_title, domain, execution_mode,
                    provider_id, model_id, label, status, attempt, fixture_run_id,
                    eval_run_id, score, total_checks, passed_checks, failed_checks,
                    started_at, finished_at, error
             FROM domain_eval_campaign_items
             WHERE campaign_id = ?1
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![campaign_id], row_to_domain_eval_campaign_item)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn evaluate_domain_quality_gate(
        &self,
        input: DomainQualityGateInput,
    ) -> Result<DomainQualityGateReport> {
        let thresholds = domain_quality_gate_thresholds(&input);
        let scope = self.resolve_domain_quality_gate_scope(&input)?;
        let summary = self.domain_quality_gate_summary(&scope)?;
        let mut checks = Vec::new();
        push_gate_check(
            &mut checks,
            "domain_eval_runs",
            if summary.eval_runs >= thresholds.min_eval_runs {
                "passed"
            } else {
                "insufficient_data"
            },
            "p1",
            format!("at least {} domain eval run(s)", thresholds.min_eval_runs),
            summary.eval_runs.to_string(),
            "Domain gate requires explicit non-coding eval evidence; coding benchmark runs do not count.",
        );
        push_gate_check(
            &mut checks,
            "domain_eval_pass_rate",
            match summary.pass_rate {
                Some(rate) if rate >= thresholds.min_pass_rate => "passed",
                Some(_) => "failed",
                None => "insufficient_data",
            },
            "p1",
            format!("pass rate >= {:.0}%", thresholds.min_pass_rate * 100.0),
            summary
                .pass_rate
                .map(|rate| format!("{:.0}%", rate * 100.0))
                .unwrap_or_else(|| "n/a".to_string()),
            "Failed or insufficient domain eval runs block the domain quality gate.",
        );
        push_gate_check(
            &mut checks,
            "domain_eval_average_score",
            match summary.average_score {
                Some(score) if score >= thresholds.min_average_score => "passed",
                Some(_) => "failed",
                None => "insufficient_data",
            },
            "p2",
            format!("average score >= {:.2}", thresholds.min_average_score),
            summary
                .average_score
                .map(|score| format!("{score:.2}"))
                .unwrap_or_else(|| "n/a".to_string()),
            "Average score catches partial evidence quality regressions even when status is not failed.",
        );
        push_gate_check(
            &mut checks,
            "domain_quality_runs",
            if summary.quality_runs >= thresholds.min_quality_runs {
                "passed"
            } else {
                "insufficient_data"
            },
            "p1",
            format!(
                "at least {} domain quality run(s)",
                thresholds.min_quality_runs
            ),
            summary.quality_runs.to_string(),
            "Domain Quality run/check history is required beside eval scoring.",
        );
        push_gate_check(
            &mut checks,
            "blocked_domain_quality",
            if summary.blocked_quality_runs
                + summary.failed_quality_runs
                + summary.needs_user_quality_runs
                <= thresholds.max_blocked_quality_runs
            {
                "passed"
            } else {
                "failed"
            },
            "p1",
            format!(
                "blocked/failed/needs_user quality runs <= {}",
                thresholds.max_blocked_quality_runs
            ),
            (summary.blocked_quality_runs
                + summary.failed_quality_runs
                + summary.needs_user_quality_runs)
                .to_string(),
            "Open domain quality blockers mean the non-coding task is not releasable.",
        );
        push_gate_check(
            &mut checks,
            "domain_coverage",
            if summary.domains_covered >= thresholds.min_domain_coverage {
                "passed"
            } else {
                "insufficient_data"
            },
            "p2",
            format!("at least {} domain(s)", thresholds.min_domain_coverage),
            summary.domains_covered.to_string(),
            "General eval must make the covered domains explicit and not masquerade as a global score.",
        );
        if thresholds.require_approval_safety {
            push_gate_check(
                &mut checks,
                "approval_safety",
                if summary.approval_blockers == 0 {
                    "passed"
                } else {
                    "failed"
                },
                "p1",
                "no approval blockers".to_string(),
                summary.approval_blockers.to_string(),
                "High-risk send/share/external-update actions must have explicit user approval evidence.",
            );
        }
        let status = gate_status(&checks);
        Ok(DomainQualityGateReport {
            generated_at: now_rfc3339(),
            status,
            scope: scope.scope,
            session_id: scope.session_id,
            project_id: scope.project_id,
            domain: scope.domain,
            window_days: scope.window_days,
            since: scope.since,
            thresholds,
            summary,
            checks,
        })
    }

    pub fn evaluate_domain_readiness_gate(
        &self,
        input: DomainReadinessGateInput,
    ) -> Result<DomainReadinessGateReport> {
        let thresholds = domain_readiness_gate_thresholds(&input);
        let quality_gate = self.evaluate_domain_quality_gate(DomainQualityGateInput {
            session_id: input.session_id.clone(),
            project_id: input.project_id.clone(),
            domain: input.domain.clone(),
            window_days: Some(thresholds.window_days),
            min_eval_runs: Some(thresholds.min_eval_runs),
            min_pass_rate: Some(thresholds.min_pass_rate),
            min_average_score: Some(thresholds.min_average_score),
            min_quality_runs: Some(thresholds.min_quality_runs),
            max_blocked_quality_runs: Some(thresholds.max_blocked_quality_runs),
            min_domain_coverage: Some(thresholds.min_domain_coverage),
            require_approval_safety: thresholds.require_approval_safety,
            include_synthetic: thresholds.include_synthetic,
        })?;
        let campaign_leaderboard =
            self.get_domain_eval_campaign_leaderboard(DomainEvalCampaignLeaderboardInput {
                session_id: quality_gate.session_id.clone(),
                project_id: quality_gate.project_id.clone(),
                domain: quality_gate.domain.clone(),
                window_days: Some(thresholds.window_days),
                limit: Some(MAX_DOMAIN_EVAL_CAMPAIGN_LIMIT),
                campaign_ids: Vec::new(),
            })?;
        let mut summary = self.domain_readiness_campaign_summary(
            quality_gate.session_id.as_deref(),
            quality_gate.project_id.as_deref(),
            quality_gate.domain.as_deref(),
            &quality_gate.since,
        )?;
        summary.eval_runs = quality_gate.summary.eval_runs;
        summary.quality_runs = quality_gate.summary.quality_runs;
        summary.leaderboard_rows = campaign_leaderboard.rows.len();
        summary.quality_status = quality_gate.status.clone();
        summary.leaderboard_status = campaign_leaderboard.status.clone();

        let campaign_failures = summary.failed_campaign_items
            + summary.cancelled_campaign_items
            + summary.interrupted_campaign_items;
        let mut checks = Vec::new();
        push_readiness_check(
            &mut checks,
            "domain_quality_gate",
            &quality_gate.status,
            "blocking",
            "quality gate passed".to_string(),
            quality_gate.status.clone(),
            "Readiness keeps live domain eval and domain quality evidence as the first gate.",
        );
        push_readiness_check(
            &mut checks,
            "campaign_sample",
            if summary.campaign_items >= thresholds.min_campaign_items {
                "passed"
            } else {
                "insufficient_data"
            },
            "blocking",
            format!(
                "at least {} domain campaign item(s)",
                thresholds.min_campaign_items
            ),
            format!("{} item(s)", summary.campaign_items),
            "Durable campaign evidence proves the domain workflow can be replayed and compared.",
        );
        push_readiness_check(
            &mut checks,
            "campaign_completion",
            if summary.campaign_items < thresholds.min_campaign_items {
                "insufficient_data"
            } else if summary.active_campaigns == 0
                && summary.terminal_campaign_items >= summary.campaign_items
            {
                "passed"
            } else {
                "insufficient_data"
            },
            "blocking",
            "no active or queued domain campaigns".to_string(),
            format!(
                "{} active, {}/{} terminal item(s)",
                summary.active_campaigns, summary.terminal_campaign_items, summary.campaign_items
            ),
            "Running campaigns are observable long tasks, but readiness should wait for their final result.",
        );
        let leaderboard_status = if summary.leaderboard_rows < thresholds.min_leaderboard_rows {
            "insufficient_data"
        } else {
            campaign_leaderboard.status.as_str()
        };
        push_readiness_check(
            &mut checks,
            "campaign_leaderboard",
            leaderboard_status,
            "blocking",
            format!(
                "at least {} comparable leaderboard row(s) and no failed rows",
                thresholds.min_leaderboard_rows
            ),
            format!(
                "{} row(s), status {}",
                summary.leaderboard_rows, campaign_leaderboard.status
            ),
            "The same campaign evidence feeds the model/execution comparison view.",
        );
        push_readiness_check(
            &mut checks,
            "campaign_failures",
            if campaign_failures <= thresholds.max_failed_campaign_items {
                "passed"
            } else {
                "failed"
            },
            "blocking",
            format!(
                "<= {} failed/cancelled/interrupted campaign item(s)",
                thresholds.max_failed_campaign_items
            ),
            format!("{campaign_failures} failed/cancelled/interrupted item(s)"),
            "Known domain campaign failures must be retried, fixed, or allowed to age out of the gate window.",
        );
        push_readiness_check(
            &mut checks,
            "learning_closure",
            if summary.pending_learning_campaigns > 0
                || summary.open_learning_proposals > thresholds.max_open_learning_proposals
            {
                "failed"
            } else {
                "passed"
            },
            "blocking",
            format!(
                "no unmaterialized failures and <= {} open learning proposal(s)",
                thresholds.max_open_learning_proposals
            ),
            format!(
                "{} pending campaign(s), {} open proposal(s)",
                summary.pending_learning_campaigns, summary.open_learning_proposals
            ),
            "Failed campaign evidence should become domain eval cases or guidance drafts, then be resolved by the user.",
        );

        let blockers = checks
            .iter()
            .filter(|check| check.status != "passed" && check.severity != "advisory")
            .map(|check| check.name.clone())
            .collect::<Vec<_>>();
        let status = readiness_status(&checks);
        let recommended_next_steps = domain_readiness_recommendations(&checks);

        Ok(DomainReadinessGateReport {
            generated_at: now_rfc3339(),
            status,
            scope: quality_gate.scope.clone(),
            session_id: quality_gate.session_id.clone(),
            project_id: quality_gate.project_id.clone(),
            domain: quality_gate.domain.clone(),
            since: quality_gate.since.clone(),
            thresholds,
            summary,
            checks,
            quality_gate,
            campaign_leaderboard,
            blockers,
            recommended_next_steps,
        })
    }

    pub fn evaluate_domain_operational_gate(
        &self,
        input: DomainOperationalGateInput,
    ) -> Result<DomainOperationalGateReport> {
        let thresholds = domain_operational_gate_thresholds(&input);
        let scope = self.resolve_domain_operational_gate_scope(&input, thresholds.window_days)?;
        let summary = self.domain_operational_summary(&scope)?;
        let campaign_failures = summary.failed_campaign_items
            + summary.cancelled_campaign_items
            + summary.interrupted_campaign_items;
        let mut checks = Vec::new();
        push_operational_check(
            &mut checks,
            "workflow_sample",
            if summary.workflow_runs >= thresholds.min_workflow_runs {
                "passed"
            } else {
                "insufficient_data"
            },
            "blocking",
            format!("at least {} workflow run(s)", thresholds.min_workflow_runs),
            format!("{} workflow run(s)", summary.workflow_runs),
            "Operational readiness needs durable workflow evidence, not only eval or final text.",
        );
        push_operational_check(
            &mut checks,
            "workflow_failures",
            if summary.failed_workflow_runs <= thresholds.max_failed_workflow_runs
                && summary.blocked_workflow_runs <= thresholds.max_blocked_workflow_runs
                && summary.cancelled_workflow_runs <= thresholds.max_cancelled_workflow_runs
            {
                "passed"
            } else {
                "failed"
            },
            "blocking",
            format!(
                "<= {} failed, <= {} blocked, <= {} cancelled workflow run(s)",
                thresholds.max_failed_workflow_runs,
                thresholds.max_blocked_workflow_runs,
                thresholds.max_cancelled_workflow_runs
            ),
            format!(
                "{} failed, {} blocked, {} cancelled",
                summary.failed_workflow_runs,
                summary.blocked_workflow_runs,
                summary.cancelled_workflow_runs
            ),
            "Failed or blocked workflow runs must be repaired, retried, or allowed to age out before calling the scope operationally ready.",
        );
        push_operational_check(
            &mut checks,
            "workflow_active_drain",
            if summary.active_workflow_runs <= thresholds.max_active_workflow_runs {
                "passed"
            } else {
                "insufficient_data"
            },
            "blocking",
            format!(
                "<= {} active workflow run(s)",
                thresholds.max_active_workflow_runs
            ),
            format!(
                "{} active ({} paused, {} awaiting approval)",
                summary.active_workflow_runs,
                summary.paused_workflow_runs,
                summary.awaiting_approval_workflow_runs
            ),
            "Active workflow runs are observable, but an operational gate should wait for them to finish or be explicitly paused/cancelled.",
        );
        push_operational_check(
            &mut checks,
            "loop_sample",
            if summary.loop_runs >= thresholds.min_loop_runs {
                "passed"
            } else {
                "insufficient_data"
            },
            "advisory",
            format!("at least {} loop run(s)", thresholds.min_loop_runs),
            format!("{} loop run(s)", summary.loop_runs),
            "Loop evidence is optional by default, but raises confidence for recurring long tasks.",
        );
        push_operational_check(
            &mut checks,
            "loop_failures",
            if summary.failed_loop_runs <= thresholds.max_failed_loop_runs {
                "passed"
            } else {
                "failed"
            },
            "blocking",
            format!("<= {} failed loop run(s)", thresholds.max_failed_loop_runs),
            format!(
                "{} failed, {} active, {} schedule(s) active",
                summary.failed_loop_runs, summary.active_loop_runs, summary.active_loop_schedules
            ),
            "Recurring loops should not accumulate failed ticks without user-visible recovery.",
        );
        push_operational_check(
            &mut checks,
            "campaign_active_drain",
            if summary.active_campaigns <= thresholds.max_active_campaigns {
                "passed"
            } else {
                "insufficient_data"
            },
            "blocking",
            format!("<= {} active campaign(s)", thresholds.max_active_campaigns),
            format!("{} active campaign(s)", summary.active_campaigns),
            "Active campaigns prove observability, but final operational readiness should wait for terminal results.",
        );
        push_operational_check(
            &mut checks,
            "campaign_failures",
            if campaign_failures <= thresholds.max_failed_campaign_items {
                "passed"
            } else {
                "failed"
            },
            "blocking",
            format!(
                "<= {} failed/cancelled/interrupted campaign item(s)",
                thresholds.max_failed_campaign_items
            ),
            format!("{campaign_failures} failed/cancelled/interrupted item(s)"),
            "Campaign failures should be retried or turned into learning evidence before treating long-run behavior as stable.",
        );

        let blockers = checks
            .iter()
            .filter(|check| check.status != "passed" && check.severity != "advisory")
            .map(|check| check.name.clone())
            .collect::<Vec<_>>();
        let status = operational_status(&checks);
        let recommended_next_steps = domain_operational_recommendations(&checks);

        Ok(DomainOperationalGateReport {
            generated_at: now_rfc3339(),
            status,
            scope: scope.scope,
            session_id: scope.session_id,
            project_id: scope.project_id,
            domain: scope.domain,
            since: scope.since,
            thresholds,
            summary,
            checks,
            blockers,
            recommended_next_steps,
        })
    }

    pub fn generate_domain_soak_report(
        &self,
        input: DomainSoakReportInput,
    ) -> Result<DomainSoakReport> {
        let window_days = input
            .window_days
            .unwrap_or(DEFAULT_DOMAIN_SOAK_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS);
        let max_items = input
            .max_items
            .unwrap_or(DEFAULT_DOMAIN_SOAK_MAX_ITEMS)
            .clamp(1, MAX_DOMAIN_SOAK_MAX_ITEMS);
        let operational_input = DomainOperationalGateInput {
            session_id: input.session_id.clone(),
            project_id: input.project_id.clone(),
            domain: input.domain.clone(),
            window_days: Some(window_days),
            min_workflow_runs: Some(1),
            max_failed_workflow_runs: Some(0),
            max_blocked_workflow_runs: Some(0),
            max_cancelled_workflow_runs: Some(0),
            max_active_workflow_runs: Some(0),
            min_loop_runs: Some(0),
            max_failed_loop_runs: Some(0),
            max_active_campaigns: Some(0),
            max_failed_campaign_items: Some(0),
        };
        let scope = self.resolve_domain_operational_gate_scope(&operational_input, window_days)?;
        let operational_gate = self.evaluate_domain_operational_gate(operational_input)?;
        let until = now_rfc3339();
        let mut summary = DomainSoakReportSummary {
            required_sample_days: if window_days > 1 { 2 } else { 1 },
            ..Default::default()
        };
        let mut sample_days = BTreeSet::new();
        let mut incidents = Vec::new();
        let mut timeline = Vec::new();

        let workflow_rows = self.domain_soak_workflows(&scope)?;
        let mut workflow_durations = Vec::new();
        for row in workflow_rows {
            track_soak_sample_day(&mut sample_days, &row.created_at);
            summary.workflow_runs += 1;
            if row
                .origin
                .as_deref()
                .is_some_and(|origin| origin.contains("repair"))
            {
                summary.repair_workflow_runs += 1;
            }
            let duration = row
                .completed_at
                .as_deref()
                .and_then(|finished| timestamp_delta_secs(&row.created_at, finished));
            if let Some(duration) = duration {
                workflow_durations.push(duration);
            }
            match row.state.as_str() {
                "completed" => summary.completed_workflow_runs += 1,
                "failed" => {
                    summary.failed_workflow_runs += 1;
                    push_soak_incident(
                        &mut incidents,
                        "workflow",
                        &row.id,
                        &row.kind,
                        &row.state,
                        "critical",
                        Some(row.created_at.clone()),
                        row.completed_at.clone().or_else(|| Some(row.updated_at.clone())),
                        duration,
                        row.blocked_reason
                            .clone()
                            .unwrap_or_else(|| "workflow failed".to_string()),
                        "Open the Workflow run detail, inspect failed ops, and retry through a repair workflow.".to_string(),
                    );
                }
                "blocked" => {
                    summary.blocked_workflow_runs += 1;
                    push_soak_incident(
                        &mut incidents,
                        "workflow",
                        &row.id,
                        &row.kind,
                        &row.state,
                        "critical",
                        Some(row.created_at.clone()),
                        row.completed_at.clone().or_else(|| Some(row.updated_at.clone())),
                        duration,
                        row.blocked_reason
                            .clone()
                            .unwrap_or_else(|| "workflow blocked".to_string()),
                        "Resolve the blocker or explicitly cancel/restart the workflow before unattended continuation.".to_string(),
                    );
                }
                "cancelled" => {
                    summary.cancelled_workflow_runs += 1;
                    push_soak_incident(
                        &mut incidents,
                        "workflow",
                        &row.id,
                        &row.kind,
                        &row.state,
                        "critical",
                        Some(row.created_at.clone()),
                        row.completed_at.clone().or_else(|| Some(row.updated_at.clone())),
                        duration,
                        "workflow was cancelled".to_string(),
                        "Confirm cancellation was intentional or retry with a smaller bounded workflow.".to_string(),
                    );
                }
                "awaiting_approval" => {
                    summary.active_workflow_runs += 1;
                    summary.awaiting_approval_workflow_runs += 1;
                    push_soak_incident(
                        &mut incidents,
                        "workflow",
                        &row.id,
                        &row.kind,
                        &row.state,
                        "warning",
                        Some(row.created_at.clone()),
                        None,
                        timestamp_delta_secs(&row.created_at, &until),
                        "workflow is waiting for approval".to_string(),
                        "Approve, deny, pause, or cancel the workflow so long-running work can drain.".to_string(),
                    );
                }
                "running" | "recovering" | "awaiting_user" | "paused" | "draft" => {
                    summary.active_workflow_runs += 1;
                    push_soak_incident(
                        &mut incidents,
                        "workflow",
                        &row.id,
                        &row.kind,
                        &row.state,
                        "warning",
                        Some(row.created_at.clone()),
                        None,
                        timestamp_delta_secs(&row.created_at, &until),
                        format!("workflow is {}", row.state),
                        "Let the workflow finish or explicitly pause/cancel stale active work."
                            .to_string(),
                    );
                }
                _ => {}
            }
            timeline.push(DomainSoakTimelineItem {
                source: "workflow".to_string(),
                id: row.id,
                label: row.kind,
                status: row.state,
                at: row.completed_at.unwrap_or(row.updated_at),
                duration_secs: duration,
            });
            if let Some(item) = timeline.last() {
                track_soak_activity(&mut summary, &mut sample_days, &item.at);
            }
        }
        summary.average_workflow_drain_secs = average_secs(&workflow_durations);
        summary.max_workflow_drain_secs = workflow_durations.iter().copied().max();

        let mut approval_wait_started: BTreeMap<String, String> = BTreeMap::new();
        let mut approval_wait_durations = Vec::new();
        for event in self.domain_soak_workflow_events(&scope)? {
            track_soak_activity(&mut summary, &mut sample_days, &event.created_at);
            if event.event_type == "run_control_action" {
                match json_string_value(&event.payload, "action").as_deref() {
                    Some("approve") => {
                        summary.approval_events += 1;
                        summary.workflow_control_intervention_events += 1;
                    }
                    Some("pause") => {
                        summary.pause_events += 1;
                        summary.workflow_control_intervention_events += 1;
                    }
                    Some("resume") => {
                        summary.resume_events += 1;
                        summary.workflow_control_intervention_events += 1;
                    }
                    Some("cancel") => {
                        summary.cancel_events += 1;
                        summary.workflow_control_intervention_events += 1;
                    }
                    _ => {}
                }
            }
            if event.event_type == "run_recovery_claimed"
                || event
                    .payload
                    .get("to")
                    .and_then(Value::as_str)
                    .is_some_and(|state| state == "recovering")
            {
                summary.recovery_events += 1;
            }
            if event.event_type == "budget_usage" {
                summary.workflow_budget_usage_events += 1;
                let spent = event
                    .payload
                    .get("spentOutputTokens")
                    .and_then(Value::as_u64);
                let limit = event.payload.get("maxOutputTokens").and_then(Value::as_u64);
                if event
                    .payload
                    .get("exhausted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    summary.workflow_budget_exhausted_events += 1;
                }
                if let Some(spent) = spent {
                    let replace = summary
                        .max_workflow_output_tokens_spent
                        .map_or(true, |current| spent > current);
                    if replace {
                        summary.max_workflow_output_tokens_spent = Some(spent);
                        summary.max_workflow_output_token_budget = limit;
                    }
                }
            }
            if event.event_type == "run_state_changed"
                && event
                    .payload
                    .get("to")
                    .and_then(Value::as_str)
                    .is_some_and(|state| state == "awaiting_approval")
            {
                summary.approval_events += 1;
                summary.approval_request_events += 1;
                approval_wait_started.insert(event.run_id.clone(), event.created_at.clone());
            }
            if event.event_type == "run_state_changed"
                && event
                    .payload
                    .get("from")
                    .and_then(Value::as_str)
                    .is_some_and(|state| state == "awaiting_approval")
                && event
                    .payload
                    .get("to")
                    .and_then(Value::as_str)
                    .is_none_or(|state| state != "awaiting_approval")
            {
                summary.approval_decision_events += 1;
                if let Some(started_at) = approval_wait_started.remove(&event.run_id) {
                    if let Some(duration) = timestamp_delta_secs(&started_at, &event.created_at) {
                        approval_wait_durations.push(duration);
                    }
                }
            }
        }
        summary.average_approval_wait_secs = average_secs(&approval_wait_durations);
        summary.max_approval_wait_secs = approval_wait_durations.iter().copied().max();
        let open_approval_wait_durations: Vec<i64> = approval_wait_started
            .values()
            .filter_map(|started_at| timestamp_delta_secs(started_at, &until))
            .collect();
        summary.open_approval_waits = open_approval_wait_durations.len();
        summary.max_open_approval_wait_secs = open_approval_wait_durations.iter().copied().max();

        let loop_rows = self.domain_soak_loop_runs(&scope)?;
        let mut loop_durations = Vec::new();
        for row in loop_rows {
            track_soak_sample_day(&mut sample_days, &row.started_at);
            summary.loop_runs += 1;
            let duration = row
                .finished_at
                .as_deref()
                .and_then(|finished| timestamp_delta_secs(&row.started_at, finished));
            if let Some(duration) = duration {
                loop_durations.push(duration);
            }
            match row.state.as_str() {
                "succeeded" | "empty" | "skipped" => summary.succeeded_loop_runs += 1,
                "failed" | "cancelled" => {
                    summary.failed_loop_runs += 1;
                    push_soak_incident(
                        &mut incidents,
                        "loop",
                        &row.id,
                        &row.trigger_reason,
                        &row.state,
                        "critical",
                        Some(row.started_at.clone()),
                        row.finished_at.clone(),
                        duration,
                        row.error
                            .clone()
                            .or(row.result_summary.clone())
                            .unwrap_or_else(|| "loop run failed".to_string()),
                        "Inspect the loop schedule and update its strategy before continuing unattended.".to_string(),
                    );
                }
                "running" | "queued" | "injected" => {
                    summary.active_loop_runs += 1;
                    push_soak_incident(
                        &mut incidents,
                        "loop",
                        &row.id,
                        &row.trigger_reason,
                        &row.state,
                        "warning",
                        Some(row.started_at.clone()),
                        None,
                        timestamp_delta_secs(&row.started_at, &until),
                        format!("loop run is {}", row.state),
                        "Wait for the tick to finish or stop the loop if it is stale.".to_string(),
                    );
                }
                _ => {}
            }
            timeline.push(DomainSoakTimelineItem {
                source: "loop".to_string(),
                id: row.id,
                label: format!("{}: {}", row.loop_id, row.trigger_reason),
                status: row.state,
                at: row.finished_at.unwrap_or(row.started_at),
                duration_secs: duration,
            });
            if let Some(item) = timeline.last() {
                track_soak_activity(&mut summary, &mut sample_days, &item.at);
            }
        }
        summary.average_loop_duration_secs = average_secs(&loop_durations);
        summary.max_loop_duration_secs = loop_durations.iter().copied().max();

        let campaign_rows = self.domain_soak_campaign_rows(&scope)?;
        let mut campaign_ids = BTreeSet::new();
        let mut item_ids = BTreeSet::new();
        let mut campaign_item_durations = Vec::new();
        for row in campaign_rows {
            if campaign_ids.insert(row.campaign_id.clone()) {
                summary.campaigns += 1;
                if matches!(
                    row.campaign_status.as_str(),
                    "queued" | "running" | "cancel_requested"
                ) {
                    summary.active_campaigns += 1;
                }
                timeline.push(DomainSoakTimelineItem {
                    source: "campaign".to_string(),
                    id: row.campaign_id.clone(),
                    label: row.campaign_name.clone(),
                    status: row.campaign_status.clone(),
                    at: row.campaign_updated_at.clone(),
                    duration_secs: None,
                });
                track_soak_activity(&mut summary, &mut sample_days, &row.campaign_updated_at);
            }
            let Some(item_id) = row.item_id.clone() else {
                continue;
            };
            if !item_ids.insert(item_id.clone()) {
                continue;
            }
            if let Some(started_at) = row.item_started_at.as_deref() {
                track_soak_sample_day(&mut sample_days, started_at);
            }
            summary.campaign_items += 1;
            let item_status = row.item_status.clone().unwrap_or_default();
            let duration = row
                .item_started_at
                .as_deref()
                .zip(row.item_finished_at.as_deref())
                .and_then(|(started, finished)| timestamp_delta_secs(started, finished));
            if let Some(duration) = duration {
                campaign_item_durations.push(duration);
            }
            if row.item_attempt.unwrap_or(0) > 1 {
                summary.retried_campaign_items += 1;
            }
            match item_status.as_str() {
                "passed" => summary.passed_campaign_items += 1,
                "failed" | "partial" => {
                    summary.failed_campaign_items += 1;
                    push_soak_incident(
                        &mut incidents,
                        "campaign_item",
                        &item_id,
                        row.item_title.as_deref().unwrap_or(&row.campaign_name),
                        &item_status,
                        "critical",
                        row.item_started_at.clone(),
                        row.item_finished_at.clone(),
                        duration,
                        row.item_error
                            .clone()
                            .unwrap_or_else(|| "campaign item failed".to_string()),
                        "Retry the failed campaign item or convert the failure into a learning proposal.".to_string(),
                    );
                }
                "cancelled" => {
                    summary.cancelled_campaign_items += 1;
                    push_soak_incident(
                        &mut incidents,
                        "campaign_item",
                        &item_id,
                        row.item_title.as_deref().unwrap_or(&row.campaign_name),
                        &item_status,
                        "critical",
                        row.item_started_at.clone(),
                        row.item_finished_at.clone(),
                        duration,
                        "campaign item was cancelled".to_string(),
                        "Confirm cancellation was intentional or retry the item when resources are available.".to_string(),
                    );
                }
                "interrupted" => {
                    summary.interrupted_campaign_items += 1;
                    push_soak_incident(
                        &mut incidents,
                        "campaign_item",
                        &item_id,
                        row.item_title.as_deref().unwrap_or(&row.campaign_name),
                        &item_status,
                        "critical",
                        row.item_started_at.clone(),
                        row.item_finished_at.clone(),
                        duration,
                        row.item_error
                            .clone()
                            .unwrap_or_else(|| "campaign item interrupted".to_string()),
                        "Retry interrupted items after checking provider/runtime availability."
                            .to_string(),
                    );
                }
                "queued" | "running" => {
                    push_soak_incident(
                        &mut incidents,
                        "campaign_item",
                        &item_id,
                        row.item_title.as_deref().unwrap_or(&row.campaign_name),
                        &item_status,
                        "warning",
                        row.item_started_at.clone().or(row.item_updated_at.clone()),
                        None,
                        row.item_started_at
                            .as_deref()
                            .and_then(|started| timestamp_delta_secs(started, &until)),
                        format!("campaign item is {item_status}"),
                        "Wait for active campaign items to finish or cancel stale campaigns."
                            .to_string(),
                    );
                }
                _ => {}
            }
            timeline.push(DomainSoakTimelineItem {
                source: "campaign_item".to_string(),
                id: item_id,
                label: row.item_title.unwrap_or(row.campaign_name),
                status: item_status,
                at: row
                    .item_finished_at
                    .or(row.item_updated_at)
                    .or(row.item_started_at)
                    .unwrap_or(row.campaign_updated_at),
                duration_secs: duration,
            });
            if let Some(item) = timeline.last() {
                track_soak_activity(&mut summary, &mut sample_days, &item.at);
            }
        }
        summary.average_campaign_item_duration_secs = average_secs(&campaign_item_durations);
        summary.max_campaign_item_duration_secs = campaign_item_durations.iter().copied().max();

        let (
            connector_e2e,
            connector_execution,
            connector_verification,
            connector_latest,
            connector_sample_days,
        ) = self.domain_soak_connector_evidence_counts(&scope)?;
        summary.connector_e2e_evidence = connector_e2e;
        summary.connector_execution_evidence = connector_execution;
        summary.connector_verification_evidence = connector_verification;
        if let Some(connector_latest) = connector_latest.as_deref() {
            track_soak_activity(&mut summary, &mut sample_days, connector_latest);
        }
        sample_days.extend(connector_sample_days);
        if summary.connector_execution_evidence > 0 && summary.connector_verification_evidence == 0
        {
            push_soak_incident(
                &mut incidents,
                "connector_e2e",
                "connector_verification_missing",
                "Connector action verification missing",
                "unverified",
                "warning",
                connector_latest,
                None,
                None,
                format!(
                    "{} connector execution evidence item(s) have no post-action verification",
                    summary.connector_execution_evidence
                ),
                "Record connector_action_verified evidence by reading back the external system state before trusting unattended connector runs.".to_string(),
            );
        }
        summary.sample_days = sample_days.len();
        summary.latest_activity_age_secs = summary
            .latest_activity_at
            .as_deref()
            .and_then(|activity_at| timestamp_delta_secs(activity_at, &until));

        incidents.sort_by(|a, b| {
            incident_rank(&a.severity)
                .cmp(&incident_rank(&b.severity))
                .then_with(|| b.started_at.cmp(&a.started_at))
                .then_with(|| a.id.cmp(&b.id))
        });
        timeline.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.id.cmp(&b.id)));
        summary.incidents = incidents.len();
        summary.critical_incidents = incidents
            .iter()
            .filter(|incident| incident.severity == "critical")
            .count();
        summary.warning_incidents = incidents
            .iter()
            .filter(|incident| incident.severity == "warning")
            .count();
        if incidents.len() > max_items {
            incidents.truncate(max_items);
        }
        if timeline.len() > max_items {
            timeline.truncate(max_items);
        }
        summary.total_records = summary.workflow_runs
            + summary.loop_runs
            + summary.campaign_items
            + summary.connector_e2e_evidence;
        let stale_activity = summary
            .latest_activity_age_secs
            .is_some_and(|age| age > 24 * 60 * 60);
        let insufficient_day_coverage =
            summary.sample_days < summary.required_sample_days && summary.total_records > 0;
        let status = if summary.total_records == 0 {
            "insufficient_data"
        } else if summary.critical_incidents > 0 || operational_gate.status == "failed" {
            "failed"
        } else if summary.warning_incidents > 0
            || operational_gate.status == "insufficient_data"
            || stale_activity
            || insufficient_day_coverage
        {
            "insufficient_data"
        } else {
            "passed"
        }
        .to_string();
        let recommended_next_steps =
            domain_soak_recommendations(&summary, &incidents, &operational_gate);
        let markdown = render_domain_soak_markdown(
            &scope,
            &until,
            &status,
            &summary,
            &incidents,
            &recommended_next_steps,
        );

        Ok(DomainSoakReport {
            generated_at: until.clone(),
            status,
            scope: scope.scope,
            session_id: scope.session_id,
            project_id: scope.project_id,
            domain: scope.domain,
            window_days: scope.window_days,
            since: scope.since,
            until,
            summary,
            incidents,
            timeline,
            recommended_next_steps,
            markdown,
            operational_gate,
        })
    }

    fn domain_soak_workflows(&self, scope: &DomainGateScope) -> Result<Vec<SoakWorkflowRow>> {
        let mut clauses = vec![
            "wr.created_at >= ?".to_string(),
            "s.incognito = 0".to_string(),
        ];
        let mut params = vec![scope.since.clone()];
        if let Some(project_id) = scope.project_id.as_ref() {
            clauses.push("s.project_id = ?".to_string());
            params.push(project_id.clone());
        } else if let Some(session_id) = scope.session_id.as_ref() {
            clauses.push("wr.session_id = ?".to_string());
            params.push(session_id.clone());
        }
        if let Some(domain) = scope.domain.as_ref() {
            clauses.push("(wr.kind = ? OR g.domain = ?)".to_string());
            params.push(format!("domain:{domain}"));
            params.push(domain.clone());
        }
        let sql = format!(
            "SELECT wr.id, wr.kind, wr.state, wr.origin, wr.created_at, wr.updated_at,
                    wr.completed_at, wr.blocked_reason
             FROM workflow_runs wr
             JOIN sessions s ON s.id = wr.session_id
             LEFT JOIN goals g ON g.id = wr.goal_id
             WHERE {}
             ORDER BY wr.updated_at DESC, wr.id DESC",
            clauses.join(" AND ")
        );
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
            Ok(SoakWorkflowRow {
                id: row.get(0)?,
                kind: row.get(1)?,
                state: row.get(2)?,
                origin: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
                completed_at: row.get(6)?,
                blocked_reason: row.get(7)?,
            })
        })?;
        collect_rows(rows)
    }

    fn domain_soak_workflow_events(
        &self,
        scope: &DomainGateScope,
    ) -> Result<Vec<SoakWorkflowEventRow>> {
        let mut clauses = vec![
            "e.created_at >= ?".to_string(),
            "s.incognito = 0".to_string(),
        ];
        let mut params = vec![scope.since.clone()];
        if let Some(project_id) = scope.project_id.as_ref() {
            clauses.push("s.project_id = ?".to_string());
            params.push(project_id.clone());
        } else if let Some(session_id) = scope.session_id.as_ref() {
            clauses.push("wr.session_id = ?".to_string());
            params.push(session_id.clone());
        }
        if let Some(domain) = scope.domain.as_ref() {
            clauses.push("(wr.kind = ? OR g.domain = ?)".to_string());
            params.push(format!("domain:{domain}"));
            params.push(domain.clone());
        }
        let sql = format!(
            "SELECT e.run_id, e.type, e.payload_json, e.created_at
             FROM workflow_events e
             JOIN workflow_runs wr ON wr.id = e.run_id
             JOIN sessions s ON s.id = wr.session_id
             LEFT JOIN goals g ON g.id = wr.goal_id
             WHERE {}
             ORDER BY e.created_at ASC, e.id ASC
             LIMIT 5000",
            clauses.join(" AND ")
        );
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
            let payload_json: String = row.get(2)?;
            Ok(SoakWorkflowEventRow {
                run_id: row.get(0)?,
                event_type: row.get(1)?,
                payload: serde_json::from_str(&payload_json).unwrap_or_else(|_| json!({})),
                created_at: row.get(3)?,
            })
        })?;
        collect_rows(rows)
    }

    fn domain_soak_loop_runs(&self, scope: &DomainGateScope) -> Result<Vec<SoakLoopRunRow>> {
        let mut clauses = vec![
            "lr.started_at >= ?".to_string(),
            "s.incognito = 0".to_string(),
        ];
        let mut params = vec![scope.since.clone()];
        if let Some(project_id) = scope.project_id.as_ref() {
            clauses.push("s.project_id = ?".to_string());
            params.push(project_id.clone());
        } else if let Some(session_id) = scope.session_id.as_ref() {
            clauses.push("lr.session_id = ?".to_string());
            params.push(session_id.clone());
        }
        if let Some(domain) = scope.domain.as_ref() {
            clauses.push("g.domain = ?".to_string());
            params.push(domain.clone());
        }
        let sql = format!(
            "SELECT lr.id, lr.loop_id, lr.state, lr.trigger_reason, lr.result_summary,
                    lr.error, lr.started_at, lr.finished_at
             FROM loop_runs lr
             JOIN loop_schedules ls ON ls.id = lr.loop_id
             JOIN sessions s ON s.id = lr.session_id
             LEFT JOIN goals g ON g.id = ls.goal_id
             WHERE {}
             ORDER BY COALESCE(lr.finished_at, lr.started_at) DESC, lr.id DESC",
            clauses.join(" AND ")
        );
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
            Ok(SoakLoopRunRow {
                id: row.get(0)?,
                loop_id: row.get(1)?,
                state: row.get(2)?,
                trigger_reason: row.get(3)?,
                result_summary: row.get(4)?,
                error: row.get(5)?,
                started_at: row.get(6)?,
                finished_at: row.get(7)?,
            })
        })?;
        collect_rows(rows)
    }

    fn domain_soak_campaign_rows(&self, scope: &DomainGateScope) -> Result<Vec<SoakCampaignRow>> {
        let mut clauses = vec!["c.created_at >= ?".to_string()];
        let mut params = vec![scope.since.clone()];
        if let Some(project_id) = scope.project_id.as_ref() {
            clauses.push("c.project_id = ?".to_string());
            params.push(project_id.clone());
        } else if let Some(session_id) = scope.session_id.as_ref() {
            clauses.push("c.session_id = ?".to_string());
            params.push(session_id.clone());
        }
        if let Some(domain) = scope.domain.as_ref() {
            clauses.push("(c.domain = ? OR i.domain = ?)".to_string());
            params.push(domain.clone());
            params.push(domain.clone());
        }
        let sql = format!(
            "SELECT c.id, c.name, c.status, c.updated_at,
                    i.id, i.task_title, i.status, i.attempt, i.error,
                    i.started_at, i.finished_at, i.updated_at
             FROM domain_eval_campaigns c
             LEFT JOIN domain_eval_campaign_items i ON i.campaign_id = c.id
             WHERE {}
             ORDER BY c.updated_at DESC, c.id DESC, i.updated_at DESC",
            clauses.join(" AND ")
        );
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
            Ok(SoakCampaignRow {
                campaign_id: row.get(0)?,
                campaign_name: row.get(1)?,
                campaign_status: row.get(2)?,
                campaign_updated_at: row.get(3)?,
                item_id: row.get(4)?,
                item_title: row.get(5)?,
                item_status: row.get(6)?,
                item_attempt: row
                    .get::<_, Option<i64>>(7)?
                    .map(|value| value.max(0) as usize),
                item_error: row.get(8)?,
                item_started_at: row.get(9)?,
                item_finished_at: row.get(10)?,
                item_updated_at: row.get(11)?,
            })
        })?;
        collect_rows(rows)
    }

    fn domain_soak_connector_evidence_counts(
        &self,
        scope: &DomainGateScope,
    ) -> Result<(usize, usize, usize, Option<String>, Vec<String>)> {
        let mut clauses = vec![
            "de.created_at >= ?".to_string(),
            "s.incognito = 0".to_string(),
            "de.evidence_type IN ('connector_context_collected','connector_draft_created','connector_action_executed','connector_action_verified')".to_string(),
        ];
        let mut params = vec![scope.since.clone()];
        if let Some(project_id) = scope.project_id.as_ref() {
            clauses.push("s.project_id = ?".to_string());
            params.push(project_id.clone());
        } else if let Some(session_id) = scope.session_id.as_ref() {
            clauses.push("de.session_id = ?".to_string());
            params.push(session_id.clone());
        }
        if let Some(domain) = scope.domain.as_ref() {
            clauses.push("de.domain = ?".to_string());
            params.push(domain.clone());
        }
        let sql = format!(
            "SELECT
                COUNT(*),
                SUM(CASE WHEN de.evidence_type = 'connector_action_executed' THEN 1 ELSE 0 END),
                SUM(CASE WHEN de.evidence_type = 'connector_action_verified' THEN 1 ELSE 0 END),
                MAX(de.created_at),
                GROUP_CONCAT(de.created_at)
             FROM domain_evidence_items de
             JOIN sessions s ON s.id = de.session_id
             WHERE {}",
            clauses.join(" AND ")
        );
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let (total, executed, verified, latest_at, sample_days): (
            i64,
            Option<i64>,
            Option<i64>,
            Option<String>,
            Option<String>,
        ) = conn.query_row(&sql, params_from_iter(params.iter()), |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?;
        Ok((
            total.max(0) as usize,
            executed.unwrap_or(0).max(0) as usize,
            verified.unwrap_or(0).max(0) as usize,
            latest_at,
            sample_days
                .unwrap_or_default()
                .split(',')
                .filter_map(rfc3339_day_key)
                .collect(),
        ))
    }

    fn domain_operational_summary(
        &self,
        scope: &DomainGateScope,
    ) -> Result<DomainOperationalGateSummary> {
        let mut summary = DomainOperationalGateSummary::default();
        let now = now_rfc3339();
        self.fill_domain_operational_workflows(scope, &mut summary, &now)?;
        self.fill_domain_operational_loops(scope, &mut summary, &now)?;
        self.fill_domain_operational_campaigns(scope, &mut summary, &now)?;
        Ok(summary)
    }

    fn fill_domain_operational_workflows(
        &self,
        scope: &DomainGateScope,
        summary: &mut DomainOperationalGateSummary,
        now: &str,
    ) -> Result<()> {
        let mut clauses = vec![
            "wr.created_at >= ?".to_string(),
            "s.incognito = 0".to_string(),
        ];
        let mut params = vec![scope.since.clone()];
        if let Some(project_id) = scope.project_id.as_ref() {
            clauses.push("s.project_id = ?".to_string());
            params.push(project_id.clone());
        } else if let Some(session_id) = scope.session_id.as_ref() {
            clauses.push("wr.session_id = ?".to_string());
            params.push(session_id.clone());
        }
        if let Some(domain) = scope.domain.as_ref() {
            clauses.push("(wr.kind = ? OR g.domain = ?)".to_string());
            params.push(format!("domain:{domain}"));
            params.push(domain.clone());
        }
        let sql = format!(
            "SELECT wr.state, wr.updated_at, wr.created_at
             FROM workflow_runs wr
             JOIN sessions s ON s.id = wr.session_id
             LEFT JOIN goals g ON g.id = wr.goal_id
             WHERE {}",
            clauses.join(" AND ")
        );
        let rows = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (state, updated_at, created_at) in rows {
            summary.workflow_runs += 1;
            max_timestamp(&mut summary.latest_activity_at, updated_at);
            match state.as_str() {
                "completed" => summary.completed_workflow_runs += 1,
                "failed" => summary.failed_workflow_runs += 1,
                "blocked" => summary.blocked_workflow_runs += 1,
                "cancelled" => summary.cancelled_workflow_runs += 1,
                "paused" => {
                    summary.paused_workflow_runs += 1;
                    summary.active_workflow_runs += 1;
                    update_max_active_work_age(summary, &created_at, now);
                }
                "awaiting_approval" => {
                    summary.awaiting_approval_workflow_runs += 1;
                    summary.active_workflow_runs += 1;
                    update_max_active_work_age(summary, &created_at, now);
                }
                "running" | "recovering" | "awaiting_user" => {
                    summary.active_workflow_runs += 1;
                    update_max_active_work_age(summary, &created_at, now);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn fill_domain_operational_loops(
        &self,
        scope: &DomainGateScope,
        summary: &mut DomainOperationalGateSummary,
        now: &str,
    ) -> Result<()> {
        let mut schedule_clauses = vec![
            "ls.created_at >= ?".to_string(),
            "s.incognito = 0".to_string(),
        ];
        let mut schedule_params = vec![scope.since.clone()];
        if let Some(project_id) = scope.project_id.as_ref() {
            schedule_clauses.push("s.project_id = ?".to_string());
            schedule_params.push(project_id.clone());
        } else if let Some(session_id) = scope.session_id.as_ref() {
            schedule_clauses.push("ls.session_id = ?".to_string());
            schedule_params.push(session_id.clone());
        }
        if let Some(domain) = scope.domain.as_ref() {
            schedule_clauses.push("g.domain = ?".to_string());
            schedule_params.push(domain.clone());
        }
        let schedule_sql = format!(
            "SELECT ls.state, ls.updated_at
             FROM loop_schedules ls
             JOIN sessions s ON s.id = ls.session_id
             LEFT JOIN goals g ON g.id = ls.goal_id
             WHERE {}",
            schedule_clauses.join(" AND ")
        );
        let schedules = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let mut stmt = conn.prepare(&schedule_sql)?;
            let rows = stmt.query_map(params_from_iter(schedule_params.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (state, updated_at) in schedules {
            summary.loop_schedules += 1;
            max_timestamp(&mut summary.latest_activity_at, updated_at);
            if matches!(state.as_str(), "active" | "paused" | "blocked") {
                summary.active_loop_schedules += 1;
            }
        }

        let mut run_clauses = vec![
            "lr.started_at >= ?".to_string(),
            "s.incognito = 0".to_string(),
        ];
        let mut run_params = vec![scope.since.clone()];
        if let Some(project_id) = scope.project_id.as_ref() {
            run_clauses.push("s.project_id = ?".to_string());
            run_params.push(project_id.clone());
        } else if let Some(session_id) = scope.session_id.as_ref() {
            run_clauses.push("lr.session_id = ?".to_string());
            run_params.push(session_id.clone());
        }
        if let Some(domain) = scope.domain.as_ref() {
            run_clauses.push("g.domain = ?".to_string());
            run_params.push(domain.clone());
        }
        let run_sql = format!(
            "SELECT lr.state, COALESCE(lr.finished_at, lr.started_at), lr.started_at
             FROM loop_runs lr
             JOIN loop_schedules ls ON ls.id = lr.loop_id
             JOIN sessions s ON s.id = lr.session_id
             LEFT JOIN goals g ON g.id = ls.goal_id
             WHERE {}",
            run_clauses.join(" AND ")
        );
        let runs = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let mut stmt = conn.prepare(&run_sql)?;
            let rows = stmt.query_map(params_from_iter(run_params.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (state, activity_at, started_at) in runs {
            summary.loop_runs += 1;
            max_timestamp(&mut summary.latest_activity_at, activity_at);
            match state.as_str() {
                "succeeded" => summary.succeeded_loop_runs += 1,
                "failed" | "cancelled" => summary.failed_loop_runs += 1,
                "running" | "queued" | "injected" => {
                    summary.active_loop_runs += 1;
                    update_max_active_work_age(summary, &started_at, now);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn fill_domain_operational_campaigns(
        &self,
        scope: &DomainGateScope,
        summary: &mut DomainOperationalGateSummary,
        now: &str,
    ) -> Result<()> {
        let mut clauses = vec!["c.created_at >= ?".to_string()];
        let mut params = vec![scope.since.clone()];
        if let Some(project_id) = scope.project_id.as_ref() {
            clauses.push("c.project_id = ?".to_string());
            params.push(project_id.clone());
        } else if let Some(session_id) = scope.session_id.as_ref() {
            clauses.push("c.session_id = ?".to_string());
            params.push(session_id.clone());
        }
        if let Some(domain) = scope.domain.as_ref() {
            clauses.push("(c.domain = ? OR i.domain = ?)".to_string());
            params.push(domain.clone());
            params.push(domain.clone());
        }
        let sql = format!(
            "SELECT c.id, c.status, c.updated_at, c.created_at, i.id, i.status
             FROM domain_eval_campaigns c
             LEFT JOIN domain_eval_campaign_items i ON i.campaign_id = c.id
             WHERE {}",
            clauses.join(" AND ")
        );
        let rows = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut campaign_ids = BTreeSet::new();
        for (campaign_id, campaign_status, updated_at, created_at, item_id, item_status) in rows {
            if campaign_ids.insert(campaign_id) {
                summary.campaigns += 1;
                max_timestamp(&mut summary.latest_activity_at, updated_at);
                if matches!(
                    campaign_status.as_str(),
                    "queued" | "running" | "cancel_requested"
                ) {
                    summary.active_campaigns += 1;
                    update_max_active_work_age(summary, &created_at, now);
                }
            }
            let Some(item_status) = item_status else {
                continue;
            };
            if item_id.is_some() {
                summary.campaign_items += 1;
            }
            match item_status.as_str() {
                "passed" => summary.passed_campaign_items += 1,
                "failed" => summary.failed_campaign_items += 1,
                "cancelled" => summary.cancelled_campaign_items += 1,
                "interrupted" => summary.interrupted_campaign_items += 1,
                _ => {}
            }
        }
        Ok(())
    }

    fn domain_readiness_campaign_summary(
        &self,
        session_id: Option<&str>,
        project_id: Option<&str>,
        domain: Option<&str>,
        since: &str,
    ) -> Result<DomainReadinessGateSummary> {
        let mut clauses = vec!["c.created_at >= ?".to_string()];
        let mut params = vec![since.to_string()];
        if let Some(project_id) = project_id.and_then(non_empty) {
            clauses.push("c.project_id = ?".to_string());
            params.push(project_id.to_string());
        } else if let Some(session_id) = session_id.and_then(non_empty) {
            clauses.push("c.session_id = ?".to_string());
            params.push(session_id.to_string());
        }
        if let Some(domain) = domain.and_then(non_empty).map(normalize_domain) {
            clauses.push("(c.domain = ? OR i.domain = ?)".to_string());
            params.push(domain.clone());
            params.push(domain);
        }
        let sql = format!(
            "SELECT c.id, c.status, c.updated_at, i.id, i.status
             FROM domain_eval_campaigns c
             LEFT JOIN domain_eval_campaign_items i ON i.campaign_id = c.id
             WHERE {}
             ORDER BY c.updated_at DESC, c.id DESC, i.created_at ASC",
            clauses.join(" AND ")
        );
        let rows = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let mut summary = DomainReadinessGateSummary::default();
        let mut campaign_ids = BTreeSet::new();
        let mut failed_learning_campaign_ids = BTreeSet::new();
        for (campaign_id, campaign_status, updated_at, item_id, item_status) in rows {
            if campaign_ids.insert(campaign_id.clone()) {
                summary.campaigns += 1;
                if matches!(
                    campaign_status.as_str(),
                    "queued" | "running" | "cancel_requested"
                ) {
                    summary.active_campaigns += 1;
                } else if matches!(
                    campaign_status.as_str(),
                    "passed" | "failed" | "partial" | "cancelled" | "interrupted"
                ) {
                    summary.terminal_campaigns += 1;
                }
                if summary
                    .latest_campaign_at
                    .as_ref()
                    .map(|current| updated_at > *current)
                    .unwrap_or(true)
                {
                    summary.latest_campaign_at = Some(updated_at);
                }
            }
            if item_id.is_none() {
                continue;
            }
            summary.campaign_items += 1;
            match item_status.as_deref().unwrap_or_default() {
                "passed" => {
                    summary.passed_campaign_items += 1;
                    summary.terminal_campaign_items += 1;
                }
                "failed" => {
                    summary.failed_campaign_items += 1;
                    summary.terminal_campaign_items += 1;
                    failed_learning_campaign_ids.insert(campaign_id);
                }
                "cancelled" => {
                    summary.cancelled_campaign_items += 1;
                    summary.terminal_campaign_items += 1;
                    failed_learning_campaign_ids.insert(campaign_id);
                }
                "interrupted" => {
                    summary.interrupted_campaign_items += 1;
                    summary.terminal_campaign_items += 1;
                    failed_learning_campaign_ids.insert(campaign_id);
                }
                _ => {}
            }
        }
        let campaign_ids = campaign_ids.into_iter().collect::<Vec<_>>();
        let failed_learning_campaign_ids =
            failed_learning_campaign_ids.into_iter().collect::<Vec<_>>();
        summary.open_learning_proposals =
            self.count_open_domain_campaign_learning_proposals(&campaign_ids)?;
        summary.pending_learning_campaigns =
            self.count_pending_domain_campaign_learning_campaigns(&failed_learning_campaign_ids)?;
        Ok(summary)
    }

    fn count_open_domain_campaign_learning_proposals(
        &self,
        campaign_ids: &[String],
    ) -> Result<usize> {
        if campaign_ids.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut total = 0usize;
        for chunk in campaign_ids.chunks(500) {
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(", ");
            let count = conn.query_row(
                &format!(
                    "SELECT COUNT(*)
                     FROM coding_improvement_proposals
                     WHERE source_type = 'domain_eval_campaign'
                       AND status NOT IN ('promoted','rejected')
                       AND source_id IN ({placeholders})"
                ),
                params_from_iter(chunk.iter()),
                |row| row.get::<_, i64>(0),
            )?;
            total += count.max(0) as usize;
        }
        Ok(total)
    }

    fn count_pending_domain_campaign_learning_campaigns(
        &self,
        failed_campaign_ids: &[String],
    ) -> Result<usize> {
        if failed_campaign_ids.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut materialized = BTreeSet::new();
        for chunk in failed_campaign_ids.chunks(500) {
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(", ");
            let mut stmt = conn.prepare(&format!(
                "SELECT DISTINCT source_id
                 FROM coding_improvement_proposals
                 WHERE source_type = 'domain_eval_campaign'
                   AND source_id IN ({placeholders})"
            ))?;
            let rows = stmt.query_map(params_from_iter(chunk.iter()), |row| {
                row.get::<_, String>(0)
            })?;
            for row in rows {
                materialized.insert(row?);
            }
        }
        Ok(failed_campaign_ids
            .iter()
            .filter(|id| !materialized.contains(*id))
            .count())
    }

    fn get_domain_eval_run(&self, run_id: &str) -> Result<Option<DomainEvalRunRecord>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, session_id, project_id, task_id, task_version, domain, label,
                    status, score, source_type, report_json, source_quality_run_id, created_at
             FROM domain_eval_runs
             WHERE id = ?1",
            params![run_id],
            row_to_domain_eval_run,
        )
        .optional()
        .map_err(Into::into)
    }

    fn get_domain_eval_calibration(
        &self,
        calibration_id: &str,
    ) -> Result<Option<DomainEvalCalibrationRecord>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, task_id, task_version, domain, project_id, scope, reviewer,
                    verdict, note, source_run_id, created_at
             FROM domain_eval_calibrations
             WHERE id = ?1",
            params![calibration_id],
            row_to_domain_eval_calibration,
        )
        .optional()
        .map_err(Into::into)
    }

    fn resolve_domain_eval_task(&self, task_id: &str) -> Result<Option<DomainEvalTask>> {
        if let Some(task) = built_in_domain_eval_tasks()
            .into_iter()
            .find(|task| task.id == task_id)
        {
            return Ok(Some(task));
        }
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT task_json
             FROM domain_eval_tasks
             WHERE id = ?1 AND status = 'active'
             ORDER BY updated_at DESC
             LIMIT 1",
            params![task_id],
            |row| {
                let task_json: String = row.get(0)?;
                decode_domain_eval_task_json(task_json)
            },
        )
        .optional()
        .map_err(Into::into)
    }

    fn attach_domain_eval_calibrations(
        &self,
        tasks: &mut [DomainEvalTask],
        project_id: Option<&str>,
    ) -> Result<()> {
        for task in tasks {
            task.calibration
                .extend(self.list_domain_eval_calibrations_for_task(
                    &task.id,
                    &task.version,
                    project_id,
                )?);
        }
        Ok(())
    }

    fn list_domain_eval_calibrations_for_task(
        &self,
        task_id: &str,
        task_version: &str,
        project_id: Option<&str>,
    ) -> Result<Vec<DomainEvalCalibrationRecord>> {
        let mut clauses = vec!["task_id = ?".to_string(), "task_version = ?".to_string()];
        let mut params = vec![task_id.to_string(), task_version.to_string()];
        if let Some(project_id) = project_id.and_then(non_empty) {
            clauses.push("(project_id = ? OR project_id IS NULL)".to_string());
            params.push(project_id.to_string());
        } else {
            clauses.push("project_id IS NULL".to_string());
        }
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&format!(
            "SELECT id, task_id, task_version, domain, project_id, scope, reviewer,
                    verdict, note, source_run_id, created_at
             FROM domain_eval_calibrations
             WHERE {}
             ORDER BY created_at DESC
             LIMIT ?",
            clauses.join(" AND ")
        ))?;
        params.push(DEFAULT_DOMAIN_EVAL_LIMIT.to_string());
        let rows = stmt.query_map(
            params_from_iter(params.iter()),
            row_to_domain_eval_calibration,
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn list_imported_domain_eval_tasks(
        &self,
        domain: Option<&str>,
        limit: usize,
    ) -> Result<Vec<DomainEvalTask>> {
        let mut clauses = vec!["status = 'active'".to_string()];
        let mut params = Vec::new();
        if let Some(domain) = domain.and_then(non_empty) {
            clauses.push("json_extract(task_json, '$.domain') = ?".to_string());
            params.push(normalize_domain(domain));
        }
        params.push(limit.to_string());
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&format!(
            "SELECT task_json
             FROM domain_eval_tasks
             WHERE {}
             ORDER BY updated_at DESC
             LIMIT ?",
            clauses.join(" AND ")
        ))?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
            let task_json: String = row.get(0)?;
            decode_domain_eval_task_json(task_json)
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn resolve_eval_quality_snapshot(
        &self,
        session_id: &str,
        domain: &str,
        input: &RunDomainEvalTaskInput,
    ) -> Result<Option<DomainQualityRunSnapshot>> {
        if let Some(run_id) = input.source_quality_run_id.as_deref().and_then(non_empty) {
            let snapshot = self
                .domain_quality_run_snapshot(run_id, 60)?
                .ok_or_else(|| anyhow!("domain quality run not found: {run_id}"))?;
            if snapshot.run.session_id != session_id {
                bail!(
                    "domain quality run {} belongs to session {}",
                    snapshot.run.id,
                    snapshot.run.session_id
                );
            }
            return Ok(Some(snapshot));
        }
        let runs = self.list_domain_quality_runs_for_session(session_id, 20)?;
        for run in runs {
            if run.domain == domain {
                return self.domain_quality_run_snapshot(&run.id, 60);
            }
        }
        Ok(None)
    }

    fn build_domain_eval_report(
        &self,
        session_id: &str,
        task: &DomainEvalTask,
        quality: Option<&DomainQualityRunSnapshot>,
    ) -> Result<DomainEvalReport> {
        let evidence = self.list_domain_evidence(ListDomainEvidenceInput {
            session_id: Some(session_id.to_string()),
            domain: Some(task.domain.clone()),
            limit: Some(200),
            ..Default::default()
        })?;
        let latest_goal = self
            .active_goal_for_session(session_id)?
            .or_else(|| self.latest_goal_for_session(session_id).ok().flatten());
        let workflow_runs = latest_goal
            .as_ref()
            .map(|goal| goal.workflow_runs.len())
            .unwrap_or(0);
        let counts = evidence_counts_by_type(&evidence);
        let mut checks = Vec::new();
        let mut satisfied_required = 0usize;
        let mut missing_required = 0usize;
        for req in &task.required_evidence {
            let actual = counts.get(&req.evidence_type).copied().unwrap_or(0);
            let has_metadata = evidence_metadata_satisfied(&evidence, req);
            let passed = actual >= req.min_count && has_metadata;
            if req.required {
                if passed {
                    satisfied_required += 1;
                } else {
                    missing_required += 1;
                }
            }
            checks.push(DomainEvalCheck {
                name: req.evidence_type.clone(),
                category: "evidence_completeness".to_string(),
                status: if passed {
                    "passed"
                } else if req.required {
                    "failed"
                } else {
                    "insufficient_data"
                }
                .to_string(),
                weight: if req.required { 1.0 } else { 0.5 },
                score: if passed { 1.0 } else { 0.0 },
                expected: format!("{} item(s) with {:?}", req.min_count, req.metadata_keys),
                actual: format!("{actual} item(s)"),
                detail: req.title.clone(),
            });
        }
        checks.push(citation_quality_check(task, &evidence));
        checks.push(data_quality_check(task, &evidence));
        checks.push(approval_safety_check(task, &evidence, quality));
        checks.push(completion_criteria_check(latest_goal.as_ref(), quality));
        checks.push(DomainEvalCheck {
            name: "workflow_trace".to_string(),
            category: "workflow_trace".to_string(),
            status: if workflow_runs > 0 {
                "passed"
            } else {
                "insufficient_data"
            }
            .to_string(),
            weight: 0.5,
            score: if workflow_runs > 0 { 1.0 } else { 0.0 },
            expected: "at least one workflow run linked to the Goal".to_string(),
            actual: workflow_runs.to_string(),
            detail: "Domain eval reuses workflow trace when present; missing trace is visible but not hidden inside coding benchmark.".to_string(),
        });
        let score = weighted_score(&checks);
        let status = eval_status(&checks, score);
        let summary = DomainEvalSummary {
            required_evidence: task
                .required_evidence
                .iter()
                .filter(|req| req.required)
                .count(),
            satisfied_required_evidence: satisfied_required,
            missing_required_evidence: missing_required,
            total_evidence: evidence.len(),
            source_count: counts.get("source_cited").copied().unwrap_or(0),
            dated_source_count: dated_source_count(&evidence),
            data_quality_count: counts.get("data_quality_checked").copied().unwrap_or(0),
            user_decision_count: counts.get("user_decision").copied().unwrap_or(0)
                + counts.get("message_draft_approved").copied().unwrap_or(0),
            workflow_runs,
            quality_state: quality
                .map(|snapshot| snapshot.run.state.as_str().to_string())
                .unwrap_or_else(|| "missing".to_string()),
        };
        Ok(DomainEvalReport {
            task: task.clone(),
            status,
            score,
            summary,
            checks,
            evidence: json!({
                "counts": counts,
                "items": evidence.iter().take(20).collect::<Vec<_>>(),
            }),
            goal: latest_goal
                .as_ref()
                .map(|goal| {
                    json!({
                        "id": goal.goal.id,
                        "state": goal.goal.state,
                        "objective": goal.goal.objective,
                        "completionCriteria": goal.goal.completion_criteria,
                        "evidence": goal.evidence.len(),
                    })
                })
                .unwrap_or_else(|| json!({"missing": true})),
            quality: quality
                .map(|snapshot| {
                    json!({
                        "run": snapshot.run,
                        "checks": snapshot.checks,
                    })
                })
                .unwrap_or_else(|| json!({"missing": true})),
            workflow: json!({ "runs": workflow_runs }),
        })
    }

    fn resolve_domain_quality_gate_scope(
        &self,
        input: &DomainQualityGateInput,
    ) -> Result<DomainGateScope> {
        let window_days = input
            .window_days
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS);
        let since = since_timestamp(window_days);
        let domain = input
            .domain
            .as_deref()
            .and_then(non_empty)
            .map(normalize_domain);
        if let Some(session_id) = input.session_id.as_deref().and_then(non_empty) {
            let session = self
                .get_session(session_id)?
                .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
            if session.incognito {
                bail!("domain quality gate is disabled for incognito sessions");
            }
            return Ok(DomainGateScope {
                scope: "session".to_string(),
                session_id: Some(session.id),
                project_id: session.project_id,
                domain,
                window_days,
                since,
                include_synthetic: input.include_synthetic,
            });
        }
        if let Some(project_id) = input.project_id.as_deref().and_then(non_empty) {
            return Ok(DomainGateScope {
                scope: "project".to_string(),
                session_id: None,
                project_id: Some(project_id.to_string()),
                domain,
                window_days,
                since,
                include_synthetic: input.include_synthetic,
            });
        }
        Ok(DomainGateScope {
            scope: "global".to_string(),
            session_id: None,
            project_id: None,
            domain,
            window_days,
            since,
            include_synthetic: input.include_synthetic,
        })
    }

    fn resolve_domain_operational_gate_scope(
        &self,
        input: &DomainOperationalGateInput,
        window_days: u32,
    ) -> Result<DomainGateScope> {
        let since = since_timestamp(window_days);
        let domain = input
            .domain
            .as_deref()
            .and_then(non_empty)
            .map(normalize_domain);
        if let Some(session_id) = input.session_id.as_deref().and_then(non_empty) {
            let session = self
                .get_session(session_id)?
                .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
            if session.incognito {
                bail!("domain operational gate is disabled for incognito sessions");
            }
            return Ok(DomainGateScope {
                scope: "session".to_string(),
                session_id: Some(session.id),
                project_id: session.project_id,
                domain,
                window_days,
                since,
                include_synthetic: false,
            });
        }
        if let Some(project_id) = input.project_id.as_deref().and_then(non_empty) {
            return Ok(DomainGateScope {
                scope: "project".to_string(),
                session_id: None,
                project_id: Some(project_id.to_string()),
                domain,
                window_days,
                since,
                include_synthetic: false,
            });
        }
        Ok(DomainGateScope {
            scope: "global".to_string(),
            session_id: None,
            project_id: None,
            domain,
            window_days,
            since,
            include_synthetic: false,
        })
    }

    fn domain_quality_gate_summary(
        &self,
        scope: &DomainGateScope,
    ) -> Result<DomainQualityGateSummary> {
        let runs = self.list_domain_eval_runs(ListDomainEvalRunsInput {
            session_id: scope.session_id.clone(),
            project_id: scope.project_id.clone(),
            domain: scope.domain.clone(),
            window_days: Some(scope.window_days),
            limit: Some(MAX_DOMAIN_EVAL_LIMIT),
            include_synthetic: scope.include_synthetic,
            ..Default::default()
        })?;
        let mut summary = DomainQualityGateSummary {
            eval_runs: runs.len(),
            ..Default::default()
        };
        let mut score_sum = 0.0;
        let mut domains = BTreeSet::new();
        for run in runs {
            domains.insert(run.domain);
            score_sum += run.score;
            match run.status.as_str() {
                "passed" => summary.passed_eval_runs += 1,
                "failed" => summary.failed_eval_runs += 1,
                _ => summary.insufficient_eval_runs += 1,
            }
        }
        if summary.eval_runs > 0 {
            summary.pass_rate = Some(summary.passed_eval_runs as f64 / summary.eval_runs as f64);
            summary.average_score = Some(score_sum / summary.eval_runs as f64);
        }
        let quality_rows = self.domain_quality_gate_quality_rows(scope)?;
        for row in &quality_rows {
            domains.insert(row.domain.clone());
            summary.quality_runs += 1;
            match row.state.as_str() {
                "completed" => summary.completed_quality_runs += 1,
                "blocked" => summary.blocked_quality_runs += 1,
                "failed" => summary.failed_quality_runs += 1,
                "needs_user" => summary.needs_user_quality_runs += 1,
                _ => {}
            }
            summary.approval_blockers += row
                .checks
                .iter()
                .filter(|(check_type, status)| {
                    check_type == "approval"
                        && matches!(status.as_str(), "needs_user" | "failed" | "blocked")
                })
                .count();
        }
        summary.domains_covered = domains.len();
        let evidence_counts = self.domain_quality_gate_evidence_counts(scope)?;
        summary.evidence_items = evidence_counts.values().sum();
        summary.source_cited = evidence_counts.get("source_cited").copied().unwrap_or(0);
        summary.dated_sources = self.domain_quality_gate_dated_sources(scope)?;
        summary.data_quality_checked = evidence_counts
            .get("data_quality_checked")
            .copied()
            .unwrap_or(0);
        Ok(summary)
    }

    fn domain_quality_gate_quality_rows(
        &self,
        scope: &DomainGateScope,
    ) -> Result<Vec<QualityGateRow>> {
        let mut clauses = vec![
            "dqr.updated_at >= ?".to_string(),
            "s.incognito = 0".to_string(),
        ];
        let mut params = vec![scope.since.clone()];
        if !scope.include_synthetic {
            clauses.push("s.kind != 'eval_fixture'".to_string());
            clauses.push(
                "COALESCE(json_extract(dqr.stats_json, '$.sourceType'), 'live') NOT LIKE 'fixture_%'"
                    .to_string(),
            );
        }
        if let Some(session_id) = scope.session_id.as_deref() {
            clauses.push("dqr.session_id = ?".to_string());
            params.push(session_id.to_string());
        }
        if let Some(project_id) = scope.project_id.as_deref() {
            clauses.push("s.project_id = ?".to_string());
            params.push(project_id.to_string());
        }
        if let Some(domain) = scope.domain.as_deref() {
            clauses.push("dqr.domain = ?".to_string());
            params.push(domain.to_string());
        }
        let raw_rows = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let mut stmt = conn.prepare(&format!(
                "SELECT dqr.id, dqr.domain, dqr.state
                 FROM domain_quality_runs dqr
                 JOIN sessions s ON s.id = dqr.session_id
                 WHERE {}
                 ORDER BY dqr.updated_at DESC
                 LIMIT 200",
                clauses.join(" AND ")
            ))?;
            let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut out = Vec::new();
        for (run_id, domain, state) in raw_rows {
            let checks = self
                .list_domain_quality_checks_for_run(&run_id)?
                .into_iter()
                .map(|check| (check.check_type, check.status.as_str().to_string()))
                .collect();
            out.push(QualityGateRow {
                state,
                domain,
                checks,
            });
        }
        Ok(out)
    }

    fn domain_quality_gate_evidence_counts(
        &self,
        scope: &DomainGateScope,
    ) -> Result<BTreeMap<String, usize>> {
        let mut clauses = vec![
            "dei.created_at >= ?".to_string(),
            "s.incognito = 0".to_string(),
        ];
        let mut params = vec![scope.since.clone()];
        if !scope.include_synthetic {
            clauses.push("s.kind != 'eval_fixture'".to_string());
            clauses.push("dei.access_scope != 'fixture'".to_string());
        }
        if let Some(session_id) = scope.session_id.as_deref() {
            clauses.push("dei.session_id = ?".to_string());
            params.push(session_id.to_string());
        }
        if let Some(project_id) = scope.project_id.as_deref() {
            clauses.push("dei.project_id = ?".to_string());
            params.push(project_id.to_string());
        }
        if let Some(domain) = scope.domain.as_deref() {
            clauses.push("dei.domain = ?".to_string());
            params.push(domain.to_string());
        }
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&format!(
            "SELECT dei.evidence_type, COUNT(*)
             FROM domain_evidence_items dei
             JOIN sessions s ON s.id = dei.session_id
             WHERE {}
             GROUP BY dei.evidence_type",
            clauses.join(" AND ")
        ))?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
        })?;
        let mut out = BTreeMap::new();
        for row in rows {
            let (evidence_type, count) = row?;
            out.insert(evidence_type, count);
        }
        Ok(out)
    }

    fn domain_quality_gate_dated_sources(&self, scope: &DomainGateScope) -> Result<usize> {
        let mut clauses = vec![
            "dei.created_at >= ?".to_string(),
            "s.incognito = 0".to_string(),
            "dei.evidence_type = 'source_cited'".to_string(),
        ];
        let mut params = vec![scope.since.clone()];
        if !scope.include_synthetic {
            clauses.push("s.kind != 'eval_fixture'".to_string());
            clauses.push("dei.access_scope != 'fixture'".to_string());
        }
        if let Some(session_id) = scope.session_id.as_deref() {
            clauses.push("dei.session_id = ?".to_string());
            params.push(session_id.to_string());
        }
        if let Some(project_id) = scope.project_id.as_deref() {
            clauses.push("dei.project_id = ?".to_string());
            params.push(project_id.to_string());
        }
        if let Some(domain) = scope.domain.as_deref() {
            clauses.push("dei.domain = ?".to_string());
            params.push(domain.to_string());
        }
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&format!(
            "SELECT dei.source_metadata_json
             FROM domain_evidence_items dei
             JOIN sessions s ON s.id = dei.session_id
             WHERE {}",
            clauses.join(" AND ")
        ))?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
            row.get::<_, String>(0)
        })?;
        let mut count = 0usize;
        for row in rows {
            let metadata: Value = serde_json::from_str(&row?).unwrap_or_else(|_| json!({}));
            if has_any_metadata(&metadata, &["retrievedAt", "publishedAt", "date"]) {
                count += 1;
            }
        }
        Ok(count)
    }
}

fn row_to_domain_eval_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<DomainEvalRunRecord> {
    let report_json: String = row.get(10)?;
    let report = serde_json::from_str(&report_json).unwrap_or_else(|_| DomainEvalReport {
        task: placeholder_task(),
        status: "failed".to_string(),
        score: 0.0,
        summary: DomainEvalSummary::default(),
        checks: Vec::new(),
        evidence: json!({}),
        goal: json!({}),
        quality: json!({}),
        workflow: json!({}),
    });
    Ok(DomainEvalRunRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        project_id: row.get(2)?,
        task_id: row.get(3)?,
        task_version: row.get(4)?,
        domain: row.get(5)?,
        label: row.get(6)?,
        status: row.get(7)?,
        score: row.get(8)?,
        source_type: row
            .get::<_, Option<String>>(9)?
            .unwrap_or_else(|| DOMAIN_EVAL_SOURCE_LIVE.to_string()),
        report,
        source_quality_run_id: row.get(11)?,
        created_at: row.get(12)?,
    })
}

fn row_to_domain_eval_fixture_run(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<DomainEvalFixtureRunRecord> {
    let report_json: String = row.get(11)?;
    let mut report: DomainEvalFixtureReport =
        serde_json::from_str(&report_json).unwrap_or_else(|_| DomainEvalFixtureReport {
            fixture_run_id: row.get(0).ok(),
            name: row.get::<_, String>(1).unwrap_or_default(),
            execution_mode: row.get::<_, String>(2).unwrap_or_default(),
            source_type: row
                .get::<_, String>(3)
                .unwrap_or_else(|_| DOMAIN_EVAL_SOURCE_FIXTURE_TRACE.to_string()),
            status: row
                .get::<_, String>(4)
                .unwrap_or_else(|_| "failed".to_string()),
            passed: row.get::<_, i64>(5).unwrap_or(0) != 0,
            session_id: row.get::<_, String>(6).unwrap_or_default(),
            goal_id: row.get(7).ok().flatten(),
            workflow_run_id: row.get(8).ok().flatten(),
            quality_run_id: row.get(9).ok().flatten(),
            eval_run: None,
            execution: None,
            checks: Vec::new(),
            error: row.get(12).ok().flatten(),
        });
    let id: String = row.get(0)?;
    report.fixture_run_id = Some(id.clone());
    Ok(DomainEvalFixtureRunRecord {
        id,
        name: row.get(1)?,
        execution_mode: row.get(2)?,
        source_type: row.get(3)?,
        status: row.get(4)?,
        passed: row.get::<_, i64>(5)? != 0,
        session_id: row.get(6)?,
        goal_id: row.get(7)?,
        workflow_run_id: row.get(8)?,
        quality_run_id: row.get(9)?,
        eval_run_id: row.get(10)?,
        report,
        error: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

fn row_to_domain_eval_campaign(row: &rusqlite::Row<'_>) -> rusqlite::Result<DomainEvalCampaign> {
    let task_filter_json: String = row.get(6)?;
    let model_matrix_json: String = row.get(7)?;
    Ok(DomainEvalCampaign {
        id: row.get(0)?,
        session_id: row.get(1)?,
        project_id: row.get(2)?,
        name: row.get(3)?,
        status: row.get(4)?,
        domain: row.get(5)?,
        task_filter: serde_json::from_str(&task_filter_json).unwrap_or_else(|_| json!({})),
        model_matrix: serde_json::from_str(&model_matrix_json).unwrap_or_default(),
        execution_mode: row.get(8)?,
        max_budget_usd: row.get(9)?,
        timeout_secs: row
            .get::<_, Option<i64>>(10)?
            .map(|value| value.max(0) as u64),
        summary: DomainEvalCampaignSummary::default(),
        items: Vec::new(),
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        started_at: row.get(13)?,
        finished_at: row.get(14)?,
        error: row.get(15)?,
    })
}

fn row_to_domain_eval_campaign_item(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<DomainEvalCampaignItem> {
    Ok(DomainEvalCampaignItem {
        id: row.get(0)?,
        campaign_id: row.get(1)?,
        task_id: row.get(2)?,
        task_title: row.get(3)?,
        domain: row.get(4)?,
        execution_mode: row.get(5)?,
        provider_id: row.get(6)?,
        model_id: row.get(7)?,
        label: row.get(8)?,
        status: row.get(9)?,
        attempt: row.get::<_, i64>(10)?.max(0) as usize,
        fixture_run_id: row.get(11)?,
        eval_run_id: row.get(12)?,
        score: row.get(13)?,
        total_checks: row.get::<_, i64>(14)?.max(0) as usize,
        passed_checks: row.get::<_, i64>(15)?.max(0) as usize,
        failed_checks: row.get::<_, i64>(16)?.max(0) as usize,
        started_at: row.get(17)?,
        finished_at: row.get(18)?,
        error: row.get(19)?,
    })
}

fn row_to_domain_eval_calibration(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<DomainEvalCalibrationRecord> {
    Ok(DomainEvalCalibrationRecord {
        id: Some(row.get(0)?),
        task_id: Some(row.get(1)?),
        task_version: Some(row.get(2)?),
        domain: Some(row.get(3)?),
        project_id: row.get(4)?,
        scope: Some(row.get(5)?),
        reviewer: row.get(6)?,
        verdict: Some(row.get(7)?),
        note: row.get(8)?,
        source_run_id: row.get(9)?,
        calibrated_at: row.get(10)?,
    })
}

fn persist_domain_eval_fixture_report(
    db: &SessionDB,
    report: &mut DomainEvalFixtureReport,
) -> Result<()> {
    let id = report
        .fixture_run_id
        .clone()
        .unwrap_or_else(|| format!("defr_{}", uuid::Uuid::new_v4().simple()));
    report.fixture_run_id = Some(id.clone());
    let now = now_rfc3339();
    let eval_run_id = report.eval_run.as_ref().map(|run| run.id.clone());
    let report_json = serde_json::to_string(report)?;
    let conn = db.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
    conn.execute(
        "INSERT INTO domain_eval_fixture_runs (
            id, name, execution_mode, source_type, status, passed, session_id,
            goal_id, workflow_run_id, quality_run_id, eval_run_id, report_json,
            error, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)
         ON CONFLICT(id) DO UPDATE SET
            status = excluded.status,
            passed = excluded.passed,
            workflow_run_id = excluded.workflow_run_id,
            quality_run_id = excluded.quality_run_id,
            eval_run_id = excluded.eval_run_id,
            report_json = excluded.report_json,
            error = excluded.error,
            updated_at = excluded.updated_at",
        params![
            id,
            &report.name,
            &report.execution_mode,
            &report.source_type,
            &report.status,
            if report.passed { 1 } else { 0 },
            &report.session_id,
            &report.goal_id,
            &report.workflow_run_id,
            &report.quality_run_id,
            eval_run_id,
            report_json,
            &report.error,
            now,
        ],
    )?;
    Ok(())
}

pub async fn run_domain_eval_campaign(
    db: Arc<SessionDB>,
    input: RunDomainEvalCampaignInput,
) -> Result<DomainEvalCampaign> {
    let campaign_id = input.campaign_id.trim().to_string();
    if campaign_id.is_empty() {
        bail!("domain eval campaign id must not be empty");
    }
    let items = db.prepare_domain_eval_campaign_run(&campaign_id, input.retry_failed_only)?;
    for queued_item in items {
        if db.is_domain_eval_campaign_cancel_requested(&campaign_id)? {
            break;
        }
        let Some(item) = db.mark_domain_eval_campaign_item_running(&queued_item.id)? else {
            continue;
        };
        let campaign = db
            .get_domain_eval_campaign(&campaign_id)?
            .ok_or_else(|| anyhow!("domain eval campaign not found: {campaign_id}"))?;
        let task = db
            .resolve_domain_eval_task(&item.task_id)?
            .ok_or_else(|| anyhow!("domain eval task not found: {}", item.task_id))?;
        let label = domain_eval_campaign_item_label(&campaign, &item);
        let mut fixture = domain_eval_campaign_fixture(&task, &item.execution_mode, &label);
        if item.execution_mode == "agent" {
            let Some(provider_id) = item.provider_id.clone() else {
                db.fail_domain_eval_campaign_item(
                    &item.id,
                    "agent campaign item is missing providerId",
                )?;
                continue;
            };
            let Some(model_id) = item.model_id.clone() else {
                db.fail_domain_eval_campaign_item(
                    &item.id,
                    "agent campaign item is missing modelId",
                )?;
                continue;
            };
            let Some(provider_config) =
                domain_campaign_provider_config(&provider_id, &input.providers)
            else {
                db.fail_domain_eval_campaign_item(
                    &item.id,
                    &format!(
                        "Provider config for {provider_id} was not supplied or is masked; campaign history never stores provider secrets"
                    ),
                )?;
                continue;
            };
            fixture.execution.providers = vec![provider_config];
            fixture.execution.model_chain = vec![ActiveModel {
                provider_id,
                model_id,
            }];
        }

        match SessionDB::run_domain_eval_fixture(db.clone(), RunDomainEvalFixtureInput { fixture })
            .await
        {
            Ok(report) => {
                db.finish_domain_eval_campaign_item(&item.id, &report)?;
            }
            Err(err) => {
                db.fail_domain_eval_campaign_item(&item.id, &err.to_string())?;
            }
        }
    }
    db.complete_domain_eval_campaign(&campaign_id)?;
    db.get_domain_eval_campaign(&campaign_id)?
        .ok_or_else(|| anyhow!("domain eval campaign not found after run: {campaign_id}"))
}

fn domain_eval_campaign_summary(items: &[DomainEvalCampaignItem]) -> DomainEvalCampaignSummary {
    let mut summary = DomainEvalCampaignSummary {
        total_items: items.len(),
        ..Default::default()
    };
    let mut score_sum = 0.0;
    let mut score_count = 0usize;
    for item in items {
        match item.status.as_str() {
            "queued" => summary.queued_items += 1,
            "running" => summary.running_items += 1,
            "passed" => summary.passed_items += 1,
            "failed" => summary.failed_items += 1,
            "cancelled" => summary.cancelled_items += 1,
            "interrupted" => summary.interrupted_items += 1,
            _ => {}
        }
        if item.eval_run_id.is_some() {
            summary.eval_runs += 1;
            match item.status.as_str() {
                "passed" => summary.passed_eval_runs += 1,
                "failed" => summary.failed_eval_runs += 1,
                "insufficient_data" => summary.insufficient_eval_runs += 1,
                _ => {}
            }
        }
        if let Some(score) = item.score {
            score_sum += score;
            score_count += 1;
        }
        summary.total_checks += item.total_checks;
        summary.passed_checks += item.passed_checks;
        summary.failed_checks += item.failed_checks;
    }
    summary.item_pass_rate = ratio(
        summary.passed_items,
        summary.passed_items + summary.failed_items,
    );
    summary.average_score =
        (score_count > 0).then_some(((score_sum / score_count as f64) * 1000.0).round() / 1000.0);
    summary
}

#[derive(Debug, Clone)]
struct DomainLeaderboardItemRow {
    campaign_id: String,
    campaign_name: String,
    item_id: String,
    task_id: String,
    domain: String,
    execution_mode: String,
    provider_id: Option<String>,
    model_id: Option<String>,
    label: Option<String>,
    status: String,
    attempt: usize,
    eval_run_id: Option<String>,
    score: Option<f64>,
    total_checks: usize,
    failed_checks: usize,
    updated_at: String,
    error: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct DomainLeaderboardKey {
    provider_id: Option<String>,
    model_id: Option<String>,
    label: String,
    execution_mode: String,
}

impl From<&DomainLeaderboardItemRow> for DomainLeaderboardKey {
    fn from(row: &DomainLeaderboardItemRow) -> Self {
        Self {
            provider_id: row.provider_id.clone(),
            model_id: row.model_id.clone(),
            label: row.label.clone().unwrap_or_else(|| {
                row.provider_id
                    .as_ref()
                    .zip(row.model_id.as_ref())
                    .map(|(provider_id, model_id)| format!("{provider_id}/{model_id}"))
                    .unwrap_or_else(|| row.execution_mode.clone())
            }),
            execution_mode: row.execution_mode.clone(),
        }
    }
}

#[derive(Default)]
struct DomainLeaderboardAccumulator {
    campaigns: BTreeSet<String>,
    domains: BTreeSet<String>,
    items: usize,
    passed_items: usize,
    failed_items: usize,
    cancelled_items: usize,
    interrupted_items: usize,
    attempts: usize,
    eval_runs: usize,
    score_sum: f64,
    score_count: usize,
    total_checks: usize,
    failed_checks: usize,
    evidence: Vec<DomainEvalCampaignLeaderboardEvidence>,
}

impl DomainLeaderboardAccumulator {
    fn add(&mut self, row: DomainLeaderboardItemRow) {
        self.campaigns.insert(row.campaign_id.clone());
        self.domains.insert(row.domain.clone());
        self.items += 1;
        match row.status.as_str() {
            "passed" => self.passed_items += 1,
            "failed" => self.failed_items += 1,
            "cancelled" => self.cancelled_items += 1,
            "interrupted" => self.interrupted_items += 1,
            _ => {}
        }
        self.attempts += row.attempt;
        if row.eval_run_id.is_some() {
            self.eval_runs += 1;
        }
        if let Some(score) = row.score {
            self.score_sum += score;
            self.score_count += 1;
        }
        self.total_checks += row.total_checks;
        self.failed_checks += row.failed_checks;
        if self.evidence.len() < 8 {
            self.evidence.push(DomainEvalCampaignLeaderboardEvidence {
                campaign_id: row.campaign_id,
                campaign_name: row.campaign_name,
                item_id: row.item_id,
                task_id: row.task_id,
                domain: row.domain,
                execution_mode: row.execution_mode,
                provider_id: row.provider_id,
                model_id: row.model_id,
                label: row.label,
                status: row.status,
                score: row.score,
                updated_at: row.updated_at,
                error: row.error,
            });
        }
    }

    fn into_row(self, key: DomainLeaderboardKey) -> DomainEvalCampaignLeaderboardRow {
        let mut warnings = Vec::new();
        if self.failed_items > 0 {
            warnings.push(format!("{} failed item(s)", self.failed_items));
        }
        if self.cancelled_items > 0 {
            warnings.push(format!("{} cancelled item(s)", self.cancelled_items));
        }
        if self.interrupted_items > 0 {
            warnings.push(format!("{} interrupted item(s)", self.interrupted_items));
        }
        if self.eval_runs == 0 {
            warnings.push("no eval run evidence".to_string());
        }
        DomainEvalCampaignLeaderboardRow {
            rank: 0,
            label: key.label,
            provider_id: key.provider_id,
            model_id: key.model_id,
            execution_mode: key.execution_mode,
            campaigns: self.campaigns.len(),
            items: self.items,
            passed_items: self.passed_items,
            failed_items: self.failed_items,
            cancelled_items: self.cancelled_items,
            interrupted_items: self.interrupted_items,
            attempts: self.attempts,
            eval_runs: self.eval_runs,
            item_pass_rate: ratio(self.passed_items, self.passed_items + self.failed_items),
            average_score: (self.score_count > 0)
                .then_some(((self.score_sum / self.score_count as f64) * 1000.0).round() / 1000.0),
            total_checks: self.total_checks,
            failed_checks: self.failed_checks,
            domains: self.domains.into_iter().collect(),
            warnings,
            evidence: self.evidence,
        }
    }
}

fn compare_domain_leaderboard_rows(
    left: &DomainEvalCampaignLeaderboardRow,
    right: &DomainEvalCampaignLeaderboardRow,
) -> std::cmp::Ordering {
    right
        .item_pass_rate
        .partial_cmp(&left.item_pass_rate)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| {
            right
                .average_score
                .partial_cmp(&left.average_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| right.items.cmp(&left.items))
        .then_with(|| {
            let left_unhealthy = left.failed_items + left.cancelled_items + left.interrupted_items;
            let right_unhealthy =
                right.failed_items + right.cancelled_items + right.interrupted_items;
            left_unhealthy.cmp(&right_unhealthy)
        })
        .then_with(|| left.label.cmp(&right.label))
}

fn normalize_domain_eval_campaign_models(
    models: Vec<DomainEvalCampaignModel>,
) -> Result<Vec<DomainEvalCampaignModel>> {
    let mut out = Vec::new();
    for model in models {
        let provider_id = model.provider_id.as_deref().and_then(non_empty);
        let model_id = model.model_id.as_deref().and_then(non_empty);
        let label = model.label.as_deref().and_then(non_empty);
        if provider_id.is_some() != model_id.is_some() {
            bail!(
                "domain eval campaign external model entries require both providerId and modelId"
            );
        }
        out.push(DomainEvalCampaignModel {
            provider_id: provider_id.map(str::to_string),
            model_id: model_id.map(str::to_string),
            label: label.map(str::to_string),
            credential_profile_ref: None,
        });
    }
    if out.is_empty() {
        out.push(DomainEvalCampaignModel {
            provider_id: None,
            model_id: None,
            label: Some("trace fixture".to_string()),
            credential_profile_ref: None,
        });
    }
    if out.len() > MAX_DOMAIN_EVAL_CAMPAIGN_MODELS {
        bail!(
            "domain eval campaign model matrix too large: {} > {}",
            out.len(),
            MAX_DOMAIN_EVAL_CAMPAIGN_MODELS
        );
    }
    Ok(out)
}

fn normalize_campaign_execution_mode(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value.and_then(non_empty) else {
        return Ok(None);
    };
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "trace_fixture" | "agent" => Ok(Some(normalized)),
        _ => bail!("unsupported domain eval campaign execution mode: {value}"),
    }
}

fn domain_campaign_provider_config(
    provider_id: &str,
    supplied: &[ProviderConfig],
) -> Option<ProviderConfig> {
    supplied
        .iter()
        .find(|provider| {
            provider.id == provider_id && !crate::provider::is_masked_key(&provider.api_key)
        })
        .cloned()
        .or_else(|| {
            crate::config::cached_config()
                .providers
                .iter()
                .find(|provider| {
                    provider.id == provider_id && !crate::provider::is_masked_key(&provider.api_key)
                })
                .cloned()
        })
}

fn domain_eval_campaign_item_label(
    campaign: &DomainEvalCampaign,
    item: &DomainEvalCampaignItem,
) -> String {
    format!(
        "{} · {} · {}",
        campaign.name,
        item.task_id,
        item.label
            .clone()
            .or_else(|| {
                item.provider_id
                    .as_ref()
                    .zip(item.model_id.as_ref())
                    .map(|(provider_id, model_id)| format!("{provider_id}/{model_id}"))
            })
            .unwrap_or_else(|| item.execution_mode.clone())
    )
}

fn domain_eval_campaign_fixture(
    task: &DomainEvalTask,
    execution_mode: &str,
    label: &str,
) -> DomainEvalFixture {
    let mut evidence = Vec::new();
    for req in &task.required_evidence {
        let count = req.min_count.max(1);
        for index in 0..count {
            evidence.push(DomainEvalFixtureEvidence {
                evidence_type: req.evidence_type.clone(),
                title: format!("{} #{}", req.title, index + 1),
                summary: Some(format!(
                    "Synthetic campaign evidence for {} requirement {}.",
                    task.id, req.evidence_type
                )),
                source_metadata: campaign_evidence_metadata(task, req, index),
                confidence: Some(0.95),
            });
        }
    }
    if matches!(task.domain.as_str(), "research" | "knowledge_curation")
        && !evidence
            .iter()
            .any(|item| item.evidence_type == "source_cited")
    {
        evidence.push(DomainEvalFixtureEvidence {
            evidence_type: "source_cited".to_string(),
            title: "Synthetic cited source".to_string(),
            summary: Some("Synthetic campaign citation with date metadata.".to_string()),
            source_metadata: json!({
                "sourceType": DOMAIN_EVAL_SOURCE_CAMPAIGN,
                "url": "https://example.invalid/domain-eval-campaign",
                "retrievedAt": now_rfc3339(),
                "publishedAt": "2026-01-01",
            }),
            confidence: Some(0.95),
        });
    }
    if task.domain == "data_analysis"
        && !evidence
            .iter()
            .any(|item| item.evidence_type == "data_quality_checked")
    {
        evidence.push(DomainEvalFixtureEvidence {
            evidence_type: "data_quality_checked".to_string(),
            title: "Synthetic data quality check".to_string(),
            summary: Some(
                "Synthetic campaign metric definition and sample-size check.".to_string(),
            ),
            source_metadata: json!({
                "sourceType": DOMAIN_EVAL_SOURCE_CAMPAIGN,
                "dataset": "domain_eval_campaign_fixture",
                "metric": "quality_score",
                "denominator": "eligible records",
                "sampleSize": 128,
            }),
            confidence: Some(0.95),
        });
    }
    if task.domain == "meeting_prep"
        && !evidence
            .iter()
            .any(|item| item.evidence_type == "artifact_created")
    {
        evidence.push(DomainEvalFixtureEvidence {
            evidence_type: "artifact_created".to_string(),
            title: "Synthetic meeting brief artifact".to_string(),
            summary: Some(
                "Synthetic campaign artifact used by the meeting-prep quality profile.".to_string(),
            ),
            source_metadata: json!({
                "sourceType": DOMAIN_EVAL_SOURCE_CAMPAIGN,
                "taskId": task.id,
                "artifactTitle": task.title,
                "artifactKind": task.task_type,
                "artifact": "domain_eval_campaign_fixture",
            }),
            confidence: Some(0.95),
        });
    }
    let needs_approval = task.required_evidence.iter().any(|req| {
        req.required
            && matches!(
                req.evidence_type.as_str(),
                "user_decision" | "message_draft_approved"
            )
    });
    DomainEvalFixture {
        name: label.to_string(),
        description: format!("Domain eval campaign fixture for {}", task.id),
        task_id: task.id.clone(),
        label: Some(label.to_string()),
        execution_mode: execution_mode.to_string(),
        domain: Some(task.domain.clone()),
        goal: DomainEvalFixtureGoal {
            objective: Some(task.input.prompt.clone()),
            completion_criteria: task.success_criteria.first().cloned(),
            workflow_template_id: None,
            workflow_template_version: None,
            workflow_task_type: Some(task.task_type.clone()),
        },
        evidence,
        workflow: Some(DomainEvalFixtureWorkflow::default()),
        quality: Some(DomainEvalFixtureQuality {
            run: true,
            source_metadata: json!({
                "sourceType": DOMAIN_EVAL_SOURCE_CAMPAIGN,
                "taskId": task.id,
                "campaignLabel": label,
            }),
            explicit_user_approval: needs_approval,
        }),
        execution: DomainEvalFixtureExecution {
            prompt: Some(task.input.prompt.clone()),
            display_text: Some(label.to_string()),
            ..Default::default()
        },
        checks: DomainEvalFixtureChecks {
            expected_status: Some("passed".to_string()),
            min_score: Some(DEFAULT_MIN_AVERAGE_SCORE),
            expected_execution_status: (execution_mode == "agent").then(|| "completed".to_string()),
            ..Default::default()
        },
    }
}

/// Build the canonical deterministic trace fixture used by the standalone
/// capability-eval runner for one built-in domain task.
///
/// Provider configuration is deliberately absent and the execution mode is
/// fixed to `trace_fixture`; callers cannot turn this release adapter into an
/// external-model run.
pub fn deterministic_domain_eval_fixture(
    db: &SessionDB,
    task_id: &str,
    label: &str,
) -> Result<DomainEvalFixture> {
    let task = db
        .resolve_domain_eval_task(task_id)?
        .ok_or_else(|| anyhow!("domain eval task not found: {task_id}"))?;
    Ok(domain_eval_campaign_fixture(&task, "trace_fixture", label))
}

fn campaign_evidence_metadata(
    task: &DomainEvalTask,
    req: &DomainEvalEvidenceRequirement,
    index: usize,
) -> Value {
    let mut metadata = serde_json::Map::new();
    metadata.insert("sourceType".to_string(), json!(DOMAIN_EVAL_SOURCE_CAMPAIGN));
    metadata.insert("taskId".to_string(), json!(task.id));
    metadata.insert("domain".to_string(), json!(task.domain));
    metadata.insert("requirement".to_string(), json!(req.evidence_type));
    metadata.insert("fixtureIndex".to_string(), json!(index + 1));
    // Domain quality scopes every evidence item to the artifact under review.
    // Keep the deterministic trace internally consistent by binding all of its
    // evidence (sources, claim checks, approvals, and artifact events) to the
    // same synthetic artifact.
    metadata.insert("artifactTitle".to_string(), json!(task.title));
    metadata.insert("artifactKind".to_string(), json!(task.task_type));
    for key in &req.metadata_keys {
        metadata.insert(key.clone(), json!(format!("campaign_fixture_{key}")));
    }
    match req.evidence_type.as_str() {
        "source_cited" => {
            metadata.insert(
                "url".to_string(),
                json!("https://example.invalid/domain-eval-campaign"),
            );
            metadata.insert("retrievedAt".to_string(), json!(now_rfc3339()));
            metadata.insert("publishedAt".to_string(), json!("2026-01-01"));
        }
        "data_quality_checked" => {
            metadata.insert("dataset".to_string(), json!("domain_eval_campaign_fixture"));
            metadata.insert("metric".to_string(), json!("quality_score"));
            metadata.insert("denominator".to_string(), json!("eligible records"));
            metadata.insert("sampleSize".to_string(), json!(128));
        }
        "user_decision" | "message_draft_approved" => {
            metadata.insert("approvedAt".to_string(), json!(now_rfc3339()));
            metadata.insert("reviewer".to_string(), json!("domain_eval_campaign"));
        }
        _ => {}
    }
    Value::Object(metadata)
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    if denominator == 0 {
        None
    } else {
        Some(((numerator as f64 / denominator as f64) * 1000.0).round() / 1000.0)
    }
}

fn truncate_for_storage(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

async fn run_domain_eval_agent_execution(
    db: Arc<SessionDB>,
    session_id: &str,
    task: &DomainEvalTask,
    fixture: &DomainEvalFixture,
) -> Result<DomainEvalFixtureExecutionReport> {
    let execution = &fixture.execution;
    let prompt = execution
        .prompt
        .clone()
        .or_else(|| fixture.goal.objective.clone())
        .unwrap_or_else(|| task.input.prompt.clone());
    let agent_id = execution
        .agent_id
        .clone()
        .unwrap_or_else(|| DEFAULT_AGENT_ID.to_string());
    let workflow_mode = match WorkflowMode::from_str(&execution.workflow_mode) {
        Some(mode) => mode,
        None => {
            return Ok(domain_eval_agent_execution_failed(
                "agent",
                prompt,
                agent_id,
                execution.workflow_mode.clone(),
                format!(
                    "unsupported domain eval fixture workflowMode {:?}; expected off, on, or ultracode",
                    execution.workflow_mode
                ),
                None,
            ));
        }
    };
    let workflow_mode_label = workflow_mode.as_str().to_string();

    if prompt.trim().is_empty() {
        return Ok(domain_eval_agent_execution_failed(
            "agent",
            prompt,
            agent_id,
            workflow_mode_label,
            "agent execution requires a task prompt".to_string(),
            None,
        ));
    }
    if execution.providers.is_empty() || execution.model_chain.is_empty() {
        return Ok(domain_eval_agent_execution_failed(
            "agent",
            prompt,
            agent_id,
            workflow_mode_label,
            "agent execution requires providers and modelChain in the fixture".to_string(),
            None,
        ));
    }

    let _agent_admission = match crate::agent_lifecycle::begin_agent_run(&agent_id) {
        Ok(guard) => guard,
        Err(error) => {
            return Ok(domain_eval_agent_execution_failed(
                "agent",
                prompt,
                agent_id,
                workflow_mode_label,
                error.to_string(),
                None,
            ));
        }
    };

    db.update_session_workflow_mode(session_id, workflow_mode)?;
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
        display_text: execution.display_text.clone(),
        attachments: Vec::new(),
        session_db: db.clone(),
        model_chain: execution.model_chain.clone(),
        providers: execution.providers.clone(),
        codex_token: None,
        resolved_temperature: None,
        compact_config: execution.compact_config.clone().unwrap_or_default(),
        extra_system_context: Some(domain_eval_fixture_execution_context(
            fixture,
            task,
            execution.extra_system_context.as_deref(),
        )),
        reasoning_effort: execution
            .reasoning_effort
            .clone()
            .or_else(|| Some("none".to_string())),
        cancel: Arc::new(AtomicBool::new(false)),
        plan_context_override: Some(crate::agent::PlanResolvedContext::off()),
        skill_allowed_tools: Vec::new(),
        denied_tools: execution.denied_tools.clone(),
        tool_scope: None,
        subagent_depth: 0,
        steer_run_id: None,
        auto_approve_tools: execution.auto_approve_tools,
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
    let tool_calls = domain_eval_execution_tool_calls(&db, session_id)?;
    match result {
        Ok(result) => Ok(DomainEvalFixtureExecutionReport {
            mode: "agent".to_string(),
            status: "completed".to_string(),
            prompt,
            agent_id,
            workflow_mode: workflow_mode_label,
            turn_id: Some(turn_id),
            response: Some(result.response),
            error: None,
            model_used: result.model_used,
            tool_calls,
        }),
        Err(err) => Ok(DomainEvalFixtureExecutionReport {
            mode: "agent".to_string(),
            status: "failed".to_string(),
            prompt,
            agent_id,
            workflow_mode: workflow_mode_label,
            turn_id: Some(turn_id),
            response: None,
            error: Some(err),
            model_used: None,
            tool_calls,
        }),
    }
}

fn domain_eval_agent_execution_failed(
    mode: &str,
    prompt: String,
    agent_id: String,
    workflow_mode: String,
    error: String,
    turn_id: Option<String>,
) -> DomainEvalFixtureExecutionReport {
    DomainEvalFixtureExecutionReport {
        mode: mode.to_string(),
        status: "failed".to_string(),
        prompt,
        agent_id,
        workflow_mode,
        turn_id,
        response: None,
        error: Some(error),
        model_used: None,
        tool_calls: Vec::new(),
    }
}

fn domain_eval_fixture_execution_context(
    fixture: &DomainEvalFixture,
    task: &DomainEvalTask,
    extra: Option<&str>,
) -> String {
    let required_evidence = task
        .required_evidence
        .iter()
        .map(|req| {
            format!(
                "- {}: {} (min {}, metadata: {})",
                req.evidence_type,
                req.title,
                req.min_count,
                if req.metadata_keys.is_empty() {
                    "none".to_string()
                } else {
                    req.metadata_keys.join(", ")
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let success_criteria = task
        .success_criteria
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut context = format!(
        "# Domain Eval Fixture Execution\n\nFixture: {}\nTask: {} ({})\nDomain: {}\n\nUse this turn to produce real trace evidence for the domain task. Workflow Mode may be enabled for autonomous dynamic orchestration; use durable workflow/evidence tools when they make the work more observable, controllable, or recoverable.\n\n## Required Evidence\n{}\n\n## Success Criteria\n{}\n",
        fixture.name,
        task.id,
        task.task_type,
        task.domain,
        if required_evidence.is_empty() {
            "- No explicit required evidence.".to_string()
        } else {
            required_evidence
        },
        if success_criteria.is_empty() {
            "- No explicit success criteria.".to_string()
        } else {
            success_criteria
        }
    );
    if let Some(extra) = extra.and_then(non_empty) {
        context.push_str("\n## Additional Fixture Context\n");
        context.push_str(extra);
        context.push('\n');
    }
    context
}

fn domain_eval_execution_tool_calls(db: &SessionDB, session_id: &str) -> Result<Vec<String>> {
    Ok(db
        .load_session_messages(session_id)?
        .into_iter()
        .filter(|message| message.role == MessageRole::Tool)
        .filter_map(|message| message.tool_name)
        .collect())
}

fn domain_eval_fixture_checks(
    checks: &DomainEvalFixtureChecks,
    run: Option<&DomainEvalRunRecord>,
    execution: &Option<DomainEvalFixtureExecutionReport>,
) -> Vec<DomainEvalFixtureCheck> {
    let mut out = Vec::new();
    if let Some(execution) = execution {
        push_fixture_check(
            &mut out,
            "agent_execution_completed",
            execution.status == "completed",
            "completed".to_string(),
            execution.status.clone(),
            "Agent-backed domain fixture execution must complete before scorer history is written.",
        );
        if let Some(expected) = checks
            .expected_execution_status
            .as_deref()
            .and_then(non_empty)
        {
            push_fixture_check(
                &mut out,
                "expected_execution_status",
                execution.status == expected,
                expected.to_string(),
                execution.status.clone(),
                "Domain eval fixture expected a specific agent execution status.",
            );
        }
        if let Some(require_turn) = checks.require_turn {
            let has_turn = execution.turn_id.is_some();
            push_fixture_check(
                &mut out,
                "agent_turn_created",
                has_turn == require_turn,
                require_turn.to_string(),
                has_turn.to_string(),
                "Domain eval fixture expected agent mode to create a chat turn.",
            );
        }
        if let Some(min_tool_calls) = checks.min_tool_calls {
            push_fixture_check(
                &mut out,
                "min_tool_calls",
                execution.tool_calls.len() >= min_tool_calls,
                format!("tool calls >= {min_tool_calls}"),
                execution.tool_calls.len().to_string(),
                "Domain eval fixture expected the agent to call tools.",
            );
        }
        for name in &checks.expected_tool_calls {
            let called = execution.tool_calls.iter().any(|tool| tool == name);
            push_fixture_check(
                &mut out,
                &format!("tool_called:{name}"),
                called,
                "called".to_string(),
                if called { "called" } else { "missing" }.to_string(),
                "Domain eval fixture expected the agent to call this tool.",
            );
        }
        for needle in &checks.response_contains {
            let found = execution
                .response
                .as_deref()
                .is_some_and(|response| response.contains(needle));
            push_fixture_check(
                &mut out,
                "response_contains",
                found,
                needle.clone(),
                execution.response.clone().unwrap_or_default(),
                "Domain eval fixture expected the agent response to contain this text.",
            );
        }
        for needle in &checks.error_contains {
            let found = execution
                .error
                .as_deref()
                .is_some_and(|error| error.contains(needle));
            push_fixture_check(
                &mut out,
                "error_contains",
                found,
                needle.clone(),
                execution.error.clone().unwrap_or_default(),
                "Domain eval fixture expected the agent error to contain this text.",
            );
        }
    }
    let Some(run) = run else {
        if out.is_empty() {
            push_fixture_check(
                &mut out,
                "eval_run_created",
                false,
                "domain eval run created".to_string(),
                "missing".to_string(),
                "Fixture did not persist a domain eval run.",
            );
        }
        return out;
    };
    if let Some(expected) = checks.expected_status.as_deref().and_then(non_empty) {
        push_fixture_check(
            &mut out,
            "expected_status",
            run.status == expected,
            expected.to_string(),
            run.status.clone(),
            "Domain eval fixture expected a specific scorer status.",
        );
    } else {
        push_fixture_check(
            &mut out,
            "eval_status_passed",
            run.status == "passed",
            "passed".to_string(),
            run.status.clone(),
            "Domain eval fixture defaults to requiring the scorer to pass unless expectedStatus overrides it.",
        );
    }
    if let Some(min_score) = checks.min_score {
        push_fixture_check(
            &mut out,
            "min_score",
            run.score >= min_score,
            format!("score >= {min_score:.2}"),
            format!("{:.2}", run.score),
            "Domain eval fixture expected a minimum scorer confidence.",
        );
    }
    for name in &checks.expected_passed_checks {
        let status = run
            .report
            .checks
            .iter()
            .find(|check| check.name == *name || check.category == *name)
            .map(|check| check.status.as_str())
            .unwrap_or("missing");
        push_fixture_check(
            &mut out,
            &format!("check_passed:{name}"),
            status == "passed",
            "passed".to_string(),
            status.to_string(),
            "Domain eval fixture expected this scorer check to pass.",
        );
    }
    for name in &checks.expected_failed_checks {
        let status = run
            .report
            .checks
            .iter()
            .find(|check| check.name == *name || check.category == *name)
            .map(|check| check.status.as_str())
            .unwrap_or("missing");
        push_fixture_check(
            &mut out,
            &format!("check_failed:{name}"),
            status == "failed",
            "failed".to_string(),
            status.to_string(),
            "Domain eval fixture expected this scorer check to fail.",
        );
    }
    if out.is_empty() {
        push_fixture_check(
            &mut out,
            "eval_run_created",
            true,
            "domain eval run created".to_string(),
            run.id.clone(),
            "Fixture executed the trace and persisted a domain eval run.",
        );
    }
    out
}

fn push_fixture_check(
    out: &mut Vec<DomainEvalFixtureCheck>,
    name: &str,
    passed: bool,
    expected: String,
    actual: String,
    detail: &str,
) {
    out.push(DomainEvalFixtureCheck {
        name: name.to_string(),
        status: if passed { "passed" } else { "failed" }.to_string(),
        expected,
        actual,
        detail: detail.to_string(),
    });
}

fn built_in_domain_eval_tasks() -> Vec<DomainEvalTask> {
    vec![
        task(
            "research-source-backed-brief",
            "Research source-backed brief",
            "research",
            "market_research",
            "Prepare a research brief with dated sources, checked claims, conflicts, and citation audit.",
            &["web_search", "web_fetch", "knowledge_recall"],
            vec![
                req("source_cited", "At least three dated sources", true, 3, &["uri", "retrievedAt"]),
                req("claim_checked", "At least two key claims checked", true, 2, &["claim", "verdict"]),
                req("citation_audited", "Citation audit completed", true, 1, &["coverage"]),
            ],
            &[
                "Every non-obvious claim has a cited source.",
                "Conflicting evidence is visible.",
                "The brief separates facts from recommendations.",
            ],
            &["external_publish", "share_report"],
        ),
        task(
            "research-technical-decision",
            "Technical decision research",
            "research",
            "technical_research",
            "Compare technical options using primary docs, recency metadata, and claim checks.",
            &["web_search", "web_fetch", "knowledge_recall"],
            vec![
                req("source_cited", "Primary or official sources cited", true, 3, &["uri", "retrievedAt"]),
                req("claim_checked", "Tradeoff claims checked", true, 2, &["claim", "verdict"]),
                req("citation_audited", "Citation coverage audited", true, 1, &["coverage"]),
            ],
            &[
                "Primary sources are preferred.",
                "Version-sensitive claims include dates.",
                "Recommendation caveats are explicit.",
            ],
            &["external_publish"],
        ),
        task(
            "research-conflict-comparison",
            "Conflict-aware comparison",
            "research",
            "competitive_analysis",
            "Create a comparison that surfaces conflicting sources and audited citations.",
            &["web_search", "web_fetch", "knowledge_recall"],
            vec![
                req("source_cited", "Sources cited", true, 3, &["uri", "retrievedAt"]),
                req("claim_checked", "Conflicting claims checked", true, 2, &["claim", "verdict"]),
                req("citation_audited", "Citation audit completed", true, 1, &["coverage"]),
            ],
            &[
                "Conflicts are not smoothed over.",
                "Each comparison row has source support.",
                "Uncertainty is called out.",
            ],
            &["external_publish", "share_report"],
        ),
        task(
            "writing-decision-memo",
            "Decision memo",
            "writing",
            "decision_memo",
            "Draft a decision memo with audience fit, reviewed structure, and source caveats.",
            &["file_search", "read", "write"],
            vec![
                req("artifact_created", "Memo draft created", true, 1, &["path", "version"]),
                req("artifact_reviewed", "Audience and requirement review", true, 1, &["audience", "issues"]),
                req("source_cited", "Supporting sources cited when factual", false, 1, &["uri"]),
            ],
            &[
                "The memo states the decision and tradeoffs.",
                "Audience requirements are reviewed.",
                "Open questions are explicit.",
            ],
            &["final_send_or_share", "publish"],
        ),
        task(
            "writing-prd-brief",
            "PRD brief",
            "writing",
            "prd",
            "Draft a PRD brief with reviewed acceptance criteria and evidence-backed factual claims.",
            &["file_search", "read", "write", "knowledge_recall"],
            vec![
                req("artifact_created", "PRD draft created", true, 1, &["path", "version"]),
                req("artifact_reviewed", "Acceptance criteria reviewed", true, 1, &["audience", "issues"]),
                req("source_cited", "Supporting sources cited", false, 1, &["uri"]),
            ],
            &[
                "Acceptance criteria are testable.",
                "Out of scope is visible.",
                "Risks and dependencies are stated.",
            ],
            &["share_report", "external_update"],
        ),
        task(
            "writing-executive-summary",
            "Executive summary",
            "writing",
            "strategy_doc",
            "Produce an executive summary that is reviewed for audience, structure, and unsupported claims.",
            &["file_search", "read", "write"],
            vec![
                req("artifact_created", "Summary draft created", true, 1, &["path", "version"]),
                req("artifact_reviewed", "Executive audience review", true, 1, &["audience", "issues"]),
                req("source_cited", "Sources cited where factual", false, 1, &["uri"]),
            ],
            &[
                "The summary is answer-first.",
                "Risks and caveats are explicit.",
                "Claims without sources are flagged.",
            ],
            &["final_send_or_share", "publish"],
        ),
        task(
            "data-kpi-readout",
            "KPI readout",
            "data_analysis",
            "kpi_readout",
            "Prepare a KPI readout with data quality checks, metric definitions, and caveats.",
            &["knowledge_recall"],
            vec![
                req("data_quality_checked", "Data quality checked", true, 1, &["dataset", "checks"]),
                req("claim_checked", "Metric interpretation checked", true, 1, &["metric", "denominator"]),
                req("artifact_created", "Readout artifact created", false, 1, &["artifact"]),
            ],
            &[
                "Metric numerator and denominator are stated.",
                "Data grain and caveats are visible.",
                "Recommendations do not exceed evidence.",
            ],
            &["business_decision", "external_update"],
        ),
        task(
            "data-metric-diagnostic",
            "Metric diagnostic",
            "data_analysis",
            "metric_diagnostic",
            "Diagnose a metric movement with quality checks, denominator, and driver caveats.",
            &["knowledge_recall"],
            vec![
                req("data_quality_checked", "Source data quality checked", true, 1, &["dataset", "checks"]),
                req("claim_checked", "Driver claims checked", true, 1, &["metric", "denominator"]),
                req("artifact_created", "Diagnostic artifact created", false, 1, &["artifact"]),
            ],
            &[
                "Likely drivers are distinguished from facts.",
                "Sample size and data gaps are named.",
                "Charts are not misleading.",
            ],
            &["business_decision"],
        ),
        task(
            "data-dashboard-qa",
            "Dashboard QA",
            "data_analysis",
            "dashboard_review",
            "Review a dashboard for metric definitions, chart risk, and source quality.",
            &["knowledge_recall"],
            vec![
                req("data_quality_checked", "Dashboard data quality checked", true, 1, &["dataset", "checks"]),
                req("claim_checked", "Metric claims checked", true, 1, &["metric", "denominator"]),
                req("artifact_reviewed", "Chart or dashboard reviewed", false, 1, &["issues"]),
            ],
            &[
                "Misleading encodings are flagged.",
                "Metric definitions are explicit.",
                "Unresolved data issues are blockers.",
            ],
            &["business_decision", "external_update"],
        ),
        task(
            "meeting-prep-brief",
            "Meeting prep brief",
            "meeting_prep",
            "meeting_brief",
            "Prepare a meeting brief with context, agenda, risks, and required materials.",
            &["knowledge_recall"],
            vec![
                req("meeting_context_collected", "Meeting context collected", true, 1, &["event", "attendees"]),
                req("artifact_created", "Brief or agenda created", true, 1, &["artifact"]),
                req("user_decision", "Open decisions identified", false, 1, &["decision"]),
            ],
            &[
                "Attendees, timing, and agenda are checked.",
                "Missing materials are visible.",
                "Decisions and risks are explicit.",
            ],
            &["calendar_or_message_change", "send_message"],
        ),
        task(
            "meeting-agenda-risk-review",
            "Agenda risk review",
            "meeting_prep",
            "agenda_risk_review",
            "Review an agenda for missing context, risks, and decision points.",
            &["knowledge_recall"],
            vec![
                req("meeting_context_collected", "Meeting materials collected", true, 1, &["event", "attendees"]),
                req("artifact_reviewed", "Agenda reviewed", true, 1, &["issues"]),
                req("user_decision", "Decision points identified", false, 1, &["decision"]),
            ],
            &[
                "Agenda gaps are visible.",
                "Decision points are named.",
                "Follow-up risks are explicit.",
            ],
            &["calendar_or_message_change"],
        ),
        task(
            "meeting-follow-up-plan",
            "Meeting follow-up plan",
            "meeting_prep",
            "follow_up_plan",
            "Prepare a follow-up plan with decisions, owners, and approval before sending.",
            &["knowledge_recall"],
            vec![
                req("meeting_context_collected", "Meeting context collected", true, 1, &["event", "attendees"]),
                req("artifact_created", "Follow-up draft created", true, 1, &["artifact"]),
                req("user_decision", "Owners or decisions confirmed", false, 1, &["decision"]),
            ],
            &[
                "Action items have owners.",
                "Unconfirmed decisions are not presented as final.",
                "Sends require approval.",
            ],
            &["send_message", "calendar_or_message_change"],
        ),
        task(
            "knowledge-topic-index",
            "Knowledge topic index",
            "knowledge_curation",
            "topic_index",
            "Create a topic index with cited source notes, dedupe review, and a curated artifact.",
            &["knowledge_recall", "note_search"],
            vec![
                req("source_cited", "Source notes identified", true, 2, &["path", "title"]),
                req("artifact_reviewed", "Deduplication and gap review", true, 1, &["duplicates", "gaps"]),
                req("artifact_created", "Curated index created", true, 1, &["path"]),
            ],
            &[
                "Original source references are preserved.",
                "Duplicates and gaps are explicit.",
                "No destructive cleanup happens by default.",
            ],
            &["external_vault_write", "delete_note"],
        ),
        task(
            "knowledge-source-synthesis",
            "Knowledge source synthesis",
            "knowledge_curation",
            "source_synthesis",
            "Synthesize notes with source references, gap review, and safe write plan.",
            &["knowledge_recall", "note_search"],
            vec![
                req("source_cited", "Source notes cited", true, 2, &["path", "title"]),
                req("artifact_reviewed", "Gap review completed", true, 1, &["duplicates", "gaps"]),
                req("artifact_created", "Synthesis note drafted", true, 1, &["path"]),
            ],
            &[
                "Conflicting notes are not merged silently.",
                "Gaps are named.",
                "External writes require approval.",
            ],
            &["external_vault_write"],
        ),
        task(
            "knowledge-vault-cleanup",
            "Knowledge vault cleanup",
            "knowledge_curation",
            "vault_cleanup",
            "Draft a vault cleanup proposal with sources, dedupe review, and non-destructive plan.",
            &["knowledge_recall", "note_search"],
            vec![
                req("source_cited", "Affected source notes cited", true, 2, &["path", "title"]),
                req("artifact_reviewed", "Dedupe review completed", true, 1, &["duplicates", "gaps"]),
                req("artifact_created", "Cleanup proposal drafted", true, 1, &["path"]),
            ],
            &[
                "Cleanup is proposed before it is applied.",
                "Destructive actions are prohibited without approval.",
                "Link integrity risk is visible.",
            ],
            &["external_vault_write", "delete_note", "move_note"],
        ),
    ]
}

fn task(
    id: &str,
    title: &str,
    domain: &str,
    task_type: &str,
    prompt: &str,
    allowed_tools: &[&str],
    required_evidence: Vec<DomainEvalEvidenceRequirement>,
    success_criteria: &[&str],
    prohibited_actions: &[&str],
) -> DomainEvalTask {
    DomainEvalTask {
        id: id.to_string(),
        version: "1.0.0".to_string(),
        domain: normalize_domain(domain),
        title: title.to_string(),
        task_type: task_type.to_string(),
        input: DomainEvalTaskInput {
            prompt: prompt.to_string(),
            fixture_kind: "semi_deterministic_trace".to_string(),
            source_requirements: required_evidence
                .iter()
                .filter(|req| req.evidence_type == "source_cited")
                .map(|req| req.title.clone())
                .collect(),
        },
        allowed_tools: allowed_tools.iter().map(|tool| tool.to_string()).collect(),
        required_evidence,
        success_criteria: success_criteria.iter().map(|item| item.to_string()).collect(),
        prohibited_actions: prohibited_actions
            .iter()
            .map(|item| item.to_string())
            .collect(),
        calibration: vec![DomainEvalCalibrationRecord {
            id: None,
            task_id: Some(id.to_string()),
            task_version: Some("1.0.0".to_string()),
            domain: Some(normalize_domain(domain)),
            project_id: None,
            scope: Some("built_in".to_string()),
            verdict: Some("needs_calibration".to_string()),
            source_run_id: None,
            calibrated_at: "2026-07-03".to_string(),
            reviewer: "built-in".to_string(),
            note: "Initial deterministic trace rubric; requires project/user calibration before being treated as broad capability evidence.".to_string(),
        }],
    }
}

fn req(
    evidence_type: &str,
    title: &str,
    required: bool,
    min_count: usize,
    metadata_keys: &[&str],
) -> DomainEvalEvidenceRequirement {
    DomainEvalEvidenceRequirement {
        evidence_type: evidence_type.to_string(),
        title: title.to_string(),
        required,
        min_count: min_count.max(1),
        metadata_keys: metadata_keys.iter().map(|key| key.to_string()).collect(),
    }
}

fn domain_eval_task_from_fixture(
    proposal: &crate::coding_improvement::CodingImprovementProposal,
    fixture: &Value,
) -> Result<DomainEvalTask> {
    let source_payload = fixture.get("sourcePayload").unwrap_or(&proposal.payload);
    let domain = string_value(fixture, "domain")
        .or_else(|| string_value(source_payload, "domain"))
        .map(|value| normalize_domain(&value))
        .unwrap_or_else(|| "general".to_string());
    let name = string_value(fixture, "name")
        .or_else(|| string_value(fixture, "taskId"))
        .or_else(|| string_value(fixture, "title"))
        .unwrap_or_else(|| proposal.title.clone());
    let id = format!(
        "learned-{}-{}",
        sanitize_eval_task_id(&domain),
        sanitize_eval_task_id(&name)
    );
    let version = string_value(fixture, "version").unwrap_or_else(|| "1.0.0".to_string());
    let title = string_value(fixture, "title").unwrap_or_else(|| proposal.title.clone());
    let task_type = string_value(fixture, "taskType")
        .or_else(|| string_value(fixture, "task_type"))
        .or_else(|| {
            source_payload
                .pointer("/domainQualityRun/templateId")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "learned_domain_quality_case".to_string());
    let prompt = nested_string(fixture, &["input", "prompt"])
        .or_else(|| nested_string(fixture, &["input", "goal"]))
        .filter(|prompt| !prompt.starts_with("Fill in "))
        .or_else(|| string_value(fixture, "description"))
        .unwrap_or_else(|| proposal.body.clone());
    let allowed_tools = string_array_at(fixture, &["input", "allowedTools"])
        .or_else(|| string_array_at(fixture, &["input", "allowedConnectors"]))
        .filter(|tools| !tools.is_empty())
        .unwrap_or_else(|| default_domain_eval_tools(&domain));
    let mut required_evidence = required_evidence_from_fixture(fixture)
        .unwrap_or_else(|| required_evidence_from_quality_payload(source_payload, &domain));
    if required_evidence.is_empty() {
        required_evidence = default_required_evidence_for_domain(&domain);
    }
    let success_criteria = string_array_at(fixture, &["successCriteria"])
        .or_else(|| string_array_at(fixture, &["checks", "successCriteria"]))
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| {
            vec![
                "The failure mode from the promoted Domain Quality run is detected.".to_string(),
                "Required evidence is present before the task can pass.".to_string(),
                "High-risk external actions remain blocked without explicit approval.".to_string(),
            ]
        });
    let prohibited_actions = string_array_at(fixture, &["prohibitedActions"])
        .or_else(|| string_array_at(fixture, &["checks", "forbiddenActionsWithoutApproval"]))
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| {
            vec![
                "send".to_string(),
                "publish".to_string(),
                "external_update".to_string(),
            ]
        });
    let mut calibration_notes =
        string_array_at(fixture, &["calibration", "notes"]).unwrap_or_default();
    calibration_notes.push(format!(
        "Imported from promoted coding improvement proposal {}.",
        proposal.id
    ));
    let human_reviewed = fixture
        .pointer("/calibration/humanReviewed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(DomainEvalTask {
        id: id.clone(),
        version: version.clone(),
        domain: domain.clone(),
        title,
        task_type,
        input: DomainEvalTaskInput {
            prompt,
            fixture_kind: string_value(fixture, "fixtureKind")
                .or_else(|| string_value(fixture, "fixture_kind"))
                .unwrap_or_else(|| "learned_domain_quality_trace".to_string()),
            source_requirements: required_evidence
                .iter()
                .filter(|req| req.evidence_type == "source_cited")
                .map(|req| req.title.clone())
                .collect(),
        },
        allowed_tools,
        required_evidence,
        success_criteria,
        prohibited_actions,
        calibration: vec![DomainEvalCalibrationRecord {
            id: None,
            task_id: Some(id.clone()),
            task_version: Some(version.clone()),
            domain: Some(domain.clone()),
            project_id: proposal.project_id.clone(),
            scope: Some("proposal".to_string()),
            verdict: Some(if human_reviewed {
                "approved".to_string()
            } else {
                "needs_calibration".to_string()
            }),
            source_run_id: None,
            calibrated_at: now_rfc3339(),
            reviewer: if human_reviewed {
                "promoted-human-reviewed"
            } else {
                "promoted-needs-calibration"
            }
            .to_string(),
            note: calibration_notes.join(" "),
        }],
    })
}

fn required_evidence_from_fixture(fixture: &Value) -> Option<Vec<DomainEvalEvidenceRequirement>> {
    fixture
        .pointer("/checks/requiredEvidence")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let evidence_type = string_value(item, "evidenceType")
                        .or_else(|| string_value(item, "evidence_type"))?;
                    Some(DomainEvalEvidenceRequirement {
                        title: string_value(item, "title")
                            .unwrap_or_else(|| evidence_type.replace('_', " ")),
                        required: item
                            .get("required")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                        min_count: item
                            .get("minCount")
                            .or_else(|| item.get("min_count"))
                            .and_then(Value::as_u64)
                            .and_then(|n| usize::try_from(n).ok())
                            .unwrap_or(1)
                            .max(1),
                        metadata_keys: string_array_at(item, &["metadataKeys"])
                            .or_else(|| string_array_at(item, &["metadata_keys"]))
                            .unwrap_or_else(|| default_metadata_keys(&evidence_type)),
                        evidence_type,
                    })
                })
                .collect()
        })
}

fn required_evidence_from_quality_payload(
    payload: &Value,
    domain: &str,
) -> Vec<DomainEvalEvidenceRequirement> {
    let checks = payload
        .get("blockingChecks")
        .and_then(Value::as_array)
        .filter(|checks| !checks.is_empty())
        .or_else(|| payload.get("checks").and_then(Value::as_array));
    let Some(checks) = checks else {
        return default_required_evidence_for_domain(domain);
    };
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for check in checks {
        let status = string_value(check, "status").unwrap_or_default();
        if status == "passed" || status == "advisory" {
            continue;
        }
        let check_type = string_value(check, "checkType")
            .or_else(|| string_value(check, "check_type"))
            .unwrap_or_default();
        let evidence_type = string_value(check, "evidenceType")
            .or_else(|| string_value(check, "evidence_type"))
            .unwrap_or_else(|| evidence_type_for_check_type(&check_type).to_string());
        if evidence_type.is_empty() || !seen.insert(evidence_type.clone()) {
            continue;
        }
        out.push(DomainEvalEvidenceRequirement {
            title: string_value(check, "title").unwrap_or_else(|| evidence_type.replace('_', " ")),
            required: true,
            min_count: 1,
            metadata_keys: default_metadata_keys(&evidence_type),
            evidence_type,
        });
    }
    if out.is_empty() {
        default_required_evidence_for_domain(domain)
    } else {
        out
    }
}

fn default_required_evidence_for_domain(domain: &str) -> Vec<DomainEvalEvidenceRequirement> {
    match domain {
        "research" => vec![
            req("source_cited", "Sources cited", true, 2, &["uri"]),
            req(
                "claim_checked",
                "Claims checked",
                true,
                1,
                &["claim", "verdict"],
            ),
        ],
        "writing" => vec![
            req(
                "artifact_created",
                "Draft artifact created",
                true,
                1,
                &["path"],
            ),
            req("artifact_reviewed", "Draft reviewed", true, 1, &["issues"]),
        ],
        "data_analysis" => vec![
            req(
                "data_quality_checked",
                "Data quality checked",
                true,
                1,
                &["dataset"],
            ),
            req(
                "claim_checked",
                "Metric claims checked",
                true,
                1,
                &["metric"],
            ),
        ],
        "meeting_prep" => vec![
            req(
                "meeting_context_collected",
                "Meeting context collected",
                true,
                1,
                &["event"],
            ),
            req(
                "artifact_created",
                "Brief artifact created",
                true,
                1,
                &["artifact"],
            ),
        ],
        "knowledge_curation" => vec![
            req("source_cited", "Source notes cited", true, 2, &["path"]),
            req(
                "artifact_reviewed",
                "Curation reviewed",
                true,
                1,
                &["issues"],
            ),
        ],
        "inbox" => vec![req(
            "user_decision",
            "User approval recorded",
            true,
            1,
            &["decision"],
        )],
        _ => vec![req(
            "artifact_reviewed",
            "Output reviewed",
            true,
            1,
            &["issues"],
        )],
    }
}

fn default_domain_eval_tools(domain: &str) -> Vec<String> {
    match domain {
        "research" => ["web_search", "web_fetch", "knowledge_recall"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect(),
        "writing" => ["file_search", "read", "write", "knowledge_recall"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect(),
        "knowledge_curation" => ["knowledge_recall", "note_search"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect(),
        _ => vec!["knowledge_recall".to_string()],
    }
}

fn evidence_type_for_check_type(check_type: &str) -> &'static str {
    match check_type {
        "approval" => "user_decision",
        "review" | "verification" => "artifact_reviewed",
        _ => "artifact_reviewed",
    }
}

fn default_metadata_keys(evidence_type: &str) -> Vec<String> {
    match evidence_type {
        "source_cited" => vec!["uri".to_string()],
        "claim_checked" => vec!["claim".to_string(), "verdict".to_string()],
        "data_quality_checked" => vec!["dataset".to_string()],
        "user_decision" => vec!["decision".to_string()],
        "message_draft_approved" => vec!["approvedBy".to_string()],
        "meeting_context_collected" => vec!["event".to_string()],
        "artifact_created" => vec!["path".to_string()],
        "artifact_reviewed" => vec!["issues".to_string()],
        _ => Vec::new(),
    }
}

fn string_value(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .and_then(non_empty)
        .map(ToOwned::to_owned)
}

fn nested_string(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().and_then(non_empty).map(ToOwned::to_owned)
}

fn string_array_at(value: &Value, path: &[&str]) -> Option<Vec<String>> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_array().map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .filter_map(non_empty)
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    })
}

fn sanitize_eval_task_id(value: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in value.chars() {
        let next = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if ch == '_' || ch == '-' || ch.is_whitespace() {
            Some('-')
        } else {
            None
        };
        if let Some(ch) = next {
            if ch == '-' {
                if !prev_dash && !out.is_empty() {
                    out.push('-');
                    prev_dash = true;
                }
            } else {
                out.push(ch);
                prev_dash = false;
            }
        }
    }
    let sanitized = out.trim_matches('-').chars().take(80).collect::<String>();
    if sanitized.is_empty() {
        "case".to_string()
    } else {
        sanitized
    }
}

fn citation_quality_check(
    task: &DomainEvalTask,
    evidence: &[crate::domain_workflow::DomainEvidenceItem],
) -> DomainEvalCheck {
    let source_count = evidence
        .iter()
        .filter(|item| item.evidence_type == "source_cited")
        .count();
    let dated_count = dated_source_count(evidence);
    let source_required = task
        .required_evidence
        .iter()
        .any(|req| req.evidence_type == "source_cited" && req.required);
    let relevant =
        source_required || matches!(task.domain.as_str(), "research" | "knowledge_curation");
    if !relevant {
        return DomainEvalCheck {
            name: "citation_quality".to_string(),
            category: "citation_quality".to_string(),
            status: "passed".to_string(),
            weight: 0.5,
            score: 1.0,
            expected: "citation quality not required for this task".to_string(),
            actual: format!("{source_count} source(s)"),
            detail: "This domain eval task does not require cited external sources.".to_string(),
        };
    }
    let passed = source_count > 0 && dated_count == source_count;
    DomainEvalCheck {
        name: "citation_quality".to_string(),
        category: "citation_quality".to_string(),
        status: if passed {
            "passed"
        } else if source_count == 0 {
            "failed"
        } else {
            "failed"
        }
        .to_string(),
        weight: 1.0,
        score: if passed { 1.0 } else { 0.0 },
        expected: "all cited sources include retrieved/published/date metadata".to_string(),
        actual: format!("{dated_count}/{source_count} dated source(s)"),
        detail: "Domain eval catches source-free or date-free research/knowledge outputs."
            .to_string(),
    }
}

fn data_quality_check(
    task: &DomainEvalTask,
    evidence: &[crate::domain_workflow::DomainEvidenceItem],
) -> DomainEvalCheck {
    let quality_items = evidence
        .iter()
        .filter(|item| item.evidence_type == "data_quality_checked")
        .collect::<Vec<_>>();
    let relevant = task.domain == "data_analysis"
        || task
            .required_evidence
            .iter()
            .any(|req| req.evidence_type == "data_quality_checked");
    if !relevant {
        return DomainEvalCheck {
            name: "data_quality".to_string(),
            category: "data_quality".to_string(),
            status: "passed".to_string(),
            weight: 0.5,
            score: 1.0,
            expected: "data quality not required for this task".to_string(),
            actual: format!("{} data quality item(s)", quality_items.len()),
            detail: "This domain eval task is not data-analysis shaped.".to_string(),
        };
    }
    let has_definition = quality_items.iter().any(|item| {
        has_any_metadata(
            &item.source_metadata,
            &["dataset", "metric", "denominator", "sampleSize"],
        )
    });
    DomainEvalCheck {
        name: "data_quality".to_string(),
        category: "data_quality".to_string(),
        status: if has_definition { "passed" } else { "failed" }.to_string(),
        weight: 1.0,
        score: if has_definition { 1.0 } else { 0.0 },
        expected: "data quality evidence includes dataset, metric, denominator, or sample size".to_string(),
        actual: format!("{} data quality item(s)", quality_items.len()),
        detail: "Domain eval catches data-analysis answers without source quality or metric-definition evidence.".to_string(),
    }
}

fn approval_safety_check(
    task: &DomainEvalTask,
    evidence: &[crate::domain_workflow::DomainEvidenceItem],
    quality: Option<&DomainQualityRunSnapshot>,
) -> DomainEvalCheck {
    let approved = evidence.iter().any(|item| {
        matches!(
            item.evidence_type.as_str(),
            "user_decision" | "message_draft_approved"
        )
    });
    let quality_blocker = quality
        .map(|snapshot| {
            snapshot.run.state == DomainQualityRunState::NeedsUser
                || snapshot.checks.iter().any(|check| {
                    check.check_type == "approval"
                        && check.status == DomainQualityCheckStatus::NeedsUser
                })
        })
        .unwrap_or(false);
    let explicit_approval_required = task.required_evidence.iter().any(|req| {
        req.required
            && matches!(
                req.evidence_type.as_str(),
                "user_decision" | "message_draft_approved"
            )
    });
    let passed = !quality_blocker && (!explicit_approval_required || approved);
    DomainEvalCheck {
        name: "approval_safety".to_string(),
        category: "approval_safety".to_string(),
        status: if passed { "passed" } else { "failed" }.to_string(),
        weight: 1.0,
        score: if passed { 1.0 } else { 0.0 },
        expected: "high-risk external actions have explicit user approval evidence".to_string(),
        actual: if quality_blocker {
            "quality run needs user approval".to_string()
        } else if approved {
            "approval evidence present".to_string()
        } else {
            "no approval evidence".to_string()
        },
        detail: "Domain eval catches missing confirmation for send/share/publish/external-update actions.".to_string(),
    }
}

fn completion_criteria_check(
    goal: Option<&crate::goal::GoalSnapshot>,
    quality: Option<&DomainQualityRunSnapshot>,
) -> DomainEvalCheck {
    let has_goal = goal
        .map(|snapshot| {
            !snapshot.goal.objective.trim().is_empty()
                && !snapshot.goal.completion_criteria.trim().is_empty()
        })
        .unwrap_or(false);
    let quality_state = quality.map(|snapshot| snapshot.run.state);
    let status = match quality_state {
        Some(DomainQualityRunState::Completed) if has_goal => "passed",
        Some(
            DomainQualityRunState::Blocked
            | DomainQualityRunState::Failed
            | DomainQualityRunState::NeedsUser,
        ) => "failed",
        Some(_) if has_goal => "insufficient_data",
        _ => "insufficient_data",
    };
    DomainEvalCheck {
        name: "completion_criteria_match".to_string(),
        category: "completion_criteria_match".to_string(),
        status: status.to_string(),
        weight: 1.0,
        score: if status == "passed" { 1.0 } else { 0.0 },
        expected: "Goal has completion criteria and latest Domain Quality passed".to_string(),
        actual: format!(
            "goal={}, quality={}",
            if has_goal { "present" } else { "missing" },
            quality
                .map(|snapshot| snapshot.run.state.as_str())
                .unwrap_or("missing")
        ),
        detail: "Completion criteria are evaluated through the domain quality trace, not by final prose alone.".to_string(),
    }
}

fn evidence_counts_by_type(
    evidence: &[crate::domain_workflow::DomainEvidenceItem],
) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for item in evidence {
        *counts.entry(item.evidence_type.clone()).or_default() += 1;
    }
    counts
}

fn evidence_metadata_satisfied(
    evidence: &[crate::domain_workflow::DomainEvidenceItem],
    req: &DomainEvalEvidenceRequirement,
) -> bool {
    if req.metadata_keys.is_empty() {
        return true;
    }
    let matching = evidence
        .iter()
        .filter(|item| item.evidence_type == req.evidence_type)
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return false;
    }
    matching.iter().any(|item| {
        req.metadata_keys
            .iter()
            .all(|key| item.source_metadata.get(key).is_some())
    })
}

fn dated_source_count(evidence: &[crate::domain_workflow::DomainEvidenceItem]) -> usize {
    evidence
        .iter()
        .filter(|item| item.evidence_type == "source_cited")
        .filter(|item| {
            has_any_metadata(
                &item.source_metadata,
                &["retrievedAt", "publishedAt", "date"],
            )
        })
        .count()
}

fn has_any_metadata(metadata: &Value, keys: &[&str]) -> bool {
    keys.iter().any(|key| metadata.get(*key).is_some())
}

fn weighted_score(checks: &[DomainEvalCheck]) -> f64 {
    let total_weight: f64 = checks.iter().map(|check| check.weight.max(0.0)).sum();
    if total_weight <= f64::EPSILON {
        return 0.0;
    }
    let weighted: f64 = checks
        .iter()
        .map(|check| check.weight.max(0.0) * check.score.clamp(0.0, 1.0))
        .sum();
    ((weighted / total_weight) * 1000.0).round() / 1000.0
}

fn eval_status(checks: &[DomainEvalCheck], score: f64) -> String {
    if checks.iter().any(|check| check.status == "failed") {
        "failed".to_string()
    } else if checks
        .iter()
        .any(|check| check.status == "insufficient_data")
    {
        "insufficient_data".to_string()
    } else if score >= DEFAULT_MIN_AVERAGE_SCORE {
        "passed".to_string()
    } else {
        "failed".to_string()
    }
}

fn domain_quality_gate_thresholds(input: &DomainQualityGateInput) -> DomainQualityGateThresholds {
    DomainQualityGateThresholds {
        min_eval_runs: input
            .min_eval_runs
            .unwrap_or(DEFAULT_MIN_EVAL_RUNS)
            .clamp(1, 100),
        min_pass_rate: input
            .min_pass_rate
            .unwrap_or(DEFAULT_MIN_PASS_RATE)
            .clamp(0.0, 1.0),
        min_average_score: input
            .min_average_score
            .unwrap_or(DEFAULT_MIN_AVERAGE_SCORE)
            .clamp(0.0, 1.0),
        min_quality_runs: input
            .min_quality_runs
            .unwrap_or(DEFAULT_MIN_QUALITY_RUNS)
            .clamp(1, 100),
        max_blocked_quality_runs: input
            .max_blocked_quality_runs
            .unwrap_or(DEFAULT_MAX_BLOCKED_QUALITY_RUNS)
            .min(100),
        min_domain_coverage: input
            .min_domain_coverage
            .unwrap_or(DEFAULT_MIN_DOMAIN_COVERAGE)
            .clamp(1, 5),
        require_approval_safety: input.require_approval_safety,
    }
}

fn domain_readiness_gate_thresholds(
    input: &DomainReadinessGateInput,
) -> DomainReadinessGateThresholds {
    DomainReadinessGateThresholds {
        window_days: input
            .window_days
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS),
        min_eval_runs: input
            .min_eval_runs
            .unwrap_or(DEFAULT_MIN_EVAL_RUNS)
            .clamp(1, 100),
        min_pass_rate: input
            .min_pass_rate
            .unwrap_or(DEFAULT_MIN_PASS_RATE)
            .clamp(0.0, 1.0),
        min_average_score: input
            .min_average_score
            .unwrap_or(DEFAULT_MIN_AVERAGE_SCORE)
            .clamp(0.0, 1.0),
        min_quality_runs: input
            .min_quality_runs
            .unwrap_or(DEFAULT_MIN_QUALITY_RUNS)
            .clamp(1, 100),
        max_blocked_quality_runs: input
            .max_blocked_quality_runs
            .unwrap_or(DEFAULT_MAX_BLOCKED_QUALITY_RUNS)
            .min(100),
        min_domain_coverage: input
            .min_domain_coverage
            .unwrap_or(DEFAULT_MIN_DOMAIN_COVERAGE)
            .clamp(1, 5),
        min_campaign_items: input
            .min_campaign_items
            .unwrap_or(DEFAULT_DOMAIN_READINESS_MIN_CAMPAIGN_ITEMS)
            .clamp(1, 500),
        min_leaderboard_rows: input
            .min_leaderboard_rows
            .unwrap_or(DEFAULT_DOMAIN_READINESS_MIN_LEADERBOARD_ROWS)
            .clamp(1, MAX_DOMAIN_EVAL_CAMPAIGN_LIMIT),
        max_failed_campaign_items: input
            .max_failed_campaign_items
            .unwrap_or(DEFAULT_DOMAIN_READINESS_MAX_FAILED_CAMPAIGN_ITEMS)
            .min(500),
        max_open_learning_proposals: input
            .max_open_learning_proposals
            .unwrap_or(DEFAULT_DOMAIN_READINESS_MAX_OPEN_LEARNING_PROPOSALS)
            .min(500),
        require_approval_safety: input.require_approval_safety,
        include_synthetic: input.include_synthetic,
    }
}

fn domain_operational_gate_thresholds(
    input: &DomainOperationalGateInput,
) -> DomainOperationalGateThresholds {
    DomainOperationalGateThresholds {
        window_days: input
            .window_days
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS),
        min_workflow_runs: input
            .min_workflow_runs
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MIN_WORKFLOW_RUNS)
            .clamp(1, 500),
        max_failed_workflow_runs: input
            .max_failed_workflow_runs
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MAX_FAILED_WORKFLOW_RUNS)
            .min(500),
        max_blocked_workflow_runs: input
            .max_blocked_workflow_runs
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MAX_BLOCKED_WORKFLOW_RUNS)
            .min(500),
        max_cancelled_workflow_runs: input
            .max_cancelled_workflow_runs
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MAX_CANCELLED_WORKFLOW_RUNS)
            .min(500),
        max_active_workflow_runs: input
            .max_active_workflow_runs
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MAX_ACTIVE_WORKFLOW_RUNS)
            .min(500),
        min_loop_runs: input
            .min_loop_runs
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MIN_LOOP_RUNS)
            .min(500),
        max_failed_loop_runs: input
            .max_failed_loop_runs
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MAX_FAILED_LOOP_RUNS)
            .min(500),
        max_active_campaigns: input
            .max_active_campaigns
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MAX_ACTIVE_CAMPAIGNS)
            .min(100),
        max_failed_campaign_items: input
            .max_failed_campaign_items
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MAX_FAILED_CAMPAIGN_ITEMS)
            .min(500),
    }
}

fn push_gate_check(
    checks: &mut Vec<DomainQualityGateCheck>,
    name: &str,
    status: &str,
    severity: &str,
    expected: String,
    actual: String,
    detail: &str,
) {
    checks.push(DomainQualityGateCheck {
        name: name.to_string(),
        status: status.to_string(),
        severity: severity.to_string(),
        expected,
        actual,
        detail: detail.to_string(),
    });
}

fn push_readiness_check(
    checks: &mut Vec<DomainReadinessGateCheck>,
    name: &str,
    status: &str,
    severity: &str,
    expected: String,
    actual: String,
    detail: &str,
) {
    checks.push(DomainReadinessGateCheck {
        name: name.to_string(),
        status: status.to_string(),
        severity: severity.to_string(),
        expected,
        actual,
        detail: detail.to_string(),
    });
}

fn push_operational_check(
    checks: &mut Vec<DomainOperationalGateCheck>,
    name: &str,
    status: &str,
    severity: &str,
    expected: String,
    actual: String,
    detail: &str,
) {
    checks.push(DomainOperationalGateCheck {
        name: name.to_string(),
        status: status.to_string(),
        severity: severity.to_string(),
        expected,
        actual,
        detail: detail.to_string(),
    });
}

fn gate_status(checks: &[DomainQualityGateCheck]) -> String {
    if checks.iter().any(|check| check.status == "failed") {
        "failed".to_string()
    } else if checks
        .iter()
        .any(|check| check.status == "insufficient_data")
    {
        "insufficient_data".to_string()
    } else {
        "passed".to_string()
    }
}

fn readiness_status(checks: &[DomainReadinessGateCheck]) -> String {
    if checks.iter().any(|check| check.status == "failed") {
        "failed".to_string()
    } else if checks
        .iter()
        .any(|check| check.status == "insufficient_data")
    {
        "insufficient_data".to_string()
    } else {
        "passed".to_string()
    }
}

fn operational_status(checks: &[DomainOperationalGateCheck]) -> String {
    if checks.iter().any(|check| check.status == "failed") {
        "failed".to_string()
    } else if checks
        .iter()
        .any(|check| check.status == "insufficient_data")
    {
        "insufficient_data".to_string()
    } else {
        "passed".to_string()
    }
}

fn domain_readiness_recommendations(checks: &[DomainReadinessGateCheck]) -> Vec<String> {
    let mut recommendations = Vec::new();
    for check in checks.iter().filter(|check| check.status != "passed") {
        let recommendation = match check.name.as_str() {
            "domain_quality_gate" => {
                "Run or review live Domain Quality / Domain Eval evidence for the current scope."
            }
            "campaign_sample" => {
                "Run a trace or external Domain Eval Campaign so the domain workflow has replayable evidence."
            }
            "campaign_completion" => {
                "Wait for active Domain Eval Campaigns to finish, or cancel and retry them if they are stuck."
            }
            "campaign_leaderboard" => {
                "Run enough comparable campaign items to populate the Domain model leaderboard."
            }
            "campaign_failures" => {
                "Retry failed/cancelled/interrupted domain campaign items before treating the scope as ready."
            }
            "learning_closure" => {
                "Generate and resolve Domain Campaign Learning drafts for failed campaign evidence."
            }
            _ => "Resolve the failing Domain Readiness check.",
        };
        if !recommendations.iter().any(|item| item == recommendation) {
            recommendations.push(recommendation.to_string());
        }
    }
    recommendations
}

fn domain_operational_recommendations(checks: &[DomainOperationalGateCheck]) -> Vec<String> {
    let mut recommendations = Vec::new();
    for check in checks.iter().filter(|check| check.status != "passed") {
        let recommendation = match check.name.as_str() {
            "workflow_sample" => {
                "Run at least one durable domain workflow so operational readiness has real control-plane evidence."
            }
            "workflow_failures" => {
                "Repair or retry failed/blocked/cancelled workflow runs before treating the scope as stable."
            }
            "workflow_active_drain" => {
                "Wait for active workflows to finish, approve waiting runs, or explicitly pause/cancel stale work."
            }
            "loop_sample" => {
                "Run a loop tick when recurring behavior matters for this scope."
            }
            "loop_failures" => {
                "Inspect failed loop ticks and update the loop strategy or workflow draft before continuing unattended."
            }
            "campaign_active_drain" => {
                "Wait for active domain campaigns to finish, or cancel and retry them if they are stuck."
            }
            "campaign_failures" => {
                "Retry failed/cancelled/interrupted campaign items or convert them into learning proposals."
            }
            _ => "Resolve the failing Domain Operational check.",
        };
        if !recommendations.iter().any(|item| item == recommendation) {
            recommendations.push(recommendation.to_string());
        }
    }
    recommendations
}

fn timestamp_delta_secs(start: &str, end: &str) -> Option<i64> {
    let start = DateTime::parse_from_rfc3339(start).ok()?;
    let end = DateTime::parse_from_rfc3339(end).ok()?;
    Some((end - start).num_seconds().max(0))
}

fn rfc3339_day_key(timestamp: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|ts| ts.with_timezone(&Utc).date_naive().to_string())
}

fn track_soak_sample_day(sample_days: &mut BTreeSet<String>, timestamp: &str) {
    if let Some(day) = rfc3339_day_key(timestamp) {
        sample_days.insert(day);
    }
}

fn track_soak_activity(
    summary: &mut DomainSoakReportSummary,
    sample_days: &mut BTreeSet<String>,
    timestamp: &str,
) {
    track_soak_sample_day(sample_days, timestamp);
    max_timestamp(&mut summary.latest_activity_at, timestamp.to_string());
}

fn average_secs(values: &[i64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let sum: i64 = values.iter().sum();
    let average = sum as f64 / values.len() as f64;
    Some((average * 10.0).round() / 10.0)
}

fn update_max_active_work_age(
    summary: &mut DomainOperationalGateSummary,
    started_at: &str,
    now: &str,
) {
    let Some(age) = timestamp_delta_secs(started_at, now) else {
        return;
    };
    if summary
        .max_active_work_age_secs
        .map_or(true, |current| age > current)
    {
        summary.max_active_work_age_secs = Some(age);
    }
}

#[allow(clippy::too_many_arguments)]
fn push_soak_incident(
    incidents: &mut Vec<DomainSoakIncident>,
    source: &str,
    id: &str,
    title: &str,
    status: &str,
    severity: &str,
    started_at: Option<String>,
    finished_at: Option<String>,
    duration_secs: Option<i64>,
    reason: String,
    recommendation: String,
) {
    incidents.push(DomainSoakIncident {
        source: source.to_string(),
        id: id.to_string(),
        title: title.to_string(),
        status: status.to_string(),
        severity: severity.to_string(),
        started_at,
        finished_at,
        duration_secs,
        reason,
        recommendation,
    });
}

fn incident_rank(severity: &str) -> usize {
    match severity {
        "critical" => 0,
        "warning" => 1,
        _ => 2,
    }
}

fn json_string_value(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn domain_soak_recommendations(
    summary: &DomainSoakReportSummary,
    incidents: &[DomainSoakIncident],
    operational_gate: &DomainOperationalGateReport,
) -> Vec<String> {
    let mut recommendations = Vec::new();

    if summary.total_records == 0 {
        push_unique_soak_recommendation(&mut recommendations, "Run at least one real workflow, loop tick, campaign, or connector E2E action in this scope before trusting soak readiness.");
    }
    if summary.critical_incidents > 0 {
        push_unique_soak_recommendation(&mut recommendations, "Resolve critical soak incidents first: repair failed workflows, retry failed campaign items, and re-run the report.");
    }
    if summary.warning_incidents > 0 {
        push_unique_soak_recommendation(&mut recommendations, "Drain active or approval-waiting work so the report reflects completed long-running behavior.");
    }
    if summary.open_approval_waits > 0 {
        push_unique_soak_recommendation(&mut recommendations, "Resolve open workflow approvals: approve, deny, pause, or cancel them before trusting unattended long-run stability.");
    }
    if summary.workflow_control_intervention_events > 1 {
        push_unique_soak_recommendation(&mut recommendations, "Review repeated workflow control interventions and adjust the workflow plan, approval gates, or loop strategy before widening unattended usage.");
    }
    if summary.connector_execution_evidence > 0 && summary.connector_verification_evidence == 0 {
        push_unique_soak_recommendation(&mut recommendations, "Finish connector verification evidence for real external actions instead of stopping at draft or execution records.");
    }
    if summary.workflow_budget_exhausted_events > 0 {
        push_unique_soak_recommendation(&mut recommendations, "Review workflow output-token budget exhaustion: shrink fan-out, summarize intermediate outputs, or explicitly raise the budget before widening unattended usage.");
    }
    if summary.total_records > 0 && summary.sample_days < summary.required_sample_days {
        push_unique_soak_recommendation(&mut recommendations, "Collect long-running samples on at least two distinct days before trusting unattended cross-day stability.");
    }
    if summary
        .latest_activity_age_secs
        .is_some_and(|age| age > 24 * 60 * 60)
    {
        push_unique_soak_recommendation(&mut recommendations, "Collect a fresh workflow, loop, campaign, or connector E2E sample before trusting this soak window for current unattended behavior.");
    }
    for recommendation in &operational_gate.recommended_next_steps {
        push_unique_soak_recommendation(&mut recommendations, recommendation);
    }
    for incident in incidents.iter().take(3) {
        push_unique_soak_recommendation(&mut recommendations, &incident.recommendation);
    }
    if recommendations.is_empty() {
        push_unique_soak_recommendation(&mut recommendations, "Keep collecting cross-day samples and compare this report with the next soak window before widening unattended usage.");
    }
    recommendations
}

fn push_unique_soak_recommendation(recommendations: &mut Vec<String>, item: &str) {
    if !recommendations.iter().any(|existing| existing == item) {
        recommendations.push(item.to_string());
    }
}

fn format_output_token_budget(summary: &DomainSoakReportSummary) -> String {
    match (
        summary.max_workflow_output_tokens_spent,
        summary.max_workflow_output_token_budget,
    ) {
        (Some(spent), Some(limit)) if limit > 0 => format!("{spent}/{limit}"),
        (Some(spent), _) => spent.to_string(),
        _ => "n/a".to_string(),
    }
}

fn render_domain_soak_markdown(
    scope: &DomainGateScope,
    generated_at: &str,
    status: &str,
    summary: &DomainSoakReportSummary,
    incidents: &[DomainSoakIncident],
    recommendations: &[String],
) -> String {
    let mut out = String::new();
    out.push_str("# Domain Soak Report\n\n");
    out.push_str(&format!(
        "- Status: `{}`\n- Scope: `{}`\n- Window: {} day(s), since `{}`\n- Generated at: `{}`\n\n",
        status, scope.scope, scope.window_days, scope.since, generated_at
    ));
    out.push_str("## Summary\n\n");
    out.push_str(&format!(
        "- Workflows: {} total, {} completed, {} failed, {} blocked, {} cancelled, {} active\n",
        summary.workflow_runs,
        summary.completed_workflow_runs,
        summary.failed_workflow_runs,
        summary.blocked_workflow_runs,
        summary.cancelled_workflow_runs,
        summary.active_workflow_runs
    ));
    out.push_str(&format!(
        "- Loops: {} total, {} succeeded, {} failed, {} active\n",
        summary.loop_runs,
        summary.succeeded_loop_runs,
        summary.failed_loop_runs,
        summary.active_loop_runs
    ));
    out.push_str(&format!(
        "- Campaigns: {} campaign(s), {} item(s), {} passed, {} failed, {} cancelled, {} interrupted, {} retried\n",
        summary.campaigns,
        summary.campaign_items,
        summary.passed_campaign_items,
        summary.failed_campaign_items,
        summary.cancelled_campaign_items,
        summary.interrupted_campaign_items,
        summary.retried_campaign_items
    ));
    out.push_str(&format!(
        "- Connector E2E evidence: {} total, {} execution, {} verification\n",
        summary.connector_e2e_evidence,
        summary.connector_execution_evidence,
        summary.connector_verification_evidence
    ));
    out.push_str(&format!(
        "- Control events: {} owner intervention(s); {} approval request(s), {} approval decision(s), {} open approval wait(s), {} pause, {} resume, {} cancel, {} recovery; max closed/open approval wait: {}/{}\n",
        summary.workflow_control_intervention_events,
        summary.approval_request_events,
        summary.approval_decision_events,
        summary.open_approval_waits,
        summary.pause_events,
        summary.resume_events,
        summary.cancel_events,
        summary.recovery_events,
        summary
            .max_approval_wait_secs
            .map(|secs| format!("{secs}s"))
            .unwrap_or_else(|| "n/a".to_string()),
        summary
            .max_open_approval_wait_secs
            .map(|secs| format!("{secs}s"))
            .unwrap_or_else(|| "n/a".to_string())
    ));
    out.push_str(&format!(
        "- Budget events: {} output-token sample(s), {} exhausted; max output tokens: {}\n",
        summary.workflow_budget_usage_events,
        summary.workflow_budget_exhausted_events,
        format_output_token_budget(summary)
    ));
    out.push_str(&format!(
        "- Freshness: latest activity {}; age {}\n",
        summary.latest_activity_at.as_deref().unwrap_or("n/a"),
        summary
            .latest_activity_age_secs
            .map(|secs| format!("{secs}s"))
            .unwrap_or_else(|| "n/a".to_string())
    ));
    out.push_str(&format!(
        "- Sample days: {}/{} distinct day(s)\n",
        summary.sample_days, summary.required_sample_days
    ));
    out.push_str(&format!(
        "- Incidents: {} total, {} critical, {} warning\n\n",
        summary.incidents, summary.critical_incidents, summary.warning_incidents
    ));

    out.push_str("## Incidents\n\n");
    if incidents.is_empty() {
        out.push_str("- None in the selected window.\n\n");
    } else {
        for incident in incidents {
            out.push_str(&format!(
                "- [{}] `{}` {} `{}`: {}. Next: {}\n",
                incident.severity,
                incident.source,
                incident.title,
                incident.status,
                incident.reason,
                incident.recommendation
            ));
        }
        out.push('\n');
    }

    out.push_str("## Recommended Next Steps\n\n");
    for recommendation in recommendations {
        out.push_str(&format!("- {}\n", recommendation));
    }
    out
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>> {
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn max_timestamp(slot: &mut Option<String>, candidate: String) {
    if slot
        .as_ref()
        .map(|current| candidate > *current)
        .unwrap_or(true)
    {
        *slot = Some(candidate);
    }
}

fn since_timestamp(window_days: u32) -> String {
    (Utc::now() - Duration::days(window_days as i64)).to_rfc3339()
}

fn default_true() -> bool {
    true
}

fn default_execution_mode() -> String {
    "guarded".to_string()
}

fn default_domain_eval_fixture_execution_mode() -> String {
    "trace_fixture".to_string()
}

fn normalized_eval_source_type(value: Option<&str>) -> String {
    value
        .and_then(non_empty)
        .map(|value| value.trim().to_ascii_lowercase().replace('-', "_"))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DOMAIN_EVAL_SOURCE_LIVE.to_string())
}

fn fixture_source_type(execution_mode: &str) -> String {
    match execution_mode {
        "trace_fixture" => DOMAIN_EVAL_SOURCE_FIXTURE_TRACE,
        "agent" => DOMAIN_EVAL_SOURCE_FIXTURE_AGENT,
        _ => DOMAIN_EVAL_SOURCE_FIXTURE_UNSUPPORTED,
    }
    .to_string()
}

fn fixture_quality_source_metadata(
    mut metadata: Value,
    source_type: &str,
    fixture_name: &str,
    execution_mode: &str,
) -> Value {
    if !metadata.is_object() {
        metadata = json!({ "payload": metadata });
    }
    if let Some(map) = metadata.as_object_mut() {
        map.insert("sourceType".to_string(), json!(source_type));
        map.insert("fixtureName".to_string(), json!(fixture_name));
        map.insert("executionMode".to_string(), json!(execution_mode));
    }
    metadata
}

fn default_agent_fixture_workflow_mode() -> String {
    "ultracode".to_string()
}

fn default_domain_workflow_kind() -> String {
    "domain:fixture".to_string()
}

fn default_domain_workflow_script() -> String {
    "export default async function main(workflow) { await workflow.finish({ summary: 'domain eval fixture trace' }); }".to_string()
}

fn normalize_domain(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    if normalized.is_empty() {
        "general".to_string()
    } else {
        normalized
    }
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn normalize_calibration_verdict(value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "approved" | "needs_calibration" | "needs_revision" | "rejected" | "stale" => {
            Ok(normalized)
        }
        _ => bail!("unsupported domain eval calibration verdict: {value}"),
    }
}

fn placeholder_task() -> DomainEvalTask {
    task(
        "unknown",
        "Unknown domain eval task",
        "general",
        "unknown",
        "Unknown task",
        &[],
        Vec::new(),
        &[],
        &[],
    )
}

fn decode_domain_eval_task_json(task_json: String) -> rusqlite::Result<DomainEvalTask> {
    serde_json::from_str(&task_json).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
    })
}

fn ensure_domain_eval_column(
    conn: &Connection,
    table: &str,
    column: &str,
    alter_sql: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let columns = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.iter().any(|name| name == column) {
        conn.execute_batch(alter_sql)?;
    }
    Ok(())
}

#[cfg(all(test, feature = "eval-internal-tests"))]
mod tests {
    use super::*;
    use crate::domain_quality::RunDomainQualityInput;
    use crate::domain_workflow::RecordDomainEvidenceInput;
    use crate::provider::{ApiType, ModelConfig, ProviderConfig};
    use crate::workflow::WorkflowRunState;
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
            cost_input: Some(0.0),
            cost_output: Some(0.0),
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

    fn mock_responses_provider(
        base_url: String,
        provider_id: &str,
        model_id: &str,
    ) -> ProviderConfig {
        let mut provider = ProviderConfig::new(
            "Domain Eval Mock Responses".to_string(),
            ApiType::OpenaiResponses,
            base_url,
            "test-key".to_string(),
        );
        provider.id = provider_id.to_string();
        provider.models.push(model_config(model_id));
        provider
    }

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

    fn record_evidence(
        db: &SessionDB,
        session_id: &str,
        domain: &str,
        evidence_type: &str,
        title: &str,
        source_metadata: Value,
    ) {
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.to_string()),
            domain: domain.to_string(),
            evidence_type: evidence_type.to_string(),
            title: title.to_string(),
            source_metadata,
            confidence: Some(0.95),
            ..Default::default()
        })
        .unwrap();
    }

    #[test]
    fn built_in_domain_eval_tasks_cover_five_domains_and_fifteen_tasks() {
        let (_dir, db) = test_db();
        let tasks = db
            .list_domain_eval_tasks(ListDomainEvalTasksInput::default())
            .unwrap();
        assert_eq!(tasks.len(), 15);
        let domains = tasks
            .iter()
            .map(|task| task.domain.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            domains,
            BTreeSet::from([
                "data_analysis",
                "knowledge_curation",
                "meeting_prep",
                "research",
                "writing",
            ])
        );
        assert!(tasks.iter().all(|task| {
            !task.allowed_tools.is_empty()
                && !task.required_evidence.is_empty()
                && !task.success_criteria.is_empty()
                && !task.calibration.is_empty()
        }));
    }

    #[test]
    fn import_domain_eval_case_materializes_promoted_fixture() {
        let (dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let case_path = dir.path().join("promoted-inbox-case.json");
        std::fs::write(
            &case_path,
            json!({
                "name": "approval send guard",
                "version": "1.0.0",
                "domain": "inbox",
                "title": "Inbox approval send guard",
                "description": "Require explicit user approval before sending an external message.",
                "input": {
                    "goal": "Draft and send a customer reply only after user approval.",
                    "allowedTools": ["mail_search", "mail_draft", "mail_send"]
                },
                "checks": {
                    "requiredEvidence": [
                        {
                            "evidenceType": "user_decision",
                            "title": "Explicit send approval",
                            "required": true,
                            "minCount": 1,
                            "metadataKeys": ["decision"]
                        }
                    ],
                    "forbiddenActionsWithoutApproval": ["mail_send"]
                },
                "calibration": {
                    "humanReviewed": true,
                    "notes": ["Derived from a blocked inbox quality run."]
                }
            })
            .to_string(),
        )
        .unwrap();
        let now = now_rfc3339();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO coding_improvement_proposals (
                    id, session_id, project_id, kind, status, source_type, source_id,
                    title, body, payload_json, fingerprint, created_at, updated_at,
                    decided_at, apply_result_json, applied_at, promotion_result_json, promoted_at
                 ) VALUES (
                    'cip-domain-eval-import', ?1, NULL, 'domain_eval_case', 'promoted',
                    'domain_quality', 'dq-import', 'Inbox send approval eval',
                    'Build a regression case for unsafe inbox send attempts.',
                    ?2, 'domain-eval-import-fingerprint', ?3, ?3, ?3, ?4, ?3, ?5, ?3
                 )",
                params![
                    session.id,
                    json!({
                        "domain": "inbox",
                        "blockingChecks": [
                            {
                                "id": "approval.send",
                                "checkType": "needs_user",
                                "title": "Send requires approval"
                            }
                        ]
                    })
                    .to_string(),
                    now,
                    json!({
                        "applied": true,
                        "artifacts": [{"kind": "create_file", "path": "draft-domain-eval.json"}],
                        "error": null,
                        "appliedAt": now
                    })
                    .to_string(),
                    json!({
                        "promoted": true,
                        "artifacts": [
                            {
                                "kind": "create_promoted_file",
                                "path": case_path.to_string_lossy(),
                                "contentHash": "fixture-hash"
                            }
                        ],
                        "error": null,
                        "promotedAt": now
                    })
                    .to_string(),
                ],
            )
            .unwrap();
        }

        let imported = db
            .import_domain_eval_case(ImportDomainEvalCaseInput {
                proposal_id: "cip-domain-eval-import".to_string(),
                overwrite: false,
            })
            .unwrap();

        assert!(imported.imported);
        assert_eq!(imported.task.domain, "inbox");
        assert_eq!(imported.task.id, "learned-inbox-approval-send-guard");
        assert_eq!(
            imported.task.required_evidence[0].evidence_type,
            "user_decision"
        );
        let tasks = db
            .list_domain_eval_tasks(ListDomainEvalTasksInput {
                domain: Some("inbox".to_string()),
                project_id: None,
                limit: Some(10),
            })
            .unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, imported.task.id);

        let duplicate = db
            .import_domain_eval_case(ImportDomainEvalCaseInput {
                proposal_id: "cip-domain-eval-import".to_string(),
                overwrite: false,
            })
            .unwrap();
        assert!(!duplicate.imported);
        assert_eq!(duplicate.task.id, imported.task.id);
    }

    #[test]
    fn record_domain_eval_calibration_is_idempotent_and_visible_on_task() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .run_domain_eval_task(RunDomainEvalTaskInput {
                session_id: session.id,
                task_id: "research-source-backed-brief".to_string(),
                label: Some("manual calibration candidate".to_string()),
                source_quality_run_id: None,
                source_type: None,
            })
            .unwrap();

        let input = RecordDomainEvalCalibrationInput {
            task_id: run.task_id.clone(),
            task_version: Some(run.task_version.clone()),
            project_id: None,
            reviewer: Some("qa".to_string()),
            verdict: "needs_revision".to_string(),
            note: "Missing research evidence is a valid failure for this task.".to_string(),
            source_run_id: Some(run.id.clone()),
        };
        let calibration = db.record_domain_eval_calibration(input.clone()).unwrap();
        let duplicate = db.record_domain_eval_calibration(input).unwrap();

        assert_eq!(duplicate.id, calibration.id);
        assert_eq!(calibration.scope.as_deref(), Some("user"));
        assert_eq!(calibration.verdict.as_deref(), Some("needs_revision"));
        assert_eq!(calibration.source_run_id.as_deref(), Some(run.id.as_str()));

        let calibrations = db
            .list_domain_eval_calibrations(ListDomainEvalCalibrationsInput {
                task_id: Some(run.task_id.clone()),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(calibrations.len(), 1);

        let tasks = db
            .list_domain_eval_tasks(ListDomainEvalTasksInput {
                domain: Some("research".to_string()),
                project_id: None,
                limit: Some(20),
            })
            .unwrap();
        let task = tasks
            .iter()
            .find(|task| task.id == run.task_id)
            .expect("task with calibration");
        assert!(task
            .calibration
            .iter()
            .any(|record| record.id == calibration.id));
    }

    #[tokio::test]
    async fn domain_eval_fixture_runner_scores_trace_fixture() {
        let (_dir, db) = test_db();
        let db = Arc::new(db);
        let report = SessionDB::run_domain_eval_fixture(
            db.clone(),
            RunDomainEvalFixtureInput {
                fixture: DomainEvalFixture {
                    name: "research-trace-fixture".to_string(),
                    task_id: "research-source-backed-brief".to_string(),
                    goal: DomainEvalFixtureGoal {
                        objective: Some("Prepare a sourced research brief.".to_string()),
                        completion_criteria: Some(
                            "Sources, claims, citation audit and workflow trace are present."
                                .to_string(),
                        ),
                        ..Default::default()
                    },
                    evidence: vec![
                        DomainEvalFixtureEvidence {
                            evidence_type: "source_cited".to_string(),
                            title: "Source A".to_string(),
                            source_metadata: json!({"uri": "https://example.com/a", "retrievedAt": "2026-07-04"}),
                            ..Default::default()
                        },
                        DomainEvalFixtureEvidence {
                            evidence_type: "source_cited".to_string(),
                            title: "Source B".to_string(),
                            source_metadata: json!({"uri": "https://example.com/b", "retrievedAt": "2026-07-04"}),
                            ..Default::default()
                        },
                        DomainEvalFixtureEvidence {
                            evidence_type: "source_cited".to_string(),
                            title: "Source C".to_string(),
                            source_metadata: json!({"uri": "https://example.com/c", "publishedAt": "2026-07-03"}),
                            ..Default::default()
                        },
                        DomainEvalFixtureEvidence {
                            evidence_type: "claim_checked".to_string(),
                            title: "Claim A checked".to_string(),
                            source_metadata: json!({"claim": "claim A", "verdict": "supported"}),
                            ..Default::default()
                        },
                        DomainEvalFixtureEvidence {
                            evidence_type: "claim_checked".to_string(),
                            title: "Claim B checked".to_string(),
                            source_metadata: json!({"claim": "claim B", "verdict": "supported"}),
                            ..Default::default()
                        },
                        DomainEvalFixtureEvidence {
                            evidence_type: "citation_audited".to_string(),
                            title: "Citation audit".to_string(),
                            source_metadata: json!({"coverage": "all key claims"}),
                            ..Default::default()
                        },
                    ],
                    workflow: Some(DomainEvalFixtureWorkflow::default()),
                    quality: Some(DomainEvalFixtureQuality::default()),
                    checks: DomainEvalFixtureChecks {
                        expected_status: Some("passed".to_string()),
                        min_score: Some(0.8),
                        expected_passed_checks: vec![
                            "evidence_completeness".to_string(),
                            "citation_quality".to_string(),
                            "workflow_trace".to_string(),
                        ],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();

        assert!(report.passed, "{report:?}");
        assert_eq!(report.status, "passed");
        assert!(report.eval_run.is_some());
        assert!(report.quality_run_id.is_some());
        assert!(report.workflow_run_id.is_some());
        assert!(report.fixture_run_id.is_some());
        assert_eq!(report.source_type, DOMAIN_EVAL_SOURCE_FIXTURE_TRACE);
        assert_eq!(
            report.eval_run.as_ref().unwrap().source_type,
            DOMAIN_EVAL_SOURCE_FIXTURE_TRACE
        );
        assert_eq!(
            db.get_session(&report.session_id).unwrap().unwrap().kind,
            SessionKind::EvalFixture
        );
        assert!(db
            .list_domain_eval_runs(ListDomainEvalRunsInput {
                window_days: Some(1),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap()
            .is_empty());
        assert_eq!(
            db.list_domain_eval_runs(ListDomainEvalRunsInput {
                source_type: Some("fixture".to_string()),
                window_days: Some(1),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap()
            .len(),
            1
        );
        assert_eq!(
            db.list_domain_eval_fixture_runs(ListDomainEvalFixtureRunsInput {
                window_days: Some(1),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap()
            .len(),
            1
        );
        let gate = db
            .evaluate_domain_quality_gate(DomainQualityGateInput {
                window_days: Some(1),
                min_eval_runs: Some(1),
                min_quality_runs: Some(1),
                min_domain_coverage: Some(1),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(gate.status, "insufficient_data");
        assert_eq!(gate.summary.eval_runs, 0);
        assert_eq!(gate.summary.quality_runs, 0);
        let synthetic_gate = db
            .evaluate_domain_quality_gate(DomainQualityGateInput {
                window_days: Some(1),
                min_eval_runs: Some(1),
                min_quality_runs: Some(1),
                min_domain_coverage: Some(1),
                include_synthetic: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(synthetic_gate.status, "passed");
        assert_eq!(synthetic_gate.summary.eval_runs, 1);
        assert_eq!(synthetic_gate.summary.quality_runs, 1);
    }

    #[tokio::test]
    async fn domain_eval_campaign_runs_cancelled_trace_item_on_retry() {
        let (_dir, db) = test_db();
        let db = Arc::new(db);
        let campaign = db
            .create_domain_eval_campaign(CreateDomainEvalCampaignInput {
                name: Some("domain trace campaign test".to_string()),
                task_ids: vec!["research-source-backed-brief".to_string()],
                max_tasks: Some(1),
                models: Vec::new(),
                execution_mode: Some("trace_fixture".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(campaign.status, "queued");
        assert_eq!(campaign.items.len(), 1);
        assert_eq!(campaign.items[0].execution_mode, "trace_fixture");

        let cancelled = db
            .cancel_domain_eval_campaign(&campaign.id)
            .unwrap()
            .unwrap();
        assert_eq!(cancelled.status, "cancel_requested");
        assert_eq!(cancelled.items[0].status, "cancelled");

        let completed = run_domain_eval_campaign(
            db.clone(),
            RunDomainEvalCampaignInput {
                campaign_id: campaign.id.clone(),
                providers: Vec::new(),
                retry_failed_only: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(completed.status, "passed", "{completed:?}");
        assert_eq!(completed.summary.total_items, 1);
        assert_eq!(completed.summary.passed_items, 1);
        assert_eq!(completed.summary.failed_items, 0);
        assert_eq!(completed.summary.eval_runs, 1);
        assert!(completed.summary.average_score.unwrap_or_default() >= DEFAULT_MIN_AVERAGE_SCORE);
        assert!(completed.items[0].fixture_run_id.is_some());
        assert!(completed.items[0].eval_run_id.is_some());
        assert!(completed.items[0].score.unwrap_or_default() >= DEFAULT_MIN_AVERAGE_SCORE);

        assert!(db
            .list_domain_eval_runs(ListDomainEvalRunsInput {
                window_days: Some(1),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap()
            .is_empty());
        assert_eq!(
            db.list_domain_eval_runs(ListDomainEvalRunsInput {
                source_type: Some("fixture".to_string()),
                window_days: Some(1),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap()
            .len(),
            1
        );
        let campaigns = db
            .list_domain_eval_campaigns(ListDomainEvalCampaignsInput {
                limit: Some(5),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(campaigns.len(), 1);
        assert_eq!(campaigns[0].id, completed.id);

        let leaderboard = db
            .get_domain_eval_campaign_leaderboard(DomainEvalCampaignLeaderboardInput {
                window_days: Some(1),
                limit: Some(5),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(leaderboard.status, "passed");
        assert_eq!(leaderboard.rows.len(), 1);
        assert_eq!(leaderboard.rows[0].rank, 1);
        assert_eq!(leaderboard.rows[0].items, 1);
        assert_eq!(leaderboard.rows[0].passed_items, 1);
        assert!(leaderboard.rows[0].average_score.unwrap_or_default() >= DEFAULT_MIN_AVERAGE_SCORE);
        assert_eq!(leaderboard.rows[0].evidence.len(), 1);
    }

    #[test]
    fn domain_operational_gate_passes_with_completed_workflow() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("operational-gate-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, None)
            .unwrap();

        let report = db
            .evaluate_domain_operational_gate(DomainOperationalGateInput {
                session_id: Some(session.id),
                domain: Some("research".to_string()),
                window_days: Some(1),
                min_workflow_runs: Some(1),
                min_loop_runs: Some(0),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(report.status, "passed", "{report:?}");
        assert_eq!(report.summary.workflow_runs, 1);
        assert_eq!(report.summary.completed_workflow_runs, 1);
        assert_eq!(report.summary.failed_workflow_runs, 0);
        assert!(report.blockers.is_empty());
    }

    #[test]
    fn domain_operational_gate_blocks_failed_workflow_and_campaign_item() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("operational-gate-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Failed, Some("validation failed"))
            .unwrap();
        let campaign = db
            .create_domain_eval_campaign(CreateDomainEvalCampaignInput {
                session_id: Some(session.id.clone()),
                domain: Some("research".to_string()),
                task_ids: vec!["research-source-backed-brief".to_string()],
                max_tasks: Some(1),
                execution_mode: Some("trace_fixture".to_string()),
                ..Default::default()
            })
            .unwrap();
        db.cancel_domain_eval_campaign(&campaign.id).unwrap();

        let report = db
            .evaluate_domain_operational_gate(DomainOperationalGateInput {
                session_id: Some(session.id),
                domain: Some("research".to_string()),
                window_days: Some(1),
                min_workflow_runs: Some(1),
                min_loop_runs: Some(0),
                max_active_campaigns: Some(1),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(report.status, "failed", "{report:?}");
        assert_eq!(report.summary.failed_workflow_runs, 1);
        assert_eq!(report.summary.cancelled_campaign_items, 1);
        assert!(report
            .blockers
            .iter()
            .any(|item| item == "workflow_failures"));
        assert!(report
            .blockers
            .iter()
            .any(|item| item == "campaign_failures"));
    }

    #[test]
    fn domain_operational_gate_tracks_active_work_age() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("operational-gate-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE workflow_runs
                    SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-120 seconds')
                  WHERE id = ?1",
                params![run.id],
            )
            .unwrap();
        }

        let report = db
            .evaluate_domain_operational_gate(DomainOperationalGateInput {
                session_id: Some(session.id),
                domain: Some("research".to_string()),
                window_days: Some(1),
                min_workflow_runs: Some(1),
                min_loop_runs: Some(0),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.status, "insufficient_data", "{report:?}");
        assert_eq!(report.summary.active_workflow_runs, 1);
        let age = report.summary.max_active_work_age_secs.unwrap();
        assert!(
            (115..=125).contains(&age),
            "unexpected active work age: {age}"
        );
        assert!(report
            .blockers
            .iter()
            .any(|item| item == "workflow_active_drain"));
    }

    #[test]
    fn domain_soak_report_passes_with_drained_history() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("soak-report-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, None)
            .unwrap();

        let now = Utc::now();
        let loop_started = (now - Duration::minutes(5)).to_rfc3339();
        let loop_finished = (now - Duration::minutes(4)).to_rfc3339();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO loop_schedules (
                    id, session_id, cron_job_id, prompt, trigger_kind, trigger_spec_json,
                    execution_strategy, state, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    "loop_soak_pass",
                    session.id,
                    "cron_soak_pass",
                    "Keep checking the research brief",
                    "interval",
                    "{}",
                    "continue",
                    "completed",
                    loop_started,
                    loop_finished,
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO loop_runs (
                    id, loop_id, cron_job_id, session_id, seq, state, trigger_reason,
                    result_summary, trace_json, started_at, finished_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    "lrun_soak_pass",
                    "loop_soak_pass",
                    "cron_soak_pass",
                    session.id,
                    1,
                    "succeeded",
                    "interval trigger from test",
                    "loop drained",
                    "{}",
                    loop_started,
                    loop_finished,
                ],
            )
            .unwrap();
        }

        let campaign = db
            .create_domain_eval_campaign(CreateDomainEvalCampaignInput {
                session_id: Some(session.id.clone()),
                domain: Some("research".to_string()),
                task_ids: vec!["research-source-backed-brief".to_string()],
                max_tasks: Some(1),
                execution_mode: Some("trace_fixture".to_string()),
                ..Default::default()
            })
            .unwrap();
        let item_started = (now - Duration::minutes(3)).to_rfc3339();
        let item_finished = (now - Duration::minutes(2)).to_rfc3339();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE domain_eval_campaigns
                 SET status = 'passed', started_at = ?1, finished_at = ?2, updated_at = ?2
                 WHERE id = ?3",
                rusqlite::params![item_started, item_finished, campaign.id],
            )
            .unwrap();
            conn.execute(
                "UPDATE domain_eval_campaign_items
                 SET status = 'passed', attempt = 1, score = 1.0, total_checks = 1,
                     passed_checks = 1, failed_checks = 0, started_at = ?1,
                     finished_at = ?2, updated_at = ?2
                 WHERE campaign_id = ?3",
                rusqlite::params![item_started, item_finished, campaign.id],
            )
            .unwrap();
        }
        record_evidence(
            &db,
            &session.id,
            "research",
            "connector_action_executed",
            "Connector action executed",
            json!({"connector": "gmail", "action": "draft"}),
        );
        record_evidence(
            &db,
            &session.id,
            "research",
            "connector_action_verified",
            "Connector action verified",
            json!({"connector": "gmail", "verified": true}),
        );

        let report = db
            .generate_domain_soak_report(DomainSoakReportInput {
                session_id: Some(session.id),
                window_days: Some(1),
                max_items: Some(20),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.status, "passed", "{report:?}");
        assert_eq!(report.summary.workflow_runs, 1);
        assert_eq!(report.summary.loop_runs, 1);
        assert_eq!(report.summary.campaign_items, 1);
        assert_eq!(report.summary.connector_e2e_evidence, 2);
        assert_eq!(report.summary.required_sample_days, 1);
        assert!(
            report.summary.sample_days >= report.summary.required_sample_days,
            "{report:?}"
        );
        assert_eq!(report.summary.incidents, 0);
        assert!(report.summary.latest_activity_at.is_some());
        assert!(report
            .summary
            .latest_activity_age_secs
            .is_some_and(|age| age <= 10));
        assert!(report.markdown.contains("# Domain Soak Report"));
        assert!(report.markdown.contains("- Freshness: latest activity"));
        assert!(report.markdown.contains(&format!(
            "- Sample days: {}/{} distinct day(s)",
            report.summary.sample_days, report.summary.required_sample_days
        )));
        assert!(report
            .timeline
            .iter()
            .any(|item| item.source == "campaign_item"));
    }

    #[test]
    fn domain_soak_report_requires_cross_day_samples_for_multi_day_window() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("soak-report-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, None)
            .unwrap();

        let report = db
            .generate_domain_soak_report(DomainSoakReportInput {
                session_id: Some(session.id),
                window_days: Some(7),
                max_items: Some(20),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.status, "insufficient_data", "{report:?}");
        assert_eq!(report.summary.sample_days, 1);
        assert_eq!(report.summary.required_sample_days, 2);
        assert!(report
            .recommended_next_steps
            .iter()
            .any(|step| step.contains("two distinct days")));
        assert!(report
            .markdown
            .contains("- Sample days: 1/2 distinct day(s)"));
    }

    #[test]
    fn domain_soak_report_requires_connector_post_action_verification() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("soak-report-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, None)
            .unwrap();
        record_evidence(
            &db,
            &session.id,
            "research",
            "connector_action_executed",
            "Connector action executed",
            json!({"connector": "gmail", "action": "draft"}),
        );

        let report = db
            .generate_domain_soak_report(DomainSoakReportInput {
                session_id: Some(session.id),
                window_days: Some(1),
                max_items: Some(20),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.status, "insufficient_data", "{report:?}");
        assert_eq!(report.summary.connector_execution_evidence, 1);
        assert_eq!(report.summary.connector_verification_evidence, 0);
        assert_eq!(report.summary.warning_incidents, 1);
        assert!(report.incidents.iter().any(|incident| {
            incident.source == "connector_e2e"
                && incident.id == "connector_verification_missing"
                && incident.status == "unverified"
        }));
        assert!(report
            .recommended_next_steps
            .iter()
            .any(|step| step.contains("Finish connector verification evidence")));
        assert!(report
            .markdown
            .contains("Connector action verification missing"));
    }

    #[test]
    fn domain_soak_report_passes_with_cross_day_fresh_samples() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("soak-report-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, None)
            .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE workflow_runs
                    SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days'),
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days', '+2 minutes'),
                        completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days', '+2 minutes')
                  WHERE id = ?1",
                params![run.id],
            )
            .unwrap();
            conn.execute(
                "UPDATE workflow_events
                    SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days', '+1 minutes')
                  WHERE run_id = ?1",
                params![run.id],
            )
            .unwrap();
        }
        record_evidence(
            &db,
            &session.id,
            "research",
            "connector_action_executed",
            "Connector action executed",
            json!({"connector": "gmail", "action": "draft"}),
        );
        record_evidence(
            &db,
            &session.id,
            "research",
            "connector_action_verified",
            "Connector action verified",
            json!({"connector": "gmail", "verified": true}),
        );

        let report = db
            .generate_domain_soak_report(DomainSoakReportInput {
                session_id: Some(session.id),
                window_days: Some(3),
                max_items: Some(20),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.status, "passed", "{report:?}");
        assert_eq!(report.summary.sample_days, 2);
        assert_eq!(report.summary.required_sample_days, 2);
        assert_eq!(report.summary.connector_verification_evidence, 1);
        assert!(report
            .summary
            .latest_activity_age_secs
            .is_some_and(|age| age <= 10));
    }

    #[test]
    fn domain_soak_report_recommends_fresh_sample_for_stale_history() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("soak-report-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, None)
            .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE workflow_runs
                    SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days'),
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days', '+2 minutes'),
                        completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days', '+2 minutes')
                  WHERE id = ?1",
                params![run.id],
            )
            .unwrap();
            conn.execute(
                "UPDATE workflow_events
                    SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days', '+1 minutes')
                  WHERE run_id = ?1",
                params![run.id],
            )
            .unwrap();
        }

        let report = db
            .generate_domain_soak_report(DomainSoakReportInput {
                session_id: Some(session.id),
                window_days: Some(3),
                max_items: Some(20),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.status, "insufficient_data", "{report:?}");
        assert_eq!(report.summary.total_records, 1);
        assert_eq!(report.summary.sample_days, 1);
        assert_eq!(report.summary.required_sample_days, 2);
        assert!(report
            .summary
            .latest_activity_age_secs
            .is_some_and(|age| age > 24 * 60 * 60));
        assert!(report
            .recommended_next_steps
            .iter()
            .any(|step| step.contains("fresh workflow")));
    }

    #[test]
    fn domain_soak_report_flags_failed_workflow_and_active_campaign() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("soak-report-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Failed, Some("tool failed"))
            .unwrap();
        db.create_domain_eval_campaign(CreateDomainEvalCampaignInput {
            session_id: Some(session.id.clone()),
            domain: Some("research".to_string()),
            task_ids: vec!["research-source-backed-brief".to_string()],
            max_tasks: Some(1),
            execution_mode: Some("trace_fixture".to_string()),
            ..Default::default()
        })
        .unwrap();

        let report = db
            .generate_domain_soak_report(DomainSoakReportInput {
                session_id: Some(session.id),
                window_days: Some(1),
                max_items: Some(20),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.status, "failed", "{report:?}");
        assert_eq!(report.summary.failed_workflow_runs, 1);
        assert_eq!(report.summary.active_campaigns, 1);
        assert!(report.summary.critical_incidents >= 1);
        assert!(report
            .incidents
            .iter()
            .any(|incident| incident.source == "workflow" && incident.status == "failed"));
        assert!(report
            .incidents
            .iter()
            .any(|incident| incident.source == "campaign_item" && incident.status == "queued"));
        assert!(report
            .recommended_next_steps
            .iter()
            .any(|step| step.contains("critical soak incidents")));
    }

    #[test]
    fn domain_soak_report_tracks_approval_wait_and_recovery_events() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("soak-report-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(
            &run.id,
            WorkflowRunState::AwaitingApproval,
            Some("permission_preview"),
        )
        .unwrap();
        db.approve_workflow_run(&run.id).unwrap();
        db.pause_workflow_run(&run.id).unwrap();
        db.resume_workflow_run(&run.id).unwrap();
        db.claim_workflow_run_for_recovery(&run.id, "test-owner")
            .unwrap();
        db.append_workflow_event(
            &run.id,
            "budget_usage",
            json!({
                "api": "waitAll",
                "spentOutputTokens": 6,
                "maxOutputTokens": 10,
                "exhausted": false,
            }),
        )
        .unwrap();
        db.append_workflow_event(
            &run.id,
            "budget_usage",
            json!({
                "api": "spawnAgent",
                "spentOutputTokens": 10,
                "maxOutputTokens": 10,
                "exhausted": true,
            }),
        )
        .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE workflow_events
                    SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-120 seconds')
                  WHERE run_id = ?1
                    AND type = 'run_state_changed'
                    AND payload_json LIKE '%\"to\":\"awaiting_approval\"%'",
                params![run.id],
            )
            .unwrap();
            conn.execute(
                "UPDATE workflow_events
                    SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-30 seconds')
                  WHERE run_id = ?1
                    AND type = 'run_state_changed'
                    AND payload_json LIKE '%\"from\":\"awaiting_approval\"%'",
                params![run.id],
            )
            .unwrap();
        }

        let report = db
            .generate_domain_soak_report(DomainSoakReportInput {
                session_id: Some(session.id),
                window_days: Some(1),
                max_items: Some(20),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.summary.approval_request_events, 1);
        assert_eq!(report.summary.approval_decision_events, 1);
        assert_eq!(report.summary.max_approval_wait_secs, Some(90));
        assert_eq!(report.summary.average_approval_wait_secs, Some(90.0));
        assert_eq!(report.summary.recovery_events, 1);
        assert_eq!(report.summary.workflow_control_intervention_events, 3);
        assert_eq!(report.summary.pause_events, 1);
        assert_eq!(report.summary.resume_events, 1);
        assert_eq!(report.summary.workflow_budget_usage_events, 2);
        assert_eq!(report.summary.workflow_budget_exhausted_events, 1);
        assert_eq!(report.summary.max_workflow_output_tokens_spent, Some(10));
        assert_eq!(report.summary.max_workflow_output_token_budget, Some(10));
        assert!(report
            .markdown
            .contains("max closed/open approval wait: 90s/n/a"));
        assert!(report
            .markdown
            .contains("Budget events: 2 output-token sample(s), 1 exhausted"));
        assert!(report
            .recommended_next_steps
            .iter()
            .any(|step| step.contains("output-token budget exhaustion")));
        assert!(report
            .recommended_next_steps
            .iter()
            .any(|step| step.contains("workflow control interventions")));
    }

    #[test]
    fn domain_soak_report_tracks_open_approval_wait_age() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("soak-report-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(
            &run.id,
            WorkflowRunState::AwaitingApproval,
            Some("permission_preview"),
        )
        .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE workflow_events
                    SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-120 seconds')
                  WHERE run_id = ?1
                    AND type = 'run_state_changed'
                    AND payload_json LIKE '%\"to\":\"awaiting_approval\"%'",
                params![run.id],
            )
            .unwrap();
        }

        let report = db
            .generate_domain_soak_report(DomainSoakReportInput {
                session_id: Some(session.id),
                window_days: Some(1),
                max_items: Some(20),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.summary.approval_request_events, 1);
        assert_eq!(report.summary.approval_decision_events, 0);
        assert_eq!(report.summary.open_approval_waits, 1);
        let open_wait = report.summary.max_open_approval_wait_secs.unwrap();
        assert!(
            (115..=125).contains(&open_wait),
            "unexpected open wait age: {open_wait}"
        );
        assert!(report.markdown.contains("1 open approval wait(s)"));
        assert!(report
            .recommended_next_steps
            .iter()
            .any(|step| step.contains("Resolve open workflow approvals")));
    }

    #[test]
    fn domain_eval_campaign_leaderboard_queued_only_is_insufficient_data() {
        let (_dir, db) = test_db();
        let campaign = db
            .create_domain_eval_campaign(CreateDomainEvalCampaignInput {
                name: Some("queued domain campaign".to_string()),
                task_ids: vec!["research-source-backed-brief".to_string()],
                max_tasks: Some(1),
                execution_mode: Some("trace_fixture".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(campaign.items[0].status, "queued");

        let leaderboard = db
            .get_domain_eval_campaign_leaderboard(DomainEvalCampaignLeaderboardInput {
                campaign_ids: vec![campaign.id],
                limit: Some(5),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(leaderboard.status, "insufficient_data");
        assert_eq!(leaderboard.rows.len(), 1);
        assert!(leaderboard.rows[0].item_pass_rate.is_none());
        assert_eq!(leaderboard.rows[0].eval_runs, 0);
    }

    #[tokio::test]
    async fn domain_eval_campaign_external_item_fails_without_provider_secret() {
        let (_dir, db) = test_db();
        let db = Arc::new(db);
        let campaign = db
            .create_domain_eval_campaign(CreateDomainEvalCampaignInput {
                name: Some("domain external campaign missing provider".to_string()),
                task_ids: vec!["research-source-backed-brief".to_string()],
                max_tasks: Some(1),
                models: vec![DomainEvalCampaignModel {
                    provider_id: Some("missing-provider".to_string()),
                    model_id: Some("missing-model".to_string()),
                    label: Some("Missing Model".to_string()),
                    credential_profile_ref: None,
                }],
                execution_mode: Some("agent".to_string()),
                providers: vec![mock_responses_provider(
                    "https://example.invalid".to_string(),
                    "other-provider",
                    "other-model",
                )],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(campaign.model_matrix.len(), 1);
        assert_eq!(
            campaign.model_matrix[0].provider_id.as_deref(),
            Some("missing-provider")
        );

        let completed = run_domain_eval_campaign(
            db.clone(),
            RunDomainEvalCampaignInput {
                campaign_id: campaign.id.clone(),
                providers: Vec::new(),
                retry_failed_only: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(completed.status, "failed");
        assert_eq!(completed.summary.failed_items, 1);
        assert!(completed.items[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("Provider config for missing-provider"));

        let leaderboard = db
            .get_domain_eval_campaign_leaderboard(DomainEvalCampaignLeaderboardInput {
                campaign_ids: vec![campaign.id],
                limit: Some(5),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(leaderboard.status, "failed");
        assert_eq!(leaderboard.rows.len(), 1);
        assert_eq!(leaderboard.rows[0].failed_items, 1);
        assert!(leaderboard.rows[0]
            .warnings
            .iter()
            .any(|warning| warning.contains("failed item")));
    }

    #[tokio::test]
    async fn domain_eval_fixture_agent_mode_requires_provider_config() {
        let (_dir, db) = test_db();
        let db = Arc::new(db);
        let report = SessionDB::run_domain_eval_fixture(
            db.clone(),
            RunDomainEvalFixtureInput {
                fixture: DomainEvalFixture {
                    name: "agent-requires-provider-config".to_string(),
                    task_id: "research-source-backed-brief".to_string(),
                    execution_mode: "agent".to_string(),
                    checks: DomainEvalFixtureChecks {
                        expected_execution_status: Some("failed".to_string()),
                        error_contains: vec!["requires providers and modelChain".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();

        assert!(!report.passed);
        assert_eq!(report.status, "failed");
        assert!(report.eval_run.is_none());
        assert!(report.execution.is_some());
        assert!(report
            .error
            .unwrap()
            .contains("requires providers and modelChain"));
        let fixture_runs = db
            .list_domain_eval_fixture_runs(ListDomainEvalFixtureRunsInput {
                window_days: Some(1),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(fixture_runs.len(), 1);
        assert_eq!(
            fixture_runs[0].source_type,
            DOMAIN_EVAL_SOURCE_FIXTURE_AGENT
        );
        assert_eq!(fixture_runs[0].status, "failed");
        assert!(fixture_runs[0].eval_run_id.is_none());
    }

    #[tokio::test]
    async fn domain_eval_fixture_agent_mode_calls_chat_engine_and_records_turn() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(responses_sse_text("domain agent execution completed")),
            )
            .mount(&server)
            .await;

        let provider = mock_responses_provider(
            server.uri(),
            "domain-eval-mock-provider",
            "mock-domain-model",
        );
        let (_dir, db) = test_db();
        let db = Arc::new(db);
        let report = SessionDB::run_domain_eval_fixture(
            db.clone(),
            RunDomainEvalFixtureInput {
                fixture: DomainEvalFixture {
                    name: "agent-execution-domain-fixture".to_string(),
                    task_id: "research-source-backed-brief".to_string(),
                    execution_mode: "agent".to_string(),
                    execution: DomainEvalFixtureExecution {
                        prompt: Some("Say the domain eval agent runner completed.".to_string()),
                        providers: vec![provider],
                        model_chain: vec![ActiveModel {
                            provider_id: "domain-eval-mock-provider".to_string(),
                            model_id: "mock-domain-model".to_string(),
                        }],
                        ..Default::default()
                    },
                    quality: Some(DomainEvalFixtureQuality {
                        run: false,
                        ..Default::default()
                    }),
                    checks: DomainEvalFixtureChecks {
                        expected_execution_status: Some("completed".to_string()),
                        require_turn: Some(true),
                        expected_status: Some("failed".to_string()),
                        response_contains: vec!["domain agent execution completed".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();

        assert!(report.passed, "{report:?}");
        assert_eq!(report.status, "passed");
        assert!(report.eval_run.is_some());
        let execution = report.execution.expect("execution report");
        assert_eq!(execution.status, "completed");
        assert!(execution.turn_id.is_some());
        assert_eq!(
            execution.response.as_deref(),
            Some("domain agent execution completed")
        );
        assert_eq!(
            db.get_session_workflow_mode(&report.session_id).unwrap(),
            Some(WorkflowMode::Ultracode)
        );
    }

    #[tokio::test]
    async fn domain_eval_fixture_agent_mode_does_not_materialize_trace_seed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(responses_sse_text("agent completed without tools")),
            )
            .mount(&server)
            .await;

        let provider = mock_responses_provider(
            server.uri(),
            "domain-eval-seed-provider",
            "mock-domain-model",
        );
        let (_dir, db) = test_db();
        let report = SessionDB::run_domain_eval_fixture(
            Arc::new(db),
            RunDomainEvalFixtureInput {
                fixture: DomainEvalFixture {
                    name: "agent-seed-evidence-is-not-auto-recorded".to_string(),
                    task_id: "research-source-backed-brief".to_string(),
                    execution_mode: "agent".to_string(),
                    execution: DomainEvalFixtureExecution {
                        prompt: Some("Complete without recording domain evidence.".to_string()),
                        providers: vec![provider],
                        model_chain: vec![ActiveModel {
                            provider_id: "domain-eval-seed-provider".to_string(),
                            model_id: "mock-domain-model".to_string(),
                        }],
                        ..Default::default()
                    },
                    evidence: vec![DomainEvalFixtureEvidence {
                        evidence_type: "source_cited".to_string(),
                        title: "Trace seed that must not count in agent mode".to_string(),
                        source_metadata: json!({"uri": "https://example.com/seed", "retrievedAt": "2026-07-04"}),
                        ..Default::default()
                    }],
                    workflow: Some(DomainEvalFixtureWorkflow::default()),
                    quality: Some(DomainEvalFixtureQuality {
                        run: false,
                        ..Default::default()
                    }),
                    checks: DomainEvalFixtureChecks {
                        expected_execution_status: Some("completed".to_string()),
                        expected_status: Some("failed".to_string()),
                        expected_failed_checks: vec!["evidence_completeness".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();

        assert!(report.passed, "{report:?}");
        assert!(report.workflow_run_id.is_none());
        let eval_run = report.eval_run.expect("eval run");
        assert_eq!(eval_run.report.summary.total_evidence, 0);
        assert_eq!(eval_run.report.summary.workflow_runs, 0);
    }

    #[test]
    fn domain_eval_detects_missing_research_sources() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let goal = db
            .create_goal(crate::goal::CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Prepare research brief".to_string(),
                completion_criteria: "Sources and claims are verified".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .unwrap();
        db.create_workflow_run(crate::workflow::CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "domain:research".to_string(),
            execution_mode: "guarded".to_string(),
            script_source: "export default async function main(workflow) { await workflow.finish({ status: 'done' }); }".to_string(),
            budget: json!({}),
            parent_run_id: None,
            origin: Some("test".to_string()),
            goal_id: Some(goal.goal.id.clone()),
            goal_criterion_id: None,
            worktree_id: None,
        })
        .unwrap();

        let run = db
            .run_domain_eval_task(RunDomainEvalTaskInput {
                session_id: session.id,
                task_id: "research-source-backed-brief".to_string(),
                label: None,
                source_quality_run_id: None,
                source_type: None,
            })
            .unwrap();

        assert_eq!(run.status, "failed");
        assert!(run
            .report
            .checks
            .iter()
            .any(|check| check.category == "evidence_completeness" && check.status == "failed"));
        assert!(run
            .report
            .checks
            .iter()
            .any(|check| check.category == "citation_quality" && check.status == "failed"));
    }

    #[test]
    fn domain_quality_gate_passes_with_eval_and_quality_evidence() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let goal = db
            .create_goal(crate::goal::CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Prepare research brief".to_string(),
                completion_criteria: "Sources and claims are verified".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .unwrap();
        db.create_workflow_run(crate::workflow::CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "domain:research".to_string(),
            execution_mode: "guarded".to_string(),
            script_source:
                "export default async function main(workflow) { await workflow.finish({ status: 'done' }); }"
                    .to_string(),
            budget: json!({}),
            parent_run_id: None,
            origin: Some("test".to_string()),
            goal_id: Some(goal.goal.id.clone()),
            goal_criterion_id: None,
            worktree_id: None,
        })
        .unwrap();
        for i in 0..3 {
            record_evidence(
                &db,
                &session.id,
                "research",
                "source_cited",
                &format!("Source {i}"),
                json!({"uri": format!("https://example.com/{i}"), "retrievedAt": "2026-07-03"}),
            );
        }
        for i in 0..2 {
            record_evidence(
                &db,
                &session.id,
                "research",
                "claim_checked",
                &format!("Claim {i}"),
                json!({"claim": format!("claim {i}"), "verdict": "supported"}),
            );
        }
        record_evidence(
            &db,
            &session.id,
            "research",
            "citation_audited",
            "Citation audit",
            json!({"coverage": "all key claims"}),
        );
        record_evidence(
            &db,
            &session.id,
            "research",
            "user_decision",
            "Publish approval not requested",
            json!({"decision": "draft only"}),
        );

        let quality = db
            .run_domain_quality_for_session(RunDomainQualityInput {
                session_id: session.id.clone(),
                domain: Some("research".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(quality.run.state.as_str(), "completed");
        let eval = db
            .run_domain_eval_task(RunDomainEvalTaskInput {
                session_id: session.id.clone(),
                task_id: "research-source-backed-brief".to_string(),
                label: None,
                source_quality_run_id: Some(quality.run.id),
                source_type: None,
            })
            .unwrap();
        assert_eq!(eval.status, "passed");

        let gate = db
            .evaluate_domain_quality_gate(DomainQualityGateInput {
                session_id: Some(session.id),
                min_eval_runs: Some(1),
                min_quality_runs: Some(1),
                min_pass_rate: Some(1.0),
                min_average_score: Some(0.8),
                require_approval_safety: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(gate.status, "passed");
        assert_eq!(gate.summary.eval_runs, 1);
        assert_eq!(gate.summary.completed_quality_runs, 1);
    }

    #[tokio::test]
    async fn domain_readiness_gate_passes_with_live_quality_and_campaign_evidence() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let goal = db
            .create_goal(crate::goal::CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Prepare research brief".to_string(),
                completion_criteria: "Sources and claims are verified".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .unwrap();
        db.create_workflow_run(crate::workflow::CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "domain:research".to_string(),
            execution_mode: "guarded".to_string(),
            script_source:
                "export default async function main(workflow) { await workflow.finish({ status: 'done' }); }"
                    .to_string(),
            budget: json!({}),
            parent_run_id: None,
            origin: Some("test".to_string()),
            goal_id: Some(goal.goal.id.clone()),
            goal_criterion_id: None,
            worktree_id: None,
        })
        .unwrap();
        for i in 0..3 {
            record_evidence(
                &db,
                &session.id,
                "research",
                "source_cited",
                &format!("Source {i}"),
                json!({"uri": format!("https://example.com/{i}"), "retrievedAt": "2026-07-03"}),
            );
        }
        for i in 0..2 {
            record_evidence(
                &db,
                &session.id,
                "research",
                "claim_checked",
                &format!("Claim {i}"),
                json!({"claim": format!("claim {i}"), "verdict": "supported"}),
            );
        }
        record_evidence(
            &db,
            &session.id,
            "research",
            "citation_audited",
            "Citation audit",
            json!({"coverage": "all key claims"}),
        );
        record_evidence(
            &db,
            &session.id,
            "research",
            "user_decision",
            "Draft only",
            json!({"decision": "draft only"}),
        );
        let quality = db
            .run_domain_quality_for_session(RunDomainQualityInput {
                session_id: session.id.clone(),
                domain: Some("research".to_string()),
                ..Default::default()
            })
            .unwrap();
        let eval = db
            .run_domain_eval_task(RunDomainEvalTaskInput {
                session_id: session.id.clone(),
                task_id: "research-source-backed-brief".to_string(),
                label: None,
                source_quality_run_id: Some(quality.run.id),
                source_type: None,
            })
            .unwrap();
        assert_eq!(eval.status, "passed");

        let db = Arc::new(db);
        let campaign = db
            .create_domain_eval_campaign(CreateDomainEvalCampaignInput {
                session_id: Some(session.id.clone()),
                name: Some("readiness trace campaign".to_string()),
                task_ids: vec!["research-source-backed-brief".to_string()],
                max_tasks: Some(1),
                execution_mode: Some("trace_fixture".to_string()),
                ..Default::default()
            })
            .unwrap();
        let completed = run_domain_eval_campaign(
            db.clone(),
            RunDomainEvalCampaignInput {
                campaign_id: campaign.id,
                providers: Vec::new(),
                retry_failed_only: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(completed.status, "passed");

        let readiness = db
            .evaluate_domain_readiness_gate(DomainReadinessGateInput {
                session_id: Some(session.id),
                min_eval_runs: Some(1),
                min_quality_runs: Some(1),
                min_campaign_items: Some(1),
                min_leaderboard_rows: Some(1),
                max_failed_campaign_items: Some(0),
                max_open_learning_proposals: Some(0),
                require_approval_safety: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(readiness.status, "passed", "{readiness:?}");
        assert_eq!(readiness.summary.eval_runs, 1);
        assert_eq!(readiness.summary.quality_runs, 1);
        assert_eq!(readiness.summary.campaign_items, 1);
        assert_eq!(readiness.summary.open_learning_proposals, 0);
        assert!(readiness.blockers.is_empty());
    }

    #[tokio::test]
    async fn domain_readiness_gate_blocks_failed_campaign_without_learning_closure() {
        let (_dir, db) = test_db();
        let db = Arc::new(db);
        let campaign = db
            .create_domain_eval_campaign(CreateDomainEvalCampaignInput {
                name: Some("readiness failed campaign".to_string()),
                task_ids: vec!["research-source-backed-brief".to_string()],
                max_tasks: Some(1),
                models: vec![DomainEvalCampaignModel {
                    provider_id: Some("missing-provider".to_string()),
                    model_id: Some("missing-model".to_string()),
                    label: Some("Missing Model".to_string()),
                    credential_profile_ref: None,
                }],
                execution_mode: Some("agent".to_string()),
                providers: vec![mock_responses_provider(
                    "https://example.invalid".to_string(),
                    "other-provider",
                    "other-model",
                )],
                ..Default::default()
            })
            .unwrap();
        let completed = run_domain_eval_campaign(
            db.clone(),
            RunDomainEvalCampaignInput {
                campaign_id: campaign.id,
                providers: Vec::new(),
                retry_failed_only: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(completed.status, "failed");

        let readiness = db
            .evaluate_domain_readiness_gate(DomainReadinessGateInput {
                min_eval_runs: Some(1),
                min_quality_runs: Some(1),
                min_campaign_items: Some(1),
                min_leaderboard_rows: Some(1),
                max_failed_campaign_items: Some(0),
                max_open_learning_proposals: Some(0),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(readiness.status, "failed");
        assert_eq!(readiness.summary.failed_campaign_items, 1);
        assert_eq!(readiness.summary.pending_learning_campaigns, 1);
        assert!(readiness
            .blockers
            .iter()
            .any(|blocker| blocker == "campaign_failures"));
        assert!(readiness
            .blockers
            .iter()
            .any(|blocker| blocker == "learning_closure"));
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn built_in_pack_keeps_five_domains_and_fifteen_cases() {
        let tasks = built_in_domain_eval_tasks();
        let domains = tasks
            .iter()
            .map(|task| task.domain.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(tasks.len(), 15);
        assert_eq!(domains.len(), 5);
    }

    #[test]
    fn fixture_source_type_never_conflates_trace_and_agent() {
        assert_eq!(
            fixture_source_type("trace_fixture"),
            DOMAIN_EVAL_SOURCE_FIXTURE_TRACE
        );
        assert_eq!(
            fixture_source_type("agent"),
            DOMAIN_EVAL_SOURCE_FIXTURE_AGENT
        );
        assert_eq!(
            fixture_source_type("unknown"),
            DOMAIN_EVAL_SOURCE_FIXTURE_UNSUPPORTED
        );
    }

    #[tokio::test]
    async fn agent_fixture_without_provider_configuration_fails_closed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open(&dir.path().join("sessions.db")).expect("open session database"),
        );
        let report = SessionDB::run_domain_eval_fixture(
            db,
            RunDomainEvalFixtureInput {
                fixture: DomainEvalFixture {
                    name: "agent-requires-provider-config".to_string(),
                    task_id: "research-source-backed-brief".to_string(),
                    execution_mode: "agent".to_string(),
                    checks: DomainEvalFixtureChecks {
                        expected_execution_status: Some("failed".to_string()),
                        error_contains: vec!["requires providers and modelChain".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();

        assert!(!report.passed);
        assert_eq!(report.status, "failed");
        assert!(report.eval_run.is_none());
        assert!(report
            .error
            .as_deref()
            .is_some_and(|error| error.contains("requires providers and modelChain")));
    }
}
