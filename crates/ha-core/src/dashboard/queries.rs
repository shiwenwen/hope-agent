// ── Query Functions ─────────────────────────────────────────────

use anyhow::Result;
use std::sync::Arc;
use sysinfo::{ProcessesToUpdate, System};

use crate::cron::CronDB;
use crate::logging::LogDB;
use crate::session::SessionDB;

use super::cost::resolve_cost;
use super::filters::{
    build_log_filter, build_model_usage_filter, build_session_filter, params_ref,
};
use super::types::*;

/// Overview stats: session/message/token counts, tool calls, errors, active agents/cron.
pub fn query_overview(
    session_db: &Arc<SessionDB>,
    _log_db: &Arc<LogDB>,
    cron_db: &Arc<CronDB>,
    filter: &DashboardFilter,
) -> Result<OverviewStats> {
    let sess_conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    // Session count
    let f = build_session_filter(filter, "s", None);
    let sql = format!("SELECT COUNT(*) FROM sessions s {}", f.where_sql);
    let total_sessions: u64 = sess_conn.query_row(&sql, params_ref(&f.params).as_slice(), |r| {
        crate::sql_u64(r, 0)
    })?;

    // Message count + tool calls + errors. Token/cost totals come from the
    // unified model usage ledger below so side_query / embedding / STT calls
    // are included too.
    let f = build_session_filter(filter, "s", Some("m"));
    let sql = format!(
        "SELECT COUNT(m.id),
                COALESCE(SUM(CASE WHEN m.tool_name IS NOT NULL THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN m.is_error = 1 THEN 1 ELSE 0 END), 0)
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         {}",
        f.where_sql
    );
    let (total_messages, total_tool_calls, total_errors): (u64, u64, u64) =
        sess_conn.query_row(&sql, params_ref(&f.params).as_slice(), |r| {
            Ok((
                crate::sql_u64(r, 0)?,
                crate::sql_u64(r, 1)?,
                crate::sql_u64(r, 2)?,
            ))
        })?;

    let f_usage = build_model_usage_filter(filter, "u");
    let sql = format!(
        "SELECT COALESCE(SUM(u.input_tokens), 0),
                COALESCE(SUM(u.output_tokens), 0)
         FROM model_usage_events u
         {}",
        f_usage.where_sql
    );
    let (total_input_tokens, total_output_tokens): (u64, u64) =
        sess_conn.query_row(&sql, params_ref(&f_usage.params).as_slice(), |r| {
            Ok((crate::sql_u64(r, 0)?, crate::sql_u64(r, 1)?))
        })?;

    // Active agents (distinct agent_ids in sessions within filter period)
    let f = build_session_filter(filter, "s", None);
    let sql = format!(
        "SELECT COUNT(DISTINCT s.agent_id) FROM sessions s {}",
        f.where_sql
    );
    let active_agents: u64 = sess_conn.query_row(&sql, params_ref(&f.params).as_slice(), |r| {
        crate::sql_u64(r, 0)
    })?;

    // Query average TTFT from the usage ledger.
    let f = build_model_usage_filter(filter, "u");
    let sql = format!(
        "SELECT AVG(u.ttft_ms)
         FROM model_usage_events u
         {} AND u.ttft_ms IS NOT NULL",
        if f.where_sql.is_empty() {
            "WHERE 1=1".to_string()
        } else {
            f.where_sql
        }
    );
    let avg_ttft_ms: Option<f64> = sess_conn
        .query_row(&sql, params_ref(&f.params).as_slice(), |r| r.get(0))
        .ok();

    drop(sess_conn);

    // Active cron jobs
    let cron_conn = cron_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
    let active_cron_jobs: u64 = cron_conn.query_row(
        "SELECT COUNT(*) FROM cron_jobs WHERE status = 'active'",
        [],
        |r| crate::sql_u64(r, 0),
    )?;
    drop(cron_conn);

    // Estimate cost by querying per-model token usage
    let sess_conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
    let f = build_model_usage_filter(filter, "u");
    // 按 provider_id 一并分组，成本才能按各渠道自己的配置单价结算（见 cost::resolve_cost）。
    // 分组更细只是多几行，求和后总额不变。
    let sql = format!(
        "SELECT COALESCE(u.model_id, 'unknown'),
                u.provider_id,
                COALESCE(SUM(u.input_tokens), 0),
                COALESCE(SUM(u.output_tokens), 0)
         FROM model_usage_events u
         {}
         GROUP BY u.model_id, u.provider_id",
        f.where_sql
    );
    let mut stmt = sess_conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            crate::sql_u64(r, 2)?,
            crate::sql_u64(r, 3)?,
        ))
    })?;
    let mut estimated_cost_usd = 0.0;
    for row in rows {
        let (model, provider_id, inp, out) = row?;
        estimated_cost_usd += resolve_cost(provider_id.as_deref(), &model, inp, out);
    }

    Ok(OverviewStats {
        total_sessions,
        total_messages,
        total_input_tokens,
        total_output_tokens,
        total_tool_calls,
        total_errors,
        active_agents,
        active_cron_jobs,
        estimated_cost_usd,
        avg_ttft_ms,
    })
}

