use super::*;
use crate::chat_engine::ChatSource;
use crate::session::{ChatTurnStatus, NewMessage, SessionKind};
use std::sync::Arc;

fn isolated_test(test: impl FnOnce(Arc<SessionDB>, &std::path::Path)) {
    let directory = tempfile::tempdir().unwrap();
    crate::test_support::with_env_vars(&[("HA_DATA_DIR", directory.path())], || {
        let path = directory.path().join("sessions.db");
        let db = Arc::new(SessionDB::open(&path).unwrap());
        crate::channel::db::ChannelDB::new(db.clone())
            .migrate()
            .unwrap();
        test(db, &path);
    });
}

fn seed_target(db: &SessionDB, id: &str) {
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO sessions(id,agent_id,created_at,updated_at) VALUES(?1,'ha-main',?2,?2)",
        params![id, chrono::Utc::now().to_rfc3339()],
    )
    .unwrap();
}

fn seed_source(db: &SessionDB) -> SessionMeta {
    let home = db.ensure_personal_assistant_home().unwrap();
    let message_id = db
        .append_message(
            &home.id,
            &NewMessage::user("Ask the release conversation for its progress"),
        )
        .unwrap();
    db.create_chat_turn_with_id("owner-turn", &home.id, "desktop", None, Some(message_id))
        .unwrap();
    home
}

fn input(target: &str) -> AssistantDispatchInput {
    AssistantDispatchInput {
        source_turn_id: "owner-turn".into(),
        item_key: format!("send-{target}"),
        target_session_id: target.into(),
        instruction: "Report current progress".into(),
        global_stop_epoch: 0,
    }
}

fn admit(
    db: &SessionDB,
    home: &SessionMeta,
    request: &AssistantRequest,
    turn_id: &str,
) -> Result<()> {
    let mut message = NewMessage::user(&request.instruction).with_source(ChatSource::SessionTool);
    message.attachments_meta =
        Some(serde_json::json!({"personal_assistant_request_id":request.id}).to_string());
    db.append_message_and_create_session_tool_turn_with_id(
        turn_id,
        &request.target_session_id,
        Some(&home.id),
        &message,
        request.global_stop_epoch,
    )?;
    Ok(())
}

fn seed_goal(db: &SessionDB, target: &str, goal: &str, state: &str) {
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO goals(id,session_id,objective,state,created_at,updated_at)
         VALUES(?1,?2,'Release the update',?3,?4,?4)",
        params![goal, target, state, chrono::Utc::now().to_rfc3339()],
    )
    .unwrap();
}

#[test]
fn home_and_created_targets_are_atomic_across_database_handles() {
    isolated_test(|db, path| {
        let second = SessionDB::open(&path.to_path_buf()).unwrap();
        let home = seed_source(&db);
        assert_eq!(second.ensure_personal_assistant_home().unwrap().id, home.id);
        assert_eq!(home.kind, SessionKind::PersonalAssistant);
        assert!(db
            .list_sessions_for_model(None, false, 100)
            .unwrap()
            .is_empty());
        assert!(db.has_personal_assistant_private_data().unwrap());
        assert!(db.update_session_incognito(&home.id, true).is_err());
        let target = db
            .create_personal_assistant_target(
                "owner-turn",
                "create-1",
                "ha-main",
                None,
                Some("Release"),
                0,
            )
            .unwrap();
        let retried = second
            .create_personal_assistant_target(
                "owner-turn",
                "create-1",
                "ha-main",
                None,
                Some("Release"),
                0,
            )
            .unwrap();
        assert_eq!(target.id, retried.id);
        let error = second
            .create_personal_assistant_target(
                "owner-turn",
                "create-1",
                "ha-main",
                None,
                Some("Different"),
                0,
            )
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<AssistantError>(),
            Some(&AssistantError::Conflict)
        );
    });
}

