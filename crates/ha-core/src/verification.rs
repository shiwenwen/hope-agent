//! Smart verification selector and durable verification runs.
//!
//! Phase 3.4 turns "what should I run to prove this change?" into a
//! first-class control-plane object. The selector is deterministic and
//! conservative: it recommends the smallest relevant checks from the current
//! git diff and project rules, then only executes low-risk generated commands.

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::session::{
    effective_working_dir_for_meta, load_session_git_diff, SessionDB, WorkspaceGitDiff,
    WorkspaceGitFileChange,
};
use crate::util::now_rfc3339;

const VERIFICATION_EVENT_PAYLOAD_MAX_BYTES: usize = 64 * 1024;
const VERIFICATION_OUTPUT_PREVIEW_CHARS: usize = 32 * 1024;
const DEFAULT_VERIFICATION_TIMEOUT_SECS: u64 = 120;
const MAX_VERIFICATION_STEPS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationRunState {
    Planned,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl VerificationRunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Planned,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStepState {
    Pending,
    Running,
    Passed,
    Failed,
    Skipped,
    TimedOut,
}

impl VerificationStepState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::TimedOut => "timed_out",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "passed" => Self::Passed,
            "failed" => Self::Failed,
            "skipped" => Self::Skipped,
            "timed_out" => Self::TimedOut,
            _ => Self::Pending,
        }
    }

    pub fn is_failure(self) -> bool {
        matches!(self, Self::Failed | Self::TimedOut)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationRisk {
    Low,
    Medium,
    High,
}

impl VerificationRisk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "high" => Self::High,
            "medium" => Self::Medium,
            _ => Self::Low,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationRun {
    pub id: String,
    pub session_id: String,
    pub scope: String,
    pub state: VerificationRunState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    pub summary: String,
    pub stats: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationStep {
    pub id: String,
    pub run_id: String,
    pub session_id: String,
    pub seq: i64,
    pub command: String,
    pub cwd: String,
    pub title: String,
    pub reason: String,
    pub category: String,
    pub risk: VerificationRisk,
    pub auto_run: bool,
    pub state: VerificationStepState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationEvent {
    pub id: i64,
    pub run_id: String,
    pub seq: i64,
    pub kind: String,
    pub payload: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationRunSnapshot {
    pub run: VerificationRun,
    pub steps: Vec<VerificationStep>,
    #[serde(default)]
    pub events: Vec<VerificationEvent>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanVerificationInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_commands: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub focus_paths: Vec<String>,
}

#[derive(Debug, Clone)]
struct SelectedVerificationStep {
    command: String,
    cwd: String,
    title: String,
    reason: String,
    category: String,
    risk: VerificationRisk,
    auto_run: bool,
    timeout_secs: u64,
}

#[derive(Debug, Clone)]
struct SelectionContext {
    workspace_root: PathBuf,
    repo_root: Option<PathBuf>,
    changed_files: Vec<ChangedFile>,
    policy: PolicyHints,
}

#[derive(Debug, Clone)]
struct ChangedFile {
    path: PathBuf,
    rel_path: String,
    language: String,
}

#[derive(Debug, Clone, Default)]
struct PolicyHints {
    full_suite_gated: bool,
}

pub(crate) fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS verification_runs (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            scope TEXT NOT NULL,
            state TEXT NOT NULL,
            goal_id TEXT,
            summary TEXT NOT NULL DEFAULT '',
            stats_json TEXT NOT NULL DEFAULT '{}',
            error TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            completed_at TEXT,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (goal_id) REFERENCES goals(id) ON DELETE SET NULL
        );

        CREATE TABLE IF NOT EXISTS verification_steps (
            id TEXT PRIMARY KEY,
            run_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            command TEXT NOT NULL,
            cwd TEXT NOT NULL,
            title TEXT NOT NULL,
            reason TEXT NOT NULL,
            category TEXT NOT NULL,
            risk TEXT NOT NULL,
            auto_run INTEGER NOT NULL,
            state TEXT NOT NULL,
            exit_code INTEGER,
            output_preview TEXT,
            duration_ms INTEGER,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            started_at TEXT,
            completed_at TEXT,
            FOREIGN KEY (run_id) REFERENCES verification_runs(id) ON DELETE CASCADE,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            UNIQUE(run_id, seq)
        );

        CREATE TABLE IF NOT EXISTS verification_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (run_id) REFERENCES verification_runs(id) ON DELETE CASCADE,
            UNIQUE(run_id, seq)
        );

        CREATE INDEX IF NOT EXISTS idx_verification_runs_session_updated
            ON verification_runs(session_id, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_verification_runs_goal
            ON verification_runs(goal_id, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_verification_steps_run
            ON verification_steps(run_id, seq);
        CREATE INDEX IF NOT EXISTS idx_verification_steps_session
            ON verification_steps(session_id, updated_at DESC);

        UPDATE verification_runs
           SET state = 'failed',
               error = COALESCE(error, 'Interrupted before verification completed'),
               summary = CASE
                   WHEN summary = '' THEN 'Verification interrupted before completion.'
                   ELSE summary
               END,
               updated_at = datetime('now')
         WHERE state = 'running';

        UPDATE verification_steps
           SET state = 'failed',
               output_preview = COALESCE(output_preview, 'Interrupted before verification completed'),
               updated_at = datetime('now'),
               completed_at = datetime('now')
         WHERE state = 'running';",
    )?;
    Ok(())
}

impl SessionDB {
    pub fn create_verification_run(
        &self,
        input: &PlanVerificationInput,
        session_id: &str,
        initial_state: VerificationRunState,
    ) -> Result<VerificationRun> {
        let meta = self
            .get_session(session_id)?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        if meta.incognito {
            bail!("Cannot create durable verification run for incognito session {session_id}");
        }
        if effective_working_dir_for_meta(&meta).is_none() {
            bail!("session {session_id} has no working directory");
        }

        let scope = input.scope.as_deref().unwrap_or("local");
        if scope != "local" {
            bail!("verification scope '{scope}' is not supported yet; use 'local'");
        }

        let goal_id = match input.goal_id.as_deref() {
            Some(goal_id) => {
                let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
                let goal_session_id: Option<String> = conn
                    .query_row(
                        "SELECT session_id FROM goals WHERE id = ?1",
                        params![goal_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                let goal_session_id =
                    goal_session_id.ok_or_else(|| anyhow!("goal not found: {goal_id}"))?;
                if goal_session_id != session_id {
                    bail!(
                        "goal {} belongs to session {}; expected {}",
                        goal_id,
                        goal_session_id,
                        session_id
                    );
                }
                Some(goal_id.to_string())
            }
            None => self.active_goal_id_for_session(session_id)?,
        };

        let now = now_rfc3339();
        let id = format!("ver_{}", uuid::Uuid::new_v4().simple());
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "INSERT INTO verification_runs (
                    id, session_id, scope, state, goal_id, summary, stats_json,
                    created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, '', '{}', ?6, ?6)",
                params![id, session_id, scope, initial_state.as_str(), goal_id, now],
            )?;
        }
        let run = self
            .get_verification_run(&id)?
            .ok_or_else(|| anyhow!("verification run {} was not persisted", id))?;
        let _ = self.append_verification_event(
            &run.id,
            "verification_created",
            json!({
                "sessionId": session_id,
                "scope": scope,
                "state": initial_state.as_str(),
                "goalId": goal_id,
            }),
        );
        emit_verification_run("verification:created", &run);
        Ok(run)
    }

    pub fn get_verification_run(&self, run_id: &str) -> Result<Option<VerificationRun>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, session_id, scope, state, goal_id, summary, stats_json,
                    error, created_at, updated_at, completed_at
             FROM verification_runs WHERE id = ?1",
            params![run_id],
            row_to_verification_run,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_verification_runs_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<VerificationRun>> {
        let limit = limit.clamp(1, 200) as i64;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, scope, state, goal_id, summary, stats_json,
                    error, created_at, updated_at, completed_at
             FROM verification_runs
             WHERE session_id = ?1
             ORDER BY updated_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit], row_to_verification_run)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn verification_run_snapshot(
        &self,
        run_id: &str,
        event_limit: usize,
    ) -> Result<Option<VerificationRunSnapshot>> {
        let Some(run) = self.get_verification_run(run_id)? else {
            return Ok(None);
        };
        let steps = self.list_verification_steps_for_run(run_id)?;
        let events = self.list_verification_events(run_id, event_limit)?;
        Ok(Some(VerificationRunSnapshot { run, steps, events }))
    }

    pub fn list_verification_steps_for_run(&self, run_id: &str) -> Result<Vec<VerificationStep>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, run_id, session_id, seq, command, cwd, title, reason, category,
                    risk, auto_run, state, exit_code, output_preview, duration_ms,
                    created_at, updated_at, started_at, completed_at
             FROM verification_steps
             WHERE run_id = ?1
             ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![run_id], row_to_verification_step)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn insert_verification_steps(
        &self,
        run: &VerificationRun,
        steps: &[SelectedVerificationStep],
    ) -> Result<Vec<VerificationStep>> {
        let now = now_rfc3339();
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            for (idx, step) in steps.iter().enumerate() {
                let id = format!("vers_{}", uuid::Uuid::new_v4().simple());
                conn.execute(
                    "INSERT INTO verification_steps (
                        id, run_id, session_id, seq, command, cwd, title, reason, category,
                        risk, auto_run, state, created_at, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'pending', ?12, ?12)",
                    params![
                        id,
                        run.id,
                        run.session_id,
                        (idx + 1) as i64,
                        step.command,
                        step.cwd,
                        step.title,
                        step.reason,
                        step.category,
                        step.risk.as_str(),
                        if step.auto_run { 1 } else { 0 },
                        now,
                    ],
                )?;
            }
        }
        let inserted = self.list_verification_steps_for_run(&run.id)?;
        for step in &inserted {
            let _ = self.append_verification_event(
                &run.id,
                "step_selected",
                json!({
                    "stepId": step.id,
                    "command": step.command,
                    "category": step.category,
                    "risk": step.risk,
                    "autoRun": step.auto_run,
                }),
            );
            emit_verification_step("verification:step_updated", step);
        }
        Ok(inserted)
    }

    fn update_verification_step_started(&self, step_id: &str) -> Result<VerificationStep> {
        let now = now_rfc3339();
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE verification_steps
                 SET state = 'running', updated_at = ?1, started_at = ?1
                 WHERE id = ?2",
                params![now, step_id],
            )?;
        }
        let step = self
            .get_verification_step(step_id)?
            .ok_or_else(|| anyhow!("verification step not found after start: {step_id}"))?;
        let _ = self.append_verification_event(
            &step.run_id,
            "step_started",
            json!({ "stepId": step.id, "command": step.command }),
        );
        emit_verification_step("verification:step_updated", &step);
        Ok(step)
    }

    fn update_verification_step_completed(
        &self,
        step_id: &str,
        state: VerificationStepState,
        exit_code: Option<i64>,
        output_preview: Option<String>,
        duration_ms: Option<i64>,
    ) -> Result<VerificationStep> {
        let now = now_rfc3339();
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE verification_steps
                 SET state = ?1,
                     exit_code = ?2,
                     output_preview = ?3,
                     duration_ms = ?4,
                     updated_at = ?5,
                     completed_at = ?5
                 WHERE id = ?6",
                params![
                    state.as_str(),
                    exit_code,
                    output_preview,
                    duration_ms,
                    now,
                    step_id
                ],
            )?;
        }
        let step = self
            .get_verification_step(step_id)?
            .ok_or_else(|| anyhow!("verification step not found after completion: {step_id}"))?;
        let _ = self.append_verification_event(
            &step.run_id,
            "step_completed",
            json!({
                "stepId": step.id,
                "state": step.state,
                "exitCode": step.exit_code,
                "durationMs": step.duration_ms,
            }),
        );
        emit_verification_step("verification:step_updated", &step);
        Ok(step)
    }

    fn get_verification_step(&self, step_id: &str) -> Result<Option<VerificationStep>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, run_id, session_id, seq, command, cwd, title, reason, category,
                    risk, auto_run, state, exit_code, output_preview, duration_ms,
                    created_at, updated_at, started_at, completed_at
             FROM verification_steps WHERE id = ?1",
            params![step_id],
            row_to_verification_step,
        )
        .optional()
        .map_err(Into::into)
    }

    fn complete_verification_run(
        &self,
        run_id: &str,
        summary: &str,
        stats: Value,
        failed: bool,
    ) -> Result<VerificationRunSnapshot> {
        let now = now_rfc3339();
        let stats_json = stable_json(&stats)?;
        let state = if failed {
            VerificationRunState::Failed
        } else {
            VerificationRunState::Completed
        };
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE verification_runs
                 SET state = ?1,
                     summary = ?2,
                     stats_json = ?3,
                     updated_at = ?4,
                     completed_at = ?4
                 WHERE id = ?5",
                params![state.as_str(), summary, stats_json, now, run_id],
            )?;
        }
        let snapshot = self
            .verification_run_snapshot(run_id, 100)?
            .ok_or_else(|| anyhow!("verification run {} not found after completion", run_id))?;
        let _ = self.append_verification_event(
            run_id,
            "verification_completed",
            json!({
                "state": snapshot.run.state,
                "summary": snapshot.run.summary,
                "stats": snapshot.run.stats,
            }),
        );
        self.link_verification_goal_evidence(&snapshot)?;
        emit_verification_run("verification:updated", &snapshot.run);
        Ok(snapshot)
    }

    fn finalize_verification_plan(
        &self,
        run_id: &str,
        summary: &str,
        stats: Value,
    ) -> Result<VerificationRunSnapshot> {
        let now = now_rfc3339();
        let stats_json = stable_json(&stats)?;
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE verification_runs
                 SET state = 'planned',
                     summary = ?1,
                     stats_json = ?2,
                     updated_at = ?3,
                     completed_at = NULL
                 WHERE id = ?4",
                params![summary, stats_json, now, run_id],
            )?;
        }
        let snapshot = self
            .verification_run_snapshot(run_id, 100)?
            .ok_or_else(|| anyhow!("verification run {} not found after planning", run_id))?;
        let _ = self.append_verification_event(
            run_id,
            "verification_planned",
            json!({
                "state": snapshot.run.state,
                "summary": snapshot.run.summary,
                "stats": snapshot.run.stats,
            }),
        );
        self.link_verification_goal_evidence(&snapshot)?;
        emit_verification_run("verification:updated", &snapshot.run);
        Ok(snapshot)
    }

    fn fail_verification_run(&self, run_id: &str, error: &str) -> Result<VerificationRunSnapshot> {
        let now = now_rfc3339();
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE verification_runs
                 SET state = 'failed', error = ?1, summary = ?1, updated_at = ?2, completed_at = ?2
                 WHERE id = ?3",
                params![error, now, run_id],
            )?;
        }
        let _ = self.append_verification_event(
            run_id,
            "verification_failed",
            json!({ "error": error }),
        );
        let snapshot = self
            .verification_run_snapshot(run_id, 100)?
            .ok_or_else(|| anyhow!("verification run {} not found after failure", run_id))?;
        self.link_verification_goal_evidence(&snapshot)?;
        emit_verification_run("verification:updated", &snapshot.run);
        Ok(snapshot)
    }

    fn link_verification_goal_evidence(&self, snapshot: &VerificationRunSnapshot) -> Result<()> {
        let Some(goal_id) = snapshot.run.goal_id.as_deref() else {
            return Ok(());
        };
        let failed = snapshot
            .steps
            .iter()
            .filter(|step| step.state.is_failure())
            .count();
        let passed = snapshot
            .steps
            .iter()
            .filter(|step| step.state == VerificationStepState::Passed)
            .count();
        let skipped = snapshot
            .steps
            .iter()
            .filter(|step| step.state == VerificationStepState::Skipped)
            .count();
        let run_failed = snapshot.run.state == VerificationRunState::Failed;
        let relation = if run_failed || failed > 0 {
            "validation_failed"
        } else if passed > 0 {
            "validation_passed"
        } else {
            "validation_completed"
        };
        let _ = self.link_goal_target(
            goal_id,
            "verification",
            &snapshot.run.id,
            relation,
            json!({
                "runId": snapshot.run.id,
                "summary": snapshot.run.summary,
                "state": snapshot.run.state,
                "passed": passed,
                "failed": failed,
                "skipped": skipped,
                "stats": snapshot.run.stats,
                "commands": snapshot.steps.iter().map(|step| {
                    json!({
                        "command": step.command,
                        "state": step.state,
                        "exitCode": step.exit_code,
                        "risk": step.risk,
                        "autoRun": step.auto_run,
                    })
                }).collect::<Vec<_>>(),
                "completedAt": snapshot.run.completed_at,
                "source": "smart_verification",
            }),
        );
        let _ = self.evaluate_goal(goal_id);
        Ok(())
    }

    pub fn append_verification_event(
        &self,
        run_id: &str,
        kind: &str,
        payload: Value,
    ) -> Result<VerificationEvent> {
        let payload_json = bounded_payload(payload)?;
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM verification_events WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO verification_events (run_id, seq, kind, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run_id, seq, kind, payload_json, now],
        )?;
        let id = conn.last_insert_rowid();
        let event = VerificationEvent {
            id,
            run_id: run_id.to_string(),
            seq,
            kind: kind.to_string(),
            payload: serde_json::from_str(&payload_json)?,
            created_at: now,
        };
        drop(conn);
        emit_verification_event("verification:event", &event);
        Ok(event)
    }

    pub fn list_verification_events(
        &self,
        run_id: &str,
        limit: usize,
    ) -> Result<Vec<VerificationEvent>> {
        let limit = limit.clamp(1, 500) as i64;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, run_id, seq, kind, payload_json, created_at
             FROM verification_events
             WHERE run_id = ?1
             ORDER BY seq DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![run_id, limit], row_to_verification_event)?;
        let mut events = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        events.sort_by_key(|event| event.seq);
        Ok(events)
    }
}

