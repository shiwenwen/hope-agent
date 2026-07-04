//! Coding improvement learning dashboard.
//!
//! This is the global/project rollup counterpart to the session-scoped
//! Coding Trend Report. It is intentionally read-only: it consumes existing
//! durable control-plane facts and never generates, applies, or promotes
//! improvement proposals.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use rusqlite::params_from_iter;
use serde::{Deserialize, Serialize};

use super::types::DashboardFilter;
use crate::coding_improvement::CodingRetroRecommendation;
use crate::session::SessionDB;
use crate::util::now_rfc3339;

const MAX_LIMIT: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementDashboard {
    pub generated_at: String,
    pub overview: CodingImprovementDashboardOverview,
    pub timeline: Vec<CodingImprovementTimelinePoint>,
    pub by_project: Vec<CodingImprovementProjectBucket>,
    pub domain_quality: DomainQualityDashboard,
    pub top_failures: Vec<CodingImprovementFailureBucket>,
    pub tool_call_failures: Vec<CodingImprovementFailureBucket>,
    pub proposal_statuses: Vec<CodingImprovementStatusBucket>,
    pub latest_strategy_effects: Vec<CodingImprovementStrategyEffectItem>,
    pub latest_retros: Vec<CodingImprovementRetroItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementDashboardOverview {
    pub total_sessions: u64,
    pub workflow_runs: u64,
    pub completed_workflows: u64,
    pub blocked_workflows: u64,
    pub failed_workflows: u64,
    pub workflow_completion_rate: Option<f64>,
    pub eval_runs: u64,
    pub passed_eval_runs: u64,
    pub failed_eval_runs: u64,
    pub eval_success_rate: Option<f64>,
    pub eval_pack_runs: u64,
    pub passed_eval_pack_runs: u64,
    pub failed_eval_pack_runs: u64,
    pub eval_pack_pass_rate: Option<f64>,
    pub deterministic_pack_runs: u64,
    pub external_pack_runs: u64,
    pub strategy_effect_runs: u64,
    pub improved_strategy_effects: u64,
    pub regressed_strategy_effects: u64,
    pub mixed_strategy_effects: u64,
    pub missing_tool_call_runs: u64,
    pub validation_violation_delta: i64,
    pub scope_creep_delta: i64,
    pub open_review_blockers: u64,
    pub failed_verification_steps: u64,
    pub retros: u64,
    pub retro_recommendations: u64,
    pub proposals: u64,
    pub draft_proposals: u64,
    pub applied_proposals: u64,
    pub promoted_proposals: u64,
    pub promotion_failures: u64,
    pub distillation_candidates: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementTimelinePoint {
    pub date: String,
    pub completed_workflows: u64,
    pub blocked_workflows: u64,
    pub failed_workflows: u64,
    pub eval_passed: u64,
    pub eval_failed: u64,
    pub eval_pack_passed: u64,
    pub eval_pack_failed: u64,
    pub strategy_improved: u64,
    pub strategy_regressed: u64,
    pub strategy_mixed: u64,
    pub validation_violation_delta: i64,
    pub scope_creep_delta: i64,
    pub proposals_created: u64,
    pub proposals_applied: u64,
    pub proposals_promoted: u64,
    pub retro_recommendations: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementProjectBucket {
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub sessions: u64,
    pub workflow_runs: u64,
    pub workflow_completion_rate: Option<f64>,
    pub eval_runs: u64,
    pub eval_success_rate: Option<f64>,
    pub eval_pack_runs: u64,
    pub eval_pack_pass_rate: Option<f64>,
    pub strategy_effect_runs: u64,
    pub regressed_strategy_effects: u64,
    pub open_review_blockers: u64,
    pub retro_recommendations: u64,
    pub draft_proposals: u64,
    pub applied_proposals: u64,
    pub promoted_proposals: u64,
    pub promotion_failures: u64,
    pub distillation_candidates: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementFailureBucket {
    pub category: String,
    pub label: String,
    pub severity: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementStatusBucket {
    pub status: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementStrategyEffectItem {
    pub id: String,
    pub project_id: Option<String>,
    pub strategy_type: String,
    pub baseline_label: String,
    pub candidate_label: String,
    pub verdict: String,
    pub compared_cases: u64,
    pub pass_rate_delta: f64,
    pub average_score_delta: f64,
    pub context_recall_delta: f64,
    pub validation_violation_delta: i64,
    pub scope_creep_delta: i64,
    pub execution_failure_delta: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingImprovementRetroItem {
    pub id: String,
    pub session_id: String,
    pub project_id: Option<String>,
    pub workflow_run_id: String,
    pub run_state: String,
    pub summary: String,
    #[serde(default)]
    pub recommendations: Vec<CodingRetroRecommendation>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainQualityDashboard {
    pub overview: DomainQualityDashboardOverview,
    pub timeline: Vec<DomainQualityTimelinePoint>,
    pub by_domain: Vec<DomainQualityDomainBucket>,
    pub top_blockers: Vec<DomainQualityBlockerBucket>,
    pub recent_runs: Vec<DomainQualityRunItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainQualityDashboardOverview {
    pub quality_runs: u64,
    pub completed_quality_runs: u64,
    pub blocked_quality_runs: u64,
    pub failed_quality_runs: u64,
    pub needs_user_quality_runs: u64,
    pub quality_completion_rate: Option<f64>,
    pub approval_blockers: u64,
    pub eval_runs: u64,
    pub passed_eval_runs: u64,
    pub failed_eval_runs: u64,
    pub eval_pass_rate: Option<f64>,
    pub average_eval_score: Option<f64>,
    pub domains_covered: u64,
    pub draft_domain_proposals: u64,
    pub promoted_domain_proposals: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainQualityTimelinePoint {
    pub date: String,
    pub quality_runs: u64,
    pub completed_quality_runs: u64,
    pub blocked_quality_runs: u64,
    pub failed_quality_runs: u64,
    pub needs_user_quality_runs: u64,
    pub approval_blockers: u64,
    pub eval_passed: u64,
    pub eval_failed: u64,
    pub proposals_created: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainQualityDomainBucket {
    pub domain: String,
    pub quality_runs: u64,
    pub completed_quality_runs: u64,
    pub blocked_quality_runs: u64,
    pub failed_quality_runs: u64,
    pub needs_user_quality_runs: u64,
    pub quality_completion_rate: Option<f64>,
    pub approval_blockers: u64,
    pub eval_runs: u64,
    pub eval_pass_rate: Option<f64>,
    pub average_eval_score: Option<f64>,
    pub draft_proposals: u64,
    pub promoted_proposals: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainQualityBlockerBucket {
    pub category: String,
    pub label: String,
    pub severity: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainQualityRunItem {
    pub id: String,
    pub session_id: String,
    pub project_id: Option<String>,
    pub domain: String,
    pub template_id: Option<String>,
    pub template_version: Option<String>,
    pub state: String,
    pub summary: String,
    pub failed_checks: u64,
    pub needs_user_checks: u64,
    pub approval_blockers: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default)]
struct SqlFilter {
    where_sql: String,
    params: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct ProjectAccumulator {
    project_id: Option<String>,
    sessions: u64,
    workflow_runs: u64,
    completed_workflows: u64,
    eval_runs: u64,
    passed_eval_runs: u64,
    eval_pack_runs: u64,
    passed_eval_pack_runs: u64,
    strategy_effect_runs: u64,
    regressed_strategy_effects: u64,
    open_review_blockers: u64,
    retro_recommendations: u64,
    draft_proposals: u64,
    applied_proposals: u64,
    promoted_proposals: u64,
    promotion_failures: u64,
}

#[derive(Debug, Clone, Default)]
struct DomainAccumulator {
    domain: String,
    quality_runs: u64,
    completed_quality_runs: u64,
    blocked_quality_runs: u64,
    failed_quality_runs: u64,
    needs_user_quality_runs: u64,
    approval_blockers: u64,
    eval_runs: u64,
    passed_eval_runs: u64,
    eval_score_sum: f64,
    draft_proposals: u64,
    promoted_proposals: u64,
}

pub fn query_coding_improvement_dashboard(
    db: &Arc<SessionDB>,
    filter: &DashboardFilter,
    limit: usize,
) -> Result<CodingImprovementDashboard> {
    let limit = limit.clamp(1, MAX_LIMIT);
    let conn = db.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;

    let overview = query_overview(&conn, filter)?;
    let timeline = query_timeline(&conn, filter)?;
    let by_project = query_projects(&conn, filter, limit)?;
    let domain_quality = query_domain_quality_dashboard(&conn, filter, limit)?;
    let top_failures = query_top_failures(&conn, filter, limit)?;
    let tool_call_failures = query_tool_call_failures(&conn, filter)?;
    let proposal_statuses = query_proposal_statuses(&conn, filter)?;
    let latest_strategy_effects = query_latest_strategy_effects(&conn, filter, limit)?;
    let latest_retros = query_latest_retros(&conn, filter, limit)?;

    Ok(CodingImprovementDashboard {
        generated_at: now_rfc3339(),
        overview,
        timeline,
        by_project,
        domain_quality,
        top_failures,
        tool_call_failures,
        proposal_statuses,
        latest_strategy_effects,
        latest_retros,
    })
}

fn query_overview(
    conn: &rusqlite::Connection,
    filter: &DashboardFilter,
) -> Result<CodingImprovementDashboardOverview> {
    let sessions_filter = build_fact_filter(filter, "s", "s.created_at", false);
    let total_sessions = count(
        conn,
        &format!(
            "SELECT COUNT(*) FROM sessions s {}",
            sessions_filter.where_sql
        ),
        &sessions_filter.params,
    )?;

    let workflow_filter = build_fact_filter(filter, "s", "wr.updated_at", false);
    let workflow_rows = count_by(
        conn,
        &format!(
            "SELECT wr.state, COUNT(*)
             FROM workflow_runs wr
             JOIN sessions s ON s.id = wr.session_id
             {}
             GROUP BY wr.state",
            workflow_filter.where_sql
        ),
        &workflow_filter.params,
    )?;
    let mut workflow_runs = 0;
    let mut completed_workflows = 0;
    let mut blocked_workflows = 0;
    let mut failed_workflows = 0;
    for (state, n) in workflow_rows {
        workflow_runs += n;
        match state.as_str() {
            "completed" => completed_workflows = n,
            "blocked" => blocked_workflows = n,
            "failed" => failed_workflows = n,
            _ => {}
        }
    }

    let eval_filter = build_fact_filter(filter, "s", "cer.created_at", true);
    let eval_rows = count_by(
        conn,
        &format!(
            "SELECT cer.status, COUNT(*)
             FROM coding_eval_runs cer
             LEFT JOIN sessions s ON s.id = cer.session_id
             {}
             GROUP BY cer.status",
            eval_filter.where_sql
        ),
        &eval_filter.params,
    )?;
    let mut eval_runs = 0;
    let mut passed_eval_runs = 0;
    let mut failed_eval_runs = 0;
    for (status, n) in eval_rows {
        eval_runs += n;
        match status.as_str() {
            "passed" => passed_eval_runs = n,
            "failed" => failed_eval_runs = n,
            _ => {}
        }
    }

    let pack_filter = build_fact_filter(filter, "s", "cepr.created_at", true);
    let mut stmt = conn.prepare(&format!(
        "SELECT cepr.status, cepr.baseline_kind, COUNT(*)
         FROM coding_eval_pack_runs cepr
         LEFT JOIN sessions s ON s.id = cepr.session_id
         {}
         GROUP BY cepr.status, cepr.baseline_kind",
        pack_filter.where_sql
    ))?;
    let pack_rows = stmt.query_map(params_from_iter(pack_filter.params.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            as_u64(row.get::<_, i64>(2)?),
        ))
    })?;
    let mut eval_pack_runs = 0;
    let mut passed_eval_pack_runs = 0;
    let mut failed_eval_pack_runs = 0;
    let mut deterministic_pack_runs = 0;
    let mut external_pack_runs = 0;
    for row in pack_rows {
        let (status, baseline_kind, n) = row?;
        eval_pack_runs += n;
        match status.as_str() {
            "passed" => passed_eval_pack_runs += n,
            "failed" => failed_eval_pack_runs += n,
            _ => {}
        }
        match baseline_kind.as_str() {
            "external_model" => external_pack_runs += n,
            _ => deterministic_pack_runs += n,
        }
    }

    let strategy_filter = build_fact_filter(filter, "s", "cser.created_at", true);
    let mut stmt = conn.prepare(&format!(
        "SELECT cser.verdict, COUNT(*),
                COALESCE(SUM(cser.validation_violation_delta), 0),
                COALESCE(SUM(cser.scope_creep_delta), 0)
         FROM coding_strategy_effect_runs cser
         LEFT JOIN sessions s ON s.id = cser.session_id
         {}
         GROUP BY cser.verdict",
        strategy_filter.where_sql
    ))?;
    let strategy_rows = stmt.query_map(params_from_iter(strategy_filter.params.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            as_u64(row.get::<_, i64>(1)?),
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    let mut strategy_effect_runs = 0;
    let mut improved_strategy_effects = 0;
    let mut regressed_strategy_effects = 0;
    let mut mixed_strategy_effects = 0;
    let mut validation_violation_delta = 0;
    let mut scope_creep_delta = 0;
    for row in strategy_rows {
        let (verdict, n, validation_delta, scope_delta) = row?;
        strategy_effect_runs += n;
        validation_violation_delta += validation_delta;
        scope_creep_delta += scope_delta;
        match verdict.as_str() {
            "improved" => improved_strategy_effects += n,
            "regressed" => regressed_strategy_effects += n,
            "mixed" => mixed_strategy_effects += n,
            _ => {}
        }
    }

    let missing_tool_call_runs = count_missing_tool_call_runs(conn, filter)?;

    let review_filter = append_condition(
        build_fact_filter(filter, "s", "rf.updated_at", false),
        "rf.status = 'open' AND rf.severity IN ('p0','p1','critical','high')",
    );
    let open_review_blockers = count(
        conn,
        &format!(
            "SELECT COUNT(*)
             FROM review_findings rf
             JOIN sessions s ON s.id = rf.session_id
             {}",
            review_filter.where_sql
        ),
        &review_filter.params,
    )?;

    let verification_filter = append_condition(
        build_fact_filter(filter, "s", "vs.updated_at", false),
        "vs.state IN ('failed','timed_out')",
    );
    let failed_verification_steps = count(
        conn,
        &format!(
            "SELECT COUNT(*)
             FROM verification_steps vs
             JOIN sessions s ON s.id = vs.session_id
             {}",
            verification_filter.where_sql
        ),
        &verification_filter.params,
    )?;

    let retro_filter = build_fact_filter(filter, "s", "cwr.updated_at", false);
    let (retros, retro_recommendations) = conn.query_row(
        &format!(
            "SELECT COUNT(*), COALESCE(SUM(json_array_length(cwr.recommendations_json)), 0)
             FROM coding_workflow_retros cwr
             JOIN sessions s ON s.id = cwr.session_id
             {}",
            retro_filter.where_sql
        ),
        params_from_iter(retro_filter.params.iter()),
        |row| Ok((as_u64(row.get::<_, i64>(0)?), as_u64(row.get::<_, i64>(1)?))),
    )?;

    let proposal_statuses = query_proposal_statuses(conn, filter)?;
    let mut proposals = 0;
    let mut draft_proposals = 0;
    let mut applied_proposals = 0;
    let mut promoted_proposals = 0;
    let mut promotion_failures = 0;
    for bucket in proposal_statuses {
        proposals += bucket.count;
        match bucket.status.as_str() {
            "draft" => draft_proposals = bucket.count,
            "applied" => applied_proposals = bucket.count,
            "promoted" => promoted_proposals = bucket.count,
            "promotion_failed" => promotion_failures = bucket.count,
            _ => {}
        }
    }

    Ok(CodingImprovementDashboardOverview {
        total_sessions,
        workflow_runs,
        completed_workflows,
        blocked_workflows,
        failed_workflows,
        workflow_completion_rate: ratio(completed_workflows, workflow_runs),
        eval_runs,
        passed_eval_runs,
        failed_eval_runs,
        eval_success_rate: ratio(passed_eval_runs, eval_runs),
        eval_pack_runs,
        passed_eval_pack_runs,
        failed_eval_pack_runs,
        eval_pack_pass_rate: ratio(passed_eval_pack_runs, eval_pack_runs),
        deterministic_pack_runs,
        external_pack_runs,
        strategy_effect_runs,
        improved_strategy_effects,
        regressed_strategy_effects,
        mixed_strategy_effects,
        missing_tool_call_runs,
        validation_violation_delta,
        scope_creep_delta,
        open_review_blockers,
        failed_verification_steps,
        retros,
        retro_recommendations,
        proposals,
        draft_proposals,
        applied_proposals,
        promoted_proposals,
        promotion_failures,
        distillation_candidates: draft_proposals
            + applied_proposals
            + promotion_failures
            + retro_recommendations,
    })
}

fn query_timeline(
    conn: &rusqlite::Connection,
    filter: &DashboardFilter,
) -> Result<Vec<CodingImprovementTimelinePoint>> {
    let mut days: BTreeMap<String, CodingImprovementTimelinePoint> = BTreeMap::new();

    let workflow_filter = build_fact_filter(filter, "s", "wr.updated_at", false);
    let mut stmt = conn.prepare(&format!(
        "SELECT substr(wr.updated_at, 1, 10) AS day, wr.state, COUNT(*)
         FROM workflow_runs wr
         JOIN sessions s ON s.id = wr.session_id
         {}
         GROUP BY day, wr.state",
        workflow_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(workflow_filter.params.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            as_u64(row.get::<_, i64>(2)?),
        ))
    })?;
    for row in rows {
        let (date, state, n) = row?;
        let point = day_point(&mut days, date);
        match state.as_str() {
            "completed" => point.completed_workflows += n,
            "blocked" => point.blocked_workflows += n,
            "failed" => point.failed_workflows += n,
            _ => {}
        }
    }

    let eval_filter = build_fact_filter(filter, "s", "cer.created_at", true);
    let mut stmt = conn.prepare(&format!(
        "SELECT substr(cer.created_at, 1, 10) AS day, cer.status, COUNT(*)
         FROM coding_eval_runs cer
         LEFT JOIN sessions s ON s.id = cer.session_id
         {}
         GROUP BY day, cer.status",
        eval_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(eval_filter.params.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            as_u64(row.get::<_, i64>(2)?),
        ))
    })?;
    for row in rows {
        let (date, status, n) = row?;
        let point = day_point(&mut days, date);
        match status.as_str() {
            "passed" => point.eval_passed += n,
            "failed" => point.eval_failed += n,
            _ => {}
        }
    }

    let pack_filter = build_fact_filter(filter, "s", "cepr.created_at", true);
    let mut stmt = conn.prepare(&format!(
        "SELECT substr(cepr.created_at, 1, 10) AS day, cepr.status, COUNT(*)
         FROM coding_eval_pack_runs cepr
         LEFT JOIN sessions s ON s.id = cepr.session_id
         {}
         GROUP BY day, cepr.status",
        pack_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(pack_filter.params.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            as_u64(row.get::<_, i64>(2)?),
        ))
    })?;
    for row in rows {
        let (date, status, n) = row?;
        let point = day_point(&mut days, date);
        match status.as_str() {
            "passed" => point.eval_pack_passed += n,
            "failed" => point.eval_pack_failed += n,
            _ => {}
        }
    }

    let strategy_filter = build_fact_filter(filter, "s", "cser.created_at", true);
    let mut stmt = conn.prepare(&format!(
        "SELECT substr(cser.created_at, 1, 10) AS day, cser.verdict, COUNT(*),
                COALESCE(SUM(cser.validation_violation_delta), 0),
                COALESCE(SUM(cser.scope_creep_delta), 0)
         FROM coding_strategy_effect_runs cser
         LEFT JOIN sessions s ON s.id = cser.session_id
         {}
         GROUP BY day, cser.verdict",
        strategy_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(strategy_filter.params.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            as_u64(row.get::<_, i64>(2)?),
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    for row in rows {
        let (date, verdict, n, validation_delta, scope_delta) = row?;
        let point = day_point(&mut days, date);
        point.validation_violation_delta += validation_delta;
        point.scope_creep_delta += scope_delta;
        match verdict.as_str() {
            "improved" => point.strategy_improved += n,
            "regressed" => point.strategy_regressed += n,
            "mixed" => point.strategy_mixed += n,
            _ => {}
        }
    }

    let proposal_created_filter = build_fact_filter(filter, "s", "cip.created_at", false);
    query_timeline_count(
        conn,
        &mut days,
        "proposals_created",
        &format!(
            "SELECT substr(cip.created_at, 1, 10) AS day, COUNT(*)
             FROM coding_improvement_proposals cip
             JOIN sessions s ON s.id = cip.session_id
             {}
             GROUP BY day",
            proposal_created_filter.where_sql
        ),
        &proposal_created_filter.params,
    )?;

    let proposal_applied_filter = append_condition(
        build_fact_filter(filter, "s", "cip.applied_at", false),
        "cip.applied_at IS NOT NULL",
    );
    query_timeline_count(
        conn,
        &mut days,
        "proposals_applied",
        &format!(
            "SELECT substr(cip.applied_at, 1, 10) AS day, COUNT(*)
             FROM coding_improvement_proposals cip
             JOIN sessions s ON s.id = cip.session_id
             {}
             GROUP BY day",
            proposal_applied_filter.where_sql
        ),
        &proposal_applied_filter.params,
    )?;

    let proposal_promoted_filter = append_condition(
        build_fact_filter(filter, "s", "cip.promoted_at", false),
        "cip.promoted_at IS NOT NULL",
    );
    query_timeline_count(
        conn,
        &mut days,
        "proposals_promoted",
        &format!(
            "SELECT substr(cip.promoted_at, 1, 10) AS day, COUNT(*)
             FROM coding_improvement_proposals cip
             JOIN sessions s ON s.id = cip.session_id
             {}
             GROUP BY day",
            proposal_promoted_filter.where_sql
        ),
        &proposal_promoted_filter.params,
    )?;

    let retro_filter = build_fact_filter(filter, "s", "cwr.updated_at", false);
    let mut stmt = conn.prepare(&format!(
        "SELECT substr(cwr.updated_at, 1, 10) AS day,
                COALESCE(SUM(json_array_length(cwr.recommendations_json)), 0)
         FROM coding_workflow_retros cwr
         JOIN sessions s ON s.id = cwr.session_id
         {}
         GROUP BY day",
        retro_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(retro_filter.params.iter()), |row| {
        Ok((row.get::<_, String>(0)?, as_u64(row.get::<_, i64>(1)?)))
    })?;
    for row in rows {
        let (date, n) = row?;
        day_point(&mut days, date).retro_recommendations += n;
    }

    Ok(days.into_values().collect())
}

fn query_projects(
    conn: &rusqlite::Connection,
    filter: &DashboardFilter,
    limit: usize,
) -> Result<Vec<CodingImprovementProjectBucket>> {
    let mut map: BTreeMap<String, ProjectAccumulator> = BTreeMap::new();

    let session_filter = build_fact_filter(filter, "s", "s.created_at", false);
    let mut stmt = conn.prepare(&format!(
        "SELECT s.project_id, COUNT(*)
         FROM sessions s
         {}
         GROUP BY s.project_id",
        session_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(session_filter.params.iter()), |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            as_u64(row.get::<_, i64>(1)?),
        ))
    })?;
    for row in rows {
        let (project_id, n) = row?;
        bucket(&mut map, project_id).sessions += n;
    }

    let workflow_filter = build_fact_filter(filter, "s", "wr.updated_at", false);
    let mut stmt = conn.prepare(&format!(
        "SELECT s.project_id, wr.state, COUNT(*)
         FROM workflow_runs wr
         JOIN sessions s ON s.id = wr.session_id
         {}
         GROUP BY s.project_id, wr.state",
        workflow_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(workflow_filter.params.iter()), |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, String>(1)?,
            as_u64(row.get::<_, i64>(2)?),
        ))
    })?;
    for row in rows {
        let (project_id, state, n) = row?;
        let bucket = bucket(&mut map, project_id);
        bucket.workflow_runs += n;
        if state == "completed" {
            bucket.completed_workflows += n;
        }
    }

    let eval_filter = build_fact_filter(filter, "s", "cer.created_at", true);
    let mut stmt = conn.prepare(&format!(
        "SELECT COALESCE(cer.project_id, s.project_id), cer.status, COUNT(*)
         FROM coding_eval_runs cer
         LEFT JOIN sessions s ON s.id = cer.session_id
         {}
         GROUP BY COALESCE(cer.project_id, s.project_id), cer.status",
        eval_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(eval_filter.params.iter()), |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, String>(1)?,
            as_u64(row.get::<_, i64>(2)?),
        ))
    })?;
    for row in rows {
        let (project_id, status, n) = row?;
        let bucket = bucket(&mut map, project_id);
        bucket.eval_runs += n;
        if status == "passed" {
            bucket.passed_eval_runs += n;
        }
    }

    let pack_filter = build_fact_filter(filter, "s", "cepr.created_at", true);
    let mut stmt = conn.prepare(&format!(
        "SELECT COALESCE(cepr.project_id, s.project_id), cepr.status, COUNT(*)
         FROM coding_eval_pack_runs cepr
         LEFT JOIN sessions s ON s.id = cepr.session_id
         {}
         GROUP BY COALESCE(cepr.project_id, s.project_id), cepr.status",
        pack_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(pack_filter.params.iter()), |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, String>(1)?,
            as_u64(row.get::<_, i64>(2)?),
        ))
    })?;
    for row in rows {
        let (project_id, status, n) = row?;
        let bucket = bucket(&mut map, project_id);
        bucket.eval_pack_runs += n;
        if status == "passed" {
            bucket.passed_eval_pack_runs += n;
        }
    }

    let strategy_filter = build_fact_filter(filter, "s", "cser.created_at", true);
    let mut stmt = conn.prepare(&format!(
        "SELECT COALESCE(cser.project_id, s.project_id), cser.verdict, COUNT(*)
         FROM coding_strategy_effect_runs cser
         LEFT JOIN sessions s ON s.id = cser.session_id
         {}
         GROUP BY COALESCE(cser.project_id, s.project_id), cser.verdict",
        strategy_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(strategy_filter.params.iter()), |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, String>(1)?,
            as_u64(row.get::<_, i64>(2)?),
        ))
    })?;
    for row in rows {
        let (project_id, verdict, n) = row?;
        let bucket = bucket(&mut map, project_id);
        bucket.strategy_effect_runs += n;
        if matches!(verdict.as_str(), "regressed" | "mixed") {
            bucket.regressed_strategy_effects += n;
        }
    }

    let review_filter = append_condition(
        build_fact_filter(filter, "s", "rf.updated_at", false),
        "rf.status = 'open' AND rf.severity IN ('p0','p1','critical','high')",
    );
    let mut stmt = conn.prepare(&format!(
        "SELECT s.project_id, COUNT(*)
         FROM review_findings rf
         JOIN sessions s ON s.id = rf.session_id
         {}
         GROUP BY s.project_id",
        review_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(review_filter.params.iter()), |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            as_u64(row.get::<_, i64>(1)?),
        ))
    })?;
    for row in rows {
        let (project_id, n) = row?;
        bucket(&mut map, project_id).open_review_blockers += n;
    }

    let retro_filter = build_fact_filter(filter, "s", "cwr.updated_at", false);
    let mut stmt = conn.prepare(&format!(
        "SELECT COALESCE(cwr.project_id, s.project_id),
                COALESCE(SUM(json_array_length(cwr.recommendations_json)), 0)
         FROM coding_workflow_retros cwr
         JOIN sessions s ON s.id = cwr.session_id
         {}
         GROUP BY COALESCE(cwr.project_id, s.project_id)",
        retro_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(retro_filter.params.iter()), |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            as_u64(row.get::<_, i64>(1)?),
        ))
    })?;
    for row in rows {
        let (project_id, n) = row?;
        bucket(&mut map, project_id).retro_recommendations += n;
    }

    let proposal_filter = build_fact_filter(filter, "s", "cip.updated_at", false);
    let mut stmt = conn.prepare(&format!(
        "SELECT COALESCE(cip.project_id, s.project_id), cip.status, COUNT(*)
         FROM coding_improvement_proposals cip
         JOIN sessions s ON s.id = cip.session_id
         {}
         GROUP BY COALESCE(cip.project_id, s.project_id), cip.status",
        proposal_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(proposal_filter.params.iter()), |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, String>(1)?,
            as_u64(row.get::<_, i64>(2)?),
        ))
    })?;
    for row in rows {
        let (project_id, status, n) = row?;
        let bucket = bucket(&mut map, project_id);
        match status.as_str() {
            "draft" => bucket.draft_proposals += n,
            "applied" => bucket.applied_proposals += n,
            "promoted" => bucket.promoted_proposals += n,
            "promotion_failed" => bucket.promotion_failures += n,
            _ => {}
        }
    }

    let names = load_project_names(conn)?;
    let mut buckets: Vec<_> = map
        .into_values()
        .filter(|bucket| {
            bucket.workflow_runs > 0
                || bucket.eval_runs > 0
                || bucket.eval_pack_runs > 0
                || bucket.strategy_effect_runs > 0
                || bucket.open_review_blockers > 0
                || bucket.retro_recommendations > 0
                || bucket.draft_proposals > 0
                || bucket.applied_proposals > 0
                || bucket.promoted_proposals > 0
                || bucket.promotion_failures > 0
        })
        .map(|bucket| {
            let distillation_candidates = bucket.draft_proposals
                + bucket.applied_proposals
                + bucket.promotion_failures
                + bucket.retro_recommendations;
            CodingImprovementProjectBucket {
                project_name: bucket
                    .project_id
                    .as_ref()
                    .and_then(|id| names.get(id).cloned()),
                project_id: bucket.project_id,
                sessions: bucket.sessions,
                workflow_runs: bucket.workflow_runs,
                workflow_completion_rate: ratio(bucket.completed_workflows, bucket.workflow_runs),
                eval_runs: bucket.eval_runs,
                eval_success_rate: ratio(bucket.passed_eval_runs, bucket.eval_runs),
                eval_pack_runs: bucket.eval_pack_runs,
                eval_pack_pass_rate: ratio(bucket.passed_eval_pack_runs, bucket.eval_pack_runs),
                strategy_effect_runs: bucket.strategy_effect_runs,
                regressed_strategy_effects: bucket.regressed_strategy_effects,
                open_review_blockers: bucket.open_review_blockers,
                retro_recommendations: bucket.retro_recommendations,
                draft_proposals: bucket.draft_proposals,
                applied_proposals: bucket.applied_proposals,
                promoted_proposals: bucket.promoted_proposals,
                promotion_failures: bucket.promotion_failures,
                distillation_candidates,
            }
        })
        .collect();
    buckets.sort_by(|a, b| {
        b.distillation_candidates
            .cmp(&a.distillation_candidates)
            .then_with(|| b.open_review_blockers.cmp(&a.open_review_blockers))
            .then_with(|| b.workflow_runs.cmp(&a.workflow_runs))
            .then_with(|| a.project_id.cmp(&b.project_id))
    });
    buckets.truncate(limit);
    Ok(buckets)
}