/// Coarse purpose "domain" derived from a fine-grained `operation` tag by
/// taking the substring before the first '.' (or the whole string for bare
/// tags like "session_title"/"recall_summary" that have no dot). Pure string
/// split — NOT a hardcoded lookup table — so newly added purpose tags (see
/// docs/architecture/automation-model.md §2.5) bucket correctly with zero
/// code changes here. Also naturally buckets the "kind-level generic"
/// operation tags (e.g. "agent.side_query" -> "agent", "permission_judge" ->
/// "permission_judge"), so by_operation/by_domain cover the whole ledger, not
/// just the automation-purpose subset.
fn operation_domain(operation: &str) -> &str {
    operation
        .split_once('.')
        .map(|(domain, _)| domain)
        .unwrap_or(operation)
}

/// Merges one weighted `(avg, count)` sample into a running weighted average.
/// `None` samples leave `current` unchanged (matching the historical inline
/// `if let Some(avg) = ... {}` behavior in the by_kind block below — a
/// missing sample must not reset an already-accumulated average to None).
fn merge_weighted_avg(
    current: Option<f64>,
    current_count: u64,
    sample: Option<f64>,
    sample_count: u64,
    new_count: u64,
) -> Option<f64> {
    match sample {
        Some(s) => {
            let total = current.unwrap_or(0.0) * current_count as f64 + s * sample_count as f64;
            Some(total / new_count.max(1) as f64)
        }
        None => current,
    }
}