pub async fn plan_verification_for_session(
    db: Arc<SessionDB>,
    session_id: String,
    input: PlanVerificationInput,
) -> Result<VerificationRunSnapshot> {
    let run = db.create_verification_run(&input, &session_id, VerificationRunState::Planned)?;
    let selected = match select_verification_for_session(db.clone(), &session_id, &input).await {
        Ok(selected) => selected,
        Err(err) => return db.fail_verification_run(&run.id, &err.to_string()),
    };
    let steps = db.insert_verification_steps(&run, &selected)?;
    let stats = verification_plan_stats(&steps, None, &input.focus_paths);
    let summary = verification_plan_summary(&stats);
    let snapshot = db.finalize_verification_plan(&run.id, &summary, stats)?;
    Ok(VerificationRunSnapshot {
        run: snapshot.run,
        steps: snapshot.steps,
        events: snapshot.events,
    })
}

pub async fn run_verification_for_session(
    db: Arc<SessionDB>,
    session_id: String,
    input: PlanVerificationInput,
) -> Result<VerificationRunSnapshot> {
    let run = db.create_verification_run(&input, &session_id, VerificationRunState::Running)?;
    let selected = match select_verification_for_session(db.clone(), &session_id, &input).await {
        Ok(selected) => selected,
        Err(err) => return db.fail_verification_run(&run.id, &err.to_string()),
    };
    let inserted = db.insert_verification_steps(&run, &selected)?;
    let snapshot = db
        .verification_run_snapshot(&run.id, 100)?
        .ok_or_else(|| anyhow!("verification run {} not found after start", run.id))?;
    let bg_db = db.clone();
    let bg_run_id = run.id.clone();
    let focus_paths = normalize_focus_paths(&input.focus_paths);
    tokio::spawn(async move {
        if let Err(err) = execute_verification_steps(
            bg_db.clone(),
            bg_run_id.clone(),
            selected,
            inserted,
            focus_paths,
        )
        .await
        {
            let _ = bg_db.fail_verification_run(&bg_run_id, &err.to_string());
            app_warn!(
                "verification",
                "run_failed",
                "Smart verification run {} failed: {}",
                bg_run_id,
                err
            );
        }
    });
    Ok(snapshot)
}

