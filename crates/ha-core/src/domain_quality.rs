//! Domain review and verification control plane.
//!
//! Coding review and smart verification are intentionally code-shaped: one
//! reasons about changed files, the other about shell commands. This module
//! keeps non-coding quality checks additive by evaluating domain workflow
//! evidence, approval gates, and profile rules into durable runs/checks/events.

use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::domain_workflow::{
    DomainEvidenceItem, DomainEvidenceRequirement, DomainWorkflowTemplate, ListDomainEvidenceInput,
    ListDomainWorkflowTemplatesInput,
};
use crate::session::SessionDB;
use crate::util::now_rfc3339;

const DOMAIN_QUALITY_EVENT_PAYLOAD_MAX_BYTES: usize = 64 * 1024;
const DOMAIN_QUALITY_LIMIT_DEFAULT: usize = 50;
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
struct CandidateCheck {
    check_type: String,
    profile: String,
    title: String,
    body: String,
    severity: DomainQualitySeverity,
    status: DomainQualityCheckStatus,
    evidence_type: Option<String>,
    source_metadata: Value,
}

#[derive(Debug, Clone)]
struct QualityContext {
    session_id: String,
    goal_id: Option<String>,
    domain: String,
    template: DomainWorkflowTemplate,
    evidence: Vec<DomainEvidenceItem>,
    input: RunDomainQualityInput,
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

    pub fn run_domain_quality_for_session(
        &self,
        input: RunDomainQualityInput,
    ) -> Result<DomainQualityRunSnapshot> {
        let context = self.resolve_quality_context(input)?;
        let run = self.create_domain_quality_run(&context)?;
        let result = (|| {
            let checks = build_quality_checks(&context);
            let stats = build_quality_stats(&context, &checks);
            let (state, summary) = summarize_quality(&context, &checks);
            self.complete_domain_quality_run(&run.id, state, &summary, stats, checks)
        })();
        match result {
            Ok(snapshot) => Ok(snapshot),
            Err(err) => self.fail_domain_quality_run(&run.id, &err.to_string()),
        }
    }

    fn resolve_quality_context(&self, input: RunDomainQualityInput) -> Result<QualityContext> {
        let session_id = non_empty(&input.session_id)
            .ok_or_else(|| anyhow!("session_id is required"))?
            .to_string();
        let session = self
            .get_session(&session_id)?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        if session.incognito {
            bail!("domain quality is disabled for incognito sessions");
        }

        let goal_id = match input.goal_id.as_deref().and_then(non_empty) {
            Some(goal_id) => {
                let goal = self
                    .get_goal(goal_id)?
                    .ok_or_else(|| anyhow!("goal not found: {goal_id}"))?;
                if goal.session_id != session_id {
                    bail!("goal {} belongs to session {}", goal.id, goal.session_id);
                }
                Some(goal.id)
            }
            None => self.active_goal_id_for_session(&session_id)?,
        };

        let template =
            self.resolve_domain_quality_template(&input, &session_id, goal_id.as_deref())?;
        let domain = template.domain.clone();
        let evidence = self.list_domain_evidence(ListDomainEvidenceInput {
            goal_id: None,
            session_id: Some(session_id.clone()),
            project_id: None,
            domain: Some(domain.clone()),
            evidence_type: None,
            limit: Some(DOMAIN_QUALITY_LIMIT_DEFAULT),
        })?;

        Ok(QualityContext {
            session_id,
            goal_id,
            domain,
            template,
            evidence,
            input,
        })
    }

