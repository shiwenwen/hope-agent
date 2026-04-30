use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use super::types::{ChannelSessionInfo, MessageRole, NewMessage, SessionMessage, SessionMeta};

// ── Database Manager ─────────────────────────────────────────────

pub struct SessionDB {
    pub(crate) conn: Mutex<Connection>,
}

/// Log at `app_info!` when `align_window_to_user_boundary` extends a page by
/// more than this many rows. Signals that a single user turn spans far more
/// DB rows than the requested page size — useful for tuning PAGE_SIZE and
/// deciding when virtual scrolling becomes necessary.
const LARGE_TURN_EXTENSION_LOG_THRESHOLD: usize = 200;

/// Shared SELECT for every query that hydrates a full `SessionMeta`. Column
/// positions are locked to the parser in `SessionDB::row_to_session_meta`;
/// when adding a column, append it and update both the mapper and tests.
const SESSION_META_SELECT: &str = "SELECT s.id, s.title, s.agent_id, s.provider_id, s.provider_name, s.model_id,
           s.created_at, s.updated_at,
           (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) as msg_count,
           (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id AND m.id > COALESCE(s.last_read_message_id, 0) AND m.role = 'assistant') as unread_count,
           EXISTS(
             SELECT 1 FROM messages m
             WHERE m.session_id = s.id
               AND m.id = (SELECT MAX(m2.id) FROM messages m2 WHERE m2.session_id = s.id)
               AND m.is_error = 1
           ) as has_error,
           s.is_cron, s.parent_session_id, s.plan_mode, s.project_id, s.permission_mode, s.incognito,
           cc.channel_id, cc.account_id, cc.chat_id, cc.chat_type, cc.sender_name,
           s.working_dir, s.title_source
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

        // Create tables
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                title TEXT,
                agent_id TEXT NOT NULL DEFAULT 'default',
                provider_id TEXT,
                provider_name TEXT,
                model_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                context_json TEXT,
                last_read_message_id INTEGER DEFAULT 0,
                is_cron INTEGER NOT NULL DEFAULT 0,
                parent_session_id TEXT,
                incognito INTEGER NOT NULL DEFAULT 0,
                title_source TEXT NOT NULL DEFAULT 'manual'
            );

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
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_messages_session_id ON messages(session_id);
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

            -- FTS5 full-text search for message history
            CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                content,
                content='messages',
                content_rowid='id',
                tokenize='unicode61'
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

        // Migration: fix FTS delete trigger — must match INSERT trigger's WHEN clause
        // to avoid "database disk image is malformed" errors during CASCADE delete.
        // The old trigger fired for ALL messages but only user/assistant were indexed.
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS messages_fts_ad;
             CREATE TRIGGER messages_fts_ad AFTER DELETE ON messages
             WHEN old.role IN ('user', 'assistant') AND length(old.content) > 0
             BEGIN
                 INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
             END;"
        )?;

        // Rebuild FTS index to fix any existing corruption
        let _ = conn.execute_batch("INSERT INTO messages_fts(messages_fts) VALUES('rebuild');");

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
                agent_id TEXT NOT NULL DEFAULT 'default',
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
                ON session_skill_activation(session_id);",
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

        Ok(Self {
            conn: Mutex::new(conn),
        })
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
                group.timeout_at.map(|n| n as i64),
            ],
        )?;
        Ok(())
    }

    /// Mark every still-pending ask_user_question row as answered. Called on
    /// app startup because any rows left behind from a previous process have
    /// no live in-memory oneshot to deliver answers to — restoring them in
    /// the UI would produce "No pending ask_user_question request" errors.
    pub fn expire_pending_ask_user_groups(&self) -> anyhow::Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let n = conn.execute(
            "UPDATE ask_user_questions
                SET status = 'answered', answered_at = datetime('now')
                WHERE status = 'pending'",
            [],
        )?;
        Ok(n)
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
        conn.execute(
            "UPDATE ask_user_questions
                SET status = 'answered', answered_at = datetime('now')
                WHERE status = 'pending'
                  AND timeout_at IS NOT NULL
                  AND timeout_at > 0
                  AND timeout_at <= strftime('%s','now')",
            [],
        )?;
        let mut stmt = conn.prepare(
            "SELECT payload FROM ask_user_questions
                WHERE status = 'pending' AND session_id = ?1
                ORDER BY created_at ASC
                LIMIT 50",
        )?;
        let rows = stmt.query_map(params![session_id], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            let payload = row?;
            if let Ok(group) =
                serde_json::from_str::<crate::ask_user::AskUserQuestionGroup>(&payload)
            {
                out.push(group);
            }
        }
        Ok(out)
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

        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO sessions (id, agent_id, created_at, updated_at, parent_session_id, project_id, incognito)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, agent_id, now, now, parent_session_id, project_id, incognito],
        )?;

        Ok(SessionMeta {
            id,
            title: None,
            title_source: crate::session_title::TITLE_SOURCE_MANUAL.to_string(),
            agent_id: agent_id.to_string(),
            provider_id: None,
            provider_name: None,
            model_id: None,
            created_at: now.clone(),
            updated_at: now,
            message_count: 0,
            unread_count: 0,
            has_error: false,
            pending_interaction_count: 0,
            is_cron: false,
            parent_session_id: parent_session_id.map(|s| s.to_string()),
            plan_mode: crate::plan::PlanModeState::Off,
            permission_mode: crate::permission::SessionMode::Default,
            project_id: project_id.map(|s| s.to_string()),
            channel_info: None,
            incognito,
            working_dir: None,
        })
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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

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
            "{}{} ORDER BY s.updated_at DESC{}",
            base_sql, where_sql, pagination_clause
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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking, ttft_ms, tokens_in_last,
                    tokens_cache_creation, tokens_cache_read, tool_metadata
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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

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
                    tokens_cache_creation, tokens_cache_read, tool_metadata
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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking, ttft_ms, tokens_in_last,
                    tokens_cache_creation, tokens_cache_read, tool_metadata
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
                    tokens_cache_creation, tokens_cache_read, tool_metadata
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
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
            message_count: row.get(8)?,
            unread_count: row.get(9)?,
            pending_interaction_count: 0,
            has_error: row.get::<_, i64>(10).unwrap_or(0) != 0,
            is_cron: row.get::<_, i64>(11).unwrap_or(0) != 0,
            parent_session_id: row.get(12)?,
            plan_mode: row
                .get::<_, String>(13)
                .map(|s| crate::plan::PlanModeState::from_str(&s))
                .unwrap_or_default(),
            project_id: row.get(14)?,
            permission_mode: row
                .get::<_, String>(15)
                .map(|s| crate::permission::SessionMode::parse_or_default(&s))
                .unwrap_or_default(),
            incognito: row.get::<_, i64>(16).unwrap_or(0) != 0,
            channel_info,
            working_dir: row.get(22).ok().flatten(),
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
                tokens_cache_creation, tokens_cache_read, tool_metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
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
            ],
        )?;

        let msg_id = conn.last_insert_rowid();

        // Update session's updated_at
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;

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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        match metadata {
            Some(md) => {
                conn.execute(
                    "UPDATE messages SET tool_result = ?1, tool_duration_ms = ?2, is_error = ?3, tool_metadata = ?4
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
            None => {
                conn.execute(
                    "UPDATE messages SET tool_result = ?1, tool_duration_ms = ?2, is_error = ?3
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
        self.update_session_title_with_source(
            session_id,
            title,
            crate::session_title::TITLE_SOURCE_MANUAL,
        )
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
            "UPDATE sessions SET provider_id = ?1, provider_name = ?2, model_id = ?3 WHERE id = ?4",
            params![provider_id, provider_name, model_id, session_id],
        )?;
        Ok(())
    }

    /// Update the plan mode state for a session.
    pub fn update_session_plan_mode(
        &self,
        session_id: &str,
        plan_mode: crate::plan::PlanModeState,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET plan_mode = ?1 WHERE id = ?2",
            params![plan_mode.as_str(), session_id],
        )?;
        Ok(())
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
    /// `SESSION_META_SELECT` (22+ cols, 2 COUNT subqueries, channel JOIN)
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
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET incognito = ?1 WHERE id = ?2",
            params![incognito, session_id],
        )?;
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
        conn.execute(
            "UPDATE sessions SET working_dir = ?1 WHERE id = ?2",
            params![canonical, session_id],
        )?;
        app_info!(
            "session",
            "update_session_working_dir",
            "session={} working_dir={}",
            session_id,
            canonical.as_deref().unwrap_or("<none>")
        );
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
        app_info!(
            "session",
            "purge_incognito",
            "purged incognito session {}",
            session_id
        );
        Ok(true)
    }

    /// Drain rows that reference `session_id` in tables without FK cascade
    /// (`session_skill_activation`, `learning_events`, `subagent_runs`,
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
            if let Err(e) = self.delete_session(id) {
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
    pub fn delete_session(&self, session_id: &str) -> Result<()> {
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
                    let _ = conn
                        .execute_batch("INSERT INTO messages_fts(messages_fts) VALUES('rebuild');");
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

        Ok(())
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

    // ── History Search ──────────────────────────────────────────

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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let fts_query = sanitize_fts_query(query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        // Build dynamic WHERE / params. ?1 is the FTS query; subsequent params
        // are added below in order.
        let mut where_clauses: Vec<String> = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        params_vec.push(Box::new(fts_query));

        if let Some(aid) = agent_id {
            let idx = params_vec.len() + 1;
            where_clauses.push(format!("s.agent_id = ?{}", idx));
            params_vec.push(Box::new(aid.to_string()));
        }

        if let Some(sid) = session_id {
            let idx = params_vec.len() + 1;
            where_clauses.push(format!("m.session_id = ?{}", idx));
            params_vec.push(Box::new(sid.to_string()));
        } else {
            // Global FTS path: hide incognito sessions. The in-session search
            // path (Some(sid)) already scopes to one session and is allowed to
            // search incognito content while it is open.
            where_clauses.push("s.incognito = 0".to_string());
        }

        // Session type filter — channel presence is detected via LEFT JOIN.
        if let Some(type_list) = types {
            if !type_list.is_empty() {
                let mut type_clauses: Vec<String> = Vec::new();
                for t in type_list {
                    match t {
                        SessionTypeFilter::Regular => type_clauses.push(
                            "(s.is_cron = 0 AND s.parent_session_id IS NULL AND cc.channel_id IS NULL)".to_string(),
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

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" AND {}", where_clauses.join(" AND "))
        };

        // STX/ETX (U+0002/U+0003) — never valid in user text — so the frontend
        // can split without HTML escape/unescape that could reintroduce an
        // attribute-level XSS from user-authored `<mark onclick=…>`.
        let sql = format!(
            "SELECT m.id, m.session_id, m.role,
                    snippet(messages_fts, 0, '\x02', '\x03', '…', 16) AS snippet,
                    m.timestamp,
                    s.title, s.agent_id, s.is_cron, s.parent_session_id,
                    cc.channel_id, cc.chat_type,
                    fts.rank
             FROM messages_fts fts
             JOIN messages m ON m.id = fts.rowid
             JOIN sessions s ON s.id = m.session_id
             LEFT JOIN channel_conversations cc ON cc.session_id = s.id
             WHERE messages_fts MATCH ?1{}
             ORDER BY fts.rank
             LIMIT {}",
            where_sql, limit
        );

        let mut stmt = conn.prepare(&sql)?;

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

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
                channel_type: row.get(9)?,
                channel_chat_type: row.get(10)?,
                relevance_rank: row.get::<_, f64>(11).unwrap_or(0.0),
            })
        })?;

        let results: Vec<SessionSearchResult> = rows.filter_map(|r| r.ok()).collect();
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
                    tokens_cache_creation, tokens_cache_read, tool_metadata
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
                    tokens_cache_creation, tokens_cache_read, tool_metadata
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
    use super::SessionDB;

    fn ensure_channel_conversations_table(db: &SessionDB) {
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
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                UNIQUE (channel_id, account_id, chat_id, thread_id, session_id)
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
    fn title_source_roundtrip_and_guarded_update() {
        let db_path = temp_db_path("session-title-source");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let created = db.create_session("default").expect("create session");
        assert_eq!(
            created.title_source,
            crate::session_title::TITLE_SOURCE_MANUAL
        );

        let fallback =
            crate::session::ensure_first_message_title(&db, &created.id, "帮我分析这个 Rust 报错")
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
    fn create_and_update_session_roundtrip_incognito() {
        let db_path = temp_db_path("session-incognito-roundtrip");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let created = db
            .create_session_with_project("default", None, Some(true))
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
    fn update_session_working_dir_roundtrip_and_validation() {
        let db_path = temp_db_path("session-working-dir-roundtrip");
        let db = SessionDB::open(&db_path).expect("open session db");
        ensure_channel_conversations_table(&db);

        let created = db.create_session("default").expect("create session");
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
}

/// Sanitize query for FTS5 MATCH: wrap each token in double quotes for exact matching.
fn sanitize_fts_query(query: &str) -> String {
    let tokens: Vec<String> = query
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
    tokens.join(" ")
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
    /// Source channel plugin id (e.g. "telegram", "wechat"), when this session
    /// originates from an IM channel.
    pub channel_type: Option<String>,
    /// IM channel chat kind (e.g. "dm", "group") when applicable.
    pub channel_chat_type: Option<String>,
}