/// Token usage: daily trend and breakdown by model.
pub fn query_token_usage(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
) -> Result<DashboardTokenData> {
    let conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    // Daily trend (with avg TTFT)
    let f = build_model_usage_filter(filter, "u");
    let sql = format!(
        "SELECT DATE(u.timestamp) as d,
                COALESCE(SUM(u.input_tokens), 0),
                COALESCE(SUM(u.output_tokens), 0),
                AVG(u.ttft_ms)
         FROM model_usage_events u
         {}
         GROUP BY d
         ORDER BY d ASC",
        f.where_sql
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        Ok(TokenUsageTrend {
            date: r.get(0)?,
            input_tokens: crate::sql_u64(r, 1)?,
            output_tokens: crate::sql_u64(r, 2)?,
            avg_ttft_ms: r.get(3)?,
        })
    })?;
    let trend: Vec<TokenUsageTrend> = rows.collect::<std::result::Result<_, _>>()?;

    // By model (with avg TTFT)
    let f = build_model_usage_filter(filter, "u");
    let sql = format!(
        "SELECT COALESCE(u.model_id, 'unknown'),
                COALESCE(u.provider_name, 'unknown'),
                COALESCE(SUM(u.input_tokens), 0),
                COALESCE(SUM(u.output_tokens), 0),
                AVG(u.ttft_ms),
                u.provider_id
         FROM model_usage_events u
         {}
         GROUP BY u.model_id, u.provider_name, u.provider_id
         ORDER BY SUM(u.input_tokens) + SUM(u.output_tokens) DESC",
        f.where_sql
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        let model_id: String = r.get(0)?;
        let provider_name: String = r.get(1)?;
        let input_tokens: u64 = crate::sql_u64(r, 2)?;
        let output_tokens: u64 = crate::sql_u64(r, 3)?;
        let avg_ttft_ms: Option<f64> = r.get(4)?;
        let provider_id: Option<String> = r.get(5)?;
        Ok(TokenByModel {
            estimated_cost_usd: resolve_cost(
                provider_id.as_deref(),
                &model_id,
                input_tokens,
                output_tokens,
            ),
            model_id,
            provider_name,
            provider_id,
            input_tokens,
            output_tokens,
            avg_ttft_ms,
        })
    })?;
    let by_model: Vec<TokenByModel> = rows.collect::<std::result::Result<_, _>>()?;

    let f = build_model_usage_filter(filter, "u");
    let sql = format!(
        "SELECT u.kind,
                COALESCE(u.model_id, 'unknown'),
                COUNT(*) as call_count,
                COALESCE(SUM(u.input_tokens), 0),
                COALESCE(SUM(u.output_tokens), 0),
                COALESCE(SUM(u.cache_creation_input_tokens), 0),
                COALESCE(SUM(u.cache_read_input_tokens), 0),
                COALESCE(SUM(COALESCE(u.context_input_tokens, u.input_tokens)), 0),
                COALESCE(SUM(COALESCE(u.fresh_input_tokens, u.input_tokens)), 0),
                AVG(u.duration_ms),
                AVG(u.ttft_ms),
                u.provider_id
         FROM model_usage_events u
         {}
         GROUP BY u.kind, u.model_id, u.provider_id
         ORDER BY u.kind ASC",
        f.where_sql
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            crate::sql_u64(r, 2)?,
            crate::sql_u64(r, 3)?,
            crate::sql_u64(r, 4)?,
            crate::sql_u64(r, 5)?,
            crate::sql_u64(r, 6)?,
            crate::sql_u64(r, 7)?,
            crate::sql_u64(r, 8)?,
            r.get::<_, Option<f64>>(9)?,
            r.get::<_, Option<f64>>(10)?,
            r.get::<_, Option<String>>(11)?,
        ))
    })?;
    let mut by_kind_map: std::collections::BTreeMap<String, TokenByKind> =
        std::collections::BTreeMap::new();
    for row in rows {
        let (
            kind,
            model_id,
            call_count,
            input_tokens,
            output_tokens,
            cache_creation_input_tokens,
            cache_read_input_tokens,
            context_input_tokens,
            fresh_input_tokens,
            avg_duration_ms,
            avg_ttft_ms,
            provider_id,
        ) = row?;
        let entry = by_kind_map.entry(kind.clone()).or_insert(TokenByKind {
            kind,
            call_count: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            context_input_tokens: 0,
            fresh_input_tokens: 0,
            estimated_cost_usd: 0.0,
            avg_duration_ms: None,
            avg_ttft_ms: None,
        });
        let old_calls = entry.call_count;
        entry.call_count += call_count;
        entry.input_tokens += input_tokens;
        entry.output_tokens += output_tokens;
        entry.cache_creation_input_tokens += cache_creation_input_tokens;
        entry.cache_read_input_tokens += cache_read_input_tokens;
        entry.context_input_tokens += context_input_tokens;
        entry.fresh_input_tokens += fresh_input_tokens;
        entry.estimated_cost_usd += resolve_cost(
            provider_id.as_deref(),
            &model_id,
            input_tokens,
            output_tokens,
        );
        entry.avg_duration_ms = merge_weighted_avg(
            entry.avg_duration_ms,
            old_calls,
            avg_duration_ms,
            call_count,
            entry.call_count,
        );
        entry.avg_ttft_ms = merge_weighted_avg(
            entry.avg_ttft_ms,
            old_calls,
            avg_ttft_ms,
            call_count,
            entry.call_count,
        );
    }
    let mut by_kind: Vec<TokenByKind> = by_kind_map.into_values().collect();
    by_kind.sort_by_key(|k| std::cmp::Reverse(k.input_tokens + k.output_tokens));

    // By operation (purpose tag) — same GROUP BY/merge shape as by_kind above,
    // grouped on `operation` instead of `kind`. `domain` is derived per-row
    // (operation_domain), not queried.
    let f = build_model_usage_filter(filter, "u");
    let sql = format!(
        "SELECT COALESCE(u.operation, 'unspecified'),
                COALESCE(u.model_id, 'unknown'),
                COUNT(*) as call_count,
                COALESCE(SUM(u.input_tokens), 0),
                COALESCE(SUM(u.output_tokens), 0),
                COALESCE(SUM(u.cache_creation_input_tokens), 0),
                COALESCE(SUM(u.cache_read_input_tokens), 0),
                AVG(u.duration_ms),
                AVG(u.ttft_ms),
                u.provider_id
         FROM model_usage_events u
         {}
         GROUP BY u.operation, u.model_id, u.provider_id
         ORDER BY u.operation ASC",
        f.where_sql
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            crate::sql_u64(r, 2)?,
            crate::sql_u64(r, 3)?,
            crate::sql_u64(r, 4)?,
            crate::sql_u64(r, 5)?,
            crate::sql_u64(r, 6)?,
            r.get::<_, Option<f64>>(7)?,
            r.get::<_, Option<f64>>(8)?,
            r.get::<_, Option<String>>(9)?,
        ))
    })?;
    let mut by_operation_map: std::collections::BTreeMap<String, TokenByOperation> =
        std::collections::BTreeMap::new();
    for row in rows {
        let (
            operation,
            model_id,
            call_count,
            input_tokens,
            output_tokens,
            cache_creation_input_tokens,
            cache_read_input_tokens,
            avg_duration_ms,
            avg_ttft_ms,
            provider_id,
        ) = row?;
        let domain = operation_domain(&operation).to_string();
        let entry = by_operation_map
            .entry(operation.clone())
            .or_insert(TokenByOperation {
                operation,
                domain,
                call_count: 0,
                input_tokens: 0,
                output_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                estimated_cost_usd: 0.0,
                avg_duration_ms: None,
                avg_ttft_ms: None,
            });
        let old_calls = entry.call_count;
        entry.call_count += call_count;
        entry.input_tokens += input_tokens;
        entry.output_tokens += output_tokens;
        entry.cache_creation_input_tokens += cache_creation_input_tokens;
        entry.cache_read_input_tokens += cache_read_input_tokens;
        entry.estimated_cost_usd += resolve_cost(
            provider_id.as_deref(),
            &model_id,
            input_tokens,
            output_tokens,
        );
        entry.avg_duration_ms = merge_weighted_avg(
            entry.avg_duration_ms,
            old_calls,
            avg_duration_ms,
            call_count,
            entry.call_count,
        );
        entry.avg_ttft_ms = merge_weighted_avg(
            entry.avg_ttft_ms,
            old_calls,
            avg_ttft_ms,
            call_count,
            entry.call_count,
        );
    }
    let mut by_operation: Vec<TokenByOperation> = by_operation_map.into_values().collect();
    by_operation.sort_by_key(|o| std::cmp::Reverse(o.input_tokens + o.output_tokens));

    // By domain: rollup of the already-merged by_operation rows in memory —
    // no second SQL query. Weighted-averaging an already-weighted average by
    // call_count is exactly correct here because each by_operation row's
    // avg_* truly represents the mean over exactly call_count events, by
    // construction above.
    let mut by_domain_map: std::collections::BTreeMap<String, TokenByDomain> =
        std::collections::BTreeMap::new();
    for op in &by_operation {
        let entry = by_domain_map
            .entry(op.domain.clone())
            .or_insert(TokenByDomain {
                domain: op.domain.clone(),
                call_count: 0,
                input_tokens: 0,
                output_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                estimated_cost_usd: 0.0,
                avg_duration_ms: None,
                avg_ttft_ms: None,
            });
        let old_calls = entry.call_count;
        entry.call_count += op.call_count;
        entry.input_tokens += op.input_tokens;
        entry.output_tokens += op.output_tokens;
        entry.cache_creation_input_tokens += op.cache_creation_input_tokens;
        entry.cache_read_input_tokens += op.cache_read_input_tokens;
        entry.estimated_cost_usd += op.estimated_cost_usd;
        entry.avg_duration_ms = merge_weighted_avg(
            entry.avg_duration_ms,
            old_calls,
            op.avg_duration_ms,
            op.call_count,
            entry.call_count,
        );
        entry.avg_ttft_ms = merge_weighted_avg(
            entry.avg_ttft_ms,
            old_calls,
            op.avg_ttft_ms,
            op.call_count,
            entry.call_count,
        );
    }
    let mut by_domain: Vec<TokenByDomain> = by_domain_map.into_values().collect();
    by_domain.sort_by_key(|d| std::cmp::Reverse(d.input_tokens + d.output_tokens));

    let total_cost_usd = by_model.iter().map(|m| m.estimated_cost_usd).sum();

    Ok(DashboardTokenData {
        trend,
        by_model,
        by_kind,
        by_operation,
        by_domain,
        total_cost_usd,
    })
}

