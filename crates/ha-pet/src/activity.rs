use std::sync::Arc;

use anyhow::Result;

use ha_core::session::pet_activity::PetActivityRow;
use ha_core::session::{ChatTurnStatus, SessionDB};

use super::types::{
    PetActivity, PetActivitySnapshot, PetActivityStatus, PetActivityTitleKind, PetNavigationTarget,
};

const SNAPSHOT_LIMIT: usize = 50;

fn project_row(row: PetActivityRow, pending_count: i64) -> Option<PetActivity> {
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
        "side" => PetNavigationTarget::Side {
            session_id: row.session_id.clone(),
            source_session_id: row.side_source_session_id?,
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
    // kernel 已在查询边界脱敏 incognito 行（title/agent_id 空、preview
    // None）；此分支是第二道防线，且负责 title_kind 语义。
    let (title, title_kind, agent_id) = if row.incognito {
        (None, PetActivityTitleKind::Incognito, None)
    } else if row.title.trim().is_empty() {
        (None, PetActivityTitleKind::Untitled, Some(row.agent_id))
    } else {
        (
            Some(ha_core::truncate_utf8(row.title.trim(), 160).to_string()),
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
    (!trimmed.is_empty()).then(|| ha_core::truncate_utf8(trimmed, 240).to_string())
}

fn snapshot_revision(activities: &[PetActivity]) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&ha_core::pet::activity_revision().to_le_bytes());
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
        let before = ha_core::pet::activity_revision();
        let approvals = ha_core::tools::pending_approvals_per_session().await;
        let (rows, ask_user) = db.clone().run(|db| db.pet_activity_rows()).await?;
        let after = ha_core::pet::activity_revision();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(status: ChatTurnStatus, boundary: i64, read: i64) -> PetActivityRow {
        PetActivityRow {
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
            side_source_session_id: None,
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
    fn side_activity_reopens_the_source_session_panel() {
        let mut row = row(ChatTurnStatus::Running, 10, 0);
        row.kind = "side".to_string();
        row.side_source_session_id = Some("source-1".to_string());
        assert!(matches!(
            project_row(row, 0)
                .expect("project side activity")
                .target,
            PetNavigationTarget::Side {
                session_id,
                source_session_id
            } if session_id == "s" && source_session_id == "source-1"
        ));
    }

    #[test]
    fn failed_turn_uses_visible_error_event_as_unread_boundary() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let db = SessionDB::open(&temp.path().join("sessions.db")).expect("open session database");
        let session = db.create_session("ha-main").expect("create session");
        let user_id = db
            .append_message(&session.id, &ha_core::session::NewMessage::user("hello"))
            .expect("append user message");
        db.create_chat_turn_with_id_surface(
            "failed-turn",
            &session.id,
            "http",
            None,
            Some(user_id),
            Some(crate::ChatUiSurface::MainChat),
        )
        .expect("create failed turn");
        let error_id = db
            .append_message(
                &session.id,
                &ha_core::session::NewMessage::error_event("provider failed"),
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

        let rows = db.pet_activity_rows().expect("query failed turn").0;
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