fn query_domain_quality_dashboard(
    conn: &rusqlite::Connection,
    filter: &DashboardFilter,
    limit: usize,
) -> Result<DomainQualityDashboard> {
    Ok(DomainQualityDashboard {
        overview: query_domain_quality_overview(conn, filter)?,
        timeline: query_domain_quality_timeline(conn, filter)?,
        by_domain: query_domain_quality_by_domain(conn, filter, limit)?,
        top_blockers: query_domain_quality_top_blockers(conn, filter, limit)?,
        recent_runs: query_domain_quality_recent_runs(conn, filter, limit)?,
    })
}

fn query_domain_quality_overview(
    conn: &rusqlite::Connection,
    filter: &DashboardFilter,
) -> Result<DomainQualityDashboardOverview> {
    let quality_filter = build_fact_filter(filter, "s", "dqr.updated_at", false);
    let mut stmt = conn.prepare(&format!(
        "SELECT dqr.state, COUNT(*)
         FROM domain_quality_runs dqr
         JOIN sessions s ON s.id = dqr.session_id
         {}
         GROUP BY dqr.state",
        quality_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(quality_filter.params.iter()), |row| {
        Ok((row.get::<_, String>(0)?, as_u64(row.get::<_, i64>(1)?)))
    })?;
    let mut overview = DomainQualityDashboardOverview::default();
    for row in rows {
        let (state, n) = row?;
        overview.quality_runs += n;
        match state.as_str() {
            "completed" => overview.completed_quality_runs += n,
            "blocked" => overview.blocked_quality_runs += n,
            "failed" => overview.failed_quality_runs += n,
            "needs_user" => overview.needs_user_quality_runs += n,
            _ => {}
        }
    }
    overview.quality_completion_rate =
        ratio(overview.completed_quality_runs, overview.quality_runs);

    overview.approval_blockers = count_domain_quality_approval_blockers(conn, filter)?;

    let eval_filter = build_fact_filter(filter, "s", "der.created_at", false);
    let mut stmt = conn.prepare(&format!(
        "SELECT der.status, COUNT(*), COALESCE(SUM(der.score), 0.0)
         FROM domain_eval_runs der
         JOIN sessions s ON s.id = der.session_id
         {}
         GROUP BY der.status",
        eval_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(eval_filter.params.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            as_u64(row.get::<_, i64>(1)?),
            row.get::<_, f64>(2)?,
        ))
    })?;
    let mut eval_score_sum = 0.0;
    for row in rows {
        let (status, n, score_sum) = row?;
        overview.eval_runs += n;
        eval_score_sum += score_sum;
        match status.as_str() {
            "passed" => overview.passed_eval_runs += n,
            "failed" => overview.failed_eval_runs += n,
            _ => {}
        }
    }
    overview.eval_pass_rate = ratio(overview.passed_eval_runs, overview.eval_runs);
    if overview.eval_runs > 0 {
        overview.average_eval_score = Some(eval_score_sum / overview.eval_runs as f64);
    }

    let quality_domain_filter = build_fact_filter(filter, "s", "dqr.updated_at", false);
    let eval_domain_filter = build_fact_filter(filter, "s", "der.created_at", false);
    let mut domain_params = quality_domain_filter.params.clone();
    domain_params.extend(eval_domain_filter.params.clone());
    overview.domains_covered = count(
        conn,
        &format!(
            "SELECT COUNT(*) FROM (
                SELECT DISTINCT dqr.domain
                FROM domain_quality_runs dqr
                JOIN sessions s ON s.id = dqr.session_id
                {}
                UNION
                SELECT DISTINCT der.domain
                FROM domain_eval_runs der
                JOIN sessions s ON s.id = der.session_id
                {}
             ) AS domains",
            quality_domain_filter.where_sql, eval_domain_filter.where_sql
        ),
        &domain_params,
    )?;

    let proposal_filter = append_condition(
        build_fact_filter(filter, "s", "cip.updated_at", false),
        "cip.source_type = 'domain_quality'",
    );
    let mut stmt = conn.prepare(&format!(
        "SELECT cip.status, COUNT(*)
         FROM coding_improvement_proposals cip
         JOIN sessions s ON s.id = cip.session_id
         {}
         GROUP BY cip.status",
        proposal_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(proposal_filter.params.iter()), |row| {
        Ok((row.get::<_, String>(0)?, as_u64(row.get::<_, i64>(1)?)))
    })?;
    for row in rows {
        let (status, n) = row?;
        match status.as_str() {
            "draft" => overview.draft_domain_proposals += n,
            "promoted" => overview.promoted_domain_proposals += n,
            _ => {}
        }
    }

    Ok(overview)
}

