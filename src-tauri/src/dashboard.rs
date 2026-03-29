// ── Dashboard Analytics Module ──────────────────────────────────
//
// Provides SQL aggregation queries for the dashboard, accessing
// SessionDB (sessions + messages + subagent_runs), LogDB (logs),
// and CronDB (cron_jobs + cron_run_logs).

use anyhow::Result;
use std::sync::Arc;
use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::cron::CronDB;
use crate::logging::LogDB;
use crate::session::SessionDB;

// ── Types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardFilter {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub agent_id: Option<String>,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverviewStats {
    pub total_sessions: u64,
    pub total_messages: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tool_calls: u64,
    pub total_errors: u64,
    pub active_agents: u64,
    pub active_cron_jobs: u64,
    pub estimated_cost_usd: f64,
    /// Average time to first token in milliseconds
    pub avg_ttft_ms: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageTrend {
    pub date: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Average time to first token for this date
    pub avg_ttft_ms: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenByModel {
    pub model_id: String,
    pub provider_name: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost_usd: f64,
    /// Average time to first token for this model
    pub avg_ttft_ms: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardTokenData {
    pub trend: Vec<TokenUsageTrend>,
    pub by_model: Vec<TokenByModel>,
    pub total_cost_usd: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUsageStats {
    pub tool_name: String,
    pub call_count: u64,
    pub error_count: u64,
    pub avg_duration_ms: f64,
    pub total_duration_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTrend {
    pub date: String,
    pub session_count: u64,
    pub message_count: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionByAgent {
    pub agent_id: String,
    pub session_count: u64,
    pub message_count: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSessionData {
    pub trend: Vec<SessionTrend>,
    pub by_agent: Vec<SessionByAgent>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorTrend {
    pub date: String,
    pub error_count: u64,
    pub warn_count: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorByCategory {
    pub category: String,
    pub count: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardErrorData {
    pub trend: Vec<ErrorTrend>,
    pub by_category: Vec<ErrorByCategory>,
    pub total_errors: u64,
    pub total_warnings: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJobStats {
    pub total_jobs: u64,
    pub active_jobs: u64,
    pub total_runs: u64,
    pub success_runs: u64,
    pub failed_runs: u64,
    pub avg_duration_ms: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentStats {
    pub total_runs: u64,
    pub completed: u64,
    pub failed: u64,
    pub killed: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub avg_duration_ms: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardTaskData {
    pub cron: CronJobStats,
    pub subagent: SubagentStats,
}

// ── System Metrics Types (Process-level) ────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessMemoryInfo {
    /// Process RSS (resident set size) in bytes
    pub rss_bytes: u64,
    /// Process virtual memory in bytes
    pub virtual_bytes: u64,
    /// System total memory in bytes (for context)
    pub system_total_bytes: u64,
    /// RSS as percentage of system total memory
    pub rss_percent: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessDiskIO {
    /// Total bytes read by process
    pub read_bytes: u64,
    /// Total bytes written by process
    pub written_bytes: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMetrics {
    /// Process CPU usage (percentage, can exceed 100% on multi-core)
    pub process_cpu_percent: f32,
    /// Number of CPU cores (for context)
    pub cpu_count: usize,
    /// Process memory info
    pub memory: ProcessMemoryInfo,
    /// Process disk I/O
    pub disk_io: ProcessDiskIO,
    /// Process uptime in seconds
    pub process_uptime_secs: u64,
    /// Process ID
    pub pid: u32,
    /// OS name
    pub os_name: String,
    /// Host name
    pub host_name: String,
    /// System uptime in seconds
    pub system_uptime_secs: u64,
}

// ── Detail List Types ───────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSessionItem {
    pub id: String,
    pub title: Option<String>,
    pub agent_id: String,
    pub model_id: Option<String>,
    pub message_count: u64,
    pub total_tokens: u64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardMessageItem {
    pub id: i64,
    pub session_id: String,
    pub session_title: Option<String>,
    pub role: String,
    pub content_preview: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub timestamp: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardToolCallItem {
    pub id: i64,
    pub session_id: String,
    pub session_title: Option<String>,
    pub tool_name: String,
    pub is_error: bool,
    pub duration_ms: Option<f64>,
    pub timestamp: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardErrorItem {
    pub id: i64,
    pub level: String,
    pub category: String,
    pub source: String,
    pub message: String,
    pub session_id: Option<String>,
    pub timestamp: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardAgentItem {
    pub agent_id: String,
    pub session_count: u64,
    pub message_count: u64,
    pub total_tokens: u64,
    pub last_active_at: String,
}

// ── Cost Estimation ─────────────────────────────────────────────

fn estimate_cost(model_id: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    // Pricing per 1M tokens: (input_price, output_price)
    let (input_price, output_price) = match model_id {
        // Anthropic
        m if m.contains("claude-3-5-sonnet") || m.contains("claude-3.5-sonnet") => (3.0, 15.0),
        m if m.contains("claude-3-5-haiku") || m.contains("claude-3.5-haiku") => (0.80, 4.0),
        m if m.contains("claude-3-opus") || m.contains("claude-3.0-opus") => (15.0, 75.0),
        m if m.contains("claude-3-sonnet") => (3.0, 15.0),
        m if m.contains("claude-3-haiku") || m.contains("claude-haiku-3") => (0.25, 1.25),
        m if m.contains("claude-4") || m.contains("claude-sonnet-4") => (3.0, 15.0),
        m if m.contains("claude-opus-4") => (15.0, 75.0),
        // OpenAI
        m if m.contains("gpt-4o-mini") => (0.15, 0.60),
        m if m.contains("gpt-4o") => (2.50, 10.0),
        m if m.contains("gpt-4-turbo") => (10.0, 30.0),
        m if m.contains("gpt-4") => (30.0, 60.0),
        m if m.contains("gpt-3.5") => (0.50, 1.50),
        m if m.contains("o1-mini") => (3.0, 12.0),
        m if m.contains("o1") => (15.0, 60.0),
        m if m.contains("o4-mini") => (1.10, 4.40),
        m if m.contains("o3-mini") => (1.10, 4.40),
        m if m.contains("o3") => (10.0, 40.0),
        // Google Gemini
        m if m.contains("gemini-2.5-pro") => (1.25, 10.0),
        m if m.contains("gemini-2.5-flash") => (0.15, 0.60),
        m if m.contains("gemini-2.0-flash") => (0.10, 0.40),
        m if m.contains("gemini-1.5-pro") => (1.25, 5.0),
        m if m.contains("gemini-1.5-flash") => (0.075, 0.30),
        // xAI Grok
        m if m.contains("grok-4-fast") || m.contains("grok-4-1-fast") => (0.2, 0.5),
        m if m.contains("grok-4.20") => (2.0, 6.0),
        m if m.contains("grok-4") => (3.0, 15.0),
        m if m.contains("grok-3-mini") => (0.3, 0.5),
        m if m.contains("grok-3-fast") => (5.0, 25.0),
        m if m.contains("grok-3") => (3.0, 15.0),
        m if m.contains("grok-code") => (0.2, 1.5),
        // Mistral
        m if m.contains("codestral") => (0.3, 0.9),
        m if m.contains("devstral") => (0.4, 2.0),
        m if m.contains("magistral") => (0.5, 1.5),
        m if m.contains("pixtral") => (2.0, 6.0),
        m if m.contains("mistral-large") => (0.5, 1.5),
        m if m.contains("mistral-medium") => (0.4, 2.0),
        m if m.contains("mistral-small") => (0.1, 0.3),
        // DeepSeek
        m if m.contains("deepseek-reasoner") || m.contains("DeepSeek-R1") => (0.55, 2.19),
        m if m.contains("deepseek") || m.contains("DeepSeek") => (0.27, 1.1),
        // Qwen
        m if m.contains("qwen-max") || m.contains("qwen3-max") => (2.4, 9.6),
        m if m.contains("qwen-plus") || m.contains("qwq-plus") => (0.8, 2.0),
        m if m.contains("qwen-turbo") => (0.3, 0.6),
        m if m.contains("qwen") => (0.30, 0.60),
        // GLM (Zhipu)
        m if m.contains("glm-5-turbo") => (1.2, 4.0),
        m if m.contains("glm-5") => (1.0, 3.2),
        m if m.contains("glm-4.7-flash") => (0.07, 0.4),
        m if m.contains("glm-4.7") || m.contains("glm-4-7") => (0.6, 2.2),
        m if m.contains("glm-4.6v") => (0.3, 0.9),
        m if m.contains("glm-4.6") => (0.6, 2.2),
        m if m.contains("glm-4.5-flash") => (0.0, 0.0),
        m if m.contains("glm-4.5") => (0.6, 2.2),
        // MiniMax
        m if m.contains("MiniMax") || m.contains("minimax") => (0.3, 1.2),
        // Llama (Together/HuggingFace)
        m if m.contains("Llama-4-Maverick") => (0.27, 0.85),
        m if m.contains("Llama-4-Scout") => (0.18, 0.59),
        m if m.contains("Llama-3.3-70B") || m.contains("llama-3.3-70b") => (0.88, 0.88),
        // Groq
        m if m.contains("mixtral") => (0.24, 0.24),
        _ => (3.0, 15.0), // default estimate
    };
    (input_tokens as f64 * input_price + output_tokens as f64 * output_price) / 1_000_000.0
}

// ── Filter helpers ──────────────────────────────────────────────

struct FilterClause {
    where_sql: String,
    params: Vec<Box<dyn rusqlite::types::ToSql>>,
}

/// Build WHERE clause fragments for session-based queries.
/// `session_alias` is the table alias for sessions (e.g. "s").
/// `message_alias` is the optional table alias for messages (e.g. "m"), used when
/// the query joins messages and we need to filter on message timestamp.
fn build_session_filter(
    filter: &DashboardFilter,
    session_alias: &str,
    message_alias: Option<&str>,
) -> FilterClause {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    // Exclude cron sessions and sub-agent sessions from dashboard stats
    clauses.push(format!("{}.is_cron = 0", session_alias));
    clauses.push(format!("{}.parent_session_id IS NULL", session_alias));

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
fn build_log_filter(filter: &DashboardFilter) -> FilterClause {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

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

fn params_ref(params: &[Box<dyn rusqlite::types::ToSql>]) -> Vec<&dyn rusqlite::types::ToSql> {
    params.iter().map(|p| p.as_ref()).collect()
}

// ── Query Functions ─────────────────────────────────────────────

/// Overview stats: session/message/token counts, tool calls, errors, active agents/cron.
pub fn query_overview(
    session_db: &Arc<SessionDB>,
    log_db: &Arc<LogDB>,
    cron_db: &Arc<CronDB>,
    filter: &DashboardFilter,
) -> Result<OverviewStats> {
    let sess_conn = session_db.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    // Session count
    let f = build_session_filter(filter, "s", None);
    let sql = format!("SELECT COUNT(*) FROM sessions s {}", f.where_sql);
    let total_sessions: u64 = sess_conn.query_row(&sql, params_ref(&f.params).as_slice(), |r| r.get(0))?;

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
         {}", f.where_sql
    );
    let (total_messages, total_input_tokens, total_output_tokens, total_tool_calls, total_errors): (u64, u64, u64, u64, u64) =
        sess_conn.query_row(&sql, params_ref(&f.params).as_slice(), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?;

    // Active agents (distinct agent_ids in sessions within filter period)
    let f = build_session_filter(filter, "s", None);
    let sql = format!("SELECT COUNT(DISTINCT s.agent_id) FROM sessions s {}", f.where_sql);
    let active_agents: u64 = sess_conn.query_row(&sql, params_ref(&f.params).as_slice(), |r| r.get(0))?;

    // Query average TTFT
    let f = build_session_filter(filter, "s", Some("m"));
    let sql = format!(
        "SELECT AVG(m.ttft_ms)
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         {} AND m.ttft_ms IS NOT NULL AND m.role = 'assistant'",
        if f.where_sql.is_empty() { "WHERE 1=1".to_string() } else { f.where_sql }
    );
    let avg_ttft_ms: Option<f64> = sess_conn.query_row(&sql, params_ref(&f.params).as_slice(), |r| r.get(0)).ok();

    drop(sess_conn);

    // Active cron jobs
    let cron_conn = cron_db.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
    let active_cron_jobs: u64 = cron_conn.query_row(
        "SELECT COUNT(*) FROM cron_jobs WHERE status = 'active'",
        [],
        |r| r.get(0),
    )?;
    drop(cron_conn);

    // Estimate cost by querying per-model token usage
    let sess_conn = session_db.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
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
        Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?, r.get::<_, u64>(2)?))
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
    let conn = session_db.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

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
            input_tokens: r.get(1)?,
            output_tokens: r.get(2)?,
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
        let input_tokens: u64 = r.get(2)?;
        let output_tokens: u64 = r.get(3)?;
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
    let conn = session_db.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

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
            call_count: r.get(1)?,
            error_count: r.get(2)?,
            avg_duration_ms: r.get(3)?,
            total_duration_ms: r.get(4)?,
        })
    })?;
    rows.collect::<std::result::Result<_, _>>().map_err(Into::into)
}

/// Session stats: daily trend and breakdown by agent.
pub fn query_sessions(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
) -> Result<DashboardSessionData> {
    let conn = session_db.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

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
            session_count: r.get(1)?,
            message_count: r.get(2)?,
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
            session_count: r.get(1)?,
            message_count: r.get(2)?,
            total_tokens: r.get(3)?,
        })
    })?;
    let by_agent: Vec<SessionByAgent> = rows.collect::<std::result::Result<_, _>>()?;

    Ok(DashboardSessionData { trend, by_agent })
}

/// Error/warning stats from the logs database.
pub fn query_errors(
    log_db: &Arc<LogDB>,
    filter: &DashboardFilter,
) -> Result<DashboardErrorData> {
    let conn = log_db.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

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
            error_count: r.get(1)?,
            warn_count: r.get(2)?,
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
            count: r.get(1)?,
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
            Ok((r.get(0)?, r.get(1)?))
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
    let cron_conn = cron_db.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let total_jobs: u64 = cron_conn.query_row(
        "SELECT COUNT(*) FROM cron_jobs",
        [],
        |r| r.get(0),
    )?;
    let active_jobs: u64 = cron_conn.query_row(
        "SELECT COUNT(*) FROM cron_jobs WHERE status = 'active'",
        [],
        |r| r.get(0),
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
    let (total_runs, success_runs, failed_runs, avg_duration_ms): (u64, u64, u64, f64) =
        cron_conn.query_row(&sql, params_ref(&cron_params).as_slice(), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
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
    let sess_conn = session_db.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

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
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
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

    let process = sys.process(current_pid)
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

// ── Detail List Queries ─────────────────────────────────────────

/// List sessions with message count and token totals.
pub fn query_session_list(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
) -> Result<Vec<DashboardSessionItem>> {
    let conn = session_db.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

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
            message_count: r.get(4)?,
            total_tokens: r.get(5)?,
            created_at: r.get(6)?,
            updated_at: r.get(7)?,
        })
    })?;
    rows.collect::<std::result::Result<_, _>>().map_err(Into::into)
}

/// List recent messages across all sessions.
pub fn query_message_list(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
) -> Result<Vec<DashboardMessageItem>> {
    let conn = session_db.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

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
            tokens_in: r.get(5)?,
            tokens_out: r.get(6)?,
            timestamp: r.get(7)?,
        })
    })?;
    rows.collect::<std::result::Result<_, _>>().map_err(Into::into)
}

/// List recent tool calls across all sessions.
pub fn query_tool_call_list(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
) -> Result<Vec<DashboardToolCallItem>> {
    let conn = session_db.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let f = build_session_filter(filter, "s", Some("m"));
    let extra = if f.where_sql.is_empty() {
        "WHERE m.tool_name IS NOT NULL AND m.tool_name != ''".to_string()
    } else {
        format!("{} AND m.tool_name IS NOT NULL AND m.tool_name != ''", f.where_sql)
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
    rows.collect::<std::result::Result<_, _>>().map_err(Into::into)
}

/// List recent error/warning log entries.
pub fn query_error_list(
    log_db: &Arc<LogDB>,
    filter: &DashboardFilter,
) -> Result<Vec<DashboardErrorItem>> {
    let conn = log_db.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

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
    rows.collect::<std::result::Result<_, _>>().map_err(Into::into)
}

/// List agents with session counts and token totals.
pub fn query_agent_list(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
) -> Result<Vec<DashboardAgentItem>> {
    let conn = session_db.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

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
            session_count: r.get(1)?,
            message_count: r.get(2)?,
            total_tokens: r.get(3)?,
            last_active_at: r.get(4)?,
        })
    })?;
    rows.collect::<std::result::Result<_, _>>().map_err(Into::into)
}
