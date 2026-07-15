//! Goal / Workflow / Loop / Task / Plan control-plane analytics.
//!
//! This module deliberately keeps result metrics (Goal), execution drivers
//! (Workflow / Loop), and progress signals (Task / Plan) separate. There is no
//! inferred funnel: Task and Plan do not yet carry reliable control-plane
//! foreign keys.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params_from_iter, types::Value as SqlValue, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::plan::{list_all_plans, PlanIndexFilter, PlanModeState};
use crate::session::SessionDB;

/// Wire value used by the project picker for sessions without a project.
pub const CONTROL_PLANE_UNASSIGNED_PROJECT: &str = "__unassigned__";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlPlaneDashboardFilter {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RatioMetric {
    pub numerator: u64,
    pub denominator: u64,
    pub rate: Option<f64>,
}

impl RatioMetric {
    fn new(numerator: u64, denominator: u64) -> Self {
        Self {
            numerator,
            denominator,
            rate: (denominator > 0).then_some(numerator as f64 / denominator as f64),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DurationMetric {
    pub p50_secs: Option<f64>,
    pub sample_count: u64,
    /// Rows that conceptually qualify for the metric, including legacy rows
    /// lacking an exact timestamp.
    pub eligible_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedCount {
    pub key: String,
    pub count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendPoint {
    pub date: String,
    pub created: u64,
    pub completed: u64,
    pub accepted: u64,
    pub resolved: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlPlaneSummary {
    pub goal_acceptance: RatioMetric,
    pub workflow_completion: RatioMetric,
    pub loop_strong_progress: RatioMetric,
    pub task_cohort_completion: RatioMetric,
    pub attention_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalDashboardStats {
    pub acceptance: RatioMetric,
    pub required_criteria: RatioMetric,
    pub audited_goal_count: u64,
    pub accepted_duration: DurationMetric,
    pub current_states: Vec<NamedCount>,
    pub closure_outcomes: Vec<NamedCount>,
    pub domains: Vec<NamedCount>,
    pub trend: Vec<TrendPoint>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDashboardStats {
    pub completion: RatioMetric,
    pub op_failure: RatioMetric,
    pub duration: DurationMetric,
    pub goal_binding: RatioMetric,
    pub approval_trigger: RatioMetric,
    pub current_states: Vec<NamedCount>,
    pub kinds: Vec<NamedCount>,
    pub origins: Vec<NamedCount>,
    pub trend: Vec<TrendPoint>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopDashboardStats {
    pub strong_progress: RatioMetric,
    pub no_progress: RatioMetric,
    pub duration: DurationMetric,
    pub current_blocked_schedules: u64,
    pub progress_states: Vec<NamedCount>,
    pub trigger_kinds: Vec<NamedCount>,
    pub strategies: Vec<NamedCount>,
    pub current_states: Vec<NamedCount>,
    pub trend: Vec<TrendPoint>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDashboardStats {
    pub cohort_completion: RatioMetric,
    pub current_backlog: u64,
    pub duration: DurationMetric,
    pub current_states: Vec<NamedCount>,
    pub trend: Vec<TrendPoint>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDashboardStats {
    pub cohort_completion: RatioMetric,
    pub active_now: u64,
    pub duration: DurationMetric,
    pub current_states: Vec<NamedCount>,
    pub by_agent: Vec<NamedCount>,
    pub by_project: Vec<NamedCount>,
    pub trend: Vec<TrendPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionItem {
    pub kind: String,
    pub id: String,
    pub session_id: String,
    pub title: String,
    pub status: String,
    pub reason: String,
    pub severity: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionDashboard {
    pub total: u64,
    pub items: Vec<AttentionItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlPlaneDashboard {
    pub summary: ControlPlaneSummary,
    pub goals: GoalDashboardStats,
    pub workflows: WorkflowDashboardStats,
    pub loops: LoopDashboardStats,
    pub tasks: TaskDashboardStats,
    pub plans: PlanDashboardStats,
    pub attention: AttentionDashboard,
}

#[derive(Default)]
struct Scope {
    clauses: Vec<String>,
    params: Vec<SqlValue>,
}

impl Scope {
    fn for_sessions(filter: &ControlPlaneDashboardFilter, alias: &str) -> Self {
        let mut scope = Self::default();
        scope.clauses.push(format!("{alias}.incognito = 0"));
        scope.clauses.push(format!("{alias}.is_cron = 0"));
        scope
            .clauses
            .push(format!("{alias}.parent_session_id IS NULL"));
        if let Some(agent_id) = present(filter.agent_id.as_deref()) {
            scope.clauses.push(format!("{alias}.agent_id = ?"));
            scope.params.push(agent_id.to_string().into());
        }
        if let Some(project_id) = present(filter.project_id.as_deref()) {
            if project_id == CONTROL_PLANE_UNASSIGNED_PROJECT {
                scope.clauses.push(format!("{alias}.project_id IS NULL"));
            } else {
                scope.clauses.push(format!("{alias}.project_id = ?"));
                scope.params.push(project_id.to_string().into());
            }
        }
        scope
    }

    fn window(mut self, filter: &ControlPlaneDashboardFilter, expr: &str) -> Self {
        if let Some(start) = present(filter.start_date.as_deref()) {
            self.clauses
                .push(format!("datetime({expr}) >= datetime(?)"));
            self.params.push(start.to_string().into());
        }
        if let Some(end) = present(filter.end_date.as_deref()) {
            self.clauses
                .push(format!("datetime({expr}) <= datetime(?)"));
            self.params.push(end.to_string().into());
        }
        self
    }

    fn sql(&self) -> String {
        if self.clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", self.clauses.join(" AND "))
        }
    }
}

fn present(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

fn count_groups(
    conn: &Connection,
    select_expr: &str,
    from_sql: &str,
    scope: &Scope,
) -> Result<Vec<NamedCount>> {
    let sql = format!(
        "SELECT COALESCE({select_expr}, 'unassigned'), COUNT(*) {from_sql} {} GROUP BY 1 ORDER BY 2 DESC, 1 ASC",
        scope.sql()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(scope.params.iter()), |row| {
        Ok(NamedCount {
            key: row.get(0)?,
            count: crate::sql_u64(row, 1)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 0 {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    })
}

fn parse_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn in_window(ts: &str, filter: &ControlPlaneDashboardFilter) -> bool {
    let Some(ts) = parse_utc(ts) else {
        return false;
    };
    if let Some(start) = present(filter.start_date.as_deref()).and_then(parse_utc) {
        if ts < start {
            return false;
        }
    }
    if let Some(end) = present(filter.end_date.as_deref()).and_then(parse_utc) {
        if ts > end {
            return false;
        }
    }
    true
}

fn push_trend(
    map: &mut HashMap<String, TrendPoint>,
    timestamp: &str,
    field: impl FnOnce(&mut TrendPoint),
) {
    let date = timestamp.get(..10).unwrap_or(timestamp).to_string();
    let point = map.entry(date.clone()).or_insert_with(|| TrendPoint {
        date,
        ..Default::default()
    });
    field(point);
}

fn sorted_trend(map: HashMap<String, TrendPoint>) -> Vec<TrendPoint> {
    let mut points = map.into_values().collect::<Vec<_>>();
    points.sort_by(|a, b| a.date.cmp(&b.date));
    points
}

fn query_goals(
    conn: &Connection,
    filter: &ControlPlaneDashboardFilter,
) -> Result<GoalDashboardStats> {
    let current_scope = Scope::for_sessions(filter, "s");
    let current_states = count_groups(
        conn,
        "g.state",
        "FROM goals g JOIN sessions s ON s.id = g.session_id",
        &current_scope,
    )?;
    let domains = count_groups(
        conn,
        "NULLIF(g.domain, '')",
        "FROM goals g JOIN sessions s ON s.id = g.session_id",
        &current_scope,
    )?;

    let mut terminal_scope = Scope::for_sessions(filter, "s");
    terminal_scope.clauses.push(
        "((g.closure_decision IN ('accepted_v1','cancelled','superseded') AND g.closed_at IS NOT NULL) OR g.state = 'failed')".into(),
    );
    terminal_scope = terminal_scope.window(filter, "COALESCE(g.closed_at, g.completed_at)");
    let sql = format!(
        "SELECT g.id, g.state, g.closure_decision, g.created_at,
                COALESCE(g.closed_at, g.completed_at), g.revision,
                g.final_evidence_json,
                COALESCE((SELECT MAX(ge.seq) FROM goal_events ge WHERE ge.goal_id = g.id AND ge.kind = 'goal_linked'), 0)
         FROM goals g JOIN sessions s ON s.id = g.session_id {}",
        terminal_scope.sql()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(terminal_scope.params.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, i64>(7)?,
        ))
    })?;

    let mut resolved = 0_u64;
    let mut accepted = 0_u64;
    let mut accepted_durations = Vec::new();
    let mut closure_counts: HashMap<String, u64> = HashMap::new();
    let mut trend = HashMap::new();
    // Criteria are read from the current revision's latest final audit. The
    // same row set is sufficient for terminal goals; a second scan below adds
    // non-terminal evaluated goals so the diagnostic is not closure-biased.
    for row in rows {
        let (_id, state, decision, created_at, resolved_at, _revision, _audit, _latest_seq) = row?;
        resolved += 1;
        let outcome = decision.clone().unwrap_or_else(|| state.clone());
        *closure_counts.entry(outcome).or_default() += 1;
        if decision.as_deref() == Some("accepted_v1") {
            accepted += 1;
            if let (Some(start), Some(end)) = (
                parse_utc(&created_at),
                resolved_at.as_deref().and_then(parse_utc),
            ) {
                let seconds = (end - start).num_milliseconds() as f64 / 1000.0;
                if seconds >= 0.0 {
                    accepted_durations.push(seconds);
                }
            }
        }
        if let Some(resolved_at) = resolved_at {
            push_trend(&mut trend, &resolved_at, |point| {
                point.resolved += 1;
                if decision.as_deref() == Some("accepted_v1") {
                    point.accepted += 1;
                }
            });
        }
    }

    let mut criteria_numerator = 0_u64;
    let mut criteria_denominator = 0_u64;
    let mut audited_goal_count = 0_u64;
    let audit_sql = format!(
        "SELECT g.revision, g.final_evidence_json,
                COALESCE((SELECT MAX(ge.seq) FROM goal_events ge WHERE ge.goal_id = g.id AND ge.kind = 'goal_linked'), 0)
         FROM goals g JOIN sessions s ON s.id = g.session_id
         {}",
        current_scope.sql()
    );
    let mut audit_stmt = conn.prepare(&audit_sql)?;
    let audit_rows =
        audit_stmt.query_map(params_from_iter(current_scope.params.iter()), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
    for row in audit_rows {
        let (revision, audit_json, latest_link_seq) = row?;
        let Ok(audit) = serde_json::from_str::<Value>(&audit_json) else {
            continue;
        };
        if audit.get("goalRevision").and_then(Value::as_i64) != Some(revision)
            || audit
                .get("goalLinkedEventSeq")
                .and_then(Value::as_i64)
                .unwrap_or(-1)
                < latest_link_seq
            || audit.get("auditStale").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(criteria) = audit.get("criteriaStatus").and_then(Value::as_array) else {
            continue;
        };
        let mut sampled = false;
        for criterion in criteria {
            if criterion.get("kind").and_then(Value::as_str) != Some("required") {
                continue;
            }
            let Some(status) = criterion.get("status").and_then(Value::as_str) else {
                continue;
            };
            if matches!(status, "satisfied" | "missing" | "blocked") {
                sampled = true;
                criteria_denominator += 1;
                if status == "satisfied" {
                    criteria_numerator += 1;
                }
            }
        }
        if sampled {
            audited_goal_count += 1;
        }
    }

    let mut closure_outcomes = closure_counts
        .into_iter()
        .map(|(key, count)| NamedCount { key, count })
        .collect::<Vec<_>>();
    closure_outcomes.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
    let accepted_sample_count = accepted_durations.len() as u64;
    let accepted_p50 = median(&mut accepted_durations);
    Ok(GoalDashboardStats {
        acceptance: RatioMetric::new(accepted, resolved),
        required_criteria: RatioMetric::new(criteria_numerator, criteria_denominator),
        audited_goal_count,
        accepted_duration: DurationMetric {
            p50_secs: accepted_p50,
            sample_count: accepted_sample_count,
            eligible_count: accepted,
        },
        current_states,
        closure_outcomes,
        domains,
        trend: sorted_trend(trend),
    })
}

fn query_workflows(
    conn: &Connection,
    filter: &ControlPlaneDashboardFilter,
) -> Result<WorkflowDashboardStats> {
    let current_scope = Scope::for_sessions(filter, "s");
    let current_states = count_groups(
        conn,
        "w.state",
        "FROM workflow_runs w JOIN sessions s ON s.id = w.session_id",
        &current_scope,
    )?;
    let mut terminal_scope = Scope::for_sessions(filter, "s");
    terminal_scope
        .clauses
        .push("w.state IN ('completed','failed','blocked')".into());
    terminal_scope = terminal_scope.window(filter, "w.completed_at");
    let kinds = count_groups(
        conn,
        "w.kind",
        "FROM workflow_runs w JOIN sessions s ON s.id = w.session_id",
        &terminal_scope,
    )?;
    let origins = count_groups(
        conn,
        "NULLIF(w.origin, '')",
        "FROM workflow_runs w JOIN sessions s ON s.id = w.session_id",
        &terminal_scope,
    )?;
    let sql = format!(
        "SELECT w.id, w.state, w.created_at, w.completed_at, w.goal_id,
                EXISTS(SELECT 1 FROM workflow_events e
                       WHERE e.run_id = w.id AND e.type = 'run_state_changed'
                         AND json_extract(e.payload_json, '$.to') = 'awaiting_approval')
         FROM workflow_runs w JOIN sessions s ON s.id = w.session_id {}",
        terminal_scope.sql()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(terminal_scope.params.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, bool>(5)?,
        ))
    })?;
    let mut total = 0_u64;
    let mut completed = 0_u64;
    let mut bound = 0_u64;
    let mut approval = 0_u64;
    let mut durations = Vec::new();
    let mut trend = HashMap::new();
    for row in rows {
        let (_id, state, created_at, completed_at, goal_id, approval_seen) = row?;
        total += 1;
        completed += u64::from(state == "completed");
        bound += u64::from(goal_id.is_some());
        approval += u64::from(approval_seen);
        if let (Some(start), Some(end)) = (
            parse_utc(&created_at),
            completed_at.as_deref().and_then(parse_utc),
        ) {
            let seconds = (end - start).num_milliseconds() as f64 / 1000.0;
            if seconds >= 0.0 {
                durations.push(seconds);
            }
        }
        if let Some(completed_at) = completed_at {
            push_trend(&mut trend, &completed_at, |point| point.completed += 1);
        }
    }
    let op_sql = format!(
        "SELECT COALESCE(SUM(CASE WHEN o.state = 'failed' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN o.state IN ('completed','failed') THEN 1 ELSE 0 END), 0)
         FROM workflow_ops o JOIN workflow_runs w ON w.id = o.run_id
         JOIN sessions s ON s.id = w.session_id {}",
        terminal_scope.sql()
    );
    let (failed_ops, decided_ops): (u64, u64) = conn.query_row(
        &op_sql,
        params_from_iter(terminal_scope.params.iter()),
        |row| Ok((crate::sql_u64(row, 0)?, crate::sql_u64(row, 1)?)),
    )?;
    let sample_count = durations.len() as u64;
    let p50_secs = median(&mut durations);
    Ok(WorkflowDashboardStats {
        completion: RatioMetric::new(completed, total),
        op_failure: RatioMetric::new(failed_ops, decided_ops),
        duration: DurationMetric {
            p50_secs,
            sample_count,
            eligible_count: total,
        },
        goal_binding: RatioMetric::new(bound, total),
        approval_trigger: RatioMetric::new(approval, total),
        current_states,
        kinds,
        origins,
        trend: sorted_trend(trend),
    })
}

fn query_loops(
    conn: &Connection,
    filter: &ControlPlaneDashboardFilter,
) -> Result<LoopDashboardStats> {
    let current_scope = Scope::for_sessions(filter, "s");
    let current_states = count_groups(
        conn,
        "l.state",
        "FROM loop_schedules l JOIN sessions s ON s.id = l.session_id",
        &current_scope,
    )?;
    let trigger_kinds = count_groups(
        conn,
        "l.trigger_kind",
        "FROM loop_schedules l JOIN sessions s ON s.id = l.session_id",
        &current_scope,
    )?;
    let strategies = count_groups(
        conn,
        "l.execution_strategy",
        "FROM loop_schedules l JOIN sessions s ON s.id = l.session_id",
        &current_scope,
    )?;
    let current_blocked_schedules = current_states
        .iter()
        .find(|item| item.key == "blocked")
        .map_or(0, |item| item.count);

    let mut run_scope = Scope::for_sessions(filter, "s");
    run_scope.clauses.push(
        "r.progress_state IN ('progressed','weak_progress','no_progress','blocked','failed')"
            .into(),
    );
    run_scope = run_scope.window(filter, "r.finished_at");
    let progress_states = count_groups(
        conn,
        "r.progress_state",
        "FROM loop_runs r JOIN sessions s ON s.id = r.session_id",
        &run_scope,
    )?;
    let sql = format!(
        "SELECT r.progress_state, r.started_at, r.finished_at
         FROM loop_runs r JOIN sessions s ON s.id = r.session_id {}",
        run_scope.sql()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(run_scope.params.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut total = 0_u64;
    let mut progressed = 0_u64;
    let mut no_progress = 0_u64;
    let mut durations = Vec::new();
    let mut trend = HashMap::new();
    for row in rows {
        let (progress, started_at, finished_at) = row?;
        total += 1;
        progressed += u64::from(progress == "progressed");
        no_progress += u64::from(progress == "no_progress");
        if let (Some(start), Some(end)) = (
            parse_utc(&started_at),
            finished_at.as_deref().and_then(parse_utc),
        ) {
            let seconds = (end - start).num_milliseconds() as f64 / 1000.0;
            if seconds >= 0.0 {
                durations.push(seconds);
            }
        }
        if let Some(finished_at) = finished_at {
            push_trend(&mut trend, &finished_at, |point| point.completed += 1);
        }
    }
    let sample_count = durations.len() as u64;
    let p50_secs = median(&mut durations);
    Ok(LoopDashboardStats {
        strong_progress: RatioMetric::new(progressed, total),
        no_progress: RatioMetric::new(no_progress, total),
        duration: DurationMetric {
            p50_secs,
            sample_count,
            eligible_count: total,
        },
        current_blocked_schedules,
        progress_states,
        trigger_kinds,
        strategies,
        current_states,
        trend: sorted_trend(trend),
    })
}

fn query_tasks(
    conn: &Connection,
    filter: &ControlPlaneDashboardFilter,
) -> Result<TaskDashboardStats> {
    let current_scope = Scope::for_sessions(filter, "s");
    let current_states = count_groups(
        conn,
        "t.status",
        "FROM tasks t JOIN sessions s ON s.id = t.session_id",
        &current_scope,
    )?;
    let current_backlog = current_states
        .iter()
        .filter(|item| item.key != "completed")
        .map(|item| item.count)
        .sum();
    let cohort_scope = Scope::for_sessions(filter, "s").window(filter, "t.created_at");
    let sql = format!(
        "SELECT t.status, t.created_at, t.completed_at
         FROM tasks t JOIN sessions s ON s.id = t.session_id {}",
        cohort_scope.sql()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(cohort_scope.params.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut total = 0_u64;
    let mut completed = 0_u64;
    let mut durations = Vec::new();
    let mut trend = HashMap::new();
    for row in rows {
        let (status, created_at, completed_at) = row?;
        total += 1;
        push_trend(&mut trend, &created_at, |point| point.created += 1);
        if status == "completed" {
            completed += 1;
            if let (Some(start), Some(end)) = (
                parse_utc(&created_at),
                completed_at.as_deref().and_then(parse_utc),
            ) {
                let seconds = (end - start).num_milliseconds() as f64 / 1000.0;
                if seconds >= 0.0 {
                    durations.push(seconds);
                }
            }
        }
    }
    let sample_count = durations.len() as u64;
    let p50_secs = median(&mut durations);
    Ok(TaskDashboardStats {
        cohort_completion: RatioMetric::new(completed, total),
        current_backlog,
        duration: DurationMetric {
            p50_secs,
            sample_count,
            eligible_count: completed,
        },
        current_states,
        trend: sorted_trend(trend),
    })
}

fn matches_plan_scope(
    plan: &crate::plan::PlanIndexEntry,
    filter: &ControlPlaneDashboardFilter,
) -> bool {
    if let Some(agent_id) = present(filter.agent_id.as_deref()) {
        if plan.agent_id != agent_id {
            return false;
        }
    }
    if let Some(project_id) = present(filter.project_id.as_deref()) {
        if project_id == CONTROL_PLANE_UNASSIGNED_PROJECT {
            if plan.project_id.is_some() {
                return false;
            }
        } else if plan.project_id.as_deref() != Some(project_id) {
            return false;
        }
    }
    true
}

fn aggregate_plans(
    all: Vec<crate::plan::PlanIndexEntry>,
    filter: &ControlPlaneDashboardFilter,
) -> Result<(PlanDashboardStats, Vec<AttentionItem>)> {
    let all = all
        .into_iter()
        // Dashboard attribution comes from the owning session. Orphan files
        // remain discoverable in Plan history, but cannot be safely assigned
        // to a project or proven non-incognito, so they fail closed here.
        .filter(|plan| !plan.orphan && matches_plan_scope(plan, filter))
        .collect::<Vec<_>>();
    let active_now = all
        .iter()
        .filter(|plan| {
            matches!(
                plan.state,
                PlanModeState::Planning | PlanModeState::Review | PlanModeState::Executing
            )
        })
        .count() as u64;

    let mut state_counts = HashMap::new();
    let mut attention = Vec::new();
    for plan in &all {
        *state_counts
            .entry(plan.state.as_str().to_string())
            .or_insert(0_u64) += 1;
        if plan.state == PlanModeState::Review {
            attention.push(AttentionItem {
                kind: "plan".into(),
                id: plan.session_id.clone(),
                session_id: plan.session_id.clone(),
                title: plan
                    .title
                    .clone()
                    .or_else(|| plan.session_title.clone())
                    .unwrap_or_else(|| "Plan awaiting review".into()),
                status: "review".into(),
                reason: "plan_awaiting_review".into(),
                severity: "warning".into(),
                updated_at: plan
                    .session_updated_at
                    .clone()
                    .unwrap_or_else(|| plan.updated_at.clone()),
            });
        }
    }
    let mut current_states = state_counts
        .into_iter()
        .map(|(key, count)| NamedCount { key, count })
        .collect::<Vec<_>>();
    current_states.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));

    let cohort = all
        .iter()
        .filter(|plan| in_window(&plan.created_at, filter))
        .collect::<Vec<_>>();
    let completed = cohort
        .iter()
        .filter(|plan| plan.completed_at.is_some() || plan.state == PlanModeState::Completed)
        .count() as u64;
    let mut durations = Vec::new();
    let mut trend = HashMap::new();
    let mut agents = HashMap::new();
    let mut projects = HashMap::new();
    for plan in &cohort {
        *agents.entry(plan.agent_id.clone()).or_insert(0_u64) += 1;
        *projects
            .entry(
                plan.project_id
                    .clone()
                    .unwrap_or_else(|| "unassigned".into()),
            )
            .or_insert(0_u64) += 1;
        push_trend(&mut trend, &plan.created_at, |point| point.created += 1);
        if let (Some(start), Some(end)) = (
            plan.executing_started_at.as_deref().and_then(parse_utc),
            plan.completed_at.as_deref().and_then(parse_utc),
        ) {
            let seconds = (end - start).num_milliseconds() as f64 / 1000.0;
            if seconds >= 0.0 {
                durations.push(seconds);
            }
        }
    }
    let mut by_agent = agents
        .into_iter()
        .map(|(key, count)| NamedCount { key, count })
        .collect::<Vec<_>>();
    let mut by_project = projects
        .into_iter()
        .map(|(key, count)| NamedCount { key, count })
        .collect::<Vec<_>>();
    by_agent.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
    by_project.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
    by_agent.truncate(10);
    by_project.truncate(10);
    let sample_count = durations.len() as u64;
    let p50_secs = median(&mut durations);
    Ok((
        PlanDashboardStats {
            cohort_completion: RatioMetric::new(completed, cohort.len() as u64),
            active_now,
            duration: DurationMetric {
                p50_secs,
                sample_count,
                eligible_count: completed,
            },
            current_states,
            by_agent,
            by_project,
            trend: sorted_trend(trend),
        },
        attention,
    ))
}

fn query_plans(
    filter: &ControlPlaneDashboardFilter,
) -> Result<(PlanDashboardStats, Vec<AttentionItem>)> {
    aggregate_plans(list_all_plans(&PlanIndexFilter::default())?, filter)
}

fn query_attention(
    conn: &Connection,
    filter: &ControlPlaneDashboardFilter,
) -> Result<Vec<AttentionItem>> {
    let scope = Scope::for_sessions(filter, "s");
    let mut items = Vec::new();
    let goal_sql = format!(
        "SELECT g.id, g.session_id, g.objective, g.state,
                COALESCE(g.blocked_reason,
                    CASE WHEN g.closure_decision = 'needs_strict_evidence' THEN 'needs_strict_evidence'
                         ELSE 'goal_awaiting_closure' END), g.updated_at
         FROM goals g JOIN sessions s ON s.id = g.session_id
         {} AND (g.state = 'blocked' OR g.closure_decision = 'needs_strict_evidence'
                 OR (g.state = 'completed' AND g.closure_decision IS NULL))",
        scope.sql()
    );
    let mut stmt = conn.prepare(&goal_sql)?;
    let rows = stmt.query_map(params_from_iter(scope.params.iter()), |row| {
        let state: String = row.get(3)?;
        Ok(AttentionItem {
            kind: "goal".into(),
            id: row.get(0)?,
            session_id: row.get(1)?,
            title: row.get(2)?,
            status: state.clone(),
            reason: row.get(4)?,
            severity: if state == "blocked" {
                "critical"
            } else {
                "warning"
            }
            .into(),
            updated_at: row.get(5)?,
        })
    })?;
    items.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);

    let workflow_sql = format!(
        "SELECT w.id, w.session_id, COALESCE(NULLIF(s.title, ''), w.kind), w.state,
                COALESCE(w.blocked_reason, 'workflow_requires_attention'), w.updated_at
         FROM workflow_runs w JOIN sessions s ON s.id = w.session_id
         {} AND w.state IN ('awaiting_approval','awaiting_user','blocked')",
        scope.sql()
    );
    let mut stmt = conn.prepare(&workflow_sql)?;
    let rows = stmt.query_map(params_from_iter(scope.params.iter()), |row| {
        let state: String = row.get(3)?;
        Ok(AttentionItem {
            kind: "workflow".into(),
            id: row.get(0)?,
            session_id: row.get(1)?,
            title: row.get(2)?,
            status: state.clone(),
            reason: row.get(4)?,
            severity: if state == "blocked" {
                "critical"
            } else {
                "warning"
            }
            .into(),
            updated_at: row.get(5)?,
        })
    })?;
    items.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);

    let loop_sql = format!(
        "SELECT l.id, l.session_id, substr(l.prompt, 1, 160), l.state,
                COALESCE(l.blocked_reason,
                    CASE WHEN l.no_progress_streak > 0 THEN 'consecutive_no_progress'
                         ELSE 'loop_requires_attention' END), l.updated_at
         FROM loop_schedules l JOIN sessions s ON s.id = l.session_id
         {} AND (l.state = 'blocked' OR l.no_progress_streak > 0)",
        scope.sql()
    );
    let mut stmt = conn.prepare(&loop_sql)?;
    let rows = stmt.query_map(params_from_iter(scope.params.iter()), |row| {
        let state: String = row.get(3)?;
        Ok(AttentionItem {
            kind: "loop".into(),
            id: row.get(0)?,
            session_id: row.get(1)?,
            title: row.get(2)?,
            status: state.clone(),
            reason: row.get(4)?,
            severity: if state == "blocked" {
                "critical"
            } else {
                "warning"
            }
            .into(),
            updated_at: row.get(5)?,
        })
    })?;
    items.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
    Ok(items)
}

fn finalize_attention(mut items: Vec<AttentionItem>) -> AttentionDashboard {
    let rank = |severity: &str| match severity {
        "critical" => 2,
        "warning" => 1,
        _ => 0,
    };
    // "Recent 20" is a recency contract. Severity only breaks ties at the
    // same timestamp; otherwise an old critical item must not hide newer work.
    items.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| rank(&b.severity).cmp(&rank(&a.severity)))
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert((item.kind.clone(), item.id.clone())));
    let total = items.len() as u64;
    items.truncate(20);
    AttentionDashboard { total, items }
}

/// Build the whole page from one durable-data entrypoint. SQL work is done
/// under one SessionDB read lock, then released before scanning Plan files
/// because the Plan index consults SessionDB metadata itself.
pub fn query_control_plane_dashboard(
    session_db: &Arc<SessionDB>,
    filter: &ControlPlaneDashboardFilter,
) -> Result<ControlPlaneDashboard> {
    let conn = session_db
        .conn
        .lock()
        .map_err(|error| anyhow::anyhow!("Lock error: {error}"))?;
    let goals = query_goals(&conn, filter)?;
    let workflows = query_workflows(&conn, filter)?;
    let loops = query_loops(&conn, filter)?;
    let tasks = query_tasks(&conn, filter)?;
    let mut attention_items = query_attention(&conn, filter)?;
    drop(conn);

    let (plans, plan_attention) = query_plans(filter)?;
    attention_items.extend(plan_attention);
    let attention = finalize_attention(attention_items);

    Ok(ControlPlaneDashboard {
        summary: ControlPlaneSummary {
            goal_acceptance: goals.acceptance.clone(),
            workflow_completion: workflows.completion.clone(),
            loop_strong_progress: loops.strong_progress.clone(),
            task_cohort_completion: tasks.cohort_completion.clone(),
            attention_count: attention.total,
        },
        goals,
        workflows,
        loops,
        tasks,
        plans,
        attention,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::TaskStatus;

    fn test_db(name: &str) -> (Arc<SessionDB>, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "hope-control-plane-{name}-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = Arc::new(SessionDB::open(&path).expect("open dashboard fixture db"));
        (db, path)
    }

    fn plan_entry(
        session_id: &str,
        agent_id: &str,
        project_id: Option<&str>,
        state: PlanModeState,
        created_at: &str,
        executing_started_at: Option<&str>,
        completed_at: Option<&str>,
        orphan: bool,
    ) -> crate::plan::PlanIndexEntry {
        crate::plan::PlanIndexEntry {
            session_id: session_id.into(),
            session_short_id: session_id.chars().take(8).collect(),
            session_title: Some(format!("Session {session_id}")),
            agent_id: agent_id.into(),
            project_id: project_id.map(str::to_string),
            plan_file_path: format!("/tmp/{session_id}.md"),
            state,
            title: Some(format!("Plan {session_id}")),
            created_at: created_at.into(),
            updated_at: created_at.into(),
            session_updated_at: Some(created_at.into()),
            version_count: 1,
            executing_started_at: executing_started_at.map(str::to_string),
            completed_at: completed_at.map(str::to_string),
            orphan,
        }
    }

    #[test]
    fn zero_denominator_is_null() {
        assert_eq!(RatioMetric::new(0, 0).rate, None);
    }

    #[test]
    fn p50_handles_even_and_odd_samples() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&mut [4.0, 1.0, 2.0, 3.0]), Some(2.5));
    }

    #[test]
    fn attention_is_deduplicated_sorted_and_truncated() {
        let mut items = (0..22)
            .map(|index| AttentionItem {
                kind: "goal".into(),
                id: format!("goal-{index:02}"),
                session_id: "session".into(),
                title: format!("Goal {index}"),
                status: "blocked".into(),
                reason: "test".into(),
                severity: "warning".into(),
                updated_at: format!("2026-07-15T00:{index:02}:00Z"),
            })
            .collect::<Vec<_>>();
        items.push(AttentionItem {
            kind: "goal".into(),
            id: "goal-21".into(),
            session_id: "session".into(),
            title: "Older duplicate".into(),
            status: "blocked".into(),
            reason: "test".into(),
            severity: "critical".into(),
            updated_at: "2026-07-15T00:00:00Z".into(),
        });

        let attention = finalize_attention(items);
        assert_eq!(attention.total, 22);
        assert_eq!(attention.items.len(), 20);
        assert_eq!(
            attention.items.first().map(|item| item.id.as_str()),
            Some("goal-21")
        );
        assert_eq!(
            attention.items.last().map(|item| item.id.as_str()),
            Some("goal-02")
        );
        assert_eq!(
            attention
                .items
                .iter()
                .filter(|item| item.id == "goal-21")
                .count(),
            1
        );
    }

    #[test]
    fn fixture_respects_terminal_denominators_and_excludes_non_user_sessions() {
        let (db, path) = test_db("metrics");
        {
            let conn = db.conn.lock().expect("lock fixture db");
            conn.execute_batch(
                "INSERT INTO sessions
                    (id, title, agent_id, created_at, updated_at, incognito, is_cron, parent_session_id)
                 VALUES
                    ('main', 'Main', 'agent-a', '2026-01-01T00:00:00Z', '2026-06-01T00:00:00Z', 0, 0, NULL),
                    ('private', 'Private', 'agent-a', '2026-01-01T00:00:00Z', '2026-06-01T00:00:00Z', 1, 0, NULL),
                    ('cron', 'Cron', 'agent-a', '2026-01-01T00:00:00Z', '2026-06-01T00:00:00Z', 0, 1, NULL),
                    ('child', 'Child', 'agent-a', '2026-01-01T00:00:00Z', '2026-06-01T00:00:00Z', 0, 0, 'main');

                 INSERT INTO goals
                    (id, session_id, objective, state, created_at, updated_at, completed_at,
                     final_evidence_json, last_evaluator_result_json, closure_decision, closed_at)
                 VALUES
                    ('g-accepted', 'main', 'accepted', 'completed', '2026-01-01T00:00:00Z', '2026-02-01T00:00:00Z', '2026-02-01T00:00:00Z',
                     '{\"goalRevision\":1,\"goalLinkedEventSeq\":0,\"auditStale\":false,\"criteriaStatus\":[{\"kind\":\"required\",\"status\":\"satisfied\"},{\"kind\":\"required\",\"status\":\"missing\"}]}', '{}', 'accepted_v1', '2026-02-01T00:00:00Z'),
                    ('g-cancelled', 'main', 'cancelled', 'cancelled', '2026-01-01T00:00:00Z', '2026-02-02T00:00:00Z', '2026-02-02T00:00:00Z', '{}', '{}', 'cancelled', '2026-02-02T00:00:00Z'),
                    ('g-superseded', 'main', 'superseded', 'completed', '2026-01-01T00:00:00Z', '2026-02-03T00:00:00Z', '2026-02-03T00:00:00Z', '{}', '{}', 'superseded', '2026-02-03T00:00:00Z'),
                    ('g-failed', 'main', 'failed', 'failed', '2026-01-01T00:00:00Z', '2026-02-04T00:00:00Z', '2026-02-04T00:00:00Z', '{}', '{}', NULL, NULL),
                    ('g-blocked', 'main', 'blocked', 'blocked', '2026-01-01T00:00:00Z', '2026-02-05T00:00:00Z', NULL, '{}', '{}', NULL, NULL),
                    ('g-private', 'private', 'private accepted', 'completed', '2026-01-01T00:00:00Z', '2026-02-06T00:00:00Z', '2026-02-06T00:00:00Z', '{}', '{}', 'accepted_v1', '2026-02-06T00:00:00Z'),
                    ('g-cron', 'cron', 'cron accepted', 'completed', '2026-01-01T00:00:00Z', '2026-02-07T00:00:00Z', '2026-02-07T00:00:00Z', '{}', '{}', 'accepted_v1', '2026-02-07T00:00:00Z'),
                    ('g-child', 'child', 'child blocked', 'blocked', '2026-01-01T00:00:00Z', '2026-02-08T00:00:00Z', NULL, '{}', '{}', NULL, NULL);

                 INSERT INTO workflow_runs
                    (id, session_id, kind, state, execution_mode, script_hash, script_source,
                     created_at, updated_at, completed_at)
                 VALUES
                    ('w-ok', 'main', 'dynamic', 'completed', 'safe', 'h', '', '2026-03-01T00:00:00Z', '2026-03-01T00:01:00Z', '2026-03-01T00:01:00Z'),
                    ('w-fail', 'main', 'dynamic', 'failed', 'safe', 'h', '', '2026-03-01T00:00:00Z', '2026-03-01T00:02:00Z', '2026-03-01T00:02:00Z'),
                    ('w-block', 'main', 'dynamic', 'blocked', 'safe', 'h', '', '2026-03-01T00:00:00Z', '2026-03-01T00:03:00Z', '2026-03-01T00:03:00Z'),
                    ('w-cancel', 'main', 'dynamic', 'cancelled', 'safe', 'h', '', '2026-03-01T00:00:00Z', '2026-03-01T00:04:00Z', '2026-03-01T00:04:00Z'),
                    ('w-cron', 'cron', 'dynamic', 'completed', 'safe', 'h', '', '2026-03-01T00:00:00Z', '2026-03-01T00:05:00Z', '2026-03-01T00:05:00Z'),
                    ('w-child', 'child', 'dynamic', 'blocked', 'safe', 'h', '', '2026-03-01T00:00:00Z', '2026-03-01T00:06:00Z', '2026-03-01T00:06:00Z');
                 INSERT INTO workflow_ops
                    (id, run_id, op_key, op_type, effect_class, input_hash, input_json, state, started_at, completed_at)
                 VALUES ('op-ok', 'w-ok', 'ok', 'tool', 'read', 'h', '{}', 'completed', '2026-03-01T00:00:00Z', '2026-03-01T00:00:30Z'),
                        ('op-fail', 'w-fail', 'fail', 'tool', 'read', 'h', '{}', 'failed', '2026-03-01T00:00:00Z', '2026-03-01T00:00:30Z');

                 INSERT INTO loop_schedules
                    (id, session_id, cron_job_id, prompt, trigger_kind, execution_strategy,
                     state, created_at, updated_at)
                 VALUES ('loop', 'main', 'cron-loop', 'keep going', 'interval', 'continue',
                         'blocked', '2026-04-01T00:00:00Z', '2026-04-01T00:00:00Z'),
                        ('loop-cron', 'cron', 'cron-loop-child', 'cron loop', 'interval', 'continue',
                         'blocked', '2026-04-01T00:00:00Z', '2026-04-01T00:00:00Z'),
                        ('loop-child', 'child', 'child-loop', 'child loop', 'interval', 'continue',
                         'blocked', '2026-04-01T00:00:00Z', '2026-04-01T00:00:00Z');
                 INSERT INTO loop_runs
                    (id, loop_id, cron_job_id, session_id, seq, state, trigger_reason,
                     progress_state, started_at, finished_at)
                 VALUES
                    ('lr-1', 'loop', 'cron-loop', 'main', 1, 'completed', 'test', 'progressed', '2026-04-01T00:00:00Z', '2026-04-01T00:01:00Z'),
                    ('lr-2', 'loop', 'cron-loop', 'main', 2, 'completed', 'test', 'weak_progress', '2026-04-01T00:00:00Z', '2026-04-01T00:01:00Z'),
                    ('lr-3', 'loop', 'cron-loop', 'main', 3, 'completed', 'test', 'no_progress', '2026-04-01T00:00:00Z', '2026-04-01T00:01:00Z'),
                    ('lr-4', 'loop', 'cron-loop', 'main', 4, 'failed', 'test', 'blocked', '2026-04-01T00:00:00Z', '2026-04-01T00:01:00Z'),
                    ('lr-5', 'loop', 'cron-loop', 'main', 5, 'failed', 'test', 'failed', '2026-04-01T00:00:00Z', '2026-04-01T00:01:00Z'),
                    ('lr-6', 'loop', 'cron-loop', 'main', 6, 'paused', 'test', 'awaiting_approval', '2026-04-01T00:00:00Z', '2026-04-01T00:01:00Z'),
                    ('lr-cron', 'loop-cron', 'cron-loop-child', 'cron', 1, 'completed', 'test', 'progressed', '2026-04-01T00:00:00Z', '2026-04-01T00:01:00Z'),
                    ('lr-child', 'loop-child', 'child-loop', 'child', 1, 'completed', 'test', 'progressed', '2026-04-01T00:00:00Z', '2026-04-01T00:01:00Z');",
            )
            .expect("seed control-plane fixture");
        }

        let filter = ControlPlaneDashboardFilter {
            start_date: Some("2026-01-01T00:00:00Z".into()),
            end_date: Some("2026-12-31T23:59:59Z".into()),
            agent_id: Some("agent-a".into()),
            project_id: Some(CONTROL_PLANE_UNASSIGNED_PROJECT.into()),
        };
        let conn = db.conn.lock().expect("lock fixture for assertions");
        let goals = query_goals(&conn, &filter).expect("goal metrics");
        assert_eq!(
            (goals.acceptance.numerator, goals.acceptance.denominator),
            (1, 4)
        );
        assert_eq!(goals.required_criteria.rate, Some(0.5));
        assert_eq!(goals.audited_goal_count, 1);

        let workflows = query_workflows(&conn, &filter).expect("workflow metrics");
        assert_eq!(
            (
                workflows.completion.numerator,
                workflows.completion.denominator
            ),
            (1, 3)
        );
        assert_eq!(
            (
                workflows.op_failure.numerator,
                workflows.op_failure.denominator
            ),
            (1, 2)
        );

        let loops = query_loops(&conn, &filter).expect("loop metrics");
        assert_eq!(
            (
                loops.strong_progress.numerator,
                loops.strong_progress.denominator
            ),
            (1, 5)
        );
        assert_eq!(
            (loops.no_progress.numerator, loops.no_progress.denominator),
            (1, 5)
        );
        assert_eq!(loops.current_blocked_schedules, 1);
        let attention = query_attention(&conn, &filter).expect("attention metrics");
        assert!(attention.iter().all(|item| item.session_id == "main"));
        drop(conn);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn task_cohort_is_separate_from_current_backlog_and_reports_coverage() {
        let (db, path) = test_db("task-cohort");
        {
            let conn = db.conn.lock().expect("lock task cohort db");
            conn.execute_batch(
                "INSERT INTO sessions
                    (id, title, agent_id, project_id, created_at, updated_at, incognito)
                 VALUES
                    ('main', 'Main', 'agent-a', NULL, '2026-01-01T00:00:00Z', '2026-02-01T00:00:00Z', 0),
                    ('assigned', 'Assigned', 'agent-a', 'project-x', '2026-01-01T00:00:00Z', '2026-02-01T00:00:00Z', 0),
                    ('other-agent', 'Other', 'agent-b', NULL, '2026-01-01T00:00:00Z', '2026-02-01T00:00:00Z', 0),
                    ('private', 'Private', 'agent-a', NULL, '2026-01-01T00:00:00Z', '2026-02-01T00:00:00Z', 1);
                 INSERT INTO sessions
                    (id, title, agent_id, project_id, created_at, updated_at, incognito, is_cron, parent_session_id)
                 VALUES
                    ('cron', 'Cron', 'agent-a', NULL, '2026-01-01T00:00:00Z', '2026-02-01T00:00:00Z', 0, 1, NULL),
                    ('child', 'Child', 'agent-a', NULL, '2026-01-01T00:00:00Z', '2026-02-01T00:00:00Z', 0, 0, 'main');

                 INSERT INTO tasks
                    (session_id, content, status, created_at, updated_at, completed_at)
                 VALUES
                    ('main', 'exact', 'completed', '2026-02-01T00:00:00Z', '2026-02-01T00:02:00Z', '2026-02-01T00:02:00Z'),
                    ('main', 'legacy', 'completed', '2026-02-02T00:00:00Z', '2026-02-02T00:03:00Z', NULL),
                    ('main', 'backlog', 'pending', '2026-02-03T00:00:00Z', '2026-02-03T00:00:00Z', NULL),
                    ('main', 'outside window', 'completed', '2025-12-01T00:00:00Z', '2025-12-01T00:00:00Z', NULL),
                    ('assigned', 'assigned', 'completed', '2026-02-04T00:00:00Z', '2026-02-04T00:00:00Z', NULL),
                    ('other-agent', 'other', 'completed', '2026-02-05T00:00:00Z', '2026-02-05T00:00:00Z', NULL),
                    ('private', 'private', 'completed', '2026-02-06T00:00:00Z', '2026-02-06T00:00:00Z', NULL),
                    ('cron', 'cron task', 'completed', '2026-02-07T00:00:00Z', '2026-02-07T00:00:00Z', NULL),
                    ('child', 'child task', 'pending', '2026-02-08T00:00:00Z', '2026-02-08T00:00:00Z', NULL);",
            )
            .expect("seed task cohort fixture");

            let filter = ControlPlaneDashboardFilter {
                start_date: Some("2026-02-01T00:00:00Z".into()),
                end_date: Some("2026-02-28T23:59:59Z".into()),
                agent_id: Some("agent-a".into()),
                project_id: Some(CONTROL_PLANE_UNASSIGNED_PROJECT.into()),
            };
            let tasks = query_tasks(&conn, &filter).expect("task cohort metrics");
            assert_eq!(
                (
                    tasks.cohort_completion.numerator,
                    tasks.cohort_completion.denominator
                ),
                (2, 3)
            );
            assert_eq!(tasks.current_backlog, 1);
            assert_eq!(tasks.duration.sample_count, 1);
            assert_eq!(tasks.duration.eligible_count, 2);
            assert_eq!(tasks.duration.p50_secs, Some(120.0));
            assert_eq!(
                tasks
                    .current_states
                    .iter()
                    .find(|item| item.key == "completed")
                    .map(|item| item.count),
                Some(3)
            );

            let assigned = query_tasks(
                &conn,
                &ControlPlaneDashboardFilter {
                    project_id: Some("project-x".into()),
                    ..filter
                },
            )
            .expect("assigned project metrics");
            assert_eq!(assigned.cohort_completion.denominator, 1);
        }
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn plan_cohort_preserves_archived_completion_and_excludes_orphans() {
        let plans = vec![
            plan_entry(
                "archived",
                "agent-a",
                None,
                PlanModeState::Off,
                "2026-02-01T00:00:00Z",
                Some("2026-02-01T00:00:00Z"),
                Some("2026-02-01T00:02:00Z"),
                false,
            ),
            plan_entry(
                "planning",
                "agent-a",
                None,
                PlanModeState::Planning,
                "2026-02-02T00:00:00Z",
                None,
                None,
                false,
            ),
            plan_entry(
                "review",
                "agent-a",
                None,
                PlanModeState::Review,
                "2026-02-03T00:00:00Z",
                None,
                None,
                false,
            ),
            plan_entry(
                "legacy-completed",
                "agent-a",
                None,
                PlanModeState::Completed,
                "2026-02-04T00:00:00Z",
                None,
                None,
                false,
            ),
            plan_entry(
                "old-completed",
                "agent-a",
                None,
                PlanModeState::Completed,
                "2025-12-01T00:00:00Z",
                None,
                None,
                false,
            ),
            plan_entry(
                "assigned",
                "agent-a",
                Some("project-x"),
                PlanModeState::Completed,
                "2026-02-05T00:00:00Z",
                None,
                None,
                false,
            ),
            plan_entry(
                "other-agent",
                "agent-b",
                None,
                PlanModeState::Completed,
                "2026-02-06T00:00:00Z",
                None,
                None,
                false,
            ),
            plan_entry(
                "orphan",
                "agent-a",
                None,
                PlanModeState::Review,
                "2026-02-07T00:00:00Z",
                None,
                None,
                true,
            ),
        ];
        let filter = ControlPlaneDashboardFilter {
            start_date: Some("2026-02-01T00:00:00Z".into()),
            end_date: Some("2026-02-28T23:59:59Z".into()),
            agent_id: Some("agent-a".into()),
            project_id: Some(CONTROL_PLANE_UNASSIGNED_PROJECT.into()),
        };
        let (stats, attention) =
            aggregate_plans(plans.clone(), &filter).expect("plan cohort metrics");
        assert_eq!(
            (
                stats.cohort_completion.numerator,
                stats.cohort_completion.denominator
            ),
            (2, 4)
        );
        assert_eq!(stats.active_now, 2);
        assert_eq!(stats.duration.sample_count, 1);
        assert_eq!(stats.duration.eligible_count, 2);
        assert_eq!(stats.duration.p50_secs, Some(120.0));
        assert_eq!(attention.len(), 1);
        assert_eq!(attention[0].id, "review");
        assert_eq!(
            stats
                .current_states
                .iter()
                .find(|item| item.key == "completed")
                .map(|item| item.count),
            Some(2)
        );

        let (assigned, _) = aggregate_plans(
            plans,
            &ControlPlaneDashboardFilter {
                project_id: Some("project-x".into()),
                ..filter
            },
        )
        .expect("assigned plan metrics");
        assert_eq!(assigned.cohort_completion.denominator, 1);
    }

    #[test]
    fn exact_task_and_plan_completion_timestamps_follow_lifecycle() {
        let (db, path) = test_db("timestamps");
        {
            let conn = db.conn.lock().expect("lock timestamp db");
            conn.execute(
                "INSERT INTO sessions (id, title, agent_id, created_at, updated_at)
                 VALUES ('session', 'Session', 'agent-a', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                [],
            )
            .expect("insert session");
        }

        let task = db
            .create_task("session", "ship dashboard", None)
            .expect("create task");
        assert!(task.completed_at.is_none());
        let completed = db
            .update_task(task.id, Some(TaskStatus::Completed), None, None)
            .expect("complete task");
        assert!(completed.completed_at.is_some());
        let reopened = db
            .update_task(task.id, Some(TaskStatus::InProgress), None, None)
            .expect("reopen task");
        assert!(reopened.completed_at.is_none());
        let completed_again = db
            .update_task(task.id, Some(TaskStatus::Completed), None, None)
            .expect("complete task again");
        assert!(completed_again.completed_at.is_some());

        db.update_session_plan_mode("session", PlanModeState::Planning)
            .expect("start planning");
        assert_eq!(db.get_session_plan_completed_at("session").unwrap(), None);
        db.update_session_plan_mode("session", PlanModeState::Completed)
            .expect("complete plan");
        let plan_completed_at = db
            .get_session_plan_completed_at("session")
            .unwrap()
            .expect("exact plan completion");
        db.update_session_plan_mode("session", PlanModeState::Off)
            .expect("archive plan");
        assert_eq!(
            db.get_session_plan_completed_at("session").unwrap(),
            Some(plan_completed_at)
        );
        db.update_session_plan_mode("session", PlanModeState::Planning)
            .expect("start a new plan lifecycle");
        assert_eq!(db.get_session_plan_completed_at("session").unwrap(), None);

        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
