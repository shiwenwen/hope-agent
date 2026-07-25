use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Result;
use rusqlite::OptionalExtension;

use crate::session::{ChatTurnStatus, SessionDB};

use super::types::{
    PetActivity, PetActivitySnapshot, PetActivityStatus, PetActivityTitleKind, PetNavigationTarget,
};

const SNAPSHOT_LIMIT: usize = 50;
static ACTIVITY_REVISION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
struct ActivityRow {
    session_id: String,
    title: String,
    agent_id: String,
    project_id: Option<String>,
    incognito: bool,
    kind: String,
    last_read_message_id: i64,
    status: ChatTurnStatus,
    terminal_message_id: Option<i64>,
    updated_at: String,
    preview: Option<String>,
    kb_id: Option<String>,
    anchor_note_path: Option<String>,
    design_project_id: Option<String>,
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

fn query_rows(db: &SessionDB) -> Result<(Vec<ActivityRow>, HashMap<String, i64>)> {
    let ask_user = db.count_pending_ask_user_groups_per_session()?;
    let conn = db
        .conn
        .lock()
        .map_err(|error| anyhow::anyhow!("Lock error: {error}"))?;
    let knowledge_join = if table_exists(&conn, "knowledge_chat_threads")? {
        "LEFT JOIN knowledge_chat_threads kt ON kt.session_id = s.id"
    } else {
        "LEFT JOIN (SELECT NULL AS session_id, NULL AS kb_id, NULL AS anchor_note_path) kt ON 0"
    };
    let channel_clause = if table_exists(&conn, "channel_conversations")? {
        "AND NOT EXISTS (SELECT 1 FROM channel_conversations cc WHERE cc.session_id = s.id)"
    } else {
        ""
    };
    let sql = format!(
        "SELECT s.id,
                COALESCE(s.title, ''),
                s.agent_id,
                s.project_id,
                s.incognito,
                s.kind,
                COALESCE(s.last_read_message_id, 0),
                t.status,
                COALESCE(
                    t.assistant_message_id,
                    CASE WHEN t.status = 'failed' THEN (
                        SELECT MAX(m.id)
                          FROM messages m
                         WHERE m.session_id = s.id
                           AND m.id > COALESCE(t.user_message_id, 0)
                           AND m.id < COALESCE(
                               (
                                   SELECT MIN(t3.user_message_id)
                                     FROM chat_turns t3
                                    WHERE t3.session_id = t.session_id
                                      AND t3.user_message_id > t.user_message_id
                               ),
                               9223372036854775807
                           )
                    ) END,
                    t.user_message_id
                ),
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
                dt.project_id
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
            )",
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let status: String = row.get(7)?;
        Ok(ActivityRow {
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
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok((result, ask_user))
}

fn project_row(row: ActivityRow, pending_count: i64) -> Option<PetActivity> {
    let boundary = row.terminal_message_id;
    let unseen = boundary.is_some_and(|value| value > row.last_read_message_id);
    let status = if pending_count > 0
        && matches!(
            row.status,
            ChatTurnStatus::Running | ChatTurnStatus::Cancelling
        ) {
        PetActivityStatus::NeedsInput
    } else {
        match row.status {
            ChatTurnStatus::Running | ChatTurnStatus::Cancelling => PetActivityStatus::Running,
            ChatTurnStatus::Completed if unseen => PetActivityStatus::Ready,
            ChatTurnStatus::Failed if unseen => PetActivityStatus::Blocked,
            _ => return None,
        }
    };
    let target = match row.kind.as_str() {
        "regular" => PetNavigationTarget::Regular {
            session_id: row.session_id.clone(),
            project_id: row.project_id,
        },
        "knowledge" => PetNavigationTarget::Knowledge {
            session_id: row.session_id.clone(),
            kb_id: row.kb_id?,
            anchor_note_path: row.anchor_note_path,
        },
        "design" => PetNavigationTarget::Design {
            session_id: row.session_id.clone(),
            project_id: row.design_project_id?,
            artifact_id: None,
        },
        _ => return None,
    };
    let preview = if row.incognito || status != PetActivityStatus::Ready {
        None
    } else {
        row.preview.as_deref().and_then(single_line_preview)
    };
    let (title, title_kind, agent_id) = if row.incognito {
        (None, PetActivityTitleKind::Incognito, None)
    } else if row.title.trim().is_empty() {
        (None, PetActivityTitleKind::Untitled, Some(row.agent_id))
    } else {
        (
            Some(crate::truncate_utf8(row.title.trim(), 160).to_string()),
            PetActivityTitleKind::Session,
            Some(row.agent_id),
        )
    };
    Some(PetActivity {
        activity_id: row.session_id,
        status,
        title,
        title_kind,
        agent_id,
        updated_at: row.updated_at,
        boundary,
        preview,
        target,
    })
}

fn single_line_preview(value: &str) -> Option<String> {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    (!trimmed.is_empty()).then(|| crate::truncate_utf8(trimmed, 240).to_string())
}

fn snapshot_revision(activities: &[PetActivity]) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&ACTIVITY_REVISION.load(Ordering::Relaxed).to_le_bytes());
    for activity in activities {
        hasher.update(activity.activity_id.as_bytes());
        hasher.update(&[activity.status.priority()]);
        hasher.update(&[activity.title_kind as u8]);
        if let Some(title) = activity.title.as_deref() {
            hasher.update(title.as_bytes());
        }
        hasher.update(activity.updated_at.as_bytes());
        hasher.update(&activity.boundary.unwrap_or_default().to_le_bytes());
        if let Some(preview) = activity.preview.as_deref() {
            hasher.update(preview.as_bytes());
        }
    }
    u64::from_le_bytes(
        hasher.finalize().as_bytes()[..8]
            .try_into()
            .unwrap_or([0; 8]),
    )
}

pub async fn activity_snapshot(db: Arc<SessionDB>) -> Result<PetActivitySnapshot> {
    for attempt in 0..2 {
        let before = ACTIVITY_REVISION.load(Ordering::Acquire);
        let approvals = crate::tools::approval::pending_approvals_per_session().await;
        let (rows, ask_user) = db.clone().run(query_rows).await?;
        let after = ACTIVITY_REVISION.load(Ordering::Acquire);
        if before != after && attempt == 0 {
            continue;
        }
        let mut activities = rows
            .into_iter()
            .filter_map(|row| {
                let pending = approvals
                    .get(&row.session_id)
                    .map(|aggregate| aggregate.count)
                    .unwrap_or_default()
                    + ask_user.get(&row.session_id).copied().unwrap_or_default();
                project_row(row, pending)
            })
            .collect::<Vec<_>>();
        activities.sort_by(|left, right| {
            left.status
                .priority()
                .cmp(&right.status.priority())
                .then_with(|| right.updated_at.cmp(&left.updated_at))
                .then_with(|| left.activity_id.cmp(&right.activity_id))
        });
        let total = activities.len();
        let revision = snapshot_revision(&activities);
        let dominant = activities.first().map(|activity| activity.status);
        activities.truncate(SNAPSHOT_LIMIT);
        return Ok(PetActivitySnapshot {
            revision,
            generated_at: chrono::Utc::now().to_rfc3339(),
            stale: before != after,
            dominant,
            activities,
            total: total.min(u32::MAX as usize) as u32,
            truncated: total > SNAPSHOT_LIMIT,
        });
    }
    unreachable!("pet activity snapshot retry loop returns on the second pass")
}

pub fn emit_activity_changed() {
    let revision = ACTIVITY_REVISION.fetch_add(1, Ordering::Relaxed) + 1;
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            "pet:activity_changed",
            serde_json::json!({ "revision": revision }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(status: ChatTurnStatus, boundary: i64, read: i64) -> ActivityRow {
        ActivityRow {
            session_id: "s".to_string(),
            title: "title".to_string(),
            agent_id: "ha-main".to_string(),
            project_id: None,
            incognito: false,
            kind: "regular".to_string(),
            last_read_message_id: read,
            status,
            terminal_message_id: Some(boundary),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            preview: Some("Done\nwith   details".to_string()),
            kb_id: None,
            anchor_note_path: None,
            design_project_id: None,
        }
    }

    #[test]
    fn terminal_projection_requires_unseen_boundary() {
        assert_eq!(
            project_row(row(ChatTurnStatus::Completed, 10, 9), 0)
                .unwrap()
                .status,
            PetActivityStatus::Ready
        );
        assert!(project_row(row(ChatTurnStatus::Completed, 10, 10), 0).is_none());
        assert_eq!(
            project_row(row(ChatTurnStatus::Failed, 10, 9), 0)
                .unwrap()
                .status,
            PetActivityStatus::Blocked
        );
        assert!(project_row(row(ChatTurnStatus::Interrupted, 10, 0), 0).is_none());
    }

    #[test]
    fn needs_input_beats_running() {
        assert_eq!(
            project_row(row(ChatTurnStatus::Running, 10, 0), 1)
                .unwrap()
                .status,
            PetActivityStatus::NeedsInput
        );
    }

    #[test]
    fn title_changes_advance_the_snapshot_revision() {
        let first =
            project_row(row(ChatTurnStatus::Running, 10, 0), 0).expect("project first activity");
        let mut renamed = first.clone();
        renamed.title = Some("a concise generated title".to_string());

        assert_ne!(snapshot_revision(&[first]), snapshot_revision(&[renamed]));
    }

    #[test]
    fn ready_preview_is_bounded_and_collapsed() {
        let activity =
            project_row(row(ChatTurnStatus::Completed, 10, 9), 0).expect("ready activity");
        assert_eq!(activity.preview.as_deref(), Some("Done with details"));

        let mut multibyte = row(ChatTurnStatus::Completed, 10, 9);
        multibyte.preview = Some(format!("{}\n tail", "中".repeat(100)));
        let preview = project_row(multibyte, 0)
            .and_then(|activity| activity.preview)
            .expect("bounded multibyte preview");
        assert!(preview.len() <= 240);
        assert!(!preview.contains('\n'));
    }

    #[test]
    fn knowledge_activity_does_not_require_a_note_anchor() {
        let mut row = row(ChatTurnStatus::Running, 10, 0);
        row.kind = "knowledge".to_string();
        row.kb_id = Some("kb-1".to_string());
        row.anchor_note_path = None;
        assert!(matches!(
            project_row(row, 0)
                .expect("project knowledge activity")
                .target,
            PetNavigationTarget::Knowledge {
                anchor_note_path: None,
                ..
            }
        ));
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
        assert_eq!(query_rows(&db).expect("query main turn").0.len(), 1);

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
        assert!(query_rows(&db).expect("query displaced turn").0.is_empty());
    }

    #[test]
    fn failed_turn_uses_visible_error_event_as_unread_boundary() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let db = SessionDB::open(&temp.path().join("sessions.db")).expect("open session database");
        let session = db.create_session("ha-main").expect("create session");
        let user_id = db
            .append_message(&session.id, &crate::session::NewMessage::user("hello"))
            .expect("append user message");
        db.create_chat_turn_with_id_surface(
            "failed-turn",
            &session.id,
            "http",
            None,
            Some(user_id),
            Some(crate::pet::ChatUiSurface::MainChat),
        )
        .expect("create failed turn");
        let error_id = db
            .append_message(
                &session.id,
                &crate::session::NewMessage::error_event("provider failed"),
            )
            .expect("append visible failure event");
        db.finish_chat_turn_once(
            "failed-turn",
            ChatTurnStatus::Failed,
            None,
            Some("provider failed"),
            None,
        )
        .expect("finish failed turn");
        db.mark_session_read_through(&session.id, Some(user_id))
            .expect("mark only the user row read");

        let rows = query_rows(&db).expect("query failed turn").0;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].terminal_message_id, Some(error_id));
        assert_eq!(
            project_row(rows.into_iter().next().expect("activity row"), 0)
                .expect("blocked activity")
                .status,
            PetActivityStatus::Blocked
        );
    }
}
