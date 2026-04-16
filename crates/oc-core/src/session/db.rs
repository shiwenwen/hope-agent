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
                parent_session_id TEXT
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

        // Migration: add cross_session_config_json column for per-session
        // override of the cross-session behavior awareness feature.
        let has_xs_cfg = conn
            .prepare("SELECT cross_session_config_json FROM sessions LIMIT 1")
            .is_ok();
        if !has_xs_cfg {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN cross_session_config_json TEXT;",
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
                status TEXT NOT NULL DEFAULT 'pending',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_tasks_session_id ON tasks(session_id);",
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
                agent_id TEXT NOT NULL DEFAULT 'default',
                role TEXT NOT NULL DEFAULT 'worker',
                status TEXT NOT NULL DEFAULT 'idle',
                run_id TEXT,
                session_id TEXT,
                color TEXT NOT NULL DEFAULT '#3B82F6',
                current_task_id INTEGER,
                model_override TEXT,
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
                created_at TEXT NOT NULL
            );",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
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
            if let Ok(group) = serde_json::from_str::<crate::ask_user::AskUserQuestionGroup>(&payload) {
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
        self.create_session_full(agent_id, parent_session_id, None)
    }

    /// Create a new session attached to a project.
    ///
    /// When `project_id` is `Some`, the session is bound to that project and
    /// project-scoped memories / files will be automatically injected into its
    /// system prompt.
    pub fn create_session_with_project(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
    ) -> Result<SessionMeta> {
        crate::memory_extract::flush_all_idle_extractions();
        self.create_session_full(agent_id, None, project_id)
    }

    /// Fully-parameterized session creator. Private helper called by the other
    /// `create_session*` variants so the INSERT statement exists in exactly one
    /// place.
    pub(crate) fn create_session_full(
        &self,
        agent_id: &str,
        parent_session_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<SessionMeta> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO sessions (id, agent_id, created_at, updated_at, parent_session_id, project_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, agent_id, now, now, parent_session_id, project_id],
        )?;

        Ok(SessionMeta {
            id,
            title: None,
            agent_id: agent_id.to_string(),
            provider_id: None,
            provider_name: None,
            model_id: None,
            created_at: now.clone(),
            updated_at: now,
            message_count: 0,
            unread_count: 0,
            pending_interaction_count: 0,
            is_cron: false,
            parent_session_id: parent_session_id.map(|s| s.to_string()),
            plan_mode: "off".to_string(),
            project_id: project_id.map(|s| s.to_string()),
            channel_info: None,
        })
    }

    /// Move a session to a project (or remove it from the current project when `project_id` is `None`).
    pub fn set_session_project(
        &self,
        session_id: &str,
        project_id: Option<&str>,
    ) -> Result<()> {
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
        let (sessions, _) = self.list_sessions_paged(agent_id, ProjectFilter::All, None, None)?;
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
    ) -> Result<(Vec<SessionMeta>, u32)> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let base_sql = "SELECT s.id, s.title, s.agent_id, s.provider_id, s.provider_name, s.model_id,
                        s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) as msg_count,
                        (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id AND m.id > COALESCE(s.last_read_message_id, 0) AND m.role != 'user') as unread_count,
                        s.is_cron,
                        s.parent_session_id,
                        s.plan_mode,
                        s.project_id,
                        cc.channel_id, cc.account_id, cc.chat_id, cc.chat_type, cc.sender_name
                 FROM sessions s
                 LEFT JOIN channel_conversations cc ON cc.session_id = s.id";

        let count_base = "SELECT COUNT(*) FROM sessions s";

        let row_mapper = |row: &rusqlite::Row| {
            let cc_channel_id: Option<String> = row.get(14)?;
            let channel_info = cc_channel_id.map(|ch_id| ChannelSessionInfo {
                channel_id: ch_id,
                account_id: row.get::<_, String>(15).unwrap_or_default(),
                chat_id: row.get::<_, String>(16).unwrap_or_default(),
                chat_type: row.get::<_, String>(17).unwrap_or_default(),
                sender_name: row.get(18).ok().flatten(),
            });
            Ok(SessionMeta {
                id: row.get(0)?,
                title: row.get(1)?,
                agent_id: row.get(2)?,
                provider_id: row.get(3)?,
                provider_name: row.get(4)?,
                model_id: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                message_count: row.get(8)?,
                unread_count: row.get(9)?,
                pending_interaction_count: 0,
                is_cron: row.get::<_, i64>(10).unwrap_or(0) != 0,
                parent_session_id: row.get(11)?,
                plan_mode: row
                    .get::<_, String>(12)
                    .unwrap_or_else(|_| "off".to_string()),
                project_id: row.get(13)?,
                channel_info,
            })
        };

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
                    tool_duration_ms, is_error, thinking, ttft_ms
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
    /// Returns (messages_in_asc_order, total_count).
    pub fn load_session_messages_latest(
        &self,
        session_id: &str,
        limit: u32,
    ) -> Result<(Vec<SessionMessage>, u32)> {
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
                    tool_duration_ms, is_error, thinking, ttft_ms
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
        Ok((messages, total))
    }

    /// Load messages before a given message id (for "load more" / scroll up).
    /// Returns messages in ASC order.
    pub fn load_session_messages_before(
        &self,
        session_id: &str,
        before_id: i64,
        limit: u32,
    ) -> Result<Vec<SessionMessage>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking, ttft_ms
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
        Ok(messages)
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
                tool_duration_ms, is_error, thinking, ttft_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
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
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
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
        Ok(())
    }

    /// Update session title.
    pub fn update_session_title(&self, session_id: &str, title: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET title = ?1 WHERE id = ?2",
            params![title, session_id],
        )?;
        Ok(())
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
    pub fn update_session_plan_mode(&self, session_id: &str, plan_mode: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET plan_mode = ?1 WHERE id = ?2",
            params![plan_mode, session_id],
        )?;
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

    /// Delete a session and all its messages (CASCADE) and attachments.
    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        // Try direct delete (CASCADE will handle messages + FTS trigger)
        match conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id]) {
            Ok(_) => {}
            Err(e) => {
                // FTS index corrupted — rebuild and retry
                app_warn!(
                    "session",
                    "db",
                    "delete_session failed ({}), rebuilding FTS and retrying",
                    e
                );
                let _ =
                    conn.execute_batch("INSERT INTO messages_fts(messages_fts) VALUES('rebuild');");
                conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;
            }
        }

        // Clean up plan file
        if let Ok(plans_dir) = crate::paths::plans_dir() {
            let _ = std::fs::remove_file(plans_dir.join(format!("{}.md", session_id)));
        }

        // Clean up attachments directory
        if let Ok(att_dir) = crate::paths::attachments_dir(session_id) {
            let _ = std::fs::remove_dir_all(att_dir);
        }

        Ok(())
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
        let mut stmt = conn.prepare(
            "SELECT s.id, s.title, s.agent_id, s.provider_id, s.provider_name, s.model_id,
                    s.created_at, s.updated_at,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) as msg_count,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id AND m.id > COALESCE(s.last_read_message_id, 0) AND m.role != 'user') as unread_count,
                    s.is_cron,
                    s.parent_session_id,
                    s.plan_mode,
                    s.project_id,
                    cc.channel_id, cc.account_id, cc.chat_id, cc.chat_type, cc.sender_name
             FROM sessions s
             LEFT JOIN channel_conversations cc ON cc.session_id = s.id
             WHERE s.id = ?1"
        )?;

        let mut rows = stmt.query_map(params![session_id], |row| {
            let cc_channel_id: Option<String> = row.get(14)?;
            let channel_info = cc_channel_id.map(|ch_id| ChannelSessionInfo {
                channel_id: ch_id,
                account_id: row.get::<_, String>(15).unwrap_or_default(),
                chat_id: row.get::<_, String>(16).unwrap_or_default(),
                chat_type: row.get::<_, String>(17).unwrap_or_default(),
                sender_name: row.get(18).ok().flatten(),
            });
            Ok(SessionMeta {
                id: row.get(0)?,
                title: row.get(1)?,
                agent_id: row.get(2)?,
                provider_id: row.get(3)?,
                provider_name: row.get(4)?,
                model_id: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                message_count: row.get(8)?,
                unread_count: row.get(9)?,
                pending_interaction_count: 0,
                is_cron: row.get::<_, i64>(10).unwrap_or(0) != 0,
                parent_session_id: row.get(11)?,
                plan_mode: row
                    .get::<_, String>(12)
                    .unwrap_or_else(|_| "off".to_string()),
                project_id: row.get(13)?,
                channel_info,
            })
        })?;

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

        let sql = format!(
            "SELECT m.id, m.session_id, m.role,
                    snippet(messages_fts, 0, '<mark>', '</mark>', '…', 16) AS snippet,
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
    /// Returns `(messages_in_asc_order, total_count)`. The window contains up
    /// to `before` messages with `id <= target_message_id` (inclusive of the
    /// target) and up to `after` messages with `id > target_message_id`.
    pub fn load_session_messages_around(
        &self,
        session_id: &str,
        target_message_id: i64,
        before: u32,
        after: u32,
    ) -> Result<(Vec<SessionMessage>, u32)> {
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
                    tool_duration_ms, is_error, thinking, ttft_ms
             FROM messages
             WHERE session_id = ?1 AND id <= ?2
             ORDER BY id DESC
             LIMIT ?3",
        )?;
        let before_rows = before_stmt.query_map(
            params![session_id, target_message_id, before],
            |row| Self::row_to_session_message(row),
        )?;
        let mut before_msgs = Vec::new();
        for row in before_rows {
            before_msgs.push(row?);
        }
        before_msgs.reverse();

        // Load `after` messages with id > target (ASC).
        let mut after_stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp,
                    attachments_meta, model, tokens_in, tokens_out, reasoning_effort,
                    tool_call_id, tool_name, tool_arguments, tool_result,
                    tool_duration_ms, is_error, thinking, ttft_ms
             FROM messages
             WHERE session_id = ?1 AND id > ?2
             ORDER BY id ASC
             LIMIT ?3",
        )?;
        let after_rows = after_stmt.query_map(
            params![session_id, target_message_id, after],
            |row| Self::row_to_session_message(row),
        )?;
        let mut after_msgs = Vec::new();
        for row in after_rows {
            after_msgs.push(row?);
        }

        let mut messages = before_msgs;
        messages.extend(after_msgs);
        Ok((messages, total))
    }

    // ── Cross-session awareness helpers ─────────────────────────

    /// Read the per-session override JSON for cross-session awareness, if any.
    pub fn get_session_cross_session_config_json(
        &self,
        session_id: &str,
    ) -> Result<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt =
            conn.prepare("SELECT cross_session_config_json FROM sessions WHERE id = ?1")?;
        let mut rows = stmt.query(params![session_id])?;
        if let Some(row) = rows.next()? {
            let val: Option<String> = row.get(0)?;
            return Ok(val.filter(|s| !s.is_empty()));
        }
        Ok(None)
    }

    /// Write (or clear with `None`) the per-session override JSON for
    /// cross-session awareness.
    pub fn set_session_cross_session_config_json(
        &self,
        session_id: &str,
        json: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET cross_session_config_json = ?1 WHERE id = ?2",
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
    /// Used by cross-session LLM extraction to give the model concrete recent activity.
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
        let rows = stmt.query_map(
            params![session_id, since_rfc3339, limit as i64],
            |row| row.get::<_, String>(0),
        )?;
        let mut out = Vec::new();
        for row in rows {
            let content = row?;
            out.push(crate::truncate_utf8(content.trim(), max_chars_per_msg).to_string());
        }
        Ok(out)
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
