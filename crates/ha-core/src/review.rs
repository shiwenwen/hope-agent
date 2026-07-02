//! Durable local code review engine.
//!
//! Phase 3.3 starts with a deterministic local reviewer: it scans the current
//! git working-tree diff, overlays cached LSP diagnostics, verifies each
//! candidate into a stable tri-state verdict, and persists the result as a
//! control-plane object. The data model intentionally leaves room for future
//! LLM reviewers without making the GUI or Goal evidence depend on them.

use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use similar::{ChangeTag, TextDiff};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::lsp::LspDiagnostic;
use crate::session::{
    effective_working_dir_for_meta, load_session_git_diff, SessionDB, WorkspaceGitDiff,
    WorkspaceGitFileAction, WorkspaceGitFileChange,
};
use crate::util::now_rfc3339;

const REVIEW_EVENT_PAYLOAD_MAX_BYTES: usize = 64 * 1024;
const MAX_FINDINGS_PER_RUN: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewRunState {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl ReviewRunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Running,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    P0,
    P1,
    P2,
    P3,
}

impl ReviewSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::P0 => "p0",
            Self::P1 => "p1",
            Self::P2 => "p2",
            Self::P3 => "p3",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "p0" | "critical" => Self::P0,
            "p1" | "high" => Self::P1,
            "p2" | "medium" => Self::P2,
            _ => Self::P3,
        }
    }

    pub fn is_blocking(self) -> bool {
        matches!(self, Self::P0 | Self::P1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Confirmed,
    Plausible,
    Refuted,
}

impl ReviewVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Plausible => "plausible",
            Self::Refuted => "refuted",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "confirmed" => Self::Confirmed,
            "refuted" => Self::Refuted,
            _ => Self::Plausible,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFindingStatus {
    Open,
    Resolved,
    Dismissed,
    FalsePositive,
}

impl ReviewFindingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
            Self::Dismissed => "dismissed",
            Self::FalsePositive => "false_positive",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "resolved" | "fixed" => Some(Self::Resolved),
            "dismissed" | "closed" => Some(Self::Dismissed),
            "false_positive" | "false-positive" => Some(Self::FalsePositive),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRun {
    pub id: String,
    pub session_id: String,
    pub scope: String,
    pub state: ReviewRunState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<String>,
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
pub struct ReviewFinding {
    pub id: String,
    pub run_id: String,
    pub session_id: String,
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    pub title: String,
    pub body: String,
    pub category: String,
    pub severity: ReviewSeverity,
    pub verdict: ReviewVerdict,
    pub status: ReviewFindingStatus,
    pub evidence: Value,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewEvent {
    pub id: i64,
    pub run_id: String,
    pub seq: i64,
    pub kind: String,
    pub payload: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRunSnapshot {
    pub run: ReviewRun,
    pub findings: Vec<ReviewFinding>,
    #[serde(default)]
    pub events: Vec<ReviewEvent>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReviewInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub focus_paths: Vec<String>,
}

struct CandidateFinding {
    file: String,
    start_line: Option<u32>,
    end_line: Option<u32>,
    title: String,
    body: String,
    category: String,
    severity: ReviewSeverity,
    evidence: Value,
    confidence: f64,
}

struct ReviewContext {
    changed: Vec<ChangedFile>,
    diagnostics: Vec<LspDiagnostic>,
    workspace_root: Option<String>,
    focus_paths: Vec<String>,
}

#[derive(Debug, Clone)]
struct ChangedFile {
    path: String,
    action: WorkspaceGitFileAction,
    language: String,
    changed_lines: HashSet<u32>,
    after_lines: Vec<String>,
    truncated: bool,
    lines_added: u32,
    lines_removed: u32,
}

pub(crate) fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS review_runs (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            scope TEXT NOT NULL,
            state TEXT NOT NULL,
            base_ref TEXT,
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

        CREATE TABLE IF NOT EXISTS review_findings (
            id TEXT PRIMARY KEY,
            run_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            file_path TEXT NOT NULL,
            start_line INTEGER,
            end_line INTEGER,
            title TEXT NOT NULL,
            body TEXT NOT NULL,
            category TEXT NOT NULL,
            severity TEXT NOT NULL,
            verdict TEXT NOT NULL,
            status TEXT NOT NULL,
            evidence_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            resolved_at TEXT,
            FOREIGN KEY (run_id) REFERENCES review_runs(id) ON DELETE CASCADE,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS review_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (run_id) REFERENCES review_runs(id) ON DELETE CASCADE,
            UNIQUE(run_id, seq)
        );

        CREATE INDEX IF NOT EXISTS idx_review_runs_session_updated
            ON review_runs(session_id, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_review_runs_goal
            ON review_runs(goal_id, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_review_findings_run
            ON review_findings(run_id, severity, status);
        CREATE INDEX IF NOT EXISTS idx_review_findings_session
            ON review_findings(session_id, updated_at DESC);",
    )?;
    Ok(())
}

impl SessionDB {
    pub fn create_review_run(&self, input: &RunReviewInput, session_id: &str) -> Result<ReviewRun> {
        let meta = self
            .get_session(session_id)?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        if meta.incognito {
            bail!("Cannot create durable review run for incognito session {session_id}");
        }
        if effective_working_dir_for_meta(&meta).is_none() {
            bail!("session {session_id} has no working directory");
        }

        let scope = input.scope.as_deref().unwrap_or("local");
        if scope != "local" {
            bail!("review scope '{scope}' is not supported yet; use 'local'");
        }
        if input
            .base_ref
            .as_deref()
            .is_some_and(|base_ref| !base_ref.trim().is_empty())
        {
            bail!("review base_ref is not supported yet; omit baseRef for local review");
        }
        if !input.profiles.is_empty() {
            bail!("review profiles are not supported yet; omit profiles for local review");
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
        let id = format!("rev_{}", uuid::Uuid::new_v4().simple());
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "INSERT INTO review_runs (
                    id, session_id, scope, state, base_ref, goal_id, summary,
                    stats_json, created_at, updated_at
                ) VALUES (?1, ?2, ?3, 'running', ?4, ?5, '', '{}', ?6, ?6)",
                params![id, session_id, scope, input.base_ref, goal_id, now],
            )?;
        }
        let run = self
            .get_review_run(&id)?
            .ok_or_else(|| anyhow!("review run {} was not persisted", id))?;
        let _ = self.append_review_event(
            &run.id,
            "review_started",
            json!({
                "sessionId": session_id,
                "scope": scope,
                "baseRef": input.base_ref,
                "goalId": goal_id,
                "profiles": input.profiles,
            }),
        );
        emit_review_run("review:created", &run);
        Ok(run)
    }

    pub fn get_review_run(&self, run_id: &str) -> Result<Option<ReviewRun>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, session_id, scope, state, base_ref, goal_id, summary, stats_json,
                    error, created_at, updated_at, completed_at
             FROM review_runs WHERE id = ?1",
            params![run_id],
            row_to_review_run,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_review_runs_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<ReviewRun>> {
        let limit = limit.clamp(1, 200) as i64;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, scope, state, base_ref, goal_id, summary, stats_json,
                    error, created_at, updated_at, completed_at
             FROM review_runs
             WHERE session_id = ?1
             ORDER BY updated_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit], row_to_review_run)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn list_review_findings_for_run(&self, run_id: &str) -> Result<Vec<ReviewFinding>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, run_id, session_id, file_path, start_line, end_line, title, body,
                    category, severity, verdict, status, evidence_json,
                    created_at, updated_at, resolved_at
             FROM review_findings
             WHERE run_id = ?1
             ORDER BY
                CASE severity WHEN 'p0' THEN 0 WHEN 'p1' THEN 1 WHEN 'p2' THEN 2 ELSE 3 END,
                file_path ASC,
                COALESCE(start_line, 0) ASC,
                id ASC",
        )?;
        let rows = stmt.query_map(params![run_id], row_to_review_finding)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn review_run_snapshot(
        &self,
        run_id: &str,
        event_limit: usize,
    ) -> Result<Option<ReviewRunSnapshot>> {
        let Some(run) = self.get_review_run(run_id)? else {
            return Ok(None);
        };
        let findings = self.list_review_findings_for_run(run_id)?;
        let events = self.list_review_events(run_id, event_limit)?;
        Ok(Some(ReviewRunSnapshot {
            run,
            findings,
            events,
        }))
    }

    fn complete_review_run(
        &self,
        run_id: &str,
        summary: &str,
        stats: Value,
        findings: Vec<CandidateFinding>,
    ) -> Result<ReviewRunSnapshot> {
        let now = now_rfc3339();
        let stats_json = stable_json(&stats)?;
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE review_runs
                 SET state = 'completed',
                     summary = ?1,
                     stats_json = ?2,
                     updated_at = ?3,
                     completed_at = ?3
                 WHERE id = ?4",
                params![summary, stats_json, now, run_id],
            )?;
        }
        for candidate in findings.into_iter().take(MAX_FINDINGS_PER_RUN) {
            self.insert_review_finding(run_id, candidate)?;
        }
        let snapshot = self
            .review_run_snapshot(run_id, 100)?
            .ok_or_else(|| anyhow!("review run {} not found after completion", run_id))?;
        let _ = self.append_review_event(
            run_id,
            "review_completed",
            json!({
                "summary": snapshot.run.summary,
                "stats": snapshot.run.stats,
                "findingCount": snapshot.findings.len(),
            }),
        );
        self.link_review_goal_evidence(&snapshot)?;
        emit_review_run("review:updated", &snapshot.run);
        Ok(snapshot)
    }

    fn fail_review_run(&self, run_id: &str, error: &str) -> Result<ReviewRunSnapshot> {
        let now = now_rfc3339();
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE review_runs
                 SET state = 'failed', error = ?1, summary = ?1, updated_at = ?2, completed_at = ?2
                 WHERE id = ?3",
                params![error, now, run_id],
            )?;
        }
        let _ = self.append_review_event(run_id, "review_failed", json!({ "error": error }));
        let snapshot = self
            .review_run_snapshot(run_id, 100)?
            .ok_or_else(|| anyhow!("review run {} not found after failure", run_id))?;
        emit_review_run("review:updated", &snapshot.run);
        Ok(snapshot)
    }

    fn insert_review_finding(
        &self,
        run_id: &str,
        candidate: CandidateFinding,
    ) -> Result<ReviewFinding> {
        let run = self
            .get_review_run(run_id)?
            .ok_or_else(|| anyhow!("review run not found: {run_id}"))?;
        let verdict = verify_candidate(&candidate);
        let status = if verdict == ReviewVerdict::Refuted {
            ReviewFindingStatus::Dismissed
        } else {
            ReviewFindingStatus::Open
        };
        let now = now_rfc3339();
        let id = format!("revf_{}", uuid::Uuid::new_v4().simple());
        let evidence = merge_evidence(candidate.evidence, verdict, candidate.confidence);
        let evidence_json = stable_json(&evidence)?;
        let start_line = candidate.start_line.map(i64::from);
        let end_line = candidate.end_line.map(i64::from);
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "INSERT INTO review_findings (
                    id, run_id, session_id, file_path, start_line, end_line, title, body,
                    category, severity, verdict, status, evidence_json, created_at, updated_at,
                    resolved_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14, ?15)",
                params![
                    id,
                    run.id,
                    run.session_id,
                    candidate.file,
                    start_line,
                    end_line,
                    candidate.title,
                    candidate.body,
                    candidate.category,
                    candidate.severity.as_str(),
                    verdict.as_str(),
                    status.as_str(),
                    evidence_json,
                    now,
                    if status == ReviewFindingStatus::Open {
                        None::<String>
                    } else {
                        Some(now.clone())
                    },
                ],
            )?;
        }
        let finding = self
            .get_review_finding(&id)?
            .ok_or_else(|| anyhow!("review finding {} was not persisted", id))?;
        let _ = self.append_review_event(
            run_id,
            "finding_created",
            json!({
                "findingId": finding.id,
                "severity": finding.severity,
                "verdict": finding.verdict,
                "status": finding.status,
                "file": finding.file,
                "startLine": finding.start_line,
            }),
        );
        emit_review_finding("review:finding_updated", &finding);
        Ok(finding)
    }

    pub fn get_review_finding(&self, finding_id: &str) -> Result<Option<ReviewFinding>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, run_id, session_id, file_path, start_line, end_line, title, body,
                    category, severity, verdict, status, evidence_json,
                    created_at, updated_at, resolved_at
             FROM review_findings WHERE id = ?1",
            params![finding_id],
            row_to_review_finding,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn update_review_finding_status(
        &self,
        finding_id: &str,
        status: ReviewFindingStatus,
    ) -> Result<ReviewFinding> {
        let previous = self
            .get_review_finding(finding_id)?
            .ok_or_else(|| anyhow!("review finding not found: {finding_id}"))?;
        let now = now_rfc3339();
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE review_findings
                 SET status = ?1,
                     updated_at = ?2,
                     resolved_at = CASE WHEN ?1 = 'open' THEN NULL ELSE ?2 END
                 WHERE id = ?3",
                params![status.as_str(), now, finding_id],
            )?;
        }
        let finding = self
            .get_review_finding(finding_id)?
            .ok_or_else(|| anyhow!("review finding not found after update: {finding_id}"))?;
        let _ = self.append_review_event(
            &finding.run_id,
            "finding_status_changed",
            json!({
                "findingId": finding.id,
                "from": previous.status,
                "to": finding.status,
            }),
        );
        self.refresh_goal_link_for_review_finding(&finding)?;
        emit_review_finding("review:finding_updated", &finding);
        Ok(finding)
    }

    fn link_review_goal_evidence(&self, snapshot: &ReviewRunSnapshot) -> Result<()> {
        let Some(goal_id) = snapshot.run.goal_id.as_deref() else {
            return Ok(());
        };
        let blocking = snapshot
            .findings
            .iter()
            .filter(|finding| review_finding_blocks_goal(finding))
            .count();
        let relation = if blocking == 0 {
            "review_passed"
        } else {
            "review_completed"
        };
        let _ = self.link_goal_target(
            goal_id,
            "review",
            &snapshot.run.id,
            relation,
            json!({
                "runId": snapshot.run.id,
                "summary": snapshot.run.summary,
                "blockingFindings": blocking,
                "findingCount": snapshot.findings.len(),
                "stats": snapshot.run.stats,
                "completedAt": snapshot.run.completed_at,
            }),
        );
        for finding in &snapshot.findings {
            if review_finding_blocks_goal(finding) {
                let _ = self.link_goal_target(
                    goal_id,
                    "review",
                    &finding.id,
                    "review_finding",
                    review_finding_goal_metadata(finding),
                );
            }
        }
        Ok(())
    }

    fn refresh_goal_link_for_review_finding(&self, finding: &ReviewFinding) -> Result<()> {
        let Some(run) = self.get_review_run(&finding.run_id)? else {
            return Ok(());
        };
        let Some(goal_id) = run.goal_id.as_deref() else {
            return Ok(());
        };
        let _ = self.link_goal_target(
            goal_id,
            "review",
            &finding.id,
            "review_finding",
            review_finding_goal_metadata(finding),
        );
        if let Some(snapshot) = self.review_run_snapshot(&run.id, 100)? {
            self.link_review_goal_evidence(&snapshot)?;
        }
        Ok(())
    }

    pub fn append_review_event(
        &self,
        run_id: &str,
        kind: &str,
        payload: Value,
    ) -> Result<ReviewEvent> {
        let payload_json = bounded_payload(payload)?;
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM review_events WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO review_events (run_id, seq, kind, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run_id, seq, kind, payload_json, now],
        )?;
        let id = conn.last_insert_rowid();
        let event = ReviewEvent {
            id,
            run_id: run_id.to_string(),
            seq,
            kind: kind.to_string(),
            payload: serde_json::from_str(&payload_json)?,
            created_at: now,
        };
        drop(conn);
        emit_review_event("review:event", &event);
        Ok(event)
    }

    pub fn list_review_events(&self, run_id: &str, limit: usize) -> Result<Vec<ReviewEvent>> {
        let limit = limit.clamp(1, 500) as i64;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, run_id, seq, kind, payload_json, created_at
             FROM review_events
             WHERE run_id = ?1
             ORDER BY seq DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![run_id, limit], row_to_review_event)?;
        let mut events = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        events.sort_by_key(|event| event.seq);
        Ok(events)
    }
}

