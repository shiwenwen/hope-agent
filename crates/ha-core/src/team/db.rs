use anyhow::Result;
use rusqlite::params;

use super::types::*;
use crate::session::SessionDB;

impl SessionDB {
    // ── Teams CRUD ──────────────────────────────────────────────

    pub fn insert_team(&self, team: &Team) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let config_json = serde_json::to_string(&team.config)?;
        conn.execute(
            "INSERT INTO teams (team_id, name, description, lead_session_id, lead_agent_id,
             status, created_at, updated_at, template_id, config_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                team.team_id,
                team.name,
                team.description,
                team.lead_session_id,
                team.lead_agent_id,
                team.status.as_str(),
                team.created_at,
                team.updated_at,
                team.template_id,
                config_json,
            ],
        )?;
        Ok(())
    }

    pub fn get_team(&self, team_id: &str) -> Result<Option<Team>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT team_id, name, description, lead_session_id, lead_agent_id,
                    status, created_at, updated_at, template_id, config_json
             FROM teams WHERE team_id = ?1",
        )?;
        let result = stmt.query_row(params![team_id], Self::row_to_team);
        match result {
            Ok(team) => Ok(Some(team)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list_teams_by_session(&self, session_id: &str) -> Result<Vec<Team>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT team_id, name, description, lead_session_id, lead_agent_id,
                    status, created_at, updated_at, template_id, config_json
             FROM teams WHERE lead_session_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![session_id], Self::row_to_team)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn list_active_teams(&self) -> Result<Vec<Team>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT team_id, name, description, lead_session_id, lead_agent_id,
                    status, created_at, updated_at, template_id, config_json
             FROM teams WHERE status = 'active' ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], Self::row_to_team)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn update_team_status(&self, team_id: &str, status: &TeamStatus) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE teams SET status = ?1, updated_at = datetime('now') WHERE team_id = ?2",
            params![status.as_str(), team_id],
        )?;
        Ok(())
    }

    pub fn count_active_teams_for_agent(&self, agent_id: &str) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM teams WHERE lead_agent_id = ?1 AND status = 'active'",
            params![agent_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    // ── Team Members CRUD ───────────────────────────────────────

    pub fn insert_team_member(&self, member: &TeamMember) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO team_members (member_id, team_id, name, agent_id, role, status,
             run_id, session_id, color, current_task_id, model_override, role_description,
             joined_at, last_active_at, input_tokens, output_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                member.member_id,
                member.team_id,
                member.name,
                member.agent_id,
                member.role.as_str(),
                member.status.as_str(),
                member.run_id,
                member.session_id,
                member.color,
                member.current_task_id,
                member.model_override,
                member.role_description,
                member.joined_at,
                member.last_active_at,
                member.input_tokens.unwrap_or(0) as i64,
                member.output_tokens.unwrap_or(0) as i64,
            ],
        )?;
        Ok(())
    }

    pub fn get_team_member(&self, member_id: &str) -> Result<Option<TeamMember>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let result = conn.query_row(
            "SELECT member_id, team_id, name, agent_id, role, status,
                    run_id, session_id, color, current_task_id, model_override, role_description,
                    joined_at, last_active_at, input_tokens, output_tokens
             FROM team_members WHERE member_id = ?1",
            params![member_id],
            Self::row_to_team_member,
        );
        match result {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list_team_members(&self, team_id: &str) -> Result<Vec<TeamMember>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT member_id, team_id, name, agent_id, role, status,
                    run_id, session_id, color, current_task_id, model_override, role_description,
                    joined_at, last_active_at, input_tokens, output_tokens
             FROM team_members WHERE team_id = ?1 ORDER BY joined_at ASC",
        )?;
        let rows = stmt.query_map(params![team_id], Self::row_to_team_member)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn update_team_member_status(&self, member_id: &str, status: &MemberStatus) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE team_members SET status = ?1, last_active_at = datetime('now')
             WHERE member_id = ?2",
            params![status.as_str(), member_id],
        )?;
        Ok(())
    }

    pub fn update_team_member_run(
        &self,
        member_id: &str,
        run_id: &str,
        session_id: &str,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE team_members SET run_id = ?1, session_id = ?2, status = 'working',
             last_active_at = datetime('now') WHERE member_id = ?3",
            params![run_id, session_id, member_id],
        )?;
        Ok(())
    }

    pub fn update_team_member_tokens(
        &self,
        member_id: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE team_members SET input_tokens = ?1, output_tokens = ?2 WHERE member_id = ?3",
            params![input_tokens as i64, output_tokens as i64, member_id],
        )?;
        Ok(())
    }

    pub fn update_team_member_task(&self, member_id: &str, task_id: Option<i64>) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE team_members SET current_task_id = ?1 WHERE member_id = ?2",
            params![task_id, member_id],
        )?;
        Ok(())
    }

    pub fn delete_team_member(&self, member_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "DELETE FROM team_members WHERE member_id = ?1",
            params![member_id],
        )?;
        Ok(())
    }

    pub fn find_team_member_by_name(
        &self,
        team_id: &str,
        name: &str,
    ) -> Result<Option<TeamMember>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let result = conn.query_row(
            "SELECT member_id, team_id, name, agent_id, role, status,
                    run_id, session_id, color, current_task_id, model_override, role_description,
                    joined_at, last_active_at, input_tokens, output_tokens
             FROM team_members WHERE team_id = ?1 AND name = ?2",
            params![team_id, name],
            Self::row_to_team_member,
        );
        match result {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn find_team_member_by_run_id(&self, run_id: &str) -> Result<Option<TeamMember>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let result = conn.query_row(
            "SELECT member_id, team_id, name, agent_id, role, status,
                    run_id, session_id, color, current_task_id, model_override, role_description,
                    joined_at, last_active_at, input_tokens, output_tokens
             FROM team_members WHERE run_id = ?1",
            params![run_id],
            Self::row_to_team_member,
        );
        match result {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn row_to_team(row: &rusqlite::Row) -> rusqlite::Result<Team> {
        let config_json: String = row.get(9)?;
        Ok(Team {
            team_id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            lead_session_id: row.get(3)?,
            lead_agent_id: row.get(4)?,
            status: TeamStatus::from_str(&row.get::<_, String>(5)?),
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
            template_id: row.get(8)?,
            config: serde_json::from_str(&config_json).unwrap_or_default(),
        })
    }

    fn row_to_team_member(row: &rusqlite::Row) -> rusqlite::Result<TeamMember> {
        let input: i64 = row.get(14)?;
        let output: i64 = row.get(15)?;
        Ok(TeamMember {
            member_id: row.get(0)?,
            team_id: row.get(1)?,
            name: row.get(2)?,
            agent_id: row.get(3)?,
            role: MemberRole::from_str(&row.get::<_, String>(4)?),
            status: MemberStatus::from_str(&row.get::<_, String>(5)?),
            run_id: row.get(6)?,
            session_id: row.get(7)?,
            color: row.get(8)?,
            current_task_id: row.get(9)?,
            model_override: row.get(10)?,
            role_description: row.get(11)?,
            joined_at: row.get(12)?,
            last_active_at: row.get(13)?,
            input_tokens: Some(input as u64),
            output_tokens: Some(output as u64),
        })
    }

    // ── Team Messages ───────────────────────────────────────────

    pub fn insert_team_message(&self, msg: &TeamMessage) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO team_messages (message_id, team_id, from_member_id, to_member_id,
             content, message_type, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                msg.message_id,
                msg.team_id,
                msg.from_member_id,
                msg.to_member_id,
                msg.content,
                msg.message_type.as_str(),
                msg.timestamp,
            ],
        )?;
        Ok(())
    }

    /// Load the latest `limit` team messages in ASC order, with a `has_more`
    /// flag indicating whether older messages exist beyond the window.
    ///
    /// Uses composite cursor `(timestamp, message_id)` so same-millisecond
    /// inserts are paginated deterministically. `timestamp` is RFC3339 so
    /// lexicographic comparison matches chronological order.
    pub fn list_team_messages_latest(
        &self,
        team_id: &str,
        limit: u32,
    ) -> Result<(Vec<TeamMessage>, bool)> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT message_id, team_id, from_member_id, to_member_id,
                    content, message_type, timestamp
             FROM team_messages WHERE team_id = ?1
             ORDER BY timestamp DESC, message_id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![team_id, limit as i64], Self::row_to_team_message)?;
        let mut messages: Vec<TeamMessage> = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        messages.reverse(); // oldest first

        let has_more = match messages.first() {
            Some(first) => Self::has_team_messages_before(&conn, team_id, first)?,
            None => false,
        };

        Ok((messages, has_more))
    }

    /// Load messages strictly older than the given cursor in ASC order, with
    /// `has_more`. Cursor is `(before_timestamp, before_message_id)` — the
    /// first message currently in view (client-maintained oldest cursor).
    pub fn list_team_messages_before(
        &self,
        team_id: &str,
        before_timestamp: &str,
        before_message_id: &str,
        limit: u32,
    ) -> Result<(Vec<TeamMessage>, bool)> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT message_id, team_id, from_member_id, to_member_id,
                    content, message_type, timestamp
             FROM team_messages
             WHERE team_id = ?1
               AND (timestamp < ?2
                    OR (timestamp = ?2 AND message_id < ?3))
             ORDER BY timestamp DESC, message_id DESC
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            params![team_id, before_timestamp, before_message_id, limit as i64],
            Self::row_to_team_message,
        )?;
        let mut messages: Vec<TeamMessage> = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        messages.reverse();

        let has_more = match messages.first() {
            Some(first) => Self::has_team_messages_before(&conn, team_id, first)?,
            None => false,
        };

        Ok((messages, has_more))
    }

    fn row_to_team_message(row: &rusqlite::Row) -> rusqlite::Result<TeamMessage> {
        Ok(TeamMessage {
            message_id: row.get(0)?,
            team_id: row.get(1)?,
            from_member_id: row.get(2)?,
            to_member_id: row.get(3)?,
            content: row.get(4)?,
            message_type: TeamMessageType::from_str(&row.get::<_, String>(5)?),
            timestamp: row.get(6)?,
        })
    }

    fn has_team_messages_before(
        conn: &rusqlite::Connection,
        team_id: &str,
        first: &TeamMessage,
    ) -> Result<bool> {
        let result: rusqlite::Result<i64> = conn.query_row(
            "SELECT 1 FROM team_messages
             WHERE team_id = ?1
               AND (timestamp < ?2
                    OR (timestamp = ?2 AND message_id < ?3))
             LIMIT 1",
            params![team_id, first.timestamp, first.message_id],
            |row| row.get(0),
        );
        match result {
            Ok(_) => Ok(true),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    // ── Team Tasks ──────────────────────────────────────────────

    pub fn insert_team_task(&self, task: &TeamTask) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let blocked_by = serde_json::to_string(&task.blocked_by)?;
        let blocks = serde_json::to_string(&task.blocks)?;
        conn.execute(
            "INSERT INTO team_tasks (team_id, content, status, owner_member_id, priority,
             blocked_by, blocks, column_name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                task.team_id,
                task.content,
                task.status,
                task.owner_member_id,
                task.priority,
                blocked_by,
                blocks,
                task.column_name,
                task.created_at,
                task.updated_at,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_team_task(&self, task_id: i64) -> Result<Option<TeamTask>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let result = conn.query_row(
            "SELECT id, team_id, content, status, owner_member_id, priority,
                    blocked_by, blocks, column_name, created_at, updated_at
             FROM team_tasks WHERE id = ?1",
            params![task_id],
            Self::row_to_team_task,
        );
        match result {
            Ok(t) => Ok(Some(t)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list_team_tasks(&self, team_id: &str) -> Result<Vec<TeamTask>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, team_id, content, status, owner_member_id, priority,
                    blocked_by, blocks, column_name, created_at, updated_at
             FROM team_tasks WHERE team_id = ?1 ORDER BY priority ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![team_id], Self::row_to_team_task)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn update_team_task(
        &self,
        task_id: i64,
        status: Option<&str>,
        owner: Option<&str>,
        column: Option<&str>,
        content: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut updates = vec!["updated_at = datetime('now')".to_string()];
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(s) = status {
            updates.push(format!("status = ?{}", values.len() + 1));
            values.push(Box::new(s.to_string()));
        }
        if let Some(o) = owner {
            updates.push(format!("owner_member_id = ?{}", values.len() + 1));
            values.push(Box::new(o.to_string()));
        }
        if let Some(c) = column {
            updates.push(format!("column_name = ?{}", values.len() + 1));
            values.push(Box::new(c.to_string()));
        }
        if let Some(ct) = content {
            updates.push(format!("content = ?{}", values.len() + 1));
            values.push(Box::new(ct.to_string()));
        }

        let idx = values.len() + 1;
        let sql = format!(
            "UPDATE team_tasks SET {} WHERE id = ?{}",
            updates.join(", "),
            idx
        );
        values.push(Box::new(task_id));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            values.iter().map(|b| b.as_ref()).collect();
        conn.execute(&sql, param_refs.as_slice())?;
        Ok(())
    }

    pub fn delete_team_task(&self, task_id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute("DELETE FROM team_tasks WHERE id = ?1", params![task_id])?;
        Ok(())
    }

    fn row_to_team_task(row: &rusqlite::Row) -> rusqlite::Result<TeamTask> {
        let blocked_by_str: String = row.get(6)?;
        let blocks_str: String = row.get(7)?;
        Ok(TeamTask {
            id: row.get(0)?,
            team_id: row.get(1)?,
            content: row.get(2)?,
            status: row.get(3)?,
            owner_member_id: row.get(4)?,
            priority: row.get::<_, i64>(5)? as u32,
            blocked_by: serde_json::from_str(&blocked_by_str).unwrap_or_default(),
            blocks: serde_json::from_str(&blocks_str).unwrap_or_default(),
            column_name: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
        })
    }

    // ── Team Templates ──────────────────────────────────────────

    /// Insert or replace a team template. Returns the stored row with
    /// server-assigned `created_at` / `updated_at` so callers don't need a
    /// follow-up SELECT to read back the timestamps.
    pub fn insert_team_template(&self, tpl: &TeamTemplate) -> Result<TeamTemplate> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let members_json = serde_json::to_string(&tpl.members)?;
        let now = chrono::Utc::now().to_rfc3339();
        let created_at = if tpl.created_at.is_empty() {
            now.clone()
        } else {
            tpl.created_at.clone()
        };
        let updated_at = if tpl.updated_at.is_empty() {
            now
        } else {
            tpl.updated_at.clone()
        };
        conn.execute(
            "INSERT OR REPLACE INTO team_templates (template_id, name, description,
             members_json, builtin, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6)",
            params![
                tpl.template_id,
                tpl.name,
                tpl.description,
                members_json,
                created_at,
                updated_at,
            ],
        )?;
        Ok(TeamTemplate {
            template_id: tpl.template_id.clone(),
            name: tpl.name.clone(),
            description: tpl.description.clone(),
            members: tpl.members.clone(),
            created_at,
            updated_at,
        })
    }

    pub fn list_team_templates(&self) -> Result<Vec<TeamTemplate>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT template_id, name, description, members_json, created_at, updated_at
             FROM team_templates ORDER BY updated_at DESC, name ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let members_json: String = row.get(3)?;
            Ok(TeamTemplate {
                template_id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                members: serde_json::from_str(&members_json).unwrap_or_default(),
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn delete_team_template(&self, template_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "DELETE FROM team_templates WHERE template_id = ?1",
            params![template_id],
        )?;
        Ok(())
    }
}