/// Tool usage stats: call counts, errors, durations grouped by tool name.
pub fn query_tool_usage(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
) -> Result<Vec<ToolUsageStats>> {
    let conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let f = build_session_filter(filter, "s", Some("m"));
    let sql = format!(
        "SELECT m.tool_name,
                COUNT(*) as call_count,
                COALESCE(SUM(CASE WHEN m.is_error = 1 THEN 1 ELSE 0 END), 0),
                COALESCE(AVG(m.tool_duration_ms), 0.0),
                COALESCE(SUM(m.tool_duration_ms), 0)
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         {}
           AND m.tool_name IS NOT NULL AND m.tool_name != ''
         GROUP BY m.tool_name
         ORDER BY call_count DESC",
        f.where_sql
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        Ok(ToolUsageStats {
            tool_name: r.get(0)?,
            call_count: crate::sql_u64(r, 1)?,
            error_count: crate::sql_u64(r, 2)?,
            avg_duration_ms: r.get(3)?,
            total_duration_ms: crate::sql_u64(r, 4)?,
        })
    })?;
    rows.collect::<std::result::Result<_, _>>()
        .map_err(Into::into)
}

/// Session stats: daily trend and breakdown by agent.
pub fn query_sessions(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
) -> Result<DashboardSessionData> {
    let conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    // Daily trend (join-based for performance)
    let f = build_session_filter(filter, "s", None);
    let sql = format!(
        "SELECT DATE(s.created_at) as d,
                COUNT(DISTINCT s.id) as sess_count,
                COUNT(m.id) as msg_count
         FROM sessions s
         LEFT JOIN messages m ON m.session_id = s.id
         {}
         GROUP BY d
         ORDER BY d ASC",
        f.where_sql
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        Ok(SessionTrend {
            date: r.get(0)?,
            session_count: crate::sql_u64(r, 1)?,
            message_count: crate::sql_u64(r, 2)?,
        })
    })?;
    let trend: Vec<SessionTrend> = rows.collect::<std::result::Result<_, _>>()?;

    // By agent
    let f = build_session_filter(filter, "s", None);
    let sql = format!(
        "SELECT s.agent_id,
                COUNT(DISTINCT s.id) as sess_count,
                COUNT(m.id) as msg_count,
                COALESCE(SUM(m.tokens_in), 0) + COALESCE(SUM(m.tokens_out), 0) as total_tokens
         FROM sessions s
         LEFT JOIN messages m ON m.session_id = s.id
         {}
         GROUP BY s.agent_id
         ORDER BY sess_count DESC",
        f.where_sql
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        Ok(SessionByAgent {
            agent_id: r.get(0)?,
            session_count: crate::sql_u64(r, 1)?,
            message_count: crate::sql_u64(r, 2)?,
            total_tokens: crate::sql_u64(r, 3)?,
        })
    })?;
    let by_agent: Vec<SessionByAgent> = rows.collect::<std::result::Result<_, _>>()?;

    Ok(DashboardSessionData { trend, by_agent })
}

