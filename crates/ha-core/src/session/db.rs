use anyhow::Result;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

use super::types::{
    ChannelSessionInfo, MessageRole, NewMessage, SessionKind, SessionMemoryPolicy,
    SessionMemoryPolicyValue, SessionMessage, SessionMeta,
};

/// Token snapshot for the latest persisted assistant row of a session.
/// Powers `/status`'s Context usage + Cache info panels in a single read.
#[derive(Debug, Clone, Default)]
pub struct LastAssistantTokens {
    /// Cumulative input tokens for the turn (legacy fallback for context fill).
    pub tokens_in: Option<i64>,
    /// Last-round input tokens — preferred for context-window fill.
    pub tokens_in_last: Option<i64>,
    /// Cache-write tokens for the most recent API round in the turn.
    pub tokens_cache_creation: Option<i64>,
    /// Cache-read tokens for the most recent API round in the turn.
    pub tokens_cache_read: Option<i64>,
    /// Model id stamped on the row (for context-window lookup when the
    /// active selection has changed mid-session).
    pub model: Option<String>,
}

// ── Database Manager ─────────────────────────────────────────────

/// Number of read-only connections in the pool (mirrors the memory backend).
const READ_POOL_SIZE: usize = 4;

pub struct SessionDB {
    /// Exclusive write connection — also the *only* connection used by every
    /// write path, read-write transaction, and external module that touches
    /// `.conn` directly (channel/db.rs, session/tasks.rs, session_title.rs,
    /// agent/migration.rs). Migrations run here at open().
    pub(crate) conn: Mutex<Connection>,
    /// Pool of READ_ONLY connections for the hottest pure-read methods
    /// (message loads, FTS search, sidebar list). With WAL these run
    /// concurrently with the writer instead of serializing on one mutex, so a
    /// streaming write no longer blocks the UI's sidebar/message reads.
    readers: Vec<Mutex<Connection>>,
    /// Round-robin cursor into `readers`.
    reader_idx: AtomicUsize,
}

/// Log at `app_info!` when `align_window_to_user_boundary` extends a page by
/// more than this many rows. Signals that a single user turn spans far more
/// DB rows than the requested page size — useful for tuning PAGE_SIZE and
/// deciding when virtual scrolling becomes necessary.
const LARGE_TURN_EXTENSION_LOG_THRESHOLD: usize = 200;

/// Shared SELECT for every query that hydrates a full `SessionMeta`. Column
/// positions are locked to the parser in `SessionDB::row_to_session_meta`;
/// when adding a column, append it and update both the mapper and tests.
///
/// The `unread_count` / `channel_unread_count` predicate stacks below are
/// mirrored by the project-level rollup in `project::db::ProjectDB::list`
/// (cron / subagent / source exclusions). Keep the two in sync — a divergence
/// silently desyncs the per-session badge from the project badge. The
/// `parent_session_id IS NULL` clause in the channel subquery is defensive
/// (a sub-agent session never carries `source='channel'` rows, but it must
/// never surface either count — asserted by the db tests).
const SESSION_META_SELECT: &str = "SELECT s.id, s.title, s.agent_id, s.provider_id, s.provider_name, s.model_id,
           s.created_at, s.updated_at,
           (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) as msg_count,
           (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id AND s.is_cron = 0 AND s.parent_session_id IS NULL AND m.id > COALESCE(s.last_read_message_id, 0) AND m.role = 'assistant' AND COALESCE(m.source, 'desktop') != 'channel') as unread_count,
           EXISTS(
             SELECT 1 FROM messages m
             WHERE m.session_id = s.id
               AND m.id = (SELECT MAX(m2.id) FROM messages m2 WHERE m2.session_id = s.id)
               AND m.is_error = 1
           ) as has_error,
           s.is_cron, s.parent_session_id, s.plan_mode, s.project_id, s.permission_mode, s.incognito,
           cc.channel_id, cc.account_id, cc.chat_id, cc.chat_type, cc.sender_name,
           s.working_dir, s.title_source, s.reasoning_effort, s.pinned_at, s.kind,
           (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id AND s.is_cron = 0 AND s.parent_session_id IS NULL AND m.id > COALESCE(s.last_read_message_id, 0) AND m.role = 'assistant' AND COALESCE(m.source, 'desktop') = 'channel') as channel_unread_count,
           s.sandbox_mode, s.temperature, s.runtime_defaults_initialized,
           s.execution_mode, s.workflow_mode,
           s.forked_from_session_id, s.forked_from_message_id,
           (SELECT p.title FROM sessions p WHERE p.id = s.forked_from_session_id) as forked_from_session_title
     FROM sessions s
     LEFT JOIN channel_conversations cc ON cc.session_id = s.id";

impl SessionDB {
    /// Open (or create) the database at the given path, enable WAL mode,
    /// and ensure tables exist.
    pub fn open(db_path: &PathBuf) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;

