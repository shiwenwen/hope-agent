//! Domain review and verification control plane.
//!
//! Coding review and smart verification are intentionally code-shaped: one
//! reasons about changed files, the other about shell commands. This module
//! keeps non-coding quality checks additive by evaluating domain workflow
//! evidence, approval gates, and profile rules into durable runs/checks/events.

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::domain_workflow::{DomainEvidenceItem, DomainWorkflowTemplate};
use crate::session::SessionDB;
use crate::util::now_rfc3339;

const DOMAIN_QUALITY_EVENT_PAYLOAD_MAX_BYTES: usize = 64 * 1024;
const DOMAIN_QUALITY_LIMIT_MAX: usize = 200;
const MAX_DOMAIN_CHECKS_PER_RUN: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainQualityRunState {
    Running,
    Completed,
    Failed,
    Blocked,
    NeedsUser,
    Cancelled,
}

impl DomainQualityRunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::NeedsUser => "needs_user",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "blocked" => Self::Blocked,
            "needs_user" => Self::NeedsUser,
            "cancelled" => Self::Cancelled,
            _ => Self::Running,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainQualitySeverity {
    P0,
    P1,
    P2,
    P3,
}

impl DomainQualitySeverity {
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
            "p1" | "high" | "blocking" => Self::P1,
            "p2" | "medium" | "advisory" => Self::P2,
            _ => Self::P3,
        }
    }

    pub fn is_blocking(self) -> bool {
        matches!(self, Self::P0 | Self::P1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainQualityCheckStatus {
    Passed,
    Failed,
    Blocked,
    NeedsUser,
    Advisory,
}

impl DomainQualityCheckStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::NeedsUser => "needs_user",
            Self::Advisory => "advisory",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "passed" => Self::Passed,
            "failed" => Self::Failed,
            "blocked" => Self::Blocked,
            "needs_user" => Self::NeedsUser,
            _ => Self::Advisory,
        }
    }

    pub fn blocks_goal(self) -> bool {
        matches!(self, Self::Failed | Self::Blocked | Self::NeedsUser)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainQualityRun {
    pub id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    pub domain: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_version: Option<String>,
    pub state: DomainQualityRunState,
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
pub struct DomainQualityCheck {
    pub id: String,
    pub run_id: String,
    pub session_id: String,
    pub seq: i64,
    pub check_type: String,
    pub profile: String,
    pub title: String,
    pub body: String,
    pub severity: DomainQualitySeverity,
    pub status: DomainQualityCheckStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_type: Option<String>,
    pub source_metadata: Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainQualityEvent {
    pub id: i64,
    pub run_id: String,
    pub seq: i64,
    pub kind: String,
    pub payload: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainQualityRunSnapshot {
    pub run: DomainQualityRun,
    pub checks: Vec<DomainQualityCheck>,
    #[serde(default)]
    pub events: Vec<DomainQualityEvent>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDomainQualityInput {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_kind: Option<String>,
    #[serde(default)]
    pub source_metadata: Value,
    #[serde(default)]
    pub explicit_user_approval: bool,
}

#[derive(Debug, Clone)]
pub struct CandidateCheck {
    pub check_type: String,
    pub profile: String,
    pub title: String,
    pub body: String,
    pub severity: DomainQualitySeverity,
    pub status: DomainQualityCheckStatus,
    pub evidence_type: Option<String>,
    pub source_metadata: Value,
}

#[derive(Debug, Clone)]
pub struct QualityContext {
    pub session_id: String,
    pub goal_id: Option<String>,
    pub domain: String,
    pub template: DomainWorkflowTemplate,
    pub evidence: Vec<DomainEvidenceItem>,
    pub evidence_scope: Value,
    pub input: RunDomainQualityInput,
}

pub(crate) fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS domain_quality_runs (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            goal_id TEXT,
            domain TEXT NOT NULL,
            template_id TEXT,
            template_version TEXT,
            state TEXT NOT NULL,
            summary TEXT NOT NULL DEFAULT '',
            stats_json TEXT NOT NULL DEFAULT '{}',
            error TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            completed_at TEXT,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (goal_id) REFERENCES goals(id) ON DELETE SET NULL
        );

        CREATE TABLE IF NOT EXISTS domain_quality_checks (
            id TEXT PRIMARY KEY,
            run_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            check_type TEXT NOT NULL,
            profile TEXT NOT NULL,
            title TEXT NOT NULL,
            body TEXT NOT NULL,
            severity TEXT NOT NULL,
            status TEXT NOT NULL,
            evidence_type TEXT,
            source_metadata_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (run_id) REFERENCES domain_quality_runs(id) ON DELETE CASCADE,
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS domain_quality_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            FOREIGN KEY (run_id) REFERENCES domain_quality_runs(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_domain_quality_runs_session
            ON domain_quality_runs(session_id, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_domain_quality_runs_goal
            ON domain_quality_runs(goal_id, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_domain_quality_checks_run
            ON domain_quality_checks(run_id, seq ASC);
        CREATE INDEX IF NOT EXISTS idx_domain_quality_checks_session
            ON domain_quality_checks(session_id, updated_at DESC);",
    )?;
    ensure_domain_quality_column(
        conn,
        "domain_quality_runs",
        "template_version",
        "ALTER TABLE domain_quality_runs ADD COLUMN template_version TEXT;",
    )?;
    Ok(())
}

impl SessionDB {
    pub fn list_domain_quality_runs_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<DomainQualityRun>> {
        let limit = limit.clamp(1, DOMAIN_QUALITY_LIMIT_MAX) as i64;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, goal_id, domain, template_id, template_version, state, summary,
                    stats_json, error, created_at, updated_at, completed_at
             FROM domain_quality_runs
             WHERE session_id = ?1
             ORDER BY updated_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit], row_to_domain_quality_run)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn get_domain_quality_run(&self, run_id: &str) -> Result<Option<DomainQualityRun>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, session_id, goal_id, domain, template_id, template_version, state, summary,
                    stats_json, error, created_at, updated_at, completed_at
             FROM domain_quality_runs
             WHERE id = ?1",
            params![run_id],
            row_to_domain_quality_run,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn domain_quality_run_snapshot(
        &self,
        run_id: &str,
        event_limit: usize,
    ) -> Result<Option<DomainQualityRunSnapshot>> {
        let Some(run) = self.get_domain_quality_run(run_id)? else {
            return Ok(None);
        };
        let checks = self.list_domain_quality_checks_for_run(run_id)?;
        let events = self.list_domain_quality_events(run_id, event_limit)?;
        Ok(Some(DomainQualityRunSnapshot {
            run,
            checks,
            events,
        }))
    }

    pub fn list_domain_quality_checks_for_run(
        &self,
        run_id: &str,
    ) -> Result<Vec<DomainQualityCheck>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, run_id, session_id, seq, check_type, profile, title, body,
                    severity, status, evidence_type, source_metadata_json, created_at, updated_at
             FROM domain_quality_checks
             WHERE run_id = ?1
             ORDER BY
                CASE severity WHEN 'p0' THEN 0 WHEN 'p1' THEN 1 WHEN 'p2' THEN 2 ELSE 3 END,
                CASE status WHEN 'blocked' THEN 0 WHEN 'needs_user' THEN 1 WHEN 'failed' THEN 2 WHEN 'advisory' THEN 3 ELSE 4 END,
                seq ASC",
        )?;
        let rows = stmt.query_map(params![run_id], row_to_domain_quality_check)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn create_domain_quality_run(&self, context: &QualityContext) -> Result<DomainQualityRun> {
        let now = now_rfc3339();
        let id = format!("dqr_{}", uuid::Uuid::new_v4().simple());
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "INSERT INTO domain_quality_runs (
                    id, session_id, goal_id, domain, template_id, template_version, state, summary,
                    stats_json, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', '', '{}', ?7, ?7)",
                params![
                    id,
                    context.session_id,
                    context.goal_id,
                    context.domain,
                    context.template.id,
                    context.template.version,
                    now,
                ],
            )?;
        }
        let run = self
            .get_domain_quality_run(&id)?
            .ok_or_else(|| anyhow!("domain quality run {} was not persisted", id))?;
        let _ = self.append_domain_quality_event(
            &run.id,
            "domain_quality_started",
            json!({
                "sessionId": context.session_id,
                "goalId": context.goal_id,
                "domain": context.domain,
                "templateId": context.template.id,
                "templateVersion": context.template.version,
                "profiles": active_profiles(&context.input.profiles, &context.domain),
                "artifact": {
                    "title": context.input.artifact_title,
                    "kind": context.input.artifact_kind,
                },
                "evidenceScope": context.evidence_scope,
                "source": context.input.source_metadata,
            }),
        );
        emit_domain_quality_run("domain_quality:created", &run);
        Ok(run)
    }

    pub fn complete_domain_quality_run(
        &self,
        run_id: &str,
        state: DomainQualityRunState,
        summary: &str,
        stats: Value,
        checks: Vec<CandidateCheck>,
    ) -> Result<DomainQualityRunSnapshot> {
        let now = now_rfc3339();
        let stats_json = stable_json(&stats)?;
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE domain_quality_runs
                 SET state = ?1,
                     summary = ?2,
                     stats_json = ?3,
                     updated_at = ?4,
                     completed_at = ?4
                 WHERE id = ?5",
                params![state.as_str(), summary, stats_json, now, run_id],
            )?;
        }
        self.insert_domain_quality_checks(run_id, checks)?;
        let snapshot = self
            .domain_quality_run_snapshot(run_id, 100)?
            .ok_or_else(|| anyhow!("domain quality run {} missing after completion", run_id))?;
        let _ = self.append_domain_quality_event(
            run_id,
            "domain_quality_completed",
            json!({
                "state": snapshot.run.state,
                "summary": snapshot.run.summary,
                "stats": snapshot.run.stats,
                "checkCount": snapshot.checks.len(),
            }),
        );
        self.link_domain_quality_goal_evidence(&snapshot)?;
        emit_domain_quality_run("domain_quality:updated", &snapshot.run);
        Ok(snapshot)
    }

    pub fn fail_domain_quality_run(
        &self,
        run_id: &str,
        error: &str,
    ) -> Result<DomainQualityRunSnapshot> {
        let now = now_rfc3339();
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE domain_quality_runs
                 SET state = 'failed', error = ?1, summary = ?1, updated_at = ?2, completed_at = ?2
                 WHERE id = ?3",
                params![error, now, run_id],
            )?;
        }
        let _ = self.append_domain_quality_event(
            run_id,
            "domain_quality_failed",
            json!({ "error": error }),
        );
        let snapshot = self
            .domain_quality_run_snapshot(run_id, 100)?
            .ok_or_else(|| anyhow!("domain quality run {} missing after failure", run_id))?;
        self.link_domain_quality_goal_evidence(&snapshot)?;
        emit_domain_quality_run("domain_quality:updated", &snapshot.run);
        Ok(snapshot)
    }

    fn insert_domain_quality_checks(
        &self,
        run_id: &str,
        checks: Vec<CandidateCheck>,
    ) -> Result<Vec<DomainQualityCheck>> {
        let run = self
            .get_domain_quality_run(run_id)?
            .ok_or_else(|| anyhow!("domain quality run not found: {run_id}"))?;
        let now = now_rfc3339();
        {
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
            for (idx, check) in checks
                .into_iter()
                .take(MAX_DOMAIN_CHECKS_PER_RUN)
                .enumerate()
            {
                let id = format!("dqc_{}", uuid::Uuid::new_v4().simple());
                conn.execute(
                    "INSERT INTO domain_quality_checks (
                        id, run_id, session_id, seq, check_type, profile, title, body,
                        severity, status, evidence_type, source_metadata_json,
                        created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)",
                    params![
                        id,
                        run.id,
                        run.session_id,
                        (idx + 1) as i64,
                        check.check_type,
                        check.profile,
                        check.title,
                        check.body,
                        check.severity.as_str(),
                        check.status.as_str(),
                        check.evidence_type,
                        stable_json(&check.source_metadata)?,
                        now,
                    ],
                )?;
            }
        }
        let inserted = self.list_domain_quality_checks_for_run(run_id)?;
        for check in &inserted {
            let _ = self.append_domain_quality_event(
                run_id,
                "check_recorded",
                json!({
                    "checkId": check.id,
                    "checkType": check.check_type,
                    "profile": check.profile,
                    "severity": check.severity,
                    "status": check.status,
                }),
            );
            emit_domain_quality_check("domain_quality:check_updated", check);
        }
        Ok(inserted)
    }

    fn link_domain_quality_goal_evidence(&self, snapshot: &DomainQualityRunSnapshot) -> Result<()> {
        let Some(goal_id) = snapshot.run.goal_id.as_deref() else {
            return Ok(());
        };
        let blocking = snapshot
            .checks
            .iter()
            .filter(|check| check_blocks_goal(check))
            .count();
        let needs_user = snapshot
            .checks
            .iter()
            .filter(|check| check.status == DomainQualityCheckStatus::NeedsUser)
            .count();
        let relation = match snapshot.run.state {
            DomainQualityRunState::Completed => "domain_quality_passed",
            DomainQualityRunState::NeedsUser => "domain_quality_needs_user",
            DomainQualityRunState::Blocked => "domain_quality_blocked",
            DomainQualityRunState::Failed => "domain_quality_failed",
            _ => "domain_quality_blocked",
        };
        let _ = self.link_goal_target(
            goal_id,
            "domain_quality",
            &snapshot.run.id,
            relation,
            json!({
                "runId": snapshot.run.id,
                "domain": snapshot.run.domain,
                "templateId": snapshot.run.template_id,
                "templateVersion": snapshot.run.template_version,
                "summary": snapshot.run.summary,
                "state": snapshot.run.state,
                "blockingChecks": blocking,
                "needsUser": needs_user,
                "checkCount": snapshot.checks.len(),
                "stats": snapshot.run.stats,
                "completedAt": snapshot.run.completed_at,
            }),
        );
        for check in &snapshot.checks {
            if check_blocks_goal(check) {
                let _ = self.link_goal_target(
                    goal_id,
                    "domain_quality",
                    &check.id,
                    "domain_quality_check",
                    json!({
                        "runId": snapshot.run.id,
                        "domain": snapshot.run.domain,
                        "title": check.title,
                        "summary": check.body,
                        "severity": check.severity,
                        "status": check.status,
                        "profile": check.profile,
                        "checkType": check.check_type,
                        "evidenceType": check.evidence_type,
                        "source": check.source_metadata,
                    }),
                );
            }
        }
        let _ = self.evaluate_goal(goal_id);
        Ok(())
    }

    pub fn append_domain_quality_event(
        &self,
        run_id: &str,
        kind: &str,
        payload: Value,
    ) -> Result<DomainQualityEvent> {
        let payload_json = bounded_payload(payload)?;
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM domain_quality_events WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO domain_quality_events (run_id, seq, kind, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run_id, seq, kind, payload_json, now],
        )?;
        let id = conn.last_insert_rowid();
        let event = DomainQualityEvent {
            id,
            run_id: run_id.to_string(),
            seq,
            kind: kind.to_string(),
            payload: serde_json::from_str(&payload_json)?,
            created_at: now,
        };
        drop(conn);
        emit_domain_quality_event("domain_quality:event", &event);
        Ok(event)
    }

    pub fn list_domain_quality_events(
        &self,
        run_id: &str,
        limit: usize,
    ) -> Result<Vec<DomainQualityEvent>> {
        let limit = limit.clamp(1, DOMAIN_QUALITY_LIMIT_MAX) as i64;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, run_id, seq, kind, payload_json, created_at
             FROM domain_quality_events
             WHERE run_id = ?1
             ORDER BY seq DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![run_id, limit], row_to_domain_quality_event)?;
        let mut events = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(anyhow::Error::from)?;
        events.reverse();
        Ok(events)
    }
}

