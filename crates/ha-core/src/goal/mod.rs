//! Session-scoped Goal control plane.
//!
//! A Goal is the durable "what are we trying to finish?" object above
//! workflow/task execution. It lives in `sessions.db` so it shares the same
//! lifecycle as sessions, workflow runs, and tasks.

use std::collections::HashSet;

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::session::{MessageRole, SessionDB, Task};
use crate::workflow::{WorkflowOp, WorkflowOpState, WorkflowRun, WorkflowRunState};

const GOAL_EVENT_PAYLOAD_MAX_BYTES: usize = 64 * 1024;
const GOAL_EVIDENCE_MAX_FILE_LINKS: usize = 50;
const GOAL_EVIDENCE_MAX_ARTIFACT_LINKS: usize = 25;
const GOAL_EVIDENCE_MAX_DIAGNOSTIC_LINKS: usize = 50;
const GOAL_AUTO_CONTINUE_DELAY_SECS: i64 = 10;
const GOAL_AUTO_CONTINUE_MAX_PER_REVISION: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalState {
    Active,
    Paused,
    Evaluating,
    Completed,
    Failed,
    Cancelled,
    Blocked,
}

impl GoalState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Evaluating => "evaluating",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Blocked => "blocked",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "evaluating" => Some(Self::Evaluating),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    pub fn is_open(self) -> bool {
        matches!(
            self,
            Self::Active | Self::Paused | Self::Evaluating | Self::Blocked
        )
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        match (self, next) {
            (
                Self::Active,
                Self::Paused
                | Self::Evaluating
                | Self::Completed
                | Self::Failed
                | Self::Cancelled
                | Self::Blocked,
            ) => true,
            (Self::Paused, Self::Active | Self::Evaluating | Self::Cancelled) => true,
            (
                Self::Evaluating,
                Self::Active | Self::Completed | Self::Failed | Self::Cancelled | Self::Blocked,
            ) => true,
            (Self::Blocked, Self::Active | Self::Evaluating | Self::Failed | Self::Cancelled) => {
                true
            }
            _ => false,
        }
    }
}

fn goal_is_sealed_terminal(
    state: GoalState,
    closure_decision: Option<GoalClosureDecision>,
) -> bool {
    matches!(state, GoalState::Failed | GoalState::Cancelled)
        || (state == GoalState::Completed && closure_decision.is_some())
}

fn goal_accepts_new_evidence(goal: &Goal) -> bool {
    !goal_is_sealed_terminal(goal.state, goal.closure_decision)
}

