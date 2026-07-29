//! ACP run persistence — CRUD operations on the `acp_runs` table.

use anyhow::Result;
use rusqlite::params;

use super::db::SessionDB;

use serde::{Deserialize, Serialize};

// ── ACP run 行类型（自 acp_control/types.rs 下沉：本文件持有 acp_runs 表，
// 行类型随表走；ha-acp 特征 crate 在原路径再导出）────────────────────

/// Persistent record of an ACP spawn run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpRun {
    /// Unique run identifier (UUID).
    pub run_id: String,
    /// Parent session that spawned this run.
    pub parent_session_id: String,
    /// Backend used.
    pub backend_id: String,
    /// External session ID on the agent side.
    #[serde(default)]
    pub external_session_id: Option<String>,
    /// Task description given to the agent.
    pub task: String,
    /// Current status.
    pub status: AcpRunStatus,
    /// Accumulated result text (truncated).
    #[serde(default)]
    pub result: Option<String>,
    /// Error message (if status is Error).
    #[serde(default)]
    pub error: Option<String>,
    /// Model actually used by the external agent.
    #[serde(default)]
    pub model_used: Option<String>,
    /// ISO-8601 timestamp.
    pub started_at: String,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    /// Optional user-facing label.
    #[serde(default)]
    pub label: Option<String>,
    /// Child process PID.
    #[serde(default)]
    pub pid: Option<u32>,
}

/// Status of an ACP run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcpRunStatus {
    Starting,
    Running,
    Completed,
    Error,
    Timeout,
    Killed,
}

impl AcpRunStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Error | Self::Timeout | Self::Killed
        )
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Timeout => "timeout",
            Self::Killed => "killed",
        }
    }
}

impl std::fmt::Display for AcpRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl SessionDB {
    // ── ACP Run Table Creation ──────────────────────────────────

    /// Create the acp_runs table if it does not exist.
    pub fn create_acp_runs_table(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS acp_runs (
                run_id TEXT PRIMARY KEY,
                parent_session_id TEXT NOT NULL,
                backend_id TEXT NOT NULL,
                external_session_id TEXT,
                task TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'starting',
                result TEXT,
                error TEXT,
                model_used TEXT,
                started_at TEXT NOT NULL DEFAULT (datetime('now')),
                finished_at TEXT,
                duration_ms INTEGER,
                input_tokens INTEGER,
                output_tokens INTEGER,
                label TEXT,
                pid INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_acp_runs_parent ON acp_runs(parent_session_id);
            CREATE INDEX IF NOT EXISTS idx_acp_runs_status ON acp_runs(status);",
        )?;
        Ok(())
    }

    // ── ACP Run CRUD ────────────────────────────────────────────

