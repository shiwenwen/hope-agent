//! Session-scoped Goal control plane.
//!
//! A Goal is the durable "what are we trying to finish?" object above
//! workflow/task execution. It lives in `sessions.db` so it shares the same
//! lifecycle as sessions, workflow runs, and tasks.

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::session::{MessageRole, SessionDB, Task};
use crate::workflow::{WorkflowOp, WorkflowOpState, WorkflowRun, WorkflowRunState};

const GOAL_EVENT_PAYLOAD_MAX_BYTES: usize = 64 * 1024;
const GOAL_EVIDENCE_MAX_FILE_LINKS: usize = 50;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    pub id: String,
    pub session_id: String,
    pub objective: String,
    pub completion_criteria: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalCriterionAudit {
    pub id: String,
    pub text: String,
    pub status: GoalCriterionStatus,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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
pub struct CreateGoalInput {
    pub session_id: String,
    pub objective: String,
    #[serde(default)]
    pub completion_criteria: String,
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
}

pub(crate) fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS goals (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            objective TEXT NOT NULL,
            completion_criteria TEXT NOT NULL DEFAULT '',
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
        CREATE INDEX IF NOT EXISTS idx_goal_links_goal
            ON goal_links(goal_id);
        CREATE INDEX IF NOT EXISTS idx_goal_links_target
            ON goal_links(target_type, target_id);",
    )?;
    Ok(())
}