pub async fn run_review_for_session(
    db: Arc<SessionDB>,
    session_id: String,
    input: RunReviewInput,
) -> Result<ReviewRunSnapshot> {
    let run = db.create_review_run(&input, &session_id)?;
    let result = run_review_inner(db.clone(), &session_id, &input).await;
    match result {
        Ok((ctx, mut candidates)) => {
            let candidate_total = candidates.len();
            if candidates.len() > MAX_FINDINGS_PER_RUN {
                candidates.truncate(MAX_FINDINGS_PER_RUN);
            }
            let mut stats = review_stats(&ctx, &candidates);
            if let Some(obj) = stats.as_object_mut() {
                obj.insert("candidateTotal".to_string(), json!(candidate_total));
                obj.insert(
                    "truncatedFindings".to_string(),
                    json!(candidate_total.saturating_sub(candidates.len())),
                );
            }
            let summary = review_summary(&stats);
            db.complete_review_run(&run.id, &summary, stats, candidates)
        }
        Err(err) => db.fail_review_run(&run.id, &err.to_string()),
    }
}

async fn run_review_inner(
    db: Arc<SessionDB>,
    session_id: &str,
    input: &RunReviewInput,
) -> Result<(ReviewContext, Vec<CandidateFinding>)> {
    let focus = FocusFilter::from_paths(&input.focus_paths);
    let diff = {
        let db = db.clone();
        let sid = session_id.to_string();
        tokio::task::spawn_blocking(move || load_session_git_diff(&db, &sid)).await??
    };
    let mut diagnostics = crate::lsp::diagnostics_for_session(&db, session_id)
        .await
        .map(|snapshot| snapshot.diagnostics)
        .unwrap_or_default();
    if focus.is_active() {
        diagnostics.retain(|diagnostic| {
            let path = diagnostic
                .path
                .as_deref()
                .unwrap_or(diagnostic.uri.as_str());
            focus.matches(path)
        });
    }
    let workspace_root = db
        .get_session(session_id)?
        .and_then(|meta| effective_working_dir_for_meta(&meta))
        .and_then(|path| workspace_root_for_path(Path::new(&path)));
    let mut changed = changed_files_from_diff(diff);
    if focus.is_active() {
        changed.retain(|file| focus.matches(&file.path));
    }
    let ctx = ReviewContext {
        changed,
        diagnostics,
        workspace_root,
        focus_paths: focus.requested,
    };
    let mut candidates = Vec::new();
    candidates.extend(candidates_from_lsp(&ctx));
    candidates.extend(candidates_from_changed_lines(&ctx));
    candidates.extend(candidates_from_test_coverage(&ctx));
    dedup_candidates(&mut candidates);
    candidates.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.start_line.unwrap_or(0).cmp(&b.start_line.unwrap_or(0)))
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok((ctx, candidates))
}

