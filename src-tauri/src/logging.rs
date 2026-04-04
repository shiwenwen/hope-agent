use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

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
    pub(crate) conn: Mutex<Connection>,
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
            CREATE INDEX IF NOT EXISTS idx_logs_session_id ON logs(session_id);",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO logs (timestamp, level, category, source, message, details, session_id, agent_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![now, level, category, source, message, details, session_id, agent_id],
        )?;
        Ok(())
    }

    pub fn batch_insert(&self, entries: &[PendingLog]) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let mut where_clauses: Vec<String> = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref levels) = filter.levels {
            if !levels.is_empty() {
                let placeholders: Vec<String> = levels
                    .iter()
                    .enumerate()
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
                let placeholders: Vec<String> = categories
                    .iter()
                    .enumerate()
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
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let total: u64 = conn.query_row(&count_sql, params_ref.as_slice(), |row| crate::sql_u64(row, 0))?;

        // Query page
        let offset = (page.saturating_sub(1)) * page_size;
        let query_sql = format!(
            "SELECT id, timestamp, level, category, source, message, details, session_id, agent_id
             FROM logs {} ORDER BY id DESC LIMIT ? OFFSET ?",
            where_sql
        );
        let mut all_params: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let total: u64 = conn.query_row("SELECT COUNT(*) FROM logs", [], |row| crate::sql_u64(row, 0))?;

        let mut by_level = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT level, COUNT(*) FROM logs GROUP BY level")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, crate::sql_u64(row, 1)?))
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
                Ok((row.get::<_, String>(0)?, crate::sql_u64(row, 1)?))
            })?;
            for row in rows {
                let (cat, count) = row?;
                by_category.insert(cat, count);
            }
        }

        let db_size_bytes = std::fs::metadata(db_path).map(|m| m.len()).unwrap_or(0);

        Ok(LogStats {
            total,
            by_level,
            by_category,
            db_size_bytes,
        })
    }

    pub fn clear(&self, before_date: Option<&str>) -> Result<u64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
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

// ── Log File Writer ──────────────────────────────────────────────

/// Writes plain text log lines to date-based files under ~/.opencomputer/logs/
/// Format: `[TIMESTAMP] LEVEL [CATEGORY] SOURCE — MESSAGE`
/// Files named: `opencomputer-YYYY-MM-DD.log`, auto-rotate by date and size.
struct LogFileWriter {
    logs_dir: PathBuf,
    current_file: Option<std::fs::File>,
    current_date: String,
    current_size: u64,
    max_size_bytes: u64,
}

impl LogFileWriter {
    fn new(logs_dir: PathBuf, max_size_mb: u32) -> Self {
        Self {
            logs_dir,
            current_file: None,
            current_date: String::new(),
            current_size: 0,
            max_size_bytes: max_size_mb as u64 * 1024 * 1024,
        }
    }

    fn update_max_size(&mut self, max_size_mb: u32) {
        self.max_size_bytes = max_size_mb as u64 * 1024 * 1024;
    }

    fn write_entry(&mut self, entry: &PendingLog) {
        let date = &entry.timestamp[..10]; // "2026-03-21"

        // Rotate if date changed or file exceeded max size
        if date != self.current_date || self.current_size >= self.max_size_bytes {
            self.current_file = None;
        }

        if self.current_file.is_none() {
            if let Err(e) = self.open_file(date) {
                eprintln!("Failed to open log file: {}", e);
                return;
            }
        }

        if let Some(ref mut file) = self.current_file {
            // Format: [2026-03-21T10:30:00Z] INFO [agent] agent::run — Starting chat session
            let line = if let Some(ref details) = entry.details {
                format!(
                    "[{}] {} [{}] {} — {} | {}\n",
                    entry.timestamp,
                    entry.level.to_uppercase(),
                    entry.category,
                    entry.source,
                    entry.message,
                    details
                )
            } else {
                format!(
                    "[{}] {} [{}] {} — {}\n",
                    entry.timestamp,
                    entry.level.to_uppercase(),
                    entry.category,
                    entry.source,
                    entry.message
                )
            };
            let bytes = line.as_bytes();
            if file.write_all(bytes).is_ok() {
                self.current_size += bytes.len() as u64;
            }
        }
    }