async fn execute_verification_steps(
    db: Arc<SessionDB>,
    run_id: String,
    selected: Vec<SelectedVerificationStep>,
    inserted: Vec<VerificationStep>,
    focus_paths: Vec<String>,
) -> Result<()> {
    let selected_by_command = selected
        .into_iter()
        .map(|step| (step.command.clone(), step))
        .collect::<BTreeMap<_, _>>();
    for step in inserted {
        let Some(selected_step) = selected_by_command.get(&step.command) else {
            continue;
        };
        if !step.auto_run {
            let _ = db.update_verification_step_completed(
                &step.id,
                VerificationStepState::Skipped,
                None,
                Some(format!(
                    "Skipped by policy: `{}` is a gated suggestion. Run it explicitly only if the user asks for broader validation.",
                    step.command
                )),
                Some(0),
            )?;
            continue;
        }
        db.update_verification_step_started(&step.id)?;
        let result = run_verification_command(selected_step).await;
        match result {
            Ok(result) => {
                db.update_verification_step_completed(
                    &step.id,
                    result.state,
                    result.exit_code,
                    Some(result.output_preview),
                    Some(result.duration_ms),
                )?;
            }
            Err(err) => {
                db.update_verification_step_completed(
                    &step.id,
                    VerificationStepState::Failed,
                    Some(-1),
                    Some(
                        crate::truncate_utf8(&err.to_string(), VERIFICATION_OUTPUT_PREVIEW_CHARS)
                            .to_string(),
                    ),
                    None,
                )?;
            }
        }
    }
    let steps = db.list_verification_steps_for_run(&run_id)?;
    let failed = steps.iter().any(|step| step.state.is_failure());
    let stats = verification_plan_stats(&steps, Some(failed), &focus_paths);
    let summary = verification_run_summary(&stats);
    db.complete_verification_run(&run_id, &summary, stats, failed)?;
    Ok(())
}