/// Error/warning stats from the logs database.
pub fn query_errors(log_db: &Arc<LogDB>, filter: &DashboardFilter) -> Result<DashboardErrorData> {
    let conn = log_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    // Daily trend
    let base_filter = build_log_filter(filter);
    let level_condition = if base_filter.where_sql.is_empty() {
        "WHERE level IN ('error', 'warn')".to_string()
    } else {
        format!("{} AND level IN ('error', 'warn')", base_filter.where_sql)
    };
    let sql = format!(
        "SELECT DATE(timestamp) as d,
                COALESCE(SUM(CASE WHEN level = 'error' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN level = 'warn' THEN 1 ELSE 0 END), 0)
         FROM logs
         {}
         GROUP BY d
         ORDER BY d ASC",
        level_condition
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&base_filter.params).as_slice(), |r| {
        Ok(ErrorTrend {
            date: r.get(0)?,
            error_count: crate::sql_u64(r, 1)?,
            warn_count: crate::sql_u64(r, 2)?,
        })
    })?;
    let trend: Vec<ErrorTrend> = rows.collect::<std::result::Result<_, _>>()?;

    // By category (errors only)
    let base_filter = build_log_filter(filter);
    let error_condition = if base_filter.where_sql.is_empty() {
        "WHERE level = 'error'".to_string()
    } else {
        format!("{} AND level = 'error'", base_filter.where_sql)
    };
    let sql = format!(
        "SELECT category, COUNT(*) as cnt
         FROM logs
         {}
         GROUP BY category
         ORDER BY cnt DESC",
        error_condition
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&base_filter.params).as_slice(), |r| {
        Ok(ErrorByCategory {
            category: r.get(0)?,
            count: crate::sql_u64(r, 1)?,
        })
    })?;
    let by_category: Vec<ErrorByCategory> = rows.collect::<std::result::Result<_, _>>()?;

    // Totals
    let base_filter = build_log_filter(filter);
    let level_condition = if base_filter.where_sql.is_empty() {
        "WHERE level IN ('error', 'warn')".to_string()
    } else {
        format!("{} AND level IN ('error', 'warn')", base_filter.where_sql)
    };
    let sql = format!(
        "SELECT COALESCE(SUM(CASE WHEN level = 'error' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN level = 'warn' THEN 1 ELSE 0 END), 0)
         FROM logs
         {}",
        level_condition
    );
    let (total_errors, total_warnings): (u64, u64) =
        conn.query_row(&sql, params_ref(&base_filter.params).as_slice(), |r| {
            Ok((crate::sql_u64(r, 0)?, crate::sql_u64(r, 1)?))
        })?;

    Ok(DashboardErrorData {
        trend,
        by_category,
        total_errors,
        total_warnings,
    })
}