fn changed_files_from_diff(diff: WorkspaceGitDiff) -> Vec<ChangedFile> {
    diff.changes
        .into_iter()
        .map(|change| {
            let changed_lines = changed_after_lines(&change);
            let after_lines = change
                .after
                .as_deref()
                .unwrap_or_default()
                .lines()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            ChangedFile {
                path: normalize_path(&change.path),
                action: change.action,
                language: change.language.to_string(),
                changed_lines,
                after_lines,
                truncated: change.truncated,
                lines_added: change.lines_added,
                lines_removed: change.lines_removed,
            }
        })
        .collect()
}

fn changed_after_lines(change: &WorkspaceGitFileChange) -> HashSet<u32> {
    if matches!(change.action, WorkspaceGitFileAction::Delete) {
        return HashSet::new();
    }
    if matches!(change.action, WorkspaceGitFileAction::Create) {
        return (1..=change.after.as_deref().unwrap_or_default().lines().count() as u32).collect();
    }
    let before = change.before.as_deref().unwrap_or_default();
    let after = change.after.as_deref().unwrap_or_default();
    let diff = TextDiff::from_lines(before, after);
    let mut new_line = 1u32;
    let mut out = HashSet::new();
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                new_line += 1;
            }
            ChangeTag::Delete => {}
            ChangeTag::Insert => {
                out.insert(new_line);
                new_line += 1;
            }
        }
    }
    out
}