fn query_domain_quality_timeline(
    conn: &rusqlite::Connection,
    filter: &DashboardFilter,
) -> Result<Vec<DomainQualityTimelinePoint>> {
    let mut days: BTreeMap<String, DomainQualityTimelinePoint> = BTreeMap::new();

    let quality_filter = build_fact_filter(filter, "s", "dqr.updated_at", false);
    let mut stmt = conn.prepare(&format!(
        "SELECT substr(dqr.updated_at, 1, 10), dqr.state, COUNT(*)
         FROM domain_quality_runs dqr
         JOIN sessions s ON s.id = dqr.session_id
         {}
         GROUP BY substr(dqr.updated_at, 1, 10), dqr.state",
        quality_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(quality_filter.params.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            as_u64(row.get::<_, i64>(2)?),
        ))
    })?;
    for row in rows {
        let (date, state, n) = row?;
        let point = domain_day_point(&mut days, date);
        point.quality_runs += n;
        match state.as_str() {
            "completed" => point.completed_quality_runs += n,
            "blocked" => point.blocked_quality_runs += n,
            "failed" => point.failed_quality_runs += n,
            "needs_user" => point.needs_user_quality_runs += n,
            _ => {}
        }
    }

    let approval_filter = append_condition(
        build_fact_filter(filter, "s", "dqc.updated_at", false),
        "dqc.check_type = 'approval'
         AND dqc.status IN ('failed','blocked','needs_user')",
    );
    let mut stmt = conn.prepare(&format!(
        "SELECT substr(dqc.updated_at, 1, 10), COUNT(*)
         FROM domain_quality_checks dqc
         JOIN sessions s ON s.id = dqc.session_id
         {}
         GROUP BY substr(dqc.updated_at, 1, 10)",
        approval_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(approval_filter.params.iter()), |row| {
        Ok((row.get::<_, String>(0)?, as_u64(row.get::<_, i64>(1)?)))
    })?;
    for row in rows {
        let (date, n) = row?;
        domain_day_point(&mut days, date).approval_blockers += n;
    }

    let eval_filter = build_fact_filter(filter, "s", "der.created_at", false);
    let mut stmt = conn.prepare(&format!(
        "SELECT substr(der.created_at, 1, 10), der.status, COUNT(*)
         FROM domain_eval_runs der
         JOIN sessions s ON s.id = der.session_id
         {}
         GROUP BY substr(der.created_at, 1, 10), der.status",
        eval_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(eval_filter.params.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            as_u64(row.get::<_, i64>(2)?),
        ))
    })?;
    for row in rows {
        let (date, status, n) = row?;
        let point = domain_day_point(&mut days, date);
        match status.as_str() {
            "passed" => point.eval_passed += n,
            "failed" => point.eval_failed += n,
            _ => {}
        }
    }

    let proposal_filter = append_condition(
        build_fact_filter(filter, "s", "cip.created_at", false),
        "cip.source_type = 'domain_quality'",
    );
    let mut stmt = conn.prepare(&format!(
        "SELECT substr(cip.created_at, 1, 10), COUNT(*)
         FROM coding_improvement_proposals cip
         JOIN sessions s ON s.id = cip.session_id
         {}
         GROUP BY substr(cip.created_at, 1, 10)",
        proposal_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(proposal_filter.params.iter()), |row| {
        Ok((row.get::<_, String>(0)?, as_u64(row.get::<_, i64>(1)?)))
    })?;
    for row in rows {
        let (date, n) = row?;
        domain_day_point(&mut days, date).proposals_created += n;
    }

    Ok(days.into_values().collect())
}