/// Task stats: cron jobs and subagent runs.
pub fn query_tasks(
    session_db: &Arc<SessionDB>,
    cron_db: &Arc<CronDB>,
    filter: &DashboardFilter,
) -> Result<DashboardTaskData> {
    // ── Cron stats ──
    let cron_conn = cron_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let total_jobs: u64 = cron_conn.query_row("SELECT COUNT(*) FROM cron_jobs", [], |r| {
        crate::sql_u64(r, 0)
    })?;
    let active_jobs: u64 = cron_conn.query_row(
        "SELECT COUNT(*) FROM cron_jobs WHERE status = 'active'",
        [],
        |r| crate::sql_u64(r, 0),
    )?;

    // Run logs with optional date filter
    let mut clauses: Vec<String> = Vec::new();
    let mut cron_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(ref start) = filter.start_date {
        if !start.is_empty() {
            clauses.push("started_at >= ?".to_string());
            cron_params.push(Box::new(start.clone()));
        }
    }
    if let Some(ref end) = filter.end_date {
        if !end.is_empty() {
            clauses.push("started_at <= ?".to_string());
            cron_params.push(Box::new(end.clone()));
        }
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };

    let sql = format!(
        // `total_runs` counts terminal runs only — the transient in-progress
        // 'running' row is excluded (review fix #3) so a live run doesn't inflate
        // the total; 'empty'/'cancelled' are terminal and still counted.
        // `failed_runs` counts every non-success terminal failure as the
        // complement of the known non-failure terminals (C05) — so `'timeout'`
        // (§5) AND the infra `'no_session'` literal (and any future failure tag)
        // all count, instead of an `IN ('error','timeout')` allowlist that
        // silently dropped 'no_session' from the failure denominator and inflated
        // the success rate. 'empty'/'cancelled' are non-failure terminals;
        // 'running' is in-progress (also excluded from total).
        "SELECT COALESCE(SUM(CASE WHEN status != 'running' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status NOT IN ('success', 'running', 'empty', 'cancelled') THEN 1 ELSE 0 END), 0),
                COALESCE(AVG(duration_ms), 0.0)
         FROM cron_run_logs
         {}",
        where_sql
    );
    let (total_runs, success_runs, failed_runs, avg_duration_ms): (u64, u64, u64, f64) = cron_conn
        .query_row(&sql, params_ref(&cron_params).as_slice(), |r| {
            Ok((
                crate::sql_u64(r, 0)?,
                crate::sql_u64(r, 1)?,
                crate::sql_u64(r, 2)?,
                r.get(3)?,
            ))
        })?;

    drop(cron_conn);

    let cron = CronJobStats {
        total_jobs,
        active_jobs,
        total_runs,
        success_runs,
        failed_runs,
        avg_duration_ms,
    };

    // ── Subagent stats ──
    let sess_conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let mut clauses: Vec<String> = Vec::new();
    let mut sub_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(ref start) = filter.start_date {
        if !start.is_empty() {
            clauses.push("started_at >= ?".to_string());
            sub_params.push(Box::new(start.clone()));
        }
    }
    if let Some(ref end) = filter.end_date {
        if !end.is_empty() {
            clauses.push("started_at <= ?".to_string());
            sub_params.push(Box::new(end.clone()));
        }
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };

    let sql = format!(
        "SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status = 'killed' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(AVG(duration_ms), 0.0)
         FROM subagent_runs
         {}",
        where_sql
    );
    let (total_runs, completed, failed, killed, total_input_tokens, total_output_tokens, avg_dur): (u64, u64, u64, u64, u64, u64, f64) =
        sess_conn.query_row(&sql, params_ref(&sub_params).as_slice(), |r| {
            Ok((crate::sql_u64(r, 0)?, crate::sql_u64(r, 1)?, crate::sql_u64(r, 2)?, crate::sql_u64(r, 3)?, crate::sql_u64(r, 4)?, crate::sql_u64(r, 5)?, r.get(6)?))
        })?;

    let subagent = SubagentStats {
        total_runs,
        completed,
        failed,
        killed,
        total_input_tokens,
        total_output_tokens,
        avg_duration_ms: avg_dur,
    };

    Ok(DashboardTaskData { cron, subagent })
}

