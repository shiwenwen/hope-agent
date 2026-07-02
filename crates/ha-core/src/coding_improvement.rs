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
//! eval fixtures, project guidance includes, or active managed skills. Neither
//! apply nor promotion is implicit.

use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::review::{ReviewFindingStatus, ReviewSeverity};
use crate::session::SessionDB;
use crate::skills::SkillStatus;
use crate::util::now_rfc3339;
use crate::verification::VerificationStepState;
use crate::workflow::{WorkflowOp, WorkflowRun, WorkflowRunState};

const DEFAULT_WINDOW_DAYS: u32 = 30;
const MAX_WINDOW_DAYS: u32 = 180;
const MAX_SCOPE_SESSIONS: usize = 200;
const MAX_CONTENT_PREVIEW_BYTES: usize = 12 * 1024;

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
    content: Option<String>,
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
    content: Option<String>,
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

struct ReportScope {
    session_id: String,
    project_id: Option<String>,
    session_ids: Vec<String>,
    window_days: u32,
    since: String,
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
    pub fn coding_trend_report(
        &self,
        session_id: &str,
        window_days: Option<u32>,
    ) -> Result<CodingTrendReport> {
        let scope = self.resolve_coding_report_scope(session_id, window_days)?;
        let mut report = self.build_coding_trend_report(&scope)?;
        report.proposals = self.list_coding_improvement_proposals_for_scope(&scope)?;
        Ok(report)
    }

    pub fn ensure_coding_workflow_retro_for_run(
        &self,
        run: &WorkflowRun,
    ) -> Result<Option<CodingWorkflowRetro>> {
        if !run.state.is_terminal() {
            return Ok(None);
        }
        let meta = self
            .get_session(&run.session_id)?
            .ok_or_else(|| anyhow!("session not found: {}", run.session_id))?;
        if meta.incognito {
            return Ok(None);
        }
        let ops = self.list_workflow_ops(&run.id).unwrap_or_default();
        let retro = build_workflow_retro(run, meta.project_id.clone(), &ops);
        self.upsert_coding_workflow_retro(retro)?;
        self.get_coding_workflow_retro_for_run(&run.id)
    }

    pub fn generate_coding_improvement_proposals(
        &self,
        session_id: &str,
        window_days: Option<u32>,
    ) -> Result<GenerateCodingImprovementProposalsResult> {
        let scope = self.resolve_coding_report_scope(session_id, window_days)?;
        let report = self.build_coding_trend_report(&scope)?;
        let candidates = build_proposal_candidates(&report);
        let mut inserted = 0usize;
        for candidate in candidates {
            if self.insert_coding_improvement_proposal(&scope, candidate)? {
                inserted += 1;
            }
        }
        let proposals = self.list_coding_improvement_proposals_for_scope(&scope)?;
        Ok(GenerateCodingImprovementProposalsResult {
            inserted,
            proposals,
        })
    }

    pub fn list_coding_improvement_proposals(
        &self,
        session_id: &str,
    ) -> Result<Vec<CodingImprovementProposal>> {
        let scope = self.resolve_coding_report_scope(session_id, None)?;
        self.list_coding_improvement_proposals_for_scope(&scope)
    }

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

    pub fn preview_coding_improvement_proposal_action(
        &self,
        proposal_id: &str,
    ) -> Result<CodingImprovementActionPlan> {
        let proposal = self
            .get_coding_improvement_proposal(proposal_id)?
            .ok_or_else(|| anyhow!("coding improvement proposal not found: {proposal_id}"))?;
        self.build_coding_improvement_action_plan(proposal)
    }

    pub fn apply_coding_improvement_proposal(
        &self,
        proposal_id: &str,
    ) -> Result<ApplyCodingImprovementProposalResult> {
        let proposal = self.claim_coding_improvement_proposal_apply(proposal_id)?;
        let mut plan_proposal = proposal.clone();
        plan_proposal.status = "draft".to_string();
        let plan = match self.build_coding_improvement_action_plan(plan_proposal) {
            Ok(plan) => plan,
            Err(err) => {
                let message = err.to_string();
                let record = CodingImprovementActionRecord {
                    applied: false,
                    artifacts: Vec::new(),
                    error: Some(message.clone()),
                    applied_at: None,
                };
                self.set_coding_improvement_apply_result(proposal_id, "failed", &record)?;
                bail!(message);
            }
        };
        match apply_action_plan(&plan) {
            Ok(artifacts) => {
                let record = CodingImprovementActionRecord {
                    applied: true,
                    artifacts: artifacts.clone(),
                    error: None,
                    applied_at: Some(now_rfc3339()),
                };
                self.set_coding_improvement_apply_result(proposal_id, "applied", &record)?;
                let proposal = self
                    .get_coding_improvement_proposal(proposal_id)?
                    .ok_or_else(|| anyhow!("coding improvement proposal vanished after apply"))?;
                Ok(ApplyCodingImprovementProposalResult {
                    proposal,
                    plan,
                    applied: true,
                    artifacts,
                    error: None,
                })
            }
            Err(err) => {
                let message = err.to_string();
                let record = CodingImprovementActionRecord {
                    applied: false,
                    artifacts: Vec::new(),
                    error: Some(message.clone()),
                    applied_at: None,
                };
                self.set_coding_improvement_apply_result(proposal_id, "failed", &record)?;
                let proposal = self
                    .get_coding_improvement_proposal(proposal_id)?
                    .ok_or_else(|| anyhow!("coding improvement proposal vanished after failure"))?;
                Ok(ApplyCodingImprovementProposalResult {
                    proposal,
                    plan,
                    applied: false,
                    artifacts: Vec::new(),
                    error: Some(message),
                })
            }
        }
    }

    pub fn preview_coding_improvement_proposal_promotion(
        &self,
        proposal_id: &str,
    ) -> Result<CodingImprovementPromotionPlan> {
        let proposal = self
            .get_coding_improvement_proposal(proposal_id)?
            .ok_or_else(|| anyhow!("coding improvement proposal not found: {proposal_id}"))?;
        self.build_coding_improvement_promotion_plan(proposal)
    }

    pub fn promote_coding_improvement_proposal(
        &self,
        proposal_id: &str,
    ) -> Result<PromoteCodingImprovementProposalResult> {
        let proposal = self.claim_coding_improvement_proposal_promotion(proposal_id)?;
        let plan = match self.build_coding_improvement_promotion_plan(proposal.clone()) {
            Ok(plan) => plan,
            Err(err) => {
                let message = err.to_string();
                let record = CodingImprovementPromotionRecord {
                    promoted: false,
                    artifacts: Vec::new(),
                    error: Some(message.clone()),
                    promoted_at: None,
                };
                self.set_coding_improvement_promotion_result(
                    proposal_id,
                    "promotion_failed",
                    &record,
                )?;
                bail!(message);
            }
        };
        match apply_promotion_plan(&plan) {
            Ok(artifacts) => {
                let record = CodingImprovementPromotionRecord {
                    promoted: true,
                    artifacts: artifacts.clone(),
                    error: None,
                    promoted_at: Some(now_rfc3339()),
                };
                self.set_coding_improvement_promotion_result(proposal_id, "promoted", &record)?;
                let proposal = self
                    .get_coding_improvement_proposal(proposal_id)?
                    .ok_or_else(|| {
                        anyhow!("coding improvement proposal vanished after promotion")
                    })?;
                Ok(PromoteCodingImprovementProposalResult {
                    proposal,
                    plan,
                    promoted: true,
                    artifacts,
                    error: None,
                })
            }
            Err(err) => {
                let message = err.to_string();
                let record = CodingImprovementPromotionRecord {
                    promoted: false,
                    artifacts: Vec::new(),
                    error: Some(message.clone()),
                    promoted_at: None,
                };
                self.set_coding_improvement_promotion_result(
                    proposal_id,
                    "promotion_failed",
                    &record,
                )?;
                let proposal = self
                    .get_coding_improvement_proposal(proposal_id)?
                    .ok_or_else(|| {
                        anyhow!("coding improvement proposal vanished after promotion failure")
                    })?;
                Ok(PromoteCodingImprovementProposalResult {
                    proposal,
                    plan,
                    promoted: false,
                    artifacts: Vec::new(),
                    error: Some(message),
                })
            }
        }
    }