async fn select_verification_for_session(
    db: Arc<SessionDB>,
    session_id: &str,
    input: &PlanVerificationInput,
) -> Result<Vec<SelectedVerificationStep>> {
    let ctx = build_selection_context(db, session_id, input).await?;
    let mut selected = select_verification_steps(&ctx);
    let max = input
        .max_commands
        .unwrap_or(MAX_VERIFICATION_STEPS)
        .clamp(1, MAX_VERIFICATION_STEPS);
    if selected.len() > max {
        selected.truncate(max);
    }
    Ok(selected)
}

async fn build_selection_context(
    db: Arc<SessionDB>,
    session_id: &str,
    input: &PlanVerificationInput,
) -> Result<SelectionContext> {
    let focus = FocusFilter::from_paths(&input.focus_paths);
    let meta = db
        .get_session(session_id)?
        .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
    let workspace_root = effective_working_dir_for_meta(&meta)
        .ok_or_else(|| anyhow!("session {session_id} has no working directory"))?;
    let workspace_root = PathBuf::from(workspace_root)
        .canonicalize()
        .context("resolve workspace root")?;
    let diff = {
        let db = db.clone();
        let sid = session_id.to_string();
        tokio::task::spawn_blocking(move || load_session_git_diff(&db, &sid)).await??
    };
    let repo_root = repo_root_for_path(&workspace_root);
    let policy = read_policy_hints(&workspace_root);
    let mut changed_files = changed_files_from_diff(diff, &workspace_root, repo_root.as_deref());
    if focus.is_active() {
        changed_files.retain(|file| focus.matches(&file.path, &file.rel_path));
    }
    Ok(SelectionContext {
        workspace_root,
        repo_root,
        changed_files,
        policy,
    })
}