#[test]
fn dispatch_is_idempotent_and_survives_completion_before_owner_projection() {
    isolated_test(|db, path| {
        let home = seed_source(&db);
        seed_target(&db, "target");
        let request = db
            .enqueue_personal_assistant_request(input("target"))
            .unwrap();
        assert_eq!(
            db.enqueue_personal_assistant_request(input("target"))
                .unwrap()
                .id,
            request.id
        );
        let mut changed = input("target");
        changed.instruction = "An unrelated instruction".into();
        assert_eq!(
            db.enqueue_personal_assistant_request(changed)
                .unwrap_err()
                .downcast_ref::<AssistantError>(),
            Some(&AssistantError::Conflict)
        );
        admit(&db, &home, &request, "target-turn").unwrap();
        assert!(admit(&db, &home, &request, "repeated-turn").is_err());
        assert!(db.is_personal_assistant_private_session("target").unwrap());
        assert!(!db
            .list_sessions_for_model(None, false, 100)
            .unwrap()
            .iter()
            .any(|s| s.id == "target"));
        assert!(db
            .list_sessions_for_personal_assistant(None, 100)
            .unwrap()
            .iter()
            .any(|s| s.id == "target"));
        let messages = db.load_session_messages("target").unwrap();
        assert_eq!(messages.len(), 1);
        let metadata = messages[0].attachments_meta.as_deref().unwrap();
        assert!(!metadata.contains(&home.id));
        assert!(!metadata.contains("session_message"));
        let reply = db
            .append_message("target", &NewMessage::assistant("The checks passed"))
            .unwrap();
        db.finish_chat_turn_once(
            "target-turn",
            ChatTurnStatus::Completed,
            None,
            None,
            Some(reply),
        )
        .unwrap();
        let restarted = SessionDB::open(&path.to_path_buf()).unwrap();
        let result = restarted
            .personal_assistant_request(&request.id)
            .unwrap()
            .unwrap();
        assert_eq!(result.status, "completed");
        assert_eq!(result.result_message_id, Some(reply));
        assert_eq!(result.result_preview.as_deref(), Some("The checks passed"));
        assert!(restarted
            .queued_personal_assistant_requests(10)
            .unwrap()
            .is_empty());
        assert!(restarted
            .render_personal_assistant_receipts()
            .unwrap()
            .contains(&request.id));
        restarted.reconcile_personal_assistant_requests().unwrap();
        let stored_status: String = restarted
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT status FROM personal_assistant_requests WHERE id=?1",
                [&request.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_status, "completed");
        assert!(restarted.fork_session("target", None).is_err());
        let conn = restarted.conn.lock().unwrap();
        assert!(conn.execute(
            "INSERT INTO channel_conversations(id,channel_id,account_id,chat_id,session_id,created_at,updated_at)
             VALUES('binding','telegram','bot','chat','target','now','now')",[],
        ).is_err());
    });
}

#[test]
fn cancelled_or_stopped_requests_cannot_be_claimed_later() {
    isolated_test(|db, _| {
        let home = seed_source(&db);
        seed_target(&db, "cancelled");
        seed_target(&db, "stopped");
        let cancelled = db
            .enqueue_personal_assistant_request(input("cancelled"))
            .unwrap();
        let outcome = db.cancel_personal_assistant_request(&cancelled.id).unwrap();
        assert_eq!(outcome.request.status, "cancelled");
        assert!(outcome.cancel_turn_id.is_none());
        assert!(admit(&db, &home, &cancelled, "never-created").is_err());
        assert!(db.get_chat_turn("never-created").unwrap().is_none());
        let stopped = db
            .enqueue_personal_assistant_request(input("stopped"))
            .unwrap();
        let pause = db.prepare_session_autonomy_pause("stopped").unwrap();
        db.finish_session_autonomy_resume(&pause.id).unwrap();
        let error = admit(&db, &home, &stopped, "stale-turn").unwrap_err();
        assert_eq!(
            error.downcast_ref::<AssistantError>(),
            Some(&AssistantError::Stopped)
        );
        assert!(db.load_session_messages("stopped").unwrap().is_empty());
        db.reconcile_personal_assistant_requests().unwrap();
        assert_eq!(
            db.personal_assistant_request(&stopped.id)
                .unwrap()
                .unwrap()
                .status,
            "cancelled"
        );
        assert!(db
            .queued_personal_assistant_requests(10)
            .unwrap()
            .is_empty());
    });
}

#[test]
fn crash_recovery_reports_interruption_and_never_creates_a_second_execution() {
    isolated_test(|db, path| {
        let home = seed_source(&db);
        seed_target(&db, "target");
        let request = db
            .enqueue_personal_assistant_request(input("target"))
            .unwrap();
        admit(&db, &home, &request, "crashed-turn").unwrap();
        let restarted = SessionDB::open(&path.to_path_buf()).unwrap();
        restarted.recover_stale_chat_turns().unwrap();
        let result = restarted
            .finish_personal_assistant_request(&request.id)
            .unwrap();
        assert_eq!(result.status, "interrupted");
        assert!(result.result_preview.is_none());
        assert!(restarted
            .queued_personal_assistant_requests(10)
            .unwrap()
            .is_empty());
        assert_eq!(restarted.load_session_messages("target").unwrap().len(), 1);
    });
}