    pub fn record_coding_eval_run(
        &self,
        input: RecordCodingEvalRunInput,
    ) -> Result<CodingEvalRunRecord> {
        let status = normalize_eval_status(&input.status)?;
        let session_id = input
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(session_id) = session_id.as_deref() {
            let meta = self
                .get_session(session_id)?
                .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
            if meta.incognito {
                bail!("Cannot record coding eval run for incognito session {session_id}");
            }
        }
        let project_id = input
            .project_id
            .or_else(|| {
                session_id
                    .as_deref()
                    .and_then(|sid| self.get_session(sid).ok().flatten())
                    .and_then(|meta| meta.project_id)
            })
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
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

    fn resolve_coding_report_scope(
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

    fn build_coding_trend_report(&self, scope: &ReportScope) -> Result<CodingTrendReport> {
        let mut overview = CodingTrendOverview {
            sessions: scope.session_ids.len(),
            ..CodingTrendOverview::default()
        };
        let mut eval = CodingEvalTrend::default();
        let mut review = CodingReviewTrend::default();
        let mut verification = CodingVerificationTrend::default();
        let mut repair_loop = CodingRepairLoopTrend::default();
        let mut retro = CodingRetroTrend::default();
        let mut failures: BTreeMap<String, CodingFailureBucket> = BTreeMap::new();
        let mut recent_runs = Vec::new();
        let mut review_categories: BTreeMap<String, usize> = BTreeMap::new();
        let retros = self.list_coding_workflow_retros_for_scope(scope)?;
        retro.total = retros.len();
        retro.latest_summary = retros.first().map(|item| item.summary.clone());
        for item in &retros {
            retro.recommendations += item.recommendations.len();
            match item.run_state.as_str() {
                "completed" => retro.completed += 1,
                "blocked" => retro.blocked += 1,
                "failed" => retro.failed += 1,
                "cancelled" => retro.cancelled += 1,
                _ => {}
            }
        }

        let eval_runs = self.list_coding_eval_runs_for_scope(scope)?;
        eval.runs = eval_runs.len();
        eval.passed = eval_runs
            .iter()
            .filter(|run| run.status == "passed")
            .count();
        eval.failed = eval_runs
            .iter()
            .filter(|run| run.status == "failed")
            .count();
        eval.success_rate = ratio(eval.passed, eval.passed + eval.failed);
        for run in eval_runs.iter().filter(|run| run.status == "failed") {
            add_failure(
                &mut failures,
                "eval_failed",
                format!("{} / {}", run.suite, run.name),
                &run.id,
            );
        }

        for session_id in &scope.session_ids {
            let goals = self.list_goal_rows_for_session(session_id, &scope.since)?;
            overview.goals += goals.len();
            for goal in goals {
                match goal.state.as_str() {
                    "completed" => overview.completed_goals += 1,
                    "blocked" => {
                        overview.blocked_goals += 1;
                        add_failure(
                            &mut failures,
                            classify_blocked_reason(goal.blocked_reason.as_deref()),
                            goal.blocked_reason
                                .unwrap_or_else(|| "goal blocked".to_string()),
                            "goal",
                        );
                    }
                    "failed" => add_failure(&mut failures, "goal_failed", "goal failed", "goal"),
                    _ => {}
                }
            }

            for run in self.list_workflow_runs_for_session(session_id, 200)? {
                if run.updated_at < scope.since {
                    continue;
                }
                overview.workflow_runs += 1;
                let events = self.list_workflow_events(&run.id, 500).unwrap_or_default();
                let has_repair_loop = events
                    .iter()
                    .any(|event| event.event_type.starts_with("repair_loop_"))
                    || run.script_source.contains("repairLoop");
                if has_repair_loop {
                    repair_loop.runs += 1;
                }
                match run.state {
                    WorkflowRunState::Completed => {
                        overview.completed_workflows += 1;
                        if has_repair_loop {
                            repair_loop.completed += 1;
                        }
                    }
                    WorkflowRunState::Blocked => {
                        overview.blocked_workflows += 1;
                        if has_repair_loop {
                            repair_loop.blocked += 1;
                        }
                        if run.blocked_reason.as_deref() == Some("repair_loop_attempts_exhausted") {
                            repair_loop.exhausted += 1;
                        }
                        add_failure(
                            &mut failures,
                            classify_blocked_reason(run.blocked_reason.as_deref()),
                            run.blocked_reason
                                .clone()
                                .unwrap_or_else(|| "workflow blocked".to_string()),
                            &run.id,
                        );
                    }
                    WorkflowRunState::Failed => {
                        overview.failed_workflows += 1;
                        add_failure(&mut failures, "workflow_failed", "workflow failed", &run.id);
                    }
                    WorkflowRunState::AwaitingApproval => {
                        add_failure(
                            &mut failures,
                            "permission_stall",
                            "workflow awaiting approval",
                            &run.id,
                        );
                    }
                    _ => {}
                }
                if !matches!(run.state, WorkflowRunState::Draft) {
                    recent_runs.push(CodingRunSummary {
                        run_id: run.id.clone(),
                        session_id: run.session_id.clone(),
                        goal_id: run.goal_id.clone(),
                        kind: run.kind.clone(),
                        state: run.state.as_str().to_string(),
                        blocked_reason: run.blocked_reason.clone(),
                        failure_category: if matches!(
                            run.state,
                            WorkflowRunState::Blocked | WorkflowRunState::Failed
                        ) {
                            Some(classify_blocked_reason(run.blocked_reason.as_deref()).to_string())
                        } else {
                            None
                        },
                        updated_at: run.updated_at.clone(),
                    });
                }
            }

            for review_run in self.list_review_runs_for_session(session_id, 200)? {
                if review_run.updated_at < scope.since {
                    continue;
                }
                review.runs += 1;
                let findings = self
                    .list_review_findings_for_run(&review_run.id)
                    .unwrap_or_default();
                review.findings += findings.len();
                for finding in findings {
                    *review_categories
                        .entry(finding.category.clone())
                        .or_default() += 1;
                    if is_blocking_review_finding(&finding.severity, &finding.status) {
                        review.blocking_findings += 1;
                        add_failure(
                            &mut failures,
                            "review_blocker",
                            finding.title.clone(),
                            &finding.id,
                        );
                    }
                    if finding.status == ReviewFindingStatus::Resolved {
                        review.resolved_findings += 1;
                    }
                    if finding.status == ReviewFindingStatus::FalsePositive {
                        review.false_positive_findings += 1;
                    }
                }
            }

            for verification_run in self.list_verification_runs_for_session(session_id, 200)? {
                if verification_run.updated_at < scope.since {
                    continue;
                }
                verification.runs += 1;
                let steps = self
                    .list_verification_steps_for_run(&verification_run.id)
                    .unwrap_or_default();
                if matches!(verification_run.state.as_str(), "planned") {
                    verification.planned_only_runs += 1;
                }
                if steps.is_empty() {
                    add_failure(
                        &mut failures,
                        "verification_selection_gap",
                        "verification plan selected no command",
                        &verification_run.id,
                    );
                }
                verification.steps += steps.len();
                for step in steps {
                    match step.state {
                        VerificationStepState::Passed => verification.passed_steps += 1,
                        VerificationStepState::Failed => {
                            verification.failed_steps += 1;
                            add_failure(
                                &mut failures,
                                "validation_failed",
                                step.title.clone(),
                                &step.id,
                            );
                        }
                        VerificationStepState::TimedOut => {
                            verification.timed_out_steps += 1;
                            add_failure(
                                &mut failures,
                                "validation_failed",
                                format!("{} timed out", step.title),
                                &step.id,
                            );
                        }
                        _ => {}
                    }
                }
            }
        }

        overview.goal_completion_rate = ratio(
            overview.completed_goals,
            overview.completed_goals + overview.blocked_goals,
        );
        overview.workflow_completion_rate = ratio(
            overview.completed_workflows,
            overview.completed_workflows + overview.blocked_workflows + overview.failed_workflows,
        );
        repair_loop.success_rate = ratio(
            repair_loop.completed,
            repair_loop.completed + repair_loop.blocked,
        );
        let executed =
            verification.passed_steps + verification.failed_steps + verification.timed_out_steps;
        verification.executed_success_rate = ratio(verification.passed_steps, executed);
        verification.recommendation_coverage = ratio(
            verification
                .runs
                .saturating_sub(count_zero_step_verification_runs(self, scope)?),
            verification.runs,
        );
        review.by_category = review_categories
            .into_iter()
            .map(|(key, count)| CodingMetricBucket {
                label: failure_label(&key).unwrap_or(&key).to_string(),
                key,
                count,
            })
            .collect();
        eval.backlog_candidates = self.count_eval_candidate_proposals_for_scope(scope)?;
        let mut failures = failures.into_values().collect::<Vec<_>>();
        failures.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.category.cmp(&b.category))
        });
        recent_runs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        recent_runs.truncate(12);