impl SessionDB {
    pub fn create_goal(&self, input: CreateGoalInput) -> Result<GoalSnapshot> {
        let objective = input.objective.trim();
        if objective.is_empty() {
            return Err(anyhow!("goal objective must not be empty"));
        }
        let criteria = input.completion_criteria.trim();
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
                 WHERE session_id = ?1 AND state IN ('active','paused','evaluating','blocked')
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
                id, session_id, objective, completion_criteria, state, mode_snapshot,
                budget_token_limit, budget_time_limit_secs, budget_turn_limit,
                created_at, updated_at, final_evidence_json, last_evaluator_result_json
            ) VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?6, ?7, ?8, ?9, ?9, '{}', '{}')",
            params![
                id,
                input.session_id,
                objective,
                criteria,
                mode,
                input.budget_token_limit,
                input.budget_time_limit_secs,
                input.budget_turn_limit,
                now
            ],
        )?;
        drop(conn);
        let _ = self.append_goal_event(
            &id,
            "goal_created",
            json!({
                "objective": objective,
                "completionCriteria": criteria,
                "modeSnapshot": mode,
            }),
        )?;
        let snapshot = self
            .goal_snapshot(&id, 100)?
            .ok_or_else(|| anyhow!("goal {} was not persisted", id))?;
        emit_goal("goal:created", &snapshot.goal);
        Ok(snapshot)
    }

    pub fn update_goal(&self, input: UpdateGoalInput) -> Result<GoalSnapshot> {
        let objective = input.objective.as_deref().map(str::trim);
        if objective.is_some_and(str::is_empty) {
            return Err(anyhow!("goal objective must not be empty"));
        }
        let completion_criteria = input.completion_criteria.as_deref().map(str::trim);
        if objective.is_none() && completion_criteria.is_none() {
            return self
                .goal_snapshot(&input.goal_id, 100)?
                .ok_or_else(|| anyhow!("goal {} not found", input.goal_id));
        }

        let now = now_rfc3339();
        let (previous_objective, previous_criteria, previous_state) = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let current: Option<(String, String, String)> = conn
                .query_row(
                    "SELECT objective, completion_criteria, state FROM goals WHERE id = ?1",
                    params![input.goal_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let (previous_objective, previous_criteria, state) =
                current.ok_or_else(|| anyhow!("goal {} not found", input.goal_id))?;
            let previous_state = parse_goal_state(&state)?;
            if previous_state.is_terminal() {
                return Err(anyhow!("goal {} is terminal", input.goal_id));
            }
            let next_objective = objective.unwrap_or(previous_objective.trim());
            let next_criteria = completion_criteria.unwrap_or(previous_criteria.trim());
            if next_objective == previous_objective.trim()
                && next_criteria == previous_criteria.trim()
            {
                drop(conn);
                return self
                    .goal_snapshot(&input.goal_id, 100)?
                    .ok_or_else(|| anyhow!("goal {} not found", input.goal_id));
            }
            let next_state = match previous_state {
                GoalState::Blocked | GoalState::Evaluating => GoalState::Active,
                other => other,
            };
            conn.execute(
                "UPDATE goals
                    SET objective = COALESCE(?1, objective),
                        completion_criteria = COALESCE(?2, completion_criteria),
                        state = ?3,
                        updated_at = ?4,
                        final_summary = NULL,
                        final_evidence_json = '{}',
                        blocked_reason = NULL,
                        last_evaluator_result_json = '{}'
                 WHERE id = ?5",
                params![
                    objective,
                    completion_criteria,
                    next_state.as_str(),
                    now,
                    input.goal_id
                ],
            )?;
            (previous_objective, previous_criteria, previous_state)
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
                    "state": previous_state.as_str(),
                },
                "next": {
                    "objective": snapshot.goal.objective,
                    "completionCriteria": snapshot.goal.completion_criteria,
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
            "SELECT id, session_id, objective, completion_criteria, state, mode_snapshot,
                    budget_token_limit, budget_time_limit_secs, budget_turn_limit,
                    created_at, updated_at, completed_at, final_summary, final_evidence_json,
                    blocked_reason, last_evaluator_result_json
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
                 WHERE session_id = ?1 AND state IN ('active','paused','evaluating','blocked')
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
             WHERE session_id = ?1 AND state IN ('active','paused','evaluating','blocked')
             ORDER BY updated_at DESC
             LIMIT 1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
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
        let mut snapshot = GoalSnapshot {
            goal,
            links,
            events,
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
        self.transition_goal(goal_id, GoalState::Cancelled, Some("clear_requested"))
    }

    pub fn evaluate_goal(&self, goal_id: &str) -> Result<GoalSnapshot> {
        let _ = self.transition_goal(goal_id, GoalState::Evaluating, Some("evaluate_requested"))?;
        let snapshot = self
            .goal_snapshot(goal_id, 200)?
            .ok_or_else(|| anyhow!("goal {} not found", goal_id))?;
        let audit = self.build_goal_audit(&snapshot)?;
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

    pub fn transition_goal(
        &self,
        goal_id: &str,
        next: GoalState,
        reason: Option<&str>,
    ) -> Result<GoalSnapshot> {
        let now = now_rfc3339();
        let previous = {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            let current: Option<String> = conn
                .query_row(
                    "SELECT state FROM goals WHERE id = ?1",
                    params![goal_id],
                    |row| row.get(0),
                )
                .optional()?;
            let current = current.ok_or_else(|| anyhow!("goal {} not found", goal_id))?;
            let previous = parse_goal_state(&current)?;
            if !previous.can_transition_to(next) {
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
        if goal.state.is_terminal() {
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
            _ => {}
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

    fn build_goal_audit(&self, snapshot: &GoalSnapshot) -> Result<Value> {
        Ok(build_goal_rule_audit(snapshot))
    }
}

fn split_criteria(raw: &str) -> Vec<String> {
    raw.lines()
        .flat_map(|line| line.split(';'))
        .map(|line| {
            line.trim()
                .trim_start_matches('-')
                .trim_start_matches('*')
                .trim()
                .to_string()
        })
        .filter(|line| !line.is_empty())
        .collect()
}

fn build_goal_rule_audit(snapshot: &GoalSnapshot) -> Value {
    let criteria = split_criteria(&snapshot.goal.completion_criteria);
    let evidence: Vec<Value> = snapshot.evidence.iter().map(|item| json!(item)).collect();
    let active_blockers = active_blocking_evidence(&snapshot.evidence);
    let mut achieved = Vec::new();
    let mut missing = Vec::new();
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
                    "Criterion has supporting evidence: {}",
                    criterion.text
                ));
            }
            GoalCriterionStatus::Missing => {
                missing.push(format!(
                    "Criterion lacks sufficient evidence: {}",
                    criterion.text
                ));
                next_evidence_needed.push(json!({
                    "kind": "criterion",
                    "criterionId": &criterion.id,
                    "criterion": &criterion.text,
                    "reason": &criterion.reason,
                }));
            }
            GoalCriterionStatus::Blocked => {
                blockers.push(format!(
                    "Criterion is blocked: {}",
                    criterion
                        .reason
                        .as_deref()
                        .unwrap_or(criterion.text.as_str())
                ));
            }
        }
    }

    achieved.sort();
    achieved.dedup();
    missing.sort();
    missing.dedup();
    blockers.sort();
    blockers.dedup();
    dedup_json_items(&mut next_evidence_needed);

    let status = if blockers.is_empty()
        && missing.is_empty()
        && has_strong_positive
        && snapshot
            .criteria
            .iter()
            .all(|criterion| criterion.status == GoalCriterionStatus::Satisfied)
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
        "criteria": criteria,
        "criteriaStatus": &snapshot.criteria,
        "achieved": achieved,
        "missing": missing,
        "blockers": blockers,
        "evidence": evidence,
        "nextEvidenceNeeded": next_evidence_needed,
        "budget": &snapshot.budget,
        "ruleGate": {
            "status": if blockers.is_empty() && missing.is_empty() { "passed" } else { "blocked" },
            "hardBlockers": active_blockers.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
            "strongEvidence": snapshot.evidence.iter().filter(|item| goal_evidence_is_strong_positive(item)).map(|item| item.id.clone()).collect::<Vec<_>>(),
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
    let criteria = split_criteria(&snapshot.goal.completion_criteria);
    let effective_blockers = active_blocking_evidence(&snapshot.evidence);

    criteria
        .into_iter()
        .enumerate()
        .map(|(index, text)| {
            if !effective_blockers.is_empty() {
                GoalCriterionAudit {
                    id: format!("criterion-{}", index + 1),
                    text,
                    status: GoalCriterionStatus::Blocked,
                    evidence_ids: effective_blockers
                        .iter()
                        .take(8)
                        .map(|item| item.id.clone())
                        .collect(),
                    reason: Some(
                        "Latest evidence contains a failed or blocked result.".to_string(),
                    ),
                }
            } else {
                let supporting = supporting_evidence_for_criterion(&text, &snapshot.evidence);
                let has_strong = supporting.iter().any(|item| goal_evidence_is_strong_positive(item));
                if has_strong {
                    GoalCriterionAudit {
                        id: format!("criterion-{}", index + 1),
                        text,
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
                        id: format!("criterion-{}", index + 1),
                        text,
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
                        id: format!("criterion-{}", index + 1),
                        text,
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
    criterion: &str,
    evidence: &'a [GoalEvidenceItem],
) -> Vec<&'a GoalEvidenceItem> {
    let mut out = Vec::new();
    for item in evidence {
        if goal_evidence_is_strong_positive(item) || evidence_matches_criterion(item, criterion) {
            out.push(item);
        }
    }
    out
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
            | "review_passed"
            | "diagnostic_result"
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
    )
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
        "artifact_created" => "Artifact created".to_string(),
        "review_passed" => "Review passed".to_string(),
        "review_completed" => "Review completed".to_string(),
        "review_finding" => "Review finding".to_string(),
        "diagnostic_result" => "Diagnostic result".to_string(),
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

fn goal_event_title(kind: &str) -> &'static str {
    match kind {
        "goal_created" => "Goal created",
        "goal_state_changed" => "Goal state changed",
        "goal_linked" => "Goal evidence linked",
        "goal_evaluated" => "Goal evaluated",
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

fn row_to_goal(row: &rusqlite::Row<'_>) -> rusqlite::Result<Goal> {
    let state: String = row.get(4)?;
    let final_evidence_json: String = row.get(13)?;
    let evaluator_json: String = row.get(15)?;
    Ok(Goal {
        id: row.get(0)?,
        session_id: row.get(1)?,
        objective: row.get(2)?,
        completion_criteria: row.get(3)?,
        state: parse_goal_state_sql(&state)?,
        mode_snapshot: row.get(5)?,
        budget_token_limit: row.get(6)?,
        budget_time_limit_secs: row.get(7)?,
        budget_turn_limit: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        completed_at: row.get(11)?,
        final_summary: row.get(12)?,
        final_evidence: json_from_sql(&final_evidence_json)?,
        blocked_reason: row.get(14)?,
        last_evaluator_result: json_from_sql(&evaluator_json)?,
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

fn json_from_sql(value: &str) -> rusqlite::Result<Value> {
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
            worktree_id: None,
        })
        .expect("create workflow")
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
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect_err("incognito goal must be rejected");
        assert!(err.to_string().contains("incognito"));
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