    fn open_file(&mut self, date: &str) -> Result<()> {
        std::fs::create_dir_all(&self.logs_dir)?;
        self.current_date = date.to_string();

        // Find a non-full file for this date
        let base_name = format!("opencomputer-{}.log", date);
        let path = self.logs_dir.join(&base_name);

        // If base file exists and is over max size, use numbered suffix
        let (final_path, existing_size) = if path.exists() {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if size >= self.max_size_bytes {
                // Find next available numbered file
                let mut n = 1u32;
                loop {
                    let numbered = self
                        .logs_dir
                        .join(format!("opencomputer-{}.{}.log", date, n));
                    if !numbered.exists() {
                        break (numbered, 0);
                    }
                    let s = std::fs::metadata(&numbered).map(|m| m.len()).unwrap_or(0);
                    if s < self.max_size_bytes {
                        break (numbered, s);
                    }
                    n += 1;
                }
            } else {
                (path, size)
            }
        } else {
            (path, 0)
        };

        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&final_path)?;
        self.current_file = Some(file);
        self.current_size = existing_size;
        Ok(())
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
    sender: tokio::sync::mpsc::Sender<PendingLog>,
    config: std::sync::Arc<std::sync::RwLock<LogConfig>>,
}

impl AppLogger {
    pub fn new(db: std::sync::Arc<LogDB>, logs_dir: PathBuf) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(10000);
        let config = std::sync::Arc::new(std::sync::RwLock::new(LogConfig::default()));
        let config_clone = config.clone();

        // Spawn background writer task.
        // .manage() runs before the Tokio runtime is ready, so we always use a
        // dedicated thread with its own runtime to avoid the "no reactor" panic.
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create logging runtime");
            rt.block_on(Self::writer_loop(rx, db, logs_dir, config_clone));
        });

        Self { sender: tx, config }
    }

    async fn writer_loop(
        mut rx: tokio::sync::mpsc::Receiver<PendingLog>,
        db: std::sync::Arc<LogDB>,
        logs_dir: PathBuf,
        config: std::sync::Arc<std::sync::RwLock<LogConfig>>,
    ) {
        let file_max_mb = config.read().map(|c| c.file_max_size_mb).unwrap_or(10);
        let mut file_writer = LogFileWriter::new(logs_dir, file_max_mb);
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
                        let file_enabled = config.read().map(|c| c.file_enabled).unwrap_or(true);
                        if file_enabled {
                            for entry in &buffer {
                                file_writer.write_entry(entry);
                            }
                        }
                    }
                    break;
                }
                Err(_) => {
                    // Timeout — flush what we have
                }
            }

            if !buffer.is_empty() {
                let _ = db.batch_insert(&buffer);

                // Dual-write to log file
                let (file_enabled, file_max_mb) = config
                    .read()
                    .map(|c| (c.file_enabled, c.file_max_size_mb))
                    .unwrap_or((true, 10));
                if file_enabled {
                    file_writer.update_max_size(file_max_mb);
                    for entry in &buffer {
                        file_writer.write_entry(entry);
                    }
                }

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

        let timestamp = chrono::Utc::now().to_rfc3339();

        // Dev mode: also print to stderr for console visibility
        #[cfg(debug_assertions)]
        {
            let level_upper = level.to_uppercase();
            eprintln!(
                "[{}] {} [{}] {} — {}",
                &timestamp[11..19], // HH:MM:SS
                level_upper,
                category,
                source,
                message,
            );
        }

        let entry = PendingLog {
            timestamp,
            level: level.to_string(),
            category: category.to_string(),
            source: source.to_string(),
            message: message.to_string(),
            details,
            session_id,
            agent_id,
        };
        if self.sender.try_send(entry).is_err() {
            eprintln!("[LOG] Warning: log channel full, dropping entry");
        }
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

// ── Global Logging Macros ────────────────────────────────────────
//
// Use these macros instead of `log::info!` / `log::warn!` etc. so that
// messages are written to both the SQLite database AND the log file via
// `AppLogger`.  The `log` crate only prints to the console (stderr).
//
// Usage:
//   app_info!("category", "source", "message {} {}", arg1, arg2);
//   app_warn!("category", "source", "something went wrong: {}", err);
//   app_error!("category", "source", "fatal: {}", err);
//   app_debug!("category", "source", "verbose detail: {}", val);

#[macro_export]
macro_rules! app_info {
    ($cat:expr, $src:expr, $($arg:tt)+) => {
        if let Some(logger) = $crate::get_logger() {
            logger.log("info", $cat, $src, &format!($($arg)+), None, None, None);
        }
    };
}

#[macro_export]
macro_rules! app_warn {
    ($cat:expr, $src:expr, $($arg:tt)+) => {
        if let Some(logger) = $crate::get_logger() {
            logger.log("warn", $cat, $src, &format!($($arg)+), None, None, None);
        }
    };
}

#[macro_export]
macro_rules! app_error {
    ($cat:expr, $src:expr, $($arg:tt)+) => {
        if let Some(logger) = $crate::get_logger() {
            logger.log("error", $cat, $src, &format!($($arg)+), None, None, None);
        }
    };
}

#[macro_export]
macro_rules! app_debug {
    ($cat:expr, $src:expr, $($arg:tt)+) => {
        if let Some(logger) = $crate::get_logger() {
            logger.log("debug", $cat, $src, &format!($($arg)+), None, None, None);
        }
    };
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

// ── Log File Operations ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFileInfo {
    pub name: String,
    pub size_bytes: u64,
    pub modified: String,
}

/// List all .log files under ~/.opencomputer/logs/, newest first.
pub fn list_log_files() -> Result<Vec<LogFileInfo>> {
    let logs_dir = crate::paths::logs_dir()?;
    if !logs_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&logs_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("log") {
            let meta = std::fs::metadata(&path)?;
            let modified = meta
                .modified()
                .map(|t| {
                    let dt: chrono::DateTime<chrono::Utc> = t.into();
                    dt.to_rfc3339()
                })
                .unwrap_or_default();
            files.push(LogFileInfo {
                name: entry.file_name().to_string_lossy().to_string(),
                size_bytes: meta.len(),
                modified,
            });
        }
    }
    // Sort newest first by name (date-based names sort naturally)
    files.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(files)
}