fn query_domain_quality_by_domain(
    conn: &rusqlite::Connection,
    filter: &DashboardFilter,
    limit: usize,
) -> Result<Vec<DomainQualityDomainBucket>> {
    let mut map: BTreeMap<String, DomainAccumulator> = BTreeMap::new();

    let quality_filter = build_fact_filter(filter, "s", "dqr.updated_at", false);
    let mut stmt = conn.prepare(&format!(
        "SELECT dqr.domain, dqr.state, COUNT(*)
         FROM domain_quality_runs dqr
         JOIN sessions s ON s.id = dqr.session_id
         {}
         GROUP BY dqr.domain, dqr.state",
        quality_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(quality_filter.params.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            as_u64(row.get::<_, i64>(2)?),
        ))
    })?;
    for row in rows {
        let (domain, state, n) = row?;
        let bucket = domain_bucket(&mut map, domain);
        bucket.quality_runs += n;
        match state.as_str() {
            "completed" => bucket.completed_quality_runs += n,
            "blocked" => bucket.blocked_quality_runs += n,
            "failed" => bucket.failed_quality_runs += n,
            "needs_user" => bucket.needs_user_quality_runs += n,
            _ => {}
        }
    }

    let approval_filter = append_condition(
        build_fact_filter(filter, "s", "dqc.updated_at", false),
        "dqc.check_type = 'approval'
         AND dqc.status IN ('failed','blocked','needs_user')",
    );
    let mut stmt = conn.prepare(&format!(
        "SELECT dqr.domain, COUNT(*)
         FROM domain_quality_checks dqc
         JOIN domain_quality_runs dqr ON dqr.id = dqc.run_id
         JOIN sessions s ON s.id = dqc.session_id
         {}
         GROUP BY dqr.domain",
        approval_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(approval_filter.params.iter()), |row| {
        Ok((row.get::<_, String>(0)?, as_u64(row.get::<_, i64>(1)?)))
    })?;
    for row in rows {
        let (domain, n) = row?;
        domain_bucket(&mut map, domain).approval_blockers += n;
    }

    let eval_filter = build_fact_filter(filter, "s", "der.created_at", false);
    let mut stmt = conn.prepare(&format!(
        "SELECT der.domain, der.status, COUNT(*), COALESCE(SUM(der.score), 0.0)
         FROM domain_eval_runs der
         JOIN sessions s ON s.id = der.session_id
         {}
         GROUP BY der.domain, der.status",
        eval_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(eval_filter.params.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            as_u64(row.get::<_, i64>(2)?),
            row.get::<_, f64>(3)?,
        ))
    })?;
    for row in rows {
        let (domain, status, n, score_sum) = row?;
        let bucket = domain_bucket(&mut map, domain);
        bucket.eval_runs += n;
        bucket.eval_score_sum += score_sum;
        if status == "passed" {
            bucket.passed_eval_runs += n;
        }
    }

    let proposal_filter = append_condition(
        build_fact_filter(filter, "s", "cip.updated_at", false),
        "cip.source_type = 'domain_quality'",
    );
    let mut stmt = conn.prepare(&format!(
        "SELECT COALESCE(
                    NULLIF(CAST(json_extract(cip.payload_json, '$.domain') AS TEXT), ''),
                    'unknown'
                ) AS domain,
                cip.status,
                COUNT(*)
         FROM coding_improvement_proposals cip
         JOIN sessions s ON s.id = cip.session_id
         {}
         GROUP BY domain, cip.status",
        proposal_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(proposal_filter.params.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            as_u64(row.get::<_, i64>(2)?),
        ))
    })?;
    for row in rows {
        let (domain, status, n) = row?;
        let bucket = domain_bucket(&mut map, domain);
        match status.as_str() {
            "draft" => bucket.draft_proposals += n,
            "promoted" => bucket.promoted_proposals += n,
            _ => {}
        }
    }

    let mut buckets: Vec<_> = map
        .into_values()
        .map(|bucket| DomainQualityDomainBucket {
            domain: bucket.domain,
            quality_runs: bucket.quality_runs,
            completed_quality_runs: bucket.completed_quality_runs,
            blocked_quality_runs: bucket.blocked_quality_runs,
            failed_quality_runs: bucket.failed_quality_runs,
            needs_user_quality_runs: bucket.needs_user_quality_runs,
            quality_completion_rate: ratio(bucket.completed_quality_runs, bucket.quality_runs),
            approval_blockers: bucket.approval_blockers,
            eval_runs: bucket.eval_runs,
            eval_pass_rate: ratio(bucket.passed_eval_runs, bucket.eval_runs),
            average_eval_score: if bucket.eval_runs > 0 {
                Some(bucket.eval_score_sum / bucket.eval_runs as f64)
            } else {
                None
            },
            draft_proposals: bucket.draft_proposals,
            promoted_proposals: bucket.promoted_proposals,
        })
        .collect();
    buckets.sort_by(|a, b| {
        (b.blocked_quality_runs + b.failed_quality_runs + b.needs_user_quality_runs)
            .cmp(&(a.blocked_quality_runs + a.failed_quality_runs + a.needs_user_quality_runs))
            .then_with(|| b.approval_blockers.cmp(&a.approval_blockers))
            .then_with(|| b.quality_runs.cmp(&a.quality_runs))
            .then_with(|| a.domain.cmp(&b.domain))
    });
    buckets.truncate(limit);
    Ok(buckets)
}