#[test]
fn goal_watch_uses_opt_in_baseline_and_real_transitions_across_restart() {
    isolated_test(|db, path| {
        seed_target(&db, "target");
        seed_goal(&db, "target", "goal", "blocked");
        let watch = db.watch_personal_goal("target", "goal").unwrap();
        assert!(!watch.needs_attention);
        assert!(watch.event_id.is_none());
        let reopened = SessionDB::open(&path.to_path_buf()).unwrap();
        assert!(!reopened.personal_assistant_inbox().unwrap()[0].needs_attention);
        reopened
            .transition_goal("goal", crate::goal::GoalState::Active, None)
            .unwrap();
        reopened
            .transition_goal(
                "goal",
                crate::goal::GoalState::Blocked,
                Some("decision needed"),
            )
            .unwrap();
        let warning = reopened.personal_assistant_inbox().unwrap().remove(0);
        assert!(warning.needs_attention);
        assert_eq!(warning.reason.as_deref(), Some("blocked"));
        reopened
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE goals SET objective='Edited description' WHERE id='goal'",
                [],
            )
            .unwrap();
        assert_eq!(
            reopened.personal_assistant_inbox().unwrap()[0].event_id,
            warning.event_id
        );
        reopened
            .transition_goal("goal", crate::goal::GoalState::Active, None)
            .unwrap();
        assert!(!reopened.personal_assistant_inbox().unwrap()[0].needs_attention);
        reopened
            .transition_goal("goal", crate::goal::GoalState::Blocked, None)
            .unwrap();
        assert!(reopened.personal_assistant_inbox().unwrap()[0].event_id > warning.event_id);
        reopened.unwatch_personal_goal("target", "goal").unwrap();
        assert!(
            !reopened
                .watch_personal_goal("target", "goal")
                .unwrap()
                .needs_attention
        );
    });
}

#[test]
fn watch_and_queue_scope_revocation_cannot_reappear_after_privacy_is_reenabled() {
    isolated_test(|db, _| {
        seed_source(&db);
        seed_target(&db, "target");
        seed_target(&db, "other");
        seed_goal(&db, "target", "goal", "active");
        assert!(db.watch_personal_goal("other", "goal").is_err());
        db.watch_personal_goal("target", "goal").unwrap();
        let request = db
            .enqueue_personal_assistant_request(input("target"))
            .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("UPDATE sessions SET incognito=1 WHERE id='target'", [])
                .unwrap();
            conn.execute("UPDATE sessions SET incognito=0 WHERE id='target'", [])
                .unwrap();
        }
        assert!(db.personal_assistant_inbox().unwrap().is_empty());
        assert!(db
            .personal_assistant_request(&request.id)
            .unwrap()
            .is_none());
        assert!(db
            .queued_personal_assistant_requests(10)
            .unwrap()
            .is_empty());
        db.watch_personal_goal("target", "goal").unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("INSERT INTO session_import_sources(provider,source_id,session_id,content_hash,imported_at)
                VALUES('codex','foreign','target','hash','now')",[]).unwrap();
            conn.execute(
                "DELETE FROM session_import_sources WHERE session_id='target'",
                [],
            )
            .unwrap();
        }
        assert!(db.personal_assistant_inbox().unwrap().is_empty());
        assert!(db.watch_personal_goal("other", "goal").is_err());
    });
}

#[test]
fn dispatch_keeps_ordinary_fifo_and_propagates_private_scope_to_descendants() {
    isolated_test(|db, _| {
        let home = seed_source(&db);
        seed_target(&db, "target");
        seed_target(&db, "child");
        seed_target(&db, "grandchild");
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE sessions SET parent_session_id='target' WHERE id='child'",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE sessions SET parent_session_id='child' WHERE id='grandchild'",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO queued_turn_user_messages(request_id,session_id,message,attachments_json,options_json,source,mode,status,created_at,updated_at)
                 VALUES('earlier','target','Earlier owner instruction','[]','{}','desktop','queue','queued','now','now')", [],
            ).unwrap();
        }
        let request = db
            .enqueue_personal_assistant_request(input("target"))
            .unwrap();
        assert_eq!(
            admit(&db, &home, &request, "too-early")
                .unwrap_err()
                .downcast_ref::<AssistantError>(),
            Some(&AssistantError::Busy)
        );
        assert_eq!(
            db.personal_assistant_request(&request.id)
                .unwrap()
                .unwrap()
                .status,
            "queued"
        );
        assert!(!db.is_personal_assistant_private_session("target").unwrap());
        db.conn
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM queued_turn_user_messages WHERE request_id='earlier'",
                [],
            )
            .unwrap();
        admit(&db, &home, &request, "accepted").unwrap();
        assert!(db
            .is_personal_assistant_private_session("grandchild")
            .unwrap());
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET parent_session_id=NULL WHERE id='grandchild'",
                [],
            )
            .unwrap();
        assert!(db
            .is_personal_assistant_private_session("grandchild")
            .unwrap());
        seed_target(&db, "new-child");
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET parent_session_id='target' WHERE id='new-child'",
                [],
            )
            .unwrap();
        assert!(db
            .is_personal_assistant_private_session("new-child")
            .unwrap());
    });
}

