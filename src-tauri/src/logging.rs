use anyhow::Result;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
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
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            level: "info".to_string(),
            max_age_days: 30,
            max_size_mb: 100,
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

// ── Log Level Ordering ───────────────────────────────────────────

fn level_priority(level: &str) -> u8 {
    match level {
        "error" => 0,
        "warn" => 1,
        "info" => 2,
        "debug" => 3,
        _ => 4,
    }
}

fn should_log(entry_level: &str, config_level: &str) -> bool {
    level_priority(entry_level) <= level_priority(config_level)
}

// ── Database Manager ─────────────────────────────────────────────

pub struct LogDB {
    conn: Mutex<Connection>,
}

impl LogDB {
    pub fn open(db_path: &PathBuf) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                level TEXT NOT NULL,
                category TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT '',
                message TEXT NOT NULL DEFAULT '',
                details TEXT,
                session_id TEXT,
                agent_id TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_logs_timestamp ON logs(timestamp DESC);
            CREATE INDEX IF NOT EXISTS idx_logs_level ON logs(level);
            CREATE INDEX IF NOT EXISTS idx_logs_category ON logs(category);
            CREATE INDEX IF NOT EXISTS idx_logs_session_id ON logs(session_id);"
        )?;

        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn insert(
        &self,
        level: &str,
        category: &str,
        source: &str,
        message: &str,
        details: Option<&str>,
        session_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO logs (timestamp, level, category, source, message, details, session_id, agent_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![now, level, category, source, message, details, session_id, agent_id],
        )?;
        Ok(())
    }

    pub fn batch_insert(&self, entries: &[PendingLog]) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO logs (timestamp, level, category, source, message, details, session_id, agent_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
            )?;
            for entry in entries {
                stmt.execute(params![
                    entry.timestamp,
                    entry.level,
                    entry.category,
                    entry.source,
                    entry.message,
                    entry.details,
                    entry.session_id,
                    entry.agent_id,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn query(&self, filter: &LogFilter, page: u32, page_size: u32) -> Result<LogQueryResult> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let mut where_clauses: Vec<String> = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref levels) = filter.levels {
            if !levels.is_empty() {
                let placeholders: Vec<String> = levels.iter().enumerate()
                    .map(|(_, _)| "?".to_string())
                    .collect();
                where_clauses.push(format!("level IN ({})", placeholders.join(",")));
                for l in levels {
                    param_values.push(Box::new(l.clone()));
                }
            }
        }

        if let Some(ref categories) = filter.categories {
            if !categories.is_empty() {
                let placeholders: Vec<String> = categories.iter().enumerate()
                    .map(|(_, _)| "?".to_string())
                    .collect();
                where_clauses.push(format!("category IN ({})", placeholders.join(",")));
                for c in categories {
                    param_values.push(Box::new(c.clone()));
                }
            }
        }

        if let Some(ref keyword) = filter.keyword {
            if !keyword.is_empty() {
                where_clauses.push("message LIKE ?".to_string());
                param_values.push(Box::new(format!("%{}%", keyword)));
            }
        }

        if let Some(ref sid) = filter.session_id {
            if !sid.is_empty() {
                where_clauses.push("session_id = ?".to_string());
                param_values.push(Box::new(sid.clone()));
            }
        }

        if let Some(ref start) = filter.start_time {
            if !start.is_empty() {
                where_clauses.push("timestamp >= ?".to_string());
                param_values.push(Box::new(start.clone()));
            }
        }

        if let Some(ref end) = filter.end_time {
            if !end.is_empty() {
                where_clauses.push("timestamp <= ?".to_string());
                param_values.push(Box::new(end.clone()));
            }
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        // Count total
        let count_sql = format!("SELECT COUNT(*) FROM logs {}", where_sql);
        let params_ref: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
        let total: u64 = conn.query_row(&count_sql, params_ref.as_slice(), |row| row.get(0))?;

        // Query page
        let offset = (page.saturating_sub(1)) * page_size;
        let query_sql = format!(
            "SELECT id, timestamp, level, category, source, message, details, session_id, agent_id
             FROM logs {} ORDER BY id DESC LIMIT ? OFFSET ?",
            where_sql
        );
        let mut all_params: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
        let limit_val = page_size as i64;
        let offset_val = offset as i64;
        all_params.push(&limit_val);
        all_params.push(&offset_val);

        let mut stmt = conn.prepare(&query_sql)?;
        let rows = stmt.query_map(all_params.as_slice(), |row| {
            Ok(LogEntry {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                level: row.get(2)?,
                category: row.get(3)?,
                source: row.get(4)?,
                message: row.get(5)?,
                details: row.get(6)?,
                session_id: row.get(7)?,
                agent_id: row.get(8)?,
            })
        })?;

        let mut logs = Vec::new();
        for row in rows {
            logs.push(row?);
        }

        Ok(LogQueryResult { logs, total })
    }

    pub fn get_stats(&self, db_path: &PathBuf) -> Result<LogStats> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let total: u64 = conn.query_row("SELECT COUNT(*) FROM logs", [], |row| row.get(0))?;

        let mut by_level = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT level, COUNT(*) FROM logs GROUP BY level")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?;
            for row in rows {
                let (level, count) = row?;
                by_level.insert(level, count);
            }
        }

        let mut by_category = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT category, COUNT(*) FROM logs GROUP BY category")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?;
            for row in rows {
                let (cat, count) = row?;
                by_category.insert(cat, count);
            }
        }

        let db_size_bytes = std::fs::metadata(db_path).map(|m| m.len()).unwrap_or(0);

        Ok(LogStats { total, by_level, by_category, db_size_bytes })
    }

    pub fn clear(&self, before_date: Option<&str>) -> Result<u64> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let deleted = if let Some(date) = before_date {
            conn.execute("DELETE FROM logs WHERE timestamp < ?1", params![date])?
        } else {
            conn.execute("DELETE FROM logs", [])?
        };
        Ok(deleted as u64)
    }

    pub fn cleanup_old(&self, max_age_days: u32) -> Result<u64> {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(max_age_days as i64);
        let cutoff_str = cutoff.to_rfc3339();
        self.clear(Some(&cutoff_str))
    }

    pub fn export(&self, filter: &LogFilter) -> Result<Vec<LogEntry>> {
        // Query all matching logs (no pagination)
        let result = self.query(filter, 1, u32::MAX)?;
        Ok(result.logs)
    }
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