/// System metrics: Hope Agent process CPU, memory, disk I/O (real-time snapshot).
pub fn query_system_metrics() -> Result<SystemMetrics> {
    let current_pid = sysinfo::get_current_pid()
        .map_err(|e| anyhow::anyhow!("Failed to get current PID: {}", e))?;

    let mut sys = System::new();
    // First refresh to initialize CPU measurement baseline
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[current_pid]),
        true,
        sysinfo::ProcessRefreshKind::everything(),
    );
    // Brief sleep to allow CPU usage delta measurement
    std::thread::sleep(std::time::Duration::from_millis(200));
    // Second refresh to get actual CPU usage
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[current_pid]),
        true,
        sysinfo::ProcessRefreshKind::everything(),
    );
    sys.refresh_cpu_list(sysinfo::CpuRefreshKind::default());
    sys.refresh_memory();

    let cpu_count = sys.cpus().len();

    let process = sys
        .process(current_pid)
        .ok_or_else(|| anyhow::anyhow!("Current process not found"))?;

    let process_cpu = process.cpu_usage();
    let rss = process.memory();
    let virtual_mem = process.virtual_memory();
    let disk_usage = process.disk_usage();
    let run_time = process.run_time();

    let system_total_mem = sys.total_memory();
    let rss_percent = if system_total_mem > 0 {
        (rss as f64 / system_total_mem as f64) * 100.0
    } else {
        0.0
    };

    let memory = ProcessMemoryInfo {
        rss_bytes: rss,
        virtual_bytes: virtual_mem,
        system_total_bytes: system_total_mem,
        rss_percent,
    };

    let disk_io = ProcessDiskIO {
        read_bytes: disk_usage.total_read_bytes,
        written_bytes: disk_usage.total_written_bytes,
    };

    let os_name = System::name().unwrap_or_else(|| "Unknown".to_string());
    let host_name = System::host_name().unwrap_or_else(|| "Unknown".to_string());
    let system_uptime_secs = System::uptime();

    Ok(SystemMetrics {
        process_cpu_percent: process_cpu,
        cpu_count,
        memory,
        disk_io,
        process_uptime_secs: run_time,
        pid: current_pid.as_u32(),
        os_name,
        host_name,
        system_uptime_secs,
    })
}

