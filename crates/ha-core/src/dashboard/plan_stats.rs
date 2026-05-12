// ── Plan Statistics Queries ─────────────────────────────────────
//
// Aggregates the plan index (see `crate::plan::list_all_plans`) into the
// dashboard widgets: state distribution, completion rate, per-agent and
// per-project breakdowns, 30-day creation trend, and average execution
// duration. Pure in-memory aggregation — no extra SQL — since plan totals
// stay well under 10⁴ in practice.

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

use crate::plan::{list_all_plans, PlanIndexFilter, PlanModeState};

use super::types::*;

/// Parse an RFC3339 timestamp (with or without timezone offset) into UTC.
/// Returns `None` on malformed input — callers treat that as "missing".
fn parse_rfc3339_utc(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

const TREND_DAYS: i64 = 30;
const MAX_AGENT_BUCKETS: usize = 10;
const MAX_PROJECT_BUCKETS: usize = 10;
/// Drop executing→completed deltas above this — usually means the session
/// was left running while the user wandered off; including them would
/// distort the mean.
const MAX_EXECUTION_DURATION_SECS: i64 = 7 * 24 * 3600;

pub fn query_plan_stats(filter: &DashboardFilter) -> Result<PlanStats> {
    let plan_filter = PlanIndexFilter {
        agent_id: filter.agent_id.clone().filter(|s| !s.is_empty()),
        ..Default::default()
    };
    let mut plans = list_all_plans(&plan_filter)?;

    // Filter timestamps are RFC3339 with arbitrary offset (the frontend ships
    // UTC via `toISOString()`); `plan.updated_at` is RFC3339 in local time.
    // String compare across mixed offsets misranks any non-UTC machine, so
    // parse both sides to UTC before comparing. Plans with unparseable
    // timestamps are kept (defensive — never silently drop data).
    let start_bound = filter
        .start_date
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(parse_rfc3339_utc);
    let end_bound = filter
        .end_date
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(parse_rfc3339_utc);
    if start_bound.is_some() || end_bound.is_some() {
        plans.retain(|p| {
            let Some(ts) = parse_rfc3339_utc(&p.updated_at) else {
                return true;
            };
            if let Some(start) = start_bound {
                if ts < start {
                    return false;
                }
            }
            if let Some(end) = end_bound {
                if ts > end {
                    return false;
                }
            }
            true
        });
    }

    let total = plans.len() as u64;

    let mut distribution = PlanStateDistribution::default();
    let mut completed_total: u64 = 0;
    let mut agent_buckets: HashMap<String, (u64, u64)> = HashMap::new();
    let mut project_buckets: HashMap<Option<String>, (u64, u64)> = HashMap::new();

    let now = chrono::Utc::now();
    let trend_floor = (now - chrono::Duration::days(TREND_DAYS - 1))
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("static midnight");
    let mut trend: HashMap<String, u64> = HashMap::new();

    let mut duration_sum_secs: i64 = 0;
    let mut duration_count: i64 = 0;

    for plan in &plans {
        match plan.state {
            PlanModeState::Off => distribution.off += 1,
            PlanModeState::Planning => distribution.planning += 1,
            PlanModeState::Review => distribution.review += 1,
            PlanModeState::Executing => distribution.executing += 1,
            PlanModeState::Completed => {
                distribution.completed += 1;
                completed_total += 1;
            }
        }

        let agent_slot = agent_buckets.entry(plan.agent_id.clone()).or_insert((0, 0));
        agent_slot.0 += 1;
        if plan.state == PlanModeState::Completed {
            agent_slot.1 += 1;
        }

        let project_slot = project_buckets
            .entry(plan.project_id.clone())
            .or_insert((0, 0));
        project_slot.0 += 1;
        if plan.state == PlanModeState::Completed {
            project_slot.1 += 1;
        }

        if let Ok(created_dt) = chrono::DateTime::parse_from_rfc3339(&plan.created_at) {
            let local = created_dt.naive_local();
            if local >= trend_floor {
                let day = local.date().format("%Y-%m-%d").to_string();
                *trend.entry(day).or_insert(0) += 1;
            }
        }

        // Execution duration: `plan.updated_at` is the plan markdown file
        // mtime, but a completed plan typically stops touching the file as
        // soon as the model approves it — so file mtime underestimates (or
        // even predates) the real completion time. Use `session_updated_at`
        // instead, which advances whenever `sessions.plan_mode` flips to
        // `completed` via `update_session_plan_mode`. Skip samples where
        // either bound is unparseable.
        if plan.state == PlanModeState::Completed {
            if let (Some(start_dt), Some(end_dt)) = (
                plan.executing_started_at
                    .as_deref()
                    .and_then(parse_rfc3339_utc),
                plan.session_updated_at
                    .as_deref()
                    .and_then(parse_rfc3339_utc),
            ) {
                let dur = (end_dt - start_dt).num_seconds();
                if dur > 0 && dur < MAX_EXECUTION_DURATION_SECS {
                    duration_sum_secs += dur;
                    duration_count += 1;
                }
            }
        }
    }

    let completion_rate = if total == 0 {
        0.0
    } else {
        completed_total as f64 / total as f64
    };

    let mut by_agent: Vec<PlanAgentBucket> = agent_buckets
        .into_iter()
        .map(|(agent_id, (total, completed))| PlanAgentBucket {
            agent_id,
            total,
            completed,
        })
        .collect();
    by_agent.sort_by(|a, b| {
        b.total
            .cmp(&a.total)
            .then_with(|| a.agent_id.cmp(&b.agent_id))
    });
    by_agent.truncate(MAX_AGENT_BUCKETS);

    let mut by_project: Vec<PlanProjectBucket> = project_buckets
        .into_iter()
        .map(|(project_id, (total, completed))| PlanProjectBucket {
            project_id,
            total,
            completed,
        })
        .collect();
    by_project.sort_by(|a, b| {
        b.total
            .cmp(&a.total)
            .then_with(|| a.project_id.cmp(&b.project_id))
    });
    by_project.truncate(MAX_PROJECT_BUCKETS);

    let mut creation_trend: Vec<PlanTrendPoint> = Vec::with_capacity(TREND_DAYS as usize);
    for offset in (0..TREND_DAYS).rev() {
        let day = (now - chrono::Duration::days(offset))
            .date_naive()
            .format("%Y-%m-%d")
            .to_string();
        let created = trend.get(&day).copied().unwrap_or(0);
        creation_trend.push(PlanTrendPoint { date: day, created });
    }

    let avg_execution_duration_secs = if duration_count > 0 {
        Some(duration_sum_secs as f64 / duration_count as f64)
    } else {
        None
    };

    Ok(PlanStats {
        total,
        state_distribution: distribution,
        completion_rate,
        by_agent,
        by_project,
        creation_trend,
        avg_execution_duration_secs,
        sampled_duration_count: duration_count as u64,
    })
}
