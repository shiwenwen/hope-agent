//! General-domain eval and quality gate control plane.
//!
//! Coding eval remains coding-shaped and benchmark-oriented. This module keeps
//! non-coding eval separate: built-in domain tasks, deterministic trace scoring,
//! durable domain eval run history, and a domain quality gate that reads domain
//! eval + domain quality evidence without mixing it into coding benchmark score.

use anyhow::{anyhow, bail, Result};
use chrono::{Duration, Utc};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

use crate::domain_quality::{
    DomainQualityCheckStatus, DomainQualityRunSnapshot, DomainQualityRunState,
};
use crate::domain_workflow::ListDomainEvidenceInput;
use crate::session::SessionDB;
use crate::util::now_rfc3339;

const DEFAULT_WINDOW_DAYS: u32 = 30;
const MAX_WINDOW_DAYS: u32 = 180;
const DEFAULT_DOMAIN_EVAL_LIMIT: usize = 20;
const MAX_DOMAIN_EVAL_LIMIT: usize = 100;
const DEFAULT_MIN_EVAL_RUNS: usize = 1;
const DEFAULT_MIN_PASS_RATE: f64 = 1.0;
const DEFAULT_MIN_AVERAGE_SCORE: f64 = 0.8;
const DEFAULT_MIN_QUALITY_RUNS: usize = 1;
const DEFAULT_MAX_BLOCKED_QUALITY_RUNS: usize = 0;
const DEFAULT_MIN_DOMAIN_COVERAGE: usize = 1;

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
    pub limit: Option<usize>,
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

struct DomainGateScope {
    scope: String,
    session_id: Option<String>,
    project_id: Option<String>,
    domain: Option<String>,
    window_days: u32,
    since: String,
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
            ON domain_eval_runs(status, created_at DESC);",
    )?;
    Ok(())
}

impl SessionDB {
    pub fn list_domain_eval_tasks(
        &self,
        input: ListDomainEvalTasksInput,
    ) -> Result<Vec<DomainEvalTask>> {
        let domain = input.domain.as_deref().map(normalize_domain);
        let limit = input
            .limit
            .unwrap_or(usize::MAX)
            .clamp(1, MAX_DOMAIN_EVAL_LIMIT);
        Ok(built_in_domain_eval_tasks()
            .into_iter()
            .filter(|task| {
                domain
                    .as_deref()
                    .map(|domain| task.domain == domain)
                    .unwrap_or(true)
            })
            .take(limit)
            .collect())
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
        let task = built_in_domain_eval_tasks()
            .into_iter()
            .find(|task| task.id == task_id)
            .ok_or_else(|| anyhow!("domain eval task not found: {task_id}"))?;
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
            report,
            source_quality_run_id,
            created_at: now,
        };
        let report_json = serde_json::to_string(&record.report)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO domain_eval_runs (
                id, session_id, project_id, task_id, task_version, domain, label,
                status, score, report_json, source_quality_run_id, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
                report_json,
                record.source_quality_run_id,
                record.created_at,
            ],
        )?;
        drop(conn);
        self.get_domain_eval_run(&id)?
            .ok_or_else(|| anyhow!("domain eval run vanished after insert: {id}"))
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
        params.push(limit.to_string());
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(&format!(
            "SELECT der.id, der.session_id, der.project_id, der.task_id, der.task_version,
                    der.domain, der.label, der.status, der.score, der.report_json,
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

    fn get_domain_eval_run(&self, run_id: &str) -> Result<Option<DomainEvalRunRecord>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, session_id, project_id, task_id, task_version, domain, label,
                    status, score, report_json, source_quality_run_id, created_at
             FROM domain_eval_runs
             WHERE id = ?1",
            params![run_id],
            row_to_domain_eval_run,
        )
        .optional()
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
            });
        }
        Ok(DomainGateScope {
            scope: "global".to_string(),
            session_id: None,
            project_id: None,
            domain,
            window_days,
            since,
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
    let report_json: String = row.get(9)?;
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
        report,
        source_quality_run_id: row.get(10)?,
        created_at: row.get(11)?,
    })
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

fn since_timestamp(window_days: u32) -> String {
    (Utc::now() - Duration::days(window_days as i64)).to_rfc3339()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain_quality::RunDomainQualityInput;
    use crate::domain_workflow::RecordDomainEvidenceInput;

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
            worktree_id: None,
        })
        .unwrap();

        let run = db
            .run_domain_eval_task(RunDomainEvalTaskInput {
                session_id: session.id,
                task_id: "research-source-backed-brief".to_string(),
                label: None,
                source_quality_run_id: None,
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
}
