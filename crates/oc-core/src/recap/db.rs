use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::Result;
use rusqlite::{params, Connection};

use super::types::{RecapReport, RecapReportSummary, SessionFacet, RECAP_SCHEMA_VERSION};

pub struct RecapDb {
    conn: Mutex<Connection>,
}

impl RecapDb {
    /// Open or create the recap database at the given path.
    pub fn open(db_path: &PathBuf) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS session_facets (
                session_id TEXT PRIMARY KEY,
                last_message_ts TEXT NOT NULL,
                message_count INTEGER NOT NULL,
                analysis_model TEXT NOT NULL,
                facet_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                schema_version INTEGER NOT NULL DEFAULT 1
            );
            CREATE INDEX IF NOT EXISTS idx_facets_ts ON session_facets(last_message_ts);

            CREATE TABLE IF NOT EXISTS recap_reports (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                range_start TEXT NOT NULL,
                range_end TEXT NOT NULL,
                filters_json TEXT NOT NULL,
                report_json TEXT NOT NULL,
                html_path TEXT,
                session_count INTEGER NOT NULL,
                generated_at TEXT NOT NULL,
                analysis_model TEXT NOT NULL,
                schema_version INTEGER NOT NULL DEFAULT 1
            );
            CREATE INDEX IF NOT EXISTS idx_reports_generated ON recap_reports(generated_at DESC);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_default() -> Result<Self> {
        let path = crate::paths::recap_db_path()?;
        Self::open(&path)
    }

    // ── Session facets ────────────────────────────────────────────

    /// Look up cached facet. Returns Some only when the cache is valid for
    /// the given (last_message_ts, analysis_model, schema_version).
    pub fn get_cached_facet(
        &self,
        session_id: &str,
        last_message_ts: &str,
        analysis_model: &str,
    ) -> Result<Option<SessionFacet>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT facet_json FROM session_facets
             WHERE session_id = ?1
               AND last_message_ts = ?2
               AND analysis_model = ?3
               AND schema_version = ?4",
        )?;
        let mut rows = stmt.query(params![
            session_id,
            last_message_ts,
            analysis_model,
            RECAP_SCHEMA_VERSION as i64
        ])?;
        if let Some(row) = rows.next()? {
            let json: String = row.get(0)?;
            let facet: SessionFacet = serde_json::from_str(&json)?;
            return Ok(Some(facet));
        }
        Ok(None)
    }

    /// Return the most recent cached facet for a session, *ignoring* cache
    /// validity checks. Intended for cross-session awareness where we want a
    /// best-effort enrichment and will never trigger a new LLM extraction.
    pub fn get_latest_facet(&self, session_id: &str) -> Result<Option<SessionFacet>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT facet_json FROM session_facets
             WHERE session_id = ?1
             ORDER BY last_message_ts DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![session_id])?;
        if let Some(row) = rows.next()? {
            let json: String = row.get(0)?;
            let facet: SessionFacet = serde_json::from_str(&json)?;
            return Ok(Some(facet));
        }
        Ok(None)
    }

    /// Upsert a facet record.
    pub fn save_facet(
        &self,
        facet: &SessionFacet,
        last_message_ts: &str,
        message_count: i64,
        analysis_model: &str,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let json = serde_json::to_string(facet)?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO session_facets
                 (session_id, last_message_ts, message_count, analysis_model, facet_json, created_at, schema_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(session_id) DO UPDATE SET
                 last_message_ts = excluded.last_message_ts,
                 message_count   = excluded.message_count,
                 analysis_model  = excluded.analysis_model,
                 facet_json      = excluded.facet_json,
                 created_at      = excluded.created_at,
                 schema_version  = excluded.schema_version",
            params![
                facet.session_id,
                last_message_ts,
                message_count,
                analysis_model,
                json,
                now,
                RECAP_SCHEMA_VERSION as i64
            ],
        )?;
        Ok(())
    }

    /// Garbage collect facets older than `retention_days`.
    pub fn purge_old_facets(&self, retention_days: u32) -> Result<u64> {
        if retention_days == 0 {
            return Ok(0);
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let cutoff =
            (chrono::Utc::now() - chrono::Duration::days(retention_days as i64)).to_rfc3339();
        let n = conn.execute(
            "DELETE FROM session_facets WHERE created_at < ?1",
            params![cutoff],
        )?;
        Ok(n as u64)
    }

    // ── Reports ───────────────────────────────────────────────────

    /// Persist a generated report.
    pub fn save_report(&self, report: &RecapReport) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let report_json = serde_json::to_string(report)?;
        let filters_json = serde_json::to_string(&report.meta.filters)?;
        conn.execute(
            "INSERT INTO recap_reports
                 (id, title, range_start, range_end, filters_json, report_json,
                  html_path, session_count, generated_at, analysis_model, schema_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
                 title           = excluded.title,
                 range_start     = excluded.range_start,
                 range_end       = excluded.range_end,
                 filters_json    = excluded.filters_json,
                 report_json     = excluded.report_json,
                 session_count   = excluded.session_count,
                 generated_at    = excluded.generated_at,
                 analysis_model  = excluded.analysis_model,
                 schema_version  = excluded.schema_version",
            params![
                report.meta.id,
                report.meta.title,
                report.meta.range_start,
                report.meta.range_end,
                filters_json,
                report_json,
                report.meta.session_count as i64,
                report.meta.generated_at,
                report.meta.analysis_model,
                RECAP_SCHEMA_VERSION as i64
            ],
        )?;
        Ok(())
    }

    pub fn set_html_path(&self, report_id: &str, html_path: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE recap_reports SET html_path = ?1 WHERE id = ?2",
            params![html_path, report_id],
        )?;
        Ok(())
    }

    pub fn get_report(&self, report_id: &str) -> Result<Option<RecapReport>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare("SELECT report_json FROM recap_reports WHERE id = ?1")?;
        let mut rows = stmt.query(params![report_id])?;
        if let Some(row) = rows.next()? {
            let json: String = row.get(0)?;
            let report: RecapReport = serde_json::from_str(&json)?;
            return Ok(Some(report));
        }
        Ok(None)
    }

    pub fn list_reports(&self, limit: u32) -> Result<Vec<RecapReportSummary>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, title, range_start, range_end, session_count, generated_at,
                    analysis_model, html_path
             FROM recap_reports
             ORDER BY generated_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(RecapReportSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                range_start: row.get(2)?,
                range_end: row.get(3)?,
                session_count: row.get::<_, i64>(4)? as u32,
                generated_at: row.get(5)?,
                analysis_model: row.get(6)?,
                html_path: row.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn delete_report(&self, report_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "DELETE FROM recap_reports WHERE id = ?1",
            params![report_id],
        )?;
        Ok(())
    }

    /// Return the `range_end` of the most-recent report (RFC3339), if any.
    pub fn latest_report_range_end(&self) -> Result<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT range_end FROM recap_reports ORDER BY generated_at DESC LIMIT 1",
        )?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let s: String = row.get(0)?;
            return Ok(Some(s));
        }
        Ok(None)
    }
}