        // Enable WAL mode for crash safety and better concurrent read performance
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        // Wait up to 5s on a busy lock instead of returning SQLITE_BUSY
        // immediately — removes spurious write failures under WAL contention.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        // Create tables
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                title TEXT,
                agent_id TEXT NOT NULL DEFAULT 'ha-main',
                provider_id TEXT,
                provider_name TEXT,
                model_id TEXT,
                temperature REAL,
                runtime_defaults_initialized INTEGER NOT NULL DEFAULT 0,
                reasoning_effort TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                context_json TEXT,
                last_read_message_id INTEGER DEFAULT 0,
                is_cron INTEGER NOT NULL DEFAULT 0,
                parent_session_id TEXT,
                incognito INTEGER NOT NULL DEFAULT 0,
                title_source TEXT NOT NULL DEFAULT 'manual',
                pinned_at TEXT,
                kind TEXT NOT NULL DEFAULT 'regular',
                execution_mode TEXT NOT NULL DEFAULT 'off',
                workflow_mode TEXT NOT NULL DEFAULT 'off',
                forked_from_session_id TEXT,
                forked_from_message_id INTEGER
            );

            -- Design-space per-project chat threads. Binds a `kind='design'`
            -- session to the design project it iterates on. Truth source in
            -- sessions.db (JOINs sessions/messages for the history picker);
            -- cascades on session delete. `project_id` is a plain column (the
            -- design project row lives in the separate design.db, so no FK) —
            -- design-project deletion tears these sessions down explicitly.
            CREATE TABLE IF NOT EXISTS design_chat_threads (
                session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                project_id TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_design_chat_threads_project
                ON design_chat_threads(project_id, created_at DESC);

            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL DEFAULT '',
                timestamp TEXT NOT NULL,
                attachments_meta TEXT,
                model TEXT,
                tokens_in INTEGER,
                tokens_out INTEGER,
                reasoning_effort TEXT,
                tool_call_id TEXT,
                tool_name TEXT,
                tool_arguments TEXT,
                tool_result TEXT,
                tool_duration_ms INTEGER,
                is_error INTEGER DEFAULT 0,
                ttft_ms INTEGER,
                tokens_in_last INTEGER,
                tokens_cache_creation INTEGER,
                tokens_cache_read INTEGER,
                tool_metadata TEXT,
                source TEXT,
                queue_request_id TEXT,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_messages_session_id ON messages(session_id);
            -- Composite index for role-filtered scans within a session
            -- (last assistant token row, future last-user-message lookups).
            CREATE INDEX IF NOT EXISTS idx_messages_session_role ON messages(session_id, role);
            CREATE INDEX IF NOT EXISTS idx_sessions_agent_id ON sessions(agent_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_updated_at ON sessions(updated_at DESC);

            -- Sub-agent runs
            CREATE TABLE IF NOT EXISTS subagent_runs (
                run_id TEXT PRIMARY KEY,
                parent_session_id TEXT NOT NULL,
                parent_agent_id TEXT NOT NULL,
                child_agent_id TEXT NOT NULL,
                child_session_id TEXT NOT NULL,
                task TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'spawning',
                result TEXT,
                error TEXT,
                depth INTEGER NOT NULL DEFAULT 1,
                model_used TEXT,
                started_at TEXT NOT NULL,
                finished_at TEXT,
                duration_ms INTEGER,
                label TEXT,
                attachment_count INTEGER DEFAULT 0,
                input_tokens INTEGER,
                output_tokens INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_subagent_parent ON subagent_runs(parent_session_id, started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_subagent_status ON subagent_runs(status);
            CREATE INDEX IF NOT EXISTS idx_subagent_label ON subagent_runs(label);

            -- Unified model usage ledger. Dashboard token/cost totals read this
            -- table so non-chat model calls (side_query / embedding / STT /
            -- judge / maintenance) are counted alongside normal chat turns.
            CREATE TABLE IF NOT EXISTS model_usage_events (
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
            CREATE INDEX IF NOT EXISTS idx_model_usage_provider_model ON model_usage_events(provider_id, model_id);

            -- FTS5 full-text search for message history
            CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                content,
                content='messages',
                content_rowid='id',
                tokenize='unicode61'
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS messages_trigram_fts USING fts5(
                content,
                content='messages',
                content_rowid='id',
                tokenize='trigram'
            );

            -- Triggers for automatic FTS sync (only user/assistant messages)
            CREATE TRIGGER IF NOT EXISTS messages_fts_ai AFTER INSERT ON messages
            WHEN new.role IN ('user', 'assistant') AND length(new.content) > 0
            BEGIN
                INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
            END;

            CREATE TRIGGER IF NOT EXISTS messages_fts_ad AFTER DELETE ON messages
            WHEN old.role IN ('user', 'assistant') AND length(old.content) > 0
            BEGIN
                INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
            END;

            CREATE TRIGGER IF NOT EXISTS messages_fts_au AFTER UPDATE OF content, role ON messages
            BEGIN
                INSERT INTO messages_fts(messages_fts, rowid, content)
                    SELECT 'delete', old.id, old.content
                    WHERE old.role IN ('user', 'assistant') AND length(old.content) > 0;
                INSERT INTO messages_fts(rowid, content)
                    SELECT new.id, new.content
                    WHERE new.role IN ('user', 'assistant') AND length(new.content) > 0;
            END;

            CREATE TRIGGER IF NOT EXISTS messages_trigram_fts_ai AFTER INSERT ON messages
            WHEN new.role IN ('user', 'assistant') AND length(new.content) > 0
            BEGIN
                INSERT INTO messages_trigram_fts(rowid, content) VALUES (new.id, new.content);
            END;

            CREATE TRIGGER IF NOT EXISTS messages_trigram_fts_ad AFTER DELETE ON messages
            WHEN old.role IN ('user', 'assistant') AND length(old.content) > 0
            BEGIN
                INSERT INTO messages_trigram_fts(messages_trigram_fts, rowid, content) VALUES('delete', old.id, old.content);
            END;

            CREATE TRIGGER IF NOT EXISTS messages_trigram_fts_au AFTER UPDATE OF content, role ON messages
            BEGIN
                INSERT INTO messages_trigram_fts(messages_trigram_fts, rowid, content)
                    SELECT 'delete', old.id, old.content
                    WHERE old.role IN ('user', 'assistant') AND length(old.content) > 0;
                INSERT INTO messages_trigram_fts(rowid, content)
                    SELECT new.id, new.content
                    WHERE new.role IN ('user', 'assistant') AND length(new.content) > 0;
            END;"
        )?;

        // Migration: add is_cron column if missing
        let has_is_cron = conn.prepare("SELECT is_cron FROM sessions LIMIT 1").is_ok();
        if !has_is_cron {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN is_cron INTEGER NOT NULL DEFAULT 0;",
            )?;
        }

        // Migration: add thinking column to messages if missing
        let has_thinking = conn
            .prepare("SELECT thinking FROM messages LIMIT 1")
            .is_ok();
        if !has_thinking {
            conn.execute_batch("ALTER TABLE messages ADD COLUMN thinking TEXT;")?;
        }

        // Migration: create acp_runs table if missing
        let has_acp_runs = conn.prepare("SELECT run_id FROM acp_runs LIMIT 1").is_ok();
        if !has_acp_runs {
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
        }

        // Migration: add ttft_ms column to messages if missing
        let has_ttft_ms = conn.prepare("SELECT ttft_ms FROM messages LIMIT 1").is_ok();
        if !has_ttft_ms {
            conn.execute_batch("ALTER TABLE messages ADD COLUMN ttft_ms INTEGER;")?;
        }

        // Migration: add tokens_in_last column if missing. See
        // ChatUsage::last_input_tokens for the billing-vs-UI split.
        let has_tokens_in_last = conn
            .prepare("SELECT tokens_in_last FROM messages LIMIT 1")
            .is_ok();
        if !has_tokens_in_last {
            conn.execute_batch("ALTER TABLE messages ADD COLUMN tokens_in_last INTEGER;")?;
        }

        // Migration: persist cache-token counts so they survive session reload.
        let has_tokens_cache_creation = conn
            .prepare("SELECT tokens_cache_creation FROM messages LIMIT 1")
            .is_ok();
        if !has_tokens_cache_creation {
            conn.execute_batch("ALTER TABLE messages ADD COLUMN tokens_cache_creation INTEGER;")?;
        }
        let has_tokens_cache_read = conn
            .prepare("SELECT tokens_cache_read FROM messages LIMIT 1")
            .is_ok();
        if !has_tokens_cache_read {
            conn.execute_batch("ALTER TABLE messages ADD COLUMN tokens_cache_read INTEGER;")?;
        }

        // Migration: structured tool side-output metadata (file change diffs,
        // line deltas, etc.) consumed by the right-side diff panel and the
        // tool-call header `+N -M` summary. Older rows stay NULL and the
        // frontend falls back to its pre-diff-panel rendering.
        let has_tool_metadata = conn
            .prepare("SELECT tool_metadata FROM messages LIMIT 1")
            .is_ok();
        if !has_tool_metadata {
            conn.execute_batch("ALTER TABLE messages ADD COLUMN tool_metadata TEXT;")?;
        }

        // Migration: streaming-state column for crash-resilient placeholder
        // rows. `streaming` = being written by an active turn; `completed` =
        // finalized cleanly; `orphaned` = startup sweep saw a leftover
        // streaming row from a previous (crashed) run and has not finalized it
        // yet; `recovered` = startup finalize already preserved that partial.
        // NULL is the legacy value for rows written before this column existed
        // and is treated as `completed` by all readers.
        let has_stream_status = conn
            .prepare("SELECT stream_status FROM messages LIMIT 1")
            .is_ok();
        if !has_stream_status {
            conn.execute_batch(
                "ALTER TABLE messages ADD COLUMN stream_status TEXT;
                 CREATE INDEX IF NOT EXISTS idx_messages_stream_active
                   ON messages(session_id, stream_status)
                   WHERE stream_status = 'streaming';",
            )?;
        }

        // Migration: lowercase `ChatSource` of the caller that drove this turn
        // (`desktop` / `http` / `channel` / `subagent` / `parent_injection` /
        // `cron`). Drives the IM-vs-desktop unread-badge split in `unread_count`
        // and the GUI→IM mirror's user-quote prefix. Legacy rows stay NULL and
        // are treated as `desktop` by readers (`COALESCE(source, 'desktop')`),
        // preserving any existing unread badges. NOTE: cron-session unread is
        // suppressed by `s.is_cron = 0` in the unread subquery, NOT by the source
        // string — don't rely on `source != 'channel'` to hide cron rows.
        let has_source = conn.prepare("SELECT source FROM messages LIMIT 1").is_ok();
        if !has_source {
            conn.execute_batch("ALTER TABLE messages ADD COLUMN source TEXT;")?;
        }

        // Migration: user-facing session forks. Deliberately separate from
        // `parent_session_id`, which marks hidden sub-agent child sessions.
        let has_forked_from_session_id = conn
            .prepare("SELECT forked_from_session_id FROM sessions LIMIT 1")
            .is_ok();
        if !has_forked_from_session_id {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN forked_from_session_id TEXT;")?;
        }
        let has_forked_from_message_id = conn
            .prepare("SELECT forked_from_message_id FROM sessions LIMIT 1")
            .is_ok();
        if !has_forked_from_message_id {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN forked_from_message_id INTEGER;")?;
        }
        // Keep this index after both ALTER migrations. `CREATE TABLE IF NOT
        // EXISTS` does not add columns to an existing database, so creating the
        // index in the bootstrap batch would make old databases fail before
        // they can reach these migrations.
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_sessions_forked_from ON sessions(forked_from_session_id);",
        )?;

        Self::ensure_model_usage_table(&conn)?;
        const SCHEMA_FLAG_MODEL_USAGE_BACKFILLED: i64 = 0x4;
        let schema_flags: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if schema_flags & SCHEMA_FLAG_MODEL_USAGE_BACKFILLED == 0 {
            Self::backfill_model_usage_from_messages(&conn)?;
            conn.execute_batch(&format!(
                "PRAGMA user_version = {};",
                schema_flags | SCHEMA_FLAG_MODEL_USAGE_BACKFILLED
            ))?;
        }
        Self::ensure_chat_turns_table(&conn)?;
        let has_queue_request_id = conn
            .prepare("SELECT queue_request_id FROM messages LIMIT 1")
            .is_ok();
        if !has_queue_request_id {
            conn.execute_batch("ALTER TABLE messages ADD COLUMN queue_request_id TEXT;")?;
        }
        conn.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_queue_request_id
             ON messages(queue_request_id) WHERE queue_request_id IS NOT NULL;",
        )?;
        Self::ensure_turn_message_queue_table(&conn)?;
        Self::recover_turn_message_queue(&conn)?;
        crate::goal::ensure_tables(&conn)?;
        crate::worktree::ensure_tables(&conn)?;
        crate::project_bootstrap::ensure_tables(&conn)?;
        crate::git_control::ensure_tables(&conn)?;
        crate::workflow::ensure_tables(&conn)?;
        crate::review::ensure_tables(&conn)?;
        crate::verification::ensure_tables(&conn)?;
        crate::loop_control::ensure_tables(&conn)?;
        crate::coding_improvement::ensure_tables(&conn)?;
        crate::domain_workflow::ensure_tables(&conn)?;
        crate::domain_quality::ensure_tables(&conn)?;
        crate::domain_eval::ensure_tables(&conn)?;

        // Migration: fix FTS delete trigger — must match INSERT trigger's WHEN clause
        // to avoid "database disk image is malformed" errors during CASCADE delete.
        // The old trigger fired for ALL messages but only user/assistant were indexed.
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS messages_fts_ad;
             CREATE TRIGGER messages_fts_ad AFTER DELETE ON messages
             WHEN old.role IN ('user', 'assistant') AND length(old.content) > 0
             BEGIN
                 INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
             END;
             DROP TRIGGER IF EXISTS messages_fts_au;
             CREATE TRIGGER messages_fts_au AFTER UPDATE OF content, role ON messages
             BEGIN
                 INSERT INTO messages_fts(messages_fts, rowid, content)
                     SELECT 'delete', old.id, old.content
                     WHERE old.role IN ('user', 'assistant') AND length(old.content) > 0;
                 INSERT INTO messages_fts(rowid, content)
                     SELECT new.id, new.content
                     WHERE new.role IN ('user', 'assistant') AND length(new.content) > 0;
             END;
             CREATE VIRTUAL TABLE IF NOT EXISTS messages_trigram_fts USING fts5(
                 content,
                 content='messages',
                 content_rowid='id',
                 tokenize='trigram'
             );
             DROP TRIGGER IF EXISTS messages_trigram_fts_ai;
             CREATE TRIGGER messages_trigram_fts_ai AFTER INSERT ON messages
             WHEN new.role IN ('user', 'assistant') AND length(new.content) > 0
             BEGIN
                 INSERT INTO messages_trigram_fts(rowid, content) VALUES (new.id, new.content);
             END;
             DROP TRIGGER IF EXISTS messages_trigram_fts_ad;
             CREATE TRIGGER messages_trigram_fts_ad AFTER DELETE ON messages
             WHEN old.role IN ('user', 'assistant') AND length(old.content) > 0
             BEGIN
                 INSERT INTO messages_trigram_fts(messages_trigram_fts, rowid, content) VALUES('delete', old.id, old.content);
             END;
             DROP TRIGGER IF EXISTS messages_trigram_fts_au;
             CREATE TRIGGER messages_trigram_fts_au AFTER UPDATE OF content, role ON messages
             BEGIN
                 INSERT INTO messages_trigram_fts(messages_trigram_fts, rowid, content)
                     SELECT 'delete', old.id, old.content
                     WHERE old.role IN ('user', 'assistant') AND length(old.content) > 0;
                 INSERT INTO messages_trigram_fts(rowid, content)
                     SELECT new.id, new.content
                     WHERE new.role IN ('user', 'assistant') AND length(new.content) > 0;
             END;"
        )?;

        // One-time FTS rebuild gate. The original unconditional rebuild fixed a
        // historical "database disk image is malformed" corruption, but it
        // re-scanned every `messages.content` row on *every* open — hundreds of
        // ms to seconds for heavy users, on the synchronous pre-window path.
        //
        // `PRAGMA user_version` is unused elsewhere here (all other migrations
        // are probe-based `SELECT col ... is_ok()`), so we claim it as a bitflag
        // sentinel: bit 0 = "FTS rebuild already run"; bit 1 = "trigram FTS
        // rebuild already run"; bit 2 = "model usage message backfill already
        // run". Future schema versioning can use the remaining bits. The
        // corruption-recovery rebuild in
        // `delete_session` (the only other rebuild caller) is unaffected — it
        // fires on a caught error, not on open.
        const SCHEMA_FLAG_FTS_REBUILT: i64 = 0x1;
        const SCHEMA_FLAG_TRIGRAM_FTS_REBUILT: i64 = 0x2;
        let schema_flags: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let mut next_schema_flags = schema_flags;
        if next_schema_flags & SCHEMA_FLAG_FTS_REBUILT == 0 {
            // Stamp the sentinel ONLY when the rebuild actually succeeded — if a
            // first post-upgrade open hits the very corruption this is meant to
            // heal and the rebuild errors, we must retry on the next open rather
            // than permanently skip it. PRAGMA values can't be parameterized
            // (integer literal in SQL text); the value is a private const.
            if conn
                .execute_batch("INSERT INTO messages_fts(messages_fts) VALUES('rebuild');")
                .is_ok()
            {
                next_schema_flags |= SCHEMA_FLAG_FTS_REBUILT;
            }
        }
        if next_schema_flags & SCHEMA_FLAG_TRIGRAM_FTS_REBUILT == 0 {
            if conn
                .execute_batch(
                    "INSERT INTO messages_trigram_fts(messages_trigram_fts) VALUES('rebuild');",
                )
                .is_ok()
            {
                next_schema_flags |= SCHEMA_FLAG_TRIGRAM_FTS_REBUILT;
            }
        }
        if next_schema_flags != schema_flags {
            conn.execute_batch(&format!("PRAGMA user_version = {};", next_schema_flags))?;
        }

        // Migration: add plan_mode column to sessions if missing
        let has_plan_mode = conn
            .prepare("SELECT plan_mode FROM sessions LIMIT 1")
            .is_ok();
        if !has_plan_mode {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN plan_mode TEXT DEFAULT 'off';")?;
        }

        // Migration: add plan_steps column for step progress persistence (crash recovery)
        let has_plan_steps = conn
            .prepare("SELECT plan_steps FROM sessions LIMIT 1")
            .is_ok();
        if !has_plan_steps {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN plan_steps TEXT;")?;
        }

        // Migration: persist `executing_started_at` so `maybe_complete_plan`
        // can scope tasks correctly across session-switch / app-restart. Plain
        // in-memory PlanMeta lost this stamp on `restore_from_db`, falling
        // back to the whole-session task view and letting pre-plan pending
        // tasks deadlock auto-complete.
        let has_plan_exec_started = conn
            .prepare("SELECT plan_executing_started_at FROM sessions LIMIT 1")
            .is_ok();
        if !has_plan_exec_started {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN plan_executing_started_at TEXT;")?;
        }

        // Migration: per-session tool_permission_mode so the chat input's
        // toggle (auto / ask_every_time / full_approve) is restored when the
        // user switches back to a historical session. See `SessionMeta`.
        let has_tool_perm_mode = conn
            .prepare("SELECT tool_permission_mode FROM sessions LIMIT 1")
            .is_ok();
        if !has_tool_perm_mode {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN tool_permission_mode TEXT NOT NULL DEFAULT 'auto';",
            )?;
        }

        // Migration: add project_id column for Project feature.
        let has_project_id = conn
            .prepare("SELECT project_id FROM sessions LIMIT 1")
            .is_ok();
        if !has_project_id {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN project_id TEXT;
                 CREATE INDEX IF NOT EXISTS idx_sessions_project_id ON sessions(project_id);",
            )?;
        }

        // Migration: add awareness_config_json column for per-session
        // override of the behavior awareness feature.
        let has_awareness_cfg = conn
            .prepare("SELECT awareness_config_json FROM sessions LIMIT 1")
            .is_ok();
        if !has_awareness_cfg {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN awareness_config_json TEXT;")?;
        }

        // Migration: per-session incognito mode for disabling passive memory /
        // awareness features and automatic memory extraction.
        let has_incognito = conn
            .prepare("SELECT incognito FROM sessions LIMIT 1")
            .is_ok();
        if !has_incognito {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN incognito INTEGER NOT NULL DEFAULT 0;",
            )?;
        }

        // Migration: per-session working directory for directing the model's
        // file operations. On server mode the path lives on the server's FS.
        let has_working_dir = conn
            .prepare("SELECT working_dir FROM sessions LIMIT 1")
            .is_ok();
        if !has_working_dir {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN working_dir TEXT;")?;
        }

        // Migration: per-session Think / reasoning effort override so the
        // chat input restores the user's choice after switching sessions.
        let has_reasoning_effort = conn
            .prepare("SELECT reasoning_effort FROM sessions LIMIT 1")
            .is_ok();
        if !has_reasoning_effort {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN reasoning_effort TEXT;")?;
        }

        // NULL temperature is a valid fixed provider-native default, so a
        // separate marker distinguishes it from an old row awaiting snapshot.
        let has_temperature = conn
            .prepare("SELECT temperature FROM sessions LIMIT 1")
            .is_ok();
        if !has_temperature {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN temperature REAL;")?;
        }
        let has_runtime_defaults_initialized = conn
            .prepare("SELECT runtime_defaults_initialized FROM sessions LIMIT 1")
            .is_ok();
        if !has_runtime_defaults_initialized {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN runtime_defaults_initialized INTEGER NOT NULL DEFAULT 0;",
            )?;
        }

        // Migration: track who last set the session title so automatic LLM
        // naming never overwrites user/manual titles.
        let has_title_source = conn
            .prepare("SELECT title_source FROM sessions LIMIT 1")
            .is_ok();
        if !has_title_source {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN title_source TEXT NOT NULL DEFAULT 'manual';",
            )?;
        }

        // Migration: permission system v2 — per-session permission mode
        // (default | smart | yolo) replacing the old `tool_permission_mode`
        // (auto | ask_every_time | full_approve). Clean break — old column
        // stays for forward-compat reads but is no longer the source of truth.
        let has_permission_mode = conn
            .prepare("SELECT permission_mode FROM sessions LIMIT 1")
            .is_ok();
        if !has_permission_mode {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN permission_mode TEXT NOT NULL DEFAULT 'default';",
            )?;
        }

        // Migration: per-session sandbox mode
        // (off | standard | isolated | workspace | trusted). Existing rows
        // default to `off`, then legacy agents whose default sandbox setting is
        // enabled are backfilled to that effective default so upgrades preserve
        // the old `capabilities.sandbox=true` behavior.
        let has_sandbox_mode = conn
            .prepare("SELECT sandbox_mode FROM sessions LIMIT 1")
            .is_ok();
        if !has_sandbox_mode {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN sandbox_mode TEXT NOT NULL DEFAULT 'off';",
            )?;
            let mut stmt = conn.prepare("SELECT DISTINCT agent_id FROM sessions")?;
            let agent_ids = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .filter_map(std::result::Result::ok)
                .collect::<Vec<_>>();
            drop(stmt);
            for agent_id in agent_ids {
                let mode = crate::agent_loader::load_agent(&agent_id)
                    .ok()
                    .map(|def| def.config.capabilities.effective_default_sandbox_mode())
                    .unwrap_or_default();
                if mode.enabled() {
                    conn.execute(
                        "UPDATE sessions SET sandbox_mode = ?1 WHERE agent_id = ?2",
                        params![mode.as_str(), agent_id],
                    )?;
                }
            }
        }

        // Migration: persistent per-session execution mode policy
        // (off | guarded | deep | autonomous). `/mode` writes this value and
        // the system prompt reads it on every chat entry point.
        let has_execution_mode = conn
            .prepare("SELECT execution_mode FROM sessions LIMIT 1")
            .is_ok();
        if !has_execution_mode {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN execution_mode TEXT NOT NULL DEFAULT 'off';",
            )?;
        }

        // Migration: persistent per-session Workflow Mode policy
        // (off | on | ultracode). `/workflow on` writes this value and the
        // system prompt/tool schema read it on every chat entry point.
        let has_workflow_mode = conn
            .prepare("SELECT workflow_mode FROM sessions LIMIT 1")
            .is_ok();
        if !has_workflow_mode {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN workflow_mode TEXT NOT NULL DEFAULT 'off';",
            )?;
        }

        // Migration: optional sidebar pin timestamp. NULL means unpinned;
        // non-NULL sorts above normal sessions without changing updated_at.
        let has_pinned_at = conn
            .prepare("SELECT pinned_at FROM sessions LIMIT 1")
            .is_ok();
        if !has_pinned_at {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN pinned_at TEXT;")?;
        }
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_sessions_pinned_at ON sessions(pinned_at DESC);",
        )?;

        // Migration: session classification (regular | knowledge). Knowledge
        // sessions are the knowledge-space sidebar conversations — persisted but
        // hidden from the main session list / picker. Probe-then-ALTER; fresh
        // DBs already have the column from CREATE TABLE above.
        let has_kind = conn.prepare("SELECT kind FROM sessions LIMIT 1").is_ok();
        if !has_kind {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN kind TEXT NOT NULL DEFAULT 'regular';",
            )?;
        }

        // Migration: pending ask_user_question groups for resume-after-restart.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS ask_user_questions (
                request_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                payload TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                timeout_at INTEGER,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                answered_at TEXT,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_ask_user_session ON ask_user_questions(session_id);
            CREATE INDEX IF NOT EXISTS idx_ask_user_status ON ask_user_questions(status);",
        )?;

        // Migration: session-scoped task management (TaskV2-style)
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                content TEXT NOT NULL,
                active_form TEXT,
                batch_id TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_tasks_session_id ON tasks(session_id);",
        )?;
        let has_active_form = conn
            .prepare("SELECT active_form FROM tasks LIMIT 1")
            .is_ok();
        if !has_active_form {
            conn.execute_batch("ALTER TABLE tasks ADD COLUMN active_form TEXT;")?;
        }
        let has_batch_id = conn.prepare("SELECT batch_id FROM tasks LIMIT 1").is_ok();
        if !has_batch_id {
            conn.execute_batch("ALTER TABLE tasks ADD COLUMN batch_id TEXT;")?;
        }

        // Migration: latest IDE / ACP context envelope for a session. This is
        // owner-plane state only: Review Engine and Context Retrieval may read
        // it, but it is never injected as a system instruction.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS session_ide_context (
                session_id TEXT PRIMARY KEY,
                context_json TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );",
        )?;

        // Migration: Agent Team tables
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS teams (
                team_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT,
                lead_session_id TEXT NOT NULL,
                lead_agent_id TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                template_id TEXT,
                config_json TEXT DEFAULT '{}'
            );

            CREATE TABLE IF NOT EXISTS team_members (
                member_id TEXT PRIMARY KEY,
                team_id TEXT NOT NULL,
                name TEXT NOT NULL,
                agent_id TEXT NOT NULL DEFAULT 'ha-main',
                role TEXT NOT NULL DEFAULT 'worker',
                status TEXT NOT NULL DEFAULT 'idle',
                run_id TEXT,
                session_id TEXT,
                color TEXT NOT NULL DEFAULT '#3B82F6',
                current_task_id INTEGER,
                model_override TEXT,
                role_description TEXT,
                joined_at TEXT NOT NULL,
                last_active_at TEXT,
                input_tokens INTEGER DEFAULT 0,
                output_tokens INTEGER DEFAULT 0,
                FOREIGN KEY (team_id) REFERENCES teams(team_id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_team_members_team ON team_members(team_id);

            CREATE TABLE IF NOT EXISTS team_messages (
                message_id TEXT PRIMARY KEY,
                team_id TEXT NOT NULL,
                from_member_id TEXT NOT NULL,
                to_member_id TEXT,
                content TEXT NOT NULL,
                message_type TEXT NOT NULL DEFAULT 'chat',
                timestamp TEXT NOT NULL,
                FOREIGN KEY (team_id) REFERENCES teams(team_id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_team_messages_team ON team_messages(team_id, timestamp DESC);

            CREATE TABLE IF NOT EXISTS team_tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                team_id TEXT NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                owner_member_id TEXT,
                priority INTEGER NOT NULL DEFAULT 100,
                blocked_by TEXT DEFAULT '[]',
                blocks TEXT DEFAULT '[]',
                column_name TEXT NOT NULL DEFAULT 'todo',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (team_id) REFERENCES teams(team_id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_team_tasks_team ON team_tasks(team_id);

            CREATE TABLE IF NOT EXISTS team_templates (
                template_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                members_json TEXT NOT NULL DEFAULT '[]',
                builtin INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT ''
            );

            -- Phase B'4: Learning event stream. Feeds the Dashboard
            -- Learning tab (skill lifecycle + recall effectiveness) and
            -- the Insights engine. Rows are opaque JSON + a discrete
            -- `kind` so new event types can be added without migrating.
            CREATE TABLE IF NOT EXISTS learning_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                kind TEXT NOT NULL,
                session_id TEXT,
                ref_id TEXT,
                meta_json TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_learning_events_ts
                ON learning_events(ts);
            CREATE INDEX IF NOT EXISTS idx_learning_events_kind_ts
                ON learning_events(kind, ts);

            -- Persists the per-session set of conditional skills
            -- (SKILL.md `paths:` frontmatter) that have been activated by
            -- the user or model touching a matching file. Survives App restart
            -- so `build_skills_prompt` can keep the skill visible.
            CREATE TABLE IF NOT EXISTS session_skill_activation (
                session_id TEXT NOT NULL,
                skill_name TEXT NOT NULL,
                activated_at TEXT NOT NULL,
                PRIMARY KEY (session_id, skill_name)
            );
            CREATE INDEX IF NOT EXISTS idx_session_skill_activation_session
                ON session_skill_activation(session_id);

            -- Deferred-tool V2: tools loaded through tool_search remain
            -- callable on later turns and after an app restart. Incognito
            -- sessions deliberately never write this table.
            CREATE TABLE IF NOT EXISTS session_tool_activation (
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                activated_at TEXT NOT NULL,
                PRIMARY KEY (session_id, tool_name)
            );
            CREATE INDEX IF NOT EXISTS idx_session_tool_activation_session
                ON session_tool_activation(session_id);

            CREATE TABLE IF NOT EXISTS session_memory_policy (
                session_id TEXT PRIMARY KEY,
                use_memories TEXT NOT NULL DEFAULT 'inherit'
                    CHECK (use_memories IN ('inherit', 'allow', 'deny')),
                contribute_to_memories TEXT NOT NULL DEFAULT 'inherit'
                    CHECK (contribute_to_memories IN ('inherit', 'allow', 'deny')),
                updated_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );",
        )?;

        // ── Idempotent migrations for team_* tables ─────────────────
        // team_members.role_description (nullable role identity prompt snippet)
        let has_role_description = conn
            .prepare("SELECT role_description FROM team_members LIMIT 1")
            .is_ok();
        if !has_role_description {
            conn.execute_batch("ALTER TABLE team_members ADD COLUMN role_description TEXT;")?;
        }

        // team_templates.updated_at (for ORDER BY)
        let has_template_updated_at = conn
            .prepare("SELECT updated_at FROM team_templates LIMIT 1")
            .is_ok();
        if !has_template_updated_at {
            conn.execute_batch(
                "ALTER TABLE team_templates ADD COLUMN updated_at TEXT NOT NULL DEFAULT '';",
            )?;
        }

        // One-time cleanup: drop legacy builtin templates (design moved to user-managed
        // presets via Settings → Teams panel; see AGENTS.md Team 系统 section).
        let _ = conn.execute("DELETE FROM team_templates WHERE builtin = 1", []);

        // Read-only connection pool. WAL lets these run concurrently with the
        // writer; opened AFTER all migrations above so they observe the final
        // schema. busy_timeout is connection-level, so each reader sets its own.
        let mut readers = Vec::with_capacity(READ_POOL_SIZE);
        for _ in 0..READ_POOL_SIZE {
            let r = Connection::open_with_flags(
                db_path,
                OpenFlags::SQLITE_OPEN_READ_ONLY
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX
                    | OpenFlags::SQLITE_OPEN_URI,
            )?;
            r.busy_timeout(std::time::Duration::from_secs(5))?;
            readers.push(Mutex::new(r));
        }

        Ok(Self {
            conn: Mutex::new(conn),
            readers,
            reader_idx: AtomicUsize::new(0),
        })
    }

    /// Run a synchronous DB operation on tokio's blocking pool.
    ///
    /// Every `SessionDB` method is synchronous rusqlite serialized by the
    /// write-connection `Mutex` (or the reader pool). Calling them directly
    /// from an async fn pins a tokio runtime worker for the full lock-wait +
    /// IO duration; when the underlying file IO stalls (antivirus, cloud-
    /// synced home dir), workers get consumed one by one until the entire
    /// runtime starves. **Async contexts must route SessionDB access through
    /// this method** so a stalled database only ever ties up expendable
    /// blocking-pool threads (see `crate::blocking`).
    pub async fn run<T, F>(self: &std::sync::Arc<Self>, f: F) -> T
    where
        F: FnOnce(&SessionDB) -> T + Send + 'static,
        T: Send + 'static,
    {
        let db = std::sync::Arc::clone(self);
        // Label with the caller's closure type, not the `move || f(&db)` wrapper
        // below — otherwise every SessionDB slow-op logs the same useless label.
        let label = std::any::type_name::<F>();
        crate::blocking::run_blocking_labeled(label, move || f(&db)).await
    }

    /// Get a read-only connection from the pool (round-robin `try_lock` first,
    /// then block on the round-robin target). NEVER returns the writer — read
    /// methods must not observe uncommitted writer state mid-transaction; WAL
    /// gives readers a consistent committed snapshot anyway.
    pub(crate) fn read_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        let idx = self
            .reader_idx
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % self.readers.len();
        for i in 0..self.readers.len() {
            let t = (idx + i) % self.readers.len();
            if let Ok(g) = self.readers[t].try_lock() {
                return Ok(g);
            }
        }
        self.readers[idx]
            .lock()
            .map_err(|e| anyhow::anyhow!("Session read pool lock poisoned: {e}"))
    }

    /// Insert a learning event row. Best-effort — errors are logged but
    /// never bubbled up; emitters treat this like a metric, not like a
    /// transactional write.
    pub fn record_learning_event(
        &self,
        kind: &str,
        session_id: Option<&str>,
        ref_id: Option<&str>,
        meta: Option<&serde_json::Value>,
    ) {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let meta_json = meta.map(|v| v.to_string());
        let conn = match self.conn.lock() {
            Ok(g) => g,
            Err(e) => {
                app_warn!("dashboard", "learning_event", "lock err: {}", e);
                return;
            }
        };
        if let Err(e) = conn.execute(
            "INSERT INTO learning_events (ts, kind, session_id, ref_id, meta_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ts, kind, session_id, ref_id, meta_json],
        ) {
            app_warn!(
                "dashboard",
                "learning_event",
                "insert {} failed: {}",
                kind,
                e
            );
        }
    }

    /// Raw timeline of `learning_events` matching `kind` with
    /// `ts >= since_ts`, most recent first. **Does not** deduplicate by
    /// `ref_id` and **does not** filter rows with NULL `ref_id` —
    /// callers like `skill_review_skipped` need every individual event
    /// (most skip events carry no `ref_id` at all because no skill was
    /// created), not a per-skill latest snapshot.
    pub fn recent_learning_events_timeline(
        &self,
        kind: &str,
        since_ts: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<(i64, Option<String>, Option<String>, Option<String>)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT ts, ref_id, session_id, meta_json
             FROM learning_events
             WHERE kind = ?1 AND ts >= ?2
             ORDER BY ts DESC, id DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![kind, since_ts, limit as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows.flatten() {
            out.push(r);
        }
        Ok(out)
    }

    /// Most recent `learning_events` matching `kind` with `ts >= since_ts`,
    /// deduplicated by `ref_id`. Returns `(ref_id, meta_json)` pairs, most
    /// recent first. Rows with empty `ref_id` are filtered out; `meta_json`
    /// may be `None` if the event was emitted without metadata.
    pub fn recent_learning_event_rows(
        &self,
        kind: &str,
        since_ts: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<(String, Option<String>)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        // Pick the latest meta_json per ref_id (highest ts wins).
        let mut stmt = conn.prepare(
            "SELECT le.ref_id, le.meta_json
             FROM learning_events le
             JOIN (
                 SELECT ref_id, MAX(ts) AS ts_max
                 FROM learning_events
                 WHERE kind = ?1 AND ts >= ?2 AND ref_id IS NOT NULL AND ref_id != ''
                 GROUP BY ref_id
             ) latest
             ON le.ref_id = latest.ref_id AND le.ts = latest.ts_max
             WHERE le.kind = ?1
             ORDER BY le.ts DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![kind, since_ts, limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        let mut out = Vec::new();
        for t in rows.flatten() {
            out.push(t);
        }
        Ok(out)
    }

    /// Delete learning_events older than `ts_cutoff`. Returns the number of
    /// rows removed. Called by the retention sweeper.
    pub fn prune_learning_events(&self, ts_cutoff: i64) -> anyhow::Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let n = conn.execute(
            "DELETE FROM learning_events WHERE ts < ?1",
            params![ts_cutoff],
        )?;
        Ok(n)
    }

    // ── ask_user_question Persistence ────────────────────────────

    /// Save (or replace) a pending ask_user_question group. Called before the
    /// request is emitted so a restart can resume it.
    pub fn save_ask_user_group(
        &self,
        group: &crate::ask_user::AskUserQuestionGroup,
    ) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let payload = serde_json::to_string(group)?;
        conn.execute(
            "INSERT OR REPLACE INTO ask_user_questions
                (request_id, session_id, payload, status, timeout_at, created_at)
             VALUES (?1, ?2, ?3, 'pending', ?4,
                     COALESCE((SELECT created_at FROM ask_user_questions WHERE request_id = ?1),
                              datetime('now')))",
            params![
                group.request_id,
                group.session_id,
                payload,
                group.timeout_at.map(|n| n.min(i64::MAX as u64) as i64),
            ],
        )?;
        Ok(())
    }

    /// Mark orphaned tool-created ask_user rows as answered at startup because
    /// their in-memory oneshot receivers cannot survive a process restart.
    /// Owner-side questions carry a durable `ownerResponse` handler and stay
    /// pending so their timeout tasks can be re-armed.
    pub fn expire_pending_ask_user_groups(&self) -> anyhow::Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT request_id, payload FROM ask_user_questions WHERE status = 'pending'",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut pending_rows = Vec::new();
        for row in rows {
            pending_rows.push(row?);
        }
        drop(stmt);
        let mut expired = 0usize;
        for (request_id, payload) in pending_rows {
            let keep_pending =
                serde_json::from_str::<crate::ask_user::AskUserQuestionGroup>(&payload)
                    .map(|group| group.owner_response.is_some())
                    .unwrap_or(false);
            if keep_pending {
                continue;
            }
            expired += conn.execute(
                "UPDATE ask_user_questions
                    SET status = 'answered', answered_at = datetime('now')
                    WHERE request_id = ?1 AND status = 'pending'",
                params![request_id],
            )?;
        }
        Ok(expired)
    }

    /// Mark a pending ask_user_question group as answered. Idempotent.
    pub fn mark_ask_user_answered(&self, request_id: &str) -> anyhow::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE ask_user_questions
                SET status = 'answered', answered_at = datetime('now')
                WHERE request_id = ?1 AND status = 'pending'",
            params![request_id],
        )?;
        Ok(())
    }

    /// Atomically expire one due ask-user group. Returns `true` only for the
    /// caller that won the pending -> answered transition, so duplicate timer
    /// tasks or a response racing the deadline cannot emit duplicate events.
    pub fn mark_ask_user_timed_out(&self, request_id: &str) -> anyhow::Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE ask_user_questions
                SET status = 'answered', answered_at = datetime('now')
                WHERE request_id = ?1
                  AND status = 'pending'
                  AND timeout_at IS NOT NULL
                  AND timeout_at > 0
                  AND timeout_at <= strftime('%s','now')",
            params![request_id],
        )?;
        Ok(changed > 0)
    }

    /// Return durable owner-plane questions that need their timeout tasks
    /// re-armed after process startup. Tool-created rows are intentionally
    /// excluded because their in-memory oneshot receivers cannot survive a
    /// restart and are handled by `expire_pending_ask_user_groups`.
    pub fn list_pending_owner_ask_user_groups(
        &self,
    ) -> anyhow::Result<Vec<crate::ask_user::AskUserQuestionGroup>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT payload, timeout_at, CAST(strftime('%s', created_at) AS INTEGER)
               FROM ask_user_questions
                WHERE status = 'pending'
                  AND timeout_at IS NOT NULL
                  AND timeout_at > 0",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })?;
        let server_now = chrono::Utc::now().timestamp().max(0) as u64;
        let mut out = Vec::new();
        for row in rows {
            let (payload, timeout_at, created_at) = row?;
            if let Ok(mut group) =
                serde_json::from_str::<crate::ask_user::AskUserQuestionGroup>(&payload)
            {
                if group.owner_response.is_some() {
                    group.server_now = Some(server_now);
                    if group.timeout_secs.is_none() {
                        group.timeout_secs = timeout_at
                            .zip(created_at)
                            .and_then(|(deadline, created)| deadline.checked_sub(created))
                            .and_then(|seconds| u64::try_from(seconds).ok())
                            .filter(|seconds| *seconds > 0);
                    }
                    out.push(group);
                }
            }
        }
        Ok(out)
    }

    /// Drop answered rows older than `retain_days` days so the
    /// `ask_user_questions` table doesn't accumulate indefinitely.
    pub fn purge_old_answered_ask_user_groups(&self, retain_days: u32) -> anyhow::Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let cutoff = format!("-{} days", retain_days);
        let n = conn.execute(
            "DELETE FROM ask_user_questions
                WHERE status = 'answered'
                  AND answered_at IS NOT NULL
                  AND answered_at < datetime('now', ?1)",
            params![cutoff],
        )?;
        Ok(n)
    }

    /// Count still-pending ask_user_question groups grouped by session id.
    /// Powers the "needs your response" indicator on the sidebar session list.
    /// Expired-but-not-yet-answered rows are excluded so we don't double-count
    /// zombies from a previous process; a periodic sweep elsewhere flips them
    /// to `answered`.
    pub fn count_pending_ask_user_groups_per_session(&self) -> Result<HashMap<String, i64>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT session_id, COUNT(*)
                FROM ask_user_questions
               WHERE status = 'pending'
                 AND (timeout_at IS NULL OR timeout_at = 0
                      OR timeout_at > strftime('%s','now'))
               GROUP BY session_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut out: HashMap<String, i64> = HashMap::new();
        for row in rows {
            let (sid, count) = row?;
            out.insert(sid, count);
        }
        Ok(out)
    }

    /// Load still-pending ask_user_question groups for a single session.
    /// Used by the frontend to restore the question panel when switching back
    /// to a session that had unanswered questions.
    pub fn list_pending_ask_user_groups_for_session(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<crate::ask_user::AskUserQuestionGroup>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT payload FROM ask_user_questions
                WHERE status = 'pending'
                  AND session_id = ?1
                  AND (timeout_at IS NULL OR timeout_at = 0
                       OR timeout_at > strftime('%s','now'))
                ORDER BY created_at ASC
                LIMIT 50",
        )?;
        let rows = stmt.query_map(params![session_id], |row| row.get::<_, String>(0))?;
        let server_now = chrono::Utc::now().timestamp().max(0) as u64;
        let mut out = Vec::new();
        for row in rows {
            let payload = row?;
            if let Ok(mut group) =
                serde_json::from_str::<crate::ask_user::AskUserQuestionGroup>(&payload)
            {
                group.server_now = Some(server_now);
                out.push(group);
            }
        }
        Ok(out)
    }

    pub fn get_pending_ask_user_group_by_request_id(
        &self,
        request_id: &str,
    ) -> anyhow::Result<Option<crate::ask_user::AskUserQuestionGroup>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let payload: Option<String> = conn
            .query_row(
                "SELECT payload FROM ask_user_questions
                    WHERE request_id = ?1
                      AND status = 'pending'
                      AND (timeout_at IS NULL OR timeout_at = 0
                           OR timeout_at > strftime('%s','now'))
                    LIMIT 1",
                params![request_id],
                |row| row.get(0),
            )
            .optional()?;
        payload
            .map(|payload| serde_json::from_str(&payload).map_err(Into::into))
            .transpose()
    }

    // ── Session CRUD ─────────────────────────────────────────────

    /// Create a new session, return its metadata.
    pub fn create_session(&self, agent_id: &str) -> Result<SessionMeta> {
        // Flush pending idle extractions from previous sessions
        crate::memory_extract::flush_all_idle_extractions();
        self.create_session_with_parent(agent_id, None)
    }

    /// Create a new session with an optional parent session ID (for sub-agent sessions).
    pub fn create_session_with_parent(
        &self,
        agent_id: &str,
        parent_session_id: Option<&str>,
    ) -> Result<SessionMeta> {
        self.create_session_full(agent_id, parent_session_id, None, false)
    }

    /// Create a new session, optionally bound to a project and/or marked incognito.
    ///
    /// When `project_id` is `Some`, the session is bound to that project and
    /// project-scoped memories / files will be automatically injected into its
    /// system prompt. Project + incognito are mutually exclusive — if both are
    /// requested, project wins and incognito is silently coerced to `false`.
    pub fn create_session_with_project(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
        incognito: Option<bool>,
    ) -> Result<SessionMeta> {
        crate::memory_extract::flush_all_idle_extractions();
        let incognito = incognito.unwrap_or(false) && project_id.is_none();
        self.create_session_full(agent_id, None, project_id, incognito)
    }

    /// Fully-parameterized session creator. Private helper called by the other
    /// `create_session*` variants so the INSERT statement exists in exactly one
    /// place.
    pub(crate) fn create_session_full(
        &self,
        agent_id: &str,
        parent_session_id: Option<&str>,
        project_id: Option<&str>,
        incognito: bool,
    ) -> Result<SessionMeta> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        // New sessions inherit the agent's configured default permission mode
        // (`capabilities.default_session_permission_mode`). This is the single
        // source of truth for the initial mode and applies to *all* creation
        // paths — pre-materialized project / cron / subagent sessions as well as
        // drafts. (The chat input also seeds drafts from the same field for the
        // pre-INSERT UI, but that path is skipped once a session row exists, so
        // pre-materialized sessions relied entirely on this.) Falls back to
        // `Default` when the agent config is missing or the field is unset.
        // Sandbox mode follows the same create-time inheritance model, with the
        // legacy `capabilities.sandbox=true` mapping to `standard`.
        let agent_definition = crate::agent_loader::load_agent(agent_id).ok();
        let initial_permission_mode = agent_definition
            .as_ref()
            .and_then(|def| def.config.capabilities.default_session_permission_mode)
            .unwrap_or_default();
        let initial_sandbox_mode = agent_definition
            .as_ref()
            .map(|def| def.config.capabilities.effective_default_sandbox_mode())
            .unwrap_or_default();
        let app_config = crate::config::cached_config();
        let agent_model = agent_definition
            .as_ref()
            .map(|def| def.config.model.clone())
            .unwrap_or_default();
        let (initial_model, _) = crate::provider::resolve_model_chain(&agent_model, &app_config);
        let initial_provider_name = initial_model.as_ref().and_then(|model| {
            app_config
                .providers
                .iter()
                .find(|provider| provider.id == model.provider_id)
                .map(|provider| provider.name.clone())
        });
        let initial_temperature = agent_model.temperature.or(app_config.temperature);
        let initial_reasoning_effort = agent_model
            .reasoning_effort
            .clone()
            .unwrap_or_else(|| app_config.reasoning_effort.clone());

        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO sessions (id, agent_id, provider_id, provider_name, model_id, temperature, reasoning_effort, runtime_defaults_initialized, created_at, updated_at, parent_session_id, project_id, permission_mode, sandbox_mode, incognito)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                id,
                agent_id,
                initial_model.as_ref().map(|model| model.provider_id.as_str()),
                initial_provider_name.as_deref(),
                initial_model.as_ref().map(|model| model.model_id.as_str()),
                initial_temperature,
                initial_reasoning_effort,
                now,
                now,
                parent_session_id,
                project_id,
                initial_permission_mode.as_str(),
                initial_sandbox_mode.as_str(),
                incognito
            ],
        )?;

        Ok(SessionMeta {
            id,
            title: None,
            title_source: crate::session_title::TITLE_SOURCE_MANUAL.to_string(),
            agent_id: agent_id.to_string(),
            provider_id: initial_model
                .as_ref()
                .map(|model| model.provider_id.clone()),
            provider_name: initial_provider_name,
            model_id: initial_model.as_ref().map(|model| model.model_id.clone()),
            temperature: initial_temperature,
            reasoning_effort: Some(initial_reasoning_effort),
            runtime_defaults_initialized: true,
            created_at: now.clone(),
            updated_at: now,
            pinned_at: None,
            message_count: 0,
            unread_count: 0,
            channel_unread_count: 0,
            has_error: false,
            pending_interaction_count: 0,
            is_cron: false,
            parent_session_id: parent_session_id.map(|s| s.to_string()),
            forked_from_session_id: None,
            forked_from_message_id: None,
            forked_from_session_title: None,
            plan_mode: crate::plan::PlanModeState::Off,
            execution_mode: crate::execution_mode::ExecutionMode::Off,
            workflow_mode: crate::workflow_mode::WorkflowMode::Off,
            permission_mode: initial_permission_mode,
            sandbox_mode: initial_sandbox_mode,
            project_id: project_id.map(|s| s.to_string()),
            channel_info: None,
            incognito,
            working_dir: None,
            kind: SessionKind::Regular,
        })
    }

    /// Fork a regular user-facing session into a new first-class session.
    ///
    /// The fork copies the persisted transcript up to `source_message_id`
    /// (inclusive) or the full transcript when it is `None`. It also copies the
    /// stable conversation configuration (agent/model/project/workdir,
    /// permission/sandbox/execution/workflow modes) while intentionally not
    /// copying active control-plane state such as goals, loops, workflow runs,
    /// tasks, approvals, or background jobs. Those records are live run state,
    /// not transcript history, and sharing them would couple the original and
    /// forked sessions.
    pub fn fork_session(
        &self,
        source_session_id: &str,
        source_message_id: Option<i64>,
    ) -> Result<SessionMeta> {
        let new_session_id = uuid::Uuid::new_v4().to_string();
        let fork_result = (|| -> Result<()> {
            let mut conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            let tx = conn.transaction()?;

            let (
                source_title,
                source_title_source,
                agent_id,
                provider_id,
                provider_name,
                model_id,
                reasoning_effort,
                project_id,
                permission_mode,
                sandbox_mode,
                execution_mode,
                workflow_mode,
                working_dir,
                kind,
                incognito,
                is_cron,
                parent_session_id,
            ): (
                Option<String>,
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                String,
                String,
                String,
                String,
                Option<String>,
                String,
                i64,
                i64,
                Option<String>,
            ) = tx
                .query_row(
                    "SELECT title, title_source, agent_id, provider_id, provider_name, model_id,
                        reasoning_effort, project_id, permission_mode, sandbox_mode,
                        execution_mode, workflow_mode, working_dir, kind, incognito,
                        is_cron, parent_session_id
                 FROM sessions WHERE id = ?1",
                    params![source_session_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                            row.get(8)?,
                            row.get(9)?,
                            row.get(10)?,
                            row.get(11)?,
                            row.get(12)?,
                            row.get(13)?,
                            row.get(14)?,
                            row.get(15)?,
                            row.get(16)?,
                        ))
                    },
                )
                .optional()?
                .ok_or_else(|| {
                    anyhow::anyhow!("source session not found: {}", source_session_id)
                })?;

            if incognito != 0 {
                anyhow::bail!("incognito sessions cannot be forked");
            }
            // Regular top-level chats and design-space threads are forkable; the
            // latter产物仍是设计线程（补建 design_chat_threads 锚点见下）。cron / 子会话 /
            // 其它隐藏 kind（knowledge / eval_fixture）与 incognito 仍拒。
            let is_forkable_kind = kind == SessionKind::Regular.as_str()
                || kind == SessionKind::Design.as_str();
            if is_cron != 0 || parent_session_id.is_some() || !is_forkable_kind {
                anyhow::bail!("only regular top-level sessions can be forked");
            }

            if let Some(message_id) = source_message_id {
                let exists: i64 = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM messages WHERE session_id = ?1 AND id = ?2)",
                    params![source_session_id, message_id],
                    |row| row.get(0),
                )?;
                if exists == 0 {
                    anyhow::bail!("source message not found in session: {}", message_id);
                }
            }

            let active_streaming_rows: i64 = tx.query_row(
                "SELECT COUNT(*) FROM messages
             WHERE session_id = ?1
               AND (?2 IS NULL OR id <= ?2)
               AND stream_status = 'streaming'",
                params![source_session_id, source_message_id],
                |row| row.get(0),
            )?;
            if active_streaming_rows > 0 {
                anyhow::bail!(
                    "session has an active response; wait for it to finish before forking"
                );
            }

            let copied_count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM messages
             WHERE session_id = ?1
               AND (?2 IS NULL OR id <= ?2)",
                params![source_session_id, source_message_id],
                |row| row.get(0),
            )?;
            if copied_count == 0 {
                anyhow::bail!("cannot fork an empty session");
            }

            let has_source_title = source_title
                .as_deref()
                .is_some_and(|title| !title.trim().is_empty());
            let inferred_title: Option<String> = if has_source_title {
                source_title.clone()
            } else {
                tx.query_row(
                    "SELECT content FROM messages
                 WHERE session_id = ?1
                   AND (?2 IS NULL OR id <= ?2)
                   AND role = 'user'
                   AND length(trim(content)) > 0
                 ORDER BY id ASC LIMIT 1",
                    params![source_session_id, source_message_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .map(|content| crate::truncate_utf8(content.trim(), 80).to_string())
                .filter(|title| !title.is_empty())
            };
            let title_source = if has_source_title {
                source_title_source
            } else if inferred_title.is_some() {
                crate::session_title::TITLE_SOURCE_FIRST_MESSAGE.to_string()
            } else {
                crate::session_title::TITLE_SOURCE_MANUAL.to_string()
            };
            // Fork 出的会话标题默认与源一模一样——追加 `(1)`/`(2)`/… 序号去重。设计线程按
            // 同项目分组去重，普通会话在 regular 族内去重。无标题分支不动。
            let dedup_project_filter: Option<&str> = if kind == SessionKind::Design.as_str() {
                project_id.as_deref()
            } else {
                None
            };
            let inferred_title = match inferred_title {
                Some(title) => Some(Self::dedup_fork_title(
                    &tx,
                    &kind,
                    dedup_project_filter,
                    &title,
                )?),
                None => None,
            };

            let now = chrono::Utc::now().to_rfc3339();
            tx.execute(
            "INSERT INTO sessions (
                id, title, title_source, agent_id, provider_id, provider_name, model_id,
                reasoning_effort, created_at, updated_at, parent_session_id, project_id,
                permission_mode, sandbox_mode, execution_mode, workflow_mode, working_dir,
                kind, forked_from_session_id, forked_from_message_id
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                new_session_id,
                inferred_title,
                title_source,
                agent_id,
                provider_id,
                provider_name,
                model_id,
                reasoning_effort,
                now,
                now,
                project_id,
                permission_mode,
                sandbox_mode,
                execution_mode,
                workflow_mode,
                working_dir,
                kind,
                source_session_id,
                source_message_id,
            ],
        )?;

            tx.execute(
                "INSERT INTO messages (
                session_id, role, content, timestamp, attachments_meta, model,
                tokens_in, tokens_out, reasoning_effort, tool_call_id, tool_name,
                tool_arguments, tool_result, tool_duration_ms, is_error, thinking,
                ttft_ms, tokens_in_last, tokens_cache_creation, tokens_cache_read,
                tool_metadata, stream_status, source
             )
             SELECT ?1, role, content, timestamp, attachments_meta, model,
                tokens_in, tokens_out, reasoning_effort, tool_call_id, tool_name,
                tool_arguments, tool_result, tool_duration_ms, is_error, thinking,
                ttft_ms, tokens_in_last, tokens_cache_creation, tokens_cache_read,
                tool_metadata, stream_status, source
             FROM messages
             WHERE session_id = ?2
               AND (?3 IS NULL OR id <= ?3)
             ORDER BY id ASC",
                params![new_session_id, source_session_id, source_message_id],
            )?;

            let attachment_meta_rewrites = {
                let mut stmt = tx.prepare(
                    "SELECT DISTINCT attachments_meta FROM messages
                 WHERE session_id = ?1
                   AND (?2 IS NULL OR id <= ?2)
                   AND attachments_meta IS NOT NULL",
                )?;
                let raw_values = stmt
                    .query_map(params![source_session_id, source_message_id], |row| {
                        row.get::<_, String>(0)
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                raw_values
                    .into_iter()
                    .map(|raw| {
                        let rewritten = crate::attachments::fork_attachments_meta(
                            source_session_id,
                            &new_session_id,
                            &raw,
                        )?;
                        Ok((raw, rewritten))
                    })
                    .collect::<Result<Vec<_>>>()?
            };

            for (raw, rewritten) in attachment_meta_rewrites {
                if raw != rewritten {
                    tx.execute(
                        "UPDATE messages SET attachments_meta = ?1
                     WHERE session_id = ?2 AND attachments_meta = ?3",
                        params![rewritten, new_session_id, raw],
                    )?;
                }
            }

            // 设计线程 fork：补建 `design_chat_threads` 锚点，让设计工具经
            // `project_for_session` 解析回源设计项目（否则会落到新草稿项目）。以锚表为
            // 权威来源读源 project_id，不依赖被复制的 `sessions.project_id`。
            if kind == SessionKind::Design.as_str() {
                let src_project: Option<String> = tx
                    .query_row(
                        "SELECT project_id FROM design_chat_threads WHERE session_id = ?1",
                        params![source_session_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                match src_project {
                    Some(pid) => {
                        let now_ms = chrono::Utc::now().timestamp_millis();
                        tx.execute(
                            "INSERT INTO design_chat_threads (session_id, project_id, created_at)
                             VALUES (?1, ?2, ?3)
                             ON CONFLICT(session_id) DO NOTHING",
                            params![new_session_id, pid, now_ms],
                        )?;
                    }
                    None => {
                        crate::app_warn!(
                            "design",
                            "fork",
                            "design source {} has no thread anchor; forked session {} left unanchored",
                            source_session_id,
                            new_session_id
                        );
                    }
                }
            }

            let last_message_id: Option<i64> = tx.query_row(
                "SELECT MAX(id) FROM messages WHERE session_id = ?1",
                params![new_session_id],
                |row| row.get(0),
            )?;
            if let Some(last_message_id) = last_message_id {
                tx.execute(
                    "UPDATE sessions SET last_read_message_id = ?1 WHERE id = ?2",
                    params![last_message_id, new_session_id],
                )?;
            }

            tx.commit()?;
            Ok(())
        })();

        if let Err(error) = fork_result {
            if let Ok(attachments_dir) = crate::paths::attachments_dir(&new_session_id) {
                let _ = std::fs::remove_dir_all(attachments_dir);
            }
            return Err(error);
        }

        self.get_session(&new_session_id)?
            .ok_or_else(|| anyhow::anyhow!("forked session disappeared: {}", new_session_id))
    }

    /// Split a session title into its base and optional trailing ` (N)` sequence.
    /// `"配色方案 (2)"` → `("配色方案", Some(2))`; `"配色方案"` → `("配色方案", None)`;
    /// non-numeric / malformed suffixes (`"Foo (x)"`) return the whole title as base.
    fn split_title_seq(title: &str) -> (&str, Option<u64>) {
        let trimmed = title.trim_end();
        if !trimmed.ends_with(')') {
            return (title, None);
        }
        let Some(open) = trimmed.rfind(" (") else {
            return (title, None);
        };
        let digits = &trimmed[open + 2..trimmed.len() - 1];
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return (title, None);
        }
        match digits.parse::<u64>() {
            Ok(n) => (&trimmed[..open], Some(n)),
            Err(_) => (title, None),
        }
    }

    /// Append a `(N)` sequence to a fork's candidate title so it doesn't collide
    /// with existing sessions. `base` is the candidate with any trailing ` (N)`
    /// stripped (so forking `"Foo (2)"` continues the series rather than nesting).
    /// The family is scoped to `kind` (and `project_filter` when Some — design
    /// threads dedup within their project). Returns the candidate unchanged when
    /// no same-base title exists (e.g. an inferred title with no collision).
    fn dedup_fork_title(
        tx: &rusqlite::Transaction,
        kind: &str,
        project_filter: Option<&str>,
        candidate: &str,
    ) -> Result<String> {
        let (base, _) = Self::split_title_seq(candidate.trim());
        let base = base.trim();
        if base.is_empty() {
            return Ok(candidate.to_string());
        }
        let mut stmt = tx.prepare(
            "SELECT title FROM sessions
             WHERE kind = ?1
               AND (?2 IS NULL OR project_id = ?2)
               AND title IS NOT NULL",
        )?;
        let titles = stmt
            .query_map(params![kind, project_filter], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut max_seq: Option<u64> = None;
        for existing in &titles {
            let (ebase, eseq) = Self::split_title_seq(existing.trim());
            if ebase.trim() == base {
                let seq = eseq.unwrap_or(0);
                max_seq = Some(max_seq.map_or(seq, |m| m.max(seq)));
            }
        }
        match max_seq {
            Some(m) => Ok(format!("{base} ({})", m + 1)),
            None => Ok(candidate.to_string()),
        }
    }

    /// Set a session's classification (see [`SessionKind`]). Used by the
    /// knowledge-space chat entry to mark a freshly-created session as a
    /// `Knowledge` conversation so it is hidden from the main session list.
    pub fn set_session_kind(&self, session_id: &str, kind: SessionKind) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET kind = ?1 WHERE id = ?2",
            params![kind.as_str(), session_id],
        )?;
        Ok(())
    }

    /// Move a session to a project (or remove it from the current project when `project_id` is `None`).
    pub fn set_session_project(&self, session_id: &str, project_id: Option<&str>) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET project_id = ?1 WHERE id = ?2",
            params![project_id, session_id],
        )?;
        Ok(())
    }

    /// Clear `project_id` from every session that currently references it.
    /// Used by `ProjectDB::delete` so deleting a project does not cascade-delete
    /// its sessions — they simply become unassigned.
    pub fn clear_project_from_sessions(&self, project_id: &str) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let n = conn.execute(
            "UPDATE sessions SET project_id = NULL WHERE project_id = ?1",
            params![project_id],
        )?;
        Ok(n)
    }

    /// Return up to 5 (session_id, agent_id) pairs whose session id starts
    /// with `prefix` — used by the plan-files migration to map legacy
    /// short-id filenames back to their owning session/agent. Cap is 5
    /// because callers care about "is this prefix unique?" not full lists.
    pub fn find_sessions_by_id_prefix(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt =
            conn.prepare("SELECT id, agent_id FROM sessions WHERE id LIKE ?1 || '%' LIMIT 5")?;
        let rows = stmt.query_map(params![prefix], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// List all sessions, ordered by most recently updated.
    /// Optionally filter by agent_id.
    pub fn list_sessions(&self, agent_id: Option<&str>) -> Result<Vec<SessionMeta>> {
        let (sessions, _) =
            self.list_sessions_paged(agent_id, ProjectFilter::All, None, None, None)?;
        Ok(sessions)
    }

    /// Paginated session list. Returns `(sessions, total_count)`.
    /// When `limit` is `None`, all sessions are returned (backwards-compatible).
    ///
    /// `project_filter` selects which sessions appear based on their project assignment:
    /// * [`ProjectFilter::All`] — no project filter (default behavior)
    /// * [`ProjectFilter::Unassigned`] — only sessions with `project_id IS NULL`
    /// * [`ProjectFilter::InProject`] — only sessions in the given project
    pub fn list_sessions_paged(
        &self,
        agent_id: Option<&str>,
        project_filter: ProjectFilter<'_>,
        limit: Option<u32>,
        offset: Option<u32>,
        active_session_id: Option<&str>,
    ) -> Result<(Vec<SessionMeta>, u32)> {
        self.list_sessions_paged_inner(
            agent_id,
            project_filter,
            ParentSessionFilter::All,
            limit,
            offset,
            active_session_id,
            "s.updated_at DESC",
            false,
        )
    }

    /// Sidebar-facing session list. Pinned sessions sort above the normal
    /// updated-at ordering; internal "recent session" callers should keep
    /// using [`Self::list_sessions_paged`] so old pins cannot crowd out fresh
    /// activity from a limited candidate window.
    pub fn list_sessions_paged_for_sidebar(
        &self,
        agent_id: Option<&str>,
        project_filter: ProjectFilter<'_>,
        parent_filter: ParentSessionFilter,
        limit: Option<u32>,
        offset: Option<u32>,
        active_session_id: Option<&str>,
    ) -> Result<(Vec<SessionMeta>, u32)> {
        self.list_sessions_paged_inner(
            agent_id,
            project_filter,
            parent_filter,
            limit,
            offset,
            active_session_id,
            "s.pinned_at IS NULL ASC, s.pinned_at DESC, s.updated_at DESC",
            // Cron run sessions are surfaced in the cron panel's "conversations"
            // timeline, never the main sidebar list.
            true,
        )
    }

    /// Recent regular user-facing chats, ordered by activity. This mirrors
    /// [`SessionMeta::is_regular_chat`] at the SQL layer so small LIMIT windows
    /// cannot be consumed by cron / subagent / channel / incognito / knowledge
    /// sessions before the caller gets a chance to filter them.
    pub fn list_recent_regular_chats(&self, limit: u32) -> Result<(Vec<SessionMeta>, u32)> {
        let conn = self.read_conn()?;
        let regular_where = " WHERE s.is_cron = 0
            AND s.parent_session_id IS NULL
            AND s.incognito = 0
            AND s.kind = 'regular'
            AND NOT EXISTS (
                SELECT 1 FROM channel_conversations cc_filter
                WHERE cc_filter.session_id = s.id
            )";
        let count_sql = format!("SELECT COUNT(*) FROM sessions s{regular_where}");
        let total: u32 = conn.query_row(&count_sql, [], |r| r.get::<_, u32>(0))?;
        let sql =
            format!("{SESSION_META_SELECT}{regular_where} ORDER BY s.updated_at DESC LIMIT ?1");

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit], Self::row_to_session_meta)?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }

        Ok((sessions, total))
    }

    fn list_sessions_paged_inner(
        &self,
        agent_id: Option<&str>,
        project_filter: ProjectFilter<'_>,
        parent_filter: ParentSessionFilter,
        limit: Option<u32>,
        offset: Option<u32>,
        active_session_id: Option<&str>,
        order_by: &str,
        exclude_cron: bool,
    ) -> Result<(Vec<SessionMeta>, u32)> {
        // Sidebar list is a hot read during streaming — use the read pool so a
        // concurrent message-append write doesn't block it.
        let conn = self.read_conn()?;

        // Unread only counts final `assistant` rows — tool / text_block /
        // thinking_block / event rows are artifacts of the same turn and would
        // inflate the badge (one question with N tool calls would read as 2N+).
        let base_sql = SESSION_META_SELECT;
        let count_base = "SELECT COUNT(*) FROM sessions s";
        let row_mapper = Self::row_to_session_meta;

        // Build dynamic WHERE / params.
        let mut where_clauses: Vec<String> = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(aid) = agent_id {
            let idx = params_vec.len() + 1;
            where_clauses.push(format!("s.agent_id = ?{}", idx));
            params_vec.push(Box::new(aid.to_string()));
        }

        match project_filter {
            ProjectFilter::All => {}
            ProjectFilter::Unassigned => {
                where_clauses.push("s.project_id IS NULL".to_string());
            }
            ProjectFilter::InProject(pid) => {
                let idx = params_vec.len() + 1;
                where_clauses.push(format!("s.project_id = ?{}", idx));
                params_vec.push(Box::new(pid.to_string()));
            }
        }

        match parent_filter {
            ParentSessionFilter::All => {}
            ParentSessionFilter::Root => {
                where_clauses.push("s.parent_session_id IS NULL".to_string());
            }
            ParentSessionFilter::Child => {
                where_clauses.push("s.parent_session_id IS NOT NULL".to_string());
            }
        }

        // Knowledge-space sidebar conversations live in the KB panel, never the
        // main session list / picker — hide them unconditionally (no active
        // exception, unlike incognito below).
        where_clauses.push("s.kind NOT IN ('knowledge','design','eval_fixture')".to_string());

        // Cron run sessions live in the cron panel's "conversations" timeline,
        // never the main sidebar list — hide them when the sidebar asks.
        if exclude_cron {
            where_clauses.push("s.is_cron = 0".to_string());
        }

        // Hide incognito sessions from listings (the whole point of "no
        // trace"); the currently-open session is the lone exception so the
        // sidebar still shows it while the user is in it.
        match active_session_id {
            Some(sid) => {
                let idx = params_vec.len() + 1;
                where_clauses.push(format!("(s.incognito = 0 OR s.id = ?{})", idx));
                params_vec.push(Box::new(sid.to_string()));
            }
            None => {
                where_clauses.push("s.incognito = 0".to_string());
            }
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_clauses.join(" AND "))
        };

        let pagination_clause = match limit {
            Some(l) => format!(" LIMIT {} OFFSET {}", l, offset.unwrap_or(0)),
            None => String::new(),
        };

        let count_sql = format!("{}{}", count_base, where_sql);
        let sql = format!(
            "{}{} ORDER BY {}{}",
            base_sql, where_sql, order_by, pagination_clause
        );

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

        let total: u32 =
            conn.query_row(&count_sql, param_refs.as_slice(), |r| r.get::<_, u32>(0))?;

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), row_mapper)?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }

        Ok((sessions, total))
    }

    /// Load all messages for a session.
    pub fn load_session_messages(&self, session_id: &str) -> Result<Vec<SessionMessage>> {
        let conn = self.read_conn()?;

        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking, ttft_ms, tokens_in_last,
                    tokens_cache_creation, tokens_cache_read, tool_metadata, stream_status
             FROM messages
             WHERE session_id = ?1
             ORDER BY id ASC",
        )?;

        let rows = stmt.query_map(params![session_id], |row| Self::row_to_session_message(row))?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    }

    /// Load a single persisted message by database id.
    pub fn get_message(&self, message_id: i64) -> Result<Option<SessionMessage>> {
        let conn = self.read_conn()?;
        conn.query_row(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking, ttft_ms, tokens_in_last,
                    tokens_cache_creation, tokens_cache_read, tool_metadata, stream_status
             FROM messages
             WHERE id = ?1",
            params![message_id],
            Self::row_to_session_message,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Load the latest N messages for a session (for initial page load).
    ///
    /// Returns `(messages_in_asc_order, total_count, has_more)`.
    ///
    /// The returned window is always aligned so that its first row is a
    /// `user` role message (or the session's first row). This keeps a single
    /// assistant turn — which may span dozens of tool / text_block / thinking
    /// rows under long tool loops — from being split mid-way by page
    /// boundaries. `has_more` tells the caller whether there are older rows
    /// before the returned window.
    pub fn load_session_messages_latest(
        &self,
        session_id: &str,
        limit: u32,
    ) -> Result<(Vec<SessionMessage>, u32, bool)> {
        let conn = self.read_conn()?;

        let total: u32 = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;

        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking, ttft_ms, tokens_in_last,
                    tokens_cache_creation, tokens_cache_read, tool_metadata, stream_status
             FROM messages
             WHERE session_id = ?1
             ORDER BY id DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![session_id, limit], |row| {
            Self::row_to_session_message(row)
        })?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        // Reverse to get ASC order
        messages.reverse();

        Self::align_window_to_user_boundary(&conn, session_id, &mut messages)?;

        let has_more = match messages.first() {
            Some(first) => Self::has_messages_before(&conn, session_id, first.id),
            None => false,
        };

        Ok((messages, total, has_more))
    }

    /// Load messages before a given message id (for "load more" / scroll up).
    ///
    /// Returns `(messages_in_asc_order, has_more)`. The window is aligned so
    /// its first row is a `user` message (same guarantee as
    /// `load_session_messages_latest`), preventing a single assistant turn
    /// from being split across pages.
    pub fn load_session_messages_before(
        &self,
        session_id: &str,
        before_id: i64,
        limit: u32,
    ) -> Result<(Vec<SessionMessage>, bool)> {
        let conn = self.read_conn()?;

        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking, ttft_ms, tokens_in_last,
                    tokens_cache_creation, tokens_cache_read, tool_metadata, stream_status
             FROM messages
             WHERE session_id = ?1 AND id < ?2
             ORDER BY id DESC
             LIMIT ?3",
        )?;

        let rows = stmt.query_map(params![session_id, before_id, limit], |row| {
            Self::row_to_session_message(row)
        })?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        messages.reverse();

        Self::align_window_to_user_boundary(&conn, session_id, &mut messages)?;

        let has_more = match messages.first() {
            Some(first) => Self::has_messages_before(&conn, session_id, first.id),
            None => false,
        };

        Ok((messages, has_more))
    }

    /// If the first row of an ASC-ordered window is not a `user` message,
    /// prepend the contiguous rows `[user_anchor_id, current_first_id)` so
    /// the window starts on a user boundary. When no prior user row exists
    /// (the window is pinned to the start of the session and the session
    /// happens to begin with non-user rows), extend to the session's first
    /// row. No-op when the window is empty or already aligned.
    ///
    /// Rationale: a single assistant turn may span dozens of tool / thinking
    /// / text_block rows under long tool loops. Row-count pagination would
    /// split those mid-turn, causing [`parseSessionMessages`] to misattribute
    /// "orphan" leading tools to the wrong assistant. Aligning on the DB
    /// side guarantees every returned window is a sequence of complete
    /// visual turns.
    fn align_window_to_user_boundary(
        conn: &Connection,
        session_id: &str,
        messages: &mut Vec<SessionMessage>,
    ) -> Result<()> {
        let oldest_id = match messages.first() {
            Some(first) if !matches!(first.role, MessageRole::User) => first.id,
            _ => return Ok(()),
        };

        let anchor_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM messages
                 WHERE session_id = ?1 AND id < ?2 AND role = 'user'
                 ORDER BY id DESC LIMIT 1",
                params![session_id, oldest_id],
                |row| row.get(0),
            )
            .ok();

        let fill_start: i64 = match anchor_id {
            Some(aid) => aid,
            None => {
                // No user row before oldest; fall back to the session's first
                // orphan row (session can begin with tool/thinking rows when
                // persistence is in flight).
                match conn.query_row::<Option<i64>, _, _>(
                    "SELECT MIN(id) FROM messages WHERE session_id = ?1 AND id < ?2",
                    params![session_id, oldest_id],
                    |row| row.get(0),
                ) {
                    Ok(Some(mid)) => mid,
                    _ => return Ok(()),
                }
            }
        };

        if fill_start >= oldest_id {
            return Ok(());
        }

        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking, ttft_ms, tokens_in_last,
                    tokens_cache_creation, tokens_cache_read, tool_metadata, stream_status
             FROM messages
             WHERE session_id = ?1 AND id >= ?2 AND id < ?3
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![session_id, fill_start, oldest_id], |row| {
            Self::row_to_session_message(row)
        })?;

        let mut ext = Vec::new();
        for row in rows {
            ext.push(row?);
        }

        let ext_count = ext.len();
        if ext_count > LARGE_TURN_EXTENSION_LOG_THRESHOLD {
            // Feeds PAGE_SIZE tuning / virtual-scrolling decisions: large
            // single-turn extensions mean the per-page row count is
            // materially exceeding the request.
            app_info!(
                "session",
                "db::align_user_boundary",
                "extended window by {} rows (session: {})",
                ext_count,
                session_id
            );
        }

        ext.extend(std::mem::take(messages));
        *messages = ext;
        Ok(())
    }

    /// Mirror of [`load_session_messages_before`] for forward pagination.
    /// Returns up to `limit` messages with `id > after_id` in ASC order, plus
    /// a `has_more` flag indicating whether further messages exist beyond the
    /// returned window. The trailing edge is extended to the next `user`
    /// boundary (`extend_window_to_turn_end`) so the caller never has to
    /// worry about a turn being split across pages — without that, the
    /// frontend's `parseSessionMessages` would emit a synthetic placeholder
    /// for trailing pending blocks, then a second one for the leading
    /// orphans of the next page, doubling-up an assistant bubble.
    pub fn load_session_messages_after(
        &self,
        session_id: &str,
        after_id: i64,
        limit: u32,
    ) -> Result<(Vec<SessionMessage>, bool)> {
        let conn = self.read_conn()?;

        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking, ttft_ms, tokens_in_last,
                    tokens_cache_creation, tokens_cache_read, tool_metadata, stream_status
             FROM messages
             WHERE session_id = ?1 AND id > ?2
             ORDER BY id ASC
             LIMIT ?3",
        )?;

        let rows = stmt.query_map(params![session_id, after_id, limit], |row| {
            Self::row_to_session_message(row)
        })?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }

        if let Some(last) = messages.last() {
            let anchor = last.id;
            Self::extend_window_to_turn_end(&conn, session_id, anchor, &mut messages)?;
        }

        let has_more = match messages.last() {
            Some(last) => Self::has_messages_after(&conn, session_id, last.id),
            None => false,
        };

        Ok((messages, has_more))
    }

    /// Extend the trailing edge of an ASC window so it ends at a turn
    /// boundary — i.e. either at the row immediately before the next `user`
    /// row, or at the session's last row when no further `user` exists.
    /// Mirrors `align_window_to_user_boundary` (which fixes the leading
    /// edge): without this, an `_after` page that cuts mid-turn would force
    /// `parseSessionMessages` to emit a synthetic placeholder assistant for
    /// the trailing pending blocks, and the *next* `_after` page would
    /// emit a second one for the leading orphans — splitting one logical
    /// assistant turn across multiple visual bubbles. Use the supplied
    /// `anchor_id` (caller's last loaded id, or the around-window's target
    /// id when no `after` rows landed) so this works whether the caller's
    /// vec is empty or not.
    fn extend_window_to_turn_end(
        conn: &Connection,
        session_id: &str,
        anchor_id: i64,
        messages: &mut Vec<SessionMessage>,
    ) -> Result<()> {
        // First user-row strictly after the anchor — defines the exclusive
        // upper bound. None means we extend to the session's tail.
        let next_user_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM messages
                 WHERE session_id = ?1 AND id > ?2 AND role = 'user'
                 ORDER BY id ASC LIMIT 1",
                params![session_id, anchor_id],
                |row| row.get(0),
            )
            .ok();
        let stop_exclusive: i64 = match next_user_id {
            Some(uid) => uid,
            None => {
                // Treat the row after the session's max id as the cap.
                let max_id: i64 = conn.query_row(
                    "SELECT COALESCE(MAX(id), 0) FROM messages WHERE session_id = ?1",
                    params![session_id],
                    |row| row.get(0),
                )?;
                if max_id <= anchor_id {
                    return Ok(());
                }
                max_id + 1
            }
        };
        if stop_exclusive <= anchor_id + 1 {
            return Ok(());
        }
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking, ttft_ms, tokens_in_last,
                    tokens_cache_creation, tokens_cache_read, tool_metadata, stream_status
             FROM messages
             WHERE session_id = ?1 AND id > ?2 AND id < ?3
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![session_id, anchor_id, stop_exclusive], |row| {
            Self::row_to_session_message(row)
        })?;
        for row in rows {
            messages.push(row?);
        }
        Ok(())
    }

    fn has_messages_before(conn: &Connection, session_id: &str, id: i64) -> bool {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE session_id = ?1 AND id < ?2)",
            params![session_id, id],
            |row| row.get::<_, i64>(0),
        )
        .map(|v| v != 0)
        .unwrap_or(false)
    }

    fn has_messages_after(conn: &Connection, session_id: &str, id: i64) -> bool {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE session_id = ?1 AND id > ?2)",
            params![session_id, id],
            |row| row.get::<_, i64>(0),
        )
        .map(|v| v != 0)
        .unwrap_or(false)
    }

    /// Parse a row produced by `SESSION_META_SELECT` (or that SELECT +
    /// `WHERE ...`) into a `SessionMeta`. Column indices are tied to the
    /// column order declared in the constant — keep them in sync.
    fn row_to_session_meta(row: &rusqlite::Row) -> rusqlite::Result<SessionMeta> {
        let cc_channel_id: Option<String> = row.get(17)?;
        let channel_info = cc_channel_id.map(|ch_id| ChannelSessionInfo {
            channel_id: ch_id,
            account_id: row.get::<_, String>(18).unwrap_or_default(),
            chat_id: row.get::<_, String>(19).unwrap_or_default(),
            chat_type: row.get::<_, String>(20).unwrap_or_default(),
            sender_name: row.get(21).ok().flatten(),
        });
        Ok(SessionMeta {
            id: row.get(0)?,
            title: row.get(1)?,
            title_source: row
                .get::<_, String>(23)
                .unwrap_or_else(|_| crate::session_title::TITLE_SOURCE_MANUAL.to_string()),
            agent_id: row.get(2)?,
            provider_id: row.get(3)?,
            provider_name: row.get(4)?,
            model_id: row.get(5)?,
            temperature: row.get(29).ok().flatten(),
            reasoning_effort: row.get(24).ok().flatten(),
            runtime_defaults_initialized: row.get::<_, i64>(30).unwrap_or(0) != 0,
            pinned_at: row.get(25).ok().flatten(),
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
            message_count: row.get(8)?,
            unread_count: row.get(9)?,
            // Appended near the end of SELECT (index 27) to keep the locked
            // 0..26 positions above stable.
            channel_unread_count: row.get(27)?,
            pending_interaction_count: 0,
            has_error: row.get::<_, i64>(10).unwrap_or(0) != 0,
            is_cron: row.get::<_, i64>(11).unwrap_or(0) != 0,
            parent_session_id: row.get(12)?,
            forked_from_session_id: row.get(33).ok().flatten(),
            forked_from_message_id: row.get(34).ok().flatten(),
            forked_from_session_title: row.get(35).ok().flatten(),
            plan_mode: row
                .get::<_, String>(13)
                .map(|s| crate::plan::PlanModeState::from_str(&s))
                .unwrap_or_default(),
            // Appended after sandbox_mode to keep existing locked positions
            // stable.
            execution_mode: row
                .get::<_, String>(31)
                .map(|s| crate::execution_mode::ExecutionMode::parse_or_default(&s))
                .unwrap_or_default(),
            workflow_mode: row
                .get::<_, String>(32)
                .map(|s| crate::workflow_mode::WorkflowMode::parse_or_default(&s))
                .unwrap_or_default(),
            project_id: row.get(14)?,
            permission_mode: row
                .get::<_, String>(15)
                .map(|s| crate::permission::SessionMode::parse_or_default(&s))
                .unwrap_or_default(),
            sandbox_mode: row
                .get::<_, String>(28)
                .map(|s| crate::permission::SandboxMode::parse_or_default(&s))
                .unwrap_or_default(),
            incognito: row.get::<_, i64>(16).unwrap_or(0) != 0,
            channel_info,
            working_dir: row.get(22).ok().flatten(),
            kind: row
                .get::<_, String>(26)
                .map(|s| SessionKind::from_db_string(&s))
                .unwrap_or_default(),
        })
    }

    pub(crate) fn row_to_session_message(row: &rusqlite::Row) -> rusqlite::Result<SessionMessage> {
        let is_error_val: Option<i64> = row.get(15)?;
        Ok(SessionMessage {
            id: row.get(0)?,
            session_id: row.get(1)?,
            role: MessageRole::from_str(&row.get::<_, String>(2)?),
            content: row.get(3)?,
            timestamp: row.get(4)?,
            attachments_meta: row.get(5)?,
            model: row.get(6)?,
            tokens_in: row.get(7)?,
            tokens_out: row.get(8)?,
            reasoning_effort: row.get(9)?,
            tool_call_id: row.get(10)?,
            tool_name: row.get(11)?,
            tool_arguments: row.get(12)?,
            tool_result: row.get(13)?,
            tool_duration_ms: row.get(14)?,
            is_error: is_error_val.map(|v| v != 0),
            thinking: row.get(16)?,
            ttft_ms: row.get(17)?,
            tokens_in_last: row.get(18)?,
            tokens_cache_creation: row.get(19)?,
            tokens_cache_read: row.get(20)?,
            tool_metadata: row.get::<_, Option<String>>(21).ok().flatten(),
            stream_status: row.get::<_, Option<String>>(22).ok().flatten(),
        })
    }

    /// Append a message to a session and update the session's updated_at.
    pub fn append_message(&self, session_id: &str, msg: &NewMessage) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        let timestamp = if msg.timestamp.is_empty() {
            &now
        } else {
            &msg.timestamp
        };

        conn.execute(
            "INSERT INTO messages (session_id, role, content, timestamp,
                attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                tool_call_id, tool_name, tool_arguments, tool_result,
                tool_duration_ms, is_error, thinking, ttft_ms, tokens_in_last,
                tokens_cache_creation, tokens_cache_read, tool_metadata, stream_status, source,
                queue_request_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            params![
                session_id,
                msg.role.as_str(),
                msg.content,
                timestamp,
                msg.attachments_meta,
                msg.model,
                msg.tokens_in,
                msg.tokens_out,
                msg.reasoning_effort,
                msg.tool_call_id,
                msg.tool_name,
                msg.tool_arguments,
                msg.tool_result,
                msg.tool_duration_ms,
                msg.is_error.map(|b| if b { 1i64 } else { 0i64 }),
                msg.thinking,
                msg.ttft_ms,
                msg.tokens_in_last,
                msg.tokens_cache_creation,
                msg.tokens_cache_read,
                msg.tool_metadata,
                msg.stream_status,
                msg.source,
                msg.queue_request_id,
            ],
        )?;

        let msg_id = conn.last_insert_rowid();

        // Update session's updated_at
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        let resolved_ts = timestamp.to_string();
        drop(conn);

        // Live transcript mirror for hook scripts (design §10): mirror when ANY
        // scope has handlers — the global user/managed registry OR the session
        // cwd's project/local hooks. A repo that ships only `.hope-agent/
        // hooks.json` still runs hooks that read `transcript.jsonl` (they merge
        // in per-cwd via `scopes::resolve_for_cwd`), so a project-only config
        // must keep the mirror current too — gating on `registry::global()`
        // alone left those scripts reading stale/missing history (adversarial
        // review). Never mirror incognito sessions (they leave no on-disk
        // trace). Best-effort: a mirror failure must not fail the message
        // persist. Runs after the conn lock is released because the lookups
        // re-lock it.
        let global_has = !crate::hooks::registry::global().is_empty();
        // Only consult config for the project-scope possibility when the cheap
        // global check didn't already decide it — keeps the no-hooks hot path
        // free of the cwd DB lookup below.
        let project_scope_possible = !global_has && {
            let cfg = crate::config::cached_config();
            cfg.hooks_allow_project_scope && !cfg.disable_all_hooks
        };
        if (global_has || project_scope_possible)
            && !crate::session::lookup_session_meta(Some(session_id))
                .map(|m| m.incognito)
                .unwrap_or(false)
        {
            let cwd_opt = crate::session::effective_session_working_dir(Some(session_id));
            // When only project scope might supply hooks, confirm THIS cwd
            // actually has them before mirroring — avoids writing transcripts
            // for cwds with no hooks just because project scope is globally on.
            let should_mirror = global_has
                || cwd_opt
                    .as_deref()
                    .map(std::path::Path::new)
                    .is_some_and(|c| !crate::hooks::scopes::resolve_for_cwd(Some(c)).is_empty());
            if should_mirror {
                let cwd = cwd_opt.unwrap_or_default();
                crate::hooks::transcript::TranscriptMirror::append_persisted(
                    session_id,
                    msg_id,
                    msg,
                    &resolved_ts,
                    &cwd,
                );
            }
        }

        Ok(msg_id)
    }

    /// Update an existing tool_call message with result, duration, and is_error.
    /// Matches by session_id + tool_call_id to find the original tool_call record.
    pub fn update_tool_result(
        &self,
        session_id: &str,
        call_id: &str,
        result: &str,
        duration_ms: Option<i64>,
        is_error: bool,
    ) -> Result<()> {
        self.update_tool_result_with_metadata(
            session_id,
            call_id,
            result,
            duration_ms,
            is_error,
            None,
        )
    }

    /// Same as [`Self::update_tool_result`] plus a JSON-string `tool_metadata`
    /// payload (file change diff snapshots, line deltas, etc.). When
    /// `metadata` is `None` the column is left untouched so a previously
    /// stored value (e.g. from a partial replay) survives.
    pub fn update_tool_result_with_metadata(
        &self,
        session_id: &str,
        call_id: &str,
        result: &str,
        duration_ms: Option<i64>,
        is_error: bool,
        metadata: Option<&str>,
    ) -> Result<()> {
        self.update_tool_result_with_side_outputs(
            session_id,
            call_id,
            result,
            duration_ms,
            is_error,
            metadata,
            None,
        )
    }

    /// Same as [`Self::update_tool_result_with_metadata`] plus optional
    /// `attachments_meta` side-output. This is used for file/media cards
    /// emitted by tools (`send_attachment`, `image_generate`) so the UI can
    /// restore them from history without feeding media JSON back into model
    /// context via `tool_result`.
    pub fn update_tool_result_with_side_outputs(
        &self,
        session_id: &str,
        call_id: &str,
        result: &str,
        duration_ms: Option<i64>,
        is_error: bool,
        metadata: Option<&str>,
        attachments_meta: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        // Mark `stream_status='completed'` so the next startup sweep
        // doesn't demote this row to `'orphaned'` (see `NewMessage::tool`).
        match (metadata, attachments_meta) {
            (Some(md), Some(att_meta)) => {
                conn.execute(
                    "UPDATE messages SET tool_result = ?1, tool_duration_ms = ?2, is_error = ?3, tool_metadata = ?4, attachments_meta = ?5, stream_status = 'completed'
                     WHERE session_id = ?6 AND tool_call_id = ?7",
                    params![
                        result,
                        duration_ms,
                        if is_error { 1i64 } else { 0i64 },
                        md,
                        att_meta,
                        session_id,
                        call_id
                    ],
                )?;
            }
            (Some(md), None) => {
                conn.execute(
                    "UPDATE messages SET tool_result = ?1, tool_duration_ms = ?2, is_error = ?3, tool_metadata = ?4, stream_status = 'completed'
                     WHERE session_id = ?5 AND tool_call_id = ?6",
                    params![
                        result,
                        duration_ms,
                        if is_error { 1i64 } else { 0i64 },
                        md,
                        session_id,
                        call_id
                    ],
                )?;
            }
            (None, Some(att_meta)) => {
                conn.execute(
                    "UPDATE messages SET tool_result = ?1, tool_duration_ms = ?2, is_error = ?3, attachments_meta = ?4, stream_status = 'completed'
                     WHERE session_id = ?5 AND tool_call_id = ?6",
                    params![
                        result,
                        duration_ms,
                        if is_error { 1i64 } else { 0i64 },
                        att_meta,
                        session_id,
                        call_id
                    ],
                )?;
            }
            (None, None) => {
                conn.execute(
                    "UPDATE messages SET tool_result = ?1, tool_duration_ms = ?2, is_error = ?3, stream_status = 'completed'
                     WHERE session_id = ?4 AND tool_call_id = ?5",
                    params![
                        result,
                        duration_ms,
                        if is_error { 1i64 } else { 0i64 },
                        session_id,
                        call_id
                    ],
                )?;
            }
        }
        Ok(())
    }

    /// Update an in-flight streaming placeholder row's content and status.
    /// `duration_ms` overwrites `tool_duration_ms` when `Some` — used at
    /// thinking-block finalize so the persisted duration reflects the
    /// real time spent, not the ~0ms snapshot taken when the placeholder
    /// was first inserted.
    ///
    /// Called by [`crate::chat_engine::persister::StreamPersister`] in two
    /// modes: throttled mid-stream UPDATE (`status = "streaming"`, content
    /// reflects the current accumulated buffer) and finalize at flush
    /// boundary (`status = "completed"`, content is the final buffer).
    /// Touches `sessions.updated_at` so the sidebar bumps as text streams.
    pub fn update_message_stream_content(
        &self,
        message_id: i64,
        content: &str,
        status: &str,
        duration_ms: Option<i64>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        match duration_ms {
            Some(d) => {
                conn.execute(
                    "UPDATE messages SET content = ?1, stream_status = ?2, tool_duration_ms = ?3 WHERE id = ?4",
                    params![content, status, d, message_id],
                )?;
            }
            None => {
                conn.execute(
                    "UPDATE messages SET content = ?1, stream_status = ?2 WHERE id = ?3",
                    params![content, status, message_id],
                )?;
            }
        }
        conn.execute(
            "UPDATE sessions SET updated_at = ?1
             WHERE id = (SELECT session_id FROM messages WHERE id = ?2)",
            params![now, message_id],
        )?;
        Ok(())
    }

    /// Delete a single message by id. Used at turn end to drop a trailing
    /// `text_block` placeholder once its content has been folded into the
    /// final `assistant` row's `content` — keeping both would double-render
    /// in the UI and double-index in FTS.
    pub fn delete_message_by_id(&self, message_id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute("DELETE FROM messages WHERE id = ?1", params![message_id])?;
        Ok(())
    }

    /// Load the messages that belong to the *previous* turn — the segment
    /// strictly between the second-most-recent `user` row and the most
    /// recent one. Used by crash-recovery summary injection, which runs
    /// AFTER the new user message has already been appended to the table:
    /// the orphaned partial from the prior interrupted turn lives in the
    /// gap between the prior user message (at the bottom of OFFSET 1) and
    /// the just-appended one (at the top of MAX).
    ///
    /// First-turn special case (only one user row exists): the segment
    /// ends at user1's id and starts at 0, returning `[]` — there is no
    /// "previous turn" to surface yet, which is correct.
    ///
    /// No-user case (history-only session): returns everything, since
    /// neither bound applies.
    pub fn load_previous_turn_tail(&self, session_id: &str) -> Result<Vec<SessionMessage>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking, ttft_ms, tokens_in_last,
                    tokens_cache_creation, tokens_cache_read, tool_metadata, stream_status
             FROM messages
             WHERE session_id = ?1
               AND id > COALESCE(
                   (SELECT id FROM messages
                    WHERE session_id = ?1 AND role = 'user'
                    ORDER BY id DESC
                    LIMIT 1 OFFSET 1),
                   0
               )
               AND id < COALESCE(
                   (SELECT MAX(id) FROM messages WHERE session_id = ?1 AND role = 'user'),
                   9223372036854775807
               )
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| Self::row_to_session_message(row))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Rows belonging to the in-flight (not-yet-finalized) turn — i.e.
    /// everything after the **latest** user row in the session.
    ///
    /// Mirror of [`load_previous_turn_tail`] but pointing forward
    /// instead of backward. Used by the finalize path to reverse-rebuild
    /// the partial assistant round (text_block / thinking_block / tool
    /// rows that were still streaming or got orphaned) into
    /// provider-native `context_json` blocks.
    ///
    /// **Bounds.** Returns rows with `id > <latest user.id>`. Order is
    /// ASC by id so caller sees the original interleaving.
    ///
    /// **No-user case.** Returns `[]` — there is no current turn yet.
    pub fn load_current_turn_tail(&self, session_id: &str) -> Result<Vec<SessionMessage>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking, ttft_ms, tokens_in_last,
                    tokens_cache_creation, tokens_cache_read, tool_metadata, stream_status
             FROM messages
             WHERE session_id = ?1
               AND id > COALESCE(
                   (SELECT MAX(id) FROM messages
                    WHERE session_id = ?1 AND role = 'user'),
                   0
               )
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| Self::row_to_session_message(row))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Most-recent user row for a session. `None` if the session has no
    /// user message yet (history-only session).
    pub fn last_user_message(&self, session_id: &str) -> Result<Option<SessionMessage>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking, ttft_ms, tokens_in_last,
                    tokens_cache_creation, tokens_cache_read, tool_metadata, stream_status
             FROM messages
             WHERE session_id = ?1 AND role = 'user'
             ORDER BY id DESC
             LIMIT 1",
        )?;
        let row = stmt
            .query_row(params![session_id], |row| Self::row_to_session_message(row))
            .optional()?;
        Ok(row)
    }

    /// Session ids that still have at least one `messages` row in the
    /// `orphaned` stream_status. Used by the startup sweep to finalize
    /// IM / Cron / subagent sessions whose orphaned partials don't
    /// have a matching `chat_turns` row (those entry points run with
    /// `turn_id = None`).
    pub fn sessions_with_orphaned_rows(&self) -> Result<Vec<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT session_id FROM messages
             WHERE stream_status = 'orphaned'
               AND id > COALESCE(
                   (SELECT MAX(u.id) FROM messages u
                    WHERE u.session_id = messages.session_id AND u.role = 'user'),
                   0
               )",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Sweep leftover `streaming` rows from a previous (crashed) run into
    /// `orphaned`. Called once on app startup, before any session is loaded.
    /// Returns the number of rows promoted, for logging.
    pub fn mark_orphaned_streaming_rows(&self) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let n = conn.execute(
            "UPDATE messages SET stream_status = 'orphaned'
             WHERE stream_status = 'streaming'",
            [],
        )?;
        Ok(n)
    }

    /// Mark the current turn's already-finalized orphaned stream rows as
    /// `recovered`.
    ///
    /// `orphaned` is the startup-sweep input signal. Once finalize has written
    /// the context marker + user-facing event row, keeping the same status makes
    /// the next launch process the same partial again. `recovered` preserves the
    /// UI meaning ("this block came from an interrupted run") without keeping it
    /// in the recovery queue.
    pub fn mark_current_turn_orphaned_rows_recovered(&self, session_id: &str) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let n = conn.execute(
            "UPDATE messages
             SET stream_status = 'recovered'
             WHERE session_id = ?1
               AND stream_status = 'orphaned'
               AND id > COALESCE(
                   (SELECT MAX(id) FROM messages
                    WHERE session_id = ?1 AND role = 'user'),
                   0
               )",
            params![session_id],
        )?;
        Ok(n)
    }

    /// True when an orphaned row in the current turn already has a later
    /// finalize event with the given body. Used by startup recovery to repair
    /// data written before `orphaned` rows were consumed after finalize.
    pub fn current_turn_orphaned_has_later_event(
        &self,
        session_id: &str,
        event_content: &str,
    ) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let exists: i64 = conn.query_row(
            "SELECT EXISTS(
                 SELECT 1
                 FROM messages o
                 WHERE o.session_id = ?1
                   AND o.stream_status = 'orphaned'
                   AND o.id > COALESCE(
                       (SELECT MAX(id) FROM messages
                        WHERE session_id = ?1 AND role = 'user'),
                       0
                   )
                   AND EXISTS (
                       SELECT 1
                       FROM messages e
                       WHERE e.session_id = o.session_id
                         AND e.role = 'event'
                         AND e.content = ?2
                         AND e.id > o.id
                   )
             )",
            params![session_id, event_content],
            |row| row.get(0),
        )?;
        Ok(exists != 0)
    }

    /// Check whether this session already has a `user` row whose
    /// `attachments_meta` references the given subagent `run_id` / async
    /// `job_id`. Used by `inject_and_run_parent` to stay idempotent when a
    /// cancelled injection is re-queued and retried — without this guard,
    /// every retry would append another copy of the push message.
    pub fn has_injection_user_msg(&self, session_id: &str, run_id: &str) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT 1 FROM messages
             WHERE session_id = ?1
               AND role = 'user'
               AND attachments_meta LIKE ?2
             LIMIT 1",
        )?;
        // The attachments_meta JSON always renders run_id as a bare string
        // key-value pair. Matching the quoted form avoids false positives
        // from tokens that happen to contain the id as a substring.
        let pattern = format!("%\"run_id\":\"{}\"%", run_id);
        let exists = stmt.exists(params![session_id, pattern])?;
        Ok(exists)
    }

    /// Update session title.
    pub fn update_session_title(&self, session_id: &str, title: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        // Entering title edit mode and blurring without changing the text is
        // not a manual rename. Preserve the existing source so an immediate
        // first-message fallback remains eligible for LLM refinement.
        conn.execute(
            "UPDATE sessions
                SET title = ?1, title_source = ?2
              WHERE id = ?3 AND (title IS NULL OR title <> ?1)",
            params![title, crate::session_title::TITLE_SOURCE_MANUAL, session_id],
        )?;
        Ok(())
    }

    /// Pin or unpin a session in sidebar listings. This intentionally does
    /// not bump `updated_at` — pinning is an ordering preference, not chat
    /// activity.
    pub fn set_session_pinned(&self, session_id: &str, pinned: bool) -> Result<()> {
        let pinned_at = pinned.then(|| chrono::Utc::now().to_rfc3339());
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET pinned_at = ?1 WHERE id = ?2",
            params![pinned_at, session_id],
        )?;
        Ok(())
    }

    /// Update session title and record its source.
    pub fn update_session_title_with_source(
        &self,
        session_id: &str,
        title: &str,
        title_source: &str,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET title = ?1, title_source = ?2 WHERE id = ?3",
            params![title, title_source, session_id],
        )?;
        Ok(())
    }

    /// Update a session title only if its source still matches an expected
    /// value. Returns true when the row was updated.
    pub fn update_session_title_if_source(
        &self,
        session_id: &str,
        expected_source: &str,
        title: &str,
        title_source: &str,
    ) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE sessions
                SET title = ?1, title_source = ?2
              WHERE id = ?3 AND title_source = ?4",
            params![title, title_source, session_id, expected_source],
        )?;
        Ok(changed > 0)
    }

    /// Reclassify a title without changing its text, guarded by both the
    /// current source and current title. This is used to repair legacy
    /// auto-generated Goal fallbacks without racing a real user rename.
    pub(crate) fn update_session_title_source_if_title_and_source(
        &self,
        session_id: &str,
        expected_title: &str,
        expected_source: &str,
        title_source: &str,
    ) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE sessions
                SET title_source = ?1
              WHERE id = ?2 AND title = ?3 AND title_source = ?4",
            params![title_source, session_id, expected_title, expected_source],
        )?;
        Ok(changed > 0)
    }

    /// Mark a session as a cron-triggered session.
    pub fn mark_session_cron(&self, session_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET is_cron = 1 WHERE id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// Update session's provider/model info.
    pub fn update_session_model(
        &self,
        session_id: &str,
        provider_id: Option<&str>,
        provider_name: Option<&str>,
        model_id: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET provider_id = ?1, provider_name = ?2, model_id = ?3, runtime_defaults_initialized = 1 WHERE id = ?4",
            params![provider_id, provider_name, model_id, session_id],
        )?;
        Ok(())
    }

    /// Return every persisted Session model preference, including hidden,
    /// cron, channel and sub-agent rows. Provider hard-delete repair must not
    /// leave any execution surface pointing at a removed model.
    pub fn list_session_model_preferences(&self) -> Result<Vec<(String, String, String, String)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, agent_id, provider_id, model_id
             FROM sessions
             WHERE provider_id IS NOT NULL AND model_id IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Update the Session-fixed temperature. `None` is a real snapshot of the
    /// provider-native default, so the initialized marker is always set.
    pub fn update_session_temperature(
        &self,
        session_id: &str,
        temperature: Option<f64>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET temperature = ?1, runtime_defaults_initialized = 1 WHERE id = ?2",
            params![temperature, session_id],
        )?;
        Ok(())
    }

    /// Lazily snapshot missing defaults for a legacy Session. Existing model
    /// and Think preferences win; temperature is new and is always captured.
    pub fn initialize_session_runtime_defaults(
        &self,
        session_id: &str,
        provider_id: Option<&str>,
        provider_name: Option<&str>,
        model_id: Option<&str>,
        temperature: Option<f64>,
        reasoning_effort: &str,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions
             SET provider_id = COALESCE(provider_id, ?1),
                 provider_name = COALESCE(provider_name, ?2),
                 model_id = COALESCE(model_id, ?3),
                 temperature = ?4,
                 reasoning_effort = COALESCE(reasoning_effort, ?5),
                 runtime_defaults_initialized = 1
             WHERE id = ?6 AND runtime_defaults_initialized = 0",
            params![
                provider_id,
                provider_name,
                model_id,
                temperature,
                reasoning_effort,
                session_id
            ],
        )?;
        Ok(())
    }

    /// Update the session-scoped Think / reasoning effort override.
    pub fn update_session_reasoning_effort(
        &self,
        session_id: &str,
        reasoning_effort: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET reasoning_effort = ?1, runtime_defaults_initialized = 1 WHERE id = ?2",
            params![reasoning_effort, session_id],
        )?;
        Ok(())
    }

    /// Update the plan mode state for a session.
    ///
    /// Also bumps `updated_at` so Dashboard's plan-stats execution-duration
    /// query can use `session.updated_at` as a proxy for "when did this plan
    /// reach `completed`?" — the plan markdown file mtime stops moving once
    /// the model approves the plan, so it's an unreliable signal.
    pub fn update_session_plan_mode(
        &self,
        session_id: &str,
        plan_mode: crate::plan::PlanModeState,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET plan_mode = ?1, updated_at = ?2 WHERE id = ?3",
            params![plan_mode.as_str(), now, session_id],
        )?;
        Ok(())
    }

    /// Persist `executing_started_at` so the `maybe_complete_plan` scoping
    /// logic survives restore from DB / app restart. `None` clears the stamp
    /// (used when the plan exits Executing).
    pub fn update_session_plan_executing_started_at(
        &self,
        session_id: &str,
        started_at: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET plan_executing_started_at = ?1 WHERE id = ?2",
            params![started_at, session_id],
        )?;
        Ok(())
    }

    pub fn get_session_plan_executing_started_at(
        &self,
        session_id: &str,
    ) -> Result<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt =
            conn.prepare("SELECT plan_executing_started_at FROM sessions WHERE id = ?1")?;
        let mut rows = stmt.query(params![session_id])?;
        match rows.next()? {
            Some(row) => Ok(row.get(0)?),
            None => Ok(None),
        }
    }

    /// Persist the session permission mode (`default` / `smart` / `yolo`)
    /// so the chat title bar's mode switcher is restored on revisit.
    pub fn update_session_permission_mode(
        &self,
        session_id: &str,
        mode: crate::permission::SessionMode,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET permission_mode = ?1 WHERE id = ?2",
            params![mode.as_str(), session_id],
        )?;
        Ok(())
    }

    /// Narrow read of just `sessions.permission_mode` — avoids the full
    /// `SESSION_META_SELECT` (24+ cols, 2 COUNT subqueries, channel JOIN)
    /// when callers only need the mode (e.g. `/permission` echo).
    pub fn get_session_permission_mode(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::permission::SessionMode>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare("SELECT permission_mode FROM sessions WHERE id = ?1")?;
        let row = match stmt.query_row(params![session_id], |row| row.get::<_, String>(0)) {
            Ok(s) => Some(s),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(anyhow::anyhow!("DB error: {}", e)),
        };
        Ok(row.map(|s| crate::permission::SessionMode::parse_or_default(&s)))
    }

    /// Persist the session sandbox mode (`off` / `standard` / `isolated` /
    /// `workspace` / `trusted`) so future tool calls use the selected execution
    /// posture.
    pub fn update_session_sandbox_mode(
        &self,
        session_id: &str,
        mode: crate::permission::SandboxMode,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET sandbox_mode = ?1 WHERE id = ?2",
            params![mode.as_str(), session_id],
        )?;
        Ok(())
    }

    /// Narrow read of just `sessions.sandbox_mode`.
    pub fn get_session_sandbox_mode(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::permission::SandboxMode>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare("SELECT sandbox_mode FROM sessions WHERE id = ?1")?;
        let row = match stmt.query_row(params![session_id], |row| row.get::<_, String>(0)) {
            Ok(s) => Some(s),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(anyhow::anyhow!("DB error: {}", e)),
        };
        Ok(row.map(|s| crate::permission::SandboxMode::parse_or_default(&s)))
    }

    /// Persist the session execution mode policy (`off` / `guarded` / `deep` /
    /// `autonomous`) so `/mode` survives refreshes and all chat entry points
    /// inject the same behavior contract.
    pub fn update_session_execution_mode(
        &self,
        session_id: &str,
        mode: crate::execution_mode::ExecutionMode,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let affected = conn.execute(
            "UPDATE sessions SET execution_mode = ?1, updated_at = ?2 WHERE id = ?3",
            params![mode.as_str(), now, session_id],
        )?;
        if affected == 0 {
            return Err(anyhow::anyhow!("Session not found: {}", session_id));
        }
        Ok(())
    }

    /// Narrow read of just `sessions.execution_mode`.
    pub fn get_session_execution_mode(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::execution_mode::ExecutionMode>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare("SELECT execution_mode FROM sessions WHERE id = ?1")?;
        let row = match stmt.query_row(params![session_id], |row| row.get::<_, String>(0)) {
            Ok(s) => Some(s),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(anyhow::anyhow!("DB error: {}", e)),
        };
        Ok(row.map(|s| crate::execution_mode::ExecutionMode::parse_or_default(&s)))
    }

    /// Persist the session workflow autonomy mode (`off` / `on` /
    /// `ultracode`) so `/workflow on` survives refreshes and all chat entry
    /// points expose the same workflow tool/prompt contract.
    pub fn update_session_workflow_mode(
        &self,
        session_id: &str,
        mode: crate::workflow_mode::WorkflowMode,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let incognito = conn
            .query_row(
                "SELECT incognito FROM sessions WHERE id = ?1",
                params![session_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(incognito) = incognito else {
            return Err(anyhow::anyhow!("Session not found: {}", session_id));
        };
        if mode.enabled() && incognito != 0 {
            return Err(anyhow::anyhow!(
                "Cannot enable workflow mode on an incognito session"
            ));
        }
        let affected = conn.execute(
            "UPDATE sessions SET workflow_mode = ?1, updated_at = ?2 WHERE id = ?3",
            params![mode.as_str(), now, session_id],
        )?;
        if affected == 0 {
            return Err(anyhow::anyhow!("Session not found: {}", session_id));
        }
        Ok(())
    }

    /// Narrow read of just `sessions.workflow_mode`.
    pub fn get_session_workflow_mode(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::workflow_mode::WorkflowMode>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare("SELECT workflow_mode FROM sessions WHERE id = ?1")?;
        let row = match stmt.query_row(params![session_id], |row| row.get::<_, String>(0)) {
            Ok(s) => Some(s),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(anyhow::anyhow!("DB error: {}", e)),
        };
        Ok(row.map(|s| crate::workflow_mode::WorkflowMode::parse_or_default(&s)))
    }

    /// Persist the session-scoped incognito mode flag.
    ///
    /// Refuses to enable incognito on Project / IM Channel sessions: project
    /// sessions need durable history (memory + file injection across runs);
    /// IM channel sessions are driven by an external counterparty whose
    /// messages must not vanish on close.
    pub fn update_session_incognito(&self, session_id: &str, incognito: bool) -> Result<()> {
        if incognito {
            let meta = self
                .get_session(session_id)?
                .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;
            if meta.project_id.is_some() {
                return Err(anyhow::anyhow!(
                    "Cannot enable incognito on a Project session"
                ));
            }
            if meta.channel_info.is_some() {
                return Err(anyhow::anyhow!(
                    "Cannot enable incognito on an IM Channel session"
                ));
            }
            if meta.workflow_mode.enabled() {
                return Err(anyhow::anyhow!(
                    "Cannot enable incognito while Workflow Mode is enabled"
                ));
            }
            let conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            let open_goal: Option<String> = conn
                .query_row(
                    "SELECT id FROM goals
                     WHERE session_id = ?1
                       AND (
                            state IN ('active','paused','evaluating','blocked')
                            OR (state = 'completed' AND closure_decision IS NULL)
                       )
                     LIMIT 1",
                    params![session_id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(goal_id) = open_goal {
                return Err(anyhow::anyhow!(
                    "Cannot enable incognito while session has an open Goal {}",
                    goal_id
                ));
            }
            let workflow_run: Option<String> = conn
                .query_row(
                    "SELECT id FROM workflow_runs WHERE session_id = ?1 LIMIT 1",
                    params![session_id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(run_id) = workflow_run {
                return Err(anyhow::anyhow!(
                    "Cannot enable incognito after workflow run {} was created",
                    run_id
                ));
            }
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let affected = conn.execute(
            "UPDATE sessions SET incognito = ?1 WHERE id = ?2",
            params![incognito, session_id],
        )?;
        if affected == 0 {
            return Err(anyhow::anyhow!("Session not found: {}", session_id));
        }
        if incognito {
            crate::memory_extract::cancel_idle_extraction(session_id);
        }
        Ok(())
    }

    /// Persist the session-scoped working directory.
    ///
    /// When `working_dir` is `Some(path)`, the path is canonicalized and must
    /// resolve to an existing directory; the canonical form is stored so the
    /// model sees a stable absolute path. `None` clears the selection.
    ///
    /// Project / Incognito sessions may set a working directory — the two
    /// concepts are orthogonal (Project = curated knowledge container,
    /// working directory = where file ops default to).
    pub fn update_session_working_dir(
        &self,
        session_id: &str,
        working_dir: Option<String>,
    ) -> Result<Option<String>> {
        let canonical = crate::util::canonicalize_working_dir(working_dir.as_deref())?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        // Read the prior value so the CwdChanged hook only fires on a real change.
        let old: Option<String> = conn
            .query_row(
                "SELECT working_dir FROM sessions WHERE id = ?1",
                params![session_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten();
        conn.execute(
            "UPDATE sessions SET working_dir = ?1 WHERE id = ?2",
            params![canonical, session_id],
        )?;
        drop(conn);
        app_info!(
            "session",
            "update_session_working_dir",
            "session={} working_dir={}",
            session_id,
            canonical.as_deref().unwrap_or("<none>")
        );
        // CwdChanged hook (observation): only when the path actually changed.
        if old != canonical {
            crate::hooks::fire_cwd_changed(session_id, old.as_deref(), canonical.as_deref());
        }
        Ok(canonical)
    }

    /// Return `(user_count, assistant_count)` for a session without loading
    /// the message bodies. Used by `/status` and other quick stats paths
    /// where the full conversation isn't needed.
    pub fn count_user_assistant_messages(&self, session_id: &str) -> Result<(i64, i64)> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT role, COUNT(*) FROM messages
             WHERE session_id = ?1 AND role IN ('user', 'assistant')
             GROUP BY role",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut user = 0;
        let mut assistant = 0;
        for r in rows {
            let (role, n) = r?;
            match role.as_str() {
                "user" => user = n,
                "assistant" => assistant = n,
                _ => {}
            }
        }
        Ok((user, assistant))
    }

    /// Token snapshot for the latest persisted assistant message.
    ///
    /// Used by `/status` to render Context usage + Cache info in lockstep with
    /// the GUI session-status popover (see `ChatTitleBar` `getContextUsageTokens`).
    /// Single SQL covers both panels — context-window usage prefers
    /// `tokens_in_last` (last-round input) and falls back to cumulative
    /// `tokens_in` for legacy rows; cache fields are last-round only and never
    /// summed across turns.
    pub fn get_session_last_assistant_token_row(
        &self,
        session_id: &str,
    ) -> Result<Option<LastAssistantTokens>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let row = conn
            .query_row(
                "SELECT tokens_in, tokens_in_last, tokens_cache_creation, tokens_cache_read, model
                 FROM messages
                 WHERE session_id = ?1 AND role = 'assistant'
                 ORDER BY id DESC
                 LIMIT 1",
                params![session_id],
                |row| {
                    Ok(LastAssistantTokens {
                        tokens_in: row.get::<_, Option<i64>>(0)?,
                        tokens_in_last: row.get::<_, Option<i64>>(1)?,
                        tokens_cache_creation: row.get::<_, Option<i64>>(2)?,
                        tokens_cache_read: row.get::<_, Option<i64>>(3)?,
                        model: row.get::<_, Option<String>>(4)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Change the agent bound to a session. Only allowed when the session has
    /// no user/assistant messages yet — switching agent mid-conversation would
    /// leave the system prompt and message history out of sync. Front-end
    /// should disable the control once messages exist; this is the SQL-level
    /// defense.
    pub fn update_session_agent(&self, session_id: &str, agent_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND role IN ('user', 'assistant')",
            params![session_id],
            |row| row.get(0),
        )?;
        if count > 0 {
            return Err(anyhow::anyhow!(
                "cannot change agent for session {}: already has {} messages",
                session_id,
                count
            ));
        }
        let updated = conn.execute(
            "UPDATE sessions SET agent_id = ?1 WHERE id = ?2",
            params![agent_id, session_id],
        )?;
        if updated == 0 {
            return Err(anyhow::anyhow!("session {} not found", session_id));
        }
        app_info!(
            "session",
            "update_session_agent",
            "session={} agent_id={}",
            session_id,
            agent_id
        );
        Ok(())
    }

    /// Persist plan step statuses to DB for crash recovery.
    pub fn save_plan_steps(&self, session_id: &str, steps_json: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET plan_steps = ?1 WHERE id = ?2",
            params![steps_json, session_id],
        )?;
        Ok(())
    }

    /// Load persisted plan step statuses from DB.
    pub fn load_plan_steps(&self, session_id: &str) -> Result<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare("SELECT plan_steps FROM sessions WHERE id = ?1")?;
        let result = stmt.query_row(params![session_id], |row| row.get::<_, Option<String>>(0))?;
        Ok(result)
    }

    /// Hard-delete the session if (and only if) it is currently flagged
    /// incognito. Returns `true` when a delete occurred. No-op when the
    /// session does not exist or is not incognito — both are safe outcomes
    /// for the "user navigated away from this session" caller.
    pub fn purge_session_if_incognito(&self, session_id: &str) -> Result<bool> {
        // Snapshot before the DELETE for the purge event payload (row is gone
        // afterwards). Only emitted below when a row was actually removed.
        let snapshot = self.get_session(session_id)?;
        // G4: capture descendant subagent sessions before the cascade drops
        // `subagent_runs`, so the burn denies their inner-tool approvals too.
        // (im_chat is always None here — incognito ⊥ IM Channel.)
        let cleanup_ctx = self.capture_session_cleanup_context(session_id);
        let was_incognito = {
            let conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            conn.execute(
                "DELETE FROM sessions WHERE id = ?1 AND incognito = 1",
                params![session_id],
            )? > 0
        };
        if !was_incognito {
            return Ok(false);
        }
        // Mirror the on-disk + orphan-table cleanup that `delete_session`
        // performs. The session row itself is already gone, so we only need
        // the side-effect cleanup (skipping the FTS-rebuild fallback path).
        if let Ok(plans_dir) = crate::paths::plans_dir() {
            let _ = std::fs::remove_file(plans_dir.join(format!("{}.md", session_id)));
        }
        if let Ok(att_dir) = crate::paths::attachments_dir(session_id) {
            let _ = std::fs::remove_dir_all(att_dir);
        }
        self.cleanup_session_orphan_tables(session_id);
        // Mirror delete_session_with_reason: drop the Smart-mode "already edited"
        // trust set so a burned incognito session leaves no in-memory trace
        // (burn-on-close must not survive in the per-session edit-trust map).
        crate::permission::session_edits::clear(session_id);
        app_info!(
            "session",
            "purge_incognito",
            "purged incognito session {}",
            session_id
        );
        // Emit `session:purged` after lock release for cleanup_watcher fan-out.
        if let Some(meta) = snapshot {
            crate::session::events::emit_session_deleted(
                &meta,
                crate::session::events::SessionDeleteReason::IncognitoPurge,
                &cleanup_ctx,
            );
        }
        Ok(true)
    }

    /// Drain rows that reference `session_id` in tables without FK cascade
    /// (`session_skill_activation`, `session_tool_activation`, `learning_events`, `subagent_runs`,
    /// `acp_runs`). Bundled in a single transaction to amortize fsync.
    /// Best-effort: failures are logged via `app_warn!` so a corrupted side
    /// table never blocks the primary delete.
    fn cleanup_session_orphan_tables(&self, session_id: &str) {
        let Ok(conn) = self.conn.lock() else {
            return;
        };
        let run = || -> rusqlite::Result<()> {
            conn.execute_batch("BEGIN")?;
            for sql in [
                "DELETE FROM session_skill_activation WHERE session_id = ?1",
                "DELETE FROM session_tool_activation WHERE session_id = ?1",
                "DELETE FROM learning_events WHERE session_id = ?1",
                "DELETE FROM subagent_runs WHERE parent_session_id = ?1",
                "DELETE FROM acp_runs WHERE parent_session_id = ?1",
            ] {
                conn.execute(sql, params![session_id])?;
            }
            conn.execute_batch("COMMIT")?;
            Ok(())
        };
        if let Err(e) = run() {
            let _ = conn.execute_batch("ROLLBACK");
            app_warn!(
                "session",
                "db",
                "orphan-table cleanup failed for {}: {}",
                session_id,
                e
            );
        }
    }

    /// Drain every incognito session left behind from a previous run (crash,
    /// SIGKILL, power loss). Called once during app startup before the
    /// sidebar reads the session list. Returns the number of sessions
    /// purged.
    pub fn purge_orphan_incognito_sessions(&self) -> Result<usize> {
        // Defense in depth: even when this runs (Primary tier in
        // app_init), exclude sessions that were touched in the last
        // 60 seconds. A coexisting live process could have just
        // created or written to one — the runtime_lock guard is the
        // primary safeguard, this is the second.
        // updated_at is stored as RFC3339; ISO format compares
        // lexicographically the same as chronologically.
        let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(60)).to_rfc3339();
        let ids: Vec<String> = {
            let conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            let mut stmt =
                conn.prepare("SELECT id FROM sessions WHERE incognito = 1 AND updated_at < ?1")?;
            let rows = stmt.query_map(rusqlite::params![cutoff], |r| r.get::<_, String>(0))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        for id in &ids {
            if let Err(e) = self.delete_session_with_reason(
                id,
                crate::session::events::SessionDeleteReason::OrphanSweep,
            ) {
                app_warn!(
                    "session",
                    "purge_orphan_incognito",
                    "failed to delete orphan incognito session {}: {}",
                    id,
                    e
                );
            }
        }
        if !ids.is_empty() {
            app_info!(
                "session",
                "purge_orphan_incognito",
                "swept {} orphan incognito session(s) at startup",
                ids.len()
            );
        }
        Ok(ids.len())
    }

    /// Delete a session and all its messages (CASCADE) and attachments.
    ///
    /// Thin wrapper over [`Self::delete_session_with_reason`] tagging the
    /// emitted lifecycle event as a user delete.
    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        self.delete_session_with_reason(
            session_id,
            crate::session::events::SessionDeleteReason::UserDelete,
        )
    }

    /// Delete a session and all its messages (CASCADE) and attachments, tagging
    /// the emitted `session:deleted` event with `reason`.
    ///
    /// Captures a pre-delete `SessionMeta` snapshot (the row is gone by emit
    /// time, but the event payload needs `agent_id` / `incognito`). After the
    /// row + side effects are removed and the conn lock is released, emits the
    /// lifecycle event so `cleanup_watcher` can fan out coordinated cleanup of
    /// in-memory subsystems. No event is emitted when the session did not exist.
    pub fn delete_session_with_reason(
        &self,
        session_id: &str,
        reason: crate::session::events::SessionDeleteReason,
    ) -> Result<()> {
        // Snapshot before deletion — needed for the emit payload, and lets us
        // skip the event entirely when nothing was there to delete.
        let snapshot = self.get_session(session_id)?;
        // G4 / SURFACE-2: capture cleanup context BEFORE the cascade, while the
        // `subagent_runs` and `channel_conversations` rows still exist (both are
        // gone by emit time). Done before taking the conn lock — both helpers
        // lock it themselves.
        let cleanup_ctx = self.capture_session_cleanup_context(session_id);
        {
            let conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            // Try direct delete (CASCADE handles messages + fires FTS trigger).
            match conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id]) {
                Ok(_) => {}
                Err(e) => {
                    // FTS index corrupted — rebuild and retry.
                    app_warn!(
                        "session",
                        "db",
                        "delete_session failed ({}), rebuilding FTS and retrying",
                        e
                    );
                    let _ = conn.execute_batch(
                        "INSERT INTO messages_fts(messages_fts) VALUES('rebuild');
                         INSERT INTO messages_trigram_fts(messages_trigram_fts) VALUES('rebuild');",
                    );
                    conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;
                }
            }
        }

        if let Ok(plans_dir) = crate::paths::plans_dir() {
            let _ = std::fs::remove_file(plans_dir.join(format!("{}.md", session_id)));
        }
        if let Ok(att_dir) = crate::paths::attachments_dir(session_id) {
            let _ = std::fs::remove_dir_all(att_dir);
        }
        self.cleanup_session_orphan_tables(session_id);
        // Drop the Smart-mode "already edited" trust set for this session so it
        // can't outlive the session (and doesn't accumulate in long-running
        // server processes).
        crate::permission::session_edits::clear(session_id);

        // Emit after the conn lock is released — subscribers (cleanup_watcher
        // fan-out) may re-lock the DB.
        if let Some(meta) = snapshot {
            crate::session::events::emit_session_deleted(&meta, reason, &cleanup_ctx);
        }

        Ok(())
    }

    /// Snapshot the pre-delete cleanup context for a session (G4 / SURFACE-2):
    /// its transitive subagent descendant sessions + its IM attach coordinates.
    /// Both reference rows the delete cascade removes, so this MUST run before
    /// the DELETE. Best-effort: any lookup failure yields an empty/None field.
    fn capture_session_cleanup_context(
        &self,
        session_id: &str,
    ) -> crate::session::events::SessionCleanupContext {
        let descendant_session_ids = self.collect_descendant_session_ids(session_id);
        let im_chat = crate::globals::get_channel_db()
            .and_then(|cdb| cdb.get_conversation_by_session(session_id).ok().flatten())
            .map(|c| (c.account_id, c.chat_id));
        crate::session::events::SessionCleanupContext {
            descendant_session_ids,
            im_chat,
        }
    }

    /// Persist or update the set of conditional skills activated for a session.
    /// Returns the actually-new activations (after diffing against DB).
    pub fn insert_skill_activations(
        &self,
        session_id: &str,
        skill_names: &[String],
    ) -> Result<Vec<String>> {
        if skill_names.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        let mut added = Vec::new();
        for name in skill_names {
            let changed = conn.execute(
                "INSERT OR IGNORE INTO session_skill_activation (session_id, skill_name, activated_at)
                 VALUES (?1, ?2, ?3)",
                params![session_id, name, now],
            )?;
            if changed > 0 {
                added.push(name.clone());
            }
        }
        Ok(added)
    }

    /// Load the set of activated conditional skills for a session.
    pub fn load_skill_activations(&self, session_id: &str) -> Result<Vec<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt =
            conn.prepare("SELECT skill_name FROM session_skill_activation WHERE session_id = ?1")?;
        let rows = stmt.query_map(params![session_id], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Persist deferred tools activated through `tool_search`. This is not an
    /// authorization grant: every request and execution re-applies the live
    /// dispatcher/permission filters before exposing or calling the tool.
    pub fn insert_tool_activations(
        &self,
        session_id: &str,
        tool_names: &[String],
    ) -> Result<Vec<String>> {
        if tool_names.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        let mut added = Vec::new();
        for name in tool_names {
            let changed = conn.execute(
                "INSERT OR IGNORE INTO session_tool_activation (session_id, tool_name, activated_at)
                 VALUES (?1, ?2, ?3)",
                params![session_id, name, now],
            )?;
            if changed > 0 {
                added.push(name.clone());
            }
        }
        Ok(added)
    }

    pub fn load_tool_activations(&self, session_id: &str) -> Result<Vec<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT tool_name FROM session_tool_activation
             WHERE session_id = ?1 ORDER BY activated_at, tool_name",
        )?;
        let rows = stmt.query_map(params![session_id], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn clear_tool_activations(&self, session_id: &str) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        Ok(conn.execute(
            "DELETE FROM session_tool_activation WHERE session_id = ?1",
            params![session_id],
        )?)
    }

    pub fn get_memory_policy(&self, session_id: &str) -> Result<SessionMemoryPolicy> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let session_exists = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1)",
            params![session_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !session_exists {
            anyhow::bail!("session not found: {session_id}");
        }
        let row = conn
            .query_row(
                "SELECT use_memories, contribute_to_memories
                 FROM session_memory_policy WHERE session_id = ?1",
                params![session_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        Ok(row.map_or_else(
            SessionMemoryPolicy::default,
            |(use_memories, contribute)| SessionMemoryPolicy {
                use_memories: SessionMemoryPolicyValue::parse(&use_memories),
                contribute_to_memories: SessionMemoryPolicyValue::parse(&contribute),
            },
        ))
    }

    pub fn set_memory_policy(
        &self,
        session_id: &str,
        policy: SessionMemoryPolicy,
    ) -> Result<SessionMemoryPolicy> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "INSERT INTO session_memory_policy (
                session_id, use_memories, contribute_to_memories, updated_at
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO UPDATE SET
                use_memories = excluded.use_memories,
                contribute_to_memories = excluded.contribute_to_memories,
                updated_at = excluded.updated_at",
            params![
                session_id,
                policy.use_memories.as_str(),
                policy.contribute_to_memories.as_str(),
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        if changed == 0 {
            anyhow::bail!("session not found: {session_id}");
        }
        Ok(policy)
    }

    /// Save the agent's conversation_history JSON for a session.
    pub fn save_context(&self, session_id: &str, context_json: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET context_json = ?1 WHERE id = ?2",
            params![context_json, session_id],
        )?;
        Ok(())
    }

    /// Save context only if the DB value still matches the caller's snapshot.
    pub fn save_context_if_unchanged(
        &self,
        session_id: &str,
        expected_context_json: Option<&str>,
        context_json: &str,
    ) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let changed = if let Some(expected) = expected_context_json {
            conn.execute(
                "UPDATE sessions
                 SET context_json = ?1
                 WHERE id = ?2 AND context_json = ?3",
                params![context_json, session_id, expected],
            )?
        } else {
            conn.execute(
                "UPDATE sessions
                 SET context_json = ?1
                 WHERE id = ?2 AND context_json IS NULL",
                params![context_json, session_id],
            )?
        };

        Ok(changed > 0)
    }

    /// Load the agent's conversation_history JSON for a session.
    /// Returns None if the session has no saved context.
    pub fn load_context(&self, session_id: &str) -> Result<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare("SELECT context_json FROM sessions WHERE id = ?1")?;
        let result = stmt
            .query_row(params![session_id], |row| row.get::<_, Option<String>>(0))
            .ok()
            .flatten();
        Ok(result)
    }

    /// List the ids of every session assigned to `project_id` (any status /
    /// kind). Used by project deletion to cancel each session's in-flight async
    /// jobs *before* the sessions are unassigned — afterwards there's no
    /// `project_id` link to find them and they would run on orphaned. Epic E
    /// (DELETE-6).
    pub fn session_ids_in_project(&self, project_id: &str) -> Result<Vec<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare("SELECT id FROM sessions WHERE project_id = ?1")?;
        let rows = stmt.query_map(params![project_id], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Get a single session's metadata.
    pub fn get_session(&self, session_id: &str) -> Result<Option<SessionMeta>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let sql = format!("{} WHERE s.id = ?1", SESSION_META_SELECT);
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query_map(params![session_id], Self::row_to_session_meta)?;

        match rows.next() {
            Some(Ok(meta)) => Ok(Some(meta)),
            Some(Err(e)) => Err(anyhow::anyhow!("DB error: {}", e)),
            None => Ok(None),
        }
    }

    /// Mark all messages in a session as read by updating last_read_message_id
    /// to the current maximum message id.
    pub fn mark_session_read(&self, session_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET last_read_message_id = (SELECT COALESCE(MAX(id), 0) FROM messages WHERE session_id = ?1) WHERE id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// Mark all messages in multiple sessions as read.
    pub fn mark_session_read_batch(&self, session_ids: &[String]) -> Result<()> {
        if session_ids.is_empty() {
            return Ok(());
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "UPDATE sessions SET last_read_message_id = (SELECT COALESCE(MAX(id), 0) FROM messages WHERE session_id = ?1) WHERE id = ?1"
        )?;
        for id in session_ids {
            stmt.execute(params![id])?;
        }
        Ok(())
    }

    /// Mark every regular chat session in a project as read.
    pub fn mark_project_sessions_read(&self, project_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions
                SET last_read_message_id = (
                    SELECT COALESCE(MAX(id), 0)
                    FROM messages
                    WHERE messages.session_id = sessions.id
                )
              WHERE project_id = ?1
                AND parent_session_id IS NULL
                AND NOT EXISTS (
                    SELECT 1
                    FROM channel_conversations cc
                    WHERE cc.session_id = sessions.id
                )",
            params![project_id],
        )?;
        Ok(())
    }

    /// Mark all sessions as read.
    pub fn mark_all_sessions_read(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute_batch(
            "UPDATE sessions SET last_read_message_id = (SELECT COALESCE(MAX(id), 0) FROM messages WHERE messages.session_id = sessions.id)"
        )?;
        Ok(())
    }

    // ── Cron timeline / unread (cron panel "conversations" view) ─────────────

    /// Batch-fetch `(title, unread_assistant_count)` for the given cron session
    /// ids — used to hydrate the cross-job run timeline (`cron_run_timeline`).
    /// Returns a map `session_id -> (title, unread)`; ids whose session row is
    /// missing (purged) are simply absent, and the caller falls back to the job
    /// name / 0. The unread predicate mirrors `SESSION_META_SELECT` *minus* the
    /// `is_cron = 0` clause (cron sessions are never sub-sessions and their
    /// `source` is always desktop, so those predicates are unnecessary here).
    pub fn cron_session_read_state(
        &self,
        session_ids: &[String],
    ) -> Result<std::collections::HashMap<String, (Option<String>, i64)>> {
        use std::collections::HashMap;
        if session_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        // ids come from our own run-log page, bounded by the page limit, so the
        // IN-list stays small.
        let placeholders: Vec<String> = (1..=session_ids.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT s.id, s.title,
                    (SELECT COUNT(*) FROM messages m
                      WHERE m.session_id = s.id
                        AND m.id > COALESCE(s.last_read_message_id, 0)
                        AND m.role = 'assistant') AS unread
             FROM sessions s
             WHERE s.is_cron = 1 AND s.id IN ({})",
            placeholders.join(",")
        );
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = session_ids
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();
        let mut map = HashMap::new();
        let mut rows = stmt.query(params.as_slice())?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let title: Option<String> = row.get(1)?;
            let unread: i64 = row.get(2)?;
            map.insert(id, (title, unread));
        }
        Ok(map)
    }

    /// Total unread assistant messages across ALL cron sessions (the cron
    /// sidebar badge count). The backend does NOT exclude the currently-open
    /// cron session — that's a pure aggregate; the frontend handles any
    /// "currently viewing" subtraction.
    pub fn cron_unread_total(&self) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let total: i64 = conn.query_row(
            "SELECT COUNT(*)
               FROM messages m
               JOIN sessions s ON s.id = m.session_id
              WHERE s.is_cron = 1
                AND m.role = 'assistant'
                AND m.id > COALESCE(s.last_read_message_id, 0)",
            [],
            |row| row.get(0),
        )?;
        Ok(total)
    }

    /// Mark every cron session as read (badge → 0). Mirrors `mark_session_read`'s
    /// `last_read_message_id = MAX(message id)` logic, scoped to `is_cron = 1`.
    /// Returns the number of sessions updated.
    pub fn mark_all_cron_sessions_read(&self) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let n = conn.execute(
            "UPDATE sessions
                SET last_read_message_id = (
                    SELECT COALESCE(MAX(id), 0) FROM messages
                     WHERE messages.session_id = sessions.id
                )
              WHERE is_cron = 1",
            [],
        )?;
        Ok(n)
    }

    // ── History Search ──────────────────────────────────────────

    /// True when a rusqlite error signals an FTS shadow-table
    /// corruption ("database disk image is malformed" / `SQLITE_CORRUPT`) — the
    /// trigger to rebuild the FTS indexes.
    fn is_fts_corruption(e: &rusqlite::Error) -> bool {
        match e {
            rusqlite::Error::SqliteFailure(f, msg) => {
                f.code == rusqlite::ErrorCode::DatabaseCorrupt
                    || msg
                        .as_deref()
                        .is_some_and(|m| m.contains("malformed") || m.contains("corrupt"))
            }
            _ => false,
        }
    }

    /// Rebuild message FTS indexes via the writer connection. The open-time rebuild
    /// now runs only once (gated by the `user_version` sentinel), so this is the
    /// on-demand recovery path for corruption that develops afterward — invoked
    /// from the read path when a query hits `SQLITE_CORRUPT`. Cheaper than the
    /// old unconditional every-open rebuild: it fires only on real corruption,
    /// and uses the writer because the read pool is `READ_ONLY`.
    fn rebuild_fts(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute_batch(
            "INSERT INTO messages_fts(messages_fts) VALUES('rebuild');
             INSERT INTO messages_trigram_fts(messages_trigram_fts) VALUES('rebuild');",
        )?;
        Ok(())
    }

    /// Search message history using FTS5 full-text search.
    ///
    /// Returns matching messages with session context and a highlighted snippet
    /// (containing `<mark>...</mark>` tags around matched terms).
    ///
    /// `session_id` scopes the search to a single session (used by in-session
    /// "find in page" search). `None` means "all sessions".
    ///
    /// `types` filters by session type (regular / cron / subagent / channel);
    /// `None` means "all types".
    pub fn search_messages(
        &self,
        query: &str,
        agent_id: Option<&str>,
        session_id: Option<&str>,
        types: Option<&[SessionTypeFilter]>,
        limit: usize,
    ) -> Result<Vec<SessionSearchResult>> {
        self.search_messages_inner(query, agent_id, session_id, types, limit, true)
    }

    /// Search persisted chat message content only.
    ///
    /// This shares the same FTS/trigram implementation as `search_messages`,
    /// but intentionally omits title matches for consumers that anchor every
    /// hit to a real message row (for example `sessions_search` context windows).
    pub fn search_message_content(
        &self,
        query: &str,
        agent_id: Option<&str>,
        session_id: Option<&str>,
        types: Option<&[SessionTypeFilter]>,
        limit: usize,
    ) -> Result<Vec<SessionSearchResult>> {
        self.search_messages_inner(query, agent_id, session_id, types, limit, false)
    }

    fn search_messages_inner(
        &self,
        query: &str,
        agent_id: Option<&str>,
        session_id: Option<&str>,
        types: Option<&[SessionTypeFilter]>,
        limit: usize,
        include_title_matches: bool,
    ) -> Result<Vec<SessionSearchResult>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let fts_query = sanitize_fts_query(query);
        let trigram_query = sanitize_trigram_query(query);
        let like_pattern = build_search_like_pattern(query);
        if fts_query.is_empty() && trigram_query.is_empty() && like_pattern.is_none() {
            return Ok(Vec::new());
        }

        // Build dynamic WHERE / params. Each concrete query binds its match
        // parameter first, then appends these shared filters.
        let mut where_clauses: Vec<String> = Vec::new();
        let mut filter_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(aid) = agent_id {
            where_clauses.push("s.agent_id = ?".to_string());
            filter_params.push(Box::new(aid.to_string()));
        }

        if let Some(sid) = session_id {
            where_clauses.push("m.session_id = ?".to_string());
            filter_params.push(Box::new(sid.to_string()));
        } else {
            // Global FTS path: hide incognito sessions + knowledge-space
            // conversations (the latter have their own KB-panel history search).
            // The in-session search path (Some(sid)) already scopes to one
            // session and is allowed to search its content while it is open.
            where_clauses.push("s.incognito = 0".to_string());
            where_clauses.push("s.kind NOT IN ('knowledge','design','eval_fixture')".to_string());
        }

        // Session type filter — channel presence is detected via LEFT JOIN.
        if let Some(type_list) = types {
            if !type_list.is_empty() {
                let mut type_clauses: Vec<String> = Vec::new();
                for t in type_list {
                    match t {
                        SessionTypeFilter::Regular => type_clauses.push(
                            "(s.is_cron = 0 AND s.parent_session_id IS NULL AND cc.channel_id IS NULL AND s.kind NOT IN ('knowledge','design','eval_fixture'))".to_string(),
                        ),
                        SessionTypeFilter::Cron => {
                            type_clauses.push("s.is_cron = 1".to_string())
                        }
                        SessionTypeFilter::Subagent => {
                            type_clauses.push("s.parent_session_id IS NOT NULL".to_string())
                        }
                        SessionTypeFilter::Channel => {
                            type_clauses.push("cc.channel_id IS NOT NULL".to_string())
                        }
                    }
                }
                where_clauses.push(format!("({})", type_clauses.join(" OR ")));
            }
        }

        let and_where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" AND {}", where_clauses.join(" AND "))
        };
        let plain_where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_clauses.join(" AND "))
        };

        // STX/ETX (U+0002/U+0003) — never valid in user text — so the frontend
        // can split without HTML escape/unescape that could reintroduce an
        // attribute-level XSS from user-authored `<mark onclick=…>`.
        let fts_sql = format!(
            "SELECT m.id, m.session_id, m.role,
                    snippet(messages_fts, 0, '\x02', '\x03', '…', 16) AS snippet,
                    m.timestamp,
                    s.title, s.agent_id, s.is_cron, s.parent_session_id, s.project_id,
                    cc.channel_id, cc.chat_type,
                    fts.rank
             FROM messages_fts fts
             JOIN messages m ON m.id = fts.rowid
             JOIN sessions s ON s.id = m.session_id
             LEFT JOIN channel_conversations cc ON cc.session_id = s.id
             WHERE messages_fts MATCH ?
               AND m.role IN ('user', 'assistant'){}
             ORDER BY fts.rank
             LIMIT {}",
            and_where_sql, limit
        );

        let trigram_sql = format!(
            "SELECT m.id, m.session_id, m.role,
                    snippet(messages_trigram_fts, 0, '\x02', '\x03', '…', 16) AS snippet,
                    m.timestamp,
                    s.title, s.agent_id, s.is_cron, s.parent_session_id, s.project_id,
                    cc.channel_id, cc.chat_type,
                    tri.rank
             FROM messages_trigram_fts tri
             JOIN messages m ON m.id = tri.rowid
             JOIN sessions s ON s.id = m.session_id
             LEFT JOIN channel_conversations cc ON cc.session_id = s.id
             WHERE messages_trigram_fts MATCH ?
               AND m.role IN ('user', 'assistant'){}
             ORDER BY tri.rank
             LIMIT {}",
            and_where_sql, limit
        );

        // Run the FTS query on a read-pool connection. FTS shadow-table
        // corruption surfaces while *stepping* the virtual table, so it is
        // propagated (not swallowed like benign per-row errors) to drive the
        // rebuild-and-retry below.
        let run_fts = |conn: &Connection| -> rusqlite::Result<Vec<SessionSearchResult>> {
            let mut stmt = conn.prepare(&fts_sql)?;
            let mut param_refs: Vec<&dyn rusqlite::types::ToSql> =
                Vec::with_capacity(filter_params.len() + 1);
            param_refs.push(&fts_query);
            param_refs.extend(filter_params.iter().map(|p| p.as_ref()));
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                Ok(SessionSearchResult {
                    message_id: row.get(0)?,
                    session_id: row.get(1)?,
                    message_role: row.get(2)?,
                    content_snippet: row.get(3)?,
                    timestamp: row.get(4)?,
                    session_title: row.get(5)?,
                    agent_id: row.get(6)?,
                    is_cron: row.get::<_, i64>(7).unwrap_or(0) != 0,
                    parent_session_id: row.get(8)?,
                    project_id: row.get(9)?,
                    channel_type: row.get(10)?,
                    channel_chat_type: row.get(11)?,
                    relevance_rank: row.get::<_, f64>(12).unwrap_or(0.0),
                    match_kind: SEARCH_MATCH_KIND_MESSAGE.to_string(),
                })
            })?;
            let mut results = Vec::new();
            for r in rows {
                match r {
                    Ok(item) => results.push(item),
                    // Corruption → propagate so the caller can rebuild + retry.
                    Err(e) if Self::is_fts_corruption(&e) => return Err(e),
                    // Benign per-row decode error → skip, matching the prior
                    // `filter_map(|r| r.ok())` behavior.
                    Err(_) => {}
                }
            }
            Ok(results)
        };

        let run_trigram = |conn: &Connection| -> rusqlite::Result<Vec<SessionSearchResult>> {
            let mut stmt = conn.prepare(&trigram_sql)?;
            let mut param_refs: Vec<&dyn rusqlite::types::ToSql> =
                Vec::with_capacity(filter_params.len() + 1);
            param_refs.push(&trigram_query);
            param_refs.extend(filter_params.iter().map(|p| p.as_ref()));
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                Ok(SessionSearchResult {
                    message_id: row.get(0)?,
                    session_id: row.get(1)?,
                    message_role: row.get(2)?,
                    content_snippet: row.get(3)?,
                    timestamp: row.get(4)?,
                    session_title: row.get(5)?,
                    agent_id: row.get(6)?,
                    is_cron: row.get::<_, i64>(7).unwrap_or(0) != 0,
                    parent_session_id: row.get(8)?,
                    project_id: row.get(9)?,
                    channel_type: row.get(10)?,
                    channel_chat_type: row.get(11)?,
                    relevance_rank: row.get::<_, f64>(12).unwrap_or(1_000_000.0) + 1_000_000.0,
                    match_kind: SEARCH_MATCH_KIND_MESSAGE.to_string(),
                })
            })?;
            let mut results = Vec::new();
            for r in rows {
                match r {
                    Ok(item) => results.push(item),
                    Err(e) if Self::is_fts_corruption(&e) => return Err(e),
                    Err(_) => {}
                }
            }
            Ok(results)
        };

        let mut results = Vec::new();
        let mut seen_message_ids: HashSet<i64> = HashSet::new();

        // Global sidebar search should find sessions by title too. Keep
        // in-session Cmd/Ctrl+F message-only so navigation always lands on a
        // visible bubble.
        if include_title_matches && session_id.is_none() {
            if let Some(pattern) = like_pattern.as_deref() {
                let title_sql = format!(
                    "SELECT COALESCE(lm.id, 0), s.id, 'title',
                            COALESCE(s.title, '') AS snippet_source,
                            COALESCE(lm.timestamp, s.updated_at) AS hit_timestamp,
                            s.title, s.agent_id, s.is_cron, s.parent_session_id, s.project_id,
                            cc.channel_id, cc.chat_type
                     FROM sessions s
                     LEFT JOIN channel_conversations cc ON cc.session_id = s.id
                     LEFT JOIN messages lm ON lm.id = (
                         SELECT MAX(m2.id) FROM messages m2 WHERE m2.session_id = s.id
                     )
                     {}
                     {} COALESCE(s.title, '') LIKE ? ESCAPE '\\'
                     ORDER BY s.updated_at DESC
                     LIMIT {}",
                    plain_where_sql,
                    if plain_where_sql.is_empty() {
                        "WHERE"
                    } else {
                        "AND"
                    },
                    limit
                );
                let conn = self.read_conn()?;
                let mut stmt = conn.prepare(&title_sql)?;
                let mut param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    Vec::with_capacity(filter_params.len() + 1);
                param_refs.extend(filter_params.iter().map(|p| p.as_ref()));
                param_refs.push(&pattern);
                let rows = stmt.query_map(param_refs.as_slice(), |row| {
                    let title: String = row.get(3)?;
                    Ok(SessionSearchResult {
                        message_id: row.get(0)?,
                        session_id: row.get(1)?,
                        message_role: row.get(2)?,
                        content_snippet: highlighted_search_snippet(&title, query, 80),
                        timestamp: row.get(4)?,
                        session_title: row.get(5)?,
                        agent_id: row.get(6)?,
                        is_cron: row.get::<_, i64>(7).unwrap_or(0) != 0,
                        parent_session_id: row.get(8)?,
                        project_id: row.get(9)?,
                        channel_type: row.get(10)?,
                        channel_chat_type: row.get(11)?,
                        relevance_rank: -1_000_000.0,
                        match_kind: SEARCH_MATCH_KIND_TITLE.to_string(),
                    })
                })?;
                for row in rows {
                    if results.len() >= limit {
                        break;
                    }
                    if let Ok(hit) = row {
                        results.push(hit);
                    }
                }
            }
        }

        if !fts_query.is_empty() && results.len() < limit {
            // FTS search is a hot read (Cmd+F + /sessions) — route to the read pool.
            let conn = self.read_conn()?;
            let fts_results = match run_fts(&conn) {
                Ok(results) => results,
                Err(e) if Self::is_fts_corruption(&e) => {
                    // The one-time open gate means the every-open rebuild no longer
                    // self-heals corruption that develops later, and the read pool
                    // is READ_ONLY. Recover on demand: rebuild via the writer, then
                    // retry once on a fresh read connection.
                    drop(conn);
                    app_warn!(
                        "session",
                        "db",
                        "search_messages hit FTS corruption ({}); rebuilding index and retrying",
                        e
                    );
                    self.rebuild_fts()?;
                    let conn = self.read_conn()?;
                    run_fts(&conn)
                        .map_err(|e| anyhow::anyhow!("FTS search failed after rebuild: {}", e))?
                }
                Err(e) => return Err(anyhow::anyhow!("FTS search failed: {}", e)),
            };
            for hit in fts_results {
                seen_message_ids.insert(hit.message_id);
                if results.len() < limit {
                    results.push(hit);
                }
            }
        }

        // FTS is token-based and misses common substring cases (partial English
        // words, many CJK substrings). Use the trigram side index for those
        // substring hits instead of falling back to a full `messages LIKE '%q%'`
        // scan on every global search.
        if !trigram_query.is_empty() && results.len() < limit {
            let conn = self.read_conn()?;
            let trigram_results = match run_trigram(&conn) {
                Ok(results) => results,
                Err(e) if Self::is_fts_corruption(&e) => {
                    drop(conn);
                    app_warn!(
                        "session",
                        "db",
                        "search_messages hit trigram FTS corruption ({}); rebuilding index and retrying",
                        e
                    );
                    self.rebuild_fts()?;
                    let conn = self.read_conn()?;
                    run_trigram(&conn).map_err(|e| {
                        anyhow::anyhow!("trigram FTS search failed after rebuild: {}", e)
                    })?
                }
                Err(e) => return Err(anyhow::anyhow!("trigram FTS search failed: {}", e)),
            };
            for hit in trigram_results {
                if results.len() >= limit {
                    break;
                }
                if seen_message_ids.insert(hit.message_id) {
                    results.push(hit);
                }
            }
        }

        Ok(results)
    }

    /// FTS5 search that returns up to `limit` *distinct* sessions (best-rank
    /// snippet per session). Unlike `search_messages`, the limit is applied
    /// at session granularity, so a single chatty session can't crowd out
    /// other matching sessions in the results.
    ///
    /// Used by the `/sessions <query>` picker so a common term doesn't lose
    /// matches just because one session has dozens of hits. Excludes
    /// incognito sessions (the global-search invariant matches
    /// `search_messages` with `session_id = None`).
    pub fn search_distinct_session_snippets(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, String)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let fts_query = sanitize_fts_query(query);
        let trigram_query = sanitize_trigram_query(query);
        let like_pattern = build_search_like_pattern(query);
        if fts_query.is_empty() && trigram_query.is_empty() && like_pattern.is_none() {
            return Ok(Vec::new());
        }

        let mut results: Vec<(String, String)> = Vec::new();
        let mut seen_session_ids: HashSet<String> = HashSet::new();

        if let Some(pattern) = like_pattern.as_deref() {
            let title_sql = format!(
                "SELECT s.id, COALESCE(s.title, '') AS title
                 FROM sessions s
                 WHERE s.incognito = 0
                   AND s.kind NOT IN ('knowledge','design','eval_fixture')
                   AND COALESCE(s.title, '') LIKE ?1 ESCAPE '\\'
                 ORDER BY s.updated_at DESC
                 LIMIT {}",
                limit
            );
            let mut stmt = conn.prepare(&title_sql)?;
            let rows = stmt.query_map(params![pattern], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                if results.len() >= limit {
                    break;
                }
                if let Ok((session_id, title)) = row {
                    if seen_session_ids.insert(session_id.clone()) {
                        results.push((session_id, highlighted_search_snippet(&title, query, 80)));
                    }
                }
            }
        }

        // Window function partitions by session and keeps the top-ranked
        // hit per session before applying the LIMIT, so 100 hits in one
        // session don't displace single-hit sessions further down.
        if !fts_query.is_empty() && results.len() < limit {
            let sql = format!(
                "SELECT session_id, snippet FROM (
                     SELECT m.session_id AS session_id,
                            snippet(messages_fts, 0, '\x02', '\x03', '…', 16) AS snippet,
                            fts.rank AS rank,
                            ROW_NUMBER() OVER (PARTITION BY m.session_id ORDER BY fts.rank) AS rn
                     FROM messages_fts fts
                     JOIN messages m ON m.id = fts.rowid
                     JOIN sessions s ON s.id = m.session_id
                     WHERE messages_fts MATCH ?1
                       AND m.role IN ('user', 'assistant')
                       AND s.incognito = 0
                       AND s.kind NOT IN ('knowledge','design','eval_fixture')
                 ) WHERE rn = 1
                 ORDER BY rank
                 LIMIT {}",
                limit
            );

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![fts_query], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                if results.len() >= limit {
                    break;
                }
                if let Ok((session_id, snippet)) = row {
                    if seen_session_ids.insert(session_id.clone()) {
                        results.push((session_id, snippet));
                    }
                }
            }
        }

        if !trigram_query.is_empty() && results.len() < limit {
            let trigram_sql = format!(
                "SELECT session_id, snippet FROM (
                     SELECT m.session_id AS session_id,
                            snippet(messages_trigram_fts, 0, '\x02', '\x03', '…', 16) AS snippet,
                            tri.rank AS rank,
                            ROW_NUMBER() OVER (PARTITION BY m.session_id ORDER BY tri.rank) AS rn
                     FROM messages_trigram_fts tri
                     JOIN messages m ON m.id = tri.rowid
                     JOIN sessions s ON s.id = m.session_id
                     WHERE messages_trigram_fts MATCH ?1
                       AND m.role IN ('user', 'assistant')
                       AND s.incognito = 0
                       AND s.kind NOT IN ('knowledge','design','eval_fixture')
                 ) WHERE rn = 1
                 ORDER BY rank
                 LIMIT {}",
                limit
            );
            let mut stmt = conn.prepare(&trigram_sql)?;
            let rows = stmt.query_map(params![trigram_query], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                if results.len() >= limit {
                    break;
                }
                if let Ok((session_id, snippet)) = row {
                    if seen_session_ids.insert(session_id.clone()) {
                        results.push((session_id, snippet));
                    }
                }
            }
        }

        Ok(results)
    }

    /// Load a window of messages around a target message id.
    ///
    /// Returns `(messages_in_asc_order, total_count, has_more_before,
    /// has_more_after)`. The window contains up to `before` messages with
    /// `id <= target_message_id` (inclusive of the target) and up to `after`
    /// messages with `id > target_message_id`. The `before` side is aligned
    /// to a user-row boundary; the `after` side is returned as-is (mid-turn
    /// truncation on the trailing side is acceptable since the target itself
    /// anchors the view).
    pub fn load_session_messages_around(
        &self,
        session_id: &str,
        target_message_id: i64,
        before: u32,
        after: u32,
    ) -> Result<(Vec<SessionMessage>, u32, bool, bool)> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let total: u32 = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;

        // Load `before` messages with id <= target (DESC, then reverse).
        let mut before_stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking, ttft_ms, tokens_in_last,
                    tokens_cache_creation, tokens_cache_read, tool_metadata, stream_status
             FROM messages
             WHERE session_id = ?1 AND id <= ?2
             ORDER BY id DESC
             LIMIT ?3",
        )?;
        let before_rows = before_stmt
            .query_map(params![session_id, target_message_id, before], |row| {
                Self::row_to_session_message(row)
            })?;
        let mut before_msgs = Vec::new();
        for row in before_rows {
            before_msgs.push(row?);
        }
        before_msgs.reverse();

        Self::align_window_to_user_boundary(&conn, session_id, &mut before_msgs)?;

        // Load `after` messages with id > target (ASC).
        let mut after_stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking, ttft_ms, tokens_in_last,
                    tokens_cache_creation, tokens_cache_read, tool_metadata, stream_status
             FROM messages
             WHERE session_id = ?1 AND id > ?2
             ORDER BY id ASC
             LIMIT ?3",
        )?;
        let after_rows = after_stmt
            .query_map(params![session_id, target_message_id, after], |row| {
                Self::row_to_session_message(row)
            })?;
        let mut after_msgs = Vec::new();
        for row in after_rows {
            after_msgs.push(row?);
        }

        // Extend the trailing edge to the next user boundary so the around
        // page contains complete turns. Without this a hit landing inside a
        // long tool loop would split the turn and force the frontend to
        // synthesise a placeholder assistant — and the next `_after` page
        // would synthesise a second one for the leading orphans.
        let after_anchor = after_msgs.last().map(|m| m.id).unwrap_or(target_message_id);
        Self::extend_window_to_turn_end(&conn, session_id, after_anchor, &mut after_msgs)?;

        let has_more_before = match before_msgs.first() {
            Some(first) => Self::has_messages_before(&conn, session_id, first.id),
            None => false,
        };
        let has_more_after = match after_msgs.last() {
            Some(last) => Self::has_messages_after(&conn, session_id, last.id),
            None => Self::has_messages_after(&conn, session_id, target_message_id),
        };

        let mut messages = before_msgs;
        messages.extend(after_msgs);
        Ok((messages, total, has_more_before, has_more_after))
    }

    // ── Behavior awareness helpers ──────────────────────────────

    /// Read the per-session override JSON for behavior awareness, if any.
    pub fn get_session_awareness_config_json(&self, session_id: &str) -> Result<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare("SELECT awareness_config_json FROM sessions WHERE id = ?1")?;
        let mut rows = stmt.query(params![session_id])?;
        if let Some(row) = rows.next()? {
            let val: Option<String> = row.get(0)?;
            return Ok(val.filter(|s| !s.is_empty()));
        }
        Ok(None)
    }

    /// Write (or clear with `None`) the per-session override JSON for
    /// behavior awareness.
    pub fn set_session_awareness_config_json(
        &self,
        session_id: &str,
        json: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET awareness_config_json = ?1 WHERE id = ?2",
            params![json, session_id],
        )?;
        Ok(())
    }

    /// Return the last user message of a session, truncated to `max_chars`.
    /// Used as a fallback preview when no SessionFacet is cached.
    pub fn last_user_message_preview(
        &self,
        session_id: &str,
        max_chars: usize,
    ) -> Result<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT content FROM messages
             WHERE session_id = ?1 AND role = 'user' AND length(content) > 0
             ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![session_id])?;
        if let Some(row) = rows.next()? {
            let content: String = row.get(0)?;
            let trimmed = crate::truncate_utf8(content.trim(), max_chars).to_string();
            if trimmed.is_empty() {
                return Ok(None);
            }
            return Ok(Some(trimmed));
        }
        Ok(None)
    }

    /// Whether the two most recent user messages of `session_id` are
    /// within `window_secs` of each other. Used by the auto-review trigger
    /// as a proxy for "user just corrected themselves / changed their mind"
    /// — those turns are exactly the ones where a fresh skill draft is
    /// most likely to come from a genuine learning. Returns `false` when
    /// the session has fewer than 2 user messages or the deltas can't be
    /// parsed (best-effort).
    pub fn user_messages_within(&self, session_id: &str, window_secs: u64) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT timestamp FROM messages
             WHERE session_id = ?1 AND role = 'user'
             ORDER BY id DESC LIMIT 2",
        )?;
        let mut rows = stmt.query(params![session_id])?;
        let first = match rows.next()? {
            Some(r) => r.get::<_, String>(0)?,
            None => return Ok(false),
        };
        let second = match rows.next()? {
            Some(r) => r.get::<_, String>(0)?,
            None => return Ok(false),
        };
        let (a, b) = match (
            chrono::DateTime::parse_from_rfc3339(&first),
            chrono::DateTime::parse_from_rfc3339(&second),
        ) {
            (Ok(a), Ok(b)) => (a, b),
            _ => return Ok(false),
        };
        let delta = (a - b).num_seconds().unsigned_abs();
        Ok(delta <= window_secs)
    }

    /// Return the last N user messages for a session within a time window.
    /// Used by awareness LLM extraction to give the model concrete recent activity.
    pub fn recent_user_messages_for_preview(
        &self,
        session_id: &str,
        since_rfc3339: &str,
        limit: u32,
        max_chars_per_msg: usize,
    ) -> Result<Vec<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT content FROM messages
             WHERE session_id = ?1
               AND role = 'user'
               AND length(content) > 0
               AND timestamp >= ?2
             ORDER BY id DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![session_id, since_rfc3339, limit as i64], |row| {
            row.get::<_, String>(0)
        })?;
        let mut out = Vec::new();
        for row in rows {
            let content = row?;
            out.push(crate::truncate_utf8(content.trim(), max_chars_per_msg).to_string());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionDB, SessionTypeFilter, SEARCH_MATCH_KIND_MESSAGE, SEARCH_MATCH_KIND_TITLE};
    use crate::session::{NewMessage, SessionKind};
    use rusqlite::{Connection, OptionalExtension};

    fn ensure_channel_conversations_table(db: &SessionDB) {
        // Mirror the production schema in `ChannelDB::migrate` (1:1 attach).
        let conn = db.conn.lock().expect("lock connection");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS channel_conversations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                chat_id TEXT NOT NULL,
                thread_id TEXT,
                session_id TEXT NOT NULL,
                sender_id TEXT,
                sender_name TEXT,
                chat_type TEXT NOT NULL DEFAULT 'dm',
                source TEXT NOT NULL DEFAULT 'inbound',
                attached_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );",
        )
        .expect("create channel conversations table");
    }

    fn temp_db_path(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "{}-{}-{}.sqlite3",
            name,
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        std::env::temp_dir().join(unique)
    }

    fn set_session_updated_at(db: &SessionDB, session_id: &str, updated_at: &str) {
        let conn = db.conn.lock().expect("lock connection");
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            rusqlite::params![updated_at, session_id],
        )
        .expect("update session timestamp");
    }

    #[test]
    fn session_sandbox_mode_round_trips() {
        let db_path = temp_db_path("session-sandbox-mode");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");

        assert_eq!(
            db.get_session_sandbox_mode(&session.id)
                .expect("read default sandbox mode"),
            Some(crate::permission::SandboxMode::Off)
        );

        db.update_session_sandbox_mode(&session.id, crate::permission::SandboxMode::Workspace)
            .expect("update sandbox mode");

        assert_eq!(
            db.get_session_sandbox_mode(&session.id)
                .expect("read updated sandbox mode"),
            Some(crate::permission::SandboxMode::Workspace)
        );
        assert_eq!(
            db.get_session(&session.id)
                .expect("get session")
                .expect("session exists")
                .sandbox_mode,
            crate::permission::SandboxMode::Workspace
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn session_execution_mode_round_trips() {
        let db_path = temp_db_path("session-execution-mode");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");

        assert_eq!(
            db.get_session_execution_mode(&session.id)
                .expect("read default execution mode"),
            Some(crate::execution_mode::ExecutionMode::Off)
        );

        db.update_session_execution_mode(&session.id, crate::execution_mode::ExecutionMode::Deep)
            .expect("update execution mode");

        assert_eq!(
            db.get_session_execution_mode(&session.id)
                .expect("read updated execution mode"),
            Some(crate::execution_mode::ExecutionMode::Deep)
        );
        assert_eq!(
            db.get_session(&session.id)
                .expect("get session")
                .expect("session exists")
                .execution_mode,
            crate::execution_mode::ExecutionMode::Deep
        );

        assert_eq!(
            db.get_session_workflow_mode(&session.id)
                .expect("read default workflow mode"),
            Some(crate::workflow_mode::WorkflowMode::Off)
        );

        db.update_session_workflow_mode(&session.id, crate::workflow_mode::WorkflowMode::Ultracode)
            .expect("update workflow mode");

        assert_eq!(
            db.get_session_workflow_mode(&session.id)
                .expect("read updated workflow mode"),
            Some(crate::workflow_mode::WorkflowMode::Ultracode)
        );
        assert_eq!(
            db.get_session(&session.id)
                .expect("get session")
                .expect("session exists")
                .workflow_mode,
            crate::workflow_mode::WorkflowMode::Ultracode
        );

        assert!(db
            .update_session_execution_mode(
                "missing-session",
                crate::execution_mode::ExecutionMode::Deep,
            )
            .expect_err("missing session execution mode update should fail")
            .to_string()
            .contains("Session not found"));
        assert!(
            db.update_session_workflow_mode(
                "missing-session",
                crate::workflow_mode::WorkflowMode::On,
            )
            .expect_err("missing session workflow mode update should fail")
            .to_string()
            .contains("Session not found")
        );

        let incognito_session = db
            .create_session_with_project(crate::agent_loader::DEFAULT_AGENT_ID, None, Some(true))
            .expect("create incognito session");
        assert!(db
            .update_session_workflow_mode(
                &incognito_session.id,
                crate::workflow_mode::WorkflowMode::On,
            )
            .expect_err("incognito workflow mode enable should fail")
            .to_string()
            .contains("incognito session"));
        db.update_session_workflow_mode(
            &incognito_session.id,
            crate::workflow_mode::WorkflowMode::Off,
        )
        .expect("turning workflow mode off remains allowed for incognito");

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn sessions_table_includes_incognito_column() {
        let db_path = temp_db_path("session-incognito-schema");
        let db = SessionDB::open(&db_path).expect("open session db");
        let conn = db.conn.lock().expect("lock connection");

        let mut stmt = conn
            .prepare("PRAGMA table_info(sessions)")
            .expect("prepare table info");
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query table info")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect columns");

        assert!(
            columns.iter().any(|c| c == "incognito"),
            "expected incognito column in sessions table, got {:?}",
            columns
        );

        drop(stmt);
        drop(conn);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn list_recent_regular_chats_filters_regular_sessions_before_limit() {
        let db_path = temp_db_path("session-recent-regular");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let regular_old = db.create_session("ha-main").expect("regular old");
        let regular_new = db.create_session("ha-main").expect("regular new");
        let cron = db.create_session("ha-main").expect("cron session");
        db.mark_session_cron(&cron.id).expect("mark cron");
        let subagent = db
            .create_session_full("ha-main", Some(&regular_new.id), None, false)
            .expect("subagent session");
        let knowledge = db.create_session("ha-main").expect("knowledge session");
        db.set_session_kind(&knowledge.id, SessionKind::Knowledge)
            .expect("mark knowledge");
        let incognito = db
            .create_session_full("ha-main", None, None, true)
            .expect("incognito session");
        let channel = db.create_session("ha-main").expect("channel session");
        {
            let conn = db.conn.lock().expect("lock connection");
            conn.execute(
                "INSERT INTO channel_conversations
                    (channel_id, account_id, chat_id, session_id, chat_type, source, created_at, updated_at)
                 VALUES
                    ('slack', 'acct', 'chat', ?1, 'dm', 'inbound', '2026-05-01T00:00:00Z', '2026-05-01T00:00:00Z')",
                rusqlite::params![channel.id],
            )
            .expect("insert channel conversation");
        }

        set_session_updated_at(&db, &regular_old.id, "2026-05-01T00:00:00Z");
        set_session_updated_at(&db, &regular_new.id, "2026-05-02T00:00:00Z");
        set_session_updated_at(&db, &cron.id, "2026-05-10T00:00:00Z");
        set_session_updated_at(&db, &subagent.id, "2026-05-11T00:00:00Z");
        set_session_updated_at(&db, &knowledge.id, "2026-05-12T00:00:00Z");
        set_session_updated_at(&db, &incognito.id, "2026-05-13T00:00:00Z");
        set_session_updated_at(&db, &channel.id, "2026-05-14T00:00:00Z");

        let (sessions, total) = db
            .list_recent_regular_chats(5)
            .expect("list recent regular chats");

        assert_eq!(total, 2);
        assert_eq!(
            sessions.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec![regular_new.id.as_str(), regular_old.id.as_str()]
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn search_messages_regular_filter_excludes_non_regular_sessions() {
        let db_path = temp_db_path("session-search-regular-filter");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let regular = db.create_session("ha-main").expect("regular session");
        let cron = db.create_session("ha-main").expect("cron session");
        db.mark_session_cron(&cron.id).expect("mark cron");
        let subagent = db
            .create_session_full("ha-main", Some(&regular.id), None, false)
            .expect("subagent session");
        let knowledge = db.create_session("ha-main").expect("knowledge session");
        db.set_session_kind(&knowledge.id, SessionKind::Knowledge)
            .expect("mark knowledge");
        let channel = db.create_session("ha-main").expect("channel session");
        {
            let conn = db.conn.lock().expect("lock connection");
            let now = chrono::Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO channel_conversations
                    (channel_id, account_id, chat_id, session_id, chat_type, source, created_at, updated_at)
                 VALUES
                    ('slack', 'acct', 'chat', ?1, 'dm', 'inbound', ?2, ?2)",
                rusqlite::params![channel.id, now],
            )
            .expect("insert channel conversation");
        }

        for sid in [
            &regular.id,
            &cron.id,
            &subagent.id,
            &knowledge.id,
            &channel.id,
        ] {
            db.append_message(sid, &NewMessage::user("regular-filter-needle"))
                .expect("append message");
        }

        let results = db
            .search_messages(
                "regular-filter-needle",
                None,
                None,
                Some(&[SessionTypeFilter::Regular]),
                20,
            )
            .expect("search");
        let session_ids: std::collections::HashSet<_> =
            results.iter().map(|hit| hit.session_id.as_str()).collect();

        assert_eq!(session_ids.len(), 1, "got hits from {:?}", session_ids);
        assert!(session_ids.contains(regular.id.as_str()));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn search_messages_global_matches_session_titles() {
        let db_path = temp_db_path("session-search-title");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let regular = db.create_session("ha-main").expect("regular session");
        db.update_session_title(&regular.id, "Roadmap Draft")
            .expect("set title");
        db.append_message(&regular.id, &NewMessage::user("body without the query"))
            .expect("append message");

        let incognito = db
            .create_session_full("ha-main", None, None, true)
            .expect("incognito session");
        db.update_session_title(&incognito.id, "Roadmap Private")
            .expect("set incognito title");
        db.append_message(&incognito.id, &NewMessage::user("body without the query"))
            .expect("append incognito message");

        let knowledge = db.create_session("ha-main").expect("knowledge session");
        db.set_session_kind(&knowledge.id, SessionKind::Knowledge)
            .expect("mark knowledge");
        db.update_session_title(&knowledge.id, "Roadmap Knowledge")
            .expect("set knowledge title");
        db.append_message(&knowledge.id, &NewMessage::user("body without the query"))
            .expect("append knowledge message");

        let results = db
            .search_messages(
                "Roadmap",
                None,
                None,
                Some(&[SessionTypeFilter::Regular]),
                20,
            )
            .expect("search");

        assert_eq!(results.len(), 1, "got hits from {:?}", results);
        let hit = &results[0];
        assert_eq!(hit.session_id, regular.id);
        assert_eq!(hit.match_kind, SEARCH_MATCH_KIND_TITLE);
        assert!(hit.content_snippet.contains('\u{0002}'));

        let scoped = db
            .search_messages("Roadmap", None, Some(&regular.id), None, 20)
            .expect("scoped search");
        assert!(
            scoped.is_empty(),
            "in-session search should stay message-only: {:?}",
            scoped
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn search_message_content_excludes_title_hits_without_starving_messages() {
        let db_path = temp_db_path("session-search-message-only");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        for idx in 0..3 {
            let title_only = db.create_session("ha-main").expect("title-only session");
            db.update_session_title(&title_only.id, &format!("Roadmap Title {}", idx))
                .expect("set title");
            db.append_message(&title_only.id, &NewMessage::user("body without the query"))
                .expect("append title-only message");
        }

        let message_match = db.create_session("ha-main").expect("message session");
        db.update_session_title(&message_match.id, "Unrelated title")
            .expect("set title");
        db.append_message(
            &message_match.id,
            &NewMessage::user("Roadmap in the message body"),
        )
        .expect("append matching message");

        let results = db
            .search_message_content(
                "Roadmap",
                None,
                None,
                Some(&[SessionTypeFilter::Regular]),
                1,
            )
            .expect("search");

        assert_eq!(results.len(), 1, "got hits from {:?}", results);
        let hit = &results[0];
        assert_eq!(hit.session_id, message_match.id);
        assert_eq!(hit.match_kind, SEARCH_MATCH_KIND_MESSAGE);
        assert_ne!(hit.message_id, 0);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn search_messages_uses_trigram_body_substring() {
        let db_path = temp_db_path("session-search-substring");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let session = db.create_session("ha-main").expect("session");
        db.append_message(
            &session.id,
            &NewMessage::user("tokenized-prefix-xxalphabeta-suffix"),
        )
        .expect("append message");

        let results = db
            .search_messages("alpha", None, None, Some(&[SessionTypeFilter::Regular]), 20)
            .expect("search");

        assert_eq!(results.len(), 1, "got hits from {:?}", results);
        let hit = &results[0];
        assert_eq!(hit.session_id, session.id);
        assert_eq!(hit.match_kind, SEARCH_MATCH_KIND_MESSAGE);
        assert!(hit.content_snippet.contains('\u{0002}'));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn unread_count_excludes_cron_sessions() {
        // §3 regression: cron runs persist assistant rows with source="cron"
        // (previously "channel", which silently rode the `!= 'channel'` unread
        // suppressor). Unread suppression for cron is now keyed on `s.is_cron = 0`
        // in the SESSION_META_SELECT subquery, independent of the source string —
        // a cron session must never accrue an unread badge while a regular
        // session still does.
        let db_path = temp_db_path("session-unread-excludes-cron");
        let db = SessionDB::open(&db_path).expect("open session db");
        // SESSION_META_SELECT LEFT JOINs channel_conversations (owned by ChannelDB);
        // create it so `get_session` can hydrate the unread subquery.
        ensure_channel_conversations_table(&db);

        let regular = db.create_session("ha-main").expect("regular session");
        db.append_message(
            &regular.id,
            &NewMessage::assistant("regular reply")
                .with_source(crate::chat_engine::ChatSource::Desktop),
        )
        .expect("append regular assistant");

        let cron = db.create_session("ha-main").expect("cron session");
        db.mark_session_cron(&cron.id).expect("mark cron");
        db.append_message(
            &cron.id,
            &NewMessage::assistant("cron output").with_source(crate::chat_engine::ChatSource::Cron),
        )
        .expect("append cron assistant");

        let regular_meta = db
            .get_session(&regular.id)
            .expect("get regular")
            .expect("regular exists");
        let cron_meta = db
            .get_session(&cron.id)
            .expect("get cron")
            .expect("cron exists");

        assert_eq!(
            regular_meta.unread_count, 1,
            "regular assistant reply should count as unread"
        );
        assert_eq!(
            cron_meta.unread_count, 0,
            "cron output must never accrue an unread badge"
        );

        let _ = std::fs::remove_file(&db_path);
    }

    /// §3 regression: the per-session `unread_count` subquery excludes sub-agent
    /// child sessions (`parent_session_id IS NULL`) so a backend consumer that
    /// reads the field directly can't leak sub-agent internals as user unread.
    /// The IM split is carried by the separate `channel_unread_count` column:
    /// a `source = 'channel'` assistant reply lands there, never in the regular
    /// desktop `unread_count`.
    #[test]
    fn unread_count_excludes_subagent_and_splits_channel() {
        use crate::chat_engine::ChatSource;

        let db_path = temp_db_path("session-unread-subagent-channel");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        // Regular desktop reply → desktop unread, no IM unread.
        let regular = db.create_session("ha-main").expect("regular session");
        db.append_message(
            &regular.id,
            &NewMessage::assistant("desktop reply").with_source(ChatSource::Desktop),
        )
        .expect("append regular");

        // Channel-source reply → IM unread only.
        let channel = db.create_session("ha-main").expect("channel session");
        db.append_message(
            &channel.id,
            &NewMessage::assistant("im reply").with_source(ChatSource::Channel),
        )
        .expect("append channel");

        // Sub-agent child session (desktop source) → excluded from BOTH counts.
        let sub = db
            .create_session_with_parent("ha-main", Some(&regular.id))
            .expect("subagent session");
        db.append_message(
            &sub.id,
            &NewMessage::assistant("subagent reply").with_source(ChatSource::Desktop),
        )
        .expect("append subagent");

        let regular_meta = db
            .get_session(&regular.id)
            .unwrap()
            .expect("regular exists");
        let channel_meta = db
            .get_session(&channel.id)
            .unwrap()
            .expect("channel exists");
        let sub_meta = db.get_session(&sub.id).unwrap().expect("subagent exists");

        assert_eq!(
            regular_meta.unread_count, 1,
            "desktop reply → desktop unread"
        );
        assert_eq!(
            regular_meta.channel_unread_count, 0,
            "desktop reply is not IM unread"
        );
        assert_eq!(
            channel_meta.unread_count, 0,
            "channel-source reply must not count as desktop unread"
        );
        assert_eq!(
            channel_meta.channel_unread_count, 1,
            "channel-source reply counts as IM unread"
        );
        assert_eq!(
            sub_meta.unread_count, 0,
            "sub-agent session is excluded from desktop unread"
        );
        assert_eq!(
            sub_meta.channel_unread_count, 0,
            "sub-agent session is excluded from IM unread too"
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn fork_session_copies_transcript_to_message_boundary_as_regular_read_session() {
        let db_path = temp_db_path("session-fork-boundary");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let source = db.create_session("ha-main").expect("source session");
        db.update_session_title_with_source(
            &source.id,
            "Original task",
            crate::session_title::TITLE_SOURCE_LLM,
        )
        .expect("title");
        db.append_message(&source.id, &NewMessage::user("first prompt"))
            .expect("append user");
        let boundary = db
            .append_message(&source.id, &NewMessage::assistant("first answer"))
            .expect("append assistant");
        db.append_message(&source.id, &NewMessage::user("second prompt"))
            .expect("append later user");

        let forked = db
            .fork_session(&source.id, Some(boundary))
            .expect("fork session");

        assert_ne!(forked.id, source.id);
        // Title dedup: source already holds "Original task" → fork gets a sequence.
        assert_eq!(forked.title.as_deref(), Some("Original task (1)"));
        assert_eq!(
            forked.forked_from_session_id.as_deref(),
            Some(source.id.as_str())
        );
        assert_eq!(forked.forked_from_message_id, Some(boundary));
        assert_eq!(
            forked.forked_from_session_title.as_deref(),
            Some("Original task")
        );
        assert!(forked.parent_session_id.is_none());
        assert_eq!(forked.message_count, 2);
        assert_eq!(forked.unread_count, 0, "copied history starts as read");

        let forked_messages = db
            .load_session_messages(&forked.id)
            .expect("load forked messages");
        assert_eq!(forked_messages.len(), 2);
        assert_eq!(forked_messages[0].content, "first prompt");
        assert_eq!(forked_messages[1].content, "first answer");

        let original_messages = db
            .load_session_messages(&source.id)
            .expect("load original messages");
        assert_eq!(
            original_messages.len(),
            3,
            "source transcript stays untouched"
        );

        let (visible, _) = db
            .list_sessions_paged_for_sidebar(
                None,
                super::ProjectFilter::All,
                super::ParentSessionFilter::All,
                None,
                None,
                None,
            )
            .expect("list sessions");
        assert!(
            visible.iter().any(|session| session.id == forked.id),
            "forked session must remain a first-class sidebar session"
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn sidebar_pagination_filters_project_and_parent_before_limit() {
        let db_path = temp_db_path("sidebar-filtered-pagination");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let root = db.create_session("agent-a").expect("root session");
        let child = db
            .create_session_with_parent("agent-a", Some(&root.id))
            .expect("child session");
        for _ in 0..3 {
            db.create_session_with_project("agent-a", Some("project-a"), None)
                .expect("project session");
        }

        let (roots, root_total) = db
            .list_sessions_paged_for_sidebar(
                Some("agent-a"),
                super::ProjectFilter::Unassigned,
                super::ParentSessionFilter::Root,
                Some(1),
                Some(0),
                None,
            )
            .expect("list root sidebar sessions");
        assert_eq!(root_total, 1);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, root.id);

        let (children, child_total) = db
            .list_sessions_paged_for_sidebar(
                Some("agent-a"),
                super::ProjectFilter::Unassigned,
                super::ParentSessionFilter::Child,
                Some(1),
                Some(0),
                None,
            )
            .expect("list child sidebar sessions");
        assert_eq!(child_total, 1);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id, child.id);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn fork_session_rejects_incognito_source() {
        let db_path = temp_db_path("session-fork-incognito");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let source = db
            .create_session_with_project("ha-main", None, Some(true))
            .expect("incognito session");
        db.append_message(&source.id, &NewMessage::user("private prompt"))
            .expect("append user");

        let err = db
            .fork_session(&source.id, None)
            .expect_err("incognito fork should fail");
        assert!(
            err.to_string().contains("incognito"),
            "unexpected error: {err}"
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn fork_session_copies_owned_attachments_and_survives_source_deletion() {
        let db_path = temp_db_path("session-fork-attachments");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let source = db.create_session("ha-main").expect("source session");
        let source_dir = crate::paths::attachments_dir(&source.id).expect("source attachments dir");
        std::fs::create_dir_all(&source_dir).expect("create source attachments dir");
        let upload_path = source_dir.join("uploaded-note.txt");
        let tool_path = source_dir.join("generated-report.txt");
        std::fs::write(&upload_path, b"uploaded content").expect("write upload");
        std::fs::write(&tool_path, b"generated content").expect("write tool output");

        let mut user = NewMessage::user("review these files");
        user.attachments_meta = Some(
            serde_json::json!({
                "goal_trigger": true,
                "user_attachments": [{
                    "name": "uploaded-note.txt",
                    "mime_type": "text/plain",
                    "size": 16,
                    "path": upload_path,
                    "source": "upload"
                }]
            })
            .to_string(),
        );
        db.append_message(&source.id, &user)
            .expect("append user attachment");

        let mut tool = NewMessage::tool(
            "call-fork-media",
            "send_attachment",
            r#"{"path":"generated-report.txt"}"#,
            "",
            None,
            false,
        );
        tool.attachments_meta =
            crate::session::build_tool_media_items_attachments_meta(&serde_json::json!([{
                "url": format!(
                    "/api/attachments/{}/generated-report.txt",
                    source.id
                ),
                "localPath": tool_path,
                "name": "generated-report.txt",
                "mimeType": "text/plain",
                "sizeBytes": 17,
                "kind": "file"
            }]));
        // `NewMessage::tool` models an in-flight engine row by default. This
        // fixture represents a settled tool result that is safe to fork.
        tool.stream_status = Some("completed".to_string());
        db.append_message(&source.id, &tool)
            .expect("append tool attachment");

        let forked = db
            .fork_session(&source.id, None)
            .expect("fork session with attachments");
        let forked_dir = crate::paths::attachments_dir(&forked.id).expect("forked attachments dir");
        let messages = db
            .load_session_messages(&forked.id)
            .expect("load forked messages");

        let user_meta: serde_json::Value = serde_json::from_str(
            messages[0]
                .attachments_meta
                .as_deref()
                .expect("forked user metadata"),
        )
        .expect("parse user metadata");
        let forked_upload_path = std::path::PathBuf::from(
            user_meta["user_attachments"][0]["path"]
                .as_str()
                .expect("forked upload path"),
        );
        assert!(forked_upload_path.starts_with(&forked_dir));

        let tool_meta: serde_json::Value = serde_json::from_str(
            messages[1]
                .attachments_meta
                .as_deref()
                .expect("forked tool metadata"),
        )
        .expect("parse tool metadata");
        let forked_tool_path = std::path::PathBuf::from(
            tool_meta["tool_media_items"][0]["localPath"]
                .as_str()
                .expect("forked tool path"),
        );
        assert!(forked_tool_path.starts_with(&forked_dir));
        assert_eq!(
            tool_meta["tool_media_items"][0]["url"].as_str(),
            Some(format!("/api/attachments/{}/generated-report.txt", forked.id).as_str())
        );

        db.delete_session(&source.id)
            .expect("delete source session");
        assert_eq!(
            std::fs::read(&forked_upload_path).expect("read copied upload"),
            b"uploaded content"
        );
        assert_eq!(
            std::fs::read(&forked_tool_path).expect("read copied tool output"),
            b"generated content"
        );

        db.delete_session(&forked.id)
            .expect("delete forked session");
        let _ = std::fs::remove_file(&db_path);
    }

    /// Forking a design-space thread keeps `kind='design'` and rebuilds the
    /// `design_chat_threads` anchor so the design tool still resolves the source
    /// project (regression: without the anchor the fork falls to a draft project).
    #[test]
    fn fork_session_on_design_thread_keeps_kind_and_rebuilds_anchor() {
        let db_path = temp_db_path("session-fork-design");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let project_id = "design-proj-1";
        let source = db
            .create_session_with_project("ha-main", Some(project_id), Some(false))
            .expect("design source session");
        db.set_session_kind(&source.id, SessionKind::Design)
            .expect("mark design");
        {
            let conn = db.conn.lock().expect("lock");
            conn.execute(
                "INSERT INTO design_chat_threads (session_id, project_id, created_at)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![source.id, project_id, 1_i64],
            )
            .expect("insert source anchor");
        }
        db.update_session_title_with_source(
            &source.id,
            "配色方案",
            crate::session_title::TITLE_SOURCE_LLM,
        )
        .expect("title");
        db.append_message(&source.id, &NewMessage::user("探索暖色方向"))
            .expect("append user");
        db.append_message(&source.id, &NewMessage::assistant("方案 A"))
            .expect("append assistant");

        let forked = db
            .fork_session(&source.id, None)
            .expect("fork design session");

        let forked_meta = db.get_session(&forked.id).unwrap().expect("forked exists");
        assert_eq!(
            forked_meta.kind,
            SessionKind::Design,
            "forked design thread stays kind='design'"
        );
        assert_eq!(
            forked.forked_from_session_id.as_deref(),
            Some(source.id.as_str()),
            "lineage recorded"
        );
        let anchor_pid: Option<String> = {
            let conn = db.conn.lock().expect("lock");
            conn.query_row(
                "SELECT project_id FROM design_chat_threads WHERE session_id = ?1",
                rusqlite::params![forked.id],
                |r| r.get(0),
            )
            .optional()
            .expect("query forked anchor")
        };
        assert_eq!(
            anchor_pid.as_deref(),
            Some(project_id),
            "forked design thread must rebuild its project anchor"
        );
        assert_eq!(
            forked.title.as_deref(),
            Some("配色方案 (1)"),
            "duplicate title gets a sequence suffix"
        );

        let _ = std::fs::remove_file(&db_path);
    }

    /// Forking a session no longer duplicates the title verbatim — it appends
    /// `(1)`/`(2)`/… and continues the series when forking an already-numbered one.
    #[test]
    fn fork_session_appends_sequence_to_duplicate_titles() {
        let db_path = temp_db_path("session-fork-title-seq");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let source = db.create_session("ha-main").expect("source session");
        db.update_session_title_with_source(
            &source.id,
            "配色方案",
            crate::session_title::TITLE_SOURCE_LLM,
        )
        .expect("title");
        db.append_message(&source.id, &NewMessage::user("hi"))
            .expect("append user");

        let f1 = db.fork_session(&source.id, None).expect("fork 1");
        assert_eq!(f1.title.as_deref(), Some("配色方案 (1)"));

        let f2 = db.fork_session(&source.id, None).expect("fork 2");
        assert_eq!(f2.title.as_deref(), Some("配色方案 (2)"));

        // Forking the already-numbered f1 continues the base series (no nesting).
        let f3 = db.fork_session(&f1.id, None).expect("fork 3");
        assert_eq!(f3.title.as_deref(), Some("配色方案 (3)"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn open_migrates_legacy_sessions_without_pinned_at() {
        let db_path = temp_db_path("session-legacy-no-pinned-at");
        let legacy_conn = Connection::open(&db_path).expect("open legacy db");
        legacy_conn
            .execute_batch(
                "CREATE TABLE sessions (
                    id TEXT PRIMARY KEY,
                    title TEXT,
                    agent_id TEXT NOT NULL DEFAULT 'ha-main',
                    provider_id TEXT,
                    provider_name TEXT,
                    model_id TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    context_json TEXT,
                    last_read_message_id INTEGER DEFAULT 0,
                    is_cron INTEGER NOT NULL DEFAULT 0,
                    parent_session_id TEXT
                );
                INSERT INTO sessions (id, title, agent_id, created_at, updated_at)
                VALUES ('legacy-session', 'Legacy', 'ha-main', '2026-05-23T00:00:00Z', '2026-05-23T00:00:00Z');",
            )
            .expect("create legacy schema");
        drop(legacy_conn);

        let db = SessionDB::open(&db_path).expect("open migrated session db");
        let conn = db.conn.lock().expect("lock connection");

        assert!(
            conn.prepare("SELECT pinned_at FROM sessions LIMIT 1")
                .is_ok(),
            "expected pinned_at column to be added before pinned index creation"
        );
        assert!(
            conn.prepare("SELECT sandbox_mode FROM sessions LIMIT 1")
                .is_ok(),
            "expected sandbox_mode column to be added during migration"
        );
        assert!(
            conn.prepare("SELECT execution_mode FROM sessions LIMIT 1")
                .is_ok(),
            "expected execution_mode column to be added during migration"
        );
        assert!(
            conn.prepare("SELECT workflow_mode FROM sessions LIMIT 1")
                .is_ok(),
            "expected workflow_mode column to be added during migration"
        );
        assert!(
            conn.prepare(
                "SELECT forked_from_session_id, forked_from_message_id FROM sessions LIMIT 1"
            )
            .is_ok(),
            "expected session fork columns to be added before their index"
        );

        let mut stmt = conn
            .prepare("PRAGMA index_list(sessions)")
            .expect("prepare index list");
        let indexes: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query index list")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect indexes");
        assert!(
            indexes
                .iter()
                .any(|index| index == "idx_sessions_pinned_at"),
            "expected pinned_at index after migration, got {:?}",
            indexes
        );
        assert!(
            indexes
                .iter()
                .any(|index| index == "idx_sessions_forked_from"),
            "expected fork lineage index after migration, got {:?}",
            indexes
        );

        drop(stmt);
        drop(conn);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn read_pool_is_readonly_and_sees_committed_writes() {
        let db_path = temp_db_path("session-read-pool");
        let db = SessionDB::open(&db_path).expect("open session db");
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");

        // read_conn must hand out READ_ONLY connections — a write fails.
        let probe = db
            .read_conn()
            .expect("read conn")
            .execute("CREATE TABLE __readpool_probe (x)", []);
        assert!(
            probe.is_err(),
            "read_conn must reject writes (read-only pool)"
        );

        // A committed write on the writer is visible to a subsequent read-pool
        // read (WAL gives readers the latest committed snapshot).
        let tool = crate::session::NewMessage::tool("call-rp", "noop", "{}", "", None, false);
        db.append_message(&session.id, &tool).expect("append");
        let msgs = db
            .load_session_messages(&session.id)
            .expect("read via pool");
        assert_eq!(msgs.len(), 1, "read pool must see the committed append");
    }

    #[test]
    fn orphaned_recovery_queue_only_tracks_unrecovered_current_turn_rows() {
        let db_path = temp_db_path("session-orphaned-recovered");
        let db = SessionDB::open(&db_path).expect("open session db");
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");

        db.append_message(&session.id, &crate::session::NewMessage::user("old turn"))
            .expect("append old user");
        let mut old_orphan = crate::session::NewMessage::text_block("old partial");
        old_orphan.stream_status = Some("orphaned".to_string());
        db.append_message(&session.id, &old_orphan)
            .expect("append old orphan");
        db.append_message(
            &session.id,
            &crate::session::NewMessage::user("current turn"),
        )
        .expect("append current user");

        assert!(
            db.sessions_with_orphaned_rows()
                .expect("list orphan sessions")
                .is_empty(),
            "orphaned rows before the latest user should not re-trigger startup finalize"
        );

        let mut current_orphan = crate::session::NewMessage::text_block("current partial");
        current_orphan.stream_status = Some("orphaned".to_string());
        db.append_message(&session.id, &current_orphan)
            .expect("append current orphan");

        assert_eq!(
            db.sessions_with_orphaned_rows()
                .expect("list orphan sessions"),
            vec![session.id.clone()]
        );
        assert_eq!(
            db.mark_current_turn_orphaned_rows_recovered(&session.id)
                .expect("mark recovered"),
            1
        );
        assert!(db
            .sessions_with_orphaned_rows()
            .expect("list orphan sessions after recover")
            .is_empty());

        let rows = db
            .load_current_turn_tail(&session.id)
            .expect("load current tail");
        assert_eq!(
            rows.iter()
                .find(|row| row.content == "current partial")
                .and_then(|row| row.stream_status.as_deref()),
            Some("recovered")
        );
        assert!(
            rows.iter().all(|row| row.content != "old partial"),
            "old-turn rows are outside load_current_turn_tail"
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn current_turn_orphaned_has_later_event_detects_existing_finalize_notice() {
        let db_path = temp_db_path("session-orphaned-later-event");
        let db = SessionDB::open(&db_path).expect("open session db");
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");

        db.append_message(&session.id, &crate::session::NewMessage::user("hello"))
            .expect("append user");
        let mut orphan = crate::session::NewMessage::text_block("partial");
        orphan.stream_status = Some("orphaned".to_string());
        db.append_message(&session.id, &orphan)
            .expect("append orphan");

        let notice = "上次会话异常中断,已保留中断前的内容";
        assert!(!db
            .current_turn_orphaned_has_later_event(&session.id, notice)
            .expect("detect before event"));

        db.append_message(
            &session.id,
            &crate::session::NewMessage::error_event(notice),
        )
        .expect("append event");

        assert!(db
            .current_turn_orphaned_has_later_event(&session.id, notice)
            .expect("detect after event"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn tool_media_items_persist_in_attachments_meta() {
        let db_path = temp_db_path("session-tool-media-items");
        let db = SessionDB::open(&db_path).expect("open session db");
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");
        let tool = crate::session::NewMessage::tool(
            "call-media",
            "send_attachment",
            r#"{"path":"/Users/example/report.pdf"}"#,
            "",
            None,
            false,
        );
        db.append_message(&session.id, &tool)
            .expect("append tool row");

        let media_meta =
            crate::session::build_tool_media_items_attachments_meta(&serde_json::json!([{
                "url": "/api/attachments/session/report.pdf",
                "localPath": "/Users/example/.hope-agent/attachments/session/report.pdf",
                "name": "report.pdf",
                "mimeType": "application/pdf",
                "sizeBytes": 42,
                "kind": "file"
            }]))
            .expect("media attachments meta");

        db.update_tool_result_with_side_outputs(
            &session.id,
            "call-media",
            "Sent attachment \"report.pdf\" (42 B) to the user.",
            Some(12),
            false,
            None,
            Some(&media_meta),
        )
        .expect("update tool result");

        let (messages, _, _) = db
            .load_session_messages_latest(&session.id, 20)
            .expect("load messages");
        let tool_row = messages
            .iter()
            .find(|msg| msg.tool_call_id.as_deref() == Some("call-media"))
            .expect("tool row");
        assert_eq!(
            tool_row.tool_result.as_deref(),
            Some("Sent attachment \"report.pdf\" (42 B) to the user.")
        );
        let attachments_meta = tool_row
            .attachments_meta
            .as_deref()
            .expect("attachments meta");
        assert!(attachments_meta.contains("tool_media_items"));
        assert!(attachments_meta.contains("report.pdf"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn title_source_roundtrip_and_guarded_update() {
        let db_path = temp_db_path("session-title-source");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let created = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");
        assert_eq!(
            created.title_source,
            crate::session_title::TITLE_SOURCE_MANUAL
        );

        let fallback = crate::session::ensure_first_message_title(
            &db,
            &created.id,
            "帮我分析这个 Rust 报错",
            None,
        )
        .expect("set first message title");
        assert_eq!(fallback.as_deref(), Some("帮我分析这个 Rust 报错"));

        let first = db
            .get_session(&created.id)
            .expect("get session")
            .expect("session exists");
        assert_eq!(
            first.title_source,
            crate::session_title::TITLE_SOURCE_FIRST_MESSAGE
        );

        db.update_session_title(&created.id, "帮我分析这个 Rust 报错")
            .expect("no-op rename");
        let unchanged = db
            .get_session(&created.id)
            .expect("get unchanged session")
            .expect("session exists");
        assert_eq!(
            unchanged.title_source,
            crate::session_title::TITLE_SOURCE_FIRST_MESSAGE,
            "an unchanged title must not become a manual rename"
        );

        let changed = db
            .update_session_title_if_source(
                &created.id,
                crate::session_title::TITLE_SOURCE_FIRST_MESSAGE,
                "Rust 报错分析",
                crate::session_title::TITLE_SOURCE_LLM,
            )
            .expect("guarded update");
        assert!(changed);

        db.update_session_title(&created.id, "Manual Title")
            .expect("manual rename");
        let blocked = db
            .update_session_title_if_source(
                &created.id,
                crate::session_title::TITLE_SOURCE_FIRST_MESSAGE,
                "Should Not Win",
                crate::session_title::TITLE_SOURCE_LLM,
            )
            .expect("guarded update after manual");
        assert!(!blocked);

        let loaded = db
            .get_session(&created.id)
            .expect("get session")
            .expect("session exists");
        assert_eq!(loaded.title.as_deref(), Some("Manual Title"));
        assert_eq!(
            loaded.title_source,
            crate::session_title::TITLE_SOURCE_MANUAL
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn save_context_if_unchanged_rejects_stale_snapshot() {
        let db_path = temp_db_path("session-context-cas");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let created = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");

        db.save_context(&created.id, r#"["old"]"#)
            .expect("seed context");
        let saved = db
            .save_context_if_unchanged(&created.id, Some(r#"["old"]"#), r#"["compact"]"#)
            .expect("guarded save");
        assert!(saved);
        assert_eq!(
            db.load_context(&created.id)
                .expect("load context")
                .as_deref(),
            Some(r#"["compact"]"#)
        );

        db.save_context(&created.id, r#"["new turn"]"#)
            .expect("simulate concurrent turn");
        let stale = db
            .save_context_if_unchanged(&created.id, Some(r#"["compact"]"#), r#"["stale"]"#)
            .expect("stale save should not error");
        assert!(!stale);
        assert_eq!(
            db.load_context(&created.id)
                .expect("load context")
                .as_deref(),
            Some(r#"["new turn"]"#)
        );

        let missing = db
            .save_context_if_unchanged("missing-session", None, "[]")
            .expect("missing row should not error");
        assert!(!missing);

        let empty = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session without context");
        let initial = db
            .save_context_if_unchanged(&empty.id, None, r#"["initial"]"#)
            .expect("initial guarded save");
        assert!(initial);
        assert_eq!(
            db.load_context(&empty.id).expect("load context").as_deref(),
            Some(r#"["initial"]"#)
        );
        let unexpected_null = db
            .save_context_if_unchanged(&empty.id, None, r#"["overwrite"]"#)
            .expect("non-null context should not match null expectation");
        assert!(!unexpected_null);
        assert_eq!(
            db.load_context(&empty.id).expect("load context").as_deref(),
            Some(r#"["initial"]"#)
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn create_and_update_session_roundtrip_incognito() {
        let db_path = temp_db_path("session-incognito-roundtrip");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let created = db
            .create_session_with_project(crate::agent_loader::DEFAULT_AGENT_ID, None, Some(true))
            .expect("create session");
        assert!(
            created.incognito,
            "created meta should reflect incognito=true"
        );

        let loaded = db
            .get_session(&created.id)
            .expect("get session")
            .expect("session exists");
        assert!(
            loaded.incognito,
            "stored session should persist incognito=true"
        );

        db.update_session_incognito(&created.id, false)
            .expect("update session incognito");
        let updated = db
            .get_session(&created.id)
            .expect("get updated session")
            .expect("updated session exists");
        assert!(
            !updated.incognito,
            "updated session should persist incognito=false"
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn update_session_incognito_rejects_durable_control_plane_state() {
        let db_path = temp_db_path("session-incognito-durable-control-plane");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let workflow_mode_session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create workflow mode session");
        db.update_session_workflow_mode(
            &workflow_mode_session.id,
            crate::workflow_mode::WorkflowMode::On,
        )
        .expect("enable workflow mode");
        assert!(db
            .update_session_incognito(&workflow_mode_session.id, true)
            .expect_err("workflow mode session should not become incognito")
            .to_string()
            .contains("Workflow Mode"));

        let goal_session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create goal session");
        db.create_goal(crate::goal::CreateGoalInput {
            session_id: goal_session.id.clone(),
            objective: "Ship durable goal".to_string(),
            completion_criteria: "Evidence is linked".to_string(),
            domain: None,
            workflow_template_id: None,
            workflow_template_version: None,
            workflow_task_type: None,
            budget_token_limit: None,
            budget_time_limit_secs: None,
            budget_turn_limit: None,
        })
        .expect("create goal");
        assert!(db
            .update_session_incognito(&goal_session.id, true)
            .expect_err("open goal session should not become incognito")
            .to_string()
            .contains("open Goal"));

        let workflow_run_session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create workflow run session");
        db.create_workflow_run(crate::workflow::CreateWorkflowRunInput {
            session_id: workflow_run_session.id.clone(),
            kind: "general.workflow".to_string(),
            execution_mode: "guarded".to_string(),
            script_source: "export default async function main(workflow) { await workflow.finish({ summary: 'done' }); }".to_string(),
            budget: serde_json::json!({}),
            parent_run_id: None,
            origin: None,
            goal_id: None,
            goal_criterion_id: None,
            worktree_id: None,
        })
        .expect("create workflow run");
        assert!(db
            .update_session_incognito(&workflow_run_session.id, true)
            .expect_err("workflow run session should not become incognito")
            .to_string()
            .contains("workflow run"));

        assert!(db
            .update_session_incognito("missing-session", false)
            .expect_err("missing session incognito update should fail")
            .to_string()
            .contains("Session not found"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn create_session_in_project_binds_and_coerces_incognito() {
        // Backs the lazy project-session flow: the first message's `chat`
        // command auto-creates the session with `project_id`. Verify the binding
        // persists and that a requested incognito is coerced off (project +
        // incognito are mutually exclusive — the single source of that rule).
        let db_path = temp_db_path("session-project-incognito-coerce");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let created = db
            .create_session_with_project(
                crate::agent_loader::DEFAULT_AGENT_ID,
                Some("proj-123"),
                Some(true),
            )
            .expect("create project session");
        assert_eq!(
            created.project_id.as_deref(),
            Some("proj-123"),
            "returned meta should carry the project binding"
        );
        assert!(
            !created.incognito,
            "a project binding must coerce incognito off"
        );

        let loaded = db
            .get_session(&created.id)
            .expect("get session")
            .expect("session exists");
        assert_eq!(loaded.project_id.as_deref(), Some("proj-123"));
        assert!(!loaded.incognito, "stored session must not be incognito");

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn update_session_working_dir_roundtrip_and_validation() {
        let db_path = temp_db_path("session-working-dir-roundtrip");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let created = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");
        assert!(
            created.working_dir.is_none(),
            "fresh session should have no working_dir"
        );

        // Valid existing directory — use the temp dir itself for portability.
        let tmp = std::env::temp_dir();
        let tmp_str = tmp.to_string_lossy().to_string();
        let canon = db
            .update_session_working_dir(&created.id, Some(tmp_str.clone()))
            .expect("set working_dir");
        assert!(
            canon.is_some(),
            "valid directory should return canonical path"
        );

        let loaded = db
            .get_session(&created.id)
            .expect("get session")
            .expect("session exists");
        assert!(
            loaded.working_dir.is_some(),
            "stored session should persist working_dir"
        );

        // Non-existent path should error without mutating the row.
        let bad = db.update_session_working_dir(
            &created.id,
            Some("/definitely/not/a/real/path/xyz-42".to_string()),
        );
        assert!(bad.is_err(), "non-existent path should error");
        let still_set = db
            .get_session(&created.id)
            .expect("get session again")
            .expect("session exists")
            .working_dir;
        assert!(
            still_set.is_some(),
            "previous value should remain after failed update"
        );

        // Clearing with None / empty string.
        let cleared = db
            .update_session_working_dir(&created.id, None)
            .expect("clear working_dir");
        assert!(cleared.is_none(), "clearing should return None");
        let after_clear = db
            .get_session(&created.id)
            .expect("get session after clear")
            .expect("session exists");
        assert!(
            after_clear.working_dir.is_none(),
            "working_dir should be None after clear"
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn legacy_runtime_defaults_are_snapshotted_once_and_null_temperature_is_explicit() {
        let db_path = temp_db_path("session-runtime-defaults");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);
        let created = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");
        {
            let conn = db.conn.lock().expect("lock connection");
            conn.execute(
                "UPDATE sessions SET runtime_defaults_initialized = 0, temperature = NULL, reasoning_effort = NULL WHERE id = ?1",
                rusqlite::params![created.id],
            )
            .expect("simulate legacy row");
        }

        db.initialize_session_runtime_defaults(&created.id, None, None, None, None, "high")
            .expect("snapshot defaults");
        let loaded = db
            .get_session(&created.id)
            .expect("read session")
            .expect("session exists");
        assert!(loaded.runtime_defaults_initialized);
        assert_eq!(loaded.temperature, None);
        assert_eq!(loaded.reasoning_effort.as_deref(), Some("high"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn ask_user_timeout_transition_is_atomic_and_reads_do_not_steal_it() {
        let db_path = temp_db_path("ask-user-timeout-transition");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");
        let now = chrono::Utc::now().timestamp().max(1) as u64;
        let group: crate::ask_user::AskUserQuestionGroup =
            serde_json::from_value(serde_json::json!({
                "requestId": "owner-timeout-1",
                "sessionId": session.id,
                "questions": [],
                "source": "owner",
                "timeoutAt": now - 1,
                "timeoutSecs": 1,
                "ownerResponse": { "action": "record_domain_evidence" }
            }))
            .expect("deserialize ask_user group");
        db.save_ask_user_group(&group).expect("save ask_user group");

        assert!(
            db.list_pending_ask_user_groups_for_session(&group.session_id)
                .expect("list pending groups")
                .is_empty(),
            "expired groups must not be restored to the UI"
        );
        {
            let conn = db.conn.lock().expect("lock connection");
            let status: String = conn
                .query_row(
                    "SELECT status FROM ask_user_questions WHERE request_id = ?1",
                    rusqlite::params![group.request_id],
                    |row| row.get(0),
                )
                .expect("read pending status");
            assert_eq!(
                status, "pending",
                "read paths must not consume the timeout transition"
            );
        }

        assert!(
            db.mark_ask_user_timed_out(&group.request_id)
                .expect("expire due group"),
            "the first timer must win the terminal transition"
        );
        assert!(
            !db.mark_ask_user_timed_out(&group.request_id)
                .expect("repeat timeout transition"),
            "duplicate timers must not emit a second terminal transition"
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn session_memory_policy_defaults_to_inherit_and_is_session_scoped() {
        let db_path = temp_db_path("session-memory-policy");
        let db = SessionDB::open(&db_path).expect("open session db");
        let first = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create first session");
        let second = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create second session");

        assert_eq!(
            db.get_memory_policy(&first.id).expect("default policy"),
            crate::session::SessionMemoryPolicy::default()
        );
        let saved = crate::session::SessionMemoryPolicy {
            use_memories: crate::session::SessionMemoryPolicyValue::Deny,
            contribute_to_memories: crate::session::SessionMemoryPolicyValue::Allow,
        };
        db.set_memory_policy(&first.id, saved)
            .expect("save memory policy");
        assert_eq!(db.get_memory_policy(&first.id).unwrap(), saved);
        assert_eq!(
            db.get_memory_policy(&second.id).unwrap(),
            crate::session::SessionMemoryPolicy::default()
        );
        assert!(db.get_memory_policy("missing-session").is_err());

        let conn = db.conn.lock().expect("lock connection");
        conn.execute(
            "DELETE FROM sessions WHERE id = ?1",
            rusqlite::params![first.id],
        )
        .expect("delete session row");
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM session_memory_policy WHERE session_id = ?1",
                rusqlite::params![first.id],
                |row| row.get(0),
            )
            .expect("count memory policy rows");
        assert_eq!(remaining, 0);
        drop(conn);

        let _ = std::fs::remove_file(&db_path);
    }
}

const SEARCH_MATCH_KIND_MESSAGE: &str = "message";
const SEARCH_MATCH_KIND_TITLE: &str = "title";

fn build_search_like_pattern(query: &str) -> Option<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut pattern = String::with_capacity(trimmed.len() + 2);
    pattern.push('%');
    for ch in trimmed.chars() {
        match ch {
            '%' | '_' | '\\' => {
                pattern.push('\\');
                pattern.push(ch);
            }
            _ => pattern.push(ch),
        }
    }
    pattern.push('%');
    Some(pattern)
}

fn find_search_range(text: &str, query: &str) -> Option<(usize, usize)> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }

    find_one_range(text, trimmed).or_else(|| {
        trimmed
            .split_whitespace()
            .filter(|token| !token.is_empty())
            .filter_map(|token| find_one_range(text, token))
            .next()
    })
}

fn find_one_range(text: &str, needle: &str) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return None;
    }
    if let Some(start) = text.find(needle) {
        return Some((start, start + needle.len()));
    }
    if text.is_ascii() && needle.is_ascii() {
        let lower_text = text.to_ascii_lowercase();
        let lower_needle = needle.to_ascii_lowercase();
        if let Some(start) = lower_text.find(&lower_needle) {
            return Some((start, start + needle.len()));
        }
    }
    None
}

fn byte_index_before_chars(text: &str, end: usize, count: usize) -> usize {
    if count == 0 || end == 0 {
        return end;
    }
    text[..end]
        .char_indices()
        .rev()
        .nth(count.saturating_sub(1))
        .map(|(idx, _)| idx)
        .unwrap_or(0)
}

fn byte_index_after_chars(text: &str, start: usize, count: usize) -> usize {
    if count == 0 || start >= text.len() {
        return start;
    }
    text[start..]
        .char_indices()
        .nth(count)
        .map(|(idx, _)| start + idx)
        .unwrap_or(text.len())
}

fn highlighted_search_snippet(text: &str, query: &str, context_chars: usize) -> String {
    let Some((hit_start, hit_end)) = find_search_range(text, query) else {
        return crate::truncate_utf8(text.trim(), 240).to_string();
    };

    let start = byte_index_before_chars(text, hit_start, context_chars);
    let end = byte_index_after_chars(text, hit_end, context_chars);
    let mut snippet = String::new();
    if start > 0 {
        snippet.push('…');
    }
    snippet.push_str(&text[start..hit_start]);
    snippet.push('\u{0002}');
    snippet.push_str(&text[hit_start..hit_end]);
    snippet.push('\u{0003}');
    snippet.push_str(&text[hit_end..end]);
    if end < text.len() {
        snippet.push('…');
    }
    snippet
}

/// Sanitize query for FTS5 MATCH: wrap each token in double quotes for exact matching.
pub(crate) fn sanitize_fts_query(query: &str) -> String {
    let tokens: Vec<String> = query
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
    tokens.join(" ")
}

/// Sanitize query for the FTS5 trigram index.
///
/// SQLite's built-in trigram tokenizer can only usefully match terms with at
/// least three Unicode scalar values. Shorter terms still go through the title
/// LIKE path and the regular token FTS path, but we do not fall back to scanning
/// every message body for them.
fn sanitize_trigram_query(query: &str) -> String {
    let tokens: Vec<String> = query
        .split_whitespace()
        .map(|t| t.replace('"', ""))
        .filter(|t| t.chars().count() >= 3)
        .map(|t| format!("\"{}\"", t))
        .collect();
    tokens.join(" ")
}

/// Strip the FTS5 snippet sentinels (STX/ETX = U+0002/U+0003) emitted by
/// `snippet(..., 0, …)` calls. Frontends that render `<mark>`
/// split on these bytes; surfaces without a `<mark>` rendering pipeline
/// (slash pickers, plain-text IM replies) want the bare text instead.
/// Also collapses newlines to spaces and UTF-8-safe truncates to
/// `max_bytes`.
pub(crate) fn strip_fts_snippet_sentinels(raw: &str, max_bytes: usize) -> String {
    let cleaned = raw
        .replace(['\u{0002}', '\u{0003}'], "")
        .replace(['\n', '\r'], " ");
    crate::truncate_utf8(cleaned.trim(), max_bytes).to_string()
}

/// Filter sessions by their project assignment in `list_sessions_paged`.
#[derive(Debug, Clone, Copy)]
pub enum ProjectFilter<'a> {
    /// No project filter — include sessions regardless of project assignment.
    All,
    /// Only sessions with `project_id IS NULL` (not belonging to any project).
    Unassigned,
    /// Only sessions belonging to the given project id.
    InProject(&'a str),
}

/// Filter sessions by whether they are top-level or sub-agent child sessions.
#[derive(Debug, Clone, Copy)]
pub enum ParentSessionFilter {
    /// Include both top-level and child sessions.
    All,
    /// Only top-level sessions (`parent_session_id IS NULL`).
    Root,
    /// Only sub-agent child sessions (`parent_session_id IS NOT NULL`).
    Child,
}

/// Filter for `search_messages` by session type.
#[derive(Debug, Clone, Copy)]
pub enum SessionTypeFilter {
    /// Regular chat session (not cron / subagent / channel).
    Regular,
    /// Cron-triggered session (`is_cron = 1`).
    Cron,
    /// Sub-agent session (`parent_session_id IS NOT NULL`).
    Subagent,
    /// IM channel session (present in `channel_conversations`).
    Channel,
}

impl SessionTypeFilter {
    /// Parse a string (as received from commands / HTTP) into a filter.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "regular" | "session" => Some(Self::Regular),
            "cron" => Some(Self::Cron),
            "subagent" | "sub_agent" => Some(Self::Subagent),
            "channel" => Some(Self::Channel),
            _ => None,
        }
    }
}

/// Result from searching session message history.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSearchResult {
    pub message_id: i64,
    pub session_id: String,
    pub session_title: Option<String>,
    pub agent_id: String,
    pub message_role: String,
    /// Context snippet containing `<mark>...</mark>` around matched terms.
    pub content_snippet: String,
    pub timestamp: String,
    pub relevance_rank: f64,
    pub is_cron: bool,
    pub parent_session_id: Option<String>,
    /// Project id when this hit belongs to a project-bound chat session.
    pub project_id: Option<String>,
    /// Source channel plugin id (e.g. "telegram", "wechat"), when this session
    /// originates from an IM channel.
    pub channel_type: Option<String>,
    /// IM channel chat kind (e.g. "dm", "group") when applicable.
    pub channel_chat_type: Option<String>,
    /// What matched the search query: `message` for persisted chat content,
    /// `title` for the session title row.
    pub match_kind: String,
}