fn query_domain_quality_top_blockers(
    conn: &rusqlite::Connection,
    filter: &DashboardFilter,
    limit: usize,
) -> Result<Vec<DomainQualityBlockerBucket>> {
    let blocker_filter = append_condition(
        build_fact_filter(filter, "s", "dqc.updated_at", false),
        "dqc.status IN ('failed','blocked','needs_user')",
    );
    let mut stmt = conn.prepare(&format!(
        "SELECT dqc.check_type, dqc.status, dqc.severity, COUNT(*)
         FROM domain_quality_checks dqc
         JOIN sessions s ON s.id = dqc.session_id
         {}
         GROUP BY dqc.check_type, dqc.status, dqc.severity
         ORDER BY COUNT(*) DESC, dqc.check_type ASC, dqc.status ASC
         LIMIT {}",
        blocker_filter.where_sql, limit
    ))?;
    let rows = stmt.query_map(params_from_iter(blocker_filter.params.iter()), |row| {
        let check_type = row.get::<_, String>(0)?;
        let status = row.get::<_, String>(1)?;
        let severity = row.get::<_, String>(2)?;
        let category = format!("{check_type}:{status}");
        Ok(DomainQualityBlockerBucket {
            label: domain_quality_blocker_label(&check_type, &status).to_string(),
            severity,
            category,
            count: as_u64(row.get::<_, i64>(3)?),
        })
    })?;
    collect(rows)
}

fn query_domain_quality_recent_runs(
    conn: &rusqlite::Connection,
    filter: &DashboardFilter,
    limit: usize,
) -> Result<Vec<DomainQualityRunItem>> {
    let quality_filter = build_fact_filter(filter, "s", "dqr.updated_at", false);
    let mut stmt = conn.prepare(&format!(
        "SELECT dqr.id, dqr.session_id, s.project_id, dqr.domain, dqr.template_id,
                dqr.template_version, dqr.state, dqr.summary, dqr.updated_at,
                COALESCE(SUM(CASE WHEN dqc.status IN ('failed','blocked') THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN dqc.status = 'needs_user' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN dqc.check_type = 'approval'
                                    AND dqc.status IN ('failed','blocked','needs_user')
                               THEN 1 ELSE 0 END), 0)
         FROM domain_quality_runs dqr
         JOIN sessions s ON s.id = dqr.session_id
         LEFT JOIN domain_quality_checks dqc ON dqc.run_id = dqr.id
         {}
         GROUP BY dqr.id, dqr.session_id, s.project_id, dqr.domain, dqr.template_id,
                  dqr.template_version, dqr.state, dqr.summary, dqr.updated_at
         ORDER BY dqr.updated_at DESC
         LIMIT {}",
        quality_filter.where_sql, limit
    ))?;
    let rows = stmt.query_map(params_from_iter(quality_filter.params.iter()), |row| {
        Ok(DomainQualityRunItem {
            id: row.get(0)?,
            session_id: row.get(1)?,
            project_id: row.get(2)?,
            domain: row.get(3)?,
            template_id: row.get(4)?,
            template_version: row.get(5)?,
            state: row.get(6)?,
            summary: row.get(7)?,
            updated_at: row.get(8)?,
            failed_checks: as_u64(row.get::<_, i64>(9)?),
            needs_user_checks: as_u64(row.get::<_, i64>(10)?),
            approval_blockers: as_u64(row.get::<_, i64>(11)?),
        })
    })?;
    collect(rows)
}

fn query_top_failures(
    conn: &rusqlite::Connection,
    filter: &DashboardFilter,
    limit: usize,
) -> Result<Vec<CodingImprovementFailureBucket>> {
    let proposal_filter = append_condition(
        build_fact_filter(filter, "s", "cip.updated_at", false),
        "cip.kind = 'eval_candidate'",
    );
    let mut stmt = conn.prepare(&format!(
        "SELECT COALESCE(
                    NULLIF(CAST(json_extract(cip.payload_json, '$.failure.category') AS TEXT), ''),
                    NULLIF(CAST(json_extract(cip.payload_json, '$.category') AS TEXT), ''),
                    NULLIF(cip.source_id, ''),
                    'uncategorized'
                ) AS category,
                COUNT(*) AS count
         FROM coding_improvement_proposals cip
         JOIN sessions s ON s.id = cip.session_id
         {}
         GROUP BY category
         ORDER BY count DESC, category ASC
         LIMIT {}",
        proposal_filter.where_sql, limit
    ))?;
    let rows = stmt.query_map(params_from_iter(proposal_filter.params.iter()), |row| {
        let category = row.get::<_, String>(0)?;
        Ok(CodingImprovementFailureBucket {
            label: failure_label(&category).to_string(),
            severity: failure_severity(&category).to_string(),
            category,
            count: as_u64(row.get::<_, i64>(1)?),
        })
    })?;
    collect(rows)
}

