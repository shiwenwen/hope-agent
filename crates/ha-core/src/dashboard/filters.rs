// ── Filter helpers ──────────────────────────────────────────────

use super::types::DashboardFilter;
use rusqlite::types::ToSql;

pub(super) struct FilterClause {
    pub where_sql: String,
    pub params: Vec<Box<dyn ToSql>>,
}

/// Build WHERE clause fragments for session-based queries.
/// `session_alias` is the table alias for sessions (e.g. "s").
/// `message_alias` is the optional table alias for messages (e.g. "m"), used when
/// the query joins messages and we need to filter on message timestamp.
pub(super) fn build_session_filter(
    filter: &DashboardFilter,
    session_alias: &str,
    message_alias: Option<&str>,
) -> FilterClause {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn ToSql>> = Vec::new();

    // Exclude cron sessions and sub-agent sessions from dashboard stats
    clauses.push(format!("{}.is_cron = 0", session_alias));
    clauses.push(format!("{}.parent_session_id IS NULL", session_alias));
    // Incognito sessions never surface in Dashboard stats — by definition
    // they leave no audit trail.
    clauses.push(format!("{}.incognito = 0", session_alias));

    if let Some(ref start) = filter.start_date {
        if !start.is_empty() {
            let ts_col = if let Some(ma) = message_alias {
                format!("{}.timestamp", ma)
            } else {
                format!("{}.created_at", session_alias)
            };
            clauses.push(format!("{} >= ?", ts_col));
            params.push(Box::new(start.clone()));
        }
    }

    if let Some(ref end) = filter.end_date {
        if !end.is_empty() {
            let ts_col = if let Some(ma) = message_alias {
                format!("{}.timestamp", ma)
            } else {
                format!("{}.created_at", session_alias)
            };
            clauses.push(format!("{} <= ?", ts_col));
            params.push(Box::new(end.clone()));
        }
    }

    if let Some(ref agent_id) = filter.agent_id {
        if !agent_id.is_empty() {
            clauses.push(format!("{}.agent_id = ?", session_alias));
            params.push(Box::new(agent_id.clone()));
        }
    }

    if let Some(ref provider_id) = filter.provider_id {
        if !provider_id.is_empty() {
            clauses.push(format!("{}.provider_id = ?", session_alias));
            params.push(Box::new(provider_id.clone()));
        }
    }

    if let Some(ref model_id) = filter.model_id {
        if !model_id.is_empty() {
            clauses.push(format!("{}.model_id = ?", session_alias));
            params.push(Box::new(model_id.clone()));
        }
    }

    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };

    FilterClause { where_sql, params }
}

/// Build WHERE clause for log-based queries (logs table).
pub(super) fn build_log_filter(filter: &DashboardFilter) -> FilterClause {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn ToSql>> = Vec::new();

    if let Some(ref start) = filter.start_date {
        if !start.is_empty() {
            clauses.push("timestamp >= ?".to_string());
            params.push(Box::new(start.clone()));
        }
    }

    if let Some(ref end) = filter.end_date {
        if !end.is_empty() {
            clauses.push("timestamp <= ?".to_string());
            params.push(Box::new(end.clone()));
        }
    }

    if let Some(ref agent_id) = filter.agent_id {
        if !agent_id.is_empty() {
            clauses.push("agent_id = ?".to_string());
            params.push(Box::new(agent_id.clone()));
        }
    }

    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };

    FilterClause { where_sql, params }
}

pub(super) fn params_ref(params: &[Box<dyn ToSql>]) -> Vec<&dyn ToSql> {
    params.iter().map(|p| p.as_ref()).collect()
}