    /// Insert a new ACP run record.
    pub fn insert_acp_run(
        &self,
        run_id: &str,
        parent_session_id: &str,
        backend_id: &str,
        task: &str,
        label: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO acp_runs (run_id, parent_session_id, backend_id, task, status, started_at, label)
             VALUES (?1, ?2, ?3, ?4, 'starting', ?5, ?6)",
            params![run_id, parent_session_id, backend_id, task, now, label],
        )?;
        Ok(())
    }

    /// Update an ACP run's status, PID, and external session ID when it starts running.
    pub fn update_acp_run_status(
        &self,
        run_id: &str,
        status: &str,
        pid: Option<u32>,
        external_session_id: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE acp_runs SET status = ?1, pid = COALESCE(?2, pid),
                external_session_id = COALESCE(?3, external_session_id)
             WHERE run_id = ?4",
            params![status, pid.map(|p| p as i64), external_session_id, run_id],
        )?;
        Ok(())
    }

    /// Finalize an ACP run (completed/error/timeout/killed).
    pub fn finish_acp_run(
        &self,
        run_id: &str,
        status: &str,
        result: Option<&str>,
        error: Option<&str>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();

        // Calculate duration from started_at
        let duration_ms: Option<i64> = conn
            .query_row(
                "SELECT started_at FROM acp_runs WHERE run_id = ?1",
                params![run_id],
                |row| {
                    let started_at: String = row.get(0)?;
                    if let Ok(started) = chrono::DateTime::parse_from_rfc3339(&started_at) {
                        let duration = chrono::Utc::now()
                            .signed_duration_since(started)
                            .num_milliseconds();
                        Ok(Some(duration))
                    } else {
                        Ok(None)
                    }
                },
            )
            .unwrap_or(None);

        conn.execute(
            "UPDATE acp_runs SET status = ?1, result = ?2, error = ?3,
                finished_at = ?4, duration_ms = ?5,
                input_tokens = COALESCE(?6, input_tokens),
                output_tokens = COALESCE(?7, output_tokens)
             WHERE run_id = ?8",
            params![
                status,
                result,
                error,
                now,
                duration_ms,
                input_tokens.map(|v| v as i64),
                output_tokens.map(|v| v as i64),
                run_id,
            ],
        )?;
        Ok(())
    }

    /// Get a single ACP run by ID.
    pub fn get_acp_run(&self, run_id: &str) -> Result<Option<AcpRun>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT run_id, parent_session_id, backend_id, external_session_id, task,
                    status, result, error, model_used, started_at, finished_at,
                    duration_ms, input_tokens, output_tokens, label, pid
             FROM acp_runs WHERE run_id = ?1",
        )?;

        let run = stmt
            .query_row(params![run_id], |row| Ok(row_to_acp_run(row)))
            .optional()?;

        Ok(run)
    }

    /// List ACP runs for a parent session.
    pub fn list_acp_runs(&self, parent_session_id: &str) -> Result<Vec<AcpRun>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT run_id, parent_session_id, backend_id, external_session_id, task,
                    status, result, error, model_used, started_at, finished_at,
                    duration_ms, input_tokens, output_tokens, label, pid
             FROM acp_runs WHERE parent_session_id = ?1
             ORDER BY started_at DESC",
        )?;

        let runs = stmt
            .query_map(params![parent_session_id], |row| Ok(row_to_acp_run(row)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(runs)
    }
}

/// Convert a rusqlite Row to AcpRun.
fn row_to_acp_run(row: &rusqlite::Row) -> AcpRun {
    let status_str: String = row.get::<_, String>(5).unwrap_or_default();
    let status = match status_str.as_str() {
        "starting" => AcpRunStatus::Starting,
        "running" => AcpRunStatus::Running,
        "completed" => AcpRunStatus::Completed,
        "error" => AcpRunStatus::Error,
        "timeout" => AcpRunStatus::Timeout,
        "killed" => AcpRunStatus::Killed,
        _ => AcpRunStatus::Error,
    };

    AcpRun {
        run_id: row.get(0).unwrap_or_default(),
        parent_session_id: row.get(1).unwrap_or_default(),
        backend_id: row.get(2).unwrap_or_default(),
        external_session_id: row.get(3).ok(),
        task: row.get(4).unwrap_or_default(),
        status,
        result: row.get(6).ok(),
        error: row.get(7).ok(),
        model_used: row.get(8).ok(),
        started_at: row.get(9).unwrap_or_default(),
        finished_at: row.get(10).ok(),
        duration_ms: row
            .get::<_, Option<i64>>(11)
            .ok()
            .flatten()
            .map(|v| v as u64),
        input_tokens: row
            .get::<_, Option<i64>>(12)
            .ok()
            .flatten()
            .map(|v| v as u64),
        output_tokens: row
            .get::<_, Option<i64>>(13)
            .ok()
            .flatten()
            .map(|v| v as u64),
        label: row.get(14).ok(),
        pid: row
            .get::<_, Option<i64>>(15)
            .ok()
            .flatten()
            .map(|v| v as u32),
    }
}

use rusqlite::OptionalExtension;