fn query_tool_call_failures(
    conn: &rusqlite::Connection,
    filter: &DashboardFilter,
) -> Result<Vec<CodingImprovementFailureBucket>> {
    let missing_tool_calls = count_missing_tool_call_runs(conn, filter)?;
    if missing_tool_calls == 0 {
        return Ok(Vec::new());
    }
    Ok(vec![CodingImprovementFailureBucket {
        category: "missing_tool_call".to_string(),
        label: failure_label("missing_tool_call").to_string(),
        severity: failure_severity("missing_tool_call").to_string(),
        count: missing_tool_calls,
    }])
}

fn query_proposal_statuses(
    conn: &rusqlite::Connection,
    filter: &DashboardFilter,
) -> Result<Vec<CodingImprovementStatusBucket>> {
    let proposal_filter = build_fact_filter(filter, "s", "cip.updated_at", false);
    let mut stmt = conn.prepare(&format!(
        "SELECT cip.status, COUNT(*)
         FROM coding_improvement_proposals cip
         JOIN sessions s ON s.id = cip.session_id
         {}
         GROUP BY cip.status
         ORDER BY COUNT(*) DESC, cip.status ASC",
        proposal_filter.where_sql
    ))?;
    let rows = stmt.query_map(params_from_iter(proposal_filter.params.iter()), |row| {
        Ok(CodingImprovementStatusBucket {
            status: row.get(0)?,
            count: as_u64(row.get::<_, i64>(1)?),
        })
    })?;
    collect(rows)
}

fn query_latest_strategy_effects(
    conn: &rusqlite::Connection,
    filter: &DashboardFilter,
    limit: usize,
) -> Result<Vec<CodingImprovementStrategyEffectItem>> {
    let strategy_filter = build_fact_filter(filter, "s", "cser.created_at", true);
    let mut stmt = conn.prepare(&format!(
        "SELECT cser.id, COALESCE(cser.project_id, s.project_id), cser.strategy_type,
                cser.baseline_label, cser.candidate_label, cser.verdict, cser.compared_cases,
                cser.pass_rate_delta, cser.average_score_delta, cser.context_recall_delta,
                cser.validation_violation_delta, cser.scope_creep_delta,
                cser.execution_failure_delta, cser.created_at
         FROM coding_strategy_effect_runs cser
         LEFT JOIN sessions s ON s.id = cser.session_id
         {}
         ORDER BY cser.created_at DESC
         LIMIT {}",
        strategy_filter.where_sql, limit
    ))?;
    let rows = stmt.query_map(params_from_iter(strategy_filter.params.iter()), |row| {
        Ok(CodingImprovementStrategyEffectItem {
            id: row.get(0)?,
            project_id: row.get(1)?,
            strategy_type: row.get(2)?,
            baseline_label: row.get(3)?,
            candidate_label: row.get(4)?,
            verdict: row.get(5)?,
            compared_cases: as_u64(row.get::<_, i64>(6)?),
            pass_rate_delta: row.get(7)?,
            average_score_delta: row.get(8)?,
            context_recall_delta: row.get(9)?,
            validation_violation_delta: row.get(10)?,
            scope_creep_delta: row.get(11)?,
            execution_failure_delta: row.get(12)?,
            created_at: row.get(13)?,
        })
    })?;
    collect(rows)
}

fn query_latest_retros(
    conn: &rusqlite::Connection,
    filter: &DashboardFilter,
    limit: usize,
) -> Result<Vec<CodingImprovementRetroItem>> {
    let retro_filter = build_fact_filter(filter, "s", "cwr.updated_at", false);
    let mut stmt = conn.prepare(&format!(
        "SELECT cwr.id, cwr.session_id, cwr.project_id, cwr.workflow_run_id,
                cwr.run_state, cwr.summary, cwr.recommendations_json, cwr.updated_at
         FROM coding_workflow_retros cwr
         JOIN sessions s ON s.id = cwr.session_id
         {}
         ORDER BY cwr.updated_at DESC
         LIMIT {}",
        retro_filter.where_sql, limit
    ))?;
    let rows = stmt.query_map(params_from_iter(retro_filter.params.iter()), |row| {
        let recommendations_json: String = row.get(6)?;
        Ok(CodingImprovementRetroItem {
            id: row.get(0)?,
            session_id: row.get(1)?,
            project_id: row.get(2)?,
            workflow_run_id: row.get(3)?,
            run_state: row.get(4)?,
            summary: row.get(5)?,
            recommendations: serde_json::from_str(&recommendations_json).unwrap_or_default(),
            updated_at: row.get(7)?,
        })
    })?;
    collect(rows)
}

fn build_fact_filter(
    filter: &DashboardFilter,
    session_alias: &str,
    time_expr: &str,
    allow_null_session: bool,
) -> SqlFilter {
    let mut clauses = Vec::new();
    let mut params = Vec::new();

    if allow_null_session {
        clauses.push(format!(
            "({session_alias}.id IS NULL OR ({session_alias}.is_cron = 0 AND {session_alias}.parent_session_id IS NULL AND {session_alias}.incognito = 0))"
        ));
    } else {
        clauses.push(format!("{session_alias}.is_cron = 0"));
        clauses.push(format!("{session_alias}.parent_session_id IS NULL"));
        clauses.push(format!("{session_alias}.incognito = 0"));
    }

    if let Some(start) = filter.start_date.as_ref().filter(|value| !value.is_empty()) {
        clauses.push(format!("{time_expr} >= ?"));
        params.push(start.clone());
    }
    if let Some(end) = filter.end_date.as_ref().filter(|value| !value.is_empty()) {
        clauses.push(format!("{time_expr} <= ?"));
        params.push(end.clone());
    }
    if let Some(agent_id) = filter.agent_id.as_ref().filter(|value| !value.is_empty()) {
        clauses.push(format!("{session_alias}.agent_id = ?"));
        params.push(agent_id.clone());
    }
    if let Some(provider_id) = filter
        .provider_id
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        clauses.push(format!("{session_alias}.provider_id = ?"));
        params.push(provider_id.clone());
    }
    if let Some(model_id) = filter.model_id.as_ref().filter(|value| !value.is_empty()) {
        clauses.push(format!("{session_alias}.model_id = ?"));
        params.push(model_id.clone());
    }

    SqlFilter {
        where_sql: format!("WHERE {}", clauses.join(" AND ")),
        params,
    }
}

fn append_condition(mut filter: SqlFilter, condition: &str) -> SqlFilter {
    if filter.where_sql.is_empty() {
        filter.where_sql = format!("WHERE {condition}");
    } else {
        filter.where_sql.push_str(" AND ");
        filter.where_sql.push_str(condition);
    }
    filter
}

fn count(conn: &rusqlite::Connection, sql: &str, params: &[String]) -> Result<u64> {
    let n = conn.query_row(sql, params_from_iter(params.iter()), |row| {
        row.get::<_, i64>(0)
    })?;
    Ok(as_u64(n))
}

fn count_by(
    conn: &rusqlite::Connection,
    sql: &str,
    params: &[String],
) -> Result<Vec<(String, u64)>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        Ok((row.get::<_, String>(0)?, as_u64(row.get::<_, i64>(1)?)))
    })?;
    collect(rows)
}

fn count_missing_tool_call_runs(
    conn: &rusqlite::Connection,
    filter: &DashboardFilter,
) -> Result<u64> {
    let eval_filter = append_condition(
        build_fact_filter(filter, "s", "cer.created_at", true),
        "cer.source_type = 'coding_task_eval'
         AND COALESCE(
            CAST(json_extract(cer.metrics_json, '$.metrics.executionMode') AS TEXT),
            CAST(json_extract(cer.metrics_json, '$.metrics.execution_mode') AS TEXT),
            CAST(json_extract(cer.metrics_json, '$.executionMode') AS TEXT),
            CAST(json_extract(cer.metrics_json, '$.execution_mode') AS TEXT),
            ''
         ) = 'agent'
         AND COALESCE(
            json_array_length(json_extract(cer.metrics_json, '$.metrics.agentExecution.toolCalls')),
            json_array_length(json_extract(cer.metrics_json, '$.metrics.agent_execution.tool_calls')),
            json_array_length(json_extract(cer.metrics_json, '$.metrics.execution_tool_calls')),
            json_array_length(json_extract(cer.metrics_json, '$.execution_tool_calls')),
            0
         ) = 0",
    );
    count(
        conn,
        &format!(
            "SELECT COUNT(*)
             FROM coding_eval_runs cer
             LEFT JOIN sessions s ON s.id = cer.session_id
             {}",
            eval_filter.where_sql
        ),
        &eval_filter.params,
    )
}

fn query_timeline_count(
    conn: &rusqlite::Connection,
    days: &mut BTreeMap<String, CodingImprovementTimelinePoint>,
    field: &str,
    sql: &str,
    params: &[String],
) -> Result<()> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        Ok((row.get::<_, String>(0)?, as_u64(row.get::<_, i64>(1)?)))
    })?;
    for row in rows {
        let (date, n) = row?;
        let point = day_point(days, date);
        match field {
            "proposals_created" => point.proposals_created += n,
            "proposals_applied" => point.proposals_applied += n,
            "proposals_promoted" => point.proposals_promoted += n,
            _ => {}
        }
    }
    Ok(())
}

