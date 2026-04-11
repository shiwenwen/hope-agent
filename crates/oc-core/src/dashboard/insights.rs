// ── Insights Queries (Phase 2) ──────────────────────────────────
//
// Richer analytics: period-over-period comparison, cost trend,
// activity heatmap, hourly distribution, top sessions, model
// efficiency and aggregate health score.

use anyhow::Result;
use std::sync::Arc;

use crate::cron::CronDB;
use crate::logging::LogDB;
use crate::session::SessionDB;

use super::cost::estimate_cost;
use super::filters::{build_log_filter, build_session_filter, params_ref};
use super::queries::{query_overview, query_tool_usage};
use super::types::*;

/// Shift the filter window backward by the same span to obtain the
/// previous-period baseline. Returns `None` when start/end are unset.
fn shift_filter_backward(filter: &DashboardFilter) -> Option<DashboardFilter> {
    let start = filter.start_date.as_deref().filter(|s| !s.is_empty())?;
    let end = filter.end_date.as_deref().filter(|s| !s.is_empty())?;

    let start_dt = chrono::DateTime::parse_from_rfc3339(start).ok()?;
    let end_dt = chrono::DateTime::parse_from_rfc3339(end).ok()?;
    let span = end_dt - start_dt;
    if span.num_seconds() <= 0 {
        return None;
    }

    let prev_start = start_dt - span;
    let prev_end = start_dt;

    Some(DashboardFilter {
        start_date: Some(prev_start.to_rfc3339()),
        end_date: Some(prev_end.to_rfc3339()),
        agent_id: filter.agent_id.clone(),
        provider_id: filter.provider_id.clone(),
        model_id: filter.model_id.clone(),
    })
}

/// Overview stats with previous-period baseline for delta display.
pub fn query_overview_with_delta(
    session_db: &Arc<SessionDB>,
    log_db: &Arc<LogDB>,
    cron_db: &Arc<CronDB>,
    filter: &DashboardFilter,
) -> Result<OverviewStatsWithDelta> {
    let current = query_overview(session_db, log_db, cron_db, filter)?;
    let previous = if let Some(prev_filter) = shift_filter_backward(filter) {
        query_overview(session_db, log_db, cron_db, &prev_filter).ok()
    } else {
        None
    };
    Ok(OverviewStatsWithDelta { current, previous })
}

/// Daily cost trend derived from messages + per-model pricing.
pub fn query_cost_trend(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
) -> Result<DashboardCostTrend> {
    let conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let f = build_session_filter(filter, "s", Some("m"));
    let sql = format!(
        "SELECT DATE(m.timestamp) as d,
                COALESCE(s.model_id, 'unknown') as model,
                COALESCE(SUM(m.tokens_in), 0),
                COALESCE(SUM(m.tokens_out), 0)
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         {}
         GROUP BY d, model
         ORDER BY d ASC",
        f.where_sql
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            crate::sql_u64(r, 2)?,
            crate::sql_u64(r, 3)?,
        ))
    })?;

    // Aggregate daily costs by summing per-model cost within each day
    let mut points: Vec<CostTrendPoint> = Vec::new();
    for row in rows {
        let (date, model, tokens_in, tokens_out) = row?;
        let cost = estimate_cost(&model, tokens_in, tokens_out);
        if let Some(last) = points.last_mut() {
            if last.date == date {
                last.cost_usd += cost;
                last.input_tokens += tokens_in;
                last.output_tokens += tokens_out;
                continue;
            }
        }
        points.push(CostTrendPoint {
            date,
            cost_usd: cost,
            input_tokens: tokens_in,
            output_tokens: tokens_out,
        });
    }

    let total_cost_usd: f64 = points.iter().map(|p| p.cost_usd).sum();
    let (peak_day, peak_cost_usd) = points
        .iter()
        .max_by(|a, b| {
            a.cost_usd
                .partial_cmp(&b.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|p| (Some(p.date.clone()), p.cost_usd))
        .unwrap_or((None, 0.0));
    let avg_daily_cost_usd = if points.is_empty() {
        0.0
    } else {
        total_cost_usd / points.len() as f64
    };

    Ok(DashboardCostTrend {
        points,
        total_cost_usd,
        peak_day,
        peak_cost_usd,
        avg_daily_cost_usd,
    })
}

/// 7×24 activity heatmap: message counts per (weekday, hour-of-day).
pub fn query_activity_heatmap(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
) -> Result<DashboardHeatmap> {
    let conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let f = build_session_filter(filter, "s", Some("m"));
    let sql = format!(
        "SELECT CAST(strftime('%w', m.timestamp) AS INTEGER) as wd,
                CAST(strftime('%H', m.timestamp) AS INTEGER) as h,
                COUNT(*) as cnt
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         {}
         GROUP BY wd, h
         ORDER BY wd, h",
        f.where_sql
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        Ok(HeatmapCell {
            weekday: r.get::<_, i64>(0)? as u8,
            hour: r.get::<_, i64>(1)? as u8,
            message_count: crate::sql_u64(r, 2)?,
        })
    })?;
    let cells: Vec<HeatmapCell> = rows.collect::<std::result::Result<_, _>>()?;
    let max_value = cells.iter().map(|c| c.message_count).max().unwrap_or(0);
    let total: u64 = cells.iter().map(|c| c.message_count).sum();

    Ok(DashboardHeatmap {
        cells,
        max_value,
        total,
    })
}