        Ok(CodingTrendReport {
            session_id: scope.session_id.clone(),
            project_id: scope.project_id.clone(),
            scope: if scope.project_id.is_some() {
                "project".to_string()
            } else {
                "session".to_string()
            },
            window_days: scope.window_days,
            generated_at: now_rfc3339(),
            overview,
            eval,
            review,
            verification,
            repair_loop,
            retro,
            failures,
            recent_runs,
            retros,
            proposals: Vec::new(),
        })
    }

    fn list_goal_rows_for_session(&self, session_id: &str, since: &str) -> Result<Vec<GoalRow>> {
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

    fn list_coding_eval_runs_for_scope(
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

    fn list_coding_workflow_retros_for_scope(
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

    fn get_coding_workflow_retro_for_run(
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

    fn upsert_coding_workflow_retro(&self, retro: CodingWorkflowRetro) -> Result<()> {
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

    fn get_coding_improvement_proposal(
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

    fn list_coding_improvement_proposals_for_scope(
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

    fn count_eval_candidate_proposals_for_scope(&self, scope: &ReportScope) -> Result<usize> {
        let proposals = self.list_coding_improvement_proposals_for_scope(scope)?;
        Ok(proposals
            .iter()
            .filter(|proposal| proposal.kind == "eval_candidate")
            .count())
    }

    fn insert_coding_improvement_proposal(
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

    fn build_coding_improvement_action_plan(
        &self,
        proposal: CodingImprovementProposal,
    ) -> Result<CodingImprovementActionPlan> {
        let meta = self
            .get_session(&proposal.session_id)?
            .ok_or_else(|| anyhow!("session not found: {}", proposal.session_id))?;
        if meta.incognito {
            bail!(
                "Cannot apply coding improvement proposal for incognito session {}",
                proposal.session_id
            );
        }
        let base_dir = crate::session::effective_working_dir_for_meta(&meta)
            .map(PathBuf::from)
            .unwrap_or(crate::paths::session_dir(&proposal.session_id)?)
            .join(".hope-agent")
            .join("coding-improvement");
        build_action_plan_for_proposal(proposal, &base_dir)
    }

    fn build_coding_improvement_promotion_plan(
        &self,
        proposal: CodingImprovementProposal,
    ) -> Result<CodingImprovementPromotionPlan> {
        let meta = self
            .get_session(&proposal.session_id)?
            .ok_or_else(|| anyhow!("session not found: {}", proposal.session_id))?;
        if meta.incognito {
            bail!(
                "Cannot promote coding improvement proposal for incognito session {}",
                proposal.session_id
            );
        }
        let workspace_root =
            crate::session::effective_working_dir_for_meta(&meta).map(PathBuf::from);
        build_promotion_plan_for_proposal(proposal, workspace_root.as_deref())
    }

    fn claim_coding_improvement_proposal_apply(
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

    fn claim_coding_improvement_proposal_promotion(
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

    fn set_coding_improvement_apply_result(
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

    fn set_coding_improvement_promotion_result(
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
struct GoalRow {
    #[allow(dead_code)]
    id: String,
    state: String,
    blocked_reason: Option<String>,
    #[allow(dead_code)]
    updated_at: String,
}

struct NewProposal {
    kind: String,
    source_type: String,
    source_id: String,
    title: String,
    body: String,
    payload: Value,
    fingerprint: String,
}

fn build_proposal_candidates(report: &CodingTrendReport) -> Vec<NewProposal> {
    let mut out = Vec::new();
    for retro in report.retros.iter().take(5) {
        for recommendation in retro.recommendations.iter().take(2) {
            let kind = match recommendation.kind.as_str() {
                "eval_candidate" => "eval_candidate",
                "workflow_template" => "workflow_template",
                "skill_candidate" => "skill_candidate",
                _ => "guidance_candidate",
            };
            out.push(NewProposal {
                kind: kind.to_string(),
                source_type: "workflow_retro".to_string(),
                source_id: retro.id.clone(),
                title: recommendation.title.clone(),
                body: recommendation.rationale.clone(),
                payload: json!({
                    "proposalType": kind,
                    "retro": retro,
                    "recommendation": recommendation,
                    "scope": report.scope,
                    "projectId": report.project_id,
                }),
                fingerprint: format!(
                    "retro:{}:{}:{}",
                    report.scope_key(),
                    retro.workflow_run_id,
                    recommendation.kind
                ),
            });
        }
    }
    for failure in report.failures.iter().take(3) {
        out.push(NewProposal {
            kind: "eval_candidate".to_string(),
            source_type: "failure_taxonomy".to_string(),
            source_id: failure.category.clone(),
            title: format!("Add eval coverage for {}", failure.label),
            body: format!(
                "{} occurrence(s) in the last {} days. Convert one representative failure into a deterministic eval candidate before changing policy.",
                failure.count, report.window_days
            ),
            payload: json!({
                "proposalType": "eval_candidate",
                "failure": failure,
                "scope": report.scope,
                "projectId": report.project_id,
                "expectedSignals": expected_signals_for_failure(&failure.category),
            }),
            fingerprint: format!("eval:{}:{}", report.scope_key(), failure.category),
        });
    }

    if report.repair_loop.completed > 0 {
        out.push(NewProposal {
            kind: "workflow_template".to_string(),
            source_type: "repair_loop".to_string(),
            source_id: "completed".to_string(),
            title: "Promote successful repair loop shape".to_string(),
            body: "Recent repair loop runs completed successfully. Review whether the validation/review profile mix should become a reusable workflow draft.".to_string(),
            payload: json!({
                "proposalType": "workflow_template",
                "repairLoop": report.repair_loop,
                "recentRuns": report.recent_runs.iter().take(5).collect::<Vec<_>>(),
            }),
            fingerprint: format!("workflow-template:{}:repair-loop", report.scope_key()),
        });
    }

    if report.review.blocking_findings > 0 {
        out.push(NewProposal {
            kind: "guidance_candidate".to_string(),
            source_type: "review".to_string(),
            source_id: "blocking_findings".to_string(),
            title: "Review blocker pattern needs project guidance".to_string(),
            body: "Open P0/P1 review findings are recurring in this scope. Draft project guidance or workflow checkpoints before making this automatic.".to_string(),
            payload: json!({
                "proposalType": "guidance_candidate",
                "review": report.review,
            }),
            fingerprint: format!("guidance:{}:review-blockers", report.scope_key()),
        });
    }

    if report.verification.failed_steps + report.verification.timed_out_steps > 0 {
        out.push(NewProposal {
            kind: "guidance_candidate".to_string(),
            source_type: "verification".to_string(),
            source_id: "failed_steps".to_string(),
            title: "Verification failures need a tighter playbook".to_string(),
            body: "Recent validation failures or timeouts suggest the project may need more specific targeted verification guidance.".to_string(),
            payload: json!({
                "proposalType": "guidance_candidate",
                "verification": report.verification,
            }),
            fingerprint: format!("guidance:{}:verification-failures", report.scope_key()),
        });
    }

    if report.overview.completed_workflows > 0 && report.failures.is_empty() {
        out.push(NewProposal {
            kind: "skill_candidate".to_string(),
            source_type: "workflow".to_string(),
            source_id: "clean_success".to_string(),
            title: "Distil a clean coding workflow skill draft".to_string(),
            body: "Recent coding workflows completed without classified blockers. Review one transcript manually before promoting a reusable skill.".to_string(),
            payload: json!({
                "proposalType": "skill_candidate",
                "overview": report.overview,
                "recentRuns": report.recent_runs.iter().take(5).collect::<Vec<_>>(),
            }),
            fingerprint: format!("skill:{}:clean-workflow", report.scope_key()),
        });
    }
    out
}

fn build_action_plan_for_proposal(
    proposal: CodingImprovementProposal,
    base_dir: &Path,
) -> Result<CodingImprovementActionPlan> {
    match proposal.kind.as_str() {
        "eval_candidate" => build_eval_candidate_action_plan(proposal, base_dir),
        "workflow_template" => build_workflow_template_action_plan(proposal, base_dir),
        "guidance_candidate" => build_guidance_candidate_action_plan(proposal, base_dir),
        "skill_candidate" => build_skill_candidate_action_plan(proposal),
        other => bail!("unsupported coding improvement proposal kind: {other}"),
    }
}

fn build_eval_candidate_action_plan(
    proposal: CodingImprovementProposal,
    base_dir: &Path,
) -> Result<CodingImprovementActionPlan> {
    let failure = proposal.payload.get("failure").cloned().unwrap_or_else(|| {
        json!({
            "category": proposal.source_id,
            "label": proposal.title,
        })
    });
    let category = failure
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or(&proposal.source_id);
    let slug = proposal_slug(&proposal);
    let target = base_dir
        .join("eval-candidates")
        .join(format!("{slug}.json"));
    let fixture = json!({
        "name": slug,
        "description": format!("Draft eval candidate generated from coding improvement proposal {}.", proposal.id),
        "source": {
            "kind": "coding_improvement_proposal",
            "proposalId": proposal.id,
            "proposalTitle": proposal.title,
            "failureCategory": category,
        },
        "repo": {
            "files": [],
            "changes": []
        },
        "setup": {
            "goal": {
                "objective": format!("Reproduce {}", failure_label(category).unwrap_or(category)),
                "completionCriteria": "The fixture should fail before the product fix and pass after the fix."
            }
        },
        "runs": {
            "improvement": {
                "generateProposals": true,
                "seedEvalRuns": [
                    {
                        "suite": "coding_control_plane",
                        "name": slug,
                        "status": "failed",
                        "metrics": {
                            "sourceProposalId": proposal.id,
                            "failureCategory": category,
                        },
                        "sourceType": "coding_improvement_proposal",
                        "sourceId": proposal.id
                    }
                ]
            }
        },
        "checks": {
            "improvement": {
                "expectedFailureCategories": [category],
                "expectedProposalKinds": ["eval_candidate"],
                "minFailures": 1,
                "minProposals": 1
            }
        },
        "nextSteps": [
            "Fill repo.files and repo.changes with the smallest deterministic reproduction.",
            "Move this draft into crates/ha-core/tests/fixtures/coding_eval/ when it is review-ready."
        ]
    });
    let content = format!("{}\n", serde_json::to_string_pretty(&fixture)?);
    Ok(single_file_plan(
        proposal,
        "eval_candidate",
        "Create a deterministic eval fixture draft from this failure bucket.",
        "Create eval fixture draft",
        target,
        content,
        json!({ "fixture": fixture }),
    ))
}

fn build_workflow_template_action_plan(
    proposal: CodingImprovementProposal,
    base_dir: &Path,
) -> Result<CodingImprovementActionPlan> {
    let slug = proposal_slug(&proposal);
    let target = base_dir.join("workflows").join(format!("{slug}.md"));
    let content = format!(
        "# {}\n\nSource proposal: `{}`\n\n## Why This Exists\n\n{}\n\n## Draft Workflow Shape\n\n```js\nexport default async function main(workflow) {{\n  const task = await workflow.task.create({{ title: \"Review and verify focused change\" }});\n  const review = await workflow.review({{ label: \"focused-review\", profiles: [\"correctness\", \"tests\"] }});\n  const verification = await workflow.verify({{ label: \"targeted-verification\", maxCommands: 2 }});\n  await workflow.task.update({{ task, status: \"completed\" }});\n  await workflow.finish({{ summary: \"Review and verification completed\", review, verification }});\n}}\n```\n\n## Promotion Checklist\n\n- Confirm this shape matches at least one successful run.\n- Replace placeholder profiles and command limits with project-specific choices.\n- Add a coding eval fixture before promoting it to a reusable workflow.\n",
        proposal.title, proposal.id, proposal.body
    );
    Ok(single_file_plan(
        proposal,
        "workflow_template",
        "Create a reviewable workflow template draft.",
        "Create workflow template draft",
        target,
        content,
        json!({ "format": "markdown_workflow_template" }),
    ))
}

fn build_guidance_candidate_action_plan(
    proposal: CodingImprovementProposal,
    base_dir: &Path,
) -> Result<CodingImprovementActionPlan> {
    let slug = proposal_slug(&proposal);
    let target = base_dir.join("guidance").join(format!("{slug}.md"));
    let content = format!(
        "# {}\n\nSource proposal: `{}`\n\n## Signal\n\n{}\n\n## Draft Guidance\n\n- Before changing policy, identify the smallest reproducible example behind this signal.\n- Prefer focused review and targeted verification over broad validation suites.\n- Keep project guidance concrete: name the risky pattern, the preferred check, and the evidence needed before completion.\n\n## Evidence Payload\n\n```json\n{}\n```\n",
        proposal.title,
        proposal.id,
        proposal.body,
        serde_json::to_string_pretty(&proposal.payload)?
    );
    Ok(single_file_plan(
        proposal,
        "guidance_candidate",
        "Create a project guidance draft for manual review.",
        "Create guidance draft",
        target,
        content,
        json!({ "format": "markdown_guidance" }),
    ))
}

fn build_skill_candidate_action_plan(
    proposal: CodingImprovementProposal,
) -> Result<CodingImprovementActionPlan> {
    let slug = proposal_slug(&proposal);
    let skill_id = format!("ha-learned-{slug}-{}", short_id(&proposal.id));
    let target = crate::paths::skills_dir()?.join(&skill_id).join("SKILL.md");
    let description = format!(
        "Apply the learned workflow pattern from coding improvement proposal {}.",
        proposal.id
    );
    let body = format!(
        "---\nname: {skill_id}\ndescription: {description}\nstatus: draft\nmetadata:\n  source: coding_improvement\n  proposal_id: {}\n---\n\n# {}\n\nUse this skill when a future task matches the same successful pattern captured by the source proposal.\n\n## Operating Guidance\n\n1. Read the current task, repository rules, and relevant control-plane evidence first.\n2. Prefer focused review, targeted verification, and explicit evidence over broad checks.\n3. If the pattern does not clearly match, do not activate this skill.\n\n## Source Signal\n\n{}\n\n## Review Notes\n\n- This is a draft generated by the Coding Improvement Loop.\n- Review the original transcript or run evidence before activating it.\n- Keep the final skill short and tool-aware.\n",
        proposal.id, proposal.title, proposal.body
    );
    Ok(CodingImprovementActionPlan {
        proposal,
        target_kind: "skill_candidate".to_string(),
        summary: "Create a managed draft skill for review in the Skills panel.".to_string(),
        requires_confirmation: true,
        steps: vec![CodingImprovementActionStep {
            action: "create_managed_skill_draft".to_string(),
            label: "Create managed skill draft".to_string(),
            target_path: target.to_string_lossy().to_string(),
            target_exists: target.exists(),
            content_preview: Some(truncate_preview(&body)),
            content: Some(body),
        }],
        preview: json!({
            "skillId": skill_id,
            "description": description,
        }),
    })
}

fn build_promotion_plan_for_proposal(
    proposal: CodingImprovementProposal,
    workspace_root: Option<&Path>,
) -> Result<CodingImprovementPromotionPlan> {
    match proposal.kind.as_str() {
        "eval_candidate" => build_file_promotion_plan(
            proposal,
            workspace_root,
            "eval_candidate",
            "Promote eval candidate into the coding eval fixture suite.",
            "Promote eval fixture",
            |root, source| {
                Ok(root
                    .join("crates/ha-core/tests/fixtures/coding_eval")
                    .join(source_file_name(source)?))
            },
            None,
        ),
        "workflow_template" => build_file_promotion_plan(
            proposal,
            workspace_root,
            "workflow_template",
            "Promote workflow template into project guidance and link it from AGENTS.md.",
            "Promote workflow template",
            |root, source| {
                Ok(root
                    .join(".hope-agent/coding-improvement/promoted/workflows")
                    .join(source_file_name(source)?))
            },
            Some("Reusable workflow template"),
        ),
        "guidance_candidate" => build_file_promotion_plan(
            proposal,
            workspace_root,
            "guidance_candidate",
            "Promote guidance into project rules and link it from AGENTS.md.",
            "Promote project guidance",
            |root, source| {
                Ok(root
                    .join(".hope-agent/coding-improvement/promoted/guidance")
                    .join(source_file_name(source)?))
            },
            Some("Coding guidance"),
        ),
        "skill_candidate" => build_skill_promotion_plan(proposal),
        other => bail!("unsupported coding improvement proposal kind: {other}"),
    }
}

fn build_file_promotion_plan(
    proposal: CodingImprovementProposal,
    workspace_root: Option<&Path>,
    target_kind: &str,
    summary: &str,
    label: &str,
    target_path: impl FnOnce(&Path, &Path) -> Result<PathBuf>,
    agents_include_label: Option<&str>,
) -> Result<CodingImprovementPromotionPlan> {
    ensure_proposal_promotable(&proposal)?;
    let root = workspace_root.ok_or_else(|| {
        anyhow!(
            "promotion for {} requires a session or project working directory",
            proposal.kind
        )
    })?;
    let source = primary_action_artifact_path(&proposal)?;
    let content = std::fs::read_to_string(&source).map_err(|err| {
        anyhow!(
            "failed to read draft artifact {}: {}",
            source.display(),
            err
        )
    })?;
    let target = target_path(root, &source)?;
    let mut steps = vec![CodingImprovementPromotionStep {
        action: "create_promoted_file".to_string(),
        label: label.to_string(),
        source_path: Some(source.to_string_lossy().to_string()),
        target_path: target.to_string_lossy().to_string(),
        target_exists: target.exists(),
        source_hash: Some(short_hash(&content)),
        content_preview: Some(truncate_preview(&content)),
        content: Some(content),
    }];

    if let Some(include_label) = agents_include_label {
        let agents_path = root.join("AGENTS.md");
        let relative = target
            .strip_prefix(root)
            .unwrap_or(target.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        let include_line = format!("- {include_label}: @./{relative}");
        let current = std::fs::read_to_string(&agents_path).unwrap_or_default();
        let updated = append_agents_managed_include(&current, &include_line);
        if updated != current {
            steps.push(CodingImprovementPromotionStep {
                action: "update_agents_include".to_string(),
                label: "Link from AGENTS.md".to_string(),
                source_path: None,
                target_path: agents_path.to_string_lossy().to_string(),
                target_exists: agents_path.exists(),
                source_hash: Some(short_hash(&current)),
                content_preview: Some(truncate_preview(&updated)),
                content: Some(updated),
            });
        }
    }

    Ok(CodingImprovementPromotionPlan {
        proposal,
        target_kind: target_kind.to_string(),
        summary: summary.to_string(),
        requires_confirmation: true,
        steps,
        preview: json!({
            "workspaceRoot": root.to_string_lossy(),
            "promotionKind": target_kind,
        }),
    })
}

fn build_skill_promotion_plan(
    proposal: CodingImprovementProposal,
) -> Result<CodingImprovementPromotionPlan> {
    ensure_proposal_promotable(&proposal)?;
    let source = primary_action_artifact_path(&proposal)?;
    let skill_id = source
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("cannot infer skill id from {}", source.display()))?
        .to_string();
    let content = std::fs::read_to_string(&source)
        .map_err(|err| anyhow!("failed to read draft skill {}: {}", source.display(), err))?;
    Ok(CodingImprovementPromotionPlan {
        proposal,
        target_kind: "skill_candidate".to_string(),
        summary: "Activate the managed draft skill so it becomes available to the skill catalog."
            .to_string(),
        requires_confirmation: true,
        steps: vec![CodingImprovementPromotionStep {
            action: "activate_managed_skill".to_string(),
            label: "Activate managed skill".to_string(),
            source_path: Some(source.to_string_lossy().to_string()),
            target_path: source.to_string_lossy().to_string(),
            target_exists: source.exists(),
            source_hash: Some(short_hash(&content)),
            content_preview: Some(truncate_preview(&content)),
            content: Some(skill_id.clone()),
        }],
        preview: json!({ "skillId": skill_id }),
    })
}

fn single_file_plan(
    proposal: CodingImprovementProposal,
    target_kind: &str,
    summary: &str,
    label: &str,
    target: PathBuf,
    content: String,
    preview: Value,
) -> CodingImprovementActionPlan {
    CodingImprovementActionPlan {
        proposal,
        target_kind: target_kind.to_string(),
        summary: summary.to_string(),
        requires_confirmation: true,
        steps: vec![CodingImprovementActionStep {
            action: "create_file".to_string(),
            label: label.to_string(),
            target_path: target.to_string_lossy().to_string(),
            target_exists: target.exists(),
            content_preview: Some(truncate_preview(&content)),
            content: Some(content),
        }],
        preview,
    }
}

fn apply_action_plan(
    plan: &CodingImprovementActionPlan,
) -> Result<Vec<CodingImprovementActionArtifact>> {
    match plan.target_kind.as_str() {
        "skill_candidate" => apply_skill_candidate_plan(plan),
        _ => apply_file_plan(plan),
    }
}

fn apply_file_plan(
    plan: &CodingImprovementActionPlan,
) -> Result<Vec<CodingImprovementActionArtifact>> {
    let mut artifacts = Vec::new();
    for step in &plan.steps {
        if step.action != "create_file" {
            bail!(
                "unsupported coding improvement file action: {}",
                step.action
            );
        }
        let Some(content) = step.content.as_deref().or(step.content_preview.as_deref()) else {
            bail!("missing content for {}", step.target_path);
        };
        if step.content.is_none() && content.ends_with("[truncated]") {
            bail!(
                "refusing to apply truncated coding improvement preview for {}",
                step.target_path
            );
        }
        let path = PathBuf::from(&step.target_path);
        if path.exists() {
            bail!("target already exists: {}", path.display());
        }
        write_new_file_no_clobber(&path, content)?;
        artifacts.push(CodingImprovementActionArtifact {
            kind: step.action.clone(),
            path: path.to_string_lossy().to_string(),
            content_hash: Some(short_hash(content)),
        });
    }
    Ok(artifacts)
}

fn apply_skill_candidate_plan(
    plan: &CodingImprovementActionPlan,
) -> Result<Vec<CodingImprovementActionArtifact>> {
    let skill_id = plan
        .preview
        .get("skillId")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("skill candidate preview is missing skillId"))?;
    let description = plan
        .preview
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("Draft skill generated from a coding improvement proposal");
    let step = plan
        .steps
        .first()
        .ok_or_else(|| anyhow!("skill candidate plan has no steps"))?;
    let body = step
        .content
        .as_deref()
        .or(step.content_preview.as_deref())
        .ok_or_else(|| anyhow!("skill candidate plan is missing SKILL.md content"))?;
    if step.content.is_none() && body.ends_with("[truncated]") {
        bail!(
            "refusing to apply truncated coding improvement preview for {}",
            step.target_path
        );
    }
    let path = crate::skills::author::create_skill(
        skill_id,
        description,
        body,
        crate::skills::author::CreateOpts {
            status: SkillStatus::Draft,
            authored_by: "coding-improvement".to_string(),
            rationale: Some(plan.proposal.title.clone()),
            fail_if_exists: true,
        },
    )?;
    Ok(vec![CodingImprovementActionArtifact {
        kind: "create_managed_skill_draft".to_string(),
        path: path.to_string_lossy().to_string(),
        content_hash: Some(short_hash(body)),
    }])
}

fn apply_promotion_plan(
    plan: &CodingImprovementPromotionPlan,
) -> Result<Vec<CodingImprovementActionArtifact>> {
    let mut artifacts = Vec::new();
    for step in &plan.steps {
        match step.action.as_str() {
            "create_promoted_file" => {
                let Some(content) = step.content.as_deref().or(step.content_preview.as_deref())
                else {
                    bail!("missing promotion content for {}", step.target_path);
                };
                if step.content.is_none() && content.ends_with("[truncated]") {
                    bail!(
                        "refusing to promote truncated preview for {}",
                        step.target_path
                    );
                }
                let path = PathBuf::from(&step.target_path);
                if path.exists() {
                    let existing = std::fs::read_to_string(&path).unwrap_or_default();
                    if existing != content {
                        bail!("promotion target already exists: {}", path.display());
                    }
                    artifacts.push(CodingImprovementActionArtifact {
                        kind: "existing_promoted_file".to_string(),
                        path: path.to_string_lossy().to_string(),
                        content_hash: Some(short_hash(content)),
                    });
                    continue;
                }
                write_new_file_no_clobber(&path, content)?;
                artifacts.push(CodingImprovementActionArtifact {
                    kind: step.action.clone(),
                    path: path.to_string_lossy().to_string(),
                    content_hash: Some(short_hash(content)),
                });
            }
            "update_agents_include" => {
                let content = step
                    .content
                    .as_deref()
                    .ok_or_else(|| anyhow!("missing AGENTS.md promotion content"))?;
                let path = PathBuf::from(&step.target_path);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                crate::platform::write_atomic(&path, content.as_bytes())?;
                artifacts.push(CodingImprovementActionArtifact {
                    kind: step.action.clone(),
                    path: path.to_string_lossy().to_string(),
                    content_hash: Some(short_hash(content)),
                });
            }
            "activate_managed_skill" => {
                let skill_id = step
                    .content
                    .as_deref()
                    .ok_or_else(|| anyhow!("missing managed skill id"))?;
                crate::skills::author::set_skill_status(skill_id, SkillStatus::Active)?;
                artifacts.push(CodingImprovementActionArtifact {
                    kind: step.action.clone(),
                    path: step.target_path.clone(),
                    content_hash: step.source_hash.clone(),
                });
            }
            other => bail!("unsupported coding improvement promotion action: {other}"),
        }
    }
    Ok(artifacts)
}

fn ensure_proposal_promotable(proposal: &CodingImprovementProposal) -> Result<()> {
    match proposal.status.as_str() {
        "applied" | "promotion_failed" | "promoting" | "promoted" => {}
        other => bail!(
            "coding improvement proposal {} is not applied and cannot be promoted (status: {other})",
            proposal.id
        ),
    }
    let action = proposal
        .action
        .as_ref()
        .ok_or_else(|| anyhow!("proposal {} has no applied action record", proposal.id))?;
    if !action.applied || action.artifacts.is_empty() {
        bail!("proposal {} has no successful draft artifact", proposal.id);
    }
    Ok(())
}

fn primary_action_artifact_path(proposal: &CodingImprovementProposal) -> Result<PathBuf> {
    let action = proposal
        .action
        .as_ref()
        .ok_or_else(|| anyhow!("proposal {} has no action record", proposal.id))?;
    let artifact = action
        .artifacts
        .first()
        .ok_or_else(|| anyhow!("proposal {} has no action artifacts", proposal.id))?;
    Ok(PathBuf::from(&artifact.path))
}

fn source_file_name(source: &Path) -> Result<&std::ffi::OsStr> {
    source
        .file_name()
        .ok_or_else(|| anyhow!("draft artifact has no file name: {}", source.display()))
}

fn append_agents_managed_include(current: &str, include_line: &str) -> String {
    if current.lines().any(|line| line.trim() == include_line) {
        return current.to_string();
    }
    const START: &str = "<!-- hope-agent-coding-improvement:start -->";
    const END: &str = "<!-- hope-agent-coding-improvement:end -->";
    if let (Some(_start), Some(end)) = (current.find(START), current.find(END)) {
        let mut out = String::with_capacity(current.len() + include_line.len() + 2);
        out.push_str(&current[..end]);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(include_line);
        out.push('\n');
        out.push_str(&current[end..]);
        return out;
    }
    let mut out = current.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(START);
    out.push('\n');
    out.push_str("# Hope Agent Coding Improvements\n\n");
    out.push_str(include_line);
    out.push('\n');
    out.push_str(END);
    out.push('\n');
    out
}

fn write_new_file_no_clobber(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::AlreadyExists {
                anyhow!("target already exists: {}", path.display())
            } else {
                anyhow!("failed to create {}: {}", path.display(), err)
            }
        })?;
    file.write_all(content.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn proposal_slug(proposal: &CodingImprovementProposal) -> String {
    let source = format!(
        "{}-{}-{}",
        proposal.kind, proposal.source_id, proposal.title
    );
    let mut slug = sanitize_slug(&source);
    if slug.len() > 64 {
        slug.truncate(64);
        slug = slug.trim_matches('-').to_string();
    }
    if slug.is_empty() {
        slug = "coding-improvement".to_string();
    }
    format!("{slug}-{}", short_id(&proposal.id))
}

fn sanitize_slug(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn short_id(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect::<String>()
}

fn truncate_preview(content: &str) -> String {
    if content.len() <= MAX_CONTENT_PREVIEW_BYTES {
        return content.to_string();
    }
    let mut end = MAX_CONTENT_PREVIEW_BYTES;
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n\n[truncated]", &content[..end])
}

fn short_hash(content: &str) -> String {
    let mut hash = blake3::hash(content.as_bytes()).to_hex().to_string();
    hash.truncate(16);
    hash
}

trait ReportScopeKey {
    fn scope_key(&self) -> String;
}

impl ReportScopeKey for CodingTrendReport {
    fn scope_key(&self) -> String {
        self.project_id
            .clone()
            .unwrap_or_else(|| self.session_id.clone())
    }
}

fn expected_signals_for_failure(category: &str) -> Vec<&'static str> {
    match category {
        "validation_failed" => vec!["verification_step", "validation_failed", "command_output"],
        "eval_failed" => vec!["coding_eval_run", "fixture_name", "failure_metrics"],
        "review_blocker" => vec!["review_finding", "blocking_severity", "file_path"],
        "repair_loop_exhausted" => vec!["workflow_blocked", "repair_loop_attempts_exhausted"],
        "no_effective_diff_progress" => vec!["workflow_blocked", "diff_snapshot"],
        "permission_stall" => vec!["approval", "workflow_state"],
        "context_miss" => vec!["context_candidate", "critical_context_recall"],
        _ => vec!["workflow_run", "goal_evidence"],
    }
}

fn add_failure(
    failures: &mut BTreeMap<String, CodingFailureBucket>,
    category: &str,
    example: impl Into<String>,
    source_id: &str,
) {
    let bucket = failures
        .entry(category.to_string())
        .or_insert_with(|| CodingFailureBucket {
            category: category.to_string(),
            label: failure_label(category).unwrap_or(category).to_string(),
            count: 0,
            severity: failure_severity(category).to_string(),
            examples: Vec::new(),
        });
    bucket.count += 1;
    if bucket.examples.len() < 3 {
        let example = example.into();
        bucket.examples.push(if source_id.is_empty() {
            example
        } else {
            format!("{source_id}: {example}")
        });
    }
}

fn classify_blocked_reason(reason: Option<&str>) -> &'static str {
    let Some(reason) = reason.map(str::to_ascii_lowercase) else {
        return "workflow_blocked";
    };
    if reason.contains("repair_loop_attempts_exhausted") {
        "repair_loop_exhausted"
    } else if reason.contains("no_effective_diff") || reason.contains("no_valid_diff") {
        "no_effective_diff_progress"
    } else if reason.contains("approval") || reason.contains("permission") {
        "permission_stall"
    } else if reason.contains("context") || reason.contains("recall") || reason.contains("missing")
    {
        "context_miss"
    } else if reason.contains("validation") || reason.contains("verify") {
        "validation_failed"
    } else {
        "workflow_blocked"
    }
}

fn failure_label(category: &str) -> Option<&'static str> {
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

fn failure_severity(category: &str) -> &'static str {
    match category {
        "validation_failed"
        | "eval_failed"
        | "review_blocker"
        | "repair_loop_exhausted"
        | "permission_stall" => "high",
        "no_effective_diff_progress" | "context_miss" | "workflow_failed" => "medium",
        _ => "low",
    }
}

fn is_blocking_review_finding(severity: &ReviewSeverity, status: &ReviewFindingStatus) -> bool {
    matches!(severity, ReviewSeverity::P0 | ReviewSeverity::P1)
        && matches!(status, ReviewFindingStatus::Open)
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

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    if denominator == 0 {
        None
    } else {
        Some((numerator as f64 / denominator as f64 * 1000.0).round() / 1000.0)
    }
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

fn count_zero_step_verification_runs(db: &SessionDB, scope: &ReportScope) -> Result<usize> {
    let mut count = 0usize;
    for session_id in &scope.session_ids {
        for run in db.list_verification_runs_for_session(session_id, 200)? {
            if run.updated_at >= scope.since
                && db
                    .list_verification_steps_for_run(&run.id)
                    .unwrap_or_default()
                    .is_empty()
            {
                count += 1;
            }
        }
    }
    Ok(count)
}

fn build_workflow_retro(
    run: &WorkflowRun,
    project_id: Option<String>,
    ops: &[WorkflowOp],
) -> CodingWorkflowRetro {
    let failed_ops = ops
        .iter()
        .filter(|op| op.state.as_str() == "failed")
        .count();
    let completed_ops = ops
        .iter()
        .filter(|op| op.state.as_str() == "completed")
        .count();
    let has_review = ops.iter().any(|op| op.op_type == "review");
    let has_verify = ops
        .iter()
        .any(|op| op.op_type == "verify" || op.op_type == "validate");
    let has_diff = ops.iter().any(|op| op.op_type == "diff");
    let validation_failed = ops
        .iter()
        .any(|op| op.op_type == "validate" && op.state.as_str() == "failed")
        || ops.iter().any(|op| {
            op.op_type == "validate"
                && op
                    .output
                    .as_ref()
                    .and_then(|value| value.get("ok"))
                    .and_then(Value::as_bool)
                    == Some(false)
        });
    let mut signals = vec![CodingRetroSignal {
        kind: "workflow_terminal".to_string(),
        label: format!("Workflow ended as {}", run.state.as_str()),
        severity: if run.state == WorkflowRunState::Completed {
            "info"
        } else {
            "warn"
        }
        .to_string(),
        detail: run.blocked_reason.clone(),
    }];
    if failed_ops > 0 {
        signals.push(CodingRetroSignal {
            kind: "failed_ops".to_string(),
            label: format!("{failed_ops} workflow op(s) failed"),
            severity: "high".to_string(),
            detail: None,
        });
    }
    if validation_failed {
        signals.push(CodingRetroSignal {
            kind: "validation_failed".to_string(),
            label: "Validation failed inside workflow".to_string(),
            severity: "high".to_string(),
            detail: None,
        });
    }
    if has_review {
        signals.push(CodingRetroSignal {
            kind: "review_used".to_string(),
            label: "Review step was part of the run".to_string(),
            severity: "info".to_string(),
            detail: None,
        });
    }
    if has_verify {
        signals.push(CodingRetroSignal {
            kind: "verification_used".to_string(),
            label: "Verification step was part of the run".to_string(),
            severity: "info".to_string(),
            detail: None,
        });
    }

    let mut recommendations = Vec::new();
    match run.state {
        WorkflowRunState::Completed => {
            if failed_ops == 0 && has_review && has_verify && has_diff {
                recommendations.push(CodingRetroRecommendation {
                    kind: "workflow_template".to_string(),
                    title: "Consider promoting this successful workflow shape".to_string(),
                    rationale: "The run completed with review, verification, and diff evidence."
                        .to_string(),
                });
            }
            if !has_verify {
                recommendations.push(CodingRetroRecommendation {
                    kind: "guidance_candidate".to_string(),
                    title: "Add a verification checkpoint".to_string(),
                    rationale: "The workflow completed without an explicit verify/validate step."
                        .to_string(),
                });
            }
        }
        WorkflowRunState::Blocked | WorkflowRunState::Failed => {
            recommendations.push(CodingRetroRecommendation {
                kind: "eval_candidate".to_string(),
                title: "Capture this terminal failure as deterministic eval coverage".to_string(),
                rationale: run.blocked_reason.clone().unwrap_or_else(|| {
                    "The workflow reached a non-success terminal state.".to_string()
                }),
            });
            recommendations.push(CodingRetroRecommendation {
                kind: "guidance_candidate".to_string(),
                title: "Tighten the workflow playbook for this failure mode".to_string(),
                rationale:
                    "A recurring blocker should become concrete project guidance before automation."
                        .to_string(),
            });
        }
        WorkflowRunState::Cancelled => {
            recommendations.push(CodingRetroRecommendation {
                kind: "workflow_policy".to_string(),
                title: "Clarify stop or cancellation criteria".to_string(),
                rationale:
                    "Cancelled runs are useful signals when long-task expectations were unclear."
                        .to_string(),
            });
        }
        _ => {}
    }

    let summary = format!(
        "{} workflow {} after {} completed op(s) and {} failed op(s).",
        run.execution_mode,
        run.state.as_str(),
        completed_ops,
        failed_ops
    );
    let now = now_rfc3339();
    CodingWorkflowRetro {
        id: format!("cwr_{}", uuid::Uuid::new_v4().simple()),
        session_id: run.session_id.clone(),
        project_id,
        workflow_run_id: run.id.clone(),
        run_state: run.state.as_str().to_string(),
        summary,
        signals,
        recommendations,
        created_at: run.completed_at.clone().unwrap_or_else(|| now.clone()),
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> (tempfile::TempDir, SessionDB) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open(&dir.path().join("sessions.db")).expect("session db");
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

    #[test]
    fn report_records_eval_success_rate() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        db.record_coding_eval_run(RecordCodingEvalRunInput {
            session_id: Some(session.id.clone()),
            project_id: None,
            suite: "coding_control_plane".to_string(),
            name: "sample_pass".to_string(),
            status: "passed".to_string(),
            metrics: json!({"criticalContextRecall": 1.0}),
            source_type: None,
            source_id: None,
        })
        .unwrap();
        db.record_coding_eval_run(RecordCodingEvalRunInput {
            session_id: Some(session.id.clone()),
            project_id: None,
            suite: "coding_control_plane".to_string(),
            name: "sample_fail".to_string(),
            status: "failed".to_string(),
            metrics: json!({"criticalContextRecall": 0.5}),
            source_type: None,
            source_id: None,
        })
        .unwrap();

        let report = db.coding_trend_report(&session.id, Some(30)).unwrap();
        assert_eq!(report.eval.runs, 2);
        assert_eq!(report.eval.passed, 1);
        assert_eq!(report.eval.failed, 1);
        assert_eq!(report.eval.success_rate, Some(0.5));
    }

    #[test]
    fn proposals_are_draft_only_and_deduped() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let goal = db
            .create_goal(crate::goal::CreateGoalInput {
                session_id: session.id.clone(),
                objective: "finish".to_string(),
                completion_criteria: "validated".to_string(),
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .unwrap();
        db.transition_goal(
            &goal.goal.id,
            crate::goal::GoalState::Blocked,
            Some("context miss"),
        )
        .unwrap();

        let first = db
            .generate_coding_improvement_proposals(&session.id, Some(30))
            .unwrap();
        let second = db
            .generate_coding_improvement_proposals(&session.id, Some(30))
            .unwrap();
        assert!(first.inserted > 0);
        assert_eq!(second.inserted, 0);
        assert!(second
            .proposals
            .iter()
            .any(|proposal| proposal.kind == "eval_candidate" && proposal.status == "draft"));
    }

    #[test]
    fn apply_eval_candidate_writes_reviewable_draft_artifact() {
        let (dir, db) = test_db();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        db.update_session_working_dir(&session.id, Some(workspace.to_string_lossy().to_string()))
            .unwrap();
        let goal = db
            .create_goal(crate::goal::CreateGoalInput {
                session_id: session.id.clone(),
                objective: "finish".to_string(),
                completion_criteria: "validated".to_string(),
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .unwrap();
        db.transition_goal(
            &goal.goal.id,
            crate::goal::GoalState::Blocked,
            Some("context miss"),
        )
        .unwrap();

        let generated = db
            .generate_coding_improvement_proposals(&session.id, Some(30))
            .unwrap();
        let proposal = generated
            .proposals
            .iter()
            .find(|proposal| proposal.kind == "eval_candidate")
            .expect("eval candidate proposal");
        let plan = db
            .preview_coding_improvement_proposal_action(&proposal.id)
            .unwrap();
        assert_eq!(plan.target_kind, "eval_candidate");
        assert!(plan.steps[0]
            .target_path
            .contains(".hope-agent/coding-improvement/eval-candidates"));

        let result = db.apply_coding_improvement_proposal(&proposal.id).unwrap();
        assert!(result.applied);
        assert_eq!(result.proposal.status, "applied");
        let artifact = result.artifacts.first().expect("artifact");
        assert!(std::path::Path::new(&artifact.path).is_file());
        assert!(result.proposal.action.as_ref().is_some_and(|action| {
            action.applied && action.artifacts.len() == 1 && action.error.is_none()
        }));
    }

    #[test]
    fn apply_eval_candidate_refuses_existing_target_without_overwrite() {
        let (dir, db) = test_db();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        db.update_session_working_dir(&session.id, Some(workspace.to_string_lossy().to_string()))
            .unwrap();
        let goal = db
            .create_goal(crate::goal::CreateGoalInput {
                session_id: session.id.clone(),
                objective: "finish".to_string(),
                completion_criteria: "validated".to_string(),
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .unwrap();
        db.transition_goal(
            &goal.goal.id,
            crate::goal::GoalState::Blocked,
            Some("context miss"),
        )
        .unwrap();

        let generated = db
            .generate_coding_improvement_proposals(&session.id, Some(30))
            .unwrap();
        let proposal = generated
            .proposals
            .iter()
            .find(|proposal| proposal.kind == "eval_candidate")
            .expect("eval candidate proposal");
        let plan = db
            .preview_coding_improvement_proposal_action(&proposal.id)
            .unwrap();
        let target = std::path::PathBuf::from(&plan.steps[0].target_path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "existing draft").unwrap();

        let result = db.apply_coding_improvement_proposal(&proposal.id).unwrap();
        assert!(!result.applied);
        assert_eq!(result.proposal.status, "failed");
        assert!(result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("target already exists")));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "existing draft");
    }

    #[test]
    fn applied_proposal_cannot_be_manually_reopened_or_rejected() {
        let (dir, db) = test_db();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        db.update_session_working_dir(&session.id, Some(workspace.to_string_lossy().to_string()))
            .unwrap();
        let goal = db
            .create_goal(crate::goal::CreateGoalInput {
                session_id: session.id.clone(),
                objective: "finish".to_string(),
                completion_criteria: "validated".to_string(),
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .unwrap();
        db.transition_goal(
            &goal.goal.id,
            crate::goal::GoalState::Blocked,
            Some("context miss"),
        )
        .unwrap();

        let generated = db
            .generate_coding_improvement_proposals(&session.id, Some(30))
            .unwrap();
        let proposal = generated
            .proposals
            .iter()
            .find(|proposal| proposal.kind == "eval_candidate")
            .expect("eval candidate proposal");
        let result = db.apply_coding_improvement_proposal(&proposal.id).unwrap();
        assert!(result.applied);
        assert_eq!(result.proposal.status, "applied");

        assert!(db
            .update_coding_improvement_proposal_status(&proposal.id, "draft")
            .unwrap_err()
            .to_string()
            .contains("already applied"));
        assert!(db
            .update_coding_improvement_proposal_status(&proposal.id, "rejected")
            .unwrap_err()
            .to_string()
            .contains("already applied"));
        let stored = db
            .get_coding_improvement_proposal(&proposal.id)
            .unwrap()
            .expect("proposal");
        assert_eq!(stored.status, "applied");
        assert!(stored.action.as_ref().is_some_and(|action| action.applied));
    }

    #[test]
    fn promote_eval_candidate_refuses_existing_formal_fixture_without_overwrite() {
        let (dir, db) = test_db();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        db.update_session_working_dir(&session.id, Some(workspace.to_string_lossy().to_string()))
            .unwrap();
        db.record_coding_eval_run(RecordCodingEvalRunInput {
            session_id: Some(session.id.clone()),
            project_id: None,
            suite: "coding_control_plane".to_string(),
            name: "existing_target".to_string(),
            status: "failed".to_string(),
            metrics: json!({}),
            source_type: Some("test".to_string()),
            source_id: Some("existing_target".to_string()),
        })
        .unwrap();

        let generated = db
            .generate_coding_improvement_proposals(&session.id, Some(30))
            .unwrap();
        let proposal = generated
            .proposals
            .iter()
            .find(|proposal| proposal.kind == "eval_candidate")
            .expect("eval candidate proposal");
        let applied = db.apply_coding_improvement_proposal(&proposal.id).unwrap();
        assert!(applied.applied);
        let draft_path = std::path::PathBuf::from(&applied.artifacts[0].path);
        let target = workspace
            .join("crates/ha-core/tests/fixtures/coding_eval")
            .join(draft_path.file_name().unwrap());
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "existing fixture").unwrap();

        let result = db
            .promote_coding_improvement_proposal(&proposal.id)
            .unwrap();
        assert!(!result.promoted);
        assert_eq!(result.proposal.status, "promotion_failed");
        assert!(result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("promotion target already exists")));
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "existing fixture"
        );
    }

    #[test]
    fn ordinary_workflow_block_does_not_count_as_repair_loop() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(crate::workflow::CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "coding.workflow".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: "export default async function main(workflow) { await workflow.block({ reason: 'context missing' }); }".to_string(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("test".to_string()),
                goal_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(
            &run.id,
            crate::workflow::WorkflowRunState::Running,
            Some("test"),
        )
        .unwrap();
        db.append_workflow_event(
            &run.id,
            "workflow_block_requested",
            json!({ "reason": "context missing" }),
        )
        .unwrap();
        db.transition_workflow_run(
            &run.id,
            crate::workflow::WorkflowRunState::Blocked,
            Some("context missing"),
        )
        .unwrap();

        let report = db.coding_trend_report(&session.id, Some(30)).unwrap();
        assert_eq!(report.repair_loop.runs, 0);
        assert_eq!(report.repair_loop.blocked, 0);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.category == "context_miss"));
    }
}