#[derive(Clone)]
pub struct AppLogger {
    sender: tokio::sync::mpsc::UnboundedSender<PendingLog>,
    config: std::sync::Arc<std::sync::RwLock<LogConfig>>,
}

impl AppLogger {
    pub fn new(db: std::sync::Arc<LogDB>) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let config = std::sync::Arc::new(std::sync::RwLock::new(LogConfig::default()));

        // Spawn background writer task
        tokio::spawn(Self::writer_loop(rx, db));

        Self { sender: tx, config }
    }

    async fn writer_loop(
        mut rx: tokio::sync::mpsc::UnboundedReceiver<PendingLog>,
        db: std::sync::Arc<LogDB>,
    ) {
        let mut buffer: Vec<PendingLog> = Vec::new();
        let flush_interval = tokio::time::Duration::from_millis(200);

        loop {
            // Wait for first message or timeout
            let msg = tokio::time::timeout(flush_interval, rx.recv()).await;

            match msg {
                Ok(Some(entry)) => {
                    buffer.push(entry);
                    // Drain any additional immediately available messages
                    while buffer.len() < 100 {
                        match rx.try_recv() {
                            Ok(entry) => buffer.push(entry),
                            Err(_) => break,
                        }
                    }
                }
                Ok(None) => {
                    // Channel closed, flush remaining and exit
                    if !buffer.is_empty() {
                        let _ = db.batch_insert(&buffer);
                    }
                    break;
                }
                Err(_) => {
                    // Timeout — flush what we have
                }
            }

            if !buffer.is_empty() {
                let _ = db.batch_insert(&buffer);
                buffer.clear();
            }
        }
    }

    pub fn log(
        &self,
        level: &str,
        category: &str,
        source: &str,
        message: &str,
        details: Option<String>,
        session_id: Option<String>,
        agent_id: Option<String>,
    ) {
        // Check if enabled and level passes
        if let Ok(config) = self.config.read() {
            if !config.enabled || !should_log(level, &config.level) {
                return;
            }
        }

        let entry = PendingLog {
            timestamp: chrono::Utc::now().to_rfc3339(),
            level: level.to_string(),
            category: category.to_string(),
            source: source.to_string(),
            message: message.to_string(),
            details,
            session_id,
            agent_id,
        };
        let _ = self.sender.send(entry);
    }

    pub fn update_config(&self, config: LogConfig) {
        if let Ok(mut c) = self.config.write() {
            *c = config;
        }
    }

    pub fn get_config(&self) -> LogConfig {
        self.config.read().map(|c| c.clone()).unwrap_or_default()
    }
}

// ── Config Persistence ───────────────────────────────────────────

const LOG_CONFIG_FILE: &str = "log_config.json";

pub fn load_log_config() -> Result<LogConfig> {
    let path = crate::paths::root_dir()?.join(LOG_CONFIG_FILE);
    if path.exists() {
        let data = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&data)?)
    } else {
        Ok(LogConfig::default())
    }
}

pub fn save_log_config(config: &LogConfig) -> Result<()> {
    let path = crate::paths::root_dir()?.join(LOG_CONFIG_FILE);
    let data = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, data)?;
    Ok(())
}

// ── Database Path Helper ─────────────────────────────────────────

pub fn db_path() -> Result<PathBuf> {
    Ok(crate::paths::root_dir()?.join("logs.db"))
}

// ── Sensitive Data Redaction ─────────────────────────────────────

/// Redact potentially sensitive values from a JSON string for logging.
pub fn redact_sensitive(input: &str) -> String {
    let sensitive_keys = [
        "api_key", "apiKey", "api-key",
        "access_token", "accessToken",
        "refresh_token", "refreshToken",
        "authorization", "Authorization",
        "x-api-key", "bearer",
        "password", "secret",
    ];

    let mut result = input.to_string();
    for key in &sensitive_keys {
        // Simple pattern: "key":"value" or "key": "value"
        let patterns = [
            format!("\"{}\":\"", key),
            format!("\"{}\": \"", key),
        ];
        for pattern in &patterns {
            if let Some(start) = result.find(pattern) {
                let value_start = start + pattern.len();
                if let Some(end) = result[value_start..].find('"') {
                    let before = &result[..value_start];
                    let after = &result[value_start + end..];
                    result = format!("{}[REDACTED]{}", before, after);
                }
            }
        }
    }
    result
}