fn changed_files_from_diff(
    diff: WorkspaceGitDiff,
    workspace_root: &Path,
    repo_root: Option<&Path>,
) -> Vec<ChangedFile> {
    diff.changes
        .into_iter()
        .map(|change| changed_file_from_change(change, workspace_root, repo_root))
        .collect()
}

fn changed_file_from_change(
    change: WorkspaceGitFileChange,
    workspace_root: &Path,
    repo_root: Option<&Path>,
) -> ChangedFile {
    let path = PathBuf::from(&change.path);
    let rel_path = relative_path(&path, repo_root.unwrap_or(workspace_root));
    ChangedFile {
        path,
        rel_path,
        language: change.language.to_string(),
    }
}

fn select_verification_steps(ctx: &SelectionContext) -> Vec<SelectedVerificationStep> {
    if ctx.changed_files.is_empty() {
        return Vec::new();
    }
    let cwd = ctx
        .repo_root
        .as_deref()
        .unwrap_or(&ctx.workspace_root)
        .to_string_lossy()
        .to_string();
    let mut out = Vec::<SelectedVerificationStep>::new();
    let mut seen = HashSet::<String>::new();

    let mut rust_packages = BTreeSet::<String>::new();
    let mut rust_test_packages = BTreeSet::<String>::new();
    let mut workspace_rust_manifest_touched = false;
    let mut has_frontend = false;
    let mut has_i18n = false;
    let mut api_surface_touched = false;
    let mut docs_or_markdown = true;

    for file in &ctx.changed_files {
        let rel = file.rel_path.as_str();
        let lower = rel.to_ascii_lowercase();
        if !is_docs_or_markdown_path(&lower) {
            docs_or_markdown = false;
        }
        if is_i18n_path(&lower) {
            has_i18n = true;
        }
        if is_api_surface_path(&lower) {
            api_surface_touched = true;
        }
        if is_frontend_path(&lower, &file.language) {
            has_frontend = true;
        }
        if is_rust_path(&lower, &file.language) {
            let package_root = ctx.repo_root.as_deref().unwrap_or(&ctx.workspace_root);
            if let Some(pkg) = rust_package_for_path(&file.path, package_root) {
                if is_test_path(&lower) {
                    rust_test_packages.insert(pkg);
                } else {
                    rust_packages.insert(pkg);
                }
            } else if lower.ends_with("cargo.toml") || lower.ends_with("cargo.lock") {
                workspace_rust_manifest_touched = true;
            }
        }
    }

    for package in rust_packages {
        if is_safe_cargo_package(&package) {
            push_step(
                &mut out,
                &mut seen,
                SelectedVerificationStep {
                    command: format!("cargo check -p {package} --locked"),
                    cwd: cwd.clone(),
                    title: format!("Check Rust crate {package}"),
                    reason: "Rust source changed; run the smallest package-level type/build check instead of the full workspace suite.".to_string(),
                    category: "rust".to_string(),
                    risk: VerificationRisk::Low,
                    auto_run: true,
                    timeout_secs: DEFAULT_VERIFICATION_TIMEOUT_SECS,
                },
            );
        }
    }
    for package in rust_test_packages {
        if is_safe_cargo_package(&package) {
            push_step(
                &mut out,
                &mut seen,
                SelectedVerificationStep {
                    command: format!("cargo check -p {package} --tests --locked"),
                    cwd: cwd.clone(),
                    title: format!("Check Rust tests for {package}"),
                    reason: "Rust test code changed; compile that package's test targets without running the full test suite.".to_string(),
                    category: "rust".to_string(),
                    risk: VerificationRisk::Low,
                    auto_run: true,
                    timeout_secs: DEFAULT_VERIFICATION_TIMEOUT_SECS,
                },
            );
        }
    }
    if has_frontend {
        push_step(
            &mut out,
            &mut seen,
            SelectedVerificationStep {
                command: "pnpm typecheck".to_string(),
                cwd: cwd.clone(),
                title: "Typecheck frontend".to_string(),
                reason: "TypeScript/React files changed; project rules prefer typecheck as targeted validation.".to_string(),
                category: "frontend".to_string(),
                risk: VerificationRisk::Low,
                auto_run: true,
                timeout_secs: DEFAULT_VERIFICATION_TIMEOUT_SECS,
            },
        );
    }
    if has_i18n {
        push_step(
            &mut out,
            &mut seen,
            SelectedVerificationStep {
                command: "node scripts/sync-i18n.mjs --check".to_string(),
                cwd: cwd.clone(),
                title: "Check i18n completeness".to_string(),
                reason: "Locale files or i18n sync tooling changed; verify translation key parity."
                    .to_string(),
                category: "i18n".to_string(),
                risk: VerificationRisk::Low,
                auto_run: true,
                timeout_secs: DEFAULT_VERIFICATION_TIMEOUT_SECS,
            },
        );
    }
    if api_surface_touched {
        push_step(
            &mut out,
            &mut seen,
            SelectedVerificationStep {
                command: "git diff --check".to_string(),
                cwd: cwd.clone(),
                title: "Check diff whitespace".to_string(),
                reason: "API/transport surface changed; start with a cheap diff sanity check before heavier owner API alignment checks.".to_string(),
                category: "sanity".to_string(),
                risk: VerificationRisk::Low,
                auto_run: true,
                timeout_secs: 30,
            },
        );
    }
    if docs_or_markdown || out.is_empty() {
        push_step(
            &mut out,
            &mut seen,
            SelectedVerificationStep {
                command: "git diff --check".to_string(),
                cwd: cwd.clone(),
                title: "Check diff whitespace".to_string(),
                reason: "Documentation or mixed changes should at least pass whitespace/conflict-marker sanity before completion.".to_string(),
                category: "sanity".to_string(),
                risk: VerificationRisk::Low,
                auto_run: true,
                timeout_secs: 30,
            },
        );
    }
    if workspace_rust_manifest_touched {
        push_step(
            &mut out,
            &mut seen,
            SelectedVerificationStep {
                command: "cargo check --workspace --locked".to_string(),
                cwd: cwd.clone(),
                title: "Gated workspace Rust check".to_string(),
                reason: "Cargo workspace metadata changed. This broader check is useful but not auto-run because project rules avoid full workspace validation by default.".to_string(),
                category: "rust".to_string(),
                risk: VerificationRisk::High,
                auto_run: false,
                timeout_secs: DEFAULT_VERIFICATION_TIMEOUT_SECS,
            },
        );
    }
    if ctx.policy.full_suite_gated {
        push_step(
            &mut out,
            &mut seen,
            SelectedVerificationStep {
                command: "pnpm lint && pnpm test".to_string(),
                cwd,
                title: "Gated frontend full checks".to_string(),
                reason: "Project instructions mention full checks/pre-push gates. They remain user-gated and are not run by smart verification.".to_string(),
                category: "policy".to_string(),
                risk: VerificationRisk::High,
                auto_run: false,
                timeout_secs: DEFAULT_VERIFICATION_TIMEOUT_SECS,
            },
        );
    }
    out
}

