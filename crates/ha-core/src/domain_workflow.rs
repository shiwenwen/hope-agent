//! General domain workflow control plane.
//!
//! This module is additive to the coding-first control plane. It stores
//! reusable non-coding workflow manifests and general evidence items in
//! `sessions.db`, then links evidence back to Goal via the existing goal_links
//! table.

use anyhow::{anyhow, bail, Result};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::session::SessionDB;
use crate::workflow::preview_workflow_script_for_session;

const DOMAIN_TEMPLATE_LIMIT_DEFAULT: usize = 50;
const DOMAIN_TEMPLATE_LIMIT_MAX: usize = 200;
const DOMAIN_EVIDENCE_LIMIT_DEFAULT: usize = 50;
const DOMAIN_EVIDENCE_LIMIT_MAX: usize = 200;
const DOMAIN_EXPORT_GUARD_REVIEW_ITEMS_MAX: usize = 12;
const DOMAIN_CONNECTOR_ACTION_GUARD_EVIDENCE_MAX: usize = 12;
const DOMAIN_CONNECTOR_E2E_GATE_EVIDENCE_MAX: usize = 16;
pub const EVENT_DOMAIN_EVIDENCE_RECORDED: &str = "domain_evidence:recorded";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainWorkflowTemplate {
    pub id: String,
    pub version: String,
    pub title: String,
    pub domain: String,
    #[serde(default)]
    pub task_types: Vec<String>,
    pub default_mode: String,
    #[serde(default)]
    pub required_evidence: Vec<DomainEvidenceRequirement>,
    #[serde(default)]
    pub recommended_tools: Vec<String>,
    #[serde(default)]
    pub approval_gates: Vec<DomainApprovalGate>,
    #[serde(default)]
    pub verification_policy: Vec<DomainVerificationRule>,
    #[serde(default)]
    pub stop_conditions: Vec<String>,
    pub output_contract: String,
    #[serde(default)]
    pub eval_criteria: Vec<String>,
    #[serde(default)]
    pub prompt_hints: Vec<String>,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvidenceRequirement {
    pub evidence_type: String,
    pub title: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_count: Option<usize>,
    #[serde(default)]
    pub metadata_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainApprovalGate {
    pub action: String,
    pub reason: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainVerificationRule {
    pub rule: String,
    pub severity: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDomainWorkflowTemplatesInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default)]
    pub include_disabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveDomainWorkflowTemplateInput {
    pub template: DomainWorkflowTemplateDraft,
    #[serde(default)]
    pub explicit_save_consent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainWorkflowTemplateDraft {
    pub id: String,
    #[serde(default)]
    pub version: String,
    pub title: String,
    pub domain: String,
    #[serde(default)]
    pub task_types: Vec<String>,
    #[serde(default)]
    pub default_mode: String,
    #[serde(default)]
    pub required_evidence: Vec<DomainEvidenceRequirement>,
    #[serde(default)]
    pub recommended_tools: Vec<String>,
    #[serde(default)]
    pub approval_gates: Vec<DomainApprovalGate>,
    #[serde(default)]
    pub verification_policy: Vec<DomainVerificationRule>,
    #[serde(default)]
    pub stop_conditions: Vec<String>,
    #[serde(default)]
    pub output_contract: String,
    #[serde(default)]
    pub eval_criteria: Vec<String>,
    #[serde(default)]
    pub prompt_hints: Vec<String>,
    #[serde(default)]
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default = "serde_default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewDomainWorkflowInput {
    pub template_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_override: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_context: Option<String>,
    #[serde(default = "serde_default_true")]
    pub require_plan_confirmation: bool,
}

impl Default for PreviewDomainWorkflowInput {
    fn default() -> Self {
        Self {
            template_id: String::new(),
            version: None,
            session_id: String::new(),
            goal_id: None,
            task_type: None,
            objective: None,
            mode_override: None,
            user_context: None,
            require_plan_confirmation: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainWorkflowDraft {
    pub template: DomainWorkflowTemplate,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    pub execution_mode: String,
    pub workflow_kind: String,
    pub script_source: String,
    pub script_preview: crate::workflow::WorkflowScriptPreview,
    #[serde(default)]
    pub required_evidence: Vec<DomainEvidenceRequirement>,
    #[serde(default)]
    pub approval_gates: Vec<DomainApprovalGate>,
    #[serde(default)]
    pub verification_policy: Vec<DomainVerificationRule>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordDomainEvidenceInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub domain: String,
    pub evidence_type: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub source_metadata: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redaction_status: Option<String>,
}

struct PreparedDomainEvidence {
    id: String,
    goal_id: Option<String>,
    session_id: String,
    project_id: Option<String>,
    domain: String,
    evidence_type: String,
    title: String,
    summary: Option<String>,
    source_metadata_json: String,
    confidence: Option<f64>,
    access_scope: String,
    redaction_status: String,
    now: String,
}

fn insert_prepared_domain_evidence(
    conn: &Connection,
    prepared: &PreparedDomainEvidence,
) -> Result<()> {
    conn.execute(
        "INSERT INTO domain_evidence_items (
            id, goal_id, session_id, project_id, domain, evidence_type, title,
            summary, source_metadata_json, confidence, access_scope, redaction_status,
            created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)",
        params![
            prepared.id.as_str(),
            prepared.goal_id.as_deref(),
            prepared.session_id.as_str(),
            prepared.project_id.as_deref(),
            prepared.domain.as_str(),
            prepared.evidence_type.as_str(),
            prepared.title.as_str(),
            prepared.summary.as_deref(),
            prepared.source_metadata_json.as_str(),
            prepared.confidence,
            prepared.access_scope.as_str(),
            prepared.redaction_status.as_str(),
            prepared.now.as_str(),
        ],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDomainEvidenceInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEvidenceItem {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub domain: String,
    pub evidence_type: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub source_metadata: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    pub access_scope: String,
    pub redaction_status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainArtifactExportGuardInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_kind: Option<String>,
    #[serde(default = "serde_default_true")]
    pub require_artifact_created: bool,
    #[serde(default = "serde_default_true")]
    pub require_artifact_reviewed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_sensitive_unreviewed: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_redaction_pending: Option<usize>,
}

impl Default for DomainArtifactExportGuardInput {
    fn default() -> Self {
        Self {
            goal_id: None,
            session_id: None,
            project_id: None,
            domain: None,
            artifact_path: None,
            artifact_title: None,
            artifact_kind: None,
            require_artifact_created: true,
            require_artifact_reviewed: true,
            max_sensitive_unreviewed: None,
            max_redaction_pending: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainArtifactExportGuardThresholds {
    pub require_artifact_created: bool,
    pub require_artifact_reviewed: bool,
    pub max_sensitive_unreviewed: usize,
    pub max_redaction_pending: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainArtifactExportGuardScope {
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainArtifactExportGuardSummary {
    pub evidence_items: usize,
    pub artifact_created: usize,
    pub artifact_reviewed: usize,
    pub export_reviewed: usize,
    pub sensitive_evidence: usize,
    pub sensitive_unreviewed: usize,
    pub redaction_pending: usize,
    pub private_or_connector_evidence: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainArtifactExportGuardCheck {
    pub name: String,
    pub status: String,
    pub severity: String,
    pub expected: String,
    pub actual: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainArtifactExportGuardEvidence {
    pub id: String,
    pub evidence_type: String,
    pub title: String,
    pub access_scope: String,
    pub redaction_status: String,
    pub created_at: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainArtifactExportGuardReport {
    pub generated_at: String,
    pub status: String,
    pub scope: DomainArtifactExportGuardScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_kind: Option<String>,
    pub thresholds: DomainArtifactExportGuardThresholds,
    pub summary: DomainArtifactExportGuardSummary,
    pub checks: Vec<DomainArtifactExportGuardCheck>,
    pub blockers: Vec<String>,
    pub recommended_next_steps: Vec<String>,
    pub evidence_requiring_review: Vec<DomainArtifactExportGuardEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainConnectorActionGuardInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default = "serde_default_true")]
    pub require_explicit_approval: bool,
    #[serde(default = "serde_default_true")]
    pub require_rollback_plan: bool,
    #[serde(default = "serde_default_true")]
    pub require_export_guard_for_delivery: bool,
}

impl Default for DomainConnectorActionGuardInput {
    fn default() -> Self {
        Self {
            goal_id: None,
            session_id: None,
            project_id: None,
            domain: None,
            tool_name: None,
            connector: None,
            action: None,
            require_explicit_approval: true,
            require_rollback_plan: true,
            require_export_guard_for_delivery: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainConnectorActionGuardThresholds {
    pub require_explicit_approval: bool,
    pub require_rollback_plan: bool,
    pub require_export_guard_for_delivery: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainConnectorActionGuardScope {
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainConnectorActionGuardSummary {
    pub evidence_items: usize,
    pub action_evidence: usize,
    pub approval_evidence: usize,
    pub rollback_evidence: usize,
    pub sensitive_evidence: usize,
    pub delivery_action: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export_guard_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainConnectorActionGuardCheck {
    pub name: String,
    pub status: String,
    pub severity: String,
    pub expected: String,
    pub actual: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainConnectorActionGuardEvidence {
    pub id: String,
    pub evidence_type: String,
    pub title: String,
    pub access_scope: String,
    pub redaction_status: String,
    pub created_at: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainConnectorActionGuardReport {
    pub generated_at: String,
    pub status: String,
    pub scope: DomainConnectorActionGuardScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk: Option<String>,
    pub thresholds: DomainConnectorActionGuardThresholds,
    pub summary: DomainConnectorActionGuardSummary,
    pub checks: Vec<DomainConnectorActionGuardCheck>,
    pub blockers: Vec<String>,
    pub recommended_next_steps: Vec<String>,
    pub related_evidence: Vec<DomainConnectorActionGuardEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainConnectorE2EGateInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default = "serde_default_true")]
    pub require_connector_input: bool,
    #[serde(default = "serde_default_true")]
    pub require_draft: bool,
    #[serde(default = "serde_default_true")]
    pub require_explicit_approval: bool,
    #[serde(default = "serde_default_true")]
    pub require_execution_result: bool,
    #[serde(default = "serde_default_true")]
    pub require_post_action_verification: bool,
    #[serde(default = "serde_default_true")]
    pub require_rollback_plan: bool,
    #[serde(default = "serde_default_true")]
    pub require_export_guard_for_delivery: bool,
}

impl Default for DomainConnectorE2EGateInput {
    fn default() -> Self {
        Self {
            goal_id: None,
            session_id: None,
            project_id: None,
            domain: None,
            tool_name: None,
            connector: None,
            action: None,
            require_connector_input: true,
            require_draft: true,
            require_explicit_approval: true,
            require_execution_result: true,
            require_post_action_verification: true,
            require_rollback_plan: true,
            require_export_guard_for_delivery: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainConnectorE2EGateThresholds {
    pub require_connector_input: bool,
    pub require_draft: bool,
    pub require_explicit_approval: bool,
    pub require_execution_result: bool,
    pub require_post_action_verification: bool,
    pub require_rollback_plan: bool,
    pub require_export_guard_for_delivery: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainConnectorE2EGateScope {
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainConnectorE2EGateSummary {
    pub evidence_items: usize,
    pub connector_input_evidence: usize,
    pub draft_evidence: usize,
    pub approval_evidence: usize,
    pub execution_evidence: usize,
    pub verification_evidence: usize,
    pub rollback_evidence: usize,
    pub sensitive_evidence: usize,
    pub delivery_action: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector_action_guard_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export_guard_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainConnectorE2EGateCheck {
    pub name: String,
    pub status: String,
    pub severity: String,
    pub expected: String,
    pub actual: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainConnectorE2EGateEvidence {
    pub id: String,
    pub evidence_type: String,
    pub title: String,
    pub access_scope: String,
    pub redaction_status: String,
    pub created_at: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainConnectorE2EGateReport {
    pub generated_at: String,
    pub status: String,
    pub scope: DomainConnectorE2EGateScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk: Option<String>,
    pub thresholds: DomainConnectorE2EGateThresholds,
    pub summary: DomainConnectorE2EGateSummary,
    pub checks: Vec<DomainConnectorE2EGateCheck>,
    pub blockers: Vec<String>,
    pub recommended_next_steps: Vec<String>,
    pub related_evidence: Vec<DomainConnectorE2EGateEvidence>,
}

pub(crate) fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS domain_workflow_templates (
            id TEXT NOT NULL,
            version TEXT NOT NULL,
            title TEXT NOT NULL,
            domain TEXT NOT NULL,
            task_types_json TEXT NOT NULL DEFAULT '[]',
            default_mode TEXT NOT NULL DEFAULT 'guarded',
            required_evidence_json TEXT NOT NULL DEFAULT '[]',
            recommended_tools_json TEXT NOT NULL DEFAULT '[]',
            approval_gates_json TEXT NOT NULL DEFAULT '[]',
            verification_policy_json TEXT NOT NULL DEFAULT '[]',
            stop_conditions_json TEXT NOT NULL DEFAULT '[]',
            output_contract TEXT NOT NULL DEFAULT '',
            eval_criteria_json TEXT NOT NULL DEFAULT '[]',
            prompt_hints_json TEXT NOT NULL DEFAULT '[]',
            scope TEXT NOT NULL DEFAULT 'user',
            project_id TEXT,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (id, version)
        );

        CREATE TABLE IF NOT EXISTS domain_evidence_items (
            id TEXT PRIMARY KEY,
            goal_id TEXT,
            session_id TEXT NOT NULL,
            project_id TEXT,
            domain TEXT NOT NULL,
            evidence_type TEXT NOT NULL,
            title TEXT NOT NULL,
            summary TEXT,
            source_metadata_json TEXT NOT NULL DEFAULT '{}',
            confidence REAL,
            access_scope TEXT NOT NULL DEFAULT 'session',
            redaction_status TEXT NOT NULL DEFAULT 'none',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (goal_id) REFERENCES goals(id) ON DELETE SET NULL,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_domain_templates_domain
            ON domain_workflow_templates(domain, enabled, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_domain_templates_project
            ON domain_workflow_templates(project_id, enabled, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_domain_evidence_goal
            ON domain_evidence_items(goal_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_domain_evidence_session
            ON domain_evidence_items(session_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_domain_evidence_project
            ON domain_evidence_items(project_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_domain_evidence_domain
            ON domain_evidence_items(domain, evidence_type, created_at DESC);",
    )?;
    Ok(())
}

impl SessionDB {
    pub fn list_domain_workflow_templates(
        &self,
        input: ListDomainWorkflowTemplatesInput,
    ) -> Result<Vec<DomainWorkflowTemplate>> {
        let limit = input
            .limit
            .unwrap_or(DOMAIN_TEMPLATE_LIMIT_DEFAULT)
            .clamp(1, DOMAIN_TEMPLATE_LIMIT_MAX);
        let domain = normalized_opt(input.domain.as_deref()).map(normalize_domain);
        let task_type = normalized_opt(input.task_type.as_deref()).map(normalize_task_type);
        let project_id = normalized_opt(input.project_id.as_deref());

        let mut templates = built_in_domain_templates()
            .into_iter()
            .filter(|template| input.include_disabled || template.enabled)
            .filter(|template| {
                domain
                    .as_ref()
                    .map(|value| template.domain == *value)
                    .unwrap_or(true)
            })
            .filter(|template| {
                task_type
                    .as_ref()
                    .map(|value| template.task_types.iter().any(|task| task == value))
                    .unwrap_or(true)
            })
            .collect::<Vec<_>>();

        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut clauses = Vec::new();
        let mut params = Vec::new();
        if !input.include_disabled {
            clauses.push("enabled = 1".to_string());
        }
        if let Some(domain) = domain.as_ref() {
            clauses.push("domain = ?".to_string());
            params.push(domain.clone());
        }
        if let Some(project_id) = project_id.as_ref() {
            clauses.push("(project_id IS NULL OR project_id = ?)".to_string());
            params.push(project_id.to_string());
        } else {
            clauses.push("project_id IS NULL".to_string());
        }
        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        params.push(limit.to_string());
        let mut stmt = conn.prepare(&format!(
            "SELECT id, version, title, domain, task_types_json, default_mode,
                    required_evidence_json, recommended_tools_json, approval_gates_json,
                    verification_policy_json, stop_conditions_json, output_contract,
                    eval_criteria_json, prompt_hints_json, scope, project_id, enabled,
                    created_at, updated_at
             FROM domain_workflow_templates
             {where_sql}
             ORDER BY updated_at DESC, id ASC
             LIMIT ?"
        ))?;
        let rows = stmt.query_map(params_from_iter(params.iter()), domain_template_from_row)?;
        for row in rows {
            let template = row?;
            if task_type
                .as_ref()
                .map(|value| template.task_types.iter().any(|task| task == value))
                .unwrap_or(true)
            {
                templates.retain(|existing| {
                    !(existing.id == template.id && existing.version == template.version)
                });
                templates.push(template);
            }
        }
        templates.sort_by(|a, b| {
            domain_rank(&a.domain)
                .cmp(&domain_rank(&b.domain))
                .then_with(|| a.scope.cmp(&b.scope))
                .then_with(|| a.title.cmp(&b.title))
        });
        templates.truncate(limit);
        Ok(templates)
    }

    pub fn get_domain_workflow_template(
        &self,
        id: &str,
        version: Option<&str>,
    ) -> Result<Option<DomainWorkflowTemplate>> {
        let id = id.trim();
        if id.is_empty() {
            bail!("domain workflow template id must not be empty");
        }
        let version = version.and_then(non_empty);
        let builtins = built_in_domain_templates();
        if let Some(template) = builtins.iter().find(|template| {
            template.id == id
                && version
                    .map(|value| template.version == value)
                    .unwrap_or(true)
        }) {
            return Ok(Some(template.clone()));
        }
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        if let Some(version) = version {
            conn.query_row(
                "SELECT id, version, title, domain, task_types_json, default_mode,
                        required_evidence_json, recommended_tools_json, approval_gates_json,
                        verification_policy_json, stop_conditions_json, output_contract,
                        eval_criteria_json, prompt_hints_json, scope, project_id, enabled,
                        created_at, updated_at
                 FROM domain_workflow_templates
                 WHERE id = ?1 AND version = ?2",
                params![id, version],
                domain_template_from_row,
            )
            .optional()
            .map_err(Into::into)
        } else {
            conn.query_row(
                "SELECT id, version, title, domain, task_types_json, default_mode,
                        required_evidence_json, recommended_tools_json, approval_gates_json,
                        verification_policy_json, stop_conditions_json, output_contract,
                        eval_criteria_json, prompt_hints_json, scope, project_id, enabled,
                        created_at, updated_at
                 FROM domain_workflow_templates
                 WHERE id = ?1
                 ORDER BY updated_at DESC
                 LIMIT 1",
                params![id],
                domain_template_from_row,
            )
            .optional()
            .map_err(Into::into)
        }
    }

    pub fn save_domain_workflow_template(
        &self,
        input: SaveDomainWorkflowTemplateInput,
    ) -> Result<DomainWorkflowTemplate> {
        if !input.explicit_save_consent {
            bail!("saving a domain workflow template requires explicit consent");
        }
        let template = normalize_template_draft(input.template)?;
        if template.scope == "built_in" {
            bail!("custom domain workflow templates cannot use built_in scope");
        }
        if built_in_domain_templates()
            .iter()
            .any(|builtin| builtin.id == template.id && builtin.version == template.version)
        {
            bail!("custom template cannot overwrite built-in template/version");
        }
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO domain_workflow_templates (
                id, version, title, domain, task_types_json, default_mode,
                required_evidence_json, recommended_tools_json, approval_gates_json,
                verification_policy_json, stop_conditions_json, output_contract,
                eval_criteria_json, prompt_hints_json, scope, project_id, enabled,
                created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?18
             )
             ON CONFLICT(id, version) DO UPDATE SET
                title = excluded.title,
                domain = excluded.domain,
                task_types_json = excluded.task_types_json,
                default_mode = excluded.default_mode,
                required_evidence_json = excluded.required_evidence_json,
                recommended_tools_json = excluded.recommended_tools_json,
                approval_gates_json = excluded.approval_gates_json,
                verification_policy_json = excluded.verification_policy_json,
                stop_conditions_json = excluded.stop_conditions_json,
                output_contract = excluded.output_contract,
                eval_criteria_json = excluded.eval_criteria_json,
                prompt_hints_json = excluded.prompt_hints_json,
                scope = excluded.scope,
                project_id = excluded.project_id,
                enabled = excluded.enabled,
                updated_at = excluded.updated_at",
            params![
                template.id,
                template.version,
                template.title,
                template.domain,
                stable_json(&template.task_types)?,
                template.default_mode,
                stable_json(&template.required_evidence)?,
                stable_json(&template.recommended_tools)?,
                stable_json(&template.approval_gates)?,
                stable_json(&template.verification_policy)?,
                stable_json(&template.stop_conditions)?,
                template.output_contract,
                stable_json(&template.eval_criteria)?,
                stable_json(&template.prompt_hints)?,
                template.scope,
                template.project_id,
                if template.enabled { 1i64 } else { 0i64 },
                now,
            ],
        )?;
        drop(conn);
        self.get_domain_workflow_template(&template.id, Some(&template.version))?
            .ok_or_else(|| anyhow!("domain workflow template missing after save"))
    }

    pub fn preview_domain_workflow(
        &self,
        input: PreviewDomainWorkflowInput,
    ) -> Result<DomainWorkflowDraft> {
        let session_id = input.session_id.trim();
        if session_id.is_empty() {
            bail!("session_id is required");
        }
        let session = self
            .get_session(session_id)?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        if session.incognito {
            bail!("domain workflow preview is disabled for incognito sessions");
        }
        let template = self
            .get_domain_workflow_template(&input.template_id, input.version.as_deref())?
            .ok_or_else(|| anyhow!("domain workflow template not found: {}", input.template_id))?;
        if !template.enabled {
            bail!("domain workflow template is disabled: {}", template.id);
        }
        let goal = match input.goal_id.as_deref().and_then(non_empty) {
            Some(goal_id) => Some(
                self.get_goal(goal_id)?
                    .ok_or_else(|| anyhow!("goal not found: {goal_id}"))?,
            ),
            None => self
                .active_goal_for_session(session_id)?
                .map(|snapshot| snapshot.goal),
        };
        if let Some(goal) = goal.as_ref() {
            if goal.session_id != session_id {
                bail!("goal {} does not belong to session {}", goal.id, session_id);
            }
        }
        let task_type = input
            .task_type
            .as_deref()
            .and_then(non_empty)
            .map(normalize_task_type)
            .or_else(|| template.task_types.first().cloned())
            .unwrap_or_else(|| "general".to_string());
        if !template.task_types.is_empty()
            && !template.task_types.iter().any(|task| task == &task_type)
        {
            bail!(
                "task type {} is not supported by template {}",
                task_type,
                template.id
            );
        }
        let execution_mode = input
            .mode_override
            .as_deref()
            .and_then(non_empty)
            .map(normalize_mode)
            .unwrap_or_else(|| normalize_mode(&template.default_mode));
        let objective = input
            .objective
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string)
            .or_else(|| goal.as_ref().map(|goal| goal.objective.clone()))
            .unwrap_or_else(|| template.title.clone());
        let warnings = domain_workflow_warnings(&template);
        let script_source = render_domain_workflow_script(
            &template,
            goal.as_ref().map(|goal| goal.id.as_str()),
            &task_type,
            &objective,
            input.user_context.as_deref().unwrap_or_default(),
            input.require_plan_confirmation,
        );
        let script_preview = preview_workflow_script_for_session(
            self,
            session_id,
            &script_source,
            Some(&execution_mode),
        );
        Ok(DomainWorkflowDraft {
            template: template.clone(),
            session_id: session_id.to_string(),
            goal_id: goal.map(|goal| goal.id),
            execution_mode,
            workflow_kind: format!("domain:{}", template.domain),
            script_source,
            script_preview,
            required_evidence: template.required_evidence.clone(),
            approval_gates: template.approval_gates.clone(),
            verification_policy: template.verification_policy.clone(),
            warnings,
        })
    }

    pub fn record_domain_evidence(
        &self,
        input: RecordDomainEvidenceInput,
    ) -> Result<DomainEvidenceItem> {
        let prepared = self.prepare_domain_evidence(input)?;
        let id = prepared.id.clone();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        insert_prepared_domain_evidence(&conn, &prepared)?;
        drop(conn);
        self.finalize_domain_evidence(&id, false)
    }

    /// Commit an owner ask_user answer and its evidence in one SQLite
    /// transaction. The pending-row predicate is the idempotency claim: only one
    /// concurrent surface can insert evidence, and a crash cannot leave evidence
    /// recorded while the question remains pending/times out later.
    pub fn record_owner_ask_user_evidence_and_answer(
        &self,
        request_id: &str,
        input: RecordDomainEvidenceInput,
    ) -> Result<DomainEvidenceItem> {
        let prepared = self.prepare_domain_evidence(input)?;
        let id = prepared.id.clone();
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        let pending: bool = tx.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM ask_user_questions
                 WHERE request_id = ?1
                   AND status = 'pending'
                   AND (timeout_at IS NULL OR timeout_at = 0
                        OR timeout_at > strftime('%s','now'))
             )",
            params![request_id],
            |row| row.get(0),
        )?;
        if !pending {
            bail!("No pending ask_user_question request: {request_id}");
        }
        insert_prepared_domain_evidence(&tx, &prepared)?;
        let changed = tx.execute(
            "UPDATE ask_user_questions
                SET status = 'answered', answered_at = datetime('now')
              WHERE request_id = ?1 AND status = 'pending'",
            params![request_id],
        )?;
        if changed != 1 {
            bail!("No pending ask_user_question request: {request_id}");
        }
        tx.commit()?;
        drop(conn);
        // Goal-link projection is derived from the committed evidence. A link
        // failure must not turn an already-committed user answer into a retry
        // that can duplicate its primary evidence.
        self.finalize_domain_evidence(&id, true)
    }

    fn prepare_domain_evidence(
        &self,
        input: RecordDomainEvidenceInput,
    ) -> Result<PreparedDomainEvidence> {
        let goal_id = input
            .goal_id
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        let requested_session_id = input
            .session_id
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        let mut session_id = requested_session_id.clone();
        let mut project_id = input
            .project_id
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        if let Some(goal_id) = goal_id.as_deref() {
            let goal = self
                .get_goal(goal_id)?
                .ok_or_else(|| anyhow!("goal not found: {goal_id}"))?;
            if let Some(requested_session_id) = requested_session_id.as_deref() {
                if requested_session_id != goal.session_id {
                    bail!(
                        "goal {} does not belong to session {}",
                        goal.id,
                        requested_session_id
                    );
                }
            }
            session_id = Some(goal.session_id.clone());
        }
        let Some(session_id) = session_id.as_deref() else {
            bail!("record_domain_evidence requires goal_id or session_id");
        };
        let session = self
            .get_session(session_id)?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        if session.incognito {
            bail!("domain evidence is disabled for incognito sessions");
        }
        if let Some(session_project_id) = session.project_id.as_deref() {
            if let Some(requested_project_id) = project_id.as_deref() {
                if requested_project_id != session_project_id {
                    bail!(
                        "session {} belongs to project {}, not {}",
                        session_id,
                        session_project_id,
                        requested_project_id
                    );
                }
            }
            project_id = Some(session_project_id.to_string());
        }

        let domain = normalize_domain(&input.domain);
        let evidence_type = normalize_evidence_type(&input.evidence_type)?;
        let title = input.title.trim();
        if title.is_empty() {
            bail!("domain evidence title must not be empty");
        }
        let confidence = input.confidence.map(|value| value.clamp(0.0, 1.0));
        let access_scope = input
            .access_scope
            .as_deref()
            .and_then(non_empty)
            .map(normalize_access_scope)
            .unwrap_or_else(|| "session".to_string());
        let redaction_status = input
            .redaction_status
            .as_deref()
            .and_then(non_empty)
            .map(normalize_redaction_status)
            .unwrap_or_else(|| "none".to_string());
        let source_metadata = ensure_object(input.source_metadata);
        Ok(PreparedDomainEvidence {
            id: format!("devi_{}", uuid::Uuid::new_v4().simple()),
            goal_id,
            session_id: session_id.to_string(),
            project_id,
            domain,
            evidence_type,
            title: title.to_string(),
            summary: input.summary,
            source_metadata_json: stable_json(&source_metadata)?,
            confidence,
            access_scope,
            redaction_status,
            now: now_rfc3339(),
        })
    }

    fn finalize_domain_evidence(
        &self,
        id: &str,
        best_effort_goal_link: bool,
    ) -> Result<DomainEvidenceItem> {
        let item = self
            .get_domain_evidence(id)?
            .ok_or_else(|| anyhow!("domain evidence missing after insert"))?;
        if let Some(goal_id) = item.goal_id.as_deref() {
            let link_result = self.link_goal_target(
                goal_id,
                "domain_evidence",
                &item.id,
                &item.evidence_type,
                json!({
                    "domain": item.domain,
                    "title": item.title,
                    "summary": item.summary,
                    "confidence": item.confidence,
                    "accessScope": item.access_scope,
                    "redactionStatus": item.redaction_status,
                    "source": item.source_metadata,
                }),
            );
            if let Err(e) = link_result {
                if best_effort_goal_link {
                    app_warn!(
                        "domain_workflow",
                        "owner_ask_user_link",
                        "Evidence {} committed but goal link projection failed: {}",
                        item.id,
                        e
                    );
                } else {
                    return Err(e);
                }
            }
        }
        emit_domain_evidence_recorded(&item);
        Ok(item)
    }

    pub fn list_domain_evidence(
        &self,
        input: ListDomainEvidenceInput,
    ) -> Result<Vec<DomainEvidenceItem>> {
        let limit = input
            .limit
            .unwrap_or(DOMAIN_EVIDENCE_LIMIT_DEFAULT)
            .clamp(1, DOMAIN_EVIDENCE_LIMIT_MAX);
        let mut clauses = Vec::new();
        let mut params = Vec::new();
        if let Some(goal_id) = input.goal_id.as_deref().and_then(non_empty) {
            clauses.push("goal_id = ?".to_string());
            params.push(goal_id.to_string());
        }
        if let Some(session_id) = input.session_id.as_deref().and_then(non_empty) {
            clauses.push("session_id = ?".to_string());
            params.push(session_id.to_string());
        }
        if let Some(project_id) = input.project_id.as_deref().and_then(non_empty) {
            clauses.push("project_id = ?".to_string());
            params.push(project_id.to_string());
        }
        if let Some(domain) = input.domain.as_deref().and_then(non_empty) {
            clauses.push("domain = ?".to_string());
            params.push(normalize_domain(domain));
        }
        if let Some(evidence_type) = input.evidence_type.as_deref().and_then(non_empty) {
            clauses.push("evidence_type = ?".to_string());
            params.push(normalize_evidence_type(evidence_type)?);
        }
        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        params.push(limit.to_string());
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&format!(
            "SELECT id, goal_id, session_id, project_id, domain, evidence_type, title,
                    summary, source_metadata_json, confidence, access_scope, redaction_status,
                    created_at, updated_at
             FROM domain_evidence_items
             {where_sql}
             ORDER BY created_at DESC, id DESC
             LIMIT ?"
        ))?;
        let rows = stmt.query_map(params_from_iter(params.iter()), domain_evidence_from_row)?;
        collect_rows(rows)
    }

    fn get_domain_evidence(&self, id: &str) -> Result<Option<DomainEvidenceItem>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, goal_id, session_id, project_id, domain, evidence_type, title,
                    summary, source_metadata_json, confidence, access_scope, redaction_status,
                    created_at, updated_at
             FROM domain_evidence_items
             WHERE id = ?1",
            params![id],
            domain_evidence_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn evaluate_domain_artifact_export_guard(
        &self,
        input: DomainArtifactExportGuardInput,
    ) -> Result<DomainArtifactExportGuardReport> {
        let goal_id = input
            .goal_id
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        let requested_session_id = input
            .session_id
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        let mut session_id = requested_session_id.clone();
        if let Some(goal_id) = goal_id.as_deref() {
            let goal = self
                .get_goal(goal_id)?
                .ok_or_else(|| anyhow!("goal not found: {goal_id}"))?;
            if let Some(requested_session_id) = requested_session_id.as_deref() {
                if requested_session_id != goal.session_id {
                    bail!(
                        "goal {} does not belong to session {}",
                        goal.id,
                        requested_session_id
                    );
                }
            }
            session_id = Some(goal.session_id.clone());
        }
        let Some(session_id) = session_id.as_deref() else {
            bail!("evaluate_domain_artifact_export_guard requires goal_id or session_id");
        };

        let session = self
            .get_session(session_id)?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        if session.incognito {
            bail!("artifact export guard is disabled for incognito sessions");
        }

        let mut project_id = input
            .project_id
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        if let Some(session_project_id) = session.project_id.as_deref() {
            if let Some(requested_project_id) = project_id.as_deref() {
                if requested_project_id != session_project_id {
                    bail!(
                        "session {} belongs to project {}, not {}",
                        session_id,
                        session_project_id,
                        requested_project_id
                    );
                }
            }
            project_id = Some(session_project_id.to_string());
        } else if let Some(requested_project_id) = project_id.as_deref() {
            bail!(
                "session {} is not bound to project {}",
                session_id,
                requested_project_id
            );
        }

        let domain = input
            .domain
            .as_deref()
            .and_then(non_empty)
            .map(normalize_domain);
        let thresholds = DomainArtifactExportGuardThresholds {
            require_artifact_created: input.require_artifact_created,
            require_artifact_reviewed: input.require_artifact_reviewed,
            max_sensitive_unreviewed: input.max_sensitive_unreviewed.unwrap_or(0),
            max_redaction_pending: input.max_redaction_pending.unwrap_or(0),
        };
        let evidence = self.list_domain_evidence(ListDomainEvidenceInput {
            goal_id: goal_id.clone(),
            session_id: Some(session_id.to_string()),
            project_id: None,
            domain: domain.clone(),
            evidence_type: None,
            limit: Some(DOMAIN_EVIDENCE_LIMIT_MAX),
        })?;

        let artifact_created = evidence
            .iter()
            .filter(|item| item.evidence_type == "artifact_created")
            .count();
        let artifact_reviewed = evidence
            .iter()
            .filter(|item| item.evidence_type == "artifact_reviewed")
            .count();
        let export_reviewed = evidence
            .iter()
            .filter(|item| domain_evidence_has_export_review_marker(item))
            .count();
        let sensitive_evidence = evidence
            .iter()
            .filter(|item| domain_evidence_requires_export_review(item))
            .count();
        let sensitive_unreviewed = if sensitive_evidence > 0 && export_reviewed == 0 {
            sensitive_evidence
        } else {
            0
        };
        let redaction_pending = evidence
            .iter()
            .filter(|item| {
                item.redaction_status == "pending" || item.redaction_status == "sensitive"
            })
            .count();
        let private_or_connector_evidence = evidence
            .iter()
            .filter(|item| item.access_scope == "private" || item.access_scope == "connector")
            .count();

        let summary = DomainArtifactExportGuardSummary {
            evidence_items: evidence.len(),
            artifact_created,
            artifact_reviewed,
            export_reviewed,
            sensitive_evidence,
            sensitive_unreviewed,
            redaction_pending,
            private_or_connector_evidence,
        };

        let mut checks = Vec::new();
        push_export_guard_check(
            &mut checks,
            "evidence_scope",
            if summary.evidence_items > 0 {
                "passed"
            } else {
                "insufficient_data"
            },
            "p2",
            "At least one domain evidence item is recorded",
            &summary.evidence_items.to_string(),
            "The guard needs recorded evidence before it can approve export or sharing.",
        );
        push_export_guard_check(
            &mut checks,
            "artifact_created",
            if !thresholds.require_artifact_created || summary.artifact_created > 0 {
                "passed"
            } else {
                "insufficient_data"
            },
            "p1",
            "artifact_created evidence is present",
            &summary.artifact_created.to_string(),
            "A final report, document, spreadsheet, brief, or draft artifact must be recorded.",
        );
        push_export_guard_check(
            &mut checks,
            "artifact_reviewed",
            if !thresholds.require_artifact_reviewed || summary.artifact_reviewed > 0 {
                "passed"
            } else {
                "insufficient_data"
            },
            "p1",
            "artifact_reviewed evidence is present",
            &summary.artifact_reviewed.to_string(),
            "The artifact needs an audience, requirement, or quality review before delivery.",
        );
        push_export_guard_check(
            &mut checks,
            "redaction_status",
            if summary.redaction_pending <= thresholds.max_redaction_pending {
                "passed"
            } else {
                "failed"
            },
            "p0",
            &format!(
                "redaction pending or sensitive evidence <= {}",
                thresholds.max_redaction_pending
            ),
            &summary.redaction_pending.to_string(),
            "Pending or sensitive evidence must be resolved before export.",
        );
        push_export_guard_check(
            &mut checks,
            "sensitive_evidence",
            if summary.sensitive_unreviewed <= thresholds.max_sensitive_unreviewed {
                "passed"
            } else {
                "failed"
            },
            "p0",
            &format!(
                "private, connector, or redacted evidence without explicit export review <= {}",
                thresholds.max_sensitive_unreviewed
            ),
            &summary.sensitive_unreviewed.to_string(),
            "Sensitive evidence requires explicit exportReview/exportReady/redactionChecked metadata on artifact_reviewed evidence.",
        );

        let status = export_guard_status(&checks);
        let blockers = checks
            .iter()
            .filter(|check| check.status != "passed")
            .map(|check| format!("{}: {}", check.name, check.detail))
            .collect::<Vec<_>>();
        let recommended_next_steps = export_guard_recommendations(&checks);
        let evidence_requiring_review = evidence
            .iter()
            .filter(|item| domain_evidence_requires_export_review(item))
            .take(DOMAIN_EXPORT_GUARD_REVIEW_ITEMS_MAX)
            .map(domain_export_guard_evidence)
            .collect();

        Ok(DomainArtifactExportGuardReport {
            generated_at: now_rfc3339(),
            status,
            scope: DomainArtifactExportGuardScope {
                scope: if goal_id.is_some() {
                    "goal".to_string()
                } else {
                    "session".to_string()
                },
                goal_id,
                session_id: Some(session_id.to_string()),
                project_id,
                domain,
            },
            artifact_path: input
                .artifact_path
                .as_deref()
                .and_then(non_empty)
                .map(str::to_string),
            artifact_title: input
                .artifact_title
                .as_deref()
                .and_then(non_empty)
                .map(str::to_string),
            artifact_kind: input
                .artifact_kind
                .as_deref()
                .and_then(non_empty)
                .map(str::to_string),
            thresholds,
            summary,
            checks,
            blockers,
            recommended_next_steps,
            evidence_requiring_review,
        })
    }

    pub fn evaluate_domain_connector_action_guard(
        &self,
        input: DomainConnectorActionGuardInput,
    ) -> Result<DomainConnectorActionGuardReport> {
        let goal_id = input
            .goal_id
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        let requested_session_id = input
            .session_id
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        let mut session_id = requested_session_id.clone();
        if let Some(goal_id) = goal_id.as_deref() {
            let goal = self
                .get_goal(goal_id)?
                .ok_or_else(|| anyhow!("goal not found: {goal_id}"))?;
            if let Some(requested_session_id) = requested_session_id.as_deref() {
                if requested_session_id != goal.session_id {
                    bail!(
                        "goal {} does not belong to session {}",
                        goal.id,
                        requested_session_id
                    );
                }
            }
            session_id = Some(goal.session_id.clone());
        }
        let Some(session_id) = session_id.as_deref() else {
            bail!("evaluate_domain_connector_action_guard requires goal_id or session_id");
        };

        let session = self
            .get_session(session_id)?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        if session.incognito {
            bail!("connector action guard is disabled for incognito sessions");
        }

        let mut project_id = input
            .project_id
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        if let Some(session_project_id) = session.project_id.as_deref() {
            if let Some(requested_project_id) = project_id.as_deref() {
                if requested_project_id != session_project_id {
                    bail!(
                        "session {} belongs to project {}, not {}",
                        session_id,
                        session_project_id,
                        requested_project_id
                    );
                }
            }
            project_id = Some(session_project_id.to_string());
        } else if let Some(requested_project_id) = project_id.as_deref() {
            bail!(
                "session {} is not bound to project {}",
                session_id,
                requested_project_id
            );
        }

        let domain = input
            .domain
            .as_deref()
            .and_then(non_empty)
            .map(normalize_domain);
        let tool_name = input
            .tool_name
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        let mut connector = input
            .connector
            .as_deref()
            .and_then(non_empty)
            .map(normalize_task_type);
        let mut action = input
            .action
            .as_deref()
            .and_then(non_empty)
            .map(normalize_connector_action);
        if let Some(tool_name) = tool_name.as_deref() {
            if let Some((classified_connector, classified_action)) =
                crate::permission::engine::classify_external_connector_action(tool_name, &json!({}))
            {
                connector.get_or_insert(classified_connector);
                action.get_or_insert(classified_action);
            }
        }

        let thresholds = DomainConnectorActionGuardThresholds {
            require_explicit_approval: input.require_explicit_approval,
            require_rollback_plan: input.require_rollback_plan,
            require_export_guard_for_delivery: input.require_export_guard_for_delivery,
        };
        let evidence = self.list_domain_evidence(ListDomainEvidenceInput {
            goal_id: goal_id.clone(),
            session_id: Some(session_id.to_string()),
            project_id: None,
            domain: domain.clone(),
            evidence_type: None,
            limit: Some(DOMAIN_EVIDENCE_LIMIT_MAX),
        })?;

        if connector.is_none() {
            connector = evidence
                .iter()
                .find_map(|item| domain_connector_metadata_string(item, "connector"));
        }
        if action.is_none() {
            action = evidence
                .iter()
                .find_map(|item| domain_connector_action_from_evidence(item));
        }

        let action_evidence = evidence
            .iter()
            .filter(|item| domain_evidence_mentions_connector_action(item))
            .count();
        let approval_evidence = evidence
            .iter()
            .filter(|item| domain_evidence_has_connector_approval_marker(item))
            .count();
        let rollback_evidence = evidence
            .iter()
            .filter(|item| domain_evidence_has_rollback_marker(item))
            .count();
        let sensitive_evidence = evidence
            .iter()
            .filter(|item| domain_evidence_requires_export_review(item))
            .count();
        let delivery_action =
            domain_connector_action_requires_export_guard(action.as_deref(), tool_name.as_deref());
        let export_guard = if thresholds.require_export_guard_for_delivery && delivery_action {
            Some(
                self.evaluate_domain_artifact_export_guard(DomainArtifactExportGuardInput {
                    goal_id: goal_id.clone(),
                    session_id: Some(session_id.to_string()),
                    project_id: project_id.clone(),
                    domain: domain.clone(),
                    ..Default::default()
                })?,
            )
        } else {
            None
        };
        let export_guard_status = export_guard.as_ref().map(|report| report.status.clone());

        let summary = DomainConnectorActionGuardSummary {
            evidence_items: evidence.len(),
            action_evidence,
            approval_evidence,
            rollback_evidence,
            sensitive_evidence,
            delivery_action,
            export_guard_status: export_guard_status.clone(),
        };

        let mut checks = Vec::new();
        push_connector_action_guard_check(
            &mut checks,
            "action_scope",
            if tool_name.is_some() || connector.is_some() || action.is_some() || action_evidence > 0
            {
                "passed"
            } else {
                "insufficient_data"
            },
            "p1",
            "External connector action is identified",
            &domain_connector_actual(&tool_name, &connector, &action, action_evidence),
            "Record or pass the connector action before deciding whether it may run.",
        );
        push_connector_action_guard_check(
            &mut checks,
            "explicit_user_approval",
            if !thresholds.require_explicit_approval || approval_evidence > 0 {
                "passed"
            } else {
                "failed"
            },
            "p0",
            "message_draft_approved/user_decision or explicitUserApproval metadata is present",
            &approval_evidence.to_string(),
            "External system mutations must have explicit user approval evidence before execution.",
        );
        push_connector_action_guard_check(
            &mut checks,
            "rollback_plan",
            if !thresholds.require_rollback_plan || rollback_evidence > 0 {
                "passed"
            } else {
                "insufficient_data"
            },
            "p1",
            "rollbackPlan/undoPlan/recoveryPlan metadata is present",
            &rollback_evidence.to_string(),
            "The user should see how to undo or recover from the external action.",
        );
        if let Some(status) = export_guard_status.as_deref() {
            push_connector_action_guard_check(
                &mut checks,
                "artifact_export_guard",
                if status == "passed" {
                    "passed"
                } else if status == "failed" {
                    "failed"
                } else {
                    "insufficient_data"
                },
                "p0",
                "Delivery action has passed Artifact Export Guard",
                status,
                "Sending, sharing, uploading, exporting, or publishing should pass final artifact/export review first.",
            );
        }

        let status = connector_action_guard_status(&checks);
        let blockers = checks
            .iter()
            .filter(|check| check.status != "passed")
            .map(|check| format!("{}: {}", check.name, check.detail))
            .collect::<Vec<_>>();
        let recommended_next_steps = connector_action_guard_recommendations(&checks);
        let related_evidence = evidence
            .iter()
            .filter(|item| {
                domain_evidence_mentions_connector_action(item)
                    || domain_evidence_has_connector_approval_marker(item)
                    || domain_evidence_has_rollback_marker(item)
                    || domain_evidence_requires_export_review(item)
            })
            .take(DOMAIN_CONNECTOR_ACTION_GUARD_EVIDENCE_MAX)
            .map(domain_connector_action_guard_evidence)
            .collect();

        Ok(DomainConnectorActionGuardReport {
            generated_at: now_rfc3339(),
            status,
            scope: DomainConnectorActionGuardScope {
                scope: if goal_id.is_some() {
                    "goal".to_string()
                } else {
                    "session".to_string()
                },
                goal_id,
                session_id: Some(session_id.to_string()),
                project_id,
                domain,
            },
            tool_name,
            connector,
            action,
            risk: Some("external_system_mutation".to_string()),
            thresholds,
            summary,
            checks,
            blockers,
            recommended_next_steps,
            related_evidence,
        })
    }

    pub fn evaluate_domain_connector_e2e_gate(
        &self,
        input: DomainConnectorE2EGateInput,
    ) -> Result<DomainConnectorE2EGateReport> {
        let goal_id = input
            .goal_id
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        let requested_session_id = input
            .session_id
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        let mut session_id = requested_session_id.clone();
        if let Some(goal_id) = goal_id.as_deref() {
            let goal = self
                .get_goal(goal_id)?
                .ok_or_else(|| anyhow!("goal not found: {goal_id}"))?;
            if let Some(requested_session_id) = requested_session_id.as_deref() {
                if requested_session_id != goal.session_id {
                    bail!(
                        "goal {} does not belong to session {}",
                        goal.id,
                        requested_session_id
                    );
                }
            }
            session_id = Some(goal.session_id.clone());
        }

        let mut project_id = input
            .project_id
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        if let Some(session_id) = session_id.as_deref() {
            let session = self
                .get_session(session_id)?
                .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
            if session.incognito {
                bail!("connector e2e gate is disabled for incognito sessions");
            }
            if let Some(session_project_id) = session.project_id.as_deref() {
                if let Some(requested_project_id) = project_id.as_deref() {
                    if requested_project_id != session_project_id {
                        bail!(
                            "session {} belongs to project {}, not {}",
                            session_id,
                            session_project_id,
                            requested_project_id
                        );
                    }
                }
                project_id = Some(session_project_id.to_string());
            } else if let Some(requested_project_id) = project_id.as_deref() {
                bail!(
                    "session {} is not bound to project {}",
                    session_id,
                    requested_project_id
                );
            }
        }

        let domain = input
            .domain
            .as_deref()
            .and_then(non_empty)
            .map(normalize_domain);
        let tool_name = input
            .tool_name
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string);
        let mut connector = input
            .connector
            .as_deref()
            .and_then(non_empty)
            .map(normalize_task_type);
        let mut action = input
            .action
            .as_deref()
            .and_then(non_empty)
            .map(normalize_connector_action);
        if let Some(tool_name) = tool_name.as_deref() {
            if let Some((classified_connector, classified_action)) =
                crate::permission::engine::classify_external_connector_action(tool_name, &json!({}))
            {
                connector.get_or_insert(classified_connector);
                action.get_or_insert(classified_action);
            }
        }

        let thresholds = DomainConnectorE2EGateThresholds {
            require_connector_input: input.require_connector_input,
            require_draft: input.require_draft,
            require_explicit_approval: input.require_explicit_approval,
            require_execution_result: input.require_execution_result,
            require_post_action_verification: input.require_post_action_verification,
            require_rollback_plan: input.require_rollback_plan,
            require_export_guard_for_delivery: input.require_export_guard_for_delivery,
        };
        let evidence = self.list_domain_evidence(ListDomainEvidenceInput {
            goal_id: goal_id.clone(),
            session_id: session_id.clone(),
            project_id: if session_id.is_none() {
                project_id.clone()
            } else {
                None
            },
            domain: domain.clone(),
            evidence_type: None,
            limit: Some(DOMAIN_EVIDENCE_LIMIT_MAX),
        })?;

        if connector.is_none() {
            connector = evidence
                .iter()
                .find_map(|item| domain_connector_metadata_string(item, "connector"));
        }
        if action.is_none() {
            action = evidence
                .iter()
                .find_map(|item| domain_connector_action_from_evidence(item));
        }

        let connector_input_evidence = evidence
            .iter()
            .filter(|item| domain_evidence_has_connector_input(item))
            .count();
        let draft_evidence = evidence
            .iter()
            .filter(|item| domain_evidence_has_connector_draft_marker(item))
            .count();
        let approval_evidence = evidence
            .iter()
            .filter(|item| domain_evidence_has_connector_approval_marker(item))
            .count();
        let execution_evidence = evidence
            .iter()
            .filter(|item| domain_evidence_has_connector_execution_marker(item))
            .count();
        let verification_evidence = evidence
            .iter()
            .filter(|item| domain_evidence_has_connector_verification_marker(item))
            .count();
        let rollback_evidence = evidence
            .iter()
            .filter(|item| domain_evidence_has_rollback_marker(item))
            .count();
        let sensitive_evidence = evidence
            .iter()
            .filter(|item| domain_evidence_requires_export_review(item))
            .count();
        let delivery_action =
            domain_connector_action_requires_export_guard(action.as_deref(), tool_name.as_deref());
        let connector_action_guard = if let Some(session_id) = session_id.as_deref() {
            Some(
                self.evaluate_domain_connector_action_guard(DomainConnectorActionGuardInput {
                    goal_id: goal_id.clone(),
                    session_id: Some(session_id.to_string()),
                    project_id: project_id.clone(),
                    domain: domain.clone(),
                    tool_name: tool_name.clone(),
                    connector: connector.clone(),
                    action: action.clone(),
                    require_explicit_approval: thresholds.require_explicit_approval,
                    require_rollback_plan: thresholds.require_rollback_plan,
                    require_export_guard_for_delivery: thresholds.require_export_guard_for_delivery,
                })?,
            )
        } else {
            None
        };
        let connector_action_guard_status = connector_action_guard
            .as_ref()
            .map(|guard| guard.status.clone());
        let export_guard_status = connector_action_guard
            .as_ref()
            .and_then(|guard| guard.summary.export_guard_status.clone());

        let summary = DomainConnectorE2EGateSummary {
            evidence_items: evidence.len(),
            connector_input_evidence,
            draft_evidence,
            approval_evidence,
            execution_evidence,
            verification_evidence,
            rollback_evidence,
            sensitive_evidence,
            delivery_action,
            connector_action_guard_status: connector_action_guard_status.clone(),
            export_guard_status: export_guard_status.clone(),
        };

        let mut checks = Vec::new();
        push_connector_e2e_gate_check(
            &mut checks,
            "connector_input",
            if !thresholds.require_connector_input
                || (connector_input_evidence > 0 && connector.is_some())
            {
                "passed"
            } else {
                "insufficient_data"
            },
            "p1",
            "Connector/account source evidence is present",
            &format!(
                "connector={}, connectorInputEvidence={}",
                connector.as_deref().unwrap_or("unknown"),
                connector_input_evidence
            ),
            "A real connector E2E cannot pass without evidence from the external account or connector fixture.",
        );
        push_connector_e2e_gate_check(
            &mut checks,
            "draft_or_preview",
            if !thresholds.require_draft || draft_evidence > 0 {
                "passed"
            } else {
                "insufficient_data"
            },
            "p1",
            "Draft/preview evidence exists before the external mutation",
            &draft_evidence.to_string(),
            "Show the user the exact content or change preview before requesting approval.",
        );
        push_connector_e2e_gate_check(
            &mut checks,
            "explicit_user_approval",
            if !thresholds.require_explicit_approval || approval_evidence > 0 {
                "passed"
            } else {
                "failed"
            },
            "p0",
            "Explicit approval evidence is present",
            &approval_evidence.to_string(),
            "External connector mutations must not execute without user approval evidence.",
        );
        push_connector_e2e_gate_check(
            &mut checks,
            "action_execution",
            if !thresholds.require_execution_result || execution_evidence > 0 {
                "passed"
            } else {
                "insufficient_data"
            },
            "p1",
            "Execution result evidence exists with external result metadata",
            &execution_evidence.to_string(),
            "Record the connector result id/status after the external mutation succeeds or fails.",
        );
        push_connector_e2e_gate_check(
            &mut checks,
            "post_action_verification",
            if !thresholds.require_post_action_verification || verification_evidence > 0 {
                "passed"
            } else {
                "insufficient_data"
            },
            "p1",
            "Post-action verification evidence is present",
            &verification_evidence.to_string(),
            "Verify the external system state after the connector mutation.",
        );
        push_connector_e2e_gate_check(
            &mut checks,
            "rollback_plan",
            if !thresholds.require_rollback_plan || rollback_evidence > 0 {
                "passed"
            } else {
                "insufficient_data"
            },
            "p1",
            "Rollback or recovery plan evidence is present",
            &rollback_evidence.to_string(),
            "Record how the user can undo or recover from the external mutation.",
        );
        push_connector_e2e_gate_check(
            &mut checks,
            "connector_action_guard",
            match connector_action_guard
                .as_ref()
                .map(|guard| guard.status.as_str())
            {
                Some("passed") => "passed",
                Some("failed") => "failed",
                _ => "insufficient_data",
            },
            "p0",
            "Connector Action Guard passes",
            connector_action_guard_status
                .as_deref()
                .unwrap_or("not_evaluated_without_session_or_goal"),
            "The lower-level connector action guard must pass before the E2E is accepted.",
        );
        if thresholds.require_export_guard_for_delivery && delivery_action {
            let actual = export_guard_status
                .as_deref()
                .unwrap_or("not_evaluated_or_not_applicable");
            push_connector_e2e_gate_check(
                &mut checks,
                "artifact_export_guard",
                match actual {
                    "passed" => "passed",
                    "failed" => "failed",
                    _ => "insufficient_data",
                },
                "p0",
                "Delivery connector action passes Artifact Export Guard",
                actual,
                "Send/share/publish/upload/export actions must pass final artifact/export review.",
            );
        }

        let status = connector_e2e_gate_status(&checks);
        let blockers = checks
            .iter()
            .filter(|check| check.status != "passed")
            .map(|check| format!("{}: {}", check.name, check.detail))
            .collect::<Vec<_>>();
        let recommended_next_steps = connector_e2e_gate_recommendations(&checks);
        let related_evidence = evidence
            .iter()
            .filter(|item| {
                domain_evidence_has_connector_input(item)
                    || domain_evidence_has_connector_draft_marker(item)
                    || domain_evidence_has_connector_approval_marker(item)
                    || domain_evidence_has_connector_execution_marker(item)
                    || domain_evidence_has_connector_verification_marker(item)
                    || domain_evidence_has_rollback_marker(item)
                    || domain_evidence_requires_export_review(item)
            })
            .take(DOMAIN_CONNECTOR_E2E_GATE_EVIDENCE_MAX)
            .map(domain_connector_e2e_gate_evidence)
            .collect();

        Ok(DomainConnectorE2EGateReport {
            generated_at: now_rfc3339(),
            status,
            scope: DomainConnectorE2EGateScope {
                scope: if goal_id.is_some() {
                    "goal".to_string()
                } else if session_id.is_some() {
                    "session".to_string()
                } else if project_id.is_some() {
                    "project".to_string()
                } else {
                    "global".to_string()
                },
                goal_id,
                session_id,
                project_id,
                domain,
            },
            tool_name,
            connector,
            action,
            risk: Some("external_connector_e2e".to_string()),
            thresholds,
            summary,
            checks,
            blockers,
            recommended_next_steps,
            related_evidence,
        })
    }
}

fn built_in_domain_templates() -> Vec<DomainWorkflowTemplate> {
    vec![
        builtin_template(
            "research-brief",
            "Research brief",
            "research",
            &["market_research", "technical_research", "competitive_analysis"],
            "guarded",
            vec![
                req("source_cited", "At least three dated sources", true, Some(3), &["uri", "retrievedAt"]),
                req("claim_checked", "Important claims checked against evidence", true, Some(2), &["claim", "verdict"]),
                req("citation_audited", "Citation audit completed", true, Some(1), &["coverage"]),
            ],
            &["web_search", "web_fetch", "knowledge_recall"],
            vec![gate("external_publish", "User must approve before publishing or sharing research output", true)],
            vec![
                rule("citation_freshness", "blocking", "Flag undated or stale sources."),
                rule("claim_cross_check", "blocking", "Key claims need source support and conflict notes."),
            ],
            &["Sources conflict without resolution", "Required citations are missing"],
            "A concise answer-first research brief with cited sources, conflict notes, and next-step recommendations.",
            &["Every non-obvious claim needs an attached source.", "Separate facts, assumptions, and recommendations."],
            &["Prefer primary or official sources when available.", "Call out uncertainty instead of smoothing it over."],
        ),
        builtin_template(
            "writing-brief",
            "Structured writing deliverable",
            "writing",
            &["decision_memo", "prd", "weekly_report", "strategy_doc"],
            "guarded",
            vec![
                req("artifact_created", "Draft artifact created", true, Some(1), &["path", "version"]),
                req("artifact_reviewed", "Draft reviewed against audience and requirements", true, Some(1), &["audience", "issues"]),
                req("source_cited", "Supporting sources cited when factual claims appear", false, Some(1), &["uri"]),
            ],
            &["file_search", "read", "write"],
            vec![gate("final_send_or_share", "User approves before sending, publishing, or sharing", true)],
            vec![
                rule("structure_review", "blocking", "Check outline, audience fit, missing sections, and requirement coverage."),
                rule("citation_gap", "advisory", "Flag unsupported factual claims."),
            ],
            &["Audience or acceptance criteria are unclear", "User approval is required before publication"],
            "A polished document draft with explicit audience, structure, open questions, and review notes.",
            &["Draft must answer the user's actual decision or communication need."],
            &["Keep visible progress in tasks: outline, draft, review, finalize."],
        ),
        builtin_template(
            "data-analysis-readout",
            "Data analysis readout",
            "data_analysis",
            &["metric_diagnostic", "kpi_readout", "dashboard_review"],
            "guarded",
            vec![
                req("data_quality_checked", "Data quality checked", true, Some(1), &["dataset", "checks"]),
                req("claim_checked", "Metric interpretation checked", true, Some(1), &["metric", "denominator"]),
                req("artifact_created", "Report or dashboard artifact created", false, Some(1), &["artifact"]),
            ],
            &["knowledge_recall"],
            vec![gate("business_decision", "User confirms before acting on material business recommendation", true)],
            vec![
                rule("metric_definition", "blocking", "State numerator, denominator, time window, filters, and exclusions."),
                rule("sample_size", "blocking", "Flag insufficient sample size or missing source coverage."),
                rule("chart_review", "advisory", "Check for misleading chart encodings."),
            ],
            &["Data source or metric definition is missing", "Quality checks fail"],
            "An evidence-backed readout with metric definitions, caveats, drivers, and recommended action.",
            &["Every chart or number needs a named source and grain."],
            &["Prefer transparent uncertainty over false precision."],
        ),
        builtin_template(
            "meeting-prep",
            "Meeting prep brief",
            "meeting_prep",
            &["meeting_brief", "agenda_risk_review"],
            "guarded",
            vec![
                req("meeting_context_collected", "Meeting context collected", true, Some(1), &["event", "attendees"]),
                req("artifact_created", "Brief or agenda created", true, Some(1), &["artifact"]),
                req("user_decision", "Open asks or decisions identified", false, Some(1), &["decision"]),
            ],
            &["knowledge_recall"],
            vec![gate("calendar_or_message_change", "User approves calendar edits, messages, and external updates", true)],
            vec![
                rule("attendee_time_check", "blocking", "Check attendees, time, agenda, and materials."),
                rule("decision_points", "advisory", "Surface expected decisions and risks."),
            ],
            &["Calendar context is missing", "Required materials are unavailable"],
            "A meeting brief with context, goals, agenda, risks, decisions, and questions.",
            &["Separate preparation facts from suggested talking points."],
            &["Never send follow-ups or change calendar events without explicit approval."],
        ),
        builtin_template(
            "knowledge-curation",
            "Knowledge curation",
            "knowledge_curation",
            &["topic_index", "vault_cleanup", "source_synthesis"],
            "guarded",
            vec![
                req("source_cited", "Source notes or documents identified", true, Some(2), &["path", "title"]),
                req("artifact_reviewed", "Deduplication and gap review completed", true, Some(1), &["duplicates", "gaps"]),
                req("artifact_created", "Index or curated note created", true, Some(1), &["path"]),
            ],
            &["knowledge_recall", "note_search"],
            vec![gate("external_vault_write", "User approves writes to external knowledge roots", true)],
            vec![
                rule("dedupe", "blocking", "Flag duplicate or conflicting notes."),
                rule("link_integrity", "advisory", "Check missing or broken references."),
            ],
            &["Access to source notes is unavailable", "External vault write approval is missing"],
            "A curated note, index, or cleanup proposal with sources, tags, gaps, and safe write plan.",
            &["Preserve original source references and avoid destructive cleanup by default."],
            &["Draft first; apply only through explicit owner action."],
        ),
        builtin_template(
            "inbox-comms",
            "Inbox and communications",
            "inbox",
            &["reply_draft", "thread_triage", "follow_up_plan"],
            "guarded",
            vec![
                req("source_cited", "Relevant thread or message cited", true, Some(1), &["threadId", "messageId"]),
                req("message_draft_approved", "User approved message before send", true, Some(1), &["draftId"]),
                req("claim_checked", "Facts and commitments checked", true, Some(1), &["facts"]),
            ],
            &["knowledge_recall"],
            vec![gate("send_message", "User explicitly approves before sending, forwarding, archiving, or deleting", true)],
            vec![
                rule("recipient_attachment_check", "blocking", "Check recipients, attachments, facts, and tone."),
                rule("commitment_check", "blocking", "Surface promises, deadlines, and asks before approval."),
            ],
            &["User has not approved the outgoing message", "Thread facts are ambiguous"],
            "A triage summary or reply draft with explicit facts, commitments, recipients, and approval state.",
            &["Drafts are safe; sends and destructive mailbox actions require explicit approval."],
            &["Keep tone matched to the relationship and task."],
        ),
        builtin_template(
            "project-ops",
            "Project operations",
            "project_ops",
            &["status_update", "risk_register", "planning_review"],
            "guarded",
            vec![
                req("artifact_created", "Status or plan artifact created", true, Some(1), &["artifact"]),
                req("user_decision", "Owners, deadlines, or tradeoffs confirmed", true, Some(1), &["owner", "deadline"]),
                req("claim_checked", "Risks and dependencies checked", true, Some(1), &["risk", "dependency"]),
            ],
            &["knowledge_recall"],
            vec![gate("external_status_change", "User approves external status changes or task updates", true)],
            vec![
                rule("owner_deadline_check", "blocking", "Every action item needs an owner, deadline, and status."),
                rule("dependency_risk_check", "advisory", "Flag blocked dependencies and stale risks."),
            ],
            &["Owner or deadline is missing for critical actions", "External update requires approval"],
            "A project update or plan with owners, deadlines, risks, dependencies, and next actions.",
            &["Do not claim execution of external project changes without confirmation."],
            &["Make blockers and decisions easy to scan."],
        ),
    ]
}

fn builtin_template(
    id: &str,
    title: &str,
    domain: &str,
    task_types: &[&str],
    default_mode: &str,
    required_evidence: Vec<DomainEvidenceRequirement>,
    recommended_tools: &[&str],
    approval_gates: Vec<DomainApprovalGate>,
    verification_policy: Vec<DomainVerificationRule>,
    stop_conditions: &[&str],
    output_contract: &str,
    eval_criteria: &[&str],
    prompt_hints: &[&str],
) -> DomainWorkflowTemplate {
    let now = "builtin".to_string();
    DomainWorkflowTemplate {
        id: id.to_string(),
        version: "1.0.0".to_string(),
        title: title.to_string(),
        domain: normalize_domain(domain),
        task_types: task_types
            .iter()
            .map(|value| normalize_task_type(value))
            .collect(),
        default_mode: normalize_mode(default_mode),
        required_evidence,
        recommended_tools: recommended_tools
            .iter()
            .map(|value| value.to_string())
            .collect(),
        approval_gates,
        verification_policy,
        stop_conditions: stop_conditions
            .iter()
            .map(|value| value.to_string())
            .collect(),
        output_contract: output_contract.to_string(),
        eval_criteria: eval_criteria
            .iter()
            .map(|value| value.to_string())
            .collect(),
        prompt_hints: prompt_hints.iter().map(|value| value.to_string()).collect(),
        scope: "built_in".to_string(),
        project_id: None,
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
    }
}

fn req(
    evidence_type: &str,
    title: &str,
    required: bool,
    min_count: Option<usize>,
    metadata_keys: &[&str],
) -> DomainEvidenceRequirement {
    DomainEvidenceRequirement {
        evidence_type: evidence_type.to_string(),
        title: title.to_string(),
        required,
        min_count,
        metadata_keys: metadata_keys
            .iter()
            .map(|value| value.to_string())
            .collect(),
    }
}

fn gate(action: &str, reason: &str, required: bool) -> DomainApprovalGate {
    DomainApprovalGate {
        action: action.to_string(),
        reason: reason.to_string(),
        required,
    }
}

fn rule(rule: &str, severity: &str, description: &str) -> DomainVerificationRule {
    DomainVerificationRule {
        rule: rule.to_string(),
        severity: severity.to_string(),
        description: description.to_string(),
    }
}

fn render_domain_workflow_script(
    template: &DomainWorkflowTemplate,
    goal_id: Option<&str>,
    task_type: &str,
    objective: &str,
    user_context: &str,
    require_plan_confirmation: bool,
) -> String {
    let evidence = serde_json::to_string_pretty(&template.required_evidence).unwrap_or_default();
    let gates = serde_json::to_string_pretty(&template.approval_gates).unwrap_or_default();
    let verification =
        serde_json::to_string_pretty(&template.verification_policy).unwrap_or_default();
    let hints = template.prompt_hints.join("\n- ");
    let stop_conditions = template.stop_conditions.join("\n- ");
    let plan_confirmation_step = if require_plan_confirmation {
        format!(
            r#"  await workflow.askUser({{
    label: "domain-plan-confirmation",
    questions: [{{
      id: "confirm-domain-workflow",
      header: "Plan",
      question: {plan_question},
      options: [
        {{ label: "Proceed", value: "proceed", description: "Use this domain workflow and evidence plan." }},
        {{ label: "Adjust", value: "adjust", description: "Pause so the plan can be edited." }}
      ]
    }}]
  }});"#,
            plan_question = json_string(&format!(
                "Confirm this {} workflow plan before I proceed.",
                template.title
            )),
        )
    } else {
        r#"  await workflow.trace({
    label: "domain-plan-confirmation-skipped",
    payload: { reason: "loop_auto_workflow" }
  });"#
            .to_string()
    };
    format!(
        r#"export default async function main(workflow) {{
  const task = await workflow.task.create({{
    title: {task_title},
    label: "domain-{domain}"
  }});

  await workflow.task.update({{
    task,
    status: "in_progress",
    content: {task_content}
  }});

  const evidencePlan = {evidence};
  const approvalGates = {gates};
  const verificationPolicy = {verification};
  const budget = {{ max_runtime_secs: 300, max_ops: 16 }};

{plan_confirmation_step}

  await workflow.task.update({{
    task,
    content: [
      "Domain: {domain}",
      "Task type: {task_type}",
      "Goal: {goal_id}",
      "Required evidence: " + evidencePlan.map((item) => item.evidenceType + ":" + item.title).join(", "),
      "Approval gates: " + approvalGates.map((item) => item.action).join(", "),
      "Verification policy: " + verificationPolicy.map((item) => item.rule).join(", ")
    ].join("\n")
  }});

  const verificationPlan = await workflow.verify({{
    label: "domain-verification-plan",
    maxCommands: 3
  }});

  await workflow.finish({{
    status: "draft_ready",
    domain: {domain_json},
    templateId: {template_id_json},
    templateVersion: {template_version_json},
    taskType: {task_type_json},
    objective: {objective_json},
    outputContract: {output_contract},
    promptHints: {prompt_hints},
    stopConditions: {stop_conditions},
    verificationPlan,
    userContext: {user_context},
    budget
  }});
}}"#,
        task_title = json_string(&format!("{}: {}", template.title, objective)),
        task_content = json_string(&format!(
            "Prepare {} workflow draft and collect required evidence.",
            template.domain
        )),
        domain = template.domain,
        task_type = task_type,
        goal_id = goal_id.unwrap_or("none"),
        plan_confirmation_step = plan_confirmation_step,
        domain_json = json_string(&template.domain),
        template_id_json = json_string(&template.id),
        template_version_json = json_string(&template.version),
        task_type_json = json_string(task_type),
        objective_json = json_string(objective),
        output_contract = json_string(&template.output_contract),
        prompt_hints = json_string(&hints),
        stop_conditions = json_string(&stop_conditions),
        user_context = json_string(user_context),
    )
}

fn domain_workflow_warnings(template: &DomainWorkflowTemplate) -> Vec<String> {
    let mut warnings = Vec::new();
    if template.required_evidence.is_empty() {
        warnings.push("template has no required evidence".to_string());
    }
    if template.approval_gates.is_empty() {
        warnings.push("template has no approval gates".to_string());
    }
    if template.verification_policy.is_empty() {
        warnings.push("template has no verification policy".to_string());
    }
    warnings
}

fn normalize_template_draft(draft: DomainWorkflowTemplateDraft) -> Result<DomainWorkflowTemplate> {
    let id = normalize_template_id(&draft.id)?;
    let version = normalized_opt(Some(draft.version.as_str()))
        .unwrap_or("1.0.0")
        .to_string();
    let title = draft.title.trim().to_string();
    if title.is_empty() {
        bail!("domain workflow template title must not be empty");
    }
    let domain = normalize_domain(&draft.domain);
    let task_types = draft
        .task_types
        .iter()
        .map(|value| normalize_task_type(value))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if task_types.is_empty() {
        bail!("domain workflow template requires at least one task type");
    }
    let now = now_rfc3339();
    if normalize_scope(&draft.scope) == "project"
        && draft.project_id.as_deref().and_then(non_empty).is_none()
    {
        bail!("project-scoped domain workflow templates require project_id");
    }
    let required_evidence = draft
        .required_evidence
        .into_iter()
        .map(normalize_evidence_requirement)
        .collect::<Result<Vec<_>>>()?;
    let approval_gates = draft
        .approval_gates
        .into_iter()
        .map(normalize_approval_gate)
        .collect::<Result<Vec<_>>>()?;
    let verification_policy = draft
        .verification_policy
        .into_iter()
        .map(normalize_verification_rule)
        .collect::<Result<Vec<_>>>()?;
    Ok(DomainWorkflowTemplate {
        id,
        version,
        title,
        domain,
        task_types,
        default_mode: normalize_mode(&draft.default_mode),
        required_evidence,
        recommended_tools: draft.recommended_tools,
        approval_gates,
        verification_policy,
        stop_conditions: draft.stop_conditions,
        output_contract: draft.output_contract.trim().to_string(),
        eval_criteria: draft.eval_criteria,
        prompt_hints: draft.prompt_hints,
        scope: normalize_scope(&draft.scope),
        project_id: normalized_opt(draft.project_id.as_deref()).map(str::to_string),
        enabled: draft.enabled,
        created_at: now.clone(),
        updated_at: now,
    })
}

fn normalize_evidence_requirement(
    mut requirement: DomainEvidenceRequirement,
) -> Result<DomainEvidenceRequirement> {
    requirement.evidence_type = normalize_evidence_type(&requirement.evidence_type)?;
    requirement.title = requirement.title.trim().to_string();
    if requirement.title.is_empty() {
        bail!("domain evidence requirement title must not be empty");
    }
    requirement.metadata_keys = requirement
        .metadata_keys
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    Ok(requirement)
}

fn normalize_approval_gate(mut gate: DomainApprovalGate) -> Result<DomainApprovalGate> {
    gate.action = normalize_task_type(&gate.action);
    gate.reason = gate.reason.trim().to_string();
    if gate.action.is_empty() {
        bail!("domain approval gate action must not be empty");
    }
    if gate.reason.is_empty() {
        bail!("domain approval gate reason must not be empty");
    }
    Ok(gate)
}

fn normalize_verification_rule(mut rule: DomainVerificationRule) -> Result<DomainVerificationRule> {
    rule.rule = normalize_task_type(&rule.rule);
    rule.severity = match rule.severity.trim().to_ascii_lowercase().as_str() {
        "blocking" => "blocking".to_string(),
        "advisory" => "advisory".to_string(),
        "info" => "info".to_string(),
        _ => "advisory".to_string(),
    };
    rule.description = rule.description.trim().to_string();
    if rule.rule.is_empty() {
        bail!("domain verification rule name must not be empty");
    }
    if rule.description.is_empty() {
        bail!("domain verification rule description must not be empty");
    }
    Ok(rule)
}

fn domain_template_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DomainWorkflowTemplate> {
    let task_types_json: String = row.get(4)?;
    let required_evidence_json: String = row.get(6)?;
    let recommended_tools_json: String = row.get(7)?;
    let approval_gates_json: String = row.get(8)?;
    let verification_policy_json: String = row.get(9)?;
    let stop_conditions_json: String = row.get(10)?;
    let eval_criteria_json: String = row.get(12)?;
    let prompt_hints_json: String = row.get(13)?;
    Ok(DomainWorkflowTemplate {
        id: row.get(0)?,
        version: row.get(1)?,
        title: row.get(2)?,
        domain: row.get(3)?,
        task_types: serde_json::from_str(&task_types_json).unwrap_or_default(),
        default_mode: row.get(5)?,
        required_evidence: serde_json::from_str(&required_evidence_json).unwrap_or_default(),
        recommended_tools: serde_json::from_str(&recommended_tools_json).unwrap_or_default(),
        approval_gates: serde_json::from_str(&approval_gates_json).unwrap_or_default(),
        verification_policy: serde_json::from_str(&verification_policy_json).unwrap_or_default(),
        stop_conditions: serde_json::from_str(&stop_conditions_json).unwrap_or_default(),
        output_contract: row.get(11)?,
        eval_criteria: serde_json::from_str(&eval_criteria_json).unwrap_or_default(),
        prompt_hints: serde_json::from_str(&prompt_hints_json).unwrap_or_default(),
        scope: row.get(14)?,
        project_id: row.get(15)?,
        enabled: row.get::<_, i64>(16)? != 0,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
    })
}

fn domain_evidence_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DomainEvidenceItem> {
    let source_metadata_json: String = row.get(8)?;
    Ok(DomainEvidenceItem {
        id: row.get(0)?,
        goal_id: row.get(1)?,
        session_id: row.get(2)?,
        project_id: row.get(3)?,
        domain: row.get(4)?,
        evidence_type: row.get(5)?,
        title: row.get(6)?,
        summary: row.get(7)?,
        source_metadata: serde_json::from_str(&source_metadata_json).unwrap_or_else(|_| json!({})),
        confidence: row.get(9)?,
        access_scope: row.get(10)?,
        redaction_status: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

fn push_export_guard_check(
    checks: &mut Vec<DomainArtifactExportGuardCheck>,
    name: &str,
    status: &str,
    severity: &str,
    expected: &str,
    actual: &str,
    detail: &str,
) {
    checks.push(DomainArtifactExportGuardCheck {
        name: name.to_string(),
        status: status.to_string(),
        severity: severity.to_string(),
        expected: expected.to_string(),
        actual: actual.to_string(),
        detail: detail.to_string(),
    });
}

fn export_guard_status(checks: &[DomainArtifactExportGuardCheck]) -> String {
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

fn export_guard_recommendations(checks: &[DomainArtifactExportGuardCheck]) -> Vec<String> {
    let mut steps = Vec::new();
    for check in checks.iter().filter(|check| check.status != "passed") {
        let step = match check.name.as_str() {
            "evidence_scope" => {
                "Record the final task evidence before attempting to export or share the artifact."
            }
            "artifact_created" => {
                "Record artifact_created evidence with the final artifact path, title, or version."
            }
            "artifact_reviewed" => {
                "Run or record artifact_reviewed evidence for audience, requirements, and quality."
            }
            "redaction_status" => {
                "Resolve pending or sensitive redaction states before sharing outside the session."
            }
            "sensitive_evidence" => {
                "Add an explicit exportReview/exportReady/redactionChecked marker to artifact_reviewed evidence."
            }
            _ => "Review the blocked export guard check and add the missing evidence.",
        };
        if !steps.iter().any(|existing| existing == step) {
            steps.push(step.to_string());
        }
    }
    steps
}

fn domain_evidence_requires_export_review(item: &DomainEvidenceItem) -> bool {
    matches!(item.access_scope.as_str(), "private" | "connector")
        || matches!(
            item.redaction_status.as_str(),
            "sensitive" | "pending" | "redacted"
        )
}

fn domain_evidence_has_export_review_marker(item: &DomainEvidenceItem) -> bool {
    if item.evidence_type != "artifact_reviewed" {
        return false;
    }
    let metadata = &item.source_metadata;
    json_bool(metadata, "exportReview")
        || json_bool(metadata, "exportReady")
        || json_bool(metadata, "redactionChecked")
        || metadata
            .get("export")
            .is_some_and(|value| json_bool(value, "reviewed") || json_bool(value, "ready"))
        || metadata.get("review").is_some_and(|value| {
            json_bool(value, "exportReady") || json_bool(value, "redactionChecked")
        })
}

fn domain_export_guard_evidence(item: &DomainEvidenceItem) -> DomainArtifactExportGuardEvidence {
    DomainArtifactExportGuardEvidence {
        id: item.id.clone(),
        evidence_type: item.evidence_type.clone(),
        title: item.title.clone(),
        access_scope: item.access_scope.clone(),
        redaction_status: item.redaction_status.clone(),
        created_at: item.created_at.clone(),
        reason: domain_export_guard_evidence_reason(item),
    }
}

fn domain_export_guard_evidence_reason(item: &DomainEvidenceItem) -> String {
    if item.redaction_status == "pending" {
        "redaction pending".to_string()
    } else if item.redaction_status == "sensitive" {
        "sensitive evidence".to_string()
    } else if item.redaction_status == "redacted" {
        "redacted evidence requires export review".to_string()
    } else if item.access_scope == "private" {
        "private scope".to_string()
    } else if item.access_scope == "connector" {
        "connector scope".to_string()
    } else {
        "requires export review".to_string()
    }
}

fn push_connector_action_guard_check(
    checks: &mut Vec<DomainConnectorActionGuardCheck>,
    name: &str,
    status: &str,
    severity: &str,
    expected: &str,
    actual: &str,
    detail: &str,
) {
    checks.push(DomainConnectorActionGuardCheck {
        name: name.to_string(),
        status: status.to_string(),
        severity: severity.to_string(),
        expected: expected.to_string(),
        actual: actual.to_string(),
        detail: detail.to_string(),
    });
}

fn connector_action_guard_status(checks: &[DomainConnectorActionGuardCheck]) -> String {
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

fn connector_action_guard_recommendations(
    checks: &[DomainConnectorActionGuardCheck],
) -> Vec<String> {
    let mut steps = Vec::new();
    for check in checks.iter().filter(|check| check.status != "passed") {
        let step = match check.name.as_str() {
            "action_scope" => {
                "Record the target connector and external action before running the connector mutation."
            }
            "explicit_user_approval" => {
                "Ask the user to approve the exact external action and record message_draft_approved or user_decision evidence."
            }
            "rollback_plan" => {
                "Add rollbackPlan, undoPlan, recoveryPlan, or canRollback metadata so the action has a recovery path."
            }
            "artifact_export_guard" => {
                "Run the Artifact Export Guard and resolve final artifact review, sensitive source, or redaction blockers."
            }
            _ => "Review the blocked connector action guard check and add the missing evidence.",
        };
        if !steps.iter().any(|existing| existing == step) {
            steps.push(step.to_string());
        }
    }
    steps
}

fn domain_connector_actual(
    tool_name: &Option<String>,
    connector: &Option<String>,
    action: &Option<String>,
    action_evidence: usize,
) -> String {
    format!(
        "tool={}, connector={}, action={}, actionEvidence={}",
        tool_name.as_deref().unwrap_or("unknown"),
        connector.as_deref().unwrap_or("unknown"),
        action.as_deref().unwrap_or("unknown"),
        action_evidence
    )
}

fn domain_evidence_mentions_connector_action(item: &DomainEvidenceItem) -> bool {
    let metadata = &item.source_metadata;
    metadata.get("requestedAction").is_some()
        || metadata.get("action").is_some()
        || metadata.get("externalAction").is_some()
        || metadata.get("toolName").is_some()
        || metadata.get("connector").is_some()
        || json_bool(metadata, "highRiskAction")
}

fn domain_connector_action_from_evidence(item: &DomainEvidenceItem) -> Option<String> {
    ["requestedAction", "externalAction", "action", "toolName"]
        .iter()
        .find_map(|key| domain_connector_metadata_string(item, key))
        .map(|value| normalize_connector_action(&value))
}

fn domain_connector_metadata_string(item: &DomainEvidenceItem, key: &str) -> Option<String> {
    json_string_value(&item.source_metadata, key)
        .or_else(|| {
            item.source_metadata
                .get("connector")
                .and_then(|value| json_string_value(value, key))
        })
        .or_else(|| {
            item.source_metadata
                .get("action")
                .and_then(|value| json_string_value(value, key))
        })
}

fn domain_evidence_has_connector_approval_marker(item: &DomainEvidenceItem) -> bool {
    if matches!(
        item.evidence_type.as_str(),
        "message_draft_approved" | "user_decision"
    ) {
        return true;
    }
    let metadata = &item.source_metadata;
    json_bool(metadata, "explicitUserApproval")
        || json_bool(metadata, "userApproved")
        || json_bool(metadata, "approved")
        || metadata
            .get("approval")
            .is_some_and(|value| json_bool(value, "explicit") || json_bool(value, "approved"))
        || metadata
            .get("decision")
            .is_some_and(|value| json_bool(value, "approved") || json_bool(value, "confirmed"))
}

fn domain_evidence_has_rollback_marker(item: &DomainEvidenceItem) -> bool {
    let metadata = &item.source_metadata;
    json_bool(metadata, "canRollback")
        || json_string_value(metadata, "rollbackPlan").is_some()
        || json_string_value(metadata, "undoPlan").is_some()
        || json_string_value(metadata, "recoveryPlan").is_some()
        || metadata.get("rollback").is_some_and(|value| {
            json_bool(value, "available")
                || json_string_value(value, "plan").is_some()
                || json_string_value(value, "description").is_some()
        })
}

fn domain_connector_action_requires_export_guard(
    action: Option<&str>,
    tool_name: Option<&str>,
) -> bool {
    let mut text = String::new();
    if let Some(action) = action {
        text.push_str(action);
        text.push(' ');
    }
    if let Some(tool_name) = tool_name {
        text.push_str(tool_name);
    }
    let text = text.to_ascii_lowercase();
    [
        "send", "reply", "forward", "share", "publish", "export", "upload", "submit",
    ]
    .iter()
    .any(|keyword| text.contains(keyword))
}

fn domain_connector_action_guard_evidence(
    item: &DomainEvidenceItem,
) -> DomainConnectorActionGuardEvidence {
    DomainConnectorActionGuardEvidence {
        id: item.id.clone(),
        evidence_type: item.evidence_type.clone(),
        title: item.title.clone(),
        access_scope: item.access_scope.clone(),
        redaction_status: item.redaction_status.clone(),
        created_at: item.created_at.clone(),
        reason: domain_connector_action_guard_evidence_reason(item),
    }
}

fn domain_connector_action_guard_evidence_reason(item: &DomainEvidenceItem) -> String {
    if domain_evidence_has_connector_approval_marker(item) {
        "approval evidence".to_string()
    } else if domain_evidence_has_rollback_marker(item) {
        "rollback plan".to_string()
    } else if domain_evidence_mentions_connector_action(item) {
        "external action evidence".to_string()
    } else if domain_evidence_requires_export_review(item) {
        "sensitive source".to_string()
    } else {
        "related evidence".to_string()
    }
}

fn push_connector_e2e_gate_check(
    checks: &mut Vec<DomainConnectorE2EGateCheck>,
    name: &str,
    status: &str,
    severity: &str,
    expected: &str,
    actual: &str,
    detail: &str,
) {
    checks.push(DomainConnectorE2EGateCheck {
        name: name.to_string(),
        status: status.to_string(),
        severity: severity.to_string(),
        expected: expected.to_string(),
        actual: actual.to_string(),
        detail: detail.to_string(),
    });
}

fn connector_e2e_gate_status(checks: &[DomainConnectorE2EGateCheck]) -> String {
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

fn connector_e2e_gate_recommendations(checks: &[DomainConnectorE2EGateCheck]) -> Vec<String> {
    let mut steps = Vec::new();
    for check in checks.iter().filter(|check| check.status != "passed") {
        let step = match check.name.as_str() {
            "connector_input" => {
                "Run the connector read step or attach a deterministic connector fixture before claiming E2E coverage."
            }
            "draft_or_preview" => {
                "Record connector_draft_created, artifact_created, or message_draft_approved evidence with the exact preview shown to the user."
            }
            "explicit_user_approval" => {
                "Ask the user to approve the exact external mutation and record explicit approval evidence before execution."
            }
            "action_execution" => {
                "Record connector_action_executed evidence with connector, action, result id, and status after the mutation."
            }
            "post_action_verification" => {
                "Read the external system back and record connector_action_verified or reviewed evidence."
            }
            "rollback_plan" => {
                "Add rollbackPlan, undoPlan, recoveryPlan, or canRollback metadata for the external action."
            }
            "connector_action_guard" => {
                "Resolve the Connector Action Guard blockers before accepting the full connector E2E."
            }
            "artifact_export_guard" => {
                "Resolve Artifact Export Guard blockers for send/share/publish/upload/export actions."
            }
            _ => "Review the blocked connector E2E check and add the missing evidence.",
        };
        if !steps.iter().any(|existing| existing == step) {
            steps.push(step.to_string());
        }
    }
    steps
}

fn domain_evidence_has_connector_input(item: &DomainEvidenceItem) -> bool {
    item.access_scope == "connector"
        || domain_connector_metadata_string(item, "connector").is_some()
        || json_string_value(&item.source_metadata, "accountId").is_some()
        || json_string_value(&item.source_metadata, "externalSource").is_some()
        || item
            .source_metadata
            .get("connector")
            .is_some_and(|value| !value.is_null())
}

fn domain_evidence_has_connector_draft_marker(item: &DomainEvidenceItem) -> bool {
    matches!(
        item.evidence_type.as_str(),
        "connector_draft_created" | "message_draft_approved"
    ) || (item.evidence_type == "artifact_created"
        && (json_bool(&item.source_metadata, "draftCreated")
            || json_bool(&item.source_metadata, "previewReady")
            || item.source_metadata.get("draft").is_some()
            || item.source_metadata.get("preview").is_some()
            || domain_evidence_mentions_connector_action(item)))
}

fn domain_evidence_has_connector_execution_marker(item: &DomainEvidenceItem) -> bool {
    if item.evidence_type == "connector_action_executed" {
        return true;
    }
    let metadata = &item.source_metadata;
    json_bool(metadata, "actionExecuted")
        || json_bool(metadata, "executed")
        || metadata.get("execution").is_some_and(|value| {
            json_bool(value, "success")
                || matches!(
                    json_string_value(value, "status")
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .as_str(),
                    "success" | "succeeded" | "sent" | "created" | "updated" | "done"
                )
        })
        || metadata.get("result").is_some_and(|value| {
            json_bool(value, "success")
                || json_string_value(value, "id").is_some()
                || matches!(
                    json_string_value(value, "status")
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .as_str(),
                    "success" | "succeeded" | "sent" | "created" | "updated" | "done"
                )
        })
}

fn domain_evidence_has_connector_verification_marker(item: &DomainEvidenceItem) -> bool {
    if item.evidence_type == "connector_action_verified" {
        return true;
    }
    let metadata = &item.source_metadata;
    json_bool(metadata, "postActionVerification")
        || json_bool(metadata, "deliveryVerified")
        || json_bool(metadata, "externalStateVerified")
        || metadata.get("verification").is_some_and(|value| {
            json_bool(value, "passed")
                || json_bool(value, "verified")
                || matches!(
                    json_string_value(value, "status")
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .as_str(),
                    "passed" | "verified" | "success" | "succeeded"
                )
        })
        || (item.evidence_type == "artifact_reviewed"
            && (json_bool(metadata, "postActionReview")
                || json_bool(metadata, "verifiedAfterSend")
                || json_bool(metadata, "externalStateVerified")))
}

fn domain_connector_e2e_gate_evidence(item: &DomainEvidenceItem) -> DomainConnectorE2EGateEvidence {
    DomainConnectorE2EGateEvidence {
        id: item.id.clone(),
        evidence_type: item.evidence_type.clone(),
        title: item.title.clone(),
        access_scope: item.access_scope.clone(),
        redaction_status: item.redaction_status.clone(),
        created_at: item.created_at.clone(),
        reason: domain_connector_e2e_gate_evidence_reason(item),
    }
}

fn domain_connector_e2e_gate_evidence_reason(item: &DomainEvidenceItem) -> String {
    if domain_evidence_has_connector_execution_marker(item) {
        "connector execution result".to_string()
    } else if domain_evidence_has_connector_verification_marker(item) {
        "post-action verification".to_string()
    } else if domain_evidence_has_connector_approval_marker(item) {
        "user approval".to_string()
    } else if domain_evidence_has_connector_draft_marker(item) {
        "draft or preview".to_string()
    } else if domain_evidence_has_rollback_marker(item) {
        "rollback plan".to_string()
    } else if domain_evidence_has_connector_input(item) {
        "connector input".to_string()
    } else if domain_evidence_requires_export_review(item) {
        "export-sensitive evidence".to_string()
    } else {
        "related evidence".to_string()
    }
}

fn json_bool(value: &Value, key: &str) -> bool {
    match value.get(key) {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "true" | "yes" | "1"
            )
        }
        _ => false,
    }
}

fn json_string_value(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalize_template_id(value: &str) -> Result<String> {
    let id = value.trim().to_ascii_lowercase().replace('_', "-");
    if id.is_empty() {
        bail!("domain workflow template id must not be empty");
    }
    if !id
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        bail!("domain workflow template id must contain only lowercase letters, digits, or '-'");
    }
    Ok(id)
}

fn normalize_domain(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

fn normalize_task_type(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

fn normalize_connector_action(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

fn normalize_mode(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" => "off".to_string(),
        "guarded" | "smart" | "" => "guarded".to_string(),
        "deep" => "deep".to_string(),
        "autonomous" => "autonomous".to_string(),
        _ => "guarded".to_string(),
    }
}

fn normalize_scope(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "project" => "project".to_string(),
        "user" | "global" | "" => "user".to_string(),
        _ => "user".to_string(),
    }
}

fn normalize_evidence_type(value: &str) -> Result<String> {
    let value = normalize_task_type(value);
    match value.as_str() {
        "source_cited"
        | "claim_checked"
        | "user_decision"
        | "artifact_created"
        | "artifact_reviewed"
        | "data_quality_checked"
        | "citation_audited"
        | "message_draft_approved"
        | "meeting_context_collected"
        | "connector_context_collected"
        | "connector_draft_created"
        | "connector_action_executed"
        | "connector_action_verified" => Ok(value),
        _ => bail!("unsupported domain evidence type: {value}"),
    }
}

fn normalize_access_scope(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "public" => "public".to_string(),
        "project" => "project".to_string(),
        "connector" => "connector".to_string(),
        "private" => "private".to_string(),
        "session" | "" => "session".to_string(),
        _ => "session".to_string(),
    }
}

fn normalize_redaction_status(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "redacted" => "redacted".to_string(),
        "pending" => "pending".to_string(),
        "sensitive" => "sensitive".to_string(),
        "none" | "" => "none".to_string(),
        _ => "none".to_string(),
    }
}

fn ensure_object(value: Value) -> Value {
    if value.is_object() {
        value
    } else {
        json!({ "value": value })
    }
}

fn domain_rank(domain: &str) -> usize {
    match domain {
        "research" => 0,
        "writing" => 1,
        "data_analysis" => 2,
        "meeting_prep" => 3,
        "knowledge_curation" => 4,
        "inbox" => 5,
        "project_ops" => 6,
        _ => 99,
    }
}

fn normalized_opt(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>> {
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn emit_domain_evidence_recorded(item: &DomainEvidenceItem) {
    let Some(bus) = crate::globals::get_event_bus() else {
        return;
    };
    bus.emit(
        EVENT_DOMAIN_EVIDENCE_RECORDED,
        json!({
            "id": item.id,
            "sessionId": item.session_id,
            "goalId": item.goal_id,
            "projectId": item.project_id,
            "domain": item.domain,
            "evidenceType": item.evidence_type,
            "title": item.title,
            "createdAt": item.created_at,
        }),
    );
}

fn stable_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn serde_default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goal::CreateGoalInput;
    use crate::session::SessionDB;
    use tempfile::tempdir;

    struct TestDb {
        _dir: tempfile::TempDir,
        db: SessionDB,
    }

    fn test_db() -> TestDb {
        let dir = tempdir().expect("tempdir");
        let db = SessionDB::open(&dir.path().join("sessions.db")).expect("open db");
        ensure_channel_conversations_table(&db);
        TestDb { _dir: dir, db }
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

    fn create_session(db: &SessionDB) -> String {
        db.create_session("ha-main").expect("create session").id
    }

    #[test]
    fn domain_workflow_registry_lists_builtins_and_previews_script() {
        let test = test_db();
        let db = &test.db;
        let session_id = create_session(&db);
        assert_eq!(normalize_mode("deep"), "deep");
        assert_eq!(normalize_mode("autonomous"), "autonomous");
        let templates = db
            .list_domain_workflow_templates(ListDomainWorkflowTemplatesInput {
                domain: Some("research".to_string()),
                ..Default::default()
            })
            .expect("list templates");
        assert!(templates
            .iter()
            .any(|template| template.id == "research-brief"));

        let draft = db
            .preview_domain_workflow(PreviewDomainWorkflowInput {
                template_id: "research-brief".to_string(),
                session_id,
                objective: Some("Compare two AI coding agents".to_string()),
                ..Default::default()
            })
            .expect("preview domain workflow");
        assert_eq!(draft.template.domain, "research");
        assert_eq!(draft.execution_mode, "guarded");
        assert!(draft.script_source.contains("workflow.askUser"));
        assert!(
            draft
                .script_preview
                .gate
                .issues
                .iter()
                .all(|issue| !format!("{:?}", issue.severity).eq_ignore_ascii_case("error")),
            "script gate errors: {:?}",
            draft.script_preview.gate.issues
        );
    }

    #[test]
    fn domain_evidence_links_into_goal_snapshot() {
        let test = test_db();
        let db = &test.db;
        let session_id = create_session(&db);
        let goal = db
            .create_goal(CreateGoalInput {
                session_id: session_id.clone(),
                objective: "Write a sourced research brief".to_string(),
                completion_criteria: "brief includes cited sources".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");
        let evidence = db
            .record_domain_evidence(RecordDomainEvidenceInput {
                goal_id: Some(goal.goal.id.clone()),
                domain: "research".to_string(),
                evidence_type: "source_cited".to_string(),
                title: "Official source cited".to_string(),
                summary: Some("Source supports the brief".to_string()),
                source_metadata: json!({
                    "title": "Official docs",
                    "uri": "https://example.com/docs",
                    "retrievedAt": "2026-07-03T00:00:00Z"
                }),
                confidence: Some(0.9),
                access_scope: Some("public".to_string()),
                redaction_status: Some("none".to_string()),
                ..Default::default()
            })
            .expect("record evidence");
        let other_session_id = create_session(db);
        let cross_session_err = db
            .record_domain_evidence(RecordDomainEvidenceInput {
                goal_id: Some(goal.goal.id.clone()),
                session_id: Some(other_session_id),
                domain: "research".to_string(),
                evidence_type: "source_cited".to_string(),
                title: "Cross-session source".to_string(),
                ..Default::default()
            })
            .expect_err("cross-session evidence should fail");
        assert!(
            cross_session_err
                .to_string()
                .contains("does not belong to session"),
            "{cross_session_err}"
        );
        let items = db
            .list_domain_evidence(ListDomainEvidenceInput {
                goal_id: Some(goal.goal.id.clone()),
                ..Default::default()
            })
            .expect("list evidence");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, evidence.id);

        let snapshot = db
            .goal_snapshot(&goal.goal.id, 100)
            .expect("goal snapshot")
            .expect("goal exists");
        assert!(snapshot
            .evidence
            .iter()
            .any(|item| item.source_type == "domain_evidence"
                && item.relation == "source_cited"
                && item.title.contains("Official source cited")));
    }

    #[test]
    fn owner_ask_user_evidence_and_terminal_state_commit_once() {
        let test = test_db();
        let db = &test.db;
        let session_id = create_session(db);
        let group: crate::ask_user::AskUserQuestionGroup = serde_json::from_value(json!({
            "requestId": "owner-answer-once",
            "sessionId": session_id,
            "questions": [],
            "source": "owner",
            "ownerResponse": { "action": "record_domain_evidence" }
        }))
        .expect("deserialize owner question");
        db.save_ask_user_group(&group).expect("save owner question");
        let input = RecordDomainEvidenceInput {
            session_id: Some(group.session_id.clone()),
            domain: "general".to_string(),
            evidence_type: "user_decision".to_string(),
            title: "Owner answer".to_string(),
            summary: Some("approved".to_string()),
            ..Default::default()
        };

        db.record_owner_ask_user_evidence_and_answer(&group.request_id, input.clone())
            .expect("first answer commits");
        assert!(db
            .record_owner_ask_user_evidence_and_answer(&group.request_id, input)
            .is_err());
        let evidence = db
            .list_domain_evidence(ListDomainEvidenceInput {
                session_id: Some(group.session_id.clone()),
                ..Default::default()
            })
            .expect("list evidence");
        assert_eq!(
            evidence.len(),
            1,
            "duplicate answers must not duplicate evidence"
        );
        assert!(db
            .get_pending_ask_user_group_by_request_id(&group.request_id)
            .expect("read pending")
            .is_none());
    }

    #[test]
    fn domain_artifact_export_guard_passes_with_reviewed_artifact_and_redaction_check() {
        let test = test_db();
        let db = &test.db;
        let session_id = create_session(&db);
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "writing".to_string(),
            evidence_type: "artifact_created".to_string(),
            title: "Decision memo draft".to_string(),
            source_metadata: json!({ "path": "memo.md", "version": "v1" }),
            access_scope: Some("session".to_string()),
            redaction_status: Some("none".to_string()),
            ..Default::default()
        })
        .expect("record artifact");
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "writing".to_string(),
            evidence_type: "source_cited".to_string(),
            title: "Private source summarized".to_string(),
            source_metadata: json!({ "connector": "drive", "title": "Internal brief" }),
            access_scope: Some("private".to_string()),
            redaction_status: Some("redacted".to_string()),
            ..Default::default()
        })
        .expect("record sensitive evidence");
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "writing".to_string(),
            evidence_type: "artifact_reviewed".to_string(),
            title: "Decision memo export review".to_string(),
            source_metadata: json!({
                "audience": "external stakeholder",
                "exportReview": true,
                "redactionChecked": true
            }),
            access_scope: Some("session".to_string()),
            redaction_status: Some("none".to_string()),
            ..Default::default()
        })
        .expect("record review");

        let guard = db
            .evaluate_domain_artifact_export_guard(DomainArtifactExportGuardInput {
                session_id: Some(session_id),
                domain: Some("writing".to_string()),
                ..Default::default()
            })
            .expect("evaluate export guard");
        assert_eq!(guard.status, "passed");
        assert_eq!(guard.summary.artifact_created, 1);
        assert_eq!(guard.summary.artifact_reviewed, 1);
        assert_eq!(guard.summary.export_reviewed, 1);
        assert_eq!(guard.summary.sensitive_evidence, 1);
        assert_eq!(guard.summary.sensitive_unreviewed, 0);
        assert_eq!(guard.evidence_requiring_review.len(), 1);
    }

    #[test]
    fn domain_artifact_export_guard_blocks_pending_sensitive_evidence() {
        let test = test_db();
        let db = &test.db;
        let session_id = create_session(&db);
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "writing".to_string(),
            evidence_type: "artifact_created".to_string(),
            title: "Decision memo draft".to_string(),
            source_metadata: json!({ "path": "memo.md" }),
            ..Default::default()
        })
        .expect("record artifact");
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "writing".to_string(),
            evidence_type: "source_cited".to_string(),
            title: "Connector source still pending".to_string(),
            source_metadata: json!({ "connector": "gmail", "threadId": "thr_1" }),
            access_scope: Some("connector".to_string()),
            redaction_status: Some("pending".to_string()),
            ..Default::default()
        })
        .expect("record pending evidence");

        let guard = db
            .evaluate_domain_artifact_export_guard(DomainArtifactExportGuardInput {
                session_id: Some(session_id),
                domain: Some("writing".to_string()),
                ..Default::default()
            })
            .expect("evaluate export guard");
        assert_eq!(guard.status, "failed");
        assert!(guard
            .checks
            .iter()
            .any(|check| check.name == "artifact_reviewed" && check.status == "insufficient_data"));
        assert!(guard
            .checks
            .iter()
            .any(|check| check.name == "redaction_status" && check.status == "failed"));
        assert!(guard
            .checks
            .iter()
            .any(|check| check.name == "sensitive_evidence" && check.status == "failed"));
        assert_eq!(guard.summary.redaction_pending, 1);
        assert_eq!(guard.summary.sensitive_unreviewed, 1);
    }

    #[test]
    fn domain_connector_action_guard_passes_with_approval_rollback_and_export_review() {
        let test = test_db();
        let db = &test.db;
        let session_id = create_session(&db);
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "inbox".to_string(),
            evidence_type: "artifact_created".to_string(),
            title: "Reply draft".to_string(),
            source_metadata: json!({
                "path": "reply.md",
                "requestedAction": "send email",
                "connector": "gmail"
            }),
            ..Default::default()
        })
        .expect("record draft");
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "inbox".to_string(),
            evidence_type: "artifact_reviewed".to_string(),
            title: "Reply reviewed for export".to_string(),
            source_metadata: json!({
                "exportReview": true,
                "redactionChecked": true
            }),
            ..Default::default()
        })
        .expect("record export review");
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "inbox".to_string(),
            evidence_type: "message_draft_approved".to_string(),
            title: "User approved sending reply".to_string(),
            source_metadata: json!({
                "explicitUserApproval": true,
                "requestedAction": "send email",
                "connector": "gmail",
                "rollbackPlan": "Send a follow-up correction if the message is wrong."
            }),
            ..Default::default()
        })
        .expect("record approval");

        let guard = db
            .evaluate_domain_connector_action_guard(DomainConnectorActionGuardInput {
                session_id: Some(session_id),
                domain: Some("inbox".to_string()),
                tool_name: Some("mcp__gmail__send_email".to_string()),
                ..Default::default()
            })
            .expect("evaluate connector guard");

        assert_eq!(guard.status, "passed");
        assert_eq!(guard.connector.as_deref(), Some("gmail"));
        assert_eq!(guard.action.as_deref(), Some("send message"));
        assert_eq!(guard.summary.approval_evidence, 1);
        assert_eq!(guard.summary.rollback_evidence, 1);
        assert_eq!(guard.summary.export_guard_status.as_deref(), Some("passed"));
    }

    #[test]
    fn domain_connector_action_guard_blocks_missing_explicit_approval() {
        let test = test_db();
        let db = &test.db;
        let session_id = create_session(&db);
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "meeting_prep".to_string(),
            evidence_type: "meeting_context_collected".to_string(),
            title: "Calendar event context".to_string(),
            source_metadata: json!({
                "requestedAction": "create calendar event",
                "connector": "calendar",
                "rollbackPlan": "Delete the event if the time is wrong."
            }),
            access_scope: Some("connector".to_string()),
            ..Default::default()
        })
        .expect("record context");

        let guard = db
            .evaluate_domain_connector_action_guard(DomainConnectorActionGuardInput {
                session_id: Some(session_id),
                domain: Some("meeting_prep".to_string()),
                tool_name: Some(crate::tools::feishu::TOOL_CALENDAR_CREATE_EVENT.to_string()),
                ..Default::default()
            })
            .expect("evaluate connector guard");

        assert_eq!(guard.status, "failed");
        assert!(guard
            .checks
            .iter()
            .any(|check| check.name == "explicit_user_approval" && check.status == "failed"));
        assert_eq!(guard.summary.rollback_evidence, 1);
    }

    #[test]
    fn domain_connector_e2e_gate_passes_with_full_connector_lifecycle() {
        let test = test_db();
        let db = &test.db;
        let session_id = create_session(&db);
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "inbox".to_string(),
            evidence_type: "connector_context_collected".to_string(),
            title: "Gmail thread loaded".to_string(),
            source_metadata: json!({
                "connector": "gmail",
                "accountId": "acct_test",
                "threadId": "thr_1",
                "requestedAction": "send email"
            }),
            access_scope: Some("connector".to_string()),
            ..Default::default()
        })
        .expect("record connector input");
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "inbox".to_string(),
            evidence_type: "connector_draft_created".to_string(),
            title: "Reply draft previewed".to_string(),
            source_metadata: json!({
                "connector": "gmail",
                "requestedAction": "send email",
                "previewReady": true
            }),
            ..Default::default()
        })
        .expect("record draft");
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "inbox".to_string(),
            evidence_type: "message_draft_approved".to_string(),
            title: "User approved reply send".to_string(),
            source_metadata: json!({
                "connector": "gmail",
                "requestedAction": "send email",
                "explicitUserApproval": true,
                "rollbackPlan": "Send a correction reply if the content is wrong."
            }),
            ..Default::default()
        })
        .expect("record approval");
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "inbox".to_string(),
            evidence_type: "artifact_created".to_string(),
            title: "Reply artifact".to_string(),
            source_metadata: json!({
                "connector": "gmail",
                "requestedAction": "send email",
                "path": "reply.md",
                "draftCreated": true
            }),
            ..Default::default()
        })
        .expect("record artifact");
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "inbox".to_string(),
            evidence_type: "artifact_reviewed".to_string(),
            title: "Reply export reviewed".to_string(),
            source_metadata: json!({
                "exportReview": true,
                "redactionChecked": true
            }),
            ..Default::default()
        })
        .expect("record export review");
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "inbox".to_string(),
            evidence_type: "connector_action_executed".to_string(),
            title: "Gmail reply sent".to_string(),
            source_metadata: json!({
                "connector": "gmail",
                "action": "send email",
                "messageId": "msg_1",
                "execution": { "status": "sent" }
            }),
            access_scope: Some("connector".to_string()),
            ..Default::default()
        })
        .expect("record execution");
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "inbox".to_string(),
            evidence_type: "connector_action_verified".to_string(),
            title: "Gmail sent state verified".to_string(),
            source_metadata: json!({
                "connector": "gmail",
                "messageId": "msg_1",
                "verification": { "status": "verified" }
            }),
            access_scope: Some("connector".to_string()),
            ..Default::default()
        })
        .expect("record verification");

        let gate = db
            .evaluate_domain_connector_e2e_gate(DomainConnectorE2EGateInput {
                session_id: Some(session_id),
                domain: Some("inbox".to_string()),
                tool_name: Some("mcp__gmail__send_email".to_string()),
                ..Default::default()
            })
            .expect("evaluate connector e2e gate");

        assert_eq!(gate.status, "passed");
        assert_eq!(gate.scope.scope, "session");
        assert_eq!(gate.summary.connector_input_evidence, 6);
        assert_eq!(gate.summary.draft_evidence, 3);
        assert_eq!(gate.summary.approval_evidence, 1);
        assert_eq!(gate.summary.execution_evidence, 1);
        assert_eq!(gate.summary.verification_evidence, 1);
        assert_eq!(gate.summary.rollback_evidence, 1);
        assert_eq!(
            gate.summary.connector_action_guard_status.as_deref(),
            Some("passed")
        );
        assert_eq!(gate.summary.export_guard_status.as_deref(), Some("passed"));
    }

    #[test]
    fn domain_connector_e2e_gate_keeps_missing_execution_as_insufficient_data() {
        let test = test_db();
        let db = &test.db;
        let session_id = create_session(&db);
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "meeting_prep".to_string(),
            evidence_type: "connector_context_collected".to_string(),
            title: "Calendar context loaded".to_string(),
            source_metadata: json!({
                "connector": "calendar",
                "requestedAction": "create calendar event"
            }),
            access_scope: Some("connector".to_string()),
            ..Default::default()
        })
        .expect("record connector input");
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "meeting_prep".to_string(),
            evidence_type: "connector_draft_created".to_string(),
            title: "Calendar event previewed".to_string(),
            source_metadata: json!({
                "connector": "calendar",
                "requestedAction": "create calendar event",
                "previewReady": true
            }),
            ..Default::default()
        })
        .expect("record draft");
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.clone()),
            domain: "meeting_prep".to_string(),
            evidence_type: "user_decision".to_string(),
            title: "User approved creating event".to_string(),
            source_metadata: json!({
                "connector": "calendar",
                "requestedAction": "create calendar event",
                "explicitUserApproval": true,
                "rollbackPlan": "Delete the event if the attendees or time are wrong."
            }),
            ..Default::default()
        })
        .expect("record approval");

        let gate = db
            .evaluate_domain_connector_e2e_gate(DomainConnectorE2EGateInput {
                session_id: Some(session_id),
                domain: Some("meeting_prep".to_string()),
                connector: Some("calendar".to_string()),
                action: Some("create calendar event".to_string()),
                require_export_guard_for_delivery: false,
                ..Default::default()
            })
            .expect("evaluate connector e2e gate");

        assert_eq!(gate.status, "insufficient_data");
        assert!(gate
            .checks
            .iter()
            .any(|check| check.name == "action_execution" && check.status == "insufficient_data"));
        assert!(gate
            .checks
            .iter()
            .any(|check| check.name == "explicit_user_approval" && check.status == "passed"));
    }
}