fn check_blocks_goal(check: &DomainQualityCheck) -> bool {
    check.severity.is_blocking() && check.status.blocks_goal()
}

pub fn active_profiles(requested: &[String], domain: &str) -> Vec<String> {
    let mut profiles = requested
        .iter()
        .map(|profile| normalize_domain(profile))
        .filter(|profile| !profile.is_empty())
        .collect::<Vec<_>>();
    if profiles.is_empty() {
        profiles.push(domain.to_string());
        profiles.push("required_evidence".to_string());
        profiles.push("approval_gate".to_string());
    }
    profiles.sort();
    profiles.dedup();
    profiles
}

fn row_to_domain_quality_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<DomainQualityRun> {
    let state: String = row.get(6)?;
    let stats_json: String = row.get(8)?;
    Ok(DomainQualityRun {
        id: row.get(0)?,
        session_id: row.get(1)?,
        goal_id: row.get(2)?,
        domain: row.get(3)?,
        template_id: row.get(4)?,
        template_version: row.get(5)?,
        state: DomainQualityRunState::from_str(&state),
        summary: row.get(7)?,
        stats: serde_json::from_str(&stats_json).unwrap_or_else(|_| json!({})),
        error: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        completed_at: row.get(12)?,
    })
}

fn ensure_domain_quality_column(
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

fn row_to_domain_quality_check(row: &rusqlite::Row<'_>) -> rusqlite::Result<DomainQualityCheck> {
    let severity: String = row.get(8)?;
    let status: String = row.get(9)?;
    let source_metadata_json: String = row.get(11)?;
    Ok(DomainQualityCheck {
        id: row.get(0)?,
        run_id: row.get(1)?,
        session_id: row.get(2)?,
        seq: row.get(3)?,
        check_type: row.get(4)?,
        profile: row.get(5)?,
        title: row.get(6)?,
        body: row.get(7)?,
        severity: DomainQualitySeverity::from_str(&severity),
        status: DomainQualityCheckStatus::from_str(&status),
        evidence_type: row.get(10)?,
        source_metadata: serde_json::from_str(&source_metadata_json).unwrap_or_else(|_| json!({})),
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

fn row_to_domain_quality_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<DomainQualityEvent> {
    let payload_json: String = row.get(4)?;
    Ok(DomainQualityEvent {
        id: row.get(0)?,
        run_id: row.get(1)?,
        seq: row.get(2)?,
        kind: row.get(3)?,
        payload: serde_json::from_str(&payload_json).unwrap_or_else(|_| json!({})),
        created_at: row.get(5)?,
    })
}

pub fn normalize_domain(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn stable_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn bounded_payload(payload: Value) -> Result<String> {
    let mut s = stable_json(&payload)?;
    if s.len() > DOMAIN_QUALITY_EVENT_PAYLOAD_MAX_BYTES {
        s = stable_json(&json!({
            "truncated": true,
            "preview": crate::truncate_utf8(&s, DOMAIN_QUALITY_EVENT_PAYLOAD_MAX_BYTES),
        }))?;
    }
    Ok(s)
}

fn emit_domain_quality_run<T: Serialize>(event: &str, run: &T) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(event, json!(run));
    }
}

fn emit_domain_quality_check(event: &str, check: &DomainQualityCheck) {
    emit_domain_quality_run(event, check);
}

fn emit_domain_quality_event(event: &str, quality_event: &DomainQualityEvent) {
    emit_domain_quality_run(event, quality_event);
}