fn push_step(
    out: &mut Vec<SelectedVerificationStep>,
    seen: &mut HashSet<String>,
    step: SelectedVerificationStep,
) {
    if seen.insert(step.command.clone()) {
        out.push(step);
    }
}

struct CommandExecutionResult {
    state: VerificationStepState,
    exit_code: Option<i64>,
    output_preview: String,
    duration_ms: i64,
}

async fn run_verification_command(
    step: &SelectedVerificationStep,
) -> Result<CommandExecutionResult> {
    let started = Instant::now();
    let mut cmd = crate::platform::default_shell_command_tokio(&step.command);
    cmd.current_dir(&step.cwd);
    cmd.kill_on_drop(true);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let shell_env = crate::tools::exec::login_shell_env();
    if shell_env.is_empty() {
        if let Some(shell_path) = crate::tools::exec::get_login_shell_path() {
            cmd.env("PATH", shell_path);
        }
    } else {
        for (key, value) in shell_env {
            cmd.env(key, value);
        }
    }
    let timeout_secs = step.timeout_secs.max(1);
    let output = match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        cmd.output(),
    )
    .await
    {
        Ok(output) => {
            output.with_context(|| format!("run verification command `{}`", step.command))?
        }
        Err(_) => {
            return Ok(CommandExecutionResult {
                state: VerificationStepState::TimedOut,
                exit_code: None,
                output_preview: format!(
                    "Command timed out after {timeout_secs}s: {}",
                    step.command
                ),
                duration_ms: started.elapsed().as_millis().min(i64::MAX as u128) as i64,
            });
        }
    };
    let exit_code = output.status.code().unwrap_or(-1) as i64;
    let mut combined = String::new();
    if !output.stdout.is_empty() {
        combined.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        if !combined.ends_with('\n') && !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    if !combined.ends_with('\n') && !combined.is_empty() {
        combined.push('\n');
    }
    combined.push_str(&format!("[exit code: {exit_code}]"));
    let output_preview =
        crate::truncate_utf8(combined.trim(), VERIFICATION_OUTPUT_PREVIEW_CHARS).to_string();
    Ok(CommandExecutionResult {
        state: if exit_code == 0 {
            VerificationStepState::Passed
        } else {
            VerificationStepState::Failed
        },
        exit_code: Some(exit_code),
        output_preview,
        duration_ms: started.elapsed().as_millis().min(i64::MAX as u128) as i64,
    })
}

fn verification_plan_stats(
    steps: &[VerificationStep],
    failed_override: Option<bool>,
    focus_paths: &[String],
) -> Value {
    let total = steps.len();
    let runnable = steps.iter().filter(|step| step.auto_run).count();
    let gated = total.saturating_sub(runnable);
    let passed = steps
        .iter()
        .filter(|step| step.state == VerificationStepState::Passed)
        .count();
    let failed = steps.iter().filter(|step| step.state.is_failure()).count();
    let skipped = steps
        .iter()
        .filter(|step| step.state == VerificationStepState::Skipped)
        .count();
    json!({
        "total": total,
        "runnable": runnable,
        "gated": gated,
        "passed": passed,
        "failed": failed,
        "skipped": skipped,
        "ok": failed_override.map(|failed| !failed).unwrap_or(failed == 0),
        "focused": !focus_paths.is_empty(),
        "focusPaths": focus_paths,
        "commands": steps.iter().map(|step| {
            json!({
                "command": step.command,
                "category": step.category,
                "risk": step.risk,
                "autoRun": step.auto_run,
                "state": step.state,
                "exitCode": step.exit_code,
            })
        }).collect::<Vec<_>>(),
    })
}