#[test]
fn evaluation_source_cannot_lose_its_provenance_in_the_durable_queue() {
    isolated_test(|db, _| {
        let home = seed_source(&db);
        seed_target(&db, "target");
        let context = crate::eval_context::EvalRunContext {
            evidence_kind: "model_campaign".into(),
            campaign_id: "mcampaign_assistant".into(),
            plan_digest: "1".repeat(64),
            suite_id: "hope-core".into(),
            suite_version: "1.0.0".into(),
            suite_digest: "2".repeat(64),
            case_id: "HA-ASSISTANT-001".into(),
            case_digest: "3".repeat(64),
            trial_id: format!("mtrial_assistant_{}", uuid::Uuid::new_v4().simple()),
            trial_index: 0,
            arm: "control".into(),
            fault_profile: "clean".into(),
            orchestration_profile: None,
            trace_id: "trace_assistant".into(),
            root_span_id: "span_assistant".into(),
            model_role: "anchor".into(),
            seed: 1,
            source: "local_cli".into(),
            commit_sha: "4".repeat(40),
            dirty: true,
            app_version: "0.72.0".into(),
            required_signals: Vec::new(),
            faults: Vec::new(),
            budget: Default::default(),
        };
        let _guard = crate::eval_context::register_root_session(&home.id, context).unwrap();
        let error = db
            .enqueue_personal_assistant_request(input("target"))
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<AssistantError>(),
            Some(&AssistantError::InvalidInput)
        );
        assert!(db.list_personal_assistant_requests().unwrap().is_empty());
        assert!(db
            .create_personal_assistant_target("owner-turn", "eval-create", "ha-main", None, None, 0)
            .is_err());
    });
}

#[test]
fn home_edit_resend_rejects_without_changing_source_or_queued_dispatch() {
    isolated_test(|db, _| {
        let home = seed_source(&db);
        seed_target(&db, "target");
        let request = db
            .enqueue_personal_assistant_request(input("target"))
            .unwrap();
        db.finish_chat_turn_once("owner-turn", ChatTurnStatus::Completed, None, None, None)
            .unwrap();
        let original = db.load_session_messages(&home.id).unwrap();
        let original_receipts = db.render_personal_assistant_receipts().unwrap();

        for (source, queue_source) in [
            ("desktop", crate::session::QueuedTurnMessageSource::Desktop),
            ("http", crate::session::QueuedTurnMessageSource::Http),
        ] {
            let replacement_turn = format!("rewrite-{source}");
            let admission = db
                .reserve_direct_turn_admission(&home.id, &replacement_turn, queue_source, None)
                .unwrap()
                .expect("reserve direct admission");
            let error = db
                .replace_last_user_message_for_edit(
                    &home.id,
                    request.source_message_id,
                    &NewMessage::user("Do not do that work"),
                    &replacement_turn,
                    source,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap_err();
            assert!(error.to_string().contains("history cannot be rewritten"));
            assert!(db.release_direct_turn_admission(admission).unwrap());
            assert!(db.get_chat_turn(&replacement_turn).unwrap().is_none());
            // Rejected admission never deletes the source or invents a new
            // instruction. The explicit original delegation remains visible.
            let remaining = db.load_session_messages(&home.id).unwrap();
            assert_eq!(remaining.len(), original.len());
            assert_eq!(remaining[0].id, request.source_message_id);
            assert_eq!(remaining[0].content, original[0].content);
            assert_eq!(
                db.get_chat_turn("owner-turn").unwrap().unwrap().status,
                ChatTurnStatus::Completed
            );
            let queued = db.queued_personal_assistant_requests(10).unwrap();
            assert_eq!(queued.len(), 1);
            assert_eq!(queued[0].id, request.id);
            assert_eq!(queued[0].instruction, request.instruction);
            assert_eq!(
                db.render_personal_assistant_receipts().unwrap(),
                original_receipts
            );
        }
    });
}