fn candidates_from_lsp(ctx: &ReviewContext) -> Vec<CandidateFinding> {
    let changed_by_path = ctx
        .changed
        .iter()
        .map(|file| (normalize_path(&file.path), file))
        .collect::<HashMap<_, _>>();
    let mut candidates = Vec::new();
    for diagnostic in &ctx.diagnostics {
        let Some(path) = diagnostic.path.as_deref() else {
            continue;
        };
        let path = normalize_path(path);
        let Some(changed) = changed_by_path.get(&path) else {
            continue;
        };
        let line = diagnostic.range.start_line;
        if !changed.changed_lines.contains(&line)
            && !matches!(changed.action, WorkspaceGitFileAction::Create)
        {
            continue;
        }
        let (severity, confidence) = match diagnostic.severity.as_str() {
            "error" => (ReviewSeverity::P1, 0.95),
            "warning" => (ReviewSeverity::P2, 0.78),
            _ => (ReviewSeverity::P3, 0.62),
        };
        let source = diagnostic.source.as_deref().unwrap_or("language server");
        candidates.push(CandidateFinding {
            file: path,
            start_line: Some(line),
            end_line: Some(diagnostic.range.end_line.max(line)),
            title: format!("{} diagnostic on changed code", severity_label(severity)),
            body: format!(
                "{} reported on a line changed by this diff: {}",
                source, diagnostic.message
            ),
            category: "correctness".to_string(),
            severity,
            evidence: json!({
                "kind": "lsp_diagnostic",
                "source": diagnostic.source,
                "code": diagnostic.code,
                "message": diagnostic.message,
                "diagnosticSeverity": diagnostic.severity,
                "uri": diagnostic.uri,
                "workspaceRoot": ctx.workspace_root,
            }),
            confidence,
        });
    }
    candidates
}