fn verification_plan_summary(stats: &Value) -> String {
    let total = stats.get("total").and_then(Value::as_u64).unwrap_or(0);
    let runnable = stats.get("runnable").and_then(Value::as_u64).unwrap_or(0);
    let gated = stats.get("gated").and_then(Value::as_u64).unwrap_or(0);
    let focused = stats
        .get("focused")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if total == 0 {
        if focused {
            return "No focused verification command is needed for the selected context."
                .to_string();
        }
        return "No local verification command is needed for the current diff.".to_string();
    }
    if gated > 0 {
        if focused {
            return format!(
                "Selected {total} focused verification command(s): {runnable} runnable, {gated} gated by policy."
            );
        }
        return format!(
            "Selected {total} verification command(s): {runnable} runnable, {gated} gated by policy."
        );
    }
    if focused {
        return format!("Selected {total} focused verification command(s).");
    }
    format!("Selected {total} targeted verification command(s).")
}

fn verification_run_summary(stats: &Value) -> String {
    let total = stats.get("total").and_then(Value::as_u64).unwrap_or(0);
    let passed = stats.get("passed").and_then(Value::as_u64).unwrap_or(0);
    let failed = stats.get("failed").and_then(Value::as_u64).unwrap_or(0);
    let skipped = stats.get("skipped").and_then(Value::as_u64).unwrap_or(0);
    let focused = stats
        .get("focused")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if total == 0 {
        if focused {
            return "Focused verification completed: no local commands were needed for the selected context."
                .to_string();
        }
        return "Verification completed: no local commands were needed for the current diff."
            .to_string();
    }
    if failed > 0 {
        if focused {
            return format!("Focused verification failed: {failed}/{total} command(s) failed.");
        }
        return format!("Verification failed: {failed}/{total} command(s) failed.");
    }
    if skipped > 0 {
        if focused {
            return format!(
                "Focused verification passed for runnable commands: {passed} passed, {skipped} gated suggestion(s) skipped."
            );
        }
        return format!(
            "Verification passed for runnable commands: {passed} passed, {skipped} gated suggestion(s) skipped."
        );
    }
    if focused {
        return format!(
            "Focused verification passed: {passed}/{total} targeted command(s) succeeded."
        );
    }
    format!("Verification passed: {passed}/{total} targeted command(s) succeeded.")
}

fn row_to_verification_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<VerificationRun> {
    let stats_json: String = row.get(6)?;
    Ok(VerificationRun {
        id: row.get(0)?,
        session_id: row.get(1)?,
        scope: row.get(2)?,
        state: VerificationRunState::from_str(row.get::<_, String>(3)?.as_str()),
        goal_id: row.get(4)?,
        summary: row.get(5)?,
        stats: serde_json::from_str(&stats_json).unwrap_or_else(|_| json!({})),
        error: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        completed_at: row.get(10)?,
    })
}

fn row_to_verification_step(row: &rusqlite::Row<'_>) -> rusqlite::Result<VerificationStep> {
    Ok(VerificationStep {
        id: row.get(0)?,
        run_id: row.get(1)?,
        session_id: row.get(2)?,
        seq: row.get(3)?,
        command: row.get(4)?,
        cwd: row.get(5)?,
        title: row.get(6)?,
        reason: row.get(7)?,
        category: row.get(8)?,
        risk: VerificationRisk::from_str(row.get::<_, String>(9)?.as_str()),
        auto_run: row.get::<_, i64>(10)? != 0,
        state: VerificationStepState::from_str(row.get::<_, String>(11)?.as_str()),
        exit_code: row.get(12)?,
        output_preview: row.get(13)?,
        duration_ms: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
        started_at: row.get(17)?,
        completed_at: row.get(18)?,
    })
}

fn row_to_verification_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<VerificationEvent> {
    let payload_json: String = row.get(4)?;
    Ok(VerificationEvent {
        id: row.get(0)?,
        run_id: row.get(1)?,
        seq: row.get(2)?,
        kind: row.get(3)?,
        payload: serde_json::from_str(&payload_json).unwrap_or_else(|_| json!({})),
        created_at: row.get(5)?,
    })
}

fn repo_root_for_path(path: &Path) -> Option<PathBuf> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("rev-parse")
        .arg("--show-toplevel")
        .current_dir(path);
    crate::platform::hide_console(&mut cmd);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if root.is_empty() {
        None
    } else {
        Some(PathBuf::from(root))
    }
}

fn rust_package_for_path(path: &Path, workspace_root: &Path) -> Option<String> {
    let mut dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    loop {
        let manifest = dir.join("Cargo.toml");
        if manifest.exists() {
            if let Some(name) = cargo_package_name(&manifest) {
                return Some(name);
            }
        }
        if dir == workspace_root || !dir.pop() {
            break;
        }
    }
    None
}

fn cargo_package_name(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("name") {
            let (_, value) = rest.split_once('=')?;
            return value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .split_whitespace()
                .next()
                .map(ToOwned::to_owned);
        }
    }
    None
}

fn read_policy_hints(workspace_root: &Path) -> PolicyHints {
    let mut hints = PolicyHints::default();
    for name in ["AGENTS.md", "CLAUDE.md"] {
        let path = workspace_root.join(name);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lower = text.to_ascii_lowercase();
        if lower.contains("不要主动跑全套")
            || lower.contains("full pre-push")
            || lower.contains("pre-push")
        {
            hints.full_suite_gated = true;
        }
    }
    hints
}

fn relative_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[derive(Debug, Clone, Default)]
struct FocusFilter {
    requested: Vec<String>,
    normalized: HashSet<String>,
}

impl FocusFilter {
    fn from_paths(paths: &[String]) -> Self {
        let mut requested = Vec::new();
        let mut normalized = HashSet::new();
        for path in paths {
            let normalized_path = normalize_focus_path(path);
            if normalized_path.is_empty() || !normalized.insert(normalized_path.clone()) {
                continue;
            }
            requested.push(normalized_path);
        }
        Self {
            requested,
            normalized,
        }
    }

