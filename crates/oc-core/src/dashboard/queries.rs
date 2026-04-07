// ── Query Functions ─────────────────────────────────────────────

use anyhow::Result;
use std::sync::Arc;
use sysinfo::{ProcessesToUpdate, System};

use crate::cron::CronDB;
use crate::logging::LogDB;
use crate::session::SessionDB;

use super::cost::estimate_cost;
use super::filters::{build_log_filter, build_session_filter, params_ref};
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
    let total_sessions: u64 =
        sess_conn.query_row(&sql, params_ref(&f.params).as_slice(), |r| crate::sql_u64(r, 0))?;

    // Message count + token sums + tool calls + errors
    let f = build_session_filter(filter, "s", Some("m"));
    let sql = format!(
        "SELECT COUNT(m.id),
                COALESCE(SUM(m.tokens_in), 0),
                COALESCE(SUM(m.tokens_out), 0),
                COALESCE(SUM(CASE WHEN m.tool_name IS NOT NULL THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN m.is_error = 1 THEN 1 ELSE 0 END), 0)
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         {}",
        f.where_sql
    );
    let (total_messages, total_input_tokens, total_output_tokens, total_tool_calls, total_errors): (u64, u64, u64, u64, u64) =
        sess_conn.query_row(&sql, params_ref(&f.params).as_slice(), |r| {
            Ok((crate::sql_u64(r, 0)?, crate::sql_u64(r, 1)?, crate::sql_u64(r, 2)?, crate::sql_u64(r, 3)?, crate::sql_u64(r, 4)?))
        })?;

    // Active agents (distinct agent_ids in sessions within filter period)
    let f = build_session_filter(filter, "s", None);
    let sql = format!(
        "SELECT COUNT(DISTINCT s.agent_id) FROM sessions s {}",
        f.where_sql
    );
    let active_agents: u64 =
        sess_conn.query_row(&sql, params_ref(&f.params).as_slice(), |r| crate::sql_u64(r, 0))?;

    // Query average TTFT
    let f = build_session_filter(filter, "s", Some("m"));
    let sql = format!(
        "SELECT AVG(m.ttft_ms)
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         {} AND m.ttft_ms IS NOT NULL AND m.role = 'assistant'",
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
    let f = build_session_filter(filter, "s", Some("m"));
    let sql = format!(
        "SELECT COALESCE(s.model_id, 'unknown'),
                COALESCE(SUM(m.tokens_in), 0),
                COALESCE(SUM(m.tokens_out), 0)
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         {}
         GROUP BY s.model_id",
        f.where_sql
    );
    let mut stmt = sess_conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        Ok((
            r.get::<_, String>(0)?,
            crate::sql_u64(r, 1)?,
            crate::sql_u64(r, 2)?,
        ))
    })?;
    let mut estimated_cost_usd = 0.0;
    for row in rows {
        let (model, inp, out) = row?;
        estimated_cost_usd += estimate_cost(&model, inp, out);
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
    let f = build_session_filter(filter, "s", Some("m"));
    let sql = format!(
        "SELECT DATE(m.timestamp) as d,
                COALESCE(SUM(m.tokens_in), 0),
                COALESCE(SUM(m.tokens_out), 0),
                AVG(CASE WHEN m.ttft_ms IS NOT NULL AND m.role = 'assistant' THEN m.ttft_ms END)
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
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
    let f = build_session_filter(filter, "s", Some("m"));
    let sql = format!(
        "SELECT COALESCE(s.model_id, 'unknown'),
                COALESCE(s.provider_name, 'unknown'),
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
        let input_tokens: u64 = crate::sql_u64(r, 2)?;
        let output_tokens: u64 = crate::sql_u64(r, 3)?;
        let avg_ttft_ms: Option<f64> = r.get(4)?;
        Ok(TokenByModel {
            estimated_cost_usd: estimate_cost(&model_id, input_tokens, output_tokens),
            model_id,
            provider_name,
            input_tokens,
            output_tokens,
            avg_ttft_ms,
        })
    })?;
    let by_model: Vec<TokenByModel> = rows.collect::<std::result::Result<_, _>>()?;

    let total_cost_usd = by_model.iter().map(|m| m.estimated_cost_usd).sum();

    Ok(DashboardTokenData {
        trend,
        by_model,
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

    let total_jobs: u64 =
        cron_conn.query_row("SELECT COUNT(*) FROM cron_jobs", [], |r| crate::sql_u64(r, 0))?;
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
        "SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status = 'error' THEN 1 ELSE 0 END), 0),
                COALESCE(AVG(duration_ms), 0.0)
         FROM cron_run_logs
         {}",
        where_sql
    );
    let (total_runs, success_runs, failed_runs, avg_duration_ms): (u64, u64, u64, f64) = cron_conn
        .query_row(&sql, params_ref(&cron_params).as_slice(), |r| {
            Ok((crate::sql_u64(r, 0)?, crate::sql_u64(r, 1)?, crate::sql_u64(r, 2)?, r.get(3)?))
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

/// System metrics: OpenComputer process CPU, memory, disk I/O (real-time snapshot).
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
