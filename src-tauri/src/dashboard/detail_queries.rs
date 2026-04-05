// ── Detail List Queries ─────────────────────────────────────────

use anyhow::Result;
use std::sync::Arc;

use crate::logging::LogDB;
use crate::session::SessionDB;

use super::filters::{build_log_filter, build_session_filter, params_ref};
use super::types::*;

/// List sessions with message count and token totals.
pub fn query_session_list(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
) -> Result<Vec<DashboardSessionItem>> {
    let conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let f = build_session_filter(filter, "s", None);
    let sql = format!(
        "SELECT s.id, s.title, s.agent_id, s.model_id,
                (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) as msg_count,
                (SELECT COALESCE(SUM(m.tokens_in), 0) + COALESCE(SUM(m.tokens_out), 0) FROM messages m WHERE m.session_id = s.id) as total_tokens,
                s.created_at, s.updated_at
         FROM sessions s
         {}
         ORDER BY s.updated_at DESC
         LIMIT 100",
        f.where_sql
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        Ok(DashboardSessionItem {
            id: r.get(0)?,
            title: r.get(1)?,
            agent_id: r.get(2)?,
            model_id: r.get(3)?,
            message_count: crate::sql_u64(r, 4)?,
            total_tokens: crate::sql_u64(r, 5)?,
            created_at: r.get(6)?,
            updated_at: r.get(7)?,
        })
    })?;
    rows.collect::<std::result::Result<_, _>>()
        .map_err(Into::into)
}

/// List recent messages across all sessions.
pub fn query_message_list(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
) -> Result<Vec<DashboardMessageItem>> {
    let conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let f = build_session_filter(filter, "s", Some("m"));
    let sql = format!(
        "SELECT m.id, m.session_id, s.title, m.role,
                SUBSTR(m.content, 1, 200),
                COALESCE(m.tokens_in, 0),
                COALESCE(m.tokens_out, 0),
                m.timestamp
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         {}
         ORDER BY m.timestamp DESC
         LIMIT 100",
        f.where_sql
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        Ok(DashboardMessageItem {
            id: r.get(0)?,
            session_id: r.get(1)?,
            session_title: r.get(2)?,
            role: r.get(3)?,
            content_preview: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            tokens_in: crate::sql_u64(r, 5)?,
            tokens_out: crate::sql_u64(r, 6)?,
            timestamp: r.get(7)?,
        })
    })?;
    rows.collect::<std::result::Result<_, _>>()
        .map_err(Into::into)
}

/// List recent tool calls across all sessions.
pub fn query_tool_call_list(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
) -> Result<Vec<DashboardToolCallItem>> {
    let conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let f = build_session_filter(filter, "s", Some("m"));
    let extra = if f.where_sql.is_empty() {
        "WHERE m.tool_name IS NOT NULL AND m.tool_name != ''".to_string()
    } else {
        format!(
            "{} AND m.tool_name IS NOT NULL AND m.tool_name != ''",
            f.where_sql
        )
    };
    let sql = format!(
        "SELECT m.id, m.session_id, s.title, m.tool_name,
                COALESCE(m.is_error, 0),
                m.tool_duration_ms,
                m.timestamp
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         {}
         ORDER BY m.timestamp DESC
         LIMIT 100",
        extra
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        Ok(DashboardToolCallItem {
            id: r.get(0)?,
            session_id: r.get(1)?,
            session_title: r.get(2)?,
            tool_name: r.get(3)?,
            is_error: r.get::<_, i64>(4)? != 0,
            duration_ms: r.get(5)?,
            timestamp: r.get(6)?,
        })
    })?;
    rows.collect::<std::result::Result<_, _>>()
        .map_err(Into::into)
}

/// List recent error/warning log entries.
pub fn query_error_list(
    log_db: &Arc<LogDB>,
    filter: &DashboardFilter,
) -> Result<Vec<DashboardErrorItem>> {
    let conn = log_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let base = build_log_filter(filter);
    let condition = if base.where_sql.is_empty() {
        "WHERE level IN ('error', 'warn')".to_string()
    } else {
        format!("{} AND level IN ('error', 'warn')", base.where_sql)
    };
    let sql = format!(
        "SELECT id, level, category, source, message, session_id, timestamp
         FROM logs
         {}
         ORDER BY timestamp DESC
         LIMIT 100",
        condition
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&base.params).as_slice(), |r| {
        Ok(DashboardErrorItem {
            id: r.get(0)?,
            level: r.get(1)?,
            category: r.get(2)?,
            source: r.get(3)?,
            message: r.get(4)?,
            session_id: r.get(5)?,
            timestamp: r.get(6)?,
        })
    })?;
    rows.collect::<std::result::Result<_, _>>()
        .map_err(Into::into)
}

/// List agents with session counts and token totals.
pub fn query_agent_list(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
) -> Result<Vec<DashboardAgentItem>> {
    let conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let f = build_session_filter(filter, "s", None);
    let sql = format!(
        "SELECT s.agent_id,
                COUNT(DISTINCT s.id) as sess_count,
                COUNT(m.id) as msg_count,
                COALESCE(SUM(m.tokens_in), 0) + COALESCE(SUM(m.tokens_out), 0) as total_tokens,
                MAX(s.updated_at) as last_active
         FROM sessions s
         LEFT JOIN messages m ON m.session_id = s.id
         {}
         GROUP BY s.agent_id
         ORDER BY sess_count DESC",
        f.where_sql
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref(&f.params).as_slice(), |r| {
        Ok(DashboardAgentItem {
            agent_id: r.get(0)?,
            session_count: crate::sql_u64(r, 1)?,
            message_count: crate::sql_u64(r, 2)?,
            total_tokens: crate::sql_u64(r, 3)?,
            last_active_at: r.get(4)?,
        })
    })?;
    rows.collect::<std::result::Result<_, _>>()
        .map_err(Into::into)
}
