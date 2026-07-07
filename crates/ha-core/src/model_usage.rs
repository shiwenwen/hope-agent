//! Unified model usage ledger.
//!
//! The dashboard's token totals are sourced from this append-only table rather
//! than from chat messages, because model calls also happen in side queries,
//! embeddings, STT, recap, knowledge maintenance, permission judging, and other
//! background paths.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use crate::session::SessionDB;

pub const KIND_CHAT: &str = "chat";
pub const KIND_SIDE_QUERY: &str = "side_query";
pub const KIND_EMBEDDING: &str = "embedding";
pub const KIND_STT: &str = "stt";
pub const KIND_JUDGE: &str = "judge";
pub const KIND_SUMMARIZE: &str = "summarize";
pub const KIND_WEB_SEARCH: &str = "web_search";
pub const KIND_IMAGE_GENERATION: &str = "image_generation";
pub const KIND_PROVIDER_TEST: &str = "provider_test";

#[derive(Debug, Clone, Default)]
pub struct ModelUsageEvent {
    /// Optional idempotency key. Use stable keys when mirroring durable rows
    /// such as chat messages; transient one-shot calls may leave this `None`.
    pub request_key: Option<String>,
    pub timestamp: Option<String>,
    pub kind: String,
    pub operation: Option<String>,
    pub source: Option<String>,
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    pub model_id: Option<String>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub duration_ms: Option<u64>,
    pub ttft_ms: Option<u64>,
    pub success: bool,
    pub error: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

impl ModelUsageEvent {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            success: true,
            ..Default::default()
        }
    }

    pub fn with_usage(
        mut self,
        input_tokens: u64,
        output_tokens: u64,
        cache_creation_input_tokens: u64,
        cache_read_input_tokens: u64,
    ) -> Self {
        self.input_tokens = Some(input_tokens);
        self.output_tokens = Some(output_tokens);
        self.cache_creation_input_tokens = Some(cache_creation_input_tokens);
        self.cache_read_input_tokens = Some(cache_read_input_tokens);
        self
    }
}

fn opt_i64(v: Option<u64>) -> Option<i64> {
    v.map(|n| n.min(i64::MAX as u64) as i64)
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Convert a Rust caller location into a stable-enough operation tag.
pub fn caller_operation(location: &'static std::panic::Location<'static>) -> String {
    let file = location.file();
    let file = file
        .split_once("/crates/")
        .map(|(_, rest)| format!("crates/{rest}"))
        .or_else(|| {
            file.split_once("/src/")
                .map(|(_, rest)| format!("src/{rest}"))
        })
        .unwrap_or_else(|| file.to_string());
    format!("{}:{}", file, location.line())
}

impl SessionDB {
    pub(crate) fn ensure_model_usage_table(conn: &rusqlite::Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS model_usage_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                request_key TEXT UNIQUE,
                timestamp TEXT NOT NULL,
                kind TEXT NOT NULL,
                operation TEXT,
                source TEXT,
                provider_id TEXT,
                provider_name TEXT,
                model_id TEXT,
                session_id TEXT,
                agent_id TEXT,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_creation_input_tokens INTEGER,
                cache_read_input_tokens INTEGER,
                duration_ms INTEGER,
                ttft_ms INTEGER,
                success INTEGER NOT NULL DEFAULT 1,
                error TEXT,
                metadata TEXT,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_model_usage_timestamp ON model_usage_events(timestamp);
             CREATE INDEX IF NOT EXISTS idx_model_usage_kind_timestamp ON model_usage_events(kind, timestamp);
             CREATE INDEX IF NOT EXISTS idx_model_usage_session ON model_usage_events(session_id);
             CREATE INDEX IF NOT EXISTS idx_model_usage_provider_model ON model_usage_events(provider_id, model_id);",
        )?;
        Ok(())
    }

    pub(crate) fn backfill_model_usage_from_messages(conn: &rusqlite::Connection) -> Result<()> {
        conn.execute(
            "INSERT OR IGNORE INTO model_usage_events (
                request_key, timestamp, kind, operation, source,
                provider_id, provider_name, model_id, session_id, agent_id,
                input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens,
                duration_ms, ttft_ms, success
             )
             SELECT
                'message:' || m.id,
                m.timestamp,
                'chat',
                'chat',
                COALESCE(m.source, 'desktop'),
                s.provider_id,
                s.provider_name,
                COALESCE(m.model, s.model_id),
                m.session_id,
                s.agent_id,
                m.tokens_in,
                m.tokens_out,
                m.tokens_cache_creation,
                m.tokens_cache_read,
                m.tool_duration_ms,
                m.ttft_ms,
                CASE WHEN COALESCE(m.is_error, 0) = 0 THEN 1 ELSE 0 END
             FROM messages m
             JOIN sessions s ON s.id = m.session_id
             WHERE m.role = 'assistant'
               AND COALESCE(s.incognito, 0) = 0
               AND (
                    m.tokens_in IS NOT NULL
                 OR m.tokens_out IS NOT NULL
                 OR m.tokens_cache_creation IS NOT NULL
                 OR m.tokens_cache_read IS NOT NULL
               )",
            [],
        )?;
        Ok(())
    }

    pub fn insert_model_usage_event(&self, event: &ModelUsageEvent) -> Result<bool> {
        if event.kind.trim().is_empty() {
            return Ok(false);
        }

        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        if let Some(session_id) = event.session_id.as_deref() {
            let Some(incognito) = conn
                .query_row(
                    "SELECT incognito FROM sessions WHERE id = ?1",
                    params![session_id],
                    |r| r.get::<_, i64>(0),
                )
                .optional()?
            else {
                return Ok(false);
            };
            if incognito != 0 {
                return Ok(false);
            }
        }

        let metadata = event
            .metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let timestamp = event.timestamp.clone().unwrap_or_else(now_rfc3339);
        let changed = conn.execute(
            "INSERT OR IGNORE INTO model_usage_events (
                request_key, timestamp, kind, operation, source,
                provider_id, provider_name, model_id, session_id, agent_id,
                input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens,
                duration_ms, ttft_ms, success, error, metadata
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                event.request_key.as_deref(),
                timestamp.as_str(),
                event.kind.as_str(),
                event.operation.as_deref(),
                event.source.as_deref(),
                event.provider_id.as_deref(),
                event.provider_name.as_deref(),
                event.model_id.as_deref(),
                event.session_id.as_deref(),
                event.agent_id.as_deref(),
                opt_i64(event.input_tokens),
                opt_i64(event.output_tokens),
                opt_i64(event.cache_creation_input_tokens),
                opt_i64(event.cache_read_input_tokens),
                opt_i64(event.duration_ms),
                opt_i64(event.ttft_ms),
                if event.success { 1 } else { 0 },
                event.error.as_deref(),
                metadata.as_deref(),
            ],
        )?;
        Ok(changed > 0)
    }
}

pub fn record_model_usage_best_effort(event: ModelUsageEvent) {
    match crate::get_session_db() {
        Some(db) => {
            if let Err(e) = db.insert_model_usage_event(&event) {
                app_warn!(
                    "model_usage",
                    "record",
                    "failed to record model usage: {}",
                    e
                );
            }
        }
        None => {
            app_warn!(
                "model_usage",
                "record",
                "session db unavailable; dropping model usage event kind={}",
                event.kind
            );
        }
    }
}