fn candidates_from_changed_lines(ctx: &ReviewContext) -> Vec<CandidateFinding> {
    let mut out = Vec::new();
    for file in &ctx.changed {
        if file.truncated {
            out.push(CandidateFinding {
                file: file.path.clone(),
                start_line: None,
                end_line: None,
                title: "Large changed file was only partially reviewable".to_string(),
                body: "The diff content exceeded the inline review cap, so local review could only use file-level metadata. Run a focused review on this file before relying on the result.".to_string(),
                category: "maintainability".to_string(),
                severity: ReviewSeverity::P3,
                evidence: json!({
                    "kind": "truncated_diff",
                    "linesAdded": file.lines_added,
                    "linesRemoved": file.lines_removed,
                }),
                confidence: 0.55,
            });
        }
        for line in &file.changed_lines {
            let Some(text) = file.after_lines.get(line.saturating_sub(1) as usize) else {
                continue;
            };
            let trimmed = text.trim();
            if is_conflict_marker(trimmed) {
                out.push(CandidateFinding {
                    file: file.path.clone(),
                    start_line: Some(*line),
                    end_line: Some(*line),
                    title: "Merge conflict marker left in changed code".to_string(),
                    body: "This line contains a git conflict marker. Shipping it would usually break parsing or expose unresolved conflict text to users.".to_string(),
                    category: "correctness".to_string(),
                    severity: ReviewSeverity::P1,
                    evidence: json!({ "kind": "conflict_marker", "line": trimmed }),
                    confidence: 0.99,
                });
            }
            if !is_test_path(&file.path) {
                if let Some(kind) = debug_statement_kind(trimmed, &file.language) {
                    out.push(CandidateFinding {
                    file: file.path.clone(),
                    start_line: Some(*line),
                    end_line: Some(*line),
                    title: "Debug output added in production code".to_string(),
                    body: format!(
                        "The changed line adds `{}`. If this is not intentionally user-facing logging, remove it or route it through the project's logging policy.",
                        kind
                    ),
                    category: "maintainability".to_string(),
                    severity: ReviewSeverity::P2,
                    evidence: json!({ "kind": "debug_statement", "statement": kind, "line": trimmed }),
                    confidence: 0.68,
                });
                }
            }
            if secret_pattern(trimmed).is_some() {
                out.push(CandidateFinding {
                    file: file.path.clone(),
                    start_line: Some(*line),
                    end_line: Some(*line),
                    title: "Possible secret added to source".to_string(),
                    body: "The changed line resembles a credential or private key. Remove it from the commit and rotate the value if it was real.".to_string(),
                    category: "security".to_string(),
                    severity: ReviewSeverity::P1,
                    evidence: json!({ "kind": "secret_pattern", "linePreview": redact_secret_line(trimmed) }),
                    confidence: 0.86,
                });
            }
        }
    }
    out
}