#[cfg(test)]
mod purpose_breakdown_tests {
    use super::*;
    use crate::model_usage::ModelUsageEvent;
    use crate::session::SessionDB;

    #[test]
    fn operation_domain_splits_on_first_dot() {
        assert_eq!(operation_domain("recap.facets"), "recap");
        assert_eq!(
            operation_domain("knowledge_maintenance.auto_tag"),
            "knowledge_maintenance"
        );
        assert_eq!(operation_domain("session_title"), "session_title");
        assert_eq!(operation_domain("recall_summary"), "recall_summary");
        assert_eq!(operation_domain("agent.side_query"), "agent");
        assert_eq!(operation_domain("permission_judge"), "permission_judge");
    }

    fn temp_db_path(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "{}-{}-{}.sqlite3",
            name,
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        std::env::temp_dir().join(unique)
    }

    fn insert_event(db: &SessionDB, operation: &str, model_id: &str, input: u64, output: u64) {
        let event = ModelUsageEvent {
            operation: Some(operation.to_string()),
            model_id: Some(model_id.to_string()),
            ..ModelUsageEvent::new("side_query").with_usage(input, output, 0, 0)
        };
        db.insert_model_usage_event(&event).expect("insert event");
    }

    #[test]
    fn by_operation_and_by_domain_totals_match_by_kind() {
        let path = temp_db_path("purpose-breakdown");
        let db = Arc::new(SessionDB::open(&path).expect("open"));

        insert_event(&db, "recap.facets", "claude-haiku-4-5", 100, 50);
        insert_event(&db, "recap.sections", "claude-haiku-4-5", 200, 80);
        insert_event(&db, "knowledge.ocr", "claude-sonnet-5", 300, 150);
        // no operation set -> falls into "unspecified", still counted
        let no_op_event = ModelUsageEvent {
            model_id: Some("claude-sonnet-5".to_string()),
            ..ModelUsageEvent::new("chat").with_usage(40, 20, 0, 0)
        };
        db.insert_model_usage_event(&no_op_event)
            .expect("insert event");

        let filter = DashboardFilter::default();
        let data = query_token_usage(&db, &filter).expect("query");

        let by_operation_total: u64 = data
            .by_operation
            .iter()
            .map(|o| o.input_tokens + o.output_tokens)
            .sum();
        let by_kind_total: u64 = data
            .by_kind
            .iter()
            .map(|k| k.input_tokens + k.output_tokens)
            .sum();
        assert_eq!(by_operation_total, by_kind_total);

        let recap_domain = data
            .by_domain
            .iter()
            .find(|d| d.domain == "recap")
            .expect("recap domain present");
        assert_eq!(recap_domain.call_count, 2);
        assert_eq!(recap_domain.input_tokens, 300);
        assert_eq!(recap_domain.output_tokens, 130);

        let unspecified = data
            .by_operation
            .iter()
            .find(|o| o.operation == "unspecified")
            .expect("unspecified operation present");
        assert_eq!(unspecified.domain, "unspecified");
        assert_eq!(unspecified.call_count, 1);

        let _ = std::fs::remove_file(&path);
    }
}