/// Read a log file with optional tail (last N lines). Returns the content as a string.
/// If `tail_lines` is Some(n), returns only the last n lines; otherwise returns full content.
pub fn read_log_file(filename: &str, tail_lines: Option<u32>) -> Result<String> {
    // Sanitize filename to prevent path traversal
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err(anyhow::anyhow!("Invalid log filename"));
    }
    let logs_dir = crate::paths::logs_dir()?;
    let path = logs_dir.join(filename);
    if !path.exists() {
        return Err(anyhow::anyhow!("Log file not found: {}", filename));
    }

    let content = std::fs::read_to_string(&path)?;

    if let Some(n) = tail_lines {
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(n as usize);
        Ok(lines[start..].join("\n"))
    } else {
        Ok(content)
    }
}

/// Clean up old log files beyond max_age_days.
pub fn cleanup_old_log_files(max_age_days: u32) -> Result<u64> {
    let logs_dir = crate::paths::logs_dir()?;
    if !logs_dir.exists() {
        return Ok(0);
    }
    let cutoff = chrono::Utc::now() - chrono::Duration::days(max_age_days as i64);
    let cutoff_date = cutoff.format("%Y-%m-%d").to_string();
    let mut removed = 0u64;
    for entry in std::fs::read_dir(&logs_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        // Parse date from filename: opencomputer-YYYY-MM-DD.log or opencomputer-YYYY-MM-DD.N.log
        if let Some(date_part) = name.strip_prefix("opencomputer-") {
            let date = &date_part[..10.min(date_part.len())];
            if date < cutoff_date.as_str() {
                let _ = std::fs::remove_file(entry.path());
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// Get the path to today's log file (for display in UI).
pub fn current_log_file_path() -> Result<String> {
    let logs_dir = crate::paths::logs_dir()?;
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let path = logs_dir.join(format!("opencomputer-{}.log", today));
    Ok(path.to_string_lossy().to_string())
}

// ── Sensitive Data Redaction ─────────────────────────────────────

/// Redact potentially sensitive values from a JSON string for logging.
pub fn redact_sensitive(input: &str) -> String {
    let sensitive_keys = [
        "api_key",
        "apiKey",
        "api-key",
        "access_token",
        "accessToken",
        "refresh_token",
        "refreshToken",
        "authorization",
        "Authorization",
        "x-api-key",
        "bearer",
        "password",
        "secret",
    ];

    let mut result = input.to_string();
    for key in &sensitive_keys {
        // Pattern 1: "key":"value" or "key": "value" (JSON string values)
        let patterns = [format!("\"{}\":\"", key), format!("\"{}\": \"", key)];
        for pattern in &patterns {
            let mut search_from = 0;
            while search_from < result.len() {
                if let Some(pos) = result[search_from..].find(pattern.as_str()) {
                    let start = search_from + pos;
                    let value_start = start + pattern.len();
                    if let Some(end) = result[value_start..].find('"') {
                        let before = &result[..value_start];
                        let after = &result[value_start + end..];
                        result = format!("{}[REDACTED]{}", before, after);
                        search_from = value_start + "[REDACTED]".len();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
        }

        // Pattern 2: URL query parameters (?key=value& or &key=value&)
        for sep in &["?", "&"] {
            let url_pattern = format!("{}{}=", sep, key);
            let mut search_from = 0;
            while search_from < result.len() {
                if let Some(pos) = result[search_from..].find(url_pattern.as_str()) {
                    let start = search_from + pos;
                    let value_start = start + url_pattern.len();
                    let end = result[value_start..]
                        .find(|c: char| c == '&' || c == ' ' || c == '"' || c == '\n')
                        .unwrap_or(result.len() - value_start);
                    let before = &result[..value_start];
                    let after = &result[value_start + end..];
                    result = format!("{}[REDACTED]{}", before, after);
                    search_from = value_start + "[REDACTED]".len();
                } else {
                    break;
                }
            }
        }
    }
    result
}