fn day_point(
    days: &mut BTreeMap<String, CodingImprovementTimelinePoint>,
    date: String,
) -> &mut CodingImprovementTimelinePoint {
    days.entry(date.clone())
        .or_insert_with(|| CodingImprovementTimelinePoint {
            date,
            ..Default::default()
        })
}

fn domain_day_point(
    days: &mut BTreeMap<String, DomainQualityTimelinePoint>,
    date: String,
) -> &mut DomainQualityTimelinePoint {
    days.entry(date.clone())
        .or_insert_with(|| DomainQualityTimelinePoint {
            date,
            ..Default::default()
        })
}

fn collect<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>> {
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn bucket(
    map: &mut BTreeMap<String, ProjectAccumulator>,
    project_id: Option<String>,
) -> &mut ProjectAccumulator {
    let key = project_key(project_id.as_deref());
    map.entry(key).or_insert_with(|| ProjectAccumulator {
        project_id,
        ..Default::default()
    })
}

fn domain_bucket(
    map: &mut BTreeMap<String, DomainAccumulator>,
    domain: String,
) -> &mut DomainAccumulator {
    map.entry(domain.clone())
        .or_insert_with(|| DomainAccumulator {
            domain,
            ..Default::default()
        })
}

fn project_key(project_id: Option<&str>) -> String {
    project_id.unwrap_or("__unassigned__").to_string()
}

fn load_project_names(conn: &rusqlite::Connection) -> Result<HashMap<String, String>> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM sqlite_master
         WHERE type = 'table' AND name = 'projects'",
        [],
        |row| row.get(0),
    )?;
    if exists == 0 {
        return Ok(HashMap::new());
    }

    let mut stmt = conn.prepare("SELECT id, name FROM projects WHERE archived = 0")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(collect(rows)?.into_iter().collect())
}

fn ratio(numerator: u64, denominator: u64) -> Option<f64> {
    if denominator == 0 {
        None
    } else {
        Some(numerator as f64 / denominator as f64)
    }
}

fn as_u64(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

fn count_domain_quality_approval_blockers(
    conn: &rusqlite::Connection,
    filter: &DashboardFilter,
) -> Result<u64> {
    let approval_filter = append_condition(
        build_fact_filter(filter, "s", "dqc.updated_at", false),
        "dqc.check_type = 'approval'
         AND dqc.status IN ('failed','blocked','needs_user')",
    );
    count(
        conn,
        &format!(
            "SELECT COUNT(*)
             FROM domain_quality_checks dqc
             JOIN sessions s ON s.id = dqc.session_id
             {}",
            approval_filter.where_sql
        ),
        &approval_filter.params,
    )
}

fn failure_label(category: &str) -> &'static str {
    match category {
        "validation_failed" => "Validation failed",
        "eval_failed" => "Eval failed",
        "review_blocker" => "Review blocker",
        "repair_loop_exhausted" => "Repair loop exhausted",
        "no_effective_diff_progress" => "No effective diff progress",
        "permission_stall" => "Permission stall",
        "context_miss" => "Context miss",
        "missing_tool_call" => "Missing tool call",
        "verification_selection_gap" => "Verification selection gap",
        "workflow_failed" => "Workflow failed",
        "workflow_blocked" => "Workflow blocked",
        "goal_failed" => "Goal failed",
        _ => "Uncategorized",
    }
}

fn failure_severity(category: &str) -> &'static str {
    match category {
        "review_blocker" | "repair_loop_exhausted" | "eval_failed" | "validation_failed" => "high",
        "permission_stall"
        | "context_miss"
        | "missing_tool_call"
        | "verification_selection_gap" => "medium",
        _ => "low",
    }
}