    fn resolve_domain_quality_template(
        &self,
        input: &RunDomainQualityInput,
        session_id: &str,
        goal_id: Option<&str>,
    ) -> Result<DomainWorkflowTemplate> {
        if let Some(template_id) = input.template_id.as_deref().and_then(non_empty) {
            return self
                .get_domain_workflow_template(template_id, input.template_version.as_deref())?
                .ok_or_else(|| anyhow!("domain workflow template not found: {template_id}"));
        }

        let explicit_domain = input
            .domain
            .as_deref()
            .and_then(non_empty)
            .map(normalize_domain);
        if explicit_domain.is_none() {
            if let Some(goal_id) = goal_id {
                if let Some(goal) = self.get_goal(goal_id)? {
                    if let Some(template_id) =
                        goal.workflow_template_id.as_deref().and_then(non_empty)
                    {
                        return self
                            .get_domain_workflow_template(
                                template_id,
                                goal.workflow_template_version.as_deref(),
                            )?
                            .ok_or_else(|| {
                                anyhow!("domain workflow template not found: {template_id}")
                            });
                    }
                }
            }
        }
        let inferred_domain = explicit_domain
            .or_else(|| infer_domain_from_quality_input(self, session_id, goal_id, input).ok());
        let domain = inferred_domain.unwrap_or_else(|| "writing".to_string());
        let templates = self.list_domain_workflow_templates(ListDomainWorkflowTemplatesInput {
            domain: Some(domain.clone()),
            task_type: None,
            project_id: None,
            include_disabled: false,
            limit: Some(1),
        })?;
        templates
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("no domain workflow template available for domain {domain}"))
    }

    fn create_domain_quality_run(&self, context: &QualityContext) -> Result<DomainQualityRun> {
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
            }),
        );
        emit_domain_quality_run("domain_quality:created", &run);
        Ok(run)
    }

    fn complete_domain_quality_run(
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

    fn fail_domain_quality_run(
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

fn infer_domain_from_quality_input(
    db: &SessionDB,
    session_id: &str,
    goal_id: Option<&str>,
    input: &RunDomainQualityInput,
) -> Result<String> {
    if let Some(kind) = input.artifact_kind.as_deref().and_then(non_empty) {
        let domain = infer_domain_from_text(kind);
        if domain != "writing" || kind.to_ascii_lowercase().contains("write") {
            return Ok(domain);
        }
    }
    if let Some(goal_id) = goal_id {
        if let Some(goal) = db.get_goal(goal_id)? {
            if let Some(domain) = goal.domain.as_deref().and_then(non_empty) {
                return Ok(normalize_domain(domain));
            }
            let text = format!("{} {}", goal.objective, goal.completion_criteria);
            let lower = text.to_ascii_lowercase();
            let domain = infer_domain_from_text(&text);
            if domain != "writing"
                || contains_any(
                    &lower,
                    &[
                        "writing", "write", "doc", "memo", "report", "文档", "写作", "报告",
                    ],
                )
            {
                return Ok(domain);
            }
        }
    }
    let evidence = db.list_domain_evidence(ListDomainEvidenceInput {
        goal_id: goal_id.map(str::to_string),
        session_id: Some(session_id.to_string()),
        project_id: None,
        domain: None,
        evidence_type: None,
        limit: Some(50),
    })?;
    let mut counts = BTreeMap::<String, usize>::new();
    for item in evidence {
        *counts.entry(item.domain).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(domain, _)| domain)
        .ok_or_else(|| anyhow!("no domain signal available"))
}

fn infer_domain_from_text(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    if contains_any(
        &lower,
        &[
            "data",
            "metric",
            "kpi",
            "dashboard",
            "chart",
            "数据",
            "指标",
            "图表",
        ],
    ) {
        "data_analysis".to_string()
    } else if contains_any(
        &lower,
        &[
            "email", "inbox", "reply", "message", "gmail", "邮件", "收件", "回复",
        ],
    ) {
        "inbox".to_string()
    } else if contains_any(
        &lower,
        &[
            "meeting", "calendar", "agenda", "attendee", "会议", "日程", "议程",
        ],
    ) {
        "meeting_prep".to_string()
    } else if contains_any(
        &lower,
        &["knowledge", "note", "vault", "obsidian", "知识", "笔记"],
    ) {
        "knowledge_curation".to_string()
    } else if contains_any(
        &lower,
        &[
            "research", "source", "citation", "cite", "调研", "研究", "引用", "资料",
        ],
    ) {
        "research".to_string()
    } else if contains_any(
        &lower,
        &[
            "project", "status", "owner", "deadline", "risk", "项目", "进度", "风险",
        ],
    ) {
        "project_ops".to_string()
    } else {
        "writing".to_string()
    }
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn build_quality_checks(context: &QualityContext) -> Vec<CandidateCheck> {
    let mut checks = Vec::new();
    let counts = evidence_counts(&context.evidence);
    for req in &context.template.required_evidence {
        checks.push(requirement_check(
            req,
            counts.get(&req.evidence_type).copied().unwrap_or(0),
        ));
    }
    checks.extend(domain_profile_checks(context, &counts));
    checks.extend(approval_gate_checks(context));
    dedup_checks(&mut checks);
    checks
}

fn requirement_check(req: &DomainEvidenceRequirement, actual: usize) -> CandidateCheck {
    let min = req.min_count.unwrap_or(1).max(1);
    let ok = actual >= min;
    let required = req.required;
    let status = if ok {
        DomainQualityCheckStatus::Passed
    } else if required {
        DomainQualityCheckStatus::Failed
    } else {
        DomainQualityCheckStatus::Advisory
    };
    let severity = if ok {
        DomainQualitySeverity::P3
    } else if required {
        DomainQualitySeverity::P1
    } else {
        DomainQualitySeverity::P3
    };
    CandidateCheck {
        check_type: "verification".to_string(),
        profile: "required_evidence".to_string(),
        title: req.title.clone(),
        body: if ok {
            format!("{actual}/{min} required evidence item(s) are present.")
        } else if required {
            format!(
                "Missing required evidence: {actual}/{min} item(s) for {}.",
                req.evidence_type
            )
        } else {
            format!(
                "Optional evidence is thin: {actual}/{min} item(s) for {}.",
                req.evidence_type
            )
        },
        severity,
        status,
        evidence_type: Some(req.evidence_type.clone()),
        source_metadata: json!({
            "actual": actual,
            "required": min,
            "requiredEvidence": required,
            "metadataKeys": req.metadata_keys,
        }),
    }
}

fn domain_profile_checks(
    context: &QualityContext,
    counts: &BTreeMap<String, usize>,
) -> Vec<CandidateCheck> {
    match context.domain.as_str() {
        "research" => vec![
            threshold_check("verification", "research", "Key claims have source support", counts, "claim_checked", 2),
            threshold_check("verification", "research", "Sources are cited", counts, "source_cited", 3),
            threshold_check("review", "research", "Citation audit is complete", counts, "citation_audited", 1),
            dated_sources_check(context),
        ],
        "writing" => vec![
            threshold_check("review", "writing", "Draft artifact exists", counts, "artifact_created", 1),
            threshold_check("review", "writing", "Draft reviewed against audience and requirements", counts, "artifact_reviewed", 1),
            advisory_check("review", "writing", "Terminology and audience fit need human judgment", "Review notes should explicitly cover audience, terminology, unmet requirements, and citation gaps."),
        ],
        "data_analysis" => vec![
            threshold_check("verification", "data_analysis", "Data quality checked", counts, "data_quality_checked", 1),
            threshold_check("verification", "data_analysis", "Metric interpretation checked", counts, "claim_checked", 1),
            data_quality_metadata_check(context),
        ],
        "meeting_prep" => vec![
            threshold_check("review", "meeting_prep", "Meeting context collected", counts, "meeting_context_collected", 1),
            threshold_check("review", "meeting_prep", "Brief or agenda exists", counts, "artifact_created", 1),
            advisory_check("review", "meeting_prep", "Decision points and risks surfaced", "Open decisions, risks, missing materials, and unread attachments should be explicit before the meeting."),
        ],
        "inbox" => vec![
            threshold_check("review", "inbox", "Thread or message source cited", counts, "source_cited", 1),
            threshold_check("verification", "inbox", "Facts and commitments checked", counts, "claim_checked", 1),
            threshold_check("approval", "inbox", "Message draft approved before send", counts, "message_draft_approved", 1),
        ],
        "knowledge_curation" => vec![
            threshold_check("verification", "knowledge_curation", "Source notes identified", counts, "source_cited", 2),
            threshold_check("review", "knowledge_curation", "Deduplication and gap review completed", counts, "artifact_reviewed", 1),
            threshold_check("review", "knowledge_curation", "Curated note or index exists", counts, "artifact_created", 1),
        ],
        "project_ops" => vec![
            threshold_check("review", "project_ops", "Status or plan artifact exists", counts, "artifact_created", 1),
            threshold_check("verification", "project_ops", "Risks and dependencies checked", counts, "claim_checked", 1),
            threshold_check("approval", "project_ops", "Owners and tradeoffs confirmed", counts, "user_decision", 1),
        ],
        _ => vec![advisory_check(
            "review",
            &context.domain,
            "General domain review",
            "No specialized profile matched; rely on required evidence and approval gates.",
        )],
    }
}

fn threshold_check(
    check_type: &str,
    profile: &str,
    title: &str,
    counts: &BTreeMap<String, usize>,
    evidence_type: &str,
    min: usize,
) -> CandidateCheck {
    let actual = counts.get(evidence_type).copied().unwrap_or(0);
    let ok = actual >= min;
    CandidateCheck {
        check_type: check_type.to_string(),
        profile: profile.to_string(),
        title: title.to_string(),
        body: if ok {
            format!("{actual}/{min} evidence item(s) satisfy {evidence_type}.")
        } else {
            format!("Need at least {min} {evidence_type} evidence item(s); found {actual}.")
        },
        severity: if ok {
            DomainQualitySeverity::P3
        } else {
            DomainQualitySeverity::P1
        },
        status: if ok {
            DomainQualityCheckStatus::Passed
        } else {
            DomainQualityCheckStatus::Failed
        },
        evidence_type: Some(evidence_type.to_string()),
        source_metadata: json!({ "actual": actual, "required": min }),
    }
}

fn advisory_check(check_type: &str, profile: &str, title: &str, body: &str) -> CandidateCheck {
    CandidateCheck {
        check_type: check_type.to_string(),
        profile: profile.to_string(),
        title: title.to_string(),
        body: body.to_string(),
        severity: DomainQualitySeverity::P3,
        status: DomainQualityCheckStatus::Advisory,
        evidence_type: None,
        source_metadata: json!({}),
    }
}

fn dated_sources_check(context: &QualityContext) -> CandidateCheck {
    let sources = context
        .evidence
        .iter()
        .filter(|item| item.evidence_type == "source_cited")
        .collect::<Vec<_>>();
    let dated = sources
        .iter()
        .filter(|item| {
            item.source_metadata.get("retrievedAt").is_some()
                || item.source_metadata.get("publishedAt").is_some()
                || item.source_metadata.get("date").is_some()
        })
        .count();
    let status = if sources.is_empty() {
        DomainQualityCheckStatus::Failed
    } else if dated == sources.len() {
        DomainQualityCheckStatus::Passed
    } else {
        DomainQualityCheckStatus::Advisory
    };
    CandidateCheck {
        check_type: "verification".to_string(),
        profile: "research".to_string(),
        title: "Source freshness is visible".to_string(),
        body: if status == DomainQualityCheckStatus::Passed {
            "Every cited source includes retrieved/published/date metadata.".to_string()
        } else if status == DomainQualityCheckStatus::Failed {
            "No cited sources are available, so freshness cannot be checked.".to_string()
        } else {
            format!(
                "{dated}/{} cited source(s) include date metadata.",
                sources.len()
            )
        },
        severity: if status == DomainQualityCheckStatus::Failed {
            DomainQualitySeverity::P1
        } else {
            DomainQualitySeverity::P3
        },
        status,
        evidence_type: Some("source_cited".to_string()),
        source_metadata: json!({ "sources": sources.len(), "dated": dated }),
    }
}

fn data_quality_metadata_check(context: &QualityContext) -> CandidateCheck {
    let quality_items = context
        .evidence
        .iter()
        .filter(|item| item.evidence_type == "data_quality_checked")
        .collect::<Vec<_>>();
    let has_definition = quality_items.iter().any(|item| {
        item.source_metadata.get("dataset").is_some()
            || item.source_metadata.get("metric").is_some()
            || item.source_metadata.get("denominator").is_some()
            || item.source_metadata.get("sampleSize").is_some()
    });
    CandidateCheck {
        check_type: "verification".to_string(),
        profile: "data_analysis".to_string(),
        title: "Metric definition and sample context are visible".to_string(),
        body: if has_definition {
            "Data quality evidence includes dataset, metric, denominator, or sample-size metadata."
                .to_string()
        } else {
            "Add dataset, metric definition, denominator, sample size, outlier, or chart-review metadata to the data quality evidence.".to_string()
        },
        severity: if has_definition {
            DomainQualitySeverity::P3
        } else {
            DomainQualitySeverity::P2
        },
        status: if has_definition {
            DomainQualityCheckStatus::Passed
        } else {
            DomainQualityCheckStatus::Advisory
        },
        evidence_type: Some("data_quality_checked".to_string()),
        source_metadata: json!({ "qualityEvidence": quality_items.len(), "hasDefinition": has_definition }),
    }
}

fn approval_gate_checks(context: &QualityContext) -> Vec<CandidateCheck> {
    let requested_action = context
        .input
        .source_metadata
        .get("requestedAction")
        .or_else(|| context.input.source_metadata.get("action"))
        .and_then(Value::as_str)
        .map(normalize_domain);
    let high_risk_requested = context
        .input
        .source_metadata
        .get("highRiskAction")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    context
        .template
        .approval_gates
        .iter()
        .filter(|gate| gate.required)
        .map(|gate| {
            let action = normalize_domain(&gate.action);
            let applies = high_risk_requested
                || requested_action
                    .as_ref()
                    .map(|requested| requested == &action)
                    .unwrap_or(false);
            let status = if context.input.explicit_user_approval {
                DomainQualityCheckStatus::Passed
            } else if applies {
                DomainQualityCheckStatus::NeedsUser
            } else {
                DomainQualityCheckStatus::Advisory
            };
            CandidateCheck {
                check_type: "approval".to_string(),
                profile: "approval_gate".to_string(),
                title: format!("Approval gate: {}", gate.action),
                body: if status == DomainQualityCheckStatus::Passed {
                    "Explicit user approval was provided for this quality run.".to_string()
                } else if status == DomainQualityCheckStatus::NeedsUser {
                    format!(
                        "{} This high-risk action must not proceed without user confirmation.",
                        gate.reason
                    )
                } else {
                    format!(
                        "{} No matching high-risk action is currently requested.",
                        gate.reason
                    )
                },
                severity: if status == DomainQualityCheckStatus::NeedsUser {
                    DomainQualitySeverity::P0
                } else {
                    DomainQualitySeverity::P3
                },
                status,
                evidence_type: Some("user_decision".to_string()),
                source_metadata: json!({
                    "gate": gate.action,
                    "applies": applies,
                    "requestedAction": requested_action,
                    "highRiskAction": high_risk_requested,
                    "explicitUserApproval": context.input.explicit_user_approval,
                }),
            }
        })
        .collect()
}

fn build_quality_stats(context: &QualityContext, checks: &[CandidateCheck]) -> Value {
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut blocked = 0usize;
    let mut needs_user = 0usize;
    let mut advisory = 0usize;
    for check in checks {
        match check.status {
            DomainQualityCheckStatus::Passed => passed += 1,
            DomainQualityCheckStatus::Failed => failed += 1,
            DomainQualityCheckStatus::Blocked => blocked += 1,
            DomainQualityCheckStatus::NeedsUser => needs_user += 1,
            DomainQualityCheckStatus::Advisory => advisory += 1,
        }
    }
    json!({
        "domain": context.domain,
        "templateId": context.template.id,
        "templateVersion": context.template.version,
        "templateTitle": context.template.title,
        "profiles": active_profiles(&context.input.profiles, &context.domain),
        "evidence": evidence_counts(&context.evidence),
        "checks": checks.len(),
        "passed": passed,
        "failed": failed,
        "blocked": blocked,
        "needsUser": needs_user,
        "advisory": advisory,
        "artifact": {
            "title": context.input.artifact_title,
            "kind": context.input.artifact_kind,
        },
    })
}

fn summarize_quality(
    context: &QualityContext,
    checks: &[CandidateCheck],
) -> (DomainQualityRunState, String) {
    let blocking = checks
        .iter()
        .filter(|check| check.status.blocks_goal() && check.severity.is_blocking())
        .count();
    let needs_user = checks
        .iter()
        .filter(|check| check.status == DomainQualityCheckStatus::NeedsUser)
        .count();
    let failed = checks
        .iter()
        .filter(|check| check.status == DomainQualityCheckStatus::Failed)
        .count();
    if blocking > 0 && needs_user > 0 && failed == 0 {
        (
            DomainQualityRunState::NeedsUser,
            format!(
                "{} quality needs user confirmation ({} gate/check).",
                context.template.title, needs_user
            ),
        )
    } else if blocking > 0 {
        (
            DomainQualityRunState::Blocked,
            format!(
                "{} quality blocked by {} required check(s).",
                context.template.title, blocking
            ),
        )
    } else {
        (
            DomainQualityRunState::Completed,
            format!(
                "{} quality checks passed with {} advisory note(s).",
                context.template.title,
                checks
                    .iter()
                    .filter(|check| check.status == DomainQualityCheckStatus::Advisory)
                    .count()
            ),
        )
    }
}

fn evidence_counts(evidence: &[DomainEvidenceItem]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for item in evidence {
        *counts.entry(item.evidence_type.clone()).or_default() += 1;
    }
    counts
}

fn check_blocks_goal(check: &DomainQualityCheck) -> bool {
    check.severity.is_blocking() && check.status.blocks_goal()
}

fn active_profiles(requested: &[String], domain: &str) -> Vec<String> {
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

fn dedup_checks(checks: &mut Vec<CandidateCheck>) {
    let mut seen = Vec::<String>::new();
    checks.retain(|check| {
        let key = format!(
            "{}:{}:{}:{}",
            check.check_type,
            check.profile,
            check.title,
            check.evidence_type.as_deref().unwrap_or("")
        );
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
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

fn normalize_domain(value: &str) -> String {
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

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goal::{CreateGoalInput, GoalState};
    use crate::session::SessionDB;
    use tempfile::tempdir;

    fn test_db() -> (tempfile::TempDir, SessionDB) {
        let dir = tempdir().expect("tempdir");
        let db = SessionDB::open(&dir.path().join("sessions.db")).expect("open db");
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
    fn research_quality_blocks_missing_required_evidence() {
        let (_dir, db) = test_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "完成 research brief".to_string(),
                completion_criteria: "引用、claim check、citation audit 都齐全".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");

        let snapshot = db
            .run_domain_quality_for_session(RunDomainQualityInput {
                session_id: session.id.clone(),
                goal_id: Some(goal.goal.id.clone()),
                domain: Some("research".to_string()),
                ..Default::default()
            })
            .expect("run quality");

        assert_eq!(snapshot.run.state, DomainQualityRunState::Blocked);
        assert!(snapshot
            .checks
            .iter()
            .any(
                |check| check.evidence_type.as_deref() == Some("source_cited")
                    && check.status == DomainQualityCheckStatus::Failed
            ));
        let goal = db
            .goal_snapshot(&goal.goal.id, 100)
            .expect("goal snapshot")
            .expect("goal exists");
        assert_eq!(goal.goal.state, GoalState::Blocked);
        assert!(goal.evidence.iter().any(|item| {
            item.relation == "domain_quality_blocked" || item.relation == "domain_quality_check"
        }));
    }

    #[test]
    fn domain_quality_prefers_goal_workflow_template() {
        let (_dir, db) = test_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Prepare a sourced brief".to_string(),
                completion_criteria: "Draft is reviewed and ready to share".to_string(),
                domain: None,
                workflow_template_id: Some("writing-brief".to_string()),
                workflow_template_version: None,
                workflow_task_type: Some("prd".to_string()),
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");

        let snapshot = db
            .run_domain_quality_for_session(RunDomainQualityInput {
                session_id: session.id,
                goal_id: Some(goal.goal.id),
                ..Default::default()
            })
            .expect("run quality");

        assert_eq!(snapshot.run.domain, "writing");
        assert_eq!(snapshot.run.template_id.as_deref(), Some("writing-brief"));
        assert_eq!(snapshot.run.template_version.as_deref(), Some("1.0.0"));
        assert_eq!(
            snapshot
                .run
                .stats
                .get("templateVersion")
                .and_then(Value::as_str),
            Some("1.0.0")
        );
    }

    #[test]
    fn inbox_send_quality_requires_user_confirmation() {
        let (_dir, db) = test_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "draft email reply".to_string(),
                completion_criteria: "recipient facts checked and send approved".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");

        let snapshot = db
            .run_domain_quality_for_session(RunDomainQualityInput {
                session_id: session.id,
                goal_id: Some(goal.goal.id),
                domain: Some("inbox".to_string()),
                source_metadata: json!({
                    "requestedAction": "send_message",
                    "highRiskAction": true,
                }),
                ..Default::default()
            })
            .expect("run quality");

        assert!(snapshot.checks.iter().any(|check| {
            check.check_type == "approval"
                && check.status == DomainQualityCheckStatus::NeedsUser
                && check.severity == DomainQualitySeverity::P0
        }));
        assert!(matches!(
            snapshot.run.state,
            DomainQualityRunState::NeedsUser | DomainQualityRunState::Blocked
        ));
    }
}