fn candidates_from_test_coverage(ctx: &ReviewContext) -> Vec<CandidateFinding> {
    let has_test_change = ctx.changed.iter().any(|file| is_test_path(&file.path));
    if has_test_change {
        return Vec::new();
    }
    ctx.changed
        .iter()
        .filter(|file| file.lines_added + file.lines_removed > 0)
        .filter(|file| is_source_language(&file.language) && !is_test_path(&file.path))
        .take(8)
        .map(|file| CandidateFinding {
            file: file.path.clone(),
            start_line: first_changed_line(file),
            end_line: first_changed_line(file),
            title: "Source change has no nearby test update".to_string(),
            body: "This review run found source code changes but no test/spec files in the same diff. Add focused coverage or record the targeted validation that proves the behavior.".to_string(),
            category: "tests".to_string(),
            severity: ReviewSeverity::P3,
            evidence: json!({
                "kind": "no_test_change",
                "language": file.language,
                "linesAdded": file.lines_added,
                "linesRemoved": file.lines_removed,
            }),
            confidence: 0.57,
        })
        .collect()
}

fn verify_candidate(candidate: &CandidateFinding) -> ReviewVerdict {
    let kind = candidate
        .evidence
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "conflict_marker" => ReviewVerdict::Confirmed,
        "secret_pattern" => ReviewVerdict::Confirmed,
        "lsp_diagnostic" if candidate.severity.is_blocking() => ReviewVerdict::Confirmed,
        "lsp_diagnostic" | "debug_statement" | "no_test_change" | "truncated_diff" => {
            ReviewVerdict::Plausible
        }
        _ if candidate.confidence >= 0.9 => ReviewVerdict::Confirmed,
        _ if candidate.confidence < 0.35 => ReviewVerdict::Refuted,
        _ => ReviewVerdict::Plausible,
    }
}

fn review_stats(ctx: &ReviewContext, candidates: &[CandidateFinding]) -> Value {
    let mut confirmed = 0u32;
    let mut plausible = 0u32;
    let mut refuted = 0u32;
    let mut p0 = 0u32;
    let mut p1 = 0u32;
    let mut p2 = 0u32;
    let mut p3 = 0u32;
    for candidate in candidates {
        match verify_candidate(candidate) {
            ReviewVerdict::Confirmed => confirmed += 1,
            ReviewVerdict::Plausible => plausible += 1,
            ReviewVerdict::Refuted => refuted += 1,
        }
        match candidate.severity {
            ReviewSeverity::P0 => p0 += 1,
            ReviewSeverity::P1 => p1 += 1,
            ReviewSeverity::P2 => p2 += 1,
            ReviewSeverity::P3 => p3 += 1,
        }
    }
    json!({
        "filesChanged": ctx.changed.len(),
        "diagnosticsConsidered": ctx.diagnostics.len(),
        "findings": candidates.len(),
        "focused": !ctx.focus_paths.is_empty(),
        "focusPaths": ctx.focus_paths.clone(),
        "confirmed": confirmed,
        "plausible": plausible,
        "refuted": refuted,
        "p0": p0,
        "p1": p1,
        "p2": p2,
        "p3": p3,
    })
}

fn review_summary(stats: &Value) -> String {
    let findings = stats.get("findings").and_then(Value::as_u64).unwrap_or(0);
    let p0 = stats.get("p0").and_then(Value::as_u64).unwrap_or(0);
    let p1 = stats.get("p1").and_then(Value::as_u64).unwrap_or(0);
    let p2 = stats.get("p2").and_then(Value::as_u64).unwrap_or(0);
    let p3 = stats.get("p3").and_then(Value::as_u64).unwrap_or(0);
    let truncated = stats
        .get("truncatedFindings")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let focused = stats
        .get("focused")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if findings == 0 {
        if focused {
            return "Focused review completed with no local findings.".to_string();
        }
        return "Review completed with no local findings.".to_string();
    }
    let prefix = if focused {
        "Focused review completed"
    } else {
        "Review completed"
    };
    if truncated > 0 {
        return format!(
            "{prefix} with {findings} finding(s): P0 {p0}, P1 {p1}, P2 {p2}, P3 {p3}. {truncated} additional candidate(s) were omitted by the per-run cap."
        );
    }
    format!("{prefix} with {findings} finding(s): P0 {p0}, P1 {p1}, P2 {p2}, P3 {p3}.")
}

fn row_to_review_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReviewRun> {
    let stats_json: String = row.get(7)?;
    Ok(ReviewRun {
        id: row.get(0)?,
        session_id: row.get(1)?,
        scope: row.get(2)?,
        state: ReviewRunState::from_str(row.get::<_, String>(3)?.as_str()),
        base_ref: row.get(4)?,
        goal_id: row.get(5)?,
        summary: row.get(6)?,
        stats: serde_json::from_str(&stats_json).unwrap_or_else(|_| json!({})),
        error: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        completed_at: row.get(11)?,
    })
}