/// Hourly distribution (0..23) of messages and distinct sessions.
pub fn query_hourly_distribution(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
) -> Result<DashboardHourlyDistribution> {
    let conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let f = build_session_filter(filter, "s", Some("m"));
    let sql = format!(
        "SELECT CAST(strftime('%H', m.timestamp) AS INTEGER) as h,
                COUNT(*) as msg_cnt,
                COUNT(DISTINCT s.id) as sess_cnt
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         {}
         GROUP BY h
         ORDER BY h",
        f.where_sql
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        Ok((
            r.get::<_, i64>(0)? as u8,
            crate::sql_u64(r, 1)?,
            crate::sql_u64(r, 2)?,
        ))
    })?;

    // Fill missing hours with zeros
    let mut map: std::collections::HashMap<u8, (u64, u64)> = std::collections::HashMap::new();
    for row in rows {
        let (h, m, s) = row?;
        map.insert(h, (m, s));
    }
    let mut buckets: Vec<HourlyBucket> = (0u8..24)
        .map(|h| {
            let (m, s) = map.get(&h).copied().unwrap_or((0, 0));
            HourlyBucket {
                hour: h,
                message_count: m,
                session_count: s,
            }
        })
        .collect();
    buckets.sort_by_key(|b| b.hour);

    let (peak_hour, peak_message_count) = buckets
        .iter()
        .max_by_key(|b| b.message_count)
        .map(|b| (Some(b.hour), b.message_count))
        .unwrap_or((None, 0));

    Ok(DashboardHourlyDistribution {
        buckets,
        peak_hour,
        peak_message_count,
    })
}

/// Top sessions ranked by total token consumption.
pub fn query_top_sessions(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
    limit: usize,
) -> Result<Vec<TopSession>> {
    let conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let mut f = build_session_filter(filter, "s", None);
    // Append the limit as a bound parameter so we avoid string interpolation.
    let limit_box: Box<dyn rusqlite::types::ToSql> =
        Box::new(limit.max(1).min(1000) as i64);
    f.params.push(limit_box);
    let sql = format!(
        "SELECT s.id,
                s.title,
                s.agent_id,
                s.model_id,
                COUNT(m.id) as msg_cnt,
                COALESCE(SUM(m.tokens_in), 0) + COALESCE(SUM(m.tokens_out), 0) as total_tokens,
                COALESCE(SUM(m.tokens_in), 0),
                COALESCE(SUM(m.tokens_out), 0),
                s.updated_at
         FROM sessions s
         LEFT JOIN messages m ON m.session_id = s.id
         {}
         GROUP BY s.id
         ORDER BY total_tokens DESC
         LIMIT ?",
        f.where_sql
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        let id: String = r.get(0)?;
        let title: Option<String> = r.get(1)?;
        let agent_id: String = r.get(2)?;
        let model_id: Option<String> = r.get(3)?;
        let message_count: u64 = crate::sql_u64(r, 4)?;
        let total_tokens: u64 = crate::sql_u64(r, 5)?;
        let tokens_in: u64 = crate::sql_u64(r, 6)?;
        let tokens_out: u64 = crate::sql_u64(r, 7)?;
        let updated_at: String = r.get(8)?;
        let model_ref = model_id.as_deref().unwrap_or("unknown");
        let estimated_cost_usd = estimate_cost(model_ref, tokens_in, tokens_out);
        Ok(TopSession {
            id,
            title,
            agent_id,
            model_id,
            total_tokens,
            message_count,
            estimated_cost_usd,
            updated_at,
        })
    })?;

    let list: Vec<TopSession> = rows.collect::<std::result::Result<_, _>>()?;
    Ok(list)
}

