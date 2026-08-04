use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Data Structures ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub id: i64,
    pub timestamp: String,
    pub level: String,
    pub category: String,
    pub source: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LogFilter {
    pub levels: Option<Vec<String>>,
    pub categories: Option<Vec<String>>,
    pub keyword: Option<String>,
    pub session_id: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogConfig {
    pub enabled: bool,
    pub level: String,
    pub max_age_days: u32,
    pub max_size_mb: u32,
    /// Enable plain text log file output (for external tools / Agent self-inspection)
    #[serde(default = "crate::default_true")]
    pub file_enabled: bool,
    /// Max single log file size in MB before rotation (default 10MB)
    #[serde(default = "default_file_max_size")]
    pub file_max_size_mb: u32,
}
fn default_file_max_size() -> u32 {
    10
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            level: "info".to_string(),
            max_age_days: 30,
            max_size_mb: 100,
            file_enabled: true,
            file_max_size_mb: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogStats {
    pub total: u64,
    pub by_level: HashMap<String, u64>,
    pub by_category: HashMap<String, u64>,
    pub db_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogQueryResult {
    pub logs: Vec<LogEntry>,
    pub total: u64,
}

// ── Async Logger (non-blocking) ──────────────────────────────────

#[derive(Debug, Clone)]
pub struct PendingLog {
    pub timestamp: String,
    pub level: String,
    pub category: String,
    pub source: String,
    pub message: String,
    pub details: Option<String>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
}
