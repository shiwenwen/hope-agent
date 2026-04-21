use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::types::*;

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

pub(crate) fn should_log(entry_level: &str, config_level: &str) -> bool {
    level_priority(entry_level) <= level_priority(config_level)
}

// ── Database Manager ─────────────────────────────────────────────

pub struct LogDB {
    pub(crate) conn: Mutex<Connection>,
    path: PathBuf,
}

impl LogDB {
    pub fn open(db_path: &Path) -> Result<Self> {
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
            path: db_path.to_path_buf(),
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
                where_clauses.push(format!(
                    "level IN ({})",
                    crate::sql_in_placeholders(levels.len())
                ));
                for l in levels {
                    param_values.push(Box::new(l.clone()));
                }
            }
        }

        if let Some(ref categories) = filter.categories {
            if !categories.is_empty() {
                where_clauses.push(format!(
                    "category IN ({})",
                    crate::sql_in_placeholders(categories.len())
                ));
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
        let total: u64 = conn.query_row(&count_sql, params_ref.as_slice(), |row| {
            crate::sql_u64(row, 0)
        })?;

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

    pub fn get_stats(&self) -> Result<LogStats> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let total: u64 = conn.query_row("SELECT COUNT(*) FROM logs", [], |row| {
            crate::sql_u64(row, 0)
        })?;

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

        let db_size_bytes = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);

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

    /// Enforce an on-disk size ceiling for the logs database.
    ///
    /// When the DB file exceeds `max_size_mb`, the oldest rows are deleted in
    /// a single batch (targeting ~80% of the ceiling for headroom) and a
    /// `VACUUM` is issued so the file actually shrinks — plain `DELETE` in
    /// WAL mode does not reclaim space, which is why raising the GUI
    /// "max DB size" slider had no observable effect before this path
    /// existed.
    pub fn cleanup_by_size(&self, max_size_mb: u32) -> Result<u64> {
        if max_size_mb == 0 {
            return Ok(0);
        }
        let max_bytes = (max_size_mb as u64).saturating_mul(1024 * 1024);
        let current_size = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        if current_size <= max_bytes {
            return Ok(0);
        }

        let target_bytes = (max_bytes as f64 * 0.8) as u64;
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let total: u64 = conn.query_row("SELECT COUNT(*) FROM logs", [], |row| {
            crate::sql_u64(row, 0)
        })?;
        if total == 0 {
            return Ok(0);
        }

        let avg_row_bytes = (current_size / total).max(1);
        let overflow = current_size.saturating_sub(target_bytes);
        // +1 to ensure we cross the boundary when avg_row_bytes over-estimates.
        let rows_to_delete = (overflow / avg_row_bytes + 1).min(total);

        let deleted = conn.execute(
            "DELETE FROM logs WHERE id IN (SELECT id FROM logs ORDER BY timestamp ASC LIMIT ?1)",
            params![rows_to_delete as i64],
        )?;

        // VACUUM can't run inside a transaction, and batch_insert only opens
        // short-lived txs behind the same Mutex, so this is safe.
        conn.execute("VACUUM", [])?;

        Ok(deleted as u64)
    }

    pub fn export(&self, filter: &LogFilter) -> Result<Vec<LogEntry>> {
        // Query all matching logs (no pagination)
        let result = self.query(filter, 1, u32::MAX)?;
        Ok(result.logs)
    }
}