fn row_to_review_finding(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReviewFinding> {
    let evidence_json: String = row.get(12)?;
    Ok(ReviewFinding {
        id: row.get(0)?,
        run_id: row.get(1)?,
        session_id: row.get(2)?,
        file: row.get(3)?,
        start_line: row.get::<_, Option<i64>>(4)?.map(|v| v.max(0) as u32),
        end_line: row.get::<_, Option<i64>>(5)?.map(|v| v.max(0) as u32),
        title: row.get(6)?,
        body: row.get(7)?,
        category: row.get(8)?,
        severity: ReviewSeverity::from_str(row.get::<_, String>(9)?.as_str()),
        verdict: ReviewVerdict::from_str(row.get::<_, String>(10)?.as_str()),
        status: ReviewFindingStatus::from_str(row.get::<_, String>(11)?.as_str())
            .unwrap_or(ReviewFindingStatus::Open),
        evidence: serde_json::from_str(&evidence_json).unwrap_or_else(|_| json!({})),
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        resolved_at: row.get(15)?,
    })
}

fn row_to_review_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReviewEvent> {
    let payload_json: String = row.get(4)?;
    Ok(ReviewEvent {
        id: row.get(0)?,
        run_id: row.get(1)?,
        seq: row.get(2)?,
        kind: row.get(3)?,
        payload: serde_json::from_str(&payload_json).unwrap_or_else(|_| json!({})),
        created_at: row.get(5)?,
    })
}

fn review_finding_blocks_goal(finding: &ReviewFinding) -> bool {
    finding.status == ReviewFindingStatus::Open
        && finding.verdict != ReviewVerdict::Refuted
        && finding.severity.is_blocking()
}

fn review_finding_goal_metadata(finding: &ReviewFinding) -> Value {
    json!({
        "runId": finding.run_id,
        "findingId": finding.id,
        "file": finding.file,
        "startLine": finding.start_line,
        "endLine": finding.end_line,
        "severity": finding.severity.as_str(),
        "verdict": finding.verdict.as_str(),
        "status": finding.status.as_str(),
        "category": finding.category,
        "title": finding.title,
        "summary": finding.body,
        "resolvedAt": finding.resolved_at,
    })
}

fn merge_evidence(mut evidence: Value, verdict: ReviewVerdict, confidence: f64) -> Value {
    if let Some(obj) = evidence.as_object_mut() {
        obj.insert("verifier".to_string(), json!(verdict.as_str()));
        obj.insert("confidence".to_string(), json!(confidence));
    }
    evidence
}

fn dedup_candidates(candidates: &mut Vec<CandidateFinding>) {
    let mut seen = HashSet::new();
    candidates.retain(|candidate| {
        let key = format!(
            "{}:{}:{}:{}:{}",
            candidate.file,
            candidate.start_line.unwrap_or(0),
            candidate.category,
            candidate.severity.as_str(),
            candidate.title
        );
        seen.insert(key)
    });
}

fn first_changed_line(file: &ChangedFile) -> Option<u32> {
    file.changed_lines.iter().copied().min()
}

fn is_conflict_marker(line: &str) -> bool {
    line.starts_with("<<<<<<< ") || line.starts_with("=======") || line.starts_with(">>>>>>> ")
}

fn debug_statement_kind(line: &str, language: &str) -> Option<&'static str> {
    let lower = line.to_ascii_lowercase();
    if matches!(language, "typescript" | "tsx" | "javascript" | "jsx")
        && (lower.contains("console.log(")
            || lower.contains("console.debug(")
            || lower.contains("debugger;"))
    {
        return Some("console/debugger");
    }
    if language == "rust" && (lower.contains("dbg!(") || lower.contains("println!(")) {
        return Some("dbg!/println!");
    }
    if language == "python" && lower.starts_with("print(") {
        return Some("print");
    }
    None
}

fn secret_pattern(line: &str) -> Option<&'static str> {
    let lower = line.to_ascii_lowercase();
    if lower.contains("-----begin ") && lower.contains(" private key-----") {
        return Some("private_key");
    }
    if line.contains("sk-") && line.chars().filter(|ch| ch.is_ascii_alphanumeric()).count() > 30 {
        return Some("api_key");
    }
    if line.contains("AKIA") && line.len() > 24 {
        return Some("aws_access_key");
    }
    None
}

fn redact_secret_line(line: &str) -> String {
    let preview = crate::truncate_utf8(line, 96);
    preview
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { '*' } else { ch })
        .collect()
}

fn is_source_language(language: &str) -> bool {
    matches!(
        language,
        "rust"
            | "typescript"
            | "tsx"
            | "javascript"
            | "jsx"
            | "python"
            | "go"
            | "java"
            | "kotlin"
            | "swift"
            | "c"
            | "cpp"
            | "csharp"
            | "ruby"
            | "php"
            | "dart"
    )
}

fn is_test_path(path: &str) -> bool {
    let lower = path.replace('\\', "/").to_ascii_lowercase();
    lower.contains("/test/")
        || lower.contains("/tests/")
        || lower.contains("__tests__")
        || lower.contains(".test.")
        || lower.contains(".spec.")
        || lower.ends_with("_test.go")
        || lower.ends_with("_test.rs")
        || lower.ends_with("_spec.rb")
        || lower.ends_with("_test.py")
}

