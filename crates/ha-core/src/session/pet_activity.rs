//! Pet 活动投影的类型化查询（类型随表下沉：跨 sessions / chat_turns /
//! messages / knowledge_chat_threads / design_chat_threads /
//! channel_conversations 六表，特征 crate 不持 raw conn）。
//!
//! 语义属主对话投影边界（见 AGENTS「桌面宠物」红线与 docs/architecture/
//! pet.md）：只取显式携带第一方 `ChatUiSurface` 的最新 turn，排除 cron /
//! 子会话 / IM 接管会话。投影裁剪（status 映射 / 未读判定 / incognito
//! 脱敏）在 ha-pet 侧。

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::OptionalExtension;

use crate::session::{ChatTurnStatus, SessionDB};

/// 一行候选活动（pub 字段：ha-pet `project_row` 消费）。
///
/// **无痕不变量（kernel 边界脱敏）**：`incognito` 行的 `title` /
/// `agent_id` 置空、`preview` 置 `None` 在查询边界完成——公开 DTO 不携带
/// 无痕会话原始内容，不依赖消费者裁剪（ha-pet 投影侧的 incognito 分支是
/// 第二道防线）。
#[derive(Debug)]
pub struct PetActivityRow {
    pub session_id: String,
    pub title: String,
    pub agent_id: String,
    pub project_id: Option<String>,
    pub incognito: bool,
    pub kind: String,
    pub last_read_message_id: i64,
    pub status: ChatTurnStatus,
    pub terminal_message_id: Option<i64>,
    pub updated_at: String,
    pub preview: Option<String>,
    pub kb_id: Option<String>,
    pub anchor_note_path: Option<String>,
    pub design_project_id: Option<String>,
    pub side_source_session_id: Option<String>,
}

fn table_exists(conn: &rusqlite::Connection, name: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

impl SessionDB {
    /// Pet 活动候选行 + 每会话 pending ask_user 组计数（ha-pet
    /// `activity_snapshot` 消费；同步、须经 `SessionDB::run` 进阻塞池）。
    pub fn pet_activity_rows(&self) -> Result<(Vec<PetActivityRow>, HashMap<String, i64>)> {
        let ask_user = self.count_pending_ask_user_groups_per_session()?;
        let conn = self
            .conn
            .lock()
            .map_err(|error| anyhow::anyhow!("Lock error: {error}"))?;
        let knowledge_join = if table_exists(&conn, "knowledge_chat_threads")? {
            "LEFT JOIN knowledge_chat_threads kt ON kt.session_id = s.id"
        } else {
            "LEFT JOIN (SELECT NULL AS session_id, NULL AS kb_id, NULL AS anchor_note_path) kt ON 0"
        };
        let has_channel_table = table_exists(&conn, "channel_conversations")?;
        let channel_clause = if has_channel_table {
            "AND NOT EXISTS (SELECT 1 FROM channel_conversations cc WHERE cc.session_id = s.id)"
        } else {
            ""
        };
        // Hidden side navigation needs an owner the main-chat UI can reveal.
        // Missing ownership/attachment metadata must not create a dead target.
        let side_source_scope = if has_channel_table {
            super::db::regular_session_scope_sql("side_source")
        } else {
            "0".to_string()
        };
        let terminal_message_id = super::turns::TERMINAL_MESSAGE_ID_SQL;
        let sql = format!(
            "SELECT s.id,
                    COALESCE(s.title, ''),
                    s.agent_id,
                    s.project_id,
                    s.incognito,
                    s.kind,
                    COALESCE(s.last_read_message_id, 0),
                    t.status,
                    {terminal_message_id},
                    COALESCE(t.ended_at, t.updated_at, t.started_at),
                    CASE WHEN t.status = 'completed' AND t.assistant_message_id IS NOT NULL THEN (
                        SELECT substr(m.content, 1, 1024)
                          FROM messages m
                         WHERE m.id = t.assistant_message_id
                           AND m.session_id = s.id
                           AND m.role = 'assistant'
                    ) END,
                    kt.kb_id,
                    kt.anchor_note_path,
                    dt.project_id,
                    s.forked_from_session_id
               FROM sessions s
               {knowledge_join}
               LEFT JOIN design_chat_threads dt ON dt.session_id = s.id
               JOIN chat_turns t ON t.id = (
                    SELECT t2.id
                      FROM chat_turns t2
                     WHERE t2.session_id = s.id
                     ORDER BY t2.started_at DESC, t2.id DESC
                     LIMIT 1
               )
              WHERE s.is_cron = 0
                AND s.parent_session_id IS NULL
                {channel_clause}
                AND (
                    (s.kind = 'regular' AND t.ui_surface IN ('main_chat', 'quick_chat', 'pet_chat'))
                    OR (s.kind = 'knowledge' AND t.ui_surface IN ('knowledge_chat', 'pet_chat')
                        AND kt.kb_id IS NOT NULL)
                    OR (s.kind = 'design' AND t.ui_surface IN ('design_chat', 'pet_chat')
                        AND dt.project_id IS NOT NULL)
                    OR (s.kind = 'side' AND t.ui_surface IN ('side_chat', 'pet_chat')
                        AND s.archived_at IS NULL
                        AND EXISTS (
                            SELECT 1 FROM sessions side_source
                             WHERE side_source.id = s.forked_from_session_id
                               AND {side_source_scope}
                        ))
                )",
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let status: String = row.get(7)?;
            Ok(PetActivityRow {
                session_id: row.get(0)?,
                title: row.get(1)?,
                agent_id: row.get(2)?,
                project_id: row.get(3)?,
                incognito: row.get::<_, i64>(4)? != 0,
                kind: row.get(5)?,
                last_read_message_id: row.get(6)?,
                status: ChatTurnStatus::from_str(&status).unwrap_or(ChatTurnStatus::Interrupted),
                terminal_message_id: row.get(8)?,
                updated_at: row.get(9)?,
                preview: row.get(10)?,
                kb_id: row.get(11)?,
                anchor_note_path: row.get(12)?,
                design_project_id: row.get(13)?,
                side_source_session_id: row.get(14)?,
            })
        })?;
        let mut result = Vec::new();
        for row in rows {
            let mut row = row?;
            if row.incognito {
                // 无痕边界脱敏：原始 title/agent_id/preview 不出 kernel。
                row.title = String::new();
                row.agent_id = String::new();
                row.preview = None;
            }
            result.push(row);
        }
        Ok((result, ask_user))
    }
}