fn goal_can_owner_transition(
    previous: GoalState,
    next: GoalState,
    closure_decision: Option<GoalClosureDecision>,
) -> bool {
    previous.can_transition_to(next)
        || (previous == GoalState::Completed
            && closure_decision.is_none()
            && matches!(next, GoalState::Evaluating | GoalState::Cancelled))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    pub id: String,
    pub session_id: String,
    pub objective: String,
    pub completion_criteria: String,
    pub revision: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_template_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_template_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_task_type: Option<String>,
    pub state: GoalState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode_snapshot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_token_limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_time_limit_secs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_turn_limit: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_summary: Option<String>,
    pub final_evidence: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    pub last_evaluator_result: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closure_decision: Option<GoalClosureDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closure_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    #[serde(default)]
    pub follow_up_items: Vec<GoalFollowUpItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalEvent {
    pub id: i64,
    pub goal_id: String,
    pub seq: i64,
    pub kind: String,
    pub payload: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalLink {
    pub id: i64,
    pub goal_id: String,
    pub target_type: String,
    pub target_id: String,
    pub relation: String,
    pub metadata: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalCriterionStatus {
    Satisfied,
    Missing,
    Blocked,
}

impl GoalCriterionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Satisfied => "satisfied",
            Self::Missing => "missing",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalCriterionKind {
    Required,
    Optional,
    FollowUp,
}

impl Default for GoalCriterionKind {
    fn default() -> Self {
        Self::Required
    }
}

impl GoalCriterionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Optional => "optional",
            Self::FollowUp => "follow_up",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "required" => Some(Self::Required),
            "optional" => Some(Self::Optional),
            "follow_up" | "followup" => Some(Self::FollowUp),
            _ => None,
        }
    }

    fn is_required(self) -> bool {
        matches!(self, Self::Required)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalCriterionItem {
    pub id: String,
    pub text: String,
    pub kind: GoalCriterionKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalCriterionBinding {
    pub id: String,
    pub text: String,
    pub kind: GoalCriterionKind,
    pub goal_revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalCriterionAudit {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub kind: GoalCriterionKind,
    pub status: GoalCriterionStatus,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalClosureDecision {
    AcceptedV1,
    NeedsStrictEvidence,
    Cancelled,
    Superseded,
}

impl GoalClosureDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AcceptedV1 => "accepted_v1",
            Self::NeedsStrictEvidence => "needs_strict_evidence",
            Self::Cancelled => "cancelled",
            Self::Superseded => "superseded",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "accepted_v1" | "accept_v1" | "accepted" => Some(Self::AcceptedV1),
            "needs_strict_evidence" | "strict" => Some(Self::NeedsStrictEvidence),
            "cancelled" | "canceled" | "cancel" => Some(Self::Cancelled),
            "superseded" | "supersede" => Some(Self::Superseded),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalFollowUpItem {
    pub id: String,
    pub text: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalEvidenceItem {
    pub id: String,
    pub source_type: String,
    pub source_id: String,
    pub relation: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub metadata: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalTimelineItem {
    pub id: String,
    pub kind: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub metadata: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalBudgetSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_limit_secs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_limit: Option<i64>,
    pub tokens_used: i64,
    pub elapsed_secs: i64,
    pub turns_used: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_ratio: Option<f64>,
    pub warning: bool,
    pub exhausted: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub exceeded: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalSnapshot {
    pub goal: Goal,
    pub links: Vec<GoalLink>,
    pub events: Vec<GoalEvent>,
    #[serde(default)]
    pub audit_stale: bool,
    #[serde(default)]
    pub criteria_items: Vec<GoalCriterionItem>,
    #[serde(default)]
    pub criteria: Vec<GoalCriterionAudit>,
    #[serde(default)]
    pub evidence: Vec<GoalEvidenceItem>,
    #[serde(default)]
    pub timeline: Vec<GoalTimelineItem>,
    #[serde(default)]
    pub budget: GoalBudgetSnapshot,
    #[serde(default)]
    pub workflow_runs: Vec<WorkflowRun>,
    #[serde(default)]
    pub tasks: Vec<Task>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalCompletionReport {
    pub goal_id: String,
    pub session_id: String,
    pub state: GoalState,
    pub status: String,
    pub objective: String,
    pub revision: i64,
    pub summary: String,
    pub usage: GoalBudgetSnapshot,
    pub evidence_count: usize,
    #[serde(default)]
    pub achieved: Vec<String>,
    #[serde(default)]
    pub missing: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub follow_up_items: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_risk: Option<String>,
    pub generated_at: String,
}

pub fn build_goal_completion_report(
    snapshot: &GoalSnapshot,
    summary_override: Option<&str>,
) -> GoalCompletionReport {
    let audit = &snapshot.goal.final_evidence;
    let status = audit
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_else(|| snapshot.goal.state.as_str())
        .to_string();
    let summary = summary_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| snapshot.goal.final_summary.clone())
        .or_else(|| {
            audit
                .get("summary")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            if snapshot.goal.state == GoalState::Completed {
                "Goal completed.".to_string()
            } else {
                "Goal is not complete yet.".to_string()
            }
        });

    let mut follow_up_items = audit_string_vec(audit.get("followUpItems"));
    for item in &snapshot.goal.follow_up_items {
        if !follow_up_items
            .iter()
            .any(|existing| existing == &item.text)
        {
            follow_up_items.push(item.text.clone());
        }
    }

    GoalCompletionReport {
        goal_id: snapshot.goal.id.clone(),
        session_id: snapshot.goal.session_id.clone(),
        state: snapshot.goal.state,
        status,
        objective: snapshot.goal.objective.clone(),
        revision: snapshot.goal.revision,
        summary,
        usage: snapshot.budget.clone(),
        evidence_count: snapshot.evidence.len(),
        achieved: audit_string_vec(audit.get("achieved")),
        missing: audit_string_vec(audit.get("missing")),
        blockers: audit_string_vec(audit.get("blockers")),
        follow_up_items,
        remaining_risk: audit
            .get("remainingRisk")
            .and_then(Value::as_str)
            .map(str::to_string),
        generated_at: now_rfc3339(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGoalInput {
    pub session_id: String,
    pub objective: String,
    #[serde(default)]
    pub completion_criteria: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_template_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_task_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_token_limit: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_time_limit_secs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_turn_limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateGoalInput {
    pub goal_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_criteria: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_template_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_task_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseGoalInput {
    pub goal_id: String,
    pub decision: GoalClosureDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub follow_up_items: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendGoalFollowUpInput {
    pub goal_id: String,
    pub items: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

pub(crate) fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS goals (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            objective TEXT NOT NULL,
            completion_criteria TEXT NOT NULL DEFAULT '',
            revision INTEGER NOT NULL DEFAULT 1,
            domain TEXT,
            workflow_template_id TEXT,
            workflow_template_version TEXT,
            workflow_task_type TEXT,
            state TEXT NOT NULL,
            mode_snapshot TEXT,
            budget_token_limit INTEGER,
            budget_time_limit_secs INTEGER,
            budget_turn_limit INTEGER,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            completed_at TEXT,
            final_summary TEXT,
            final_evidence_json TEXT NOT NULL DEFAULT '{}',
            blocked_reason TEXT,
            last_evaluator_result_json TEXT NOT NULL DEFAULT '{}',
            closure_decision TEXT,
            closure_reason TEXT,
            closed_at TEXT,
            follow_up_json TEXT NOT NULL DEFAULT '[]',
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS goal_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            goal_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (goal_id) REFERENCES goals(id) ON DELETE CASCADE,
            UNIQUE(goal_id, seq)
        );

        CREATE TABLE IF NOT EXISTS goal_links (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            goal_id TEXT NOT NULL,
            target_type TEXT NOT NULL,
            target_id TEXT NOT NULL,
            relation TEXT NOT NULL,
            metadata_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            FOREIGN KEY (goal_id) REFERENCES goals(id) ON DELETE CASCADE,
            UNIQUE(goal_id, target_type, target_id, relation)
        );

        CREATE INDEX IF NOT EXISTS idx_goals_session_updated
            ON goals(session_id, updated_at DESC);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_goals_session_open
            ON goals(session_id)
            WHERE state IN ('active','paused','evaluating','blocked');
        CREATE INDEX IF NOT EXISTS idx_goal_events_goal_seq
            ON goal_events(goal_id, seq);
        CREATE INDEX IF NOT EXISTS idx_goal_events_goal_kind_seq
            ON goal_events(goal_id, kind, seq);
        CREATE INDEX IF NOT EXISTS idx_goal_links_goal
            ON goal_links(goal_id);
        CREATE INDEX IF NOT EXISTS idx_goal_links_target
            ON goal_links(target_type, target_id);",
    )?;
    ensure_goal_column(
        conn,
        "goals",
        "revision",
        "ALTER TABLE goals ADD COLUMN revision INTEGER NOT NULL DEFAULT 1;",
    )?;
    ensure_goal_column(
        conn,
        "goals",
        "domain",
        "ALTER TABLE goals ADD COLUMN domain TEXT;",
    )?;
    ensure_goal_column(
        conn,
        "goals",
        "workflow_template_id",
        "ALTER TABLE goals ADD COLUMN workflow_template_id TEXT;",
    )?;
    ensure_goal_column(
        conn,
        "goals",
        "workflow_template_version",
        "ALTER TABLE goals ADD COLUMN workflow_template_version TEXT;",
    )?;
    ensure_goal_column(
        conn,
        "goals",
        "workflow_task_type",
        "ALTER TABLE goals ADD COLUMN workflow_task_type TEXT;",
    )?;
    ensure_goal_column(
        conn,
        "goals",
        "closure_decision",
        "ALTER TABLE goals ADD COLUMN closure_decision TEXT;",
    )?;
    ensure_goal_column(
        conn,
        "goals",
        "closure_reason",
        "ALTER TABLE goals ADD COLUMN closure_reason TEXT;",
    )?;
    ensure_goal_column(
        conn,
        "goals",
        "closed_at",
        "ALTER TABLE goals ADD COLUMN closed_at TEXT;",
    )?;
    ensure_goal_column(
        conn,
        "goals",
        "follow_up_json",
        "ALTER TABLE goals ADD COLUMN follow_up_json TEXT NOT NULL DEFAULT '[]';",
    )?;
    Ok(())
}

impl SessionDB {
    fn resolve_goal_domain_selection(
        &self,
        domain: Option<String>,
        workflow_template_id: Option<String>,
        workflow_template_version: Option<String>,
        workflow_task_type: Option<String>,
    ) -> Result<GoalDomainSelection> {
        let requested_domain = normalize_goal_domain_field(domain.as_deref());
        let requested_template_id = normalize_goal_text_field(workflow_template_id.as_deref());
        let requested_template_version =
            normalize_goal_text_field(workflow_template_version.as_deref());
        let requested_task_type = normalize_goal_domain_field(workflow_task_type.as_deref());

        let Some(template_id) = requested_template_id else {
            return Ok(GoalDomainSelection {
                domain: requested_domain,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: requested_task_type,
            });
        };

        let template = self
            .get_domain_workflow_template(&template_id, requested_template_version.as_deref())?
            .ok_or_else(|| anyhow!("domain workflow template not found: {template_id}"))?;
        if let Some(domain) = requested_domain.as_ref() {
            if domain != &template.domain {
                return Err(anyhow!(
                    "goal domain {} does not match template {} domain {}",
                    domain,
                    template.id,
                    template.domain
                ));
            }
        }
        let task_type = requested_task_type.or_else(|| template.task_types.first().cloned());
        if let Some(task_type) = task_type.as_ref() {
            if !template.task_types.is_empty()
                && !template
                    .task_types
                    .iter()
                    .any(|candidate| candidate == task_type)
            {
                return Err(anyhow!(
                    "task type {} is not supported by template {}",
                    task_type,
                    template.id
                ));
            }
        }

        Ok(GoalDomainSelection {
            domain: Some(template.domain.clone()),
            workflow_template_id: Some(template.id),
            workflow_template_version: Some(template.version),
            workflow_task_type: task_type,
        })
    }

    pub fn create_goal(&self, input: CreateGoalInput) -> Result<GoalSnapshot> {
        let objective = input.objective.trim();
        if objective.is_empty() {
            return Err(anyhow!("goal objective must not be empty"));
        }
        let criteria = input.completion_criteria.trim();
        let domain_selection = self.resolve_goal_domain_selection(
            input.domain,
            input.workflow_template_id,
            input.workflow_template_version,
            input.workflow_task_type,
        )?;
        let now = now_rfc3339();
        let id = format!("goal_{}", uuid::Uuid::new_v4().simple());
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let (incognito, mode): (i64, String) = conn
            .query_row(
                "SELECT incognito, execution_mode FROM sessions WHERE id = ?1",
                params![input.session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow!("Session not found: {}", input.session_id))?;
        if incognito != 0 {
            return Err(anyhow!(
                "Cannot create durable goal for incognito session {}",
                input.session_id
            ));
        }
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM goals
                 WHERE session_id = ?1
                   AND (
                        state IN ('active','paused','evaluating','blocked')
                        OR (state = 'completed' AND closure_decision IS NULL)
                   )
                 LIMIT 1",
                params![input.session_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            return Err(anyhow!(
                "Session already has an open goal {}; clear or complete it first",
                existing
            ));
        }
        conn.execute(
            "INSERT INTO goals (
                id, session_id, objective, completion_criteria,
                domain, workflow_template_id, workflow_template_version, workflow_task_type,
                state, mode_snapshot,
                budget_token_limit, budget_time_limit_secs, budget_turn_limit,
                created_at, updated_at, final_evidence_json, last_evaluator_result_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', ?9, ?10, ?11, ?12, ?13, ?13, '{}', '{}')",
            params![
                id,
                input.session_id,
                objective,
                criteria,
                domain_selection.domain,
                domain_selection.workflow_template_id,
                domain_selection.workflow_template_version,
                domain_selection.workflow_task_type,
                mode,
                input.budget_token_limit,
                input.budget_time_limit_secs,
                input.budget_turn_limit,
                now
            ],
        )?;
        drop(conn);
        let snapshot = self
            .goal_snapshot(&id, 100)?
            .ok_or_else(|| anyhow!("goal {} was not persisted", id))?;
        let _ = self.append_goal_event(
            &id,
            "goal_created",
            json!({
                "objective": objective,
                "completionCriteria": criteria,
                "revision": snapshot.goal.revision,
                "criteriaItems": snapshot.criteria_items,
                "domain": snapshot.goal.domain,
                "workflowTemplateId": snapshot.goal.workflow_template_id,
                "workflowTemplateVersion": snapshot.goal.workflow_template_version,
                "workflowTaskType": snapshot.goal.workflow_task_type,
                "modeSnapshot": mode,
            }),
        )?;
        emit_goal("goal:created", &snapshot.goal);
        Ok(snapshot)
    }

    pub fn update_goal(&self, input: UpdateGoalInput) -> Result<GoalSnapshot> {
        let objective = input.objective.as_deref().map(str::trim);
        if objective.is_some_and(str::is_empty) {
            return Err(anyhow!("goal objective must not be empty"));
        }
        let completion_criteria = input.completion_criteria.as_deref().map(str::trim);
        let has_domain_update = input.domain.is_some()
            || input.workflow_template_id.is_some()
            || input.workflow_template_version.is_some()
            || input.workflow_task_type.is_some();
        if objective.is_none() && completion_criteria.is_none() && !has_domain_update {
            return self
                .goal_snapshot(&input.goal_id, 100)?
                .ok_or_else(|| anyhow!("goal {} not found", input.goal_id));
        }

        let now = now_rfc3339();
        let (
            previous_objective,
            previous_criteria,
            previous_domain,
            previous_state,
            previous_revision,
        ) = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let current: Option<(
                String,
                String,
                i64,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                String,
            )> = conn
                .query_row(
                    "SELECT objective, completion_criteria, revision, domain, workflow_template_id,
                            workflow_template_version, workflow_task_type, closure_decision, state
                     FROM goals WHERE id = ?1",
                    params![input.goal_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                            row.get(8)?,
                        ))
                    },
                )
                .optional()?;
            let (
                previous_objective,
                previous_criteria,
                previous_revision,
                previous_domain,
                previous_template_id,
                previous_template_version,
                previous_task_type,
                previous_closure_decision,
                state,
            ) = current.ok_or_else(|| anyhow!("goal {} not found", input.goal_id))?;
            let previous_state = parse_goal_state(&state)?;
            let previous_closure_decision =
                parse_goal_closure_decision_sql(previous_closure_decision)?;
            if goal_is_sealed_terminal(previous_state, previous_closure_decision) {
                return Err(anyhow!("goal {} is terminal", input.goal_id));
            }
            let previous_pending_closure =
                previous_state == GoalState::Completed && previous_closure_decision.is_none();
            let next_objective = objective.unwrap_or(previous_objective.trim());
            let next_criteria = completion_criteria.unwrap_or(previous_criteria.trim());
            drop(conn);
            let next_domain_selection = if has_domain_update {
                self.resolve_goal_domain_selection(
                    input.domain,
                    input.workflow_template_id,
                    input.workflow_template_version,
                    input.workflow_task_type,
                )?
            } else {
                GoalDomainSelection {
                    domain: previous_domain.clone(),
                    workflow_template_id: previous_template_id.clone(),
                    workflow_template_version: previous_template_version.clone(),
                    workflow_task_type: previous_task_type.clone(),
                }
            };
            if next_objective == previous_objective.trim()
                && next_criteria == previous_criteria.trim()
                && next_domain_selection.domain == previous_domain
                && next_domain_selection.workflow_template_id == previous_template_id
                && next_domain_selection.workflow_template_version == previous_template_version
                && next_domain_selection.workflow_task_type == previous_task_type
            {
                return self
                    .goal_snapshot(&input.goal_id, 100)?
                    .ok_or_else(|| anyhow!("goal {} not found", input.goal_id));
            }
            let next_state = match previous_state {
                GoalState::Blocked | GoalState::Evaluating => GoalState::Active,
                GoalState::Completed if previous_pending_closure => GoalState::Active,
                other => other,
            };
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE goals
                    SET objective = COALESCE(?1, objective),
                        completion_criteria = COALESCE(?2, completion_criteria),
                        domain = ?3,
                        workflow_template_id = ?4,
                        workflow_template_version = ?5,
                        workflow_task_type = ?6,
                        state = ?7,
                        revision = revision + 1,
                        updated_at = ?8,
                        final_summary = NULL,
                        final_evidence_json = '{}',
                        blocked_reason = NULL,
                        last_evaluator_result_json = '{}',
                        closure_decision = NULL,
                        closure_reason = NULL,
                        closed_at = NULL
                 WHERE id = ?9",
                params![
                    objective,
                    completion_criteria,
                    next_domain_selection.domain,
                    next_domain_selection.workflow_template_id,
                    next_domain_selection.workflow_template_version,
                    next_domain_selection.workflow_task_type,
                    next_state.as_str(),
                    now,
                    input.goal_id
                ],
            )?;
            (
                previous_objective,
                previous_criteria,
                json!({
                    "domain": previous_domain,
                    "workflowTemplateId": previous_template_id,
                    "workflowTemplateVersion": previous_template_version,
                    "workflowTaskType": previous_task_type,
                }),
                previous_state,
                previous_revision,
            )
        };

        let snapshot = self
            .goal_snapshot(&input.goal_id, 100)?
            .ok_or_else(|| anyhow!("goal {} not found after update", input.goal_id))?;
        let _ = self.append_goal_event(
            &input.goal_id,
            "goal_updated",
            json!({
                "previous": {
                    "objective": previous_objective,
                    "completionCriteria": previous_criteria,
                    "revision": previous_revision,
                    "domain": previous_domain.get("domain").cloned().unwrap_or(Value::Null),
                    "workflowTemplateId": previous_domain.get("workflowTemplateId").cloned().unwrap_or(Value::Null),
                    "workflowTemplateVersion": previous_domain.get("workflowTemplateVersion").cloned().unwrap_or(Value::Null),
                    "workflowTaskType": previous_domain.get("workflowTaskType").cloned().unwrap_or(Value::Null),
                    "state": previous_state.as_str(),
                },
                "next": {
                    "objective": snapshot.goal.objective,
                    "completionCriteria": snapshot.goal.completion_criteria,
                    "revision": snapshot.goal.revision,
                    "criteriaItems": snapshot.criteria_items,
                    "domain": snapshot.goal.domain,
                    "workflowTemplateId": snapshot.goal.workflow_template_id,
                    "workflowTemplateVersion": snapshot.goal.workflow_template_version,
                    "workflowTaskType": snapshot.goal.workflow_task_type,
                    "state": snapshot.goal.state.as_str(),
                },
            }),
        )?;
        emit_goal("goal:updated", &snapshot.goal);
        Ok(snapshot)
    }

    pub fn get_goal(&self, goal_id: &str) -> Result<Option<Goal>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, session_id, objective, completion_criteria,
                    revision,
                    domain, workflow_template_id, workflow_template_version, workflow_task_type,
                    state, mode_snapshot,
                    budget_token_limit, budget_time_limit_secs, budget_turn_limit,
                    created_at, updated_at, completed_at, final_summary, final_evidence_json,
                    blocked_reason, last_evaluator_result_json,
                    closure_decision, closure_reason, closed_at, follow_up_json
             FROM goals WHERE id = ?1",
            params![goal_id],
            row_to_goal,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn active_goal_for_session(&self, session_id: &str) -> Result<Option<GoalSnapshot>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let goal_id: Option<String> = conn
            .query_row(
                "SELECT id FROM goals
                 WHERE session_id = ?1
                   AND (
                        state IN ('active','paused','evaluating','blocked')
                        OR (state = 'completed' AND closure_decision IS NULL)
                   )
                 ORDER BY updated_at DESC
                 LIMIT 1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?;
        drop(conn);
        match goal_id {
            Some(id) => self.goal_snapshot(&id, 100),
            None => Ok(None),
        }
    }

    pub fn latest_goal_for_session(&self, session_id: &str) -> Result<Option<GoalSnapshot>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let goal_id: Option<String> = conn
            .query_row(
                "SELECT id FROM goals
                 WHERE session_id = ?1
                 ORDER BY updated_at DESC
                 LIMIT 1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?;
        drop(conn);
        match goal_id {
            Some(id) => self.goal_snapshot(&id, 100),
            None => Ok(None),
        }
    }

    pub fn active_goal_id_for_session(&self, session_id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id FROM goals
             WHERE session_id = ?1
               AND (
                    state IN ('active','paused','evaluating','blocked')
                    OR (state = 'completed' AND closure_decision IS NULL)
               )
             ORDER BY updated_at DESC
             LIMIT 1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn resolve_goal_criterion_binding(
        &self,
        goal_id: &str,
        criterion_id: Option<&str>,
    ) -> Result<Option<GoalCriterionBinding>> {
        let Some(criterion_id) = criterion_id
            .map(str::trim)
            .filter(|criterion_id| !criterion_id.is_empty())
        else {
            return Ok(None);
        };
        let goal = self
            .get_goal(goal_id)?
            .ok_or_else(|| anyhow!("goal {} not found", goal_id))?;
        let criteria = parse_goal_criteria_items(&goal.completion_criteria);
        let item = criteria
            .into_iter()
            .find(|item| item.id == criterion_id)
            .ok_or_else(|| {
                anyhow!(
                    "goal criterion {} not found on goal {} revision {}",
                    criterion_id,
                    goal_id,
                    goal.revision
                )
            })?;
        Ok(Some(GoalCriterionBinding {
            id: item.id,
            text: item.text,
            kind: item.kind,
            goal_revision: goal.revision,
        }))
    }

    pub fn goal_snapshot(&self, goal_id: &str, event_limit: usize) -> Result<Option<GoalSnapshot>> {
        let Some(goal) = self.get_goal(goal_id)? else {
            return Ok(None);
        };
        let links = self.list_goal_links(goal_id)?;
        let events = self.list_goal_events(goal_id, event_limit)?;
        let workflow_runs = self.list_workflow_runs_for_goal(goal_id)?;
        let tasks = self.list_tasks(&goal.session_id).unwrap_or_default();
        let evidence = build_goal_evidence_items(&links, &tasks);
        let budget = self.build_goal_budget_snapshot(&goal)?;
        let latest_goal_linked_event = self.latest_goal_linked_event_marker(goal_id)?;
        let audit_stale = goal_final_audit_stale(&goal, &latest_goal_linked_event);
        let criteria_items = parse_goal_criteria_items(&goal.completion_criteria);
        let mut snapshot = GoalSnapshot {
            goal,
            links,
            events,
            audit_stale,
            criteria_items,
            criteria: Vec::new(),
            evidence,
            timeline: Vec::new(),
            budget,
            workflow_runs,
            tasks,
        };
        snapshot.criteria = build_goal_criteria_audit(&snapshot);
        snapshot.timeline = build_goal_timeline(&snapshot);
        Ok(Some(snapshot))
    }

    fn build_goal_budget_snapshot(&self, goal: &Goal) -> Result<GoalBudgetSnapshot> {
        let token_limit = positive_limit(goal.budget_token_limit);
        let time_limit_secs = positive_limit(goal.budget_time_limit_secs);
        let turn_limit = positive_limit(goal.budget_turn_limit);
        let created_at = parse_rfc3339_utc(&goal.created_at);
        let end_at = goal
            .completed_at
            .as_deref()
            .and_then(parse_rfc3339_utc)
            .unwrap_or_else(chrono::Utc::now);
        let elapsed_secs = created_at
            .map(|created| (end_at - created).num_seconds().max(0))
            .unwrap_or(0);

        let mut tokens_used = 0i64;
        let mut turns_used = 0i64;
        for message in self
            .load_session_messages(&goal.session_id)
            .unwrap_or_default()
        {
            let Some(message_at) = parse_rfc3339_utc(&message.timestamp) else {
                continue;
            };
            if created_at
                .map(|created| message_at < created)
                .unwrap_or(false)
            {
                continue;
            }
            if message.role == MessageRole::User {
                turns_used += 1;
            }
            tokens_used += message
                .tokens_in_last
                .or(message.tokens_in)
                .unwrap_or(0)
                .max(0);
            tokens_used += message.tokens_out.unwrap_or(0).max(0);
        }

        let token_ratio = ratio(tokens_used, token_limit);
        let time_ratio = ratio(elapsed_secs, time_limit_secs);
        let turn_ratio = ratio(turns_used, turn_limit);
        let mut warnings = Vec::new();
        let mut exceeded = Vec::new();
        collect_budget_state("tokens", token_ratio, &mut warnings, &mut exceeded);
        collect_budget_state("time", time_ratio, &mut warnings, &mut exceeded);
        collect_budget_state("turns", turn_ratio, &mut warnings, &mut exceeded);

        Ok(GoalBudgetSnapshot {
            token_limit,
            time_limit_secs,
            turn_limit,
            tokens_used,
            elapsed_secs,
            turns_used,
            token_ratio,
            time_ratio,
            turn_ratio,
            warning: !warnings.is_empty(),
            exhausted: !exceeded.is_empty(),
            warnings,
            exceeded,
        })
    }

    pub(crate) fn ensure_goal_budget_allows_new_workflow(&self, goal_id: &str) -> Result<()> {
        let goal = self
            .get_goal(goal_id)?
            .ok_or_else(|| anyhow!("goal {} not found", goal_id))?;
        let budget = self.build_goal_budget_snapshot(&goal)?;
        self.emit_goal_budget_threshold_events(goal_id, &budget);
        if budget.exhausted {
            return Err(anyhow!(
                "goal {} budget exhausted: {}",
                goal_id,
                budget.exceeded.join(", ")
            ));
        }
        Ok(())
    }

    fn emit_goal_budget_threshold_events(&self, goal_id: &str, budget: &GoalBudgetSnapshot) {
        for kind in &budget.warnings {
            if self.goal_budget_event_exists(goal_id, kind, "warning") {
                continue;
            }
            let _ = self.append_goal_event(
                goal_id,
                "budget_warning",
                json!({
                    "kind": kind,
                    "level": "warning",
                    "budget": budget,
                }),
            );
        }
        for kind in &budget.exceeded {
            if self.goal_budget_event_exists(goal_id, kind, "exhausted") {
                continue;
            }
            let _ = self.append_goal_event(
                goal_id,
                "budget_warning",
                json!({
                    "kind": kind,
                    "level": "exhausted",
                    "budget": budget,
                }),
            );
        }
    }

    fn goal_budget_event_exists(&self, goal_id: &str, kind: &str, level: &str) -> bool {
        self.list_goal_events(goal_id, 500)
            .unwrap_or_default()
            .into_iter()
            .any(|event| {
                event.kind == "budget_warning"
                    && event.payload.get("kind").and_then(Value::as_str) == Some(kind)
                    && event.payload.get("level").and_then(Value::as_str) == Some(level)
            })
    }

    pub fn pause_goal(&self, goal_id: &str) -> Result<GoalSnapshot> {
        self.transition_goal(goal_id, GoalState::Paused, Some("pause_requested"))
    }

    pub fn resume_goal(&self, goal_id: &str) -> Result<GoalSnapshot> {
        self.transition_goal(goal_id, GoalState::Active, Some("resume_requested"))
    }

    pub fn clear_goal(&self, goal_id: &str) -> Result<GoalSnapshot> {
        self.close_goal(CloseGoalInput {
            goal_id: goal_id.to_string(),
            decision: GoalClosureDecision::Cancelled,
            reason: Some("clear_requested".to_string()),
            follow_up_items: Vec::new(),
        })
    }

    pub fn evaluate_goal(&self, goal_id: &str) -> Result<GoalSnapshot> {
        let _ = self.transition_goal(goal_id, GoalState::Evaluating, Some("evaluate_requested"))?;
        let snapshot = self
            .goal_snapshot(goal_id, 200)?
            .ok_or_else(|| anyhow!("goal {} not found", goal_id))?;
        let mut audit = self.build_goal_audit(&snapshot)?;
        let completed = audit
            .get("status")
            .and_then(|v| v.as_str())
            .is_some_and(|status| status == "completed");
        let next = if completed {
            GoalState::Completed
        } else {
            GoalState::Blocked
        };
        let summary = audit
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or(if completed {
                "Goal completed"
            } else {
                "Goal is not complete"
            })
            .to_string();
        let blocked_reason = if completed {
            None
        } else {
            Some(
                audit
                    .get("blockedReason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("goal_evidence_incomplete")
                    .to_string(),
            )
        };
        let now = now_rfc3339();
        audit["evaluatedAt"] = json!(now);
        let evidence_json = stable_json(&audit)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE goals
                SET state = ?1,
                    updated_at = ?2,
                    completed_at = CASE WHEN ?1 IN ('completed','failed','cancelled') THEN ?2 ELSE completed_at END,
                    final_summary = ?3,
                    final_evidence_json = ?4,
                    blocked_reason = ?5,
                    last_evaluator_result_json = ?4
             WHERE id = ?6",
            params![
                next.as_str(),
                now,
                summary,
                evidence_json,
                blocked_reason,
                goal_id
            ],
        )?;
        drop(conn);
        let _ = self.append_goal_event(goal_id, "goal_evaluated", audit)?;
        let next_snapshot = self
            .goal_snapshot(goal_id, 200)?
            .ok_or_else(|| anyhow!("goal {} not found after evaluation", goal_id))?;
        emit_goal("goal:updated", &next_snapshot.goal);
        Ok(next_snapshot)
    }

    pub fn close_goal(&self, input: CloseGoalInput) -> Result<GoalSnapshot> {
        let now = now_rfc3339();
        let reason = input.reason.as_deref().map(str::trim).and_then(non_empty);
        let (previous_state, next_state, appended_follow_ups) = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let current: Option<(String, String, String, String, i64, Option<String>)> = conn
                .query_row(
                    "SELECT session_id, state, follow_up_json, final_evidence_json, revision, closure_decision
                     FROM goals WHERE id = ?1",
                    params![input.goal_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    },
                )
                .optional()?;
            let (
                session_id,
                state,
                follow_up_json,
                final_evidence_json,
                revision,
                closure_decision,
            ) = current.ok_or_else(|| anyhow!("goal {} not found", input.goal_id))?;
            let previous_state = parse_goal_state(&state)?;
            let previous_closure_decision = parse_goal_closure_decision_sql(closure_decision)?;
            if goal_is_sealed_terminal(previous_state, previous_closure_decision) {
                return Err(anyhow!("goal {} is already closed", input.goal_id));
            }
            let mut final_evidence = json_from_sql(&final_evidence_json)?;
            if input.decision == GoalClosureDecision::AcceptedV1 {
                let final_status = final_evidence.get("status").and_then(Value::as_str);
                let final_revision = final_evidence.get("goalRevision").and_then(Value::as_i64);
                if final_status != Some("completed") || final_revision != Some(revision) {
                    return Err(anyhow!(
                        "cannot accept goal closure before the current final audit is completed"
                    ));
                }
                if let Some(baseline_seq) = goal_audit_linked_event_seq(&final_evidence) {
                    let stale_goal_link: Option<i64> = conn
                        .query_row(
                            "SELECT seq FROM goal_events
                             WHERE goal_id = ?1
                               AND kind = 'goal_linked'
                               AND seq > ?2
                             ORDER BY seq DESC
                             LIMIT 1",
                            params![input.goal_id.as_str(), baseline_seq],
                            |row| row.get(0),
                        )
                        .optional()?;
                    if stale_goal_link.is_some() {
                        return Err(anyhow!(
                            "cannot accept goal closure because newer goal evidence exists; re-run final audit first"
                        ));
                    }
                } else if let Some(evaluated_at) = goal_audit_evaluated_at(&final_evidence) {
                    let stale_goal_link: Option<String> = conn
                        .query_row(
                            "SELECT created_at FROM goal_events
                             WHERE goal_id = ?1
                               AND kind = 'goal_linked'
                               AND created_at > ?2
                             ORDER BY created_at DESC
                             LIMIT 1",
                            params![input.goal_id.as_str(), evaluated_at],
                            |row| row.get(0),
                        )
                        .optional()?;
                    if stale_goal_link.is_some() {
                        return Err(anyhow!(
                            "cannot accept goal closure because newer goal evidence exists; re-run final audit first"
                        ));
                    }
                }
            }
            let next_state = match input.decision {
                GoalClosureDecision::AcceptedV1 => GoalState::Completed,
                GoalClosureDecision::NeedsStrictEvidence => GoalState::Blocked,
                GoalClosureDecision::Cancelled | GoalClosureDecision::Superseded => {
                    GoalState::Cancelled
                }
            };
            if next_state.is_open() {
                let other_open: Option<String> = conn
                    .query_row(
                        "SELECT id FROM goals
                         WHERE session_id = ?1
                           AND id != ?2
                           AND state IN ('active','paused','evaluating','blocked')
                         LIMIT 1",
                        params![session_id, input.goal_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                if let Some(other_open) = other_open {
                    return Err(anyhow!(
                        "cannot reopen goal {}; session already has open goal {}",
                        input.goal_id,
                        other_open
                    ));
                }
            }

            let mut follow_up_items: Vec<GoalFollowUpItem> = json_vec_from_sql(&follow_up_json)?;
            let mut appended_follow_ups = Vec::new();
            let mut seen_follow_up_texts: HashSet<String> = follow_up_items
                .iter()
                .map(|item| normalize_follow_up_text_key(&item.text))
                .collect();
            for text in input
                .follow_up_items
                .iter()
                .map(|item| item.trim())
                .filter(|item| !item.is_empty())
            {
                if !seen_follow_up_texts.insert(normalize_follow_up_text_key(text)) {
                    continue;
                }
                let item = GoalFollowUpItem {
                    id: format!("followup_{}", uuid::Uuid::new_v4().simple()),
                    text: text.to_string(),
                    created_at: now.clone(),
                    source: Some("closure".to_string()),
                };
                appended_follow_ups.push(item.clone());
                follow_up_items.push(item);
            }
            let follow_up_json = serde_json::to_string(&follow_up_items)?;
            if !final_evidence.is_object() {
                final_evidence = json!({});
            }
            let blocked_reason = if input.decision == GoalClosureDecision::NeedsStrictEvidence {
                Some(reason.unwrap_or("goal_needs_strict_evidence"))
            } else {
                None
            };
            final_evidence["closure"] = json!({
                "decision": input.decision.as_str(),
                "reason": reason,
                "closedAt": if input.decision == GoalClosureDecision::NeedsStrictEvidence {
                    None
                } else {
                    Some(now.as_str())
                },
                "requiresUserAcceptance": input.decision != GoalClosureDecision::AcceptedV1,
            });
            final_evidence["goalRevision"] = json!(revision);
            let final_evidence_json = stable_json(&final_evidence)?;
            conn.execute(
                "UPDATE goals
                    SET state = ?1,
                        closure_decision = ?2,
                        closure_reason = ?3,
                        closed_at = CASE WHEN ?2 = 'needs_strict_evidence' THEN NULL ELSE ?4 END,
                        completed_at = CASE WHEN ?1 IN ('completed','failed','cancelled') THEN ?4 ELSE NULL END,
                        blocked_reason = ?5,
                        follow_up_json = ?6,
                        final_evidence_json = ?7,
                        last_evaluator_result_json = ?7,
                        updated_at = ?4
                 WHERE id = ?8",
                params![
                    next_state.as_str(),
                    input.decision.as_str(),
                    reason,
                    now,
                    blocked_reason,
                    follow_up_json,
                    final_evidence_json,
                    input.goal_id
                ],
            )?;
            (previous_state, next_state, appended_follow_ups)
        };

        let snapshot = self
            .goal_snapshot(&input.goal_id, 200)?
            .ok_or_else(|| anyhow!("goal {} not found after close", input.goal_id))?;
        let _ = self.append_goal_event(
            &input.goal_id,
            "goal_closure_decided",
            json!({
                "from": previous_state.as_str(),
                "to": next_state.as_str(),
                "decision": input.decision.as_str(),
                "reason": reason,
                "revision": snapshot.goal.revision,
                "followUpItems": appended_follow_ups,
            }),
        )?;
        emit_goal("goal:updated", &snapshot.goal);
        Ok(snapshot)
    }

    pub fn append_goal_follow_up(&self, input: AppendGoalFollowUpInput) -> Result<GoalSnapshot> {
        let now = now_rfc3339();
        let source = input
            .source
            .as_deref()
            .map(str::trim)
            .and_then(non_empty)
            .unwrap_or("owner")
            .to_string();
        let requested_items: Vec<String> = input
            .items
            .iter()
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect();
        if requested_items.is_empty() {
            return Err(anyhow!("goal follow-up item must not be empty"));
        }

        let appended_follow_ups = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let current: Option<(String, String, Option<String>)> = conn
                .query_row(
                    "SELECT state, follow_up_json, closure_decision
                     FROM goals WHERE id = ?1",
                    params![input.goal_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let (state, follow_up_json, closure_decision) =
                current.ok_or_else(|| anyhow!("goal {} not found", input.goal_id))?;
            let state = parse_goal_state(&state)?;
            let closure_decision = parse_goal_closure_decision_sql(closure_decision)?;
            if goal_is_sealed_terminal(state, closure_decision) {
                return Err(anyhow!("goal {} is already closed", input.goal_id));
            }

            let mut follow_up_items: Vec<GoalFollowUpItem> = json_vec_from_sql(&follow_up_json)?;
            let mut appended_follow_ups = Vec::new();
            let mut seen_follow_up_texts: HashSet<String> = follow_up_items
                .iter()
                .map(|item| normalize_follow_up_text_key(&item.text))
                .collect();
            for text in requested_items {
                if !seen_follow_up_texts.insert(normalize_follow_up_text_key(&text)) {
                    continue;
                }
                let item = GoalFollowUpItem {
                    id: format!("followup_{}", uuid::Uuid::new_v4().simple()),
                    text,
                    created_at: now.clone(),
                    source: Some(source.clone()),
                };
                appended_follow_ups.push(item.clone());
                follow_up_items.push(item);
            }
            if !appended_follow_ups.is_empty() {
                let follow_up_json = serde_json::to_string(&follow_up_items)?;
                conn.execute(
                    "UPDATE goals
                        SET follow_up_json = ?1,
                            updated_at = ?2
                     WHERE id = ?3",
                    params![follow_up_json, now, input.goal_id],
                )?;
            }
            appended_follow_ups
        };

        if appended_follow_ups.is_empty() {
            return self
                .goal_snapshot(&input.goal_id, 200)?
                .ok_or_else(|| anyhow!("goal {} not found", input.goal_id));
        }

        let _ = self.append_goal_event(
            &input.goal_id,
            "goal_follow_up_added",
            json!({
                "items": appended_follow_ups,
                "source": source,
            }),
        )?;
        let snapshot = self
            .goal_snapshot(&input.goal_id, 200)?
            .ok_or_else(|| anyhow!("goal {} not found", input.goal_id))?;
        emit_goal("goal:updated", &snapshot.goal);
        Ok(snapshot)
    }

    pub fn transition_goal(
        &self,
        goal_id: &str,
        next: GoalState,
        reason: Option<&str>,
    ) -> Result<GoalSnapshot> {
        let now = now_rfc3339();
        let previous = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let current: Option<(String, Option<String>)> = conn
                .query_row(
                    "SELECT state, closure_decision FROM goals WHERE id = ?1",
                    params![goal_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let (state, closure_decision) =
                current.ok_or_else(|| anyhow!("goal {} not found", goal_id))?;
            let previous = parse_goal_state(&state)?;
            let closure_decision = parse_goal_closure_decision_sql(closure_decision)?;
            if !goal_can_owner_transition(previous, next, closure_decision) {
                return Err(anyhow!(
                    "invalid goal transition {} -> {}",
                    previous.as_str(),
                    next.as_str()
                ));
            }
            conn.execute(
                "UPDATE goals
                    SET state = ?1,
                        blocked_reason = CASE WHEN ?1 = 'blocked' THEN ?2 ELSE NULL END,
                        completed_at = CASE WHEN ?1 IN ('completed','failed','cancelled') THEN ?3 ELSE completed_at END,
                        updated_at = ?3
                 WHERE id = ?4",
                params![next.as_str(), reason, now, goal_id],
            )?;
            previous
        };
        let _ = self.append_goal_event(
            goal_id,
            "goal_state_changed",
            json!({
                "from": previous.as_str(),
                "to": next.as_str(),
                "reason": reason,
            }),
        )?;
        let snapshot = self
            .goal_snapshot(goal_id, 100)?
            .ok_or_else(|| anyhow!("goal {} not found after transition", goal_id))?;
        emit_goal("goal:updated", &snapshot.goal);
        Ok(snapshot)
    }

    pub fn link_goal_target(
        &self,
        goal_id: &str,
        target_type: &str,
        target_id: &str,
        relation: &str,
        metadata: Value,
    ) -> Result<GoalLink> {
        let goal = self
            .get_goal(goal_id)?
            .ok_or_else(|| anyhow!("goal {} not found", goal_id))?;
        if !goal_accepts_new_evidence(&goal) {
            return Err(anyhow!("goal {} is terminal", goal_id));
        }
        let now = now_rfc3339();
        let metadata_json = stable_json(&metadata)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO goal_links (goal_id, target_type, target_id, relation, metadata_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(goal_id, target_type, target_id, relation)
             DO UPDATE SET metadata_json = excluded.metadata_json",
            params![goal_id, target_type, target_id, relation, metadata_json, now],
        )?;
        let id: i64 = conn.query_row(
            "SELECT id FROM goal_links
             WHERE goal_id = ?1 AND target_type = ?2 AND target_id = ?3 AND relation = ?4",
            params![goal_id, target_type, target_id, relation],
            |row| row.get(0),
        )?;
        drop(conn);
        let link = self
            .get_goal_link(id)?
            .ok_or_else(|| anyhow!("goal link {} not found after upsert", id))?;
        let _ = self.append_goal_event(
            goal_id,
            "goal_linked",
            json!({
                "targetType": target_type,
                "targetId": target_id,
                "relation": relation,
                "metadata": link.metadata,
            }),
        )?;
        emit_goal_link("goal:link_updated", &link);
        Ok(link)
    }

    pub(crate) fn link_goal_evidence_for_workflow_op(
        &self,
        run: &WorkflowRun,
        op: &WorkflowOp,
    ) -> Result<()> {
        let Some(goal_id) = run.goal_id.as_deref() else {
            return Ok(());
        };
        match op.op_type.as_str() {
            "finish" => {
                if op.state != WorkflowOpState::Completed {
                    return Ok(());
                }
                self.link_goal_artifact_evidence_for_workflow_finish(goal_id, run, op)?;
            }
            "validate" => {
                if !op.state.is_terminal() {
                    return Ok(());
                }
                let output = op.output.as_ref().unwrap_or(&Value::Null);
                let ok = op.state == WorkflowOpState::Completed
                    && output.get("ok").and_then(Value::as_bool).unwrap_or(false);
                let relation = if ok {
                    "validation_passed"
                } else {
                    "validation_failed"
                };
                let results_len = output
                    .get("results")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0);
                let metadata = json!({
                    "runId": run.id,
                    "opKey": op.op_key,
                    "opType": op.op_type,
                    "kind": run.kind,
                    "state": op.state,
                    "ok": ok,
                    "summary": output.get("summary").cloned().unwrap_or(Value::Null),
                    "results": results_len,
                    "error": op.error,
                    "completedAt": op.completed_at,
                });
                let _ = self.link_goal_target(
                    goal_id,
                    "validation",
                    &format!("{}:{}", run.id, op.op_key),
                    relation,
                    metadata,
                )?;
            }
            "diff" => {
                if op.state != WorkflowOpState::Completed {
                    return Ok(());
                }
                let output = op.output.as_ref().unwrap_or(&Value::Null);
                let changes = output
                    .get("changes")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let files_changed = changes.len();
                let lines_added: u64 = changes
                    .iter()
                    .filter_map(|change| change.get("linesAdded").and_then(Value::as_u64))
                    .sum();
                let lines_removed: u64 = changes
                    .iter()
                    .filter_map(|change| change.get("linesRemoved").and_then(Value::as_u64))
                    .sum();
                let metadata = json!({
                    "runId": run.id,
                    "opKey": op.op_key,
                    "opType": op.op_type,
                    "kind": run.kind,
                    "filesChanged": files_changed,
                    "linesAdded": lines_added,
                    "linesRemoved": lines_removed,
                    "truncated": files_changed > GOAL_EVIDENCE_MAX_FILE_LINKS,
                    "completedAt": op.completed_at,
                });
                let _ = self.link_goal_target(
                    goal_id,
                    "diff",
                    &format!("{}:{}", run.id, op.op_key),
                    "diff_snapshot",
                    metadata,
                )?;

                for change in changes.iter().take(GOAL_EVIDENCE_MAX_FILE_LINKS) {
                    let Some(path) = change.get("path").and_then(Value::as_str) else {
                        continue;
                    };
                    if path.trim().is_empty() {
                        continue;
                    }
                    let metadata = json!({
                        "runId": run.id,
                        "opKey": op.op_key,
                        "action": change.get("action").cloned().unwrap_or(Value::Null),
                        "linesAdded": change.get("linesAdded").cloned().unwrap_or(Value::Null),
                        "linesRemoved": change.get("linesRemoved").cloned().unwrap_or(Value::Null),
                        "language": change.get("language").cloned().unwrap_or(Value::Null),
                        "completedAt": op.completed_at,
                    });
                    let _ =
                        self.link_goal_target(goal_id, "file", path, "file_changed", metadata)?;
                }
            }
            "tool:lsp" => {
                if op.state != WorkflowOpState::Completed {
                    return Ok(());
                }
                self.link_goal_diagnostic_evidence_for_workflow_lsp(goal_id, run, op)?;
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn link_goal_worktree_evidence_for_workflow_run(
        &self,
        run: &WorkflowRun,
    ) -> Result<()> {
        let Some(goal_id) = run.goal_id.as_deref() else {
            return Ok(());
        };
        let Some(worktree_id) = run.worktree_id.as_deref() else {
            return Ok(());
        };
        let Some(worktree) = self.get_managed_worktree(worktree_id)? else {
            return Ok(());
        };
        self.link_goal_target(
            goal_id,
            "worktree",
            &worktree.id,
            "worktree_attached",
            goal_worktree_metadata(&worktree, Some(run)),
        )?;
        Ok(())
    }

    pub(crate) fn refresh_goal_worktree_evidence(
        &self,
        worktree: &crate::worktree::ManagedWorktree,
    ) -> Result<()> {
        let runs = self.list_workflow_runs_for_worktree(&worktree.id)?;
        for run in runs {
            let Some(goal_id) = run.goal_id.as_deref() else {
                continue;
            };
            let _ = self.link_goal_target(
                goal_id,
                "worktree",
                &worktree.id,
                "worktree_attached",
                goal_worktree_metadata(worktree, Some(&run)),
            )?;
        }
        Ok(())
    }

    fn link_goal_artifact_evidence_for_workflow_finish(
        &self,
        goal_id: &str,
        run: &WorkflowRun,
        op: &WorkflowOp,
    ) -> Result<()> {
        let Some(output) = op.output.as_ref() else {
            return Ok(());
        };
        for (index, artifact) in goal_artifacts_from_finish_output(output)
            .into_iter()
            .take(GOAL_EVIDENCE_MAX_ARTIFACT_LINKS)
            .enumerate()
        {
            let target_id = artifact_target_id(&artifact)
                .unwrap_or_else(|| format!("{}:{}:artifact#{}", run.id, op.op_key, index + 1));
            let title = artifact_title(&artifact, &target_id);
            let metadata = json!({
                "runId": run.id,
                "opKey": op.op_key,
                "opType": op.op_type,
                "kind": run.kind,
                "title": title,
                "summary": artifact_summary(&artifact),
                "artifactKind": artifact_string_any(&artifact, &["kind", "type", "artifactKind", "artifact_kind"]),
                "path": artifact_string_any(&artifact, &["path", "filePath", "file_path"]),
                "artifactId": artifact_string_any(&artifact, &["id", "artifactId", "artifact_id"]),
                "url": artifact_string_any(&artifact, &["url", "href"]),
                "hash": artifact_string_any(&artifact, &["hash", "contentHash", "content_hash"]),
                "completedAt": op.completed_at,
                "source": "workflow.finish",
            });
            let _ = self.link_goal_target(
                goal_id,
                "artifact",
                &target_id,
                "artifact_created",
                metadata,
            )?;
        }
        Ok(())
    }

    fn link_goal_diagnostic_evidence_for_workflow_lsp(
        &self,
        goal_id: &str,
        run: &WorkflowRun,
        op: &WorkflowOp,
    ) -> Result<()> {
        let action = op
            .input
            .get("args")
            .and_then(|args| args.get("action"))
            .and_then(Value::as_str)
            .unwrap_or("diagnostics");
        if !matches!(action, "diagnostics" | "sync_file") {
            return Ok(());
        }
        let Some(output) = op.output.as_ref() else {
            return Ok(());
        };
        let parsed = parse_workflow_tool_json_output(output).unwrap_or_else(|| output.clone());
        let output_action = parsed
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or(action);
        if !matches!(output_action, "diagnostics" | "sync_file") {
            return Ok(());
        }
        let diagnostics = parsed
            .get("diagnostics")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let errors = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic_severity(diagnostic) == "error")
            .count();
        let warnings = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic_severity(diagnostic) == "warning")
            .count();
        let summary = if diagnostics.is_empty() {
            "No LSP diagnostics reported".to_string()
        } else {
            format!(
                "{} LSP diagnostic(s): {} error(s), {} warning(s)",
                diagnostics.len(),
                errors,
                warnings
            )
        };
        let summary_status = if errors > 0 { "failed" } else { "passed" };
        let workspace_root = parsed.get("workspaceRoot").cloned().unwrap_or(Value::Null);
        let path = lsp_diagnostic_scope_path(&op.input, &parsed);
        let _ = self.link_goal_target(
            goal_id,
            "diagnostic",
            &format!("{}:{}:summary", run.id, op.op_key),
            "diagnostic_result",
            json!({
                "runId": run.id,
                "opKey": op.op_key,
                "opType": op.op_type,
                "kind": run.kind,
                "action": output_action,
                "status": summary_status,
                "severity": if errors > 0 { "error" } else { "none" },
                "summary": summary,
                "diagnostics": diagnostics.len(),
                "errors": errors,
                "warnings": warnings,
                "path": path,
                "workspaceRoot": workspace_root,
                "completedAt": op.completed_at,
                "source": "workflow.tool:lsp",
                "truncated": diagnostics.len() > GOAL_EVIDENCE_MAX_DIAGNOSTIC_LINKS,
            }),
        )?;
        for diagnostic in diagnostics.iter().take(GOAL_EVIDENCE_MAX_DIAGNOSTIC_LINKS) {
            let target_id = diagnostic_target_id(run, op, diagnostic);
            let severity = diagnostic_severity(diagnostic);
            let message = diagnostic_message(diagnostic);
            let path = diagnostic_path(diagnostic);
            let metadata = json!({
                "runId": run.id,
                "opKey": op.op_key,
                "opType": op.op_type,
                "kind": run.kind,
                "action": output_action,
                "path": path,
                "uri": diagnostic.get("uri").cloned().unwrap_or(Value::Null),
                "range": diagnostic.get("range").cloned().unwrap_or(Value::Null),
                "line": diagnostic
                    .get("range")
                    .and_then(|range| range.get("startLine"))
                    .and_then(Value::as_u64),
                "column": diagnostic
                    .get("range")
                    .and_then(|range| range.get("startColumn"))
                    .and_then(Value::as_u64),
                "severity": severity,
                "status": if severity == "error" { "failed" } else { "reported" },
                "source": diagnostic.get("source").cloned().unwrap_or_else(|| json!("lsp")),
                "code": diagnostic.get("code").cloned().unwrap_or(Value::Null),
                "message": message,
                "summary": format!("{}: {}", severity, message),
                "completedAt": op.completed_at,
            });
            let _ = self.link_goal_target(
                goal_id,
                "diagnostic",
                &target_id,
                "diagnostic_result",
                metadata,
            )?;
        }
        Ok(())
    }

    pub fn append_goal_event(
        &self,
        goal_id: &str,
        kind: &str,
        payload: Value,
    ) -> Result<GoalEvent> {
        let payload_json = bounded_payload(payload)?;
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM goal_events WHERE goal_id = ?1",
            params![goal_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO goal_events (goal_id, seq, kind, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![goal_id, seq, kind, payload_json, now],
        )?;
        let id = conn.last_insert_rowid();
        let event = GoalEvent {
            id,
            goal_id: goal_id.to_string(),
            seq,
            kind: kind.to_string(),
            payload: serde_json::from_str(&payload_json)?,
            created_at: now,
        };
        drop(conn);
        emit_goal_event("goal:event", &event);
        Ok(event)
    }

    pub fn list_goal_events(&self, goal_id: &str, limit: usize) -> Result<Vec<GoalEvent>> {
        let limit = limit.clamp(1, 500) as i64;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, goal_id, seq, kind, payload_json, created_at
             FROM goal_events
             WHERE goal_id = ?1
             ORDER BY seq DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![goal_id, limit], row_to_goal_event)?;
        let mut events = collect_rows(rows)?;
        events.reverse();
        Ok(events)
    }

    fn latest_goal_linked_event_marker(&self, goal_id: &str) -> Result<GoalLinkedEventMarker> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let marker: Option<GoalLinkedEventMarker> = conn
            .query_row(
                "SELECT seq, created_at
                 FROM goal_events
                 WHERE goal_id = ?1 AND kind = 'goal_linked'
                 ORDER BY seq DESC
                 LIMIT 1",
                params![goal_id],
                |row| {
                    Ok(GoalLinkedEventMarker {
                        seq: row.get(0)?,
                        created_at: Some(row.get(1)?),
                    })
                },
            )
            .optional()?;
        Ok(marker.unwrap_or_default())
    }

    pub fn list_goal_links(&self, goal_id: &str) -> Result<Vec<GoalLink>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, goal_id, target_type, target_id, relation, metadata_json, created_at
             FROM goal_links
             WHERE goal_id = ?1
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![goal_id], row_to_goal_link)?;
        collect_rows(rows)
    }

    fn get_goal_link(&self, id: i64) -> Result<Option<GoalLink>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, goal_id, target_type, target_id, relation, metadata_json, created_at
             FROM goal_links WHERE id = ?1",
            params![id],
            row_to_goal_link,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_workflow_runs_for_goal(&self, goal_id: &str) -> Result<Vec<WorkflowRun>> {
        let ids = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let mut stmt = conn.prepare(
                "SELECT id FROM workflow_runs
                 WHERE goal_id = ?1
                    OR id IN (
                        SELECT target_id FROM goal_links
                        WHERE goal_id = ?1 AND target_type = 'workflow_run'
                    )
                 ORDER BY updated_at DESC, created_at DESC",
            )?;
            let ids = stmt.query_map(params![goal_id], |row| row.get::<_, String>(0))?;
            collect_rows(ids)?
        };
        let mut runs = Vec::new();
        for id in ids {
            if let Some(run) = self.get_workflow_run(&id)? {
                runs.push(run);
            }
        }
        Ok(runs)
    }

    fn list_workflow_runs_for_worktree(&self, worktree_id: &str) -> Result<Vec<WorkflowRun>> {
        let ids = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let mut stmt = conn.prepare(
                "SELECT id FROM workflow_runs
                 WHERE worktree_id = ?1
                    OR id IN (
                        SELECT workflow_run_id FROM managed_worktrees
                        WHERE id = ?1 AND workflow_run_id IS NOT NULL
                    )
                 ORDER BY updated_at DESC, created_at DESC",
            )?;
            let ids = stmt.query_map(params![worktree_id], |row| row.get::<_, String>(0))?;
            collect_rows(ids)?
        };
        let mut runs = Vec::new();
        for id in ids {
            if let Some(run) = self.get_workflow_run(&id)? {
                runs.push(run);
            }
        }
        Ok(runs)
    }

    fn build_goal_audit(&self, snapshot: &GoalSnapshot) -> Result<Value> {
        let mut audit = build_goal_rule_audit(snapshot);
        audit["goalLinkedEventSeq"] =
            json!(self.latest_goal_linked_event_marker(&snapshot.goal.id)?.seq);
        Ok(audit)
    }
}

pub fn maybe_schedule_goal_continuation(
    db: &SessionDB,
    session_id: &str,
    agent_id: &str,
    source: crate::chat_engine::ChatSource,
    turn_id: Option<&str>,
    assistant_message_id: Option<i64>,
) -> Result<Option<crate::wakeup::ScheduleOutcome>> {
    if matches!(source, crate::chat_engine::ChatSource::Subagent) {
        return Ok(None);
    }
    let Some(snapshot) = db.active_goal_for_session(session_id)? else {
        return Ok(None);
    };
    if !goal_runner_should_continue(&snapshot) {
        return Ok(None);
    }
    let scheduled_this_turn = snapshot.events.iter().any(|event| {
        event.kind == "goal_auto_continue_scheduled"
            && event.payload.get("turnId").and_then(Value::as_str) == turn_id
    });
    if scheduled_this_turn {
        return Ok(None);
    }
    let scheduled_for_revision = snapshot
        .events
        .iter()
        .filter(|event| {
            event.kind == "goal_auto_continue_scheduled"
                && event.payload.get("goalRevision").and_then(Value::as_i64)
                    == Some(snapshot.goal.revision)
        })
        .count();
    if scheduled_for_revision >= GOAL_AUTO_CONTINUE_MAX_PER_REVISION {
        let _ = db.append_goal_event(
            &snapshot.goal.id,
            "goal_auto_continue_halted",
            json!({
                "reason": "max_auto_continues_per_revision",
                "limit": GOAL_AUTO_CONTINUE_MAX_PER_REVISION,
                "goalRevision": snapshot.goal.revision,
                "turnId": turn_id,
            }),
        );
        return Ok(None);
    }

    let note = format!(
        "<goal-continuation>\n\
         Continue the active Goal autonomously.\n\
         - Goal id: {}\n\
         - Revision: {}\n\
         - First call `goal_status` to verify the latest objective, revision, budget, and evidence.\n\
         - If required criteria are satisfied, call `goal_finish_request` before the final user summary.\n\
         - If real progress is impossible, call `goal_block_request` with concrete attempts.\n\
         - Otherwise complete one meaningful step, update tasks/checkpoints/evidence, and continue until the Goal is done.\n\
         </goal-continuation>",
        snapshot.goal.id, snapshot.goal.revision
    );
    let outcome = crate::wakeup::schedule(
        session_id,
        agent_id,
        GOAL_AUTO_CONTINUE_DELAY_SECS,
        Some(note),
        false,
    )
    .map_err(|e| anyhow!("failed to schedule goal continuation: {e:?}"))?;
    let _ = db.append_goal_event(
        &snapshot.goal.id,
        "goal_auto_continue_scheduled",
        json!({
            "wakeupId": outcome.id,
            "fireAt": outcome.fire_at,
            "delaySecs": outcome.delay_secs,
            "source": source.as_str(),
            "turnId": turn_id,
            "assistantMessageId": assistant_message_id,
            "goalRevision": snapshot.goal.revision,
            "scheduledForRevision": scheduled_for_revision + 1,
        }),
    );
    Ok(Some(outcome))
}

fn goal_runner_should_continue(snapshot: &GoalSnapshot) -> bool {
    match snapshot.goal.state {
        GoalState::Active | GoalState::Evaluating => {}
        GoalState::Blocked => {
            let reason = snapshot.goal.blocked_reason.as_deref().unwrap_or_default();
            if !matches!(
                reason,
                "goal_evidence_incomplete" | "goal_blocked_by_evidence" | ""
            ) {
                return false;
            }
        }
        GoalState::Paused | GoalState::Completed | GoalState::Failed | GoalState::Cancelled => {
            return false
        }
    }
    if snapshot.budget.exhausted {
        return false;
    }
    if snapshot.goal.closure_decision == Some(GoalClosureDecision::AcceptedV1) {
        return false;
    }
    let audit_status = snapshot
        .goal
        .final_evidence
        .get("status")
        .and_then(Value::as_str);
    audit_status != Some("completed") || snapshot.audit_stale
}

fn normalize_follow_up_text_key(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn audit_string_vec(value: Option<&Value>) -> Vec<String> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            if let Some(text) = item.as_str() {
                return Some(text.to_string());
            }
            item.get("text")
                .or_else(|| item.get("summary"))
                .or_else(|| item.get("reason"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|item| !item.trim().is_empty())
        .collect()
}

fn split_criteria(raw: &str) -> Vec<String> {
    parse_goal_criteria_items(raw)
        .into_iter()
        .map(|item| item.text)
        .collect()
}

fn parse_goal_criteria_items(raw: &str) -> Vec<GoalCriterionItem> {
    let mut items = Vec::new();
    let mut section_kind = GoalCriterionKind::Required;
    for raw_part in raw.lines().flat_map(|line| line.split(';')) {
        let mut text = clean_goal_criterion_text(raw_part);
        if text.is_empty() {
            continue;
        }
        let mut kind = section_kind;
        if let Some((parsed_kind, rest)) = parse_goal_criterion_kind_prefix(&text) {
            let rest = clean_goal_criterion_text(rest);
            if rest.is_empty() {
                section_kind = parsed_kind;
                continue;
            }
            text = rest;
            kind = parsed_kind;
        }
        items.push(GoalCriterionItem {
            id: format!("criterion-{}", items.len() + 1),
            text,
            kind,
        });
    }
    items
}

fn clean_goal_criterion_text(raw: &str) -> String {
    let mut text = raw.trim();
    loop {
        let next = text
            .trim_start_matches('-')
            .trim_start_matches('*')
            .trim_start_matches('\u{2022}')
            .trim();
        if next == text {
            break;
        }
        text = next;
    }
    for checkbox in ["[ ]", "[x]", "[X]", "\u{2610}", "\u{2611}"] {
        if let Some(rest) = text.strip_prefix(checkbox) {
            text = rest.trim();
            break;
        }
    }
    let numbered = text
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if !numbered.is_empty() {
        let rest = &text[numbered.len()..];
        if let Some(stripped) = rest
            .strip_prefix('.')
            .or_else(|| rest.strip_prefix('\u{3001}'))
            .or_else(|| rest.strip_prefix(')'))
        {
            text = stripped.trim();
        }
    }
    text.to_string()
}

fn parse_goal_criterion_kind_prefix(text: &str) -> Option<(GoalCriterionKind, &str)> {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            let label = normalize_goal_kind_label(&rest[..end]);
            if let Some(kind) = goal_kind_from_label(&label) {
                return Some((kind, &rest[end + 1..]));
            }
        }
    }
    for separator in [":", "\u{ff1a}"] {
        if let Some((label, rest)) = trimmed.split_once(separator) {
            let normalized = normalize_goal_kind_label(label);
            if let Some(kind) = goal_kind_from_label(&normalized) {
                return Some((kind, rest));
            }
        }
    }
    None
}

fn normalize_goal_kind_label(label: &str) -> String {
    label.trim().to_lowercase().replace([' ', '-'], "_")
}

fn goal_kind_from_label(label: &str) -> Option<GoalCriterionKind> {
    match label {
        "required" | "require" | "must" | "must_have" | "\u{5fc5}\u{987b}" | "\u{5fc5}\u{9700}"
        | "\u{5fc5}\u{8981}" => Some(GoalCriterionKind::Required),
        "optional" | "nice_to_have" | "\u{53ef}\u{9009}" | "\u{53ef}\u{6709}" => {
            Some(GoalCriterionKind::Optional)
        }
        "follow_up"
        | "followup"
        | "later"
        | "backlog"
        | "\u{540e}\u{7eed}"
        | "\u{540e}\u{7eed}\u{9879}"
        | "\u{589e}\u{5f3a}" => Some(GoalCriterionKind::FollowUp),
        _ => None,
    }
}

#[derive(Debug, Clone, Default)]
struct GoalLinkedEventMarker {
    seq: i64,
    created_at: Option<String>,
}

fn goal_final_audit_stale(goal: &Goal, latest_goal_linked_event: &GoalLinkedEventMarker) -> bool {
    if goal.final_summary.is_none() && goal.final_evidence == json!({}) {
        return false;
    }
    if goal
        .final_evidence
        .get("goalRevision")
        .and_then(Value::as_i64)
        != Some(goal.revision)
    {
        return true;
    }
    if let Some(baseline_seq) = goal_audit_linked_event_seq(&goal.final_evidence) {
        return latest_goal_linked_event.seq > baseline_seq;
    }
    goal_audit_evaluated_at(&goal.final_evidence).is_some_and(|evaluated_at| {
        latest_goal_linked_event
            .created_at
            .as_deref()
            .is_some_and(|created_at| created_at > evaluated_at)
    })
}

fn goal_audit_evaluated_at(final_evidence: &Value) -> Option<&str> {
    final_evidence.get("evaluatedAt").and_then(Value::as_str)
}

fn goal_audit_linked_event_seq(final_evidence: &Value) -> Option<i64> {
    final_evidence
        .get("goalLinkedEventSeq")
        .and_then(Value::as_i64)
}

fn latest_goal_linked_event_seq(events: &[GoalEvent]) -> i64 {
    events
        .iter()
        .filter(|event| event.kind == "goal_linked")
        .map(|event| event.seq)
        .max()
        .unwrap_or(0)
}

fn build_goal_rule_audit(snapshot: &GoalSnapshot) -> Value {
    let criteria = split_criteria(&snapshot.goal.completion_criteria);
    let evidence: Vec<Value> = snapshot.evidence.iter().map(|item| json!(item)).collect();
    let active_blockers = active_blocking_evidence(&snapshot.evidence);
    let mut achieved = Vec::new();
    let mut missing = Vec::new();
    let mut optional_missing = Vec::new();
    let mut follow_up_items = Vec::new();
    let mut blockers = Vec::new();
    let mut next_evidence_needed = Vec::new();

    if snapshot.workflow_runs.is_empty()
        && snapshot.tasks.is_empty()
        && snapshot.evidence.is_empty()
    {
        missing.push("No linked workflow run, task, or evidence yet.".to_string());
        next_evidence_needed.push(json!({
            "kind": "workflow_run",
            "reason": "Run a workflow or complete tasks that produce durable evidence.",
        }));
    }

    for task in &snapshot.tasks {
        if task.status == "completed" {
            achieved.push(format!("Task completed: {}", task.content));
        } else {
            missing.push(format!("Task not completed: {}", task.content));
            next_evidence_needed.push(json!({
                "kind": "task",
                "taskId": task.id,
                "reason": format!("Complete task: {}", task.content),
            }));
        }
    }

    for run in &snapshot.workflow_runs {
        let run_label = format!("workflow {} ({})", run.id, run.state.as_str());
        match run.state {
            WorkflowRunState::Completed => {
                achieved.push(format!("{run_label} completed"));
            }
            WorkflowRunState::Failed | WorkflowRunState::Blocked | WorkflowRunState::Cancelled => {
                blockers.push(format!(
                    "{run_label}: {}",
                    run.blocked_reason
                        .as_deref()
                        .unwrap_or("terminal without completion")
                ));
                next_evidence_needed.push(json!({
                    "kind": "repair_workflow",
                    "runId": &run.id,
                    "reason": "Create or complete a repair workflow after this terminal run.",
                }));
            }
            WorkflowRunState::Draft
            | WorkflowRunState::AwaitingApproval
            | WorkflowRunState::Running
            | WorkflowRunState::AwaitingUser
            | WorkflowRunState::Paused
            | WorkflowRunState::Recovering => {
                missing.push(format!("{run_label} is still in progress"));
                next_evidence_needed.push(json!({
                    "kind": "workflow_run",
                    "runId": &run.id,
                    "reason": "Finish or cancel the in-progress workflow before final audit.",
                }));
            }
        }
    }

    for item in &snapshot.evidence {
        match item.relation.as_str() {
            "workflow_completed" => {
                achieved.push(format!("Workflow completed: {}", item.source_id))
            }
            "validation_passed" => achieved.push(format!(
                "Validation passed: {}",
                item.summary.as_deref().unwrap_or(item.source_id.as_str())
            )),
            "task_completed" => achieved.push(format!(
                "Task evidence: {}",
                item.summary.as_deref().unwrap_or(item.source_id.as_str())
            )),
            "diff_snapshot" | "file_changed" | "artifact_created" | "diagnostic_result" => {
                achieved.push(format!("Evidence linked: {}", item.title));
            }
            "worktree_attached" => {
                achieved.push(format!(
                    "Worktree attached: {}",
                    item.summary.as_deref().unwrap_or(item.source_id.as_str())
                ));
            }
            _ => {}
        }
    }

    for blocker in &active_blockers {
        blockers.push(format!(
            "{}: {}",
            blocker.title,
            blocker
                .summary
                .as_deref()
                .unwrap_or(blocker.source_id.as_str())
        ));
        next_evidence_needed.push(json!({
            "kind": "hard_blocker",
            "evidenceId": &blocker.id,
            "relation": &blocker.relation,
            "reason": "Resolve this hard blocker and produce newer passing evidence.",
        }));
    }

    if snapshot.budget.exhausted {
        blockers.push(format!(
            "Goal budget exhausted: {}",
            snapshot.budget.exceeded.join(", ")
        ));
        next_evidence_needed.push(json!({
            "kind": "budget",
            "reason": "Extend the goal budget or reduce scope before creating more workflow runs.",
            "exceeded": snapshot.budget.exceeded.clone(),
        }));
    } else if snapshot.budget.warning {
        achieved.push(format!(
            "Goal budget warning: {}",
            snapshot.budget.warnings.join(", ")
        ));
    }

    let has_strong_positive = snapshot
        .evidence
        .iter()
        .any(goal_evidence_is_strong_positive);
    if !has_strong_positive {
        missing.push(
            "No final workflow completion, passing validation, or completed task evidence yet."
                .to_string(),
        );
        next_evidence_needed.push(json!({
            "kind": "final_verification",
            "reason": "Produce at least one strong completion signal: workflow_completed, validation_passed, or task_completed.",
        }));
    }

    for criterion in &snapshot.criteria {
        match criterion.status {
            GoalCriterionStatus::Satisfied => {
                achieved.push(format!(
                    "{} criterion has supporting evidence: {}",
                    criterion.kind.as_str(),
                    criterion.text
                ));
            }
            GoalCriterionStatus::Missing => {
                if criterion.kind.is_required() {
                    missing.push(format!(
                        "Required criterion lacks sufficient evidence: {}",
                        criterion.text
                    ));
                    next_evidence_needed.push(json!({
                        "kind": "criterion",
                        "criterionId": &criterion.id,
                        "criterionKind": criterion.kind.as_str(),
                        "criterion": &criterion.text,
                        "reason": &criterion.reason,
                    }));
                } else if criterion.kind == GoalCriterionKind::Optional {
                    optional_missing.push(format!(
                        "Optional criterion lacks sufficient evidence: {}",
                        criterion.text
                    ));
                } else {
                    follow_up_items.push(json!({
                        "id": &criterion.id,
                        "text": &criterion.text,
                        "source": "criterion",
                        "reason": &criterion.reason,
                    }));
                }
            }
            GoalCriterionStatus::Blocked => {
                if criterion.kind.is_required() {
                    blockers.push(format!(
                        "Required criterion is blocked: {}",
                        criterion
                            .reason
                            .as_deref()
                            .unwrap_or(criterion.text.as_str())
                    ));
                    next_evidence_needed.push(json!({
                        "kind": "criterion",
                        "criterionId": &criterion.id,
                        "criterionKind": criterion.kind.as_str(),
                        "criterion": &criterion.text,
                        "reason": &criterion.reason,
                    }));
                } else if criterion.kind == GoalCriterionKind::Optional {
                    optional_missing.push(format!(
                        "Optional criterion is blocked: {}",
                        criterion
                            .reason
                            .as_deref()
                            .unwrap_or(criterion.text.as_str())
                    ));
                } else {
                    follow_up_items.push(json!({
                        "id": &criterion.id,
                        "text": &criterion.text,
                        "source": "criterion",
                        "reason": &criterion.reason,
                    }));
                }
            }
        }
    }

    for item in &snapshot.goal.follow_up_items {
        follow_up_items.push(json!(item));
    }

    achieved.sort();
    achieved.dedup();
    missing.sort();
    missing.dedup();
    optional_missing.sort();
    optional_missing.dedup();
    blockers.sort();
    blockers.dedup();
    dedup_json_items(&mut next_evidence_needed);
    dedup_json_items(&mut follow_up_items);

    let required_criteria_passed = snapshot
        .criteria
        .iter()
        .filter(|criterion| criterion.kind.is_required())
        .all(|criterion| criterion.status == GoalCriterionStatus::Satisfied);

    let status = if blockers.is_empty()
        && missing.is_empty()
        && has_strong_positive
        && required_criteria_passed
    {
        "completed"
    } else {
        "blocked"
    };
    let blocked_reason = if status == "completed" {
        Value::Null
    } else if snapshot.budget.exhausted {
        Value::String("goal_budget_exhausted".to_string())
    } else if !blockers.is_empty() {
        Value::String("goal_blocked_by_evidence".to_string())
    } else {
        Value::String("goal_evidence_incomplete".to_string())
    };
    let summary = if status == "completed" {
        format!(
            "Goal completed with {} evidence item(s), {} achieved item(s), and rule gate passed.",
            evidence.len(),
            achieved.len()
        )
    } else {
        format!(
            "Goal is not complete: {} blocker(s), {} missing item(s), {} next evidence item(s).",
            blockers.len(),
            missing.len(),
            next_evidence_needed.len()
        )
    };

    json!({
        "status": status,
        "summary": summary,
        "blockedReason": blocked_reason,
        "objective": &snapshot.goal.objective,
        "goalRevision": snapshot.goal.revision,
        "goalLinkedEventSeq": latest_goal_linked_event_seq(&snapshot.events),
        "auditStale": false,
        "criteria": criteria,
        "criteriaItems": &snapshot.criteria_items,
        "criteriaStatus": &snapshot.criteria,
        "achieved": achieved,
        "missing": missing,
        "optionalMissing": optional_missing,
        "blockers": blockers,
        "evidence": evidence,
        "nextEvidenceNeeded": next_evidence_needed,
        "followUpItems": follow_up_items,
        "closure": {
            "decision": snapshot.goal.closure_decision.map(|decision| decision.as_str()),
            "reason": snapshot.goal.closure_reason.as_deref(),
            "closedAt": snapshot.goal.closed_at.as_deref(),
            "requiresUserAcceptance": snapshot.goal.closure_decision != Some(GoalClosureDecision::AcceptedV1),
        },
        "budget": &snapshot.budget,
        "ruleGate": {
            "status": if blockers.is_empty() && missing.is_empty() && required_criteria_passed { "passed" } else { "blocked" },
            "hardBlockers": active_blockers.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
            "strongEvidence": snapshot.evidence.iter().filter(|item| goal_evidence_is_strong_positive(item)).map(|item| item.id.clone()).collect::<Vec<_>>(),
            "requiredCriteriaPassed": required_criteria_passed,
            "llmAuditor": {
                "status": "skipped",
                "reason": "Phase 2.8 uses deterministic rule gate only; future optional LLM auditor may add rationale after hard blockers pass."
            }
        },
        "remainingRisk": if status == "completed" {
            "Rule gate passed; optional LLM audit is not enabled in this phase."
        } else {
            "More concrete workflow/task/validation evidence is required before completion can be claimed."
        },
    })
}

fn build_goal_evidence_items(links: &[GoalLink], tasks: &[Task]) -> Vec<GoalEvidenceItem> {
    let mut items = Vec::new();
    for link in links {
        if !is_goal_evidence_relation(&link.relation) {
            continue;
        }
        items.push(GoalEvidenceItem {
            id: goal_link_evidence_id(link),
            source_type: link.target_type.clone(),
            source_id: link.target_id.clone(),
            relation: link.relation.clone(),
            title: goal_link_title(link),
            summary: goal_link_summary(link),
            metadata: link.metadata.clone(),
            created_at: link.created_at.clone(),
        });
    }
    for task in tasks {
        if task.status != "completed" {
            continue;
        }
        items.push(GoalEvidenceItem {
            id: format!("task:{}", task.id),
            source_type: "task".to_string(),
            source_id: task.id.to_string(),
            relation: "task_completed".to_string(),
            title: "Task completed".to_string(),
            summary: Some(task.content.clone()),
            metadata: json!({
                "taskId": task.id,
                "status": task.status,
                "activeForm": task.active_form,
                "batchId": task.batch_id,
            }),
            created_at: task.updated_at.clone(),
        });
    }
    items.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    items
}

fn build_goal_criteria_audit(snapshot: &GoalSnapshot) -> Vec<GoalCriterionAudit> {
    let effective_blockers = active_blocking_evidence(&snapshot.evidence);

    snapshot
        .criteria_items
        .iter()
        .map(|item| {
            let criterion_blockers = effective_blockers
                .iter()
                .copied()
                .filter(|evidence| {
                    evidence_goal_criterion_id(evidence)
                        .map(|bound_id| bound_id == item.id)
                        .unwrap_or(true)
                })
                .collect::<Vec<_>>();
            if !criterion_blockers.is_empty() {
                GoalCriterionAudit {
                    id: item.id.clone(),
                    text: item.text.clone(),
                    kind: item.kind,
                    status: GoalCriterionStatus::Blocked,
                    evidence_ids: criterion_blockers
                        .iter()
                        .take(8)
                        .map(|item| item.id.clone())
                        .collect(),
                    reason: Some(
                        "Latest evidence contains a failed or blocked result.".to_string(),
                    ),
                }
            } else {
                let supporting =
                    supporting_evidence_for_criterion(&item.id, &item.text, &snapshot.evidence);
                let has_strong = supporting.iter().any(|item| goal_evidence_is_strong_positive(item));
                if has_strong {
                    GoalCriterionAudit {
                        id: item.id.clone(),
                        text: item.text.clone(),
                        kind: item.kind,
                        status: GoalCriterionStatus::Satisfied,
                        evidence_ids: supporting
                            .iter()
                            .take(8)
                            .map(|item| item.id.clone())
                            .collect(),
                        reason: Some(
                            "Strong completion or validation evidence supports this criterion."
                                .to_string(),
                        ),
                    }
                } else if !supporting.is_empty() {
                    GoalCriterionAudit {
                        id: item.id.clone(),
                        text: item.text.clone(),
                        kind: item.kind,
                        status: GoalCriterionStatus::Missing,
                        evidence_ids: supporting
                            .iter()
                            .take(8)
                            .map(|item| item.id.clone())
                            .collect(),
                        reason: Some(
                            "Implementation evidence exists, but final validation/completion evidence is missing."
                                .to_string(),
                        ),
                    }
                } else {
                    GoalCriterionAudit {
                        id: item.id.clone(),
                        text: item.text.clone(),
                        kind: item.kind,
                        status: GoalCriterionStatus::Missing,
                        evidence_ids: Vec::new(),
                        reason: Some("No supporting evidence has been linked yet.".to_string()),
                    }
                }
            }
        })
        .collect()
}

fn supporting_evidence_for_criterion<'a>(
    criterion_id: &str,
    criterion: &str,
    evidence: &'a [GoalEvidenceItem],
) -> Vec<&'a GoalEvidenceItem> {
    let mut out = Vec::new();
    for item in evidence {
        if let Some(bound_id) = evidence_goal_criterion_id(item) {
            if bound_id == criterion_id {
                out.push(item);
            }
            continue;
        }
        if goal_evidence_is_strong_positive(item) || evidence_matches_criterion(item, criterion) {
            out.push(item);
        }
    }
    out
}

fn evidence_goal_criterion_id(item: &GoalEvidenceItem) -> Option<&str> {
    item.metadata
        .get("goalCriterion")
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .or_else(|| item.metadata.get("goalCriterionId").and_then(Value::as_str))
}

fn evidence_matches_criterion(item: &GoalEvidenceItem, criterion: &str) -> bool {
    if !goal_evidence_is_positive(item) {
        return false;
    }
    let haystack = format!(
        "{} {} {} {} {}",
        item.title,
        item.summary.as_deref().unwrap_or(""),
        item.source_type,
        item.source_id,
        item.relation
    )
    .to_lowercase();
    meaningful_tokens(criterion)
        .iter()
        .any(|token| haystack.contains(token.as_str()))
}

fn meaningful_tokens(text: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "the", "and", "for", "with", "that", "this", "from", "into", "workflow", "evidence",
        "goal", "完成", "标准", "证据",
    ];
    text.split(|ch: char| !ch.is_alphanumeric())
        .map(|part| part.trim().to_lowercase())
        .filter(|part| part.len() >= 3)
        .filter(|part| !STOPWORDS.contains(&part.as_str()))
        .collect()
}

fn active_blocking_evidence(evidence: &[GoalEvidenceItem]) -> Vec<&GoalEvidenceItem> {
    let latest_validation_pass =
        latest_evidence_time(evidence, |item| item.relation == "validation_passed");
    let latest_workflow_repair = latest_evidence_time(evidence, |item| {
        item.relation == "workflow_completed" || item.relation == "validation_passed"
    });
    let latest_domain_quality_pass =
        latest_evidence_time(evidence, |item| item.relation == "domain_quality_passed");
    evidence
        .iter()
        .filter(|item| match item.relation.as_str() {
            "validation_failed" => !latest_validation_pass
                .map(|latest| latest > item.created_at.as_str())
                .unwrap_or(false),
            "workflow_failed" | "workflow_blocked" | "workflow_cancelled" => {
                !latest_workflow_repair
                    .map(|latest| latest > item.created_at.as_str())
                    .unwrap_or(false)
            }
            "review_finding" => review_finding_is_blocking(item),
            "domain_quality_failed" | "domain_quality_blocked" | "domain_quality_needs_user" => {
                !latest_domain_quality_pass
                    .map(|latest| latest > item.created_at.as_str())
                    .unwrap_or(false)
            }
            "domain_quality_check" => {
                domain_quality_check_is_blocking(item)
                    && !latest_domain_quality_pass
                        .map(|latest| latest > item.created_at.as_str())
                        .unwrap_or(false)
            }
            "diagnostic_result" => {
                diagnostic_result_is_blocking(item)
                    && !latest_validation_pass
                        .map(|latest| latest > item.created_at.as_str())
                        .unwrap_or(false)
                    && !diagnostic_result_resolved_by_newer_clean(item, evidence)
            }
            _ => false,
        })
        .collect()
}

fn latest_evidence_time<'a>(
    evidence: &'a [GoalEvidenceItem],
    predicate: impl Fn(&GoalEvidenceItem) -> bool,
) -> Option<&'a str> {
    evidence
        .iter()
        .filter(|item| predicate(item))
        .map(|item| item.created_at.as_str())
        .max()
}

fn review_finding_is_blocking(item: &GoalEvidenceItem) -> bool {
    let severity = metadata_string(&item.metadata, "severity")
        .unwrap_or_default()
        .to_lowercase();
    let status = metadata_string(&item.metadata, "status")
        .unwrap_or_else(|| "open".to_string())
        .to_lowercase();
    let verdict = metadata_string(&item.metadata, "verdict")
        .unwrap_or_default()
        .to_lowercase();
    matches!(severity.as_str(), "p0" | "p1" | "critical" | "high")
        && verdict != "refuted"
        && !matches!(
            status.as_str(),
            "resolved" | "closed" | "fixed" | "dismissed" | "false_positive" | "false-positive"
        )
}

fn domain_quality_check_is_blocking(item: &GoalEvidenceItem) -> bool {
    let severity = metadata_string(&item.metadata, "severity")
        .unwrap_or_default()
        .to_lowercase();
    let status = metadata_string(&item.metadata, "status")
        .unwrap_or_default()
        .to_lowercase();
    matches!(severity.as_str(), "p0" | "p1" | "critical" | "high")
        && matches!(status.as_str(), "failed" | "blocked" | "needs_user")
}

fn diagnostic_result_is_blocking(item: &GoalEvidenceItem) -> bool {
    let severity = metadata_string(&item.metadata, "severity")
        .unwrap_or_default()
        .to_lowercase();
    let status = metadata_string(&item.metadata, "status")
        .unwrap_or_default()
        .to_lowercase();
    let errors = metadata_u64(&item.metadata, "errors").unwrap_or(0);
    errors > 0
        || matches!(severity.as_str(), "error" | "critical" | "high")
        || matches!(status.as_str(), "failed" | "blocked")
}

fn diagnostic_result_is_clean(item: &GoalEvidenceItem) -> bool {
    let status = metadata_string(&item.metadata, "status")
        .unwrap_or_default()
        .to_lowercase();
    let errors = metadata_u64(&item.metadata, "errors").unwrap_or(0);
    item.relation == "diagnostic_result" && status == "passed" && errors == 0
}

fn diagnostic_result_resolved_by_newer_clean(
    item: &GoalEvidenceItem,
    evidence: &[GoalEvidenceItem],
) -> bool {
    evidence.iter().any(|candidate| {
        candidate.created_at > item.created_at
            && diagnostic_result_is_clean(candidate)
            && diagnostic_clean_scope_matches(item, candidate)
    })
}

fn diagnostic_clean_scope_matches(item: &GoalEvidenceItem, clean: &GoalEvidenceItem) -> bool {
    let clean_path = metadata_string(&clean.metadata, "path");
    let Some(clean_path) = clean_path.as_deref() else {
        return true;
    };
    metadata_string(&item.metadata, "path").as_deref() == Some(clean_path)
}

fn goal_worktree_metadata(
    worktree: &crate::worktree::ManagedWorktree,
    run: Option<&WorkflowRun>,
) -> Value {
    json!({
        "worktreeId": worktree.id,
        "runId": run.map(|run| run.id.clone()),
        "kind": run.map(|run| run.kind.clone()),
        "runState": run.map(|run| run.state.as_str().to_string()),
        "reverseWorkflowRunId": worktree.workflow_run_id,
        "purpose": worktree.purpose,
        "state": worktree.state,
        "label": worktree.label,
        "path": worktree.path,
        "pathExists": worktree.path_exists,
        "repoRoot": worktree.repo_root,
        "sourceWorkingDir": worktree.source_working_dir,
        "baseRef": worktree.base_ref,
        "baseBranch": worktree.base_branch,
        "baseSha": worktree.base_sha,
        "gitBranch": worktree.git_branch,
        "dirtySnapshot": worktree.dirty_snapshot,
        "archivedAt": worktree.archived_at,
        "restoredAt": worktree.restored_at,
        "handedOffAt": worktree.handed_off_at,
        "summary": goal_worktree_summary(worktree),
        "source": "managed_worktree",
    })
}

fn goal_worktree_summary(worktree: &crate::worktree::ManagedWorktree) -> String {
    let state = worktree.state.as_str();
    let path_status = if worktree.path_exists {
        "path exists"
    } else {
        "path missing"
    };
    let dirty = worktree
        .dirty_snapshot
        .as_ref()
        .map(|snapshot| {
            if snapshot.clean {
                "clean snapshot".to_string()
            } else {
                format!("{} changed file(s)", snapshot.changed_files)
            }
        })
        .unwrap_or_else(|| "no dirty snapshot".to_string());
    let handoff = if worktree.handed_off_at.is_some() {
        ", handed off"
    } else {
        ""
    };
    format!(
        "{state} at {} ({path_status}; {dirty}{handoff})",
        worktree.path
    )
}

fn dedup_json_items(items: &mut Vec<Value>) {
    let mut seen = Vec::<String>::new();
    items.retain(|item| {
        let key = stable_json(item).unwrap_or_else(|_| item.to_string());
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
}

fn positive_limit(value: Option<i64>) -> Option<i64> {
    value.filter(|limit| *limit > 0)
}

fn ratio(used: i64, limit: Option<i64>) -> Option<f64> {
    limit.map(|limit| used.max(0) as f64 / limit.max(1) as f64)
}

fn collect_budget_state(
    kind: &str,
    ratio: Option<f64>,
    warnings: &mut Vec<String>,
    exceeded: &mut Vec<String>,
) {
    let Some(ratio) = ratio else {
        return;
    };
    if ratio >= 1.0 {
        exceeded.push(kind.to_string());
    } else if ratio >= 0.8 {
        warnings.push(kind.to_string());
    }
}

fn parse_rfc3339_utc(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

fn build_goal_timeline(snapshot: &GoalSnapshot) -> Vec<GoalTimelineItem> {
    let mut items = Vec::new();
    for event in &snapshot.events {
        items.push(GoalTimelineItem {
            id: format!("event:{}", event.id),
            kind: "event".to_string(),
            title: goal_event_title(&event.kind).to_string(),
            summary: Some(event.kind.clone()),
            status: None,
            source_type: Some("goal_event".to_string()),
            source_id: Some(event.id.to_string()),
            metadata: event.payload.clone(),
            created_at: event.created_at.clone(),
        });
    }
    for run in &snapshot.workflow_runs {
        items.push(GoalTimelineItem {
            id: format!("workflow:{}", run.id),
            kind: "workflow".to_string(),
            title: format!("Workflow {}", run.kind),
            summary: run
                .blocked_reason
                .clone()
                .or_else(|| run.origin.as_ref().map(|origin| format!("origin={origin}"))),
            status: Some(run.state.as_str().to_string()),
            source_type: Some("workflow_run".to_string()),
            source_id: Some(run.id.clone()),
            metadata: json!({
                "runId": run.id,
                "kind": run.kind,
                "origin": run.origin,
                "parentRunId": run.parent_run_id,
                "scriptHash": run.script_hash,
            }),
            created_at: run.updated_at.clone(),
        });
    }
    for evidence in &snapshot.evidence {
        items.push(GoalTimelineItem {
            id: format!("evidence:{}", evidence.id),
            kind: "evidence".to_string(),
            title: evidence.title.clone(),
            summary: evidence.summary.clone(),
            status: Some(evidence.relation.clone()),
            source_type: Some(evidence.source_type.clone()),
            source_id: Some(evidence.source_id.clone()),
            metadata: evidence.metadata.clone(),
            created_at: evidence.created_at.clone(),
        });
    }
    items.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    items
}

fn is_goal_evidence_relation(relation: &str) -> bool {
    matches!(
        relation,
        "workflow_completed"
            | "workflow_failed"
            | "workflow_blocked"
            | "workflow_cancelled"
            | "validation_passed"
            | "validation_failed"
            | "validation_completed"
            | "diff_snapshot"
            | "file_changed"
            | "artifact_created"
            | "review_passed"
            | "review_completed"
            | "review_finding"
            | "diagnostic_result"
            | "worktree_attached"
            | "source_cited"
            | "claim_checked"
            | "user_decision"
            | "artifact_reviewed"
            | "data_quality_checked"
            | "citation_audited"
            | "message_draft_approved"
            | "meeting_context_collected"
            | "domain_quality_passed"
            | "domain_quality_failed"
            | "domain_quality_blocked"
            | "domain_quality_needs_user"
            | "domain_quality_check"
    )
}

fn goal_evidence_is_positive(item: &GoalEvidenceItem) -> bool {
    matches!(
        item.relation.as_str(),
        "workflow_completed"
            | "validation_passed"
            | "diff_snapshot"
            | "file_changed"
            | "artifact_created"
            | "worktree_attached"
            | "review_passed"
            | "source_cited"
            | "claim_checked"
            | "user_decision"
            | "artifact_reviewed"
            | "data_quality_checked"
            | "citation_audited"
            | "message_draft_approved"
            | "meeting_context_collected"
            | "domain_quality_passed"
            | "task_completed"
    ) || (item.relation == "diagnostic_result" && !diagnostic_result_is_blocking(item))
}

fn goal_evidence_is_strong_positive(item: &GoalEvidenceItem) -> bool {
    matches!(
        item.relation.as_str(),
        "workflow_completed" | "validation_passed" | "domain_quality_passed" | "task_completed"
    )
}

fn goal_link_evidence_id(link: &GoalLink) -> String {
    format!("{}:{}:{}", link.target_type, link.target_id, link.relation)
}

fn goal_link_title(link: &GoalLink) -> String {
    match link.relation.as_str() {
        "workflow_completed" => "Workflow completed".to_string(),
        "workflow_failed" => "Workflow failed".to_string(),
        "workflow_blocked" => "Workflow blocked".to_string(),
        "workflow_cancelled" => "Workflow cancelled".to_string(),
        "validation_passed" => "Validation passed".to_string(),
        "validation_failed" => "Validation failed".to_string(),
        "validation_completed" => "Validation completed".to_string(),
        "diff_snapshot" => {
            let files = metadata_u64(&link.metadata, "filesChanged").unwrap_or(0);
            format!(
                "Diff snapshot ({files} file{})",
                if files == 1 { "" } else { "s" }
            )
        }
        "file_changed" => format!("File changed: {}", link.target_id),
        "artifact_created" => metadata_string(&link.metadata, "title")
            .unwrap_or_else(|| "Artifact created".to_string()),
        "review_passed" => "Review passed".to_string(),
        "review_completed" => "Review completed".to_string(),
        "review_finding" => "Review finding".to_string(),
        "diagnostic_result" => metadata_string(&link.metadata, "message")
            .or_else(|| metadata_string(&link.metadata, "summary"))
            .unwrap_or_else(|| "Diagnostic result".to_string()),
        "worktree_attached" => metadata_string(&link.metadata, "label")
            .map(|label| format!("Worktree attached: {label}"))
            .unwrap_or_else(|| "Worktree attached".to_string()),
        "source_cited" => {
            metadata_string(&link.metadata, "title").unwrap_or_else(|| "Source cited".to_string())
        }
        "claim_checked" => {
            metadata_string(&link.metadata, "title").unwrap_or_else(|| "Claim checked".to_string())
        }
        "user_decision" => {
            metadata_string(&link.metadata, "title").unwrap_or_else(|| "User decision".to_string())
        }
        "artifact_reviewed" => metadata_string(&link.metadata, "title")
            .unwrap_or_else(|| "Artifact reviewed".to_string()),
        "data_quality_checked" => metadata_string(&link.metadata, "title")
            .unwrap_or_else(|| "Data quality checked".to_string()),
        "citation_audited" => metadata_string(&link.metadata, "title")
            .unwrap_or_else(|| "Citation audited".to_string()),
        "message_draft_approved" => metadata_string(&link.metadata, "title")
            .unwrap_or_else(|| "Message draft approved".to_string()),
        "meeting_context_collected" => metadata_string(&link.metadata, "title")
            .unwrap_or_else(|| "Meeting context collected".to_string()),
        "domain_quality_passed" => "Domain quality passed".to_string(),
        "domain_quality_failed" => "Domain quality failed".to_string(),
        "domain_quality_blocked" => "Domain quality blocked".to_string(),
        "domain_quality_needs_user" => "Domain quality needs user".to_string(),
        "domain_quality_check" => metadata_string(&link.metadata, "title")
            .unwrap_or_else(|| "Domain quality check".to_string()),
        other => other.replace('_', " "),
    }
}

fn goal_link_summary(link: &GoalLink) -> Option<String> {
    metadata_string(&link.metadata, "summary")
        .or_else(|| metadata_string(&link.metadata, "reason"))
        .or_else(|| metadata_string(&link.metadata, "blockedReason"))
        .or_else(|| metadata_string(&link.metadata, "state").map(|state| format!("state={state}")))
        .or_else(|| {
            if link.relation == "diff_snapshot" {
                let files = metadata_u64(&link.metadata, "filesChanged").unwrap_or(0);
                let added = metadata_u64(&link.metadata, "linesAdded").unwrap_or(0);
                let removed = metadata_u64(&link.metadata, "linesRemoved").unwrap_or(0);
                Some(format!("{files} file(s), +{added}/-{removed}"))
            } else {
                None
            }
        })
}

fn goal_artifacts_from_finish_output(output: &Value) -> Vec<Value> {
    let mut artifacts = Vec::new();
    if let Some(items) = output.get("artifacts").and_then(Value::as_array) {
        artifacts.extend(items.iter().cloned());
    }
    if let Some(item) = output.get("artifact") {
        artifacts.push(item.clone());
    }
    artifacts
}

fn artifact_target_id(artifact: &Value) -> Option<String> {
    if let Some(raw) = artifact.as_str() {
        return non_empty(raw).map(str::to_string);
    }
    artifact_string_any(
        artifact,
        &[
            "id",
            "artifactId",
            "artifact_id",
            "path",
            "filePath",
            "file_path",
            "url",
            "href",
        ],
    )
}

fn artifact_title(artifact: &Value, fallback: &str) -> String {
    artifact_string_any(artifact, &["title", "name", "label"])
        .or_else(|| {
            artifact_string_any(artifact, &["path", "filePath", "file_path"]).map(|path| {
                std::path::Path::new(&path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(path.as_str())
                    .to_string()
            })
        })
        .unwrap_or_else(|| fallback.to_string())
}

fn artifact_summary(artifact: &Value) -> Option<String> {
    if let Some(raw) = artifact.as_str() {
        return Some(raw.to_string());
    }
    artifact_string_any(artifact, &["summary", "description", "body", "path", "url"])
}

fn artifact_string_any(artifact: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        artifact
            .get(*key)
            .and_then(Value::as_str)
            .and_then(non_empty)
            .map(str::to_string)
    })
}

fn parse_workflow_tool_json_output(output: &Value) -> Option<Value> {
    match output {
        Value::String(raw) => serde_json::from_str(raw).ok(),
        Value::Object(_) => Some(output.clone()),
        _ => None,
    }
}

fn diagnostic_target_id(run: &WorkflowRun, op: &WorkflowOp, diagnostic: &Value) -> String {
    let path = diagnostic_path(diagnostic);
    let line = diagnostic
        .get("range")
        .and_then(|range| range.get("startLine"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let column = diagnostic
        .get("range")
        .and_then(|range| range.get("startColumn"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let message = diagnostic_message(diagnostic);
    let fingerprint = blake3::hash(
        format!(
            "{}\n{}\n{}\n{}\n{}",
            path,
            line,
            column,
            diagnostic_severity(diagnostic),
            message
        )
        .as_bytes(),
    );
    format!(
        "{}:{}:{}:{}:{}",
        run.id,
        op.op_key,
        path,
        line,
        &fingerprint.to_hex()[..12]
    )
}

fn diagnostic_path(diagnostic: &Value) -> String {
    diagnostic
        .get("path")
        .and_then(Value::as_str)
        .or_else(|| diagnostic.get("uri").and_then(Value::as_str))
        .unwrap_or("<unknown>")
        .to_string()
}

fn diagnostic_severity(diagnostic: &Value) -> String {
    diagnostic
        .get("severity")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_lowercase()
}

fn diagnostic_message(diagnostic: &Value) -> String {
    diagnostic
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("LSP diagnostic")
        .replace('\n', " ")
}

fn lsp_diagnostic_scope_path(input: &Value, output: &Value) -> Option<String> {
    input
        .get("args")
        .and_then(|args| args.get("path"))
        .and_then(Value::as_str)
        .and_then(non_empty)
        .or_else(|| {
            output
                .get("path")
                .and_then(Value::as_str)
                .and_then(non_empty)
        })
        .map(str::to_string)
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn goal_event_title(kind: &str) -> &'static str {
    match kind {
        "goal_created" => "Goal created",
        "goal_state_changed" => "Goal state changed",
        "goal_linked" => "Goal evidence linked",
        "goal_evaluated" => "Goal evaluated",
        "goal_closure_decided" => "Goal closure decided",
        _ => "Goal event",
    }
}

fn metadata_string(metadata: &Value, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn metadata_u64(metadata: &Value, key: &str) -> Option<u64> {
    metadata.get(key).and_then(Value::as_u64)
}

#[derive(Debug, Clone, Default)]
struct GoalDomainSelection {
    domain: Option<String>,
    workflow_template_id: Option<String>,
    workflow_template_version: Option<String>,
    workflow_task_type: Option<String>,
}

fn normalize_goal_text_field(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn normalize_goal_domain_field(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() {
                        ch.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
                .split('_')
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("_")
        })
        .filter(|value| !value.is_empty())
}

fn ensure_goal_column(conn: &Connection, table: &str, column: &str, alter_sql: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let columns = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.iter().any(|name| name == column) {
        conn.execute_batch(alter_sql)?;
    }
    Ok(())
}

fn row_to_goal(row: &rusqlite::Row<'_>) -> rusqlite::Result<Goal> {
    let state: String = row.get(9)?;
    let final_evidence_json: String = row.get(18)?;
    let evaluator_json: String = row.get(20)?;
    let closure_decision_raw: Option<String> = row.get(21)?;
    let follow_up_json: String = row.get(24)?;
    Ok(Goal {
        id: row.get(0)?,
        session_id: row.get(1)?,
        objective: row.get(2)?,
        completion_criteria: row.get(3)?,
        revision: row.get(4)?,
        domain: row.get(5)?,
        workflow_template_id: row.get(6)?,
        workflow_template_version: row.get(7)?,
        workflow_task_type: row.get(8)?,
        state: parse_goal_state_sql(&state)?,
        mode_snapshot: row.get(10)?,
        budget_token_limit: row.get(11)?,
        budget_time_limit_secs: row.get(12)?,
        budget_turn_limit: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
        completed_at: row.get(16)?,
        final_summary: row.get(17)?,
        final_evidence: json_from_sql(&final_evidence_json)?,
        blocked_reason: row.get(19)?,
        last_evaluator_result: json_from_sql(&evaluator_json)?,
        closure_decision: parse_goal_closure_decision_sql(closure_decision_raw)?,
        closure_reason: row.get(22)?,
        closed_at: row.get(23)?,
        follow_up_items: json_vec_from_sql(&follow_up_json)?,
    })
}

fn row_to_goal_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<GoalEvent> {
    let payload_json: String = row.get(4)?;
    Ok(GoalEvent {
        id: row.get(0)?,
        goal_id: row.get(1)?,
        seq: row.get(2)?,
        kind: row.get(3)?,
        payload: json_from_sql(&payload_json)?,
        created_at: row.get(5)?,
    })
}

fn row_to_goal_link(row: &rusqlite::Row<'_>) -> rusqlite::Result<GoalLink> {
    let metadata_json: String = row.get(5)?;
    Ok(GoalLink {
        id: row.get(0)?,
        goal_id: row.get(1)?,
        target_type: row.get(2)?,
        target_id: row.get(3)?,
        relation: row.get(4)?,
        metadata: json_from_sql(&metadata_json)?,
        created_at: row.get(6)?,
    })
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>> {
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn parse_goal_state(value: &str) -> Result<GoalState> {
    GoalState::from_str(value).ok_or_else(|| anyhow!("unknown goal state: {value}"))
}

fn parse_goal_state_sql(value: &str) -> rusqlite::Result<GoalState> {
    GoalState::from_str(value).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown goal state: {value}").into(),
        )
    })
}

fn parse_goal_closure_decision_sql(
    value: Option<String>,
) -> rusqlite::Result<Option<GoalClosureDecision>> {
    value
        .map(|raw| {
            GoalClosureDecision::from_str(&raw).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    format!("unknown goal closure decision: {raw}").into(),
                )
            })
        })
        .transpose()
}

fn json_from_sql(value: &str) -> rusqlite::Result<Value> {
    serde_json::from_str(value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, err.into())
    })
}

fn json_vec_from_sql<T>(value: &str) -> rusqlite::Result<Vec<T>>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, err.into())
    })
}