fn normalize_path(path: &str) -> String {
    PathBuf::from(path)
        .to_string_lossy()
        .replace('\\', "/")
        .to_string()
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

    fn matches(&self, path: &str) -> bool {
        if !self.is_active() {
            return true;
        }
        let candidate = normalize_focus_path(path);
        self.normalized
            .iter()
            .any(|focus| focus_path_matches(candidate.as_str(), focus))
    }
}

fn normalize_focus_path(path: &str) -> String {
    normalize_path(path)
        .trim()
        .trim_start_matches("./")
        .to_string()
}

fn focus_path_matches(candidate: &str, focus: &str) -> bool {
    candidate == focus
        || candidate.ends_with(&format!("/{focus}"))
        || focus.ends_with(&format!("/{candidate}"))
}

fn workspace_root_for_path(path: &Path) -> Option<String> {
    let dir = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    let out = std::process::Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if root.is_empty() {
        None
    } else {
        Some(root)
    }
}

fn severity_label(severity: ReviewSeverity) -> &'static str {
    match severity {
        ReviewSeverity::P0 => "Critical",
        ReviewSeverity::P1 => "Error",
        ReviewSeverity::P2 => "Warning",
        ReviewSeverity::P3 => "Info",
    }
}

fn bounded_payload(payload: Value) -> Result<String> {
    let mut s = stable_json(&payload)?;
    if s.len() > REVIEW_EVENT_PAYLOAD_MAX_BYTES {
        s = stable_json(&json!({
            "truncated": true,
            "preview": crate::truncate_utf8(&s, REVIEW_EVENT_PAYLOAD_MAX_BYTES),
        }))?;
    }
    Ok(s)
}

fn stable_json(value: &Value) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn emit_review_run<T: Serialize>(event: &str, run: &T) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(event, json!(run));
    }
}

fn emit_review_finding(event: &str, finding: &ReviewFinding) {
    emit_review_run(event, finding);
}

fn emit_review_event(event: &str, review_event: &ReviewEvent) {
    emit_review_run(event, review_event);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(before: &str, after: &str) -> WorkspaceGitFileChange {
        WorkspaceGitFileChange {
            kind: "file_change",
            path: "/repo/src/lib.rs".to_string(),
            action: WorkspaceGitFileAction::Edit,
            lines_added: 1,
            lines_removed: 1,
            before: Some(before.to_string()),
            after: Some(after.to_string()),
            language: "rust",
            truncated: false,
        }
    }

    #[test]
    fn changed_after_lines_track_inserted_lines() {
        let lines = changed_after_lines(&change("a\nb\nc\n", "a\nb2\nc\n"));
        assert!(lines.contains(&2));
        assert!(!lines.contains(&1));
    }

    #[test]
    fn verifier_confirms_conflict_marker() {
        let candidate = CandidateFinding {
            file: "/repo/a.rs".to_string(),
            start_line: Some(1),
            end_line: Some(1),
            title: "Merge conflict marker left in changed code".to_string(),
            body: String::new(),
            category: "correctness".to_string(),
            severity: ReviewSeverity::P1,
            evidence: json!({ "kind": "conflict_marker" }),
            confidence: 0.99,
        };
        assert_eq!(verify_candidate(&candidate), ReviewVerdict::Confirmed);
    }

    #[test]
    fn lsp_candidates_keep_one_based_diagnostic_lines() {
        let ctx = ReviewContext {
            changed: vec![ChangedFile {
                path: "/repo/src/lib.rs".to_string(),
                action: WorkspaceGitFileAction::Edit,
                language: "rust".to_string(),
                changed_lines: HashSet::from([2]),
                after_lines: vec!["fn main() {".to_string(), "missing;".to_string()],
                truncated: false,
                lines_added: 1,
                lines_removed: 0,
            }],
            diagnostics: vec![LspDiagnostic {
                uri: "file:///repo/src/lib.rs".to_string(),
                path: Some("/repo/src/lib.rs".to_string()),
                range: crate::lsp::LspRange {
                    start_line: 2,
                    start_column: 1,
                    end_line: 2,
                    end_column: 8,
                },
                severity: "error".to_string(),
                code: None,
                source: Some("rust-analyzer".to_string()),
                message: "cannot find value".to_string(),
            }],
            workspace_root: Some("/repo".to_string()),
            focus_paths: Vec::new(),
        };

        let candidates = candidates_from_lsp(&ctx);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].start_line, Some(2));
        assert_eq!(candidates[0].end_line, Some(2));
    }

    #[test]
    fn review_goal_blocker_only_open_high_severity() {
        let finding = ReviewFinding {
            id: "f".to_string(),
            run_id: "r".to_string(),
            session_id: "s".to_string(),
            file: "/repo/a.rs".to_string(),
            start_line: Some(1),
            end_line: Some(1),
            title: "x".to_string(),
            body: "x".to_string(),
            category: "correctness".to_string(),
            severity: ReviewSeverity::P1,
            verdict: ReviewVerdict::Confirmed,
            status: ReviewFindingStatus::Open,
            evidence: json!({}),
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
            resolved_at: None,
        };
        assert!(review_finding_blocks_goal(&finding));
    }
}