fn domain_quality_blocker_label(check_type: &str, status: &str) -> &'static str {
    match (check_type, status) {
        ("approval", "needs_user") => "Approval waiting",
        ("approval", _) => "Approval safety",
        ("verification", "needs_user") => "Verification needs user",
        ("verification", _) => "Verification gap",
        ("review", _) => "Review gap",
        ("planning", _) => "Planning gap",
        ("evidence", _) => "Evidence gap",
        (_, "needs_user") => "Needs user",
        (_, "blocked") => "Blocked check",
        (_, "failed") => "Failed check",
        _ => "Domain quality blocker",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_loader::DEFAULT_AGENT_ID;
    use rusqlite::params;
    use serde_json::json;

    fn test_db() -> (tempfile::TempDir, Arc<SessionDB>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(SessionDB::open(&dir.path().join("sessions.db")).expect("session db"));
        {
            let conn = db.conn.lock().expect("lock");
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS projects (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    archived INTEGER NOT NULL DEFAULT 0
                );",
            )
            .expect("projects table");
        }
        (dir, db)
    }

    #[test]
    fn dashboard_rolls_up_project_learning_signals() {
        let (_dir, db) = test_db();
        let now = now_rfc3339();
        let project_id = "proj-dashboard";
        let session = db
            .create_session_with_project(DEFAULT_AGENT_ID, Some(project_id), None)
            .unwrap();

        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO projects (id, name, archived) VALUES (?1, ?2, 0)",
                params![project_id, "Dashboard Project"],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO workflow_runs (
                    id, session_id, kind, state, execution_mode, script_hash,
                    script_source, created_at, updated_at, completed_at
                 ) VALUES (?1, ?2, 'coding.workflow', 'completed', 'guarded',
                    'hash', 'script', ?3, ?3, ?3)",
                params!["wfr_dashboard", session.id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO coding_eval_runs (
                    id, session_id, project_id, suite, name, status,
                    metrics_json, source_type, source_id, created_at
                 ) VALUES (?1, ?2, ?3, 'suite', 'eval', 'passed', '{}', 'test', 'eval', ?4)",
                params!["cer_dashboard", session.id, project_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO coding_eval_runs (
                    id, session_id, project_id, suite, name, status,
                    metrics_json, source_type, source_id, created_at
                 ) VALUES (
                    ?1, ?2, ?3, 'task_level_coding_eval', 'tool-call', 'failed',
                    ?4, 'coding_task_eval', 'tool-call', ?5
                 )",
                params![
                    "cer_tool_call_dashboard",
                    session.id,
                    project_id,
                    json!({"metrics":{"executionMode":"agent","agentExecution":{"toolCalls":[]}}})
                        .to_string(),
                    now
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO coding_eval_pack_runs (
                    id, session_id, project_id, pack_id, source_doc, label,
                    baseline_kind, status, selected_cases, automated_cases,
                    skipped_cases, passed_cases, failed_cases, total_checks,
                    report_json, source_type, source_id, created_at
                 ) VALUES (
                    ?1, ?2, ?3, 'phase5-gold-task-pack', 'docs/roadmap/coding-eval.md',
                    'external smoke', 'external_model', 'passed', 2, 2, 0, 2, 0, 12,
                    ?4, 'gold_task_pack', 'phase5-gold-task-pack', ?5
                 )",
                params![
                    "cepr_dashboard",
                    session.id,
                    project_id,
                    json!({
                        "packId": "phase5-gold-task-pack",
                        "sourceDoc": "docs/roadmap/coding-eval.md",
                        "packRunId": "cepr_dashboard",
                        "selectedCases": 2,
                        "automatedCases": 2,
                        "skippedCases": 0,
                        "passedCases": 2,
                        "failedCases": 0,
                        "totalChecks": 12,
                        "passed": true,
                        "cases": []
                    })
                    .to_string(),
                    now
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO coding_strategy_effect_runs (
                    id, session_id, project_id, strategy_type, baseline_label,
                    candidate_label, baseline_pack_run_id, candidate_pack_run_id,
                    verdict, compared_cases, pass_rate_delta, average_score_delta,
                    context_recall_delta, validation_violation_delta, scope_creep_delta,
                    execution_failure_delta, report_json, source_type, source_id, created_at
                 ) VALUES (
                    ?1, ?2, ?3, 'workflow_policy', 'before', 'after',
                    NULL, 'cepr_dashboard', 'regressed', 2, -0.5, -0.25,
                    0.0, 1, 2, 0, '{}', 'strategy_effect', 'workflow_policy', ?4
                 )",
                params!["cser_dashboard", session.id, project_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO review_runs (
                    id, session_id, scope, state, summary, stats_json, created_at, updated_at
                 ) VALUES ('rr_dashboard', ?1, 'diff', 'completed', '', '{}', ?2, ?2)",
                params![session.id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO review_findings (
                    id, run_id, session_id, file_path, title, body, category,
                    severity, verdict, status, evidence_json, created_at, updated_at
                 ) VALUES (
                    'rf_dashboard', 'rr_dashboard', ?1, 'src/lib.rs', 'Blocker',
                    'body', 'correctness', 'p1', 'confirmed', 'open', '{}', ?2, ?2
                 )",
                params![session.id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO verification_runs (
                    id, session_id, scope, state, summary, stats_json, created_at, updated_at
                 ) VALUES ('vr_dashboard', ?1, 'diff', 'failed', '', '{}', ?2, ?2)",
                params![session.id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO verification_steps (
                    id, run_id, session_id, seq, command, cwd, title, reason,
                    category, risk, auto_run, state, created_at, updated_at
                 ) VALUES (
                    'vs_dashboard', 'vr_dashboard', ?1, 0, 'cargo check', '/',
                    'Cargo check', 'compile', 'typecheck', 'low', 1, 'failed', ?2, ?2
                 )",
                params![session.id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO coding_workflow_retros (
                    id, session_id, project_id, workflow_run_id, run_state, summary,
                    signals_json, recommendations_json, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, 'completed', 'retro summary', '[]', ?5, ?6, ?6)",
                params![
                    "cwr_dashboard",
                    session.id,
                    project_id,
                    "wfr_dashboard",
                    json!([{
                        "kind": "workflow_template",
                        "title": "Distil workflow",
                        "rationale": "successful run"
                    }])
                    .to_string(),
                    now
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO coding_improvement_proposals (
                    id, session_id, project_id, kind, status, source_type,
                    source_id, title, body, payload_json, fingerprint, created_at, updated_at
                 ) VALUES (
                    'cip_dashboard', ?1, ?2, 'eval_candidate', 'draft', 'failure',
                    'validation_failed', 'Add eval', 'body',
                    '{\"failure\":{\"category\":\"validation_failed\"}}',
                    'fp_dashboard', ?3, ?3
                 )",
                params![session.id, project_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO domain_quality_runs (
                    id, session_id, goal_id, domain, template_id, template_version, state,
                    summary, stats_json, created_at, updated_at, completed_at
                 ) VALUES (
                    'dqr_research_dashboard', ?1, NULL, 'research', 'research-brief', '1.0.0',
                    'completed', 'Research quality passed', '{}', ?2, ?2, ?2
                 )",
                params![session.id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO domain_quality_runs (
                    id, session_id, goal_id, domain, template_id, template_version, state,
                    summary, stats_json, created_at, updated_at, completed_at
                 ) VALUES (
                    'dqr_inbox_dashboard', ?1, NULL, 'inbox', 'inbox-response', '1.0.0',
                    'needs_user', 'Inbox send needs approval', '{}', ?2, ?2, ?2
                 )",
                params![session.id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO domain_quality_checks (
                    id, run_id, session_id, seq, check_type, profile, title, body,
                    severity, status, evidence_type, source_metadata_json, created_at, updated_at
                 ) VALUES (
                    'dqc_research_pass', 'dqr_research_dashboard', ?1, 0, 'verification',
                    'research', 'Sources cited', 'Enough sources were cited', 'p2',
                    'passed', 'source_cited', '{}', ?2, ?2
                 )",
                params![session.id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO domain_quality_checks (
                    id, run_id, session_id, seq, check_type, profile, title, body,
                    severity, status, evidence_type, source_metadata_json, created_at, updated_at
                 ) VALUES (
                    'dqc_inbox_approval', 'dqr_inbox_dashboard', ?1, 0, 'approval',
                    'approval_gate', 'Message approved before send', 'User approval missing',
                    'p1', 'needs_user', 'user_decision', '{}', ?2, ?2
                 )",
                params![session.id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO domain_quality_checks (
                    id, run_id, session_id, seq, check_type, profile, title, body,
                    severity, status, evidence_type, source_metadata_json, created_at, updated_at
                 ) VALUES (
                    'dqc_inbox_missing', 'dqr_inbox_dashboard', ?1, 1, 'verification',
                    'required_evidence', 'Recipient facts checked', 'Required evidence missing',
                    'p1', 'failed', 'recipient_checked', '{}', ?2, ?2
                 )",
                params![session.id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO domain_eval_runs (
                    id, session_id, project_id, task_id, task_version, domain, label,
                    status, score, report_json, source_quality_run_id, created_at
                 ) VALUES (
                    'der_research_dashboard', ?1, ?2, 'research-source-coverage',
                    '1.0.0', 'research', 'Research eval', 'passed', 0.9, '{}',
                    'dqr_research_dashboard', ?3
                 )",
                params![session.id, project_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO domain_eval_runs (
                    id, session_id, project_id, task_id, task_version, domain, label,
                    status, score, report_json, source_quality_run_id, created_at
                 ) VALUES (
                    'der_inbox_dashboard', ?1, ?2, 'inbox-approval-safety',
                    '1.0.0', 'inbox', 'Inbox eval', 'failed', 0.4, '{}',
                    'dqr_inbox_dashboard', ?3
                 )",
                params![session.id, project_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO coding_improvement_proposals (
                    id, session_id, project_id, kind, status, source_type,
                    source_id, title, body, payload_json, fingerprint, created_at, updated_at
                 ) VALUES (
                    'cip_domain_dashboard', ?1, ?2, 'domain_review_profile', 'draft',
                    'domain_quality', 'dqr_inbox_dashboard', 'Add inbox review profile',
                    'body', '{\"domain\":\"inbox\"}', 'fp_domain_dashboard', ?3, ?3
                 )",
                params![session.id, project_id, now],
            )
            .unwrap();
        }

        let dashboard =
            query_coding_improvement_dashboard(&db, &DashboardFilter::default(), 8).unwrap();

        assert_eq!(dashboard.overview.total_sessions, 1);
        assert_eq!(dashboard.overview.completed_workflows, 1);
        assert_eq!(dashboard.overview.passed_eval_runs, 1);
        assert_eq!(dashboard.overview.failed_eval_runs, 1);
        assert_eq!(dashboard.overview.eval_pack_runs, 1);
        assert_eq!(dashboard.overview.passed_eval_pack_runs, 1);
        assert_eq!(dashboard.overview.external_pack_runs, 1);
        assert_eq!(dashboard.overview.strategy_effect_runs, 1);
        assert_eq!(dashboard.overview.regressed_strategy_effects, 1);
        assert_eq!(dashboard.overview.missing_tool_call_runs, 1);
        assert_eq!(dashboard.overview.validation_violation_delta, 1);
        assert_eq!(dashboard.overview.scope_creep_delta, 2);
        assert_eq!(dashboard.overview.open_review_blockers, 1);
        assert_eq!(dashboard.overview.failed_verification_steps, 1);
        assert_eq!(dashboard.overview.retro_recommendations, 1);
        assert_eq!(dashboard.overview.draft_proposals, 2);
        assert_eq!(dashboard.overview.distillation_candidates, 3);
        assert_eq!(dashboard.by_project.len(), 1);
        assert_eq!(
            dashboard.by_project[0].project_id.as_deref(),
            Some(project_id)
        );
        assert_eq!(
            dashboard.by_project[0].project_name.as_deref(),
            Some("Dashboard Project")
        );
        assert_eq!(dashboard.by_project[0].distillation_candidates, 3);
        assert_eq!(dashboard.by_project[0].eval_pack_runs, 1);
        assert_eq!(dashboard.by_project[0].strategy_effect_runs, 1);
        assert_eq!(dashboard.by_project[0].regressed_strategy_effects, 1);
        assert_eq!(dashboard.top_failures[0].category, "validation_failed");
        assert_eq!(
            dashboard.tool_call_failures[0].category,
            "missing_tool_call"
        );
        assert_eq!(dashboard.domain_quality.overview.quality_runs, 2);
        assert_eq!(dashboard.domain_quality.overview.completed_quality_runs, 1);
        assert_eq!(dashboard.domain_quality.overview.needs_user_quality_runs, 1);
        assert_eq!(dashboard.domain_quality.overview.approval_blockers, 1);
        assert_eq!(dashboard.domain_quality.overview.eval_runs, 2);
        assert_eq!(dashboard.domain_quality.overview.passed_eval_runs, 1);
        assert_eq!(dashboard.domain_quality.overview.failed_eval_runs, 1);
        assert_eq!(dashboard.domain_quality.overview.eval_pass_rate, Some(0.5));
        assert_eq!(dashboard.domain_quality.overview.domains_covered, 2);
        assert_eq!(dashboard.domain_quality.overview.draft_domain_proposals, 1);
        let inbox = dashboard
            .domain_quality
            .by_domain
            .iter()
            .find(|bucket| bucket.domain == "inbox")
            .expect("inbox domain bucket");
        assert_eq!(inbox.quality_runs, 1);
        assert_eq!(inbox.needs_user_quality_runs, 1);
        assert_eq!(inbox.approval_blockers, 1);
        assert_eq!(inbox.eval_runs, 1);
        assert_eq!(inbox.draft_proposals, 1);
        assert_eq!(
            dashboard.domain_quality.top_blockers[0].category,
            "approval:needs_user"
        );
        assert_eq!(dashboard.domain_quality.recent_runs.len(), 2);
        assert_eq!(dashboard.domain_quality.timeline.len(), 1);
        assert_eq!(dashboard.domain_quality.timeline[0].quality_runs, 2);
        assert_eq!(dashboard.domain_quality.timeline[0].approval_blockers, 1);
        assert_eq!(dashboard.domain_quality.timeline[0].eval_passed, 1);
        assert_eq!(dashboard.domain_quality.timeline[0].eval_failed, 1);
        assert_eq!(dashboard.domain_quality.timeline[0].proposals_created, 1);
        assert_eq!(dashboard.latest_strategy_effects[0].verdict, "regressed");
        assert_eq!(dashboard.latest_retros[0].recommendations.len(), 1);
        assert_eq!(dashboard.timeline.len(), 1);
    }

    #[test]
    fn dashboard_excludes_incognito_sessions() {
        let (_dir, db) = test_db();
        let now = now_rfc3339();
        let regular = db.create_session(DEFAULT_AGENT_ID).unwrap();
        let incognito = db
            .create_session_with_project(DEFAULT_AGENT_ID, None, Some(true))
            .unwrap();

        {
            let conn = db.conn.lock().unwrap();
            for (id, session_id) in [
                ("wfr_regular", regular.id.as_str()),
                ("wfr_incognito", incognito.id.as_str()),
            ] {
                conn.execute(
                    "INSERT INTO workflow_runs (
                        id, session_id, kind, state, execution_mode, script_hash,
                        script_source, created_at, updated_at, completed_at
                     ) VALUES (?1, ?2, 'coding.workflow', 'completed', 'guarded',
                        'hash', 'script', ?3, ?3, ?3)",
                    params![id, session_id, now],
                )
                .unwrap();
            }
        }

        let dashboard =
            query_coding_improvement_dashboard(&db, &DashboardFilter::default(), 8).unwrap();
        assert_eq!(dashboard.overview.total_sessions, 1);
        assert_eq!(dashboard.overview.workflow_runs, 1);
        assert_eq!(dashboard.overview.completed_workflows, 1);
    }
}