#[cfg(test)]
mod tests {
    use crate::session::SessionDB;

    fn ensure_channel_conversations_table(db: &SessionDB) {
        let conn = db.conn.lock().expect("lock database");
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

    #[test]
    fn a_new_non_ui_turn_displaces_the_previous_ui_turn() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let db = SessionDB::open(&temp.path().join("sessions.db")).expect("open session database");
        let session = db.create_session("ha-main").expect("create session");
        db.create_chat_turn_with_id_surface(
            "main-turn",
            &session.id,
            "http",
            None,
            None,
            Some(crate::pet::ChatUiSurface::MainChat),
        )
        .expect("create main UI turn");
        {
            let conn = db.conn.lock().expect("lock database");
            conn.execute(
                "UPDATE chat_turns SET started_at = '2026-01-01T00:00:00Z' WHERE id = 'main-turn'",
                [],
            )
            .expect("set first timestamp");
        }
        assert_eq!(db.pet_activity_rows().expect("query main turn").0.len(), 1);

        db.finish_chat_turn_once(
            "main-turn",
            crate::session::ChatTurnStatus::Completed,
            None,
            None,
            None,
        )
        .expect("finish main UI turn");

        db.create_chat_turn_with_id_surface("external-turn", &session.id, "http", None, None, None)
            .expect("create non-UI turn");
        {
            let conn = db.conn.lock().expect("lock database");
            conn.execute(
                "UPDATE chat_turns SET started_at = '2026-01-02T00:00:00Z' WHERE id = 'external-turn'",
                [],
            )
            .expect("set second timestamp");
        }
        assert!(db
            .pet_activity_rows()
            .expect("query displaced turn")
            .0
            .is_empty());
    }

    #[test]
    fn side_chat_ui_turn_is_projected_with_its_source_session() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let db = SessionDB::open(&temp.path().join("sessions.db")).expect("open session database");
        ensure_channel_conversations_table(&db);
        let source = db.create_session("ha-main").expect("create source session");
        let side = db.create_side_chat(&source.id).expect("create side chat");
        db.create_chat_turn_with_id_surface(
            "side-turn",
            &side.id,
            "http",
            None,
            None,
            Some(crate::pet::ChatUiSurface::SideChat),
        )
        .expect("create side UI turn");

        let rows = db.pet_activity_rows().expect("query side turn").0;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_id, side.id);
        assert_eq!(
            rows[0].side_source_session_id.as_deref(),
            Some(source.id.as_str())
        );
    }

    #[test]
    fn archived_side_owner_suppresses_running_and_late_completed_activity() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let db = SessionDB::open(&temp.path().join("sessions.db")).expect("open database");
        ensure_channel_conversations_table(&db);
        let source = db.create_session("ha-main").unwrap();
        let side = db.create_side_chat(&source.id).unwrap();
        db.create_chat_turn_with_id_surface(
            "side-turn",
            &side.id,
            "http",
            None,
            None,
            Some(crate::pet::ChatUiSurface::SideChat),
        )
        .unwrap();
        assert_eq!(db.pet_activity_rows().unwrap().0.len(), 1);

        let revision = crate::pet::activity_revision();
        db.set_session_archived(&source.id, true).unwrap();
        assert!(crate::pet::activity_revision() > revision);
        assert!(db.pet_activity_rows().unwrap().0.is_empty());
        let answer = db
            .append_message(
                &side.id,
                &crate::session::NewMessage::assistant("late answer"),
            )
            .unwrap();
        db.finish_chat_turn_once(
            "side-turn",
            crate::session::ChatTurnStatus::Completed,
            None,
            None,
            Some(answer),
        )
        .unwrap();
        assert!(db.pet_activity_rows().unwrap().0.is_empty());

        db.set_session_archived(&source.id, false).unwrap();
        let rows = db.pet_activity_rows().unwrap().0;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, crate::session::ChatTurnStatus::Completed);
        assert_eq!(
            rows[0].side_source_session_id.as_deref(),
            Some(source.id.as_str())
        );
        db.set_session_archived(&side.id, true).unwrap();
        assert!(db.pet_activity_rows().unwrap().0.is_empty());
        db.set_session_archived(&side.id, false).unwrap();
        assert_eq!(db.pet_activity_rows().unwrap().0.len(), 1);
    }
}