fn stable_json(value: &Value) -> Result<String> {
    serde_json::to_string(value).map_err(Into::into)
}

fn bounded_payload(payload: Value) -> Result<String> {
    let encoded = stable_json(&payload)?;
    if encoded.len() <= GOAL_EVENT_PAYLOAD_MAX_BYTES {
        return Ok(encoded);
    }
    let preview = crate::truncate_utf8(&encoded, GOAL_EVENT_PAYLOAD_MAX_BYTES);
    stable_json(&json!({
        "truncated": true,
        "preview": preview,
        "originalBytes": encoded.len(),
    }))
}

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use serde_json::{json, Value};

    use super::*;
    use crate::session::NewMessage;
    use crate::workflow::{
        CreateWorkflowRunInput, UpsertWorkflowOpInput, WorkflowEffectClass, WorkflowRunState,
    };

    fn temp_db() -> (tempfile::TempDir, SessionDB) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open(&dir.path().join("sessions.db")).expect("open session db");
        (dir, db)
    }

    fn create_goal_for_session(db: &SessionDB, session_id: &str) -> GoalSnapshot {
        db.create_goal(CreateGoalInput {
            session_id: session_id.to_string(),
            objective: "Ship goal mode".to_string(),
            completion_criteria: "workflow completes with evidence".to_string(),
            domain: None,
            workflow_template_id: None,
            workflow_template_version: None,
            workflow_task_type: None,
            budget_token_limit: None,
            budget_time_limit_secs: None,
            budget_turn_limit: None,
        })
        .expect("create goal")
    }

    fn create_workflow(db: &SessionDB, session_id: &str, goal_id: Option<String>) -> WorkflowRun {
        db.create_workflow_run(CreateWorkflowRunInput {
            session_id: session_id.to_string(),
            kind: "coding.workflow".to_string(),
            execution_mode: "guarded".to_string(),
            script_source: "export default async function main(workflow) {}".to_string(),
            budget: json!({ "max_script_secs": 30, "max_ops": 8 }),
            parent_run_id: None,
            origin: None,
            goal_id,
            goal_criterion_id: None,
            worktree_id: None,
        })
        .expect("create workflow")
    }

    fn insert_managed_worktree(
        db: &SessionDB,
        session_id: &str,
        worktree_id: &str,
        repo_root: &str,
        worktree_path: &str,
    ) {
        let now = now_rfc3339();
        let conn = db.conn.lock().expect("lock session db");
        conn.execute(
            "INSERT INTO managed_worktrees (
                id, session_id, child_session_id, workflow_run_id, purpose, state, label,
                repo_root, source_working_dir, path, base_ref, base_branch, base_sha,
                git_branch, dirty_snapshot_json, created_at, updated_at,
                archived_at, restored_at, handed_off_at
             ) VALUES (
                ?1, ?2, NULL, NULL, 'workflow', 'active', 'Goal worktree',
                ?3, ?3, ?4, 'HEAD', 'main', 'abc123',
                NULL, NULL, ?5, ?5,
                NULL, NULL, NULL
             )",
            params![worktree_id, session_id, repo_root, worktree_path, now],
        )
        .expect("insert managed worktree");
    }

    #[test]
    fn create_goal_rejects_incognito_session() {
        let (_dir, db) = temp_db();
        let session = db
            .create_session_with_project("ha-main", None, Some(true))
            .expect("create incognito session");

        let err = db
            .create_goal(CreateGoalInput {
                session_id: session.id,
                objective: "Do not persist".to_string(),
                completion_criteria: String::new(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect_err("incognito goal must be rejected");
        assert!(err.to_string().contains("incognito"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn goal_runner_schedules_once_per_turn_and_stops_when_paused() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);

        let first = maybe_schedule_goal_continuation(
            &db,
            &session.id,
            "ha-main",
            crate::chat_engine::ChatSource::Desktop,
            Some("turn-1"),
            Some(42),
        )
        .expect("schedule first continuation");
        assert!(first.is_some());

        let duplicate = maybe_schedule_goal_continuation(
            &db,
            &session.id,
            "ha-main",
            crate::chat_engine::ChatSource::Desktop,
            Some("turn-1"),
            Some(42),
        )
        .expect("skip duplicate continuation");
        assert!(duplicate.is_none());

        let events = db.list_goal_events(&goal.goal.id, 100).expect("events");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == "goal_auto_continue_scheduled")
                .count(),
            1
        );

        db.pause_goal(&goal.goal.id).expect("pause goal");
        let paused = maybe_schedule_goal_continuation(
            &db,
            &session.id,
            "ha-main",
            crate::chat_engine::ChatSource::Desktop,
            Some("turn-2"),
            Some(43),
        )
        .expect("paused continuation check");
        assert!(paused.is_none());

        crate::wakeup::purge_for_session(&session.id);
    }

    #[test]
    fn goal_persists_and_updates_domain_workflow_selection() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");

        let goal = db
            .create_goal(CreateGoalInput {
                session_id: session.id,
                objective: "Prepare a sourced brief".to_string(),
                completion_criteria: "Citations and claim checks are complete".to_string(),
                domain: None,
                workflow_template_id: Some("research-brief".to_string()),
                workflow_template_version: None,
                workflow_task_type: Some("technical_research".to_string()),
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal with domain workflow");

        assert_eq!(goal.goal.domain.as_deref(), Some("research"));
        assert_eq!(
            goal.goal.workflow_template_id.as_deref(),
            Some("research-brief")
        );
        assert_eq!(
            goal.goal.workflow_template_version.as_deref(),
            Some("1.0.0")
        );
        assert_eq!(
            goal.goal.workflow_task_type.as_deref(),
            Some("technical_research")
        );

        let renamed = db
            .update_goal(UpdateGoalInput {
                goal_id: goal.goal.id.clone(),
                objective: Some("Prepare a sourced brief with current browser risks".to_string()),
                completion_criteria: None,
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
            })
            .expect("update goal objective without changing domain workflow");
        assert_eq!(
            renamed.goal.workflow_template_id.as_deref(),
            Some("research-brief")
        );
        assert_eq!(
            renamed.goal.workflow_task_type.as_deref(),
            Some("technical_research")
        );

        let updated = db
            .update_goal(UpdateGoalInput {
                goal_id: goal.goal.id.clone(),
                objective: None,
                completion_criteria: None,
                domain: None,
                workflow_template_id: Some("writing-brief".to_string()),
                workflow_template_version: None,
                workflow_task_type: Some("prd".to_string()),
            })
            .expect("update goal domain workflow");
        assert_eq!(updated.goal.domain.as_deref(), Some("writing"));
        assert_eq!(
            updated.goal.workflow_template_id.as_deref(),
            Some("writing-brief")
        );
        assert_eq!(updated.goal.workflow_task_type.as_deref(), Some("prd"));

        let cleared = db
            .update_goal(UpdateGoalInput {
                goal_id: goal.goal.id,
                objective: None,
                completion_criteria: None,
                domain: Some(String::new()),
                workflow_template_id: Some(String::new()),
                workflow_template_version: Some(String::new()),
                workflow_task_type: Some(String::new()),
            })
            .expect("clear goal domain workflow");
        assert!(cleared.goal.domain.is_none());
        assert!(cleared.goal.workflow_template_id.is_none());
        assert!(cleared.goal.workflow_template_version.is_none());
        assert!(cleared.goal.workflow_task_type.is_none());
    }

    #[test]
    fn parses_structured_goal_criteria_kinds() {
        let items = parse_goal_criteria_items(
            "[required] ship durable closure\n[optional] polish copy\n[follow-up] add exports",
        );
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].kind, GoalCriterionKind::Required);
        assert_eq!(items[0].text, "ship durable closure");
        assert_eq!(items[1].kind, GoalCriterionKind::Optional);
        assert_eq!(items[1].text, "polish copy");
        assert_eq!(items[2].kind, GoalCriterionKind::FollowUp);
        assert_eq!(items[2].text, "add exports");
    }

    #[test]
    fn inline_goal_criterion_kind_does_not_leak_to_next_item() {
        let items = parse_goal_criteria_items(
            "[required] ship durable closure\n[optional] polish copy\ncapture final evidence",
        );
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].kind, GoalCriterionKind::Required);
        assert_eq!(items[1].kind, GoalCriterionKind::Optional);
        assert_eq!(items[2].kind, GoalCriterionKind::Required);

        let grouped = parse_goal_criteria_items(
            "[optional]\npolish copy\nadjust labels\n[required]\nfinal audit passes",
        );
        assert_eq!(grouped.len(), 3);
        assert_eq!(grouped[0].kind, GoalCriterionKind::Optional);
        assert_eq!(grouped[1].kind, GoalCriterionKind::Optional);
        assert_eq!(grouped[2].kind, GoalCriterionKind::Required);
    }

    #[test]
    fn parses_goal_criteria_prefix_variants_for_gui_preview_parity() {
        let items = parse_goal_criteria_items(
            "\u{5fc5}\u{987b}\u{ff1a} \u{2610} \u{8dd1}\u{5b8c}\u{9488}\u{5bf9}\u{6027}\u{68c0}\u{67e5}\n\
             1) optional: polish UX copy\n\
             * [follow-up] migrate notes to roadmap",
        );

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].kind, GoalCriterionKind::Required);
        assert_eq!(
            items[0].text,
            "\u{8dd1}\u{5b8c}\u{9488}\u{5bf9}\u{6027}\u{68c0}\u{67e5}"
        );
        assert_eq!(items[1].kind, GoalCriterionKind::Optional);
        assert_eq!(items[1].text, "polish UX copy");
        assert_eq!(items[2].kind, GoalCriterionKind::FollowUp);
        assert_eq!(items[2].text, "migrate notes to roadmap");
    }

    #[test]
    fn updating_goal_bumps_revision_and_clears_closure() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);
        let strict = db
            .close_goal(CloseGoalInput {
                goal_id: goal.goal.id.clone(),
                decision: GoalClosureDecision::NeedsStrictEvidence,
                reason: Some("needs real smoke".to_string()),
                follow_up_items: vec!["manual GUI profile".to_string()],
            })
            .expect("mark strict evidence needed");
        assert_eq!(
            strict.goal.closure_decision,
            Some(GoalClosureDecision::NeedsStrictEvidence)
        );

        let updated = db
            .update_goal(UpdateGoalInput {
                goal_id: goal.goal.id.clone(),
                objective: Some("Ship goal v2 control center".to_string()),
                completion_criteria: Some("[required] closure packet exists".to_string()),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
            })
            .expect("update goal");
        assert_eq!(updated.goal.revision, goal.goal.revision + 1);
        assert!(updated.goal.closure_decision.is_none());
        assert!(updated.goal.closure_reason.is_none());
        assert!(updated.goal.closed_at.is_none());
        assert!(updated.goal.final_summary.is_none());
        assert_eq!(
            updated.criteria.first().map(|criterion| criterion.kind),
            Some(GoalCriterionKind::Required)
        );
    }

    #[test]
    fn review_finding_blocks_goal_only_when_open_and_actionable() {
        let mut item = GoalEvidenceItem {
            id: "review:revf_1".to_string(),
            source_type: "review".to_string(),
            source_id: "revf_1".to_string(),
            relation: "review_finding".to_string(),
            title: "Review finding".to_string(),
            summary: None,
            metadata: json!({
                "severity": "p1",
                "status": "open",
                "verdict": "confirmed",
            }),
            created_at: "2026-07-01T00:00:00Z".to_string(),
        };
        assert!(review_finding_is_blocking(&item));

        item.metadata["status"] = json!("dismissed");
        assert!(!review_finding_is_blocking(&item));

        item.metadata["status"] = json!("false_positive");
        assert!(!review_finding_is_blocking(&item));

        item.metadata["status"] = json!("open");
        item.metadata["verdict"] = json!("refuted");
        assert!(!review_finding_is_blocking(&item));
    }

    #[test]
    fn workflow_creation_auto_links_active_goal() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);

        let run = create_workflow(&db, &session.id, None);
        assert_eq!(run.goal_id.as_deref(), Some(goal.goal.id.as_str()));

        let snapshot = db
            .goal_snapshot(&goal.goal.id, 100)
            .expect("goal snapshot")
            .expect("goal exists");
        assert!(snapshot.links.iter().any(|link| {
            link.target_type == "workflow_run"
                && link.target_id == run.id
                && link.relation == "execution_run"
        }));
    }

    #[test]
    fn workflow_creation_links_specific_goal_criterion() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Ship goal v2".to_string(),
                completion_criteria: "[required] write docs\n[required] pass tests".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");

        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "coding.workflow".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: "export default async function main(workflow) {}".to_string(),
                budget: json!({ "max_script_secs": 30, "max_ops": 8 }),
                parent_run_id: None,
                origin: None,
                goal_id: Some(goal.goal.id.clone()),
                goal_criterion_id: Some("criterion-2".to_string()),
                worktree_id: None,
            })
            .expect("create workflow");
        assert_eq!(run.goal_criterion_id.as_deref(), Some("criterion-2"));
        assert_eq!(run.goal_criterion_text.as_deref(), Some("pass tests"));
        assert_eq!(run.goal_criterion_kind.as_deref(), Some("required"));
        assert_eq!(run.goal_revision, Some(goal.goal.revision));

        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test_start"))
            .expect("start workflow");
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, Some("test_done"))
            .expect("complete workflow");
        let snapshot = db
            .goal_snapshot(&goal.goal.id, 100)
            .expect("goal snapshot")
            .expect("goal exists");
        let criterion_1 = snapshot
            .criteria
            .iter()
            .find(|criterion| criterion.id == "criterion-1")
            .expect("criterion 1");
        let criterion_2 = snapshot
            .criteria
            .iter()
            .find(|criterion| criterion.id == "criterion-2")
            .expect("criterion 2");
        assert_eq!(criterion_1.status, GoalCriterionStatus::Missing);
        assert_eq!(criterion_2.status, GoalCriterionStatus::Satisfied);
        assert!(snapshot.links.iter().any(|link| {
            link.target_type == "workflow_run"
                && link.target_id == run.id
                && link.relation == "workflow_completed"
                && link.metadata["goalCriterion"]["id"] == json!("criterion-2")
        }));
    }

    #[test]
    fn workflow_creation_rejects_invalid_goal_criterion() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Ship goal v2".to_string(),
                completion_criteria: "[required] write docs".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");

        let err = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id,
                kind: "coding.workflow".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: "export default async function main(workflow) {}".to_string(),
                budget: json!({ "max_script_secs": 30, "max_ops": 8 }),
                parent_run_id: None,
                origin: None,
                goal_id: Some(goal.goal.id),
                goal_criterion_id: Some("criterion-99".to_string()),
                worktree_id: None,
            })
            .expect_err("invalid criterion should fail closed");
        assert!(err.to_string().contains("criterion-99"));
    }

    #[test]
    fn workflow_worktree_links_goal_evidence_and_handoff_refreshes_it() {
        let (dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);
        let worktree_id = "wt_goal_evidence";
        let repo_root = dir.path().join("repo");
        let worktree_path = dir.path().join("worktree");
        std::fs::create_dir_all(&repo_root).expect("repo dir");
        std::fs::create_dir_all(&worktree_path).expect("worktree dir");
        let repo_root = repo_root.to_string_lossy().to_string();
        let worktree_path = worktree_path.to_string_lossy().to_string();
        insert_managed_worktree(&db, &session.id, worktree_id, &repo_root, &worktree_path);

        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "coding.workflow".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: "export default async function main(workflow) {}".to_string(),
                budget: json!({ "max_script_secs": 30, "max_ops": 8 }),
                parent_run_id: None,
                origin: None,
                goal_id: Some(goal.goal.id.clone()),
                goal_criterion_id: None,
                worktree_id: Some(worktree_id.to_string()),
            })
            .expect("create workflow with worktree");

        let snapshot = db
            .goal_snapshot(&goal.goal.id, 200)
            .expect("goal snapshot")
            .expect("goal exists");
        let link = snapshot
            .links
            .iter()
            .find(|link| {
                link.target_type == "worktree"
                    && link.target_id == worktree_id
                    && link.relation == "worktree_attached"
            })
            .expect("worktree evidence link");
        assert_eq!(
            link.metadata.get("state").and_then(Value::as_str),
            Some("active")
        );
        assert_eq!(
            link.metadata.get("pathExists").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            link.metadata.get("runId").and_then(Value::as_str),
            Some(run.id.as_str())
        );
        assert_eq!(
            link.metadata
                .get("reverseWorkflowRunId")
                .and_then(Value::as_str),
            Some(run.id.as_str())
        );
        assert!(snapshot
            .evidence
            .iter()
            .any(|item| item.relation == "worktree_attached"));

        db.handoff_managed_worktree(worktree_id)
            .expect("handoff worktree");
        let refreshed = db
            .goal_snapshot(&goal.goal.id, 200)
            .expect("refreshed goal snapshot")
            .expect("goal exists");
        let refreshed_link = refreshed
            .links
            .iter()
            .find(|link| {
                link.target_type == "worktree"
                    && link.target_id == worktree_id
                    && link.relation == "worktree_attached"
            })
            .expect("refreshed worktree evidence link");
        assert_eq!(
            refreshed_link.metadata.get("state").and_then(Value::as_str),
            Some("handoff")
        );
        assert!(refreshed_link
            .metadata
            .get("handedOffAt")
            .and_then(Value::as_str)
            .is_some());
        assert!(refreshed_link
            .metadata
            .get("summary")
            .and_then(Value::as_str)
            .is_some_and(|summary| summary.contains("handed off")));
    }

    #[test]
    fn workflow_completion_auto_evaluates_goal() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);
        let run = create_workflow(&db, &session.id, Some(goal.goal.id.clone()));

        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test_start"))
            .expect("start run");
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, Some("test_done"))
            .expect("complete run");

        let snapshot = db
            .goal_snapshot(&goal.goal.id, 200)
            .expect("goal snapshot")
            .expect("goal exists");
        assert_eq!(snapshot.goal.state, GoalState::Completed);
        assert_eq!(
            snapshot
                .goal
                .final_evidence
                .get("status")
                .and_then(Value::as_str),
            Some("completed")
        );
        assert!(snapshot.links.iter().any(|link| {
            link.target_type == "workflow_run"
                && link.target_id == run.id
                && link.relation == "workflow_completed"
        }));
    }

    #[test]
    fn final_audit_uses_full_goal_link_seq_after_long_timeline() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);
        let run = create_workflow(&db, &session.id, Some(goal.goal.id.clone()));

        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test_start"))
            .expect("start run");
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, Some("test_done"))
            .expect("complete run");

        let latest_goal_linked_seq = db
            .latest_goal_linked_event_marker(&goal.goal.id)
            .expect("latest linked marker")
            .seq;
        assert!(latest_goal_linked_seq > 0);

        for index in 0..250 {
            db.append_goal_event(
                &goal.goal.id,
                "audit_noise",
                json!({ "index": index, "note": "long timeline non-evidence event" }),
            )
            .expect("append noise event");
        }

        let evaluated = db.evaluate_goal(&goal.goal.id).expect("re-evaluate goal");
        assert_eq!(evaluated.goal.state, GoalState::Completed);
        assert_eq!(
            evaluated
                .goal
                .final_evidence
                .get("goalLinkedEventSeq")
                .and_then(Value::as_i64),
            Some(latest_goal_linked_seq)
        );

        let truncated = db
            .goal_snapshot(&goal.goal.id, 1)
            .expect("truncated snapshot")
            .expect("goal exists");
        assert!(!truncated.audit_stale);

        db.close_goal(CloseGoalInput {
            goal_id: goal.goal.id.clone(),
            decision: GoalClosureDecision::AcceptedV1,
            reason: Some("fresh audit after long timeline".to_string()),
            follow_up_items: Vec::new(),
        })
        .expect("accept closure after long timeline audit");
    }

    #[test]
    fn completed_goal_stays_visible_until_user_accepts_closure() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);
        let run = create_workflow(&db, &session.id, Some(goal.goal.id.clone()));

        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test_start"))
            .expect("start run");
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, Some("test_done"))
            .expect("complete run");

        let visible = db
            .active_goal_for_session(&session.id)
            .expect("active goal query")
            .expect("completed unaccepted goal remains visible");
        assert_eq!(visible.goal.state, GoalState::Completed);
        assert!(visible.goal.closure_decision.is_none());

        let closed = db
            .close_goal(CloseGoalInput {
                goal_id: goal.goal.id.clone(),
                decision: GoalClosureDecision::AcceptedV1,
                reason: Some("accepted deterministic evidence".to_string()),
                follow_up_items: vec![
                    "manual screenshot smoke".to_string(),
                    " manual   SCREENSHOT smoke ".to_string(),
                    "roadmap export".to_string(),
                ],
            })
            .expect("accept closure");
        assert_eq!(
            closed.goal.closure_decision,
            Some(GoalClosureDecision::AcceptedV1)
        );
        assert_eq!(
            closed
                .goal
                .final_evidence
                .get("closure")
                .and_then(|closure| closure.get("decision"))
                .and_then(Value::as_str),
            Some("accepted_v1")
        );
        assert_eq!(closed.goal.follow_up_items.len(), 2);
        assert_eq!(
            closed.goal.follow_up_items[0].text,
            "manual screenshot smoke"
        );
        assert_eq!(closed.goal.follow_up_items[1].text, "roadmap export");
        assert!(closed.goal.closed_at.is_some());
        assert!(db
            .active_goal_for_session(&session.id)
            .expect("active goal after closure")
            .is_none());
    }

    #[test]
    fn accepted_closure_cannot_be_reopened_by_later_close_decision() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);
        let run = create_workflow(&db, &session.id, Some(goal.goal.id.clone()));

        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test_start"))
            .expect("start run");
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, Some("test_done"))
            .expect("complete run");

        db.close_goal(CloseGoalInput {
            goal_id: goal.goal.id.clone(),
            decision: GoalClosureDecision::AcceptedV1,
            reason: Some("accepted deterministic evidence".to_string()),
            follow_up_items: Vec::new(),
        })
        .expect("accept closure");

        let err = db
            .close_goal(CloseGoalInput {
                goal_id: goal.goal.id.clone(),
                decision: GoalClosureDecision::NeedsStrictEvidence,
                reason: Some("late strict request".to_string()),
                follow_up_items: Vec::new(),
            })
            .expect_err("accepted closure is sealed");
        assert!(err.to_string().contains("already closed"));

        let closed = db
            .goal_snapshot(&goal.goal.id, 200)
            .expect("closed snapshot")
            .expect("goal still exists");
        assert_eq!(closed.goal.state, GoalState::Completed);
        assert_eq!(
            closed.goal.closure_decision,
            Some(GoalClosureDecision::AcceptedV1)
        );
    }

    #[test]
    fn clear_goal_records_cancelled_closure_decision() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);

        let cleared = db.clear_goal(&goal.goal.id).expect("clear goal");
        assert_eq!(cleared.goal.state, GoalState::Cancelled);
        assert_eq!(
            cleared.goal.closure_decision,
            Some(GoalClosureDecision::Cancelled)
        );
        assert_eq!(
            cleared.goal.closure_reason.as_deref(),
            Some("clear_requested")
        );
        assert!(cleared.goal.closed_at.is_some());
        assert_eq!(
            cleared
                .goal
                .final_evidence
                .get("closure")
                .and_then(|closure| closure.get("decision"))
                .and_then(Value::as_str),
            Some("cancelled")
        );
        assert!(db
            .active_goal_for_session(&session.id)
            .expect("active goal after clear")
            .is_none());
    }

    #[test]
    fn append_goal_follow_up_dedups_and_records_owner_event() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);

        let updated = db
            .append_goal_follow_up(AppendGoalFollowUpInput {
                goal_id: goal.goal.id.clone(),
                items: vec![
                    "manual browser smoke".to_string(),
                    " manual   browser SMOKE ".to_string(),
                    "export roadmap card".to_string(),
                ],
                source: Some("composer".to_string()),
            })
            .expect("append follow-up");

        assert_eq!(updated.goal.follow_up_items.len(), 2);
        assert_eq!(updated.goal.follow_up_items[0].text, "manual browser smoke");
        assert_eq!(
            updated.goal.follow_up_items[0].source.as_deref(),
            Some("composer")
        );
        assert_eq!(updated.goal.follow_up_items[1].text, "export roadmap card");
        assert!(updated
            .events
            .iter()
            .any(|event| event.kind == "goal_follow_up_added"));

        db.clear_goal(&goal.goal.id).expect("clear goal");
        let err = db
            .append_goal_follow_up(AppendGoalFollowUpInput {
                goal_id: goal.goal.id.clone(),
                items: vec!["late follow-up".to_string()],
                source: Some("composer".to_string()),
            })
            .expect_err("closed goal rejects follow-up append");
        assert!(err.to_string().contains("already closed"));
    }

    #[test]
    fn accept_closure_requires_completed_current_audit() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);

        let err = db
            .close_goal(CloseGoalInput {
                goal_id: goal.goal.id.clone(),
                decision: GoalClosureDecision::AcceptedV1,
                reason: Some("premature accept".to_string()),
                follow_up_items: Vec::new(),
            })
            .expect_err("accepted closure requires final audit");
        assert!(err.to_string().contains("current final audit"));
    }

    #[test]
    fn completed_pending_closure_goal_auto_binds_new_workflow_and_stales_audit() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);
        let run = create_workflow(&db, &session.id, Some(goal.goal.id.clone()));

        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test_start"))
            .expect("start run");
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, Some("test_done"))
            .expect("complete run");

        let visible = db
            .active_goal_for_session(&session.id)
            .expect("active goal query")
            .expect("completed unaccepted goal remains visible");
        assert_eq!(visible.goal.state, GoalState::Completed);
        assert!(!visible.audit_stale);

        let follow_up_run = create_workflow(&db, &session.id, None);
        assert_eq!(
            follow_up_run.goal_id.as_deref(),
            Some(goal.goal.id.as_str())
        );
        let stale = db
            .goal_snapshot(&goal.goal.id, 200)
            .expect("goal snapshot")
            .expect("goal exists");
        let baseline_seq = stale
            .goal
            .final_evidence
            .get("goalLinkedEventSeq")
            .and_then(Value::as_i64)
            .expect("audit baseline seq");
        assert!(latest_goal_linked_event_seq(&stale.events) > baseline_seq);
        assert!(stale.audit_stale);

        let err = db
            .close_goal(CloseGoalInput {
                goal_id: goal.goal.id.clone(),
                decision: GoalClosureDecision::AcceptedV1,
                reason: Some("accept despite new workflow".to_string()),
                follow_up_items: Vec::new(),
            })
            .expect_err("newer evidence should require re-audit");
        assert!(err.to_string().contains("newer goal evidence"));

        let updated = db
            .update_goal(UpdateGoalInput {
                goal_id: goal.goal.id.clone(),
                objective: None,
                completion_criteria: Some(
                    "workflow completes with evidence\n[required] follow-up workflow is resolved"
                        .to_string(),
                ),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
            })
            .expect("pending completed goal can be updated");
        assert_eq!(updated.goal.state, GoalState::Active);
        assert!(updated.goal.final_summary.is_none());
        assert!(updated.goal.closure_decision.is_none());
    }

    #[test]
    fn strict_closure_reopens_goal_as_blocked() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);
        let run = create_workflow(&db, &session.id, Some(goal.goal.id.clone()));

        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test_start"))
            .expect("start run");
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, Some("test_done"))
            .expect("complete run");

        let strict = db
            .close_goal(CloseGoalInput {
                goal_id: goal.goal.id.clone(),
                decision: GoalClosureDecision::NeedsStrictEvidence,
                reason: Some("real connector read-back required".to_string()),
                follow_up_items: Vec::new(),
            })
            .expect("request strict evidence");
        assert_eq!(strict.goal.state, GoalState::Blocked);
        assert_eq!(
            strict.goal.closure_decision,
            Some(GoalClosureDecision::NeedsStrictEvidence)
        );
        assert_eq!(
            strict
                .goal
                .final_evidence
                .get("closure")
                .and_then(|closure| closure.get("decision"))
                .and_then(Value::as_str),
            Some("needs_strict_evidence")
        );
        assert_eq!(
            strict.goal.blocked_reason.as_deref(),
            Some("real connector read-back required")
        );
        assert!(db
            .active_goal_for_session(&session.id)
            .expect("active strict goal")
            .is_some());
    }

    #[test]
    fn workflow_validation_op_links_goal_evidence() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);
        let run = create_workflow(&db, &session.id, Some(goal.goal.id.clone()));

        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test_start"))
            .expect("start run");
        db.upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: run.id.clone(),
            op_key: "validate-1".to_string(),
            op_type: "validate".to_string(),
            effect_class: WorkflowEffectClass::NonIdempotent,
            input: json!({ "commands": ["pnpm typecheck"] }),
            child_handle: None,
        })
        .expect("start validation op");
        db.complete_workflow_op(
            &run.id,
            "validate-1",
            json!({
                "ok": true,
                "summary": "typecheck passed",
                "results": [{ "ok": true, "command": "pnpm typecheck" }],
            }),
        )
        .expect("complete validation");

        let snapshot = db
            .goal_snapshot(&goal.goal.id, 200)
            .expect("goal snapshot")
            .expect("goal exists");
        assert!(snapshot.links.iter().any(|link| {
            link.target_type == "validation"
                && link.target_id == format!("{}:validate-1", run.id)
                && link.relation == "validation_passed"
        }));
        assert!(snapshot
            .evidence
            .iter()
            .any(|item| item.relation == "validation_passed"));
        assert_eq!(
            snapshot.criteria.first().map(|criterion| criterion.status),
            Some(GoalCriterionStatus::Satisfied)
        );
    }

    #[test]
    fn failed_validation_blocks_goal_criteria() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);
        let run = create_workflow(&db, &session.id, Some(goal.goal.id.clone()));

        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test_start"))
            .expect("start run");
        db.upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: run.id.clone(),
            op_key: "validate-1".to_string(),
            op_type: "validate".to_string(),
            effect_class: WorkflowEffectClass::NonIdempotent,
            input: json!({ "commands": ["pnpm test"] }),
            child_handle: None,
        })
        .expect("start validation op");
        db.complete_workflow_op(
            &run.id,
            "validate-1",
            json!({
                "ok": false,
                "summary": "1/1 validation command(s) failed",
                "results": [{ "ok": false, "command": "pnpm test" }],
            }),
        )
        .expect("complete validation");

        let snapshot = db.evaluate_goal(&goal.goal.id).expect("evaluate goal");
        assert_eq!(snapshot.goal.state, GoalState::Blocked);
        assert!(snapshot
            .evidence
            .iter()
            .any(|item| item.relation == "validation_failed"));
        assert_eq!(
            snapshot.criteria.first().map(|criterion| criterion.status),
            Some(GoalCriterionStatus::Blocked)
        );
    }

    #[test]
    fn failed_validation_remains_blocker_after_workflow_completed() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);
        let run = create_workflow(&db, &session.id, Some(goal.goal.id.clone()));

        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test_start"))
            .expect("start run");
        db.upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: run.id.clone(),
            op_key: "validate-1".to_string(),
            op_type: "validate".to_string(),
            effect_class: WorkflowEffectClass::NonIdempotent,
            input: json!({ "commands": ["pnpm test"] }),
            child_handle: None,
        })
        .expect("start validation op");
        db.complete_workflow_op(
            &run.id,
            "validate-1",
            json!({
                "ok": false,
                "summary": "tests failed",
                "results": [{ "ok": false, "command": "pnpm test" }],
            }),
        )
        .expect("complete failed validation");
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, Some("test_done"))
            .expect("complete run");

        let snapshot = db
            .goal_snapshot(&goal.goal.id, 200)
            .expect("goal snapshot")
            .expect("goal exists");
        assert_eq!(snapshot.goal.state, GoalState::Blocked);
        assert_eq!(
            snapshot
                .goal
                .final_evidence
                .get("status")
                .and_then(Value::as_str),
            Some("blocked")
        );
        let blockers = snapshot
            .goal
            .final_evidence
            .get("blockers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(blockers.iter().any(|item| {
            item.as_str()
                .is_some_and(|text| text.contains("Validation failed"))
        }));
    }

    #[test]
    fn workflow_diff_op_links_diff_and_file_evidence() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);
        let run = create_workflow(&db, &session.id, Some(goal.goal.id.clone()));

        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test_start"))
            .expect("start run");
        db.upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: run.id.clone(),
            op_key: "diff-1".to_string(),
            op_type: "diff".to_string(),
            effect_class: WorkflowEffectClass::Pure,
            input: json!({}),
            child_handle: None,
        })
        .expect("start diff op");
        db.complete_workflow_op(
            &run.id,
            "diff-1",
            json!({
                "kind": "file_changes",
                "changes": [{
                    "path": "src/lib.rs",
                    "action": "edit",
                    "linesAdded": 3,
                    "linesRemoved": 1,
                    "language": "rust",
                    "truncated": false,
                }],
            }),
        )
        .expect("complete diff");

        let snapshot = db
            .goal_snapshot(&goal.goal.id, 200)
            .expect("goal snapshot")
            .expect("goal exists");
        assert!(snapshot.links.iter().any(|link| {
            link.target_type == "diff"
                && link.target_id == format!("{}:diff-1", run.id)
                && link.relation == "diff_snapshot"
        }));
        assert!(snapshot.links.iter().any(|link| {
            link.target_type == "file"
                && link.target_id == "src/lib.rs"
                && link.relation == "file_changed"
        }));
        assert!(snapshot
            .evidence
            .iter()
            .any(|item| item.relation == "file_changed"));
    }

    #[test]
    fn workflow_finish_op_links_artifact_evidence() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);
        let run = create_workflow(&db, &session.id, Some(goal.goal.id.clone()));

        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test_start"))
            .expect("start run");
        db.upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: run.id.clone(),
            op_key: "finish-1".to_string(),
            op_type: "finish".to_string(),
            effect_class: WorkflowEffectClass::Pure,
            input: json!({}),
            child_handle: None,
        })
        .expect("start finish op");
        db.complete_workflow_op(
            &run.id,
            "finish-1",
            json!({
                "summary": "created release notes",
                "artifacts": [{
                    "path": "docs/release-notes.md",
                    "title": "Release notes",
                    "kind": "markdown",
                    "summary": "Draft release notes for review",
                    "hash": "abc123",
                }],
            }),
        )
        .expect("complete finish");

        let snapshot = db
            .goal_snapshot(&goal.goal.id, 200)
            .expect("goal snapshot")
            .expect("goal exists");
        assert!(snapshot.links.iter().any(|link| {
            link.target_type == "artifact"
                && link.target_id == "docs/release-notes.md"
                && link.relation == "artifact_created"
                && link.metadata.get("title").and_then(Value::as_str) == Some("Release notes")
        }));
        assert!(snapshot
            .evidence
            .iter()
            .any(|item| { item.relation == "artifact_created" && item.title == "Release notes" }));
    }

    #[test]
    fn workflow_lsp_diagnostics_link_goal_blocker_until_clean_result() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);
        let run = create_workflow(&db, &session.id, Some(goal.goal.id.clone()));

        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test_start"))
            .expect("start run");
        db.upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: run.id.clone(),
            op_key: "lsp-1".to_string(),
            op_type: "tool:lsp".to_string(),
            effect_class: WorkflowEffectClass::Pure,
            input: json!({
                "name": "lsp",
                "args": { "action": "diagnostics" },
            }),
            child_handle: None,
        })
        .expect("start lsp op");
        db.complete_workflow_op(
            &run.id,
            "lsp-1",
            Value::String(
                json!({
                    "action": "diagnostics",
                    "workspaceRoot": "/repo",
                    "diagnostics": [{
                        "uri": "file:///repo/src/lib.rs",
                        "path": "/repo/src/lib.rs",
                        "range": {
                            "startLine": 12,
                            "startColumn": 5,
                            "endLine": 12,
                            "endColumn": 16,
                        },
                        "severity": "error",
                        "code": "E0308",
                        "source": "rust-analyzer",
                        "message": "mismatched types",
                    }],
                })
                .to_string(),
            ),
        )
        .expect("complete lsp diagnostics");

        let blocked = db.evaluate_goal(&goal.goal.id).expect("evaluate goal");
        assert_eq!(blocked.goal.state, GoalState::Blocked);
        assert!(blocked.links.iter().any(|link| {
            link.target_type == "diagnostic"
                && link.relation == "diagnostic_result"
                && link.metadata.get("severity").and_then(Value::as_str) == Some("error")
        }));
        let blockers = blocked
            .goal
            .final_evidence
            .get("blockers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(blockers.iter().any(|item| {
            item.as_str()
                .is_some_and(|text| text.contains("mismatched types"))
        }));

        db.upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: run.id.clone(),
            op_key: "lsp-2".to_string(),
            op_type: "tool:lsp".to_string(),
            effect_class: WorkflowEffectClass::Pure,
            input: json!({
                "name": "lsp",
                "args": { "action": "sync_file", "path": "/repo/src/other.rs" },
            }),
            child_handle: None,
        })
        .expect("start unrelated clean lsp op");
        db.complete_workflow_op(
            &run.id,
            "lsp-2",
            Value::String(
                json!({
                    "action": "sync_file",
                    "workspaceRoot": "/repo",
                    "path": "/repo/src/other.rs",
                    "diagnostics": [],
                })
                .to_string(),
            ),
        )
        .expect("complete unrelated clean diagnostics");

        let still_blocked = db
            .evaluate_goal(&goal.goal.id)
            .expect("re-evaluate goal after unrelated clean diagnostics");
        let blockers = still_blocked
            .goal
            .final_evidence
            .get("blockers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(blockers.iter().any(|item| {
            item.as_str()
                .is_some_and(|text| text.contains("mismatched types"))
        }));

        db.upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: run.id.clone(),
            op_key: "lsp-3".to_string(),
            op_type: "tool:lsp".to_string(),
            effect_class: WorkflowEffectClass::Pure,
            input: json!({
                "name": "lsp",
                "args": { "action": "diagnostics" },
            }),
            child_handle: None,
        })
        .expect("start workspace clean lsp op");
        db.complete_workflow_op(
            &run.id,
            "lsp-3",
            Value::String(
                json!({
                    "action": "diagnostics",
                    "workspaceRoot": "/repo",
                    "diagnostics": [],
                })
                .to_string(),
            ),
        )
        .expect("complete workspace clean diagnostics");

        let clean = db
            .evaluate_goal(&goal.goal.id)
            .expect("re-evaluate goal after workspace clean diagnostics");
        let blockers = clean
            .goal
            .final_evidence
            .get("blockers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(!blockers.iter().any(|item| {
            item.as_str()
                .is_some_and(|text| text.contains("mismatched types"))
        }));
    }

    #[test]
    fn diff_only_evaluate_requires_final_verification() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);
        let run = create_workflow(&db, &session.id, Some(goal.goal.id.clone()));

        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test_start"))
            .expect("start run");
        db.upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: run.id.clone(),
            op_key: "diff-1".to_string(),
            op_type: "diff".to_string(),
            effect_class: WorkflowEffectClass::Pure,
            input: json!({}),
            child_handle: None,
        })
        .expect("start diff op");
        db.complete_workflow_op(
            &run.id,
            "diff-1",
            json!({
                "kind": "file_changes",
                "changes": [{
                    "path": "src/lib.rs",
                    "action": "edit",
                    "linesAdded": 3,
                    "linesRemoved": 1,
                    "language": "rust",
                    "truncated": false,
                }],
            }),
        )
        .expect("complete diff");

        let snapshot = db.evaluate_goal(&goal.goal.id).expect("evaluate goal");
        assert_eq!(snapshot.goal.state, GoalState::Blocked);
        assert_eq!(
            snapshot
                .goal
                .final_evidence
                .get("status")
                .and_then(Value::as_str),
            Some("blocked")
        );
        let next = snapshot
            .goal
            .final_evidence
            .get("nextEvidenceNeeded")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(next.iter().any(|item| {
            item.get("kind").and_then(Value::as_str) == Some("final_verification")
        }));
    }

    #[test]
    fn exhausted_turn_budget_rejects_new_workflow() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Stay within turn budget".to_string(),
                completion_criteria: "no extra workflow after budget".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: Some(1),
            })
            .expect("create goal");
        db.append_message(&session.id, &NewMessage::user("consume one turn"))
            .expect("append message");

        let err = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "coding.workflow".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: "export default async function main(workflow) {}".to_string(),
                budget: json!({ "max_script_secs": 30, "max_ops": 8 }),
                parent_run_id: None,
                origin: None,
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .expect_err("exhausted goal budget should reject new workflow");
        assert!(err.to_string().contains("budget exhausted"));

        let snapshot = db
            .goal_snapshot(&goal.goal.id, 200)
            .expect("goal snapshot")
            .expect("goal exists");
        assert!(snapshot.budget.exhausted);
        assert!(snapshot.budget.exceeded.iter().any(|kind| kind == "turns"));
        assert!(snapshot.events.iter().any(|event| {
            event.kind == "budget_warning"
                && event.payload.get("level").and_then(Value::as_str) == Some("exhausted")
        }));
    }

    #[test]
    fn goal_evaluate_blocks_without_evidence() {
        let (_dir, db) = temp_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = create_goal_for_session(&db, &session.id);

        let snapshot = db.evaluate_goal(&goal.goal.id).expect("evaluate goal");

        assert_eq!(snapshot.goal.state, GoalState::Blocked);
        assert_eq!(
            snapshot
                .goal
                .final_evidence
                .get("status")
                .and_then(Value::as_str),
            Some("blocked")
        );
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn emit_goal<T: Serialize>(name: &str, payload: &T) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(name, json!(payload));
    }
}

fn emit_goal_event(name: &str, event: &GoalEvent) {
    emit_goal(name, event);
}

fn emit_goal_link(name: &str, link: &GoalLink) {
    emit_goal(name, link);
}