    fn is_active(&self) -> bool {
        !self.normalized.is_empty()
    }

    fn matches(&self, path: &Path, rel_path: &str) -> bool {
        if !self.is_active() {
            return true;
        }
        let full = normalize_focus_path(&path.to_string_lossy());
        let rel = normalize_focus_path(rel_path);
        self.normalized.iter().any(|focus| {
            focus_path_matches(full.as_str(), focus) || focus_path_matches(rel.as_str(), focus)
        })
    }
}

fn normalize_focus_paths(paths: &[String]) -> Vec<String> {
    FocusFilter::from_paths(paths).requested
}

fn normalize_focus_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

fn focus_path_matches(candidate: &str, focus: &str) -> bool {
    candidate == focus
        || candidate.ends_with(&format!("/{focus}"))
        || focus.ends_with(&format!("/{candidate}"))
}

fn is_safe_cargo_package(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn is_rust_path(lower: &str, language: &str) -> bool {
    language == "rust"
        || lower.ends_with(".rs")
        || lower.ends_with("cargo.toml")
        || lower.ends_with("cargo.lock")
}

fn is_frontend_path(lower: &str, language: &str) -> bool {
    matches!(language, "typescript" | "tsx" | "javascript" | "jsx")
        || lower.ends_with(".ts")
        || lower.ends_with(".tsx")
        || lower.ends_with(".js")
        || lower.ends_with(".jsx")
        || lower.ends_with("package.json")
        || lower.ends_with("pnpm-lock.yaml")
        || lower.ends_with("vite.config.ts")
        || lower.ends_with("tailwind.config.ts")
}

fn is_i18n_path(lower: &str) -> bool {
    lower.contains("src/i18n/")
        || lower.contains("i18n/locales/")
        || lower.ends_with("scripts/sync-i18n.mjs")
}

fn is_api_surface_path(lower: &str) -> bool {
    lower == "src-tauri/src/lib.rs"
        || lower == "crates/ha-server/src/lib.rs"
        || lower == "src/lib/transport-http.ts"
        || lower == "docs/architecture/api-reference.md"
}

fn is_docs_or_markdown_path(lower: &str) -> bool {
    lower.ends_with(".md")
        || lower.starts_with("docs/")
        || lower == "agents.md"
        || lower == "claude.md"
}

fn is_test_path(lower: &str) -> bool {
    lower.contains("/test/")
        || lower.contains("/tests/")
        || lower.contains("__tests__")
        || lower.contains(".test.")
        || lower.contains(".spec.")
        || lower.ends_with("_test.go")
        || lower.ends_with("_test.rs")
        || lower.ends_with("_spec.rb")
        || lower.ends_with("_test.py")
        || lower.ends_with("tests.rs")
}

fn bounded_payload(payload: Value) -> Result<String> {
    let mut s = stable_json(&payload)?;
    if s.len() > VERIFICATION_EVENT_PAYLOAD_MAX_BYTES {
        s = stable_json(&json!({
            "truncated": true,
            "preview": crate::truncate_utf8(&s, VERIFICATION_EVENT_PAYLOAD_MAX_BYTES),
        }))?;
    }
    Ok(s)
}

fn stable_json(value: &Value) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn emit_verification_run<T: Serialize>(event: &str, run: &T) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(event, json!(run));
    }
}

fn emit_verification_step(event: &str, step: &VerificationStep) {
    emit_verification_run(event, step);
}

fn emit_verification_event(event: &str, verification_event: &VerificationEvent) {
    emit_verification_run(event, verification_event);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(files: Vec<ChangedFile>) -> SelectionContext {
        SelectionContext {
            workspace_root: PathBuf::from("/repo"),
            repo_root: Some(PathBuf::from("/repo")),
            changed_files: files,
            policy: PolicyHints {
                full_suite_gated: true,
            },
        }
    }

    fn file(rel: &str, language: &str) -> ChangedFile {
        ChangedFile {
            path: PathBuf::from("/repo").join(rel),
            rel_path: rel.to_string(),
            language: language.to_string(),
        }
    }

    #[test]
    fn selector_prefers_package_level_rust_check() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join("crates/ha-core/src")).expect("dirs");
        std::fs::write(
            temp.path().join("crates/ha-core/Cargo.toml"),
            "[package]\nname = \"ha-core\"\n",
        )
        .expect("manifest");
        let mut context = ctx(vec![ChangedFile {
            path: temp.path().join("crates/ha-core/src/lib.rs"),
            rel_path: "crates/ha-core/src/lib.rs".to_string(),
            language: "rust".to_string(),
        }]);
        context.workspace_root = temp.path().to_path_buf();
        context.repo_root = Some(temp.path().to_path_buf());

        let steps = select_verification_steps(&context);
        assert!(steps
            .iter()
            .any(|step| step.command == "cargo check -p ha-core --locked" && step.auto_run));
        assert!(steps.iter().any(|step| !step.auto_run));
    }

    #[test]
    fn selector_maps_frontend_and_i18n_changes() {
        let steps = select_verification_steps(&ctx(vec![
            file("src/components/App.tsx", "tsx"),
            file("src/i18n/locales/zh.json", "json"),
        ]));
        assert!(steps
            .iter()
            .any(|step| step.command == "pnpm typecheck" && step.auto_run));
        assert!(steps
            .iter()
            .any(|step| step.command == "node scripts/sync-i18n.mjs --check" && step.auto_run));
    }

    #[test]
    fn selector_uses_diff_check_for_docs_only() {
        let steps = select_verification_steps(&ctx(vec![file("docs/roadmap/a.md", "markdown")]));
        assert_eq!(steps[0].command, "git diff --check");
        assert!(steps[0].auto_run);
    }
}
