// ── Dashboard Types ─────────────────────────────────────────────

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
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

/// Current overview + previous-period baseline for delta computation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverviewStatsWithDelta {
    pub current: OverviewStats,
    /// Previous period shifted by the same span as the current range.
    /// `None` when no valid previous window (e.g. start_date unset).
    pub previous: Option<OverviewStats>,
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

// ── Insights Types (Phase 2 enhancements) ───────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostTrendPoint {
    pub date: String,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardCostTrend {
    pub points: Vec<CostTrendPoint>,
    pub total_cost_usd: f64,
    pub peak_day: Option<String>,
    pub peak_cost_usd: f64,
    pub avg_daily_cost_usd: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapCell {
    /// 0 = Sunday, 6 = Saturday (SQLite strftime('%w'))
    pub weekday: u8,
    pub hour: u8,
    pub message_count: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardHeatmap {
    pub cells: Vec<HeatmapCell>,
    pub max_value: u64,
    pub total: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HourlyBucket {
    pub hour: u8,
    pub message_count: u64,
    pub session_count: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardHourlyDistribution {
    pub buckets: Vec<HourlyBucket>,
    pub peak_hour: Option<u8>,
    pub peak_message_count: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopSession {
    pub id: String,
    pub title: Option<String>,
    pub agent_id: String,
    pub model_id: Option<String>,
    pub total_tokens: u64,
    pub message_count: u64,
    pub estimated_cost_usd: f64,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEfficiency {
    pub model_id: String,
    pub provider_name: String,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
    pub avg_ttft_ms: Option<f64>,
    pub message_count: u64,
    pub avg_tokens_per_message: f64,
    pub avg_cost_per_1k_tokens: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthBreakdown {
    /// 0..=100 overall score
    pub score: u8,
    pub log_error_rate_percent: f64,
    pub tool_error_rate_percent: f64,
    pub cron_success_rate_percent: f64,
    pub subagent_success_rate_percent: f64,
    /// Status: "excellent" | "good" | "warning" | "critical"
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardInsights {
    pub health: HealthBreakdown,
    pub cost_trend: DashboardCostTrend,
    pub heatmap: DashboardHeatmap,
    pub hourly: DashboardHourlyDistribution,
    pub top_sessions: Vec<TopSession>,
    pub model_efficiency: Vec<ModelEfficiency>,
}