/// Per-model efficiency: tokens, cost, speed, and derived ratios.
pub fn query_model_efficiency(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
) -> Result<Vec<ModelEfficiency>> {
    let conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let f = build_session_filter(filter, "s", Some("m"));
    let sql = format!(
        "SELECT COALESCE(s.model_id, 'unknown'),
                COALESCE(s.provider_name, 'unknown'),
                COUNT(m.id) as msg_cnt,
                COALESCE(SUM(m.tokens_in), 0),
                COALESCE(SUM(m.tokens_out), 0),
                AVG(CASE WHEN m.ttft_ms IS NOT NULL AND m.role = 'assistant' THEN m.ttft_ms END)
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         {}
         GROUP BY s.model_id, s.provider_name
         ORDER BY SUM(m.tokens_in) + SUM(m.tokens_out) DESC",
        f.where_sql
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        let model_id: String = r.get(0)?;
        let provider_name: String = r.get(1)?;
        let message_count: u64 = crate::sql_u64(r, 2)?;
        let input_tokens: u64 = crate::sql_u64(r, 3)?;
        let output_tokens: u64 = crate::sql_u64(r, 4)?;
        let avg_ttft_ms: Option<f64> = r.get(5)?;
        let total_tokens = input_tokens + output_tokens;
        let total_cost_usd = estimate_cost(&model_id, input_tokens, output_tokens);
        let avg_tokens_per_message = if message_count > 0 {
            total_tokens as f64 / message_count as f64
        } else {
            0.0
        };
        let avg_cost_per_1k_tokens = if total_tokens > 0 {
            (total_cost_usd / total_tokens as f64) * 1000.0
        } else {
            0.0
        };
        Ok(ModelEfficiency {
            model_id,
            provider_name,
            total_tokens,
            total_cost_usd,
            avg_ttft_ms,
            message_count,
            avg_tokens_per_message,
            avg_cost_per_1k_tokens,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Compute an aggregate health score 0..=100.
/// Weighting:
/// - log error rate (errors/(errors+warn)) — 25%
/// - tool error rate — 25%
/// - cron success rate — 25%
/// - subagent success rate — 25%
pub fn query_health_score(
    session_db: &Arc<SessionDB>,
    log_db: &Arc<LogDB>,
    cron_db: &Arc<CronDB>,
    filter: &DashboardFilter,
) -> Result<HealthBreakdown> {
    // Log error rate (lower is better)
    let log_conn = log_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
    let base = build_log_filter(filter);
    let where_sql = if base.where_sql.is_empty() {
        "WHERE level IN ('error','warn','info')".to_string()
    } else {
        format!("{} AND level IN ('error','warn','info')", base.where_sql)
    };
    let sql = format!(
        "SELECT COALESCE(SUM(CASE WHEN level = 'error' THEN 1 ELSE 0 END), 0),
                COALESCE(COUNT(*), 0)
         FROM logs {}",
        where_sql
    );
    let (log_errors, log_total): (u64, u64) =
        log_conn.query_row(&sql, params_ref(&base.params).as_slice(), |r| {
            Ok((crate::sql_u64(r, 0)?, crate::sql_u64(r, 1)?))
        })?;
    drop(log_conn);

    let log_error_rate_percent = if log_total > 0 {
        (log_errors as f64 / log_total as f64) * 100.0
    } else {
        0.0
    };

    // Tool error rate
    let tools = query_tool_usage(session_db, filter)?;
    let total_tool_calls: u64 = tools.iter().map(|t| t.call_count).sum();
    let total_tool_errors: u64 = tools.iter().map(|t| t.error_count).sum();
    let tool_error_rate_percent = if total_tool_calls > 0 {
        (total_tool_errors as f64 / total_tool_calls as f64) * 100.0
    } else {
        0.0
    };

    // Cron success rate
    let cron_conn = cron_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
    let mut clauses: Vec<String> = Vec::new();
    let mut cron_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(ref s) = filter.start_date {
        if !s.is_empty() {
            clauses.push("started_at >= ?".to_string());
            cron_params.push(Box::new(s.clone()));
        }
    }
    if let Some(ref e) = filter.end_date {
        if !e.is_empty() {
            clauses.push("started_at <= ?".to_string());
            cron_params.push(Box::new(e.clone()));
        }
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    let sql = format!(
        "SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END), 0)
         FROM cron_run_logs {}",
        where_sql
    );
    let (cron_total, cron_success): (u64, u64) = cron_conn
        .query_row(&sql, params_ref(&cron_params).as_slice(), |r| {
            Ok((crate::sql_u64(r, 0)?, crate::sql_u64(r, 1)?))
        })?;
    drop(cron_conn);
    let cron_success_rate_percent = if cron_total > 0 {
        (cron_success as f64 / cron_total as f64) * 100.0
    } else {
        100.0
    };

    // Subagent success rate
    let sess_conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
    let mut clauses: Vec<String> = Vec::new();
    let mut sub_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(ref s) = filter.start_date {
        if !s.is_empty() {
            clauses.push("started_at >= ?".to_string());
            sub_params.push(Box::new(s.clone()));
        }
    }
    if let Some(ref e) = filter.end_date {
        if !e.is_empty() {
            clauses.push("started_at <= ?".to_string());
            sub_params.push(Box::new(e.clone()));
        }
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    let sql = format!(
        "SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END), 0)
         FROM subagent_runs {}",
        where_sql
    );
    let (sub_total, sub_completed): (u64, u64) = sess_conn
        .query_row(&sql, params_ref(&sub_params).as_slice(), |r| {
            Ok((crate::sql_u64(r, 0)?, crate::sql_u64(r, 1)?))
        })?;
    drop(sess_conn);
    let subagent_success_rate_percent = if sub_total > 0 {
        (sub_completed as f64 / sub_total as f64) * 100.0
    } else {
        100.0
    };

    // Composite score: each dimension contributes 25 points.
    let log_score = 25.0 * (1.0 - (log_error_rate_percent / 100.0).min(1.0));
    let tool_score = 25.0 * (1.0 - (tool_error_rate_percent / 100.0).min(1.0));
    let cron_score = 25.0 * (cron_success_rate_percent / 100.0).clamp(0.0, 1.0);
    let sub_score = 25.0 * (subagent_success_rate_percent / 100.0).clamp(0.0, 1.0);
    let total = log_score + tool_score + cron_score + sub_score;
    let score = total.round().clamp(0.0, 100.0) as u8;

    let status = match score {
        90..=100 => "excellent",
        75..=89 => "good",
        50..=74 => "warning",
        _ => "critical",
    }
    .to_string();

    Ok(HealthBreakdown {
        score,
        log_error_rate_percent,
        tool_error_rate_percent,
        cron_success_rate_percent,
        subagent_success_rate_percent,
        status,
    })
}

/// One-shot aggregated insights query powering the Insights tab.
pub fn query_insights(
    session_db: &Arc<SessionDB>,
    log_db: &Arc<LogDB>,
    cron_db: &Arc<CronDB>,
    filter: &DashboardFilter,
) -> Result<DashboardInsights> {
    let health = query_health_score(session_db, log_db, cron_db, filter)?;
    let cost_trend = query_cost_trend(session_db, filter)?;
    let heatmap = query_activity_heatmap(session_db, filter)?;
    let hourly = query_hourly_distribution(session_db, filter)?;
    let top_sessions = query_top_sessions(session_db, filter, 10)?;
    let model_efficiency = query_model_efficiency(session_db, filter)?;
    Ok(DashboardInsights {
        health,
        cost_trend,
        heatmap,
        hourly,
        top_sessions,
        model_efficiency,
    })
}
