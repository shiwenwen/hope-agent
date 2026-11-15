use super::*;
use crate::personal_assistant::research::*;
use crate::personal_assistant::tests::{
    admit as admit_item, input as item_input, seed_source, seed_target,
};
use crate::personal_assistant_im::{
    IngressOutcome, PollLease, SessionGrant, TelegramInput, TelegramInputKind,
};
use crate::session::CreateStreamRun;
use chrono::Timelike;
use std::sync::Arc;
fn isolated(test: impl FnOnce(Arc<SessionDB>, &std::path::Path)) {
    let directory = tempfile::tempdir().unwrap();
    crate::test_support::with_env_vars(&[("HA_DATA_DIR", directory.path())], || {
        let db = Arc::new(SessionDB::open(&directory.path().join("sessions.db")).unwrap());
        crate::channel::db::ChannelDB::new(db.clone())
            .migrate()
            .unwrap();
        let workspace = directory.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO sessions(id,agent_id,title,working_dir,created_at,updated_at) VALUES('work','ha-main','Work',?1,'now','now')",[workspace.to_str().unwrap()]).unwrap();
        let settings = super::super::policy::AssistantPolicySettings {
            research_enabled: true,
            ..Default::default()
        };
        db.request_assistant_policy(0, settings).unwrap();
        // Unit fixture: no config or Primary process is published by this helper.
        db.conn.lock().unwrap().execute("UPDATE assistant_policy_gate SET applied_revision=requested_revision,applied_json=requested_json",[]).unwrap();
        test(db, &workspace);
    });
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn input(path: &str) -> AssistantSourceInput {
    AssistantSourceInput {
        request_key: uuid::Uuid::new_v4().to_string(),
        session_id: "work".into(),
        relative_path: path.into(),
        focus: "Watch for a meeting conflict and prepare two useful alternatives".into(),
        duration_days: 7,
    }
}
fn share(db: Arc<SessionDB>, root: &std::path::Path, path: &str) -> AssistantSource {
    std::fs::write(
        root.join(path),
        "Planning: Friday 09:00\nReview: Friday 10:00\n",
    )
    .unwrap();
    runtime().block_on(create_source(db, input(path))).unwrap()
}
fn claim(db: &SessionDB) -> PendingResearch {
    db.claim_assistant_research(chrono::Utc::now().timestamp())
        .unwrap()
        .unwrap()
}
fn admit(db: &SessionDB, pending: &PendingResearch) -> String {
    let proof = db.research_proof(&pending.run_id).unwrap();
    let execution = uuid::Uuid::new_v4().to_string();
    let registration = db
        .admit_research_stream(
            &proof,
            &crate::session::CreateStreamRun {
                run_id: execution.clone(),
                session_id: pending.session_id.clone(),
                source: "assistant_research".into(),
                stream_id: Some(uuid::Uuid::new_v4().to_string()),
                turn_id: None,
                provider_shape: None,
            },
        )
        .unwrap();
    assert!(!registration.1.is_empty());
    execution
}
fn finding(source: &AssistantSource) -> ResearchFindingInput {
    ResearchFindingInput {
        attention_until: None,
        topic_key: "meeting-conflict".into(),
        fact: "The supplied agenda has changed".into(),
        inference: "The meetings may overlap".into(),
        uncertainty: "Confirm travel time before choosing".into(),
        prepared: "Option A: move the review. Option B: join remotely.".into(),
        references: vec![SourceReference {
            source_id: source.id.clone(),
            revision: source.revision,
            version: source.version.clone().unwrap(),
            start_line: 1,
            end_line: 1,
        }],
    }
}
fn publish(db: &SessionDB, execution: &str, source: &AssistantSource) {
    db.report_assistant_research(execution, vec![finding(source)])
        .unwrap();
    assert!(
        db.list_assistant_findings(chrono::Utc::now().timestamp())
            .unwrap()
            .is_empty(),
        "a proposal is not a committed result"
    );
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE chat_stream_runs SET status='committed' WHERE run_id=?1",
            [execution],
        )
        .unwrap();
    db.settle_assistant_research(chrono::Utc::now().timestamp())
        .unwrap();
}

fn configure(db: &SessionDB, route: super::super::policy::AssistantNoticeRoute) {
    let current = db.assistant_policy_status().unwrap();
    let settings = super::super::policy::AssistantPolicySettings {
        research_enabled: true,
        notice_route: route,
        timezone: Some("UTC".into()),
        quiet_start_hour: (chrono::Utc::now().hour() + 2) % 24,
        quiet_end_hour: (chrono::Utc::now().hour() + 3) % 24,
        desktop_to_im_notices: true,
        ..Default::default()
    };
    db.request_assistant_policy(current.requested_revision, settings)
        .unwrap();
    db.conn.lock().unwrap().execute("UPDATE assistant_policy_gate SET applied_revision=requested_revision,applied_json=requested_json",[]).unwrap();
}
fn connected(db: &SessionDB, now: i64) -> PollLease {
    db.save_assistant_im_connection(100, "private_bot", &uuid::Uuid::new_v4().to_string(), now)
        .unwrap();
    let code = db.begin_assistant_im_pairing(now).unwrap();
    let lease = db.assistant_im_poll_lease(now).unwrap().unwrap();
    let event = |id, kind| TelegramInput {
        update_id: id,
        sent_at: Some(now),
        user_id: 42,
        chat_id: 42,
        kind,
    };
    db.receive_assistant_im_update(
        &lease,
        event(1, TelegramInputKind::PairCode(code.code)),
        now,
    )
    .unwrap();
    let activation = db
        .confirm_assistant_im_pairing(lease.generation, 42, 42, now)
        .unwrap();
    let token = activation
        .callback_data
        .strip_prefix(crate::personal_assistant_im::ACTIVATE_PREFIX)
        .unwrap()
        .to_owned();
    db.receive_assistant_im_update(&lease, event(2, TelegramInputKind::Activate(token)), now)
        .unwrap();
    db.set_assistant_im_notifications_paused(false).unwrap();
    db.assistant_im_poll_lease(now).unwrap().unwrap()
}
fn consent(db: &SessionDB, source: &AssistantSource, enabled: bool, lease: &PollLease, now: i64) {
    let (_, revision) =
        source_disclosure_status(&db.read_conn().unwrap(), &source.id, source.revision, now)
            .unwrap();
    db.set_assistant_source_im_disclosure(
        &source.id,
        source.revision,
        enabled,
        revision,
        &lease.connection_id,
        lease.generation,
        now,
    )
    .unwrap();
}
fn claim_notice(db: &SessionDB, route: &str, now: i64) -> Option<claims::Claimed> {
    db.reconcile_assistant_attention(now).unwrap();
    let mut conn = db.conn.lock().unwrap();
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let keys = claims::preflight_keys(&tx, route, now).unwrap();
    let result = claims::claim(
        &tx,
        &keys,
        route,
        None,
        &uuid::Uuid::new_v4().to_string(),
        now,
    )
    .unwrap();
    tx.commit().unwrap();
    result
}
fn source_finding(db: Arc<SessionDB>, root: &std::path::Path) -> AssistantSource {
    let source = share(db.clone(), root, "agenda.md");
    db.request_assistant_source_review(&source.id, &uuid::Uuid::new_v4().to_string(), now())
        .unwrap();
    let pending = claim(&db);
    let execution = admit(&db, &pending);
    publish(&db, &execution, &source);
    source
}
fn seed_item(db: &SessionDB) -> String {
    let home = seed_source(db);
    seed_target(db, "target");
    let item = db
        .enqueue_personal_assistant_request(item_input("target"))
        .unwrap();
    admit_item(db, &home, &item, "worker-turn").unwrap();
    db.create_assistant_work_stream(&CreateStreamRun {
        run_id: "worker-stream".into(),
        session_id: "target".into(),
        source: "session_tool".into(),
        stream_id: None,
        turn_id: Some("worker-turn".into()),
        provider_shape: None,
    })
    .unwrap();
    item.id
}
fn verified(db: &SessionDB) {
    db.report_assistant_item(
        "target",
        "worker-stream",
        "done",
        "Verified by readback",
        None,
    )
    .unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE chat_stream_runs SET status='committed' WHERE run_id='worker-stream'",
            [],
        )
        .unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE chat_turns SET status='completed' WHERE id='worker-turn'",
            [],
        )
        .unwrap();
    db.reconcile_personal_assistant_requests().unwrap();
    db.settle_assistant_execution("worker-stream").unwrap();
}

#[test]
fn source_disclosure_is_separate_default_off_and_actual_sent_text_stays_private() {
    isolated(|db, root| {
        configure(&db, super::super::policy::AssistantNoticeRoute::Im);
        let lease = connected(&db, now());
        let source = source_finding(db.clone(), root);
        assert!(!db.list_assistant_sources().unwrap()[0].im_summary_allowed);
        assert!(claim_notice(&db, "im", now() + 61).is_none());
        consent(&db, &source, true, &lease, now());
        let notice = claim_notice(&db, "im", now() + 61).unwrap();
        assert!(notice.body.contains("Option A"));
        assert!(notice.body.len() < 3500);
        assert!(db
            .assistant_attention_im_send_current(&notice.id, now() + 61)
            .unwrap());
        let conn = db.read_conn().unwrap();
        let copies: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE content LIKE '%Option A%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(copies, 0);
        let copies: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM assistant_im_deliveries WHERE text LIKE '%Option A%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(copies, 0);
        let copies: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM assistant_attention_payloads WHERE body LIKE '%Option A%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(copies, 1);
    });
}
#[test]
fn disclosure_revoke_and_reenable_never_revives_old_reply_or_cached_send() {
    isolated(|db, root| {
        configure(&db, super::super::policy::AssistantNoticeRoute::Im);
        let lease = connected(&db, now());
        let source = source_finding(db.clone(), root);
        consent(&db, &source, true, &lease, now());
        let notice = claim_notice(&db, "im", now() + 61).unwrap();
        db.finish_assistant_attention_im(&notice.id, Some(100), now() + 61)
            .unwrap();
        let event = TelegramInput {
            update_id: 3,
            sent_at: Some(now()),
            user_id: 42,
            chat_id: 42,
            kind: TelegramInputKind::Reply {
                message_id: 101,
                reply_to_message_id: 100,
                text: "Prepare a third option".into(),
            },
        };
        assert_eq!(
            db.receive_assistant_im_update(&lease, event, now())
                .unwrap(),
            IngressOutcome::Queued
        );
        let conn = db.read_conn().unwrap();
        let query: String = conn
            .query_row(
                "SELECT id FROM assistant_research_queries WHERE im_reply_id IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(replies::query_scope_current(&conn, &query, now()).unwrap());
        let body: String = conn
            .query_row(
                "SELECT prior_finding FROM assistant_research_queries WHERE id=?1",
                [&query],
                |row| row.get(0),
            )
            .unwrap();
        assert!(body.contains("Option A"));
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM assistant_im_commands", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        drop(conn);
        consent(&db, &source, false, &lease, now());
        consent(&db, &source, true, &lease, now());
        assert!(!replies::query_scope_current(&db.read_conn().unwrap(), &query, now()).unwrap());
        assert_eq!(
            db.read_conn()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM assistant_attention_payloads",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        assert!(
            claim_notice(&db, "im", now() + 62).is_none(),
            "no second disposition after re-enable"
        );
    });
}
#[test]
fn final_send_rechecks_source_disclosure_stop_policy_and_full_input_scope() {
    for boundary in ["disclosure", "source", "stop", "policy"] {
        isolated(|db, root| {
            configure(&db, super::super::policy::AssistantNoticeRoute::Im);
            let lease = connected(&db, now());
            let source = source_finding(db.clone(), root);
            consent(&db, &source, true, &lease, now());
            let notice = claim_notice(&db, "im", now() + 61).unwrap();
            assert!(db
                .assistant_attention_im_send_current(&notice.id, now() + 61)
                .unwrap());
            match boundary {
                "disclosure" => consent(&db, &source, false, &lease, now()),
                "source" => db.revoke_assistant_source(&source.id, now()).unwrap(),
                "stop" => {
                    let home = db.ensure_personal_assistant_home().unwrap();
                    db.prepare_session_autonomy_pause(&home.id).unwrap();
                }
                _ => {
                    let status = db.assistant_policy_status().unwrap();
                    db.request_assistant_policy(status.requested_revision, Default::default())
                        .unwrap();
                }
            }
            assert!(
                !db.assistant_attention_im_send_current(&notice.id, now() + 61)
                    .unwrap(),
                "{boundary}"
            );
            db.cancel_unsubmitted_assistant_attention_im(&notice.id, now() + 61)
                .unwrap();
            assert_eq!(
                db.read_conn()
                    .unwrap()
                    .query_row(
                        "SELECT COUNT(*) FROM assistant_im_notification_attempts",
                        [],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                1
            );
        });
    }
}
#[test]
fn stale_pairing_cannot_enable_source_disclosure_and_unknown_reply_cannot_enter_ordinary_history() {
    isolated(|db, root| {
        configure(&db, super::super::policy::AssistantNoticeRoute::Im);
        let lease = connected(&db, now());
        let source = source_finding(db.clone(), root);
        assert!(db
            .set_assistant_source_im_disclosure(
                &source.id,
                source.revision,
                true,
                0,
                "old-connection",
                lease.generation,
                now()
            )
            .is_err());
        let event = TelegramInput {
            update_id: 3,
            sent_at: Some(now()),
            user_id: 42,
            chat_id: 42,
            kind: TelegramInputKind::Reply {
                message_id: 101,
                reply_to_message_id: 999,
                text: "PRIVATE_FOLLOWUP".into(),
            },
        };
        assert_eq!(
            db.receive_assistant_im_update(&lease, event, now())
                .unwrap(),
            IngressOutcome::Ignored
        );
        assert_eq!(
            db.read_conn()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM assistant_im_commands", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            0
        );
    });
}
#[test]
fn task_coalescing_presence_viewed_suppression_and_two_clients_share_one_claim() {
    isolated(|db, _| {
        configure(&db, super::super::policy::AssistantNoticeRoute::Desktop);
        seed_item(&db);
        verified(&db);
        assert!(claim_notice(&db, "desktop", now()).is_none());
        let client = uuid::Uuid::new_v4().to_string();
        db.update_assistant_presence(&client, true, now() + 61)
            .unwrap();
        assert!(claim_notice(&db, "desktop", now() + 62).is_none());
        db.update_assistant_presence(&client, false, now() + 62)
            .unwrap();
        let notice = claim_notice(&db, "desktop", now() + 62).unwrap();
        assert!(notice.body.contains("核实"));
        let other = SessionDB::open(&root_path(&db)).unwrap();
        assert!(claim_notice(&other, "desktop", now() + 63).is_none());
        let count: i64 = db
            .read_conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM assistant_im_notification_attempts",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    });
}
fn root_path(_db: &SessionDB) -> std::path::PathBuf {
    crate::paths::root_dir().unwrap().join("sessions.db")
}
#[test]
fn an_opened_update_or_answered_question_is_not_notified() {
    for question in [false, true] {
        isolated(|db, _| {
            configure(&db, super::super::policy::AssistantNoticeRoute::Desktop);
            seed_item(&db);
            if question {
                let group = db
                    .create_assistant_question("worker-stream", "Which option?", vec![])
                    .unwrap();
                let target = group.owner_response.unwrap().assistant_item.unwrap();
                let answers:Vec<crate::ask_user::AskUserQuestionAnswer>=serde_json::from_value(serde_json::json!([{"questionId":"decision","selected":[],"customInput":"A","files":[]}])).unwrap();
                db.answer_assistant_question(&group.request_id, &target, &answers)
                    .unwrap();
            } else {
                verified(&db);
                let update = db.list_assistant_updates().unwrap().pop().unwrap();
                db.view_assistant_update(&update.id).unwrap();
            }
            assert!(claim_notice(&db, "desktop", now() + 61).is_none());
        });
    }
}
#[test]
fn home_background_guard_and_exact_ordinary_execution_do_not_mix() {
    isolated(|db, _| {
        configure(&db, super::super::policy::AssistantNoticeRoute::Desktop);
        seed_item(&db);
        // The target has A's persistent privacy marker, but a fresh ordinary
        // owner's turn is not assistant-owned execution.
        assert!(db.is_personal_assistant_private_session("target").unwrap());
        verified(&db);
        db.create_chat_turn_with_id("ordinary-turn", "target", "desktop", None, None)
            .unwrap();
        assert_eq!(
            db.assistant_notification_execution_ownership("ordinary-turn")
                .unwrap(),
            NotificationOwnership::Ordinary
        );
        assert_eq!(
            db.assistant_notification_execution_ownership("worker-stream")
                .unwrap(),
            NotificationOwnership::Assistant
        );
        assert_eq!(
            db.assistant_notification_execution_ownership("missing")
                .unwrap(),
            NotificationOwnership::Unknown
        );
        assert!(signals::private_source_with_conn(
            &db.read_conn().unwrap(),
            &db.ensure_personal_assistant_home().unwrap().id
        )
        .unwrap());
        assert!(!signals::private_source_with_conn(&db.read_conn().unwrap(), "target").unwrap());
    });
}

#[test]
fn a_late_mobile_answer_is_consumed_and_does_not_poison_the_next_update() {
    isolated(|db, _| {
        configure(&db, super::super::policy::AssistantNoticeRoute::Im);
        let lease = connected(&db, now());
        seed_item(&db);
        db.set_assistant_im_grant(
            SessionGrant {
                session_id: "target".into(),
                revision: 0,
                query_status: true,
                delegate: true,
                stop: true,
                completion_notice: true,
                expires_at: now() + 3600,
            },
            now(),
        )
        .unwrap();
        let question = db
            .create_assistant_question("worker-stream", "Choose one", vec![])
            .unwrap();
        let notice = claim_notice(&db, "im", now() + 1).unwrap();
        db.finish_assistant_attention_im(&notice.id, Some(100), now() + 1)
            .unwrap();
        let target = question.owner_response.unwrap().assistant_item.unwrap();
        let answers:Vec<crate::ask_user::AskUserQuestionAnswer>=serde_json::from_value(serde_json::json!([{"questionId":"decision","selected":[],"customInput":"Desktop answer","files":[]}])).unwrap();
        db.answer_assistant_question(&question.request_id, &target, &answers)
            .unwrap();
        let event = |id, kind| TelegramInput {
            update_id: id,
            sent_at: Some(now()),
            user_id: 42,
            chat_id: 42,
            kind,
        };
        assert_eq!(
            db.receive_assistant_im_update(
                &lease,
                event(
                    3,
                    TelegramInputKind::Reply {
                        message_id: 101,
                        reply_to_message_id: 100,
                        text: "Late mobile answer".into()
                    }
                ),
                now()
            )
            .unwrap(),
            IngressOutcome::Ignored
        );
        assert_eq!(
            db.receive_assistant_im_update(
                &lease,
                event(4, TelegramInputKind::Message("Fresh request".into())),
                now()
            )
            .unwrap(),
            IngressOutcome::Queued
        );
        assert_eq!(
            db.assistant_im_poll_lease(now()).unwrap().unwrap().cursor,
            5
        );
    });
}
#[test]
fn blank_oversize_or_over_capacity_source_replies_are_consumed_without_model_work() {
    for case in ["blank", "oversize", "full"] {
        isolated(|db, root| {
            configure(&db, super::super::policy::AssistantNoticeRoute::Im);
            let lease = connected(&db, now());
            let source = source_finding(db.clone(), root);
            consent(&db, &source, true, &lease, now());
            let notice = claim_notice(&db, "im", now() + 61).unwrap();
            db.finish_assistant_attention_im(&notice.id, Some(100), now() + 61)
                .unwrap();
            if case == "full" {
                for _ in 0..8 {
                    db.request_assistant_source_review(
                        &source.id,
                        &uuid::Uuid::new_v4().to_string(),
                        now(),
                    )
                    .unwrap();
                }
            }
            let text = match case {
                "blank" => "   ".into(),
                "oversize" => "x".repeat(8_001),
                _ => "Another option".into(),
            };
            let event = TelegramInput {
                update_id: 3,
                sent_at: Some(now()),
                user_id: 42,
                chat_id: 42,
                kind: TelegramInputKind::Reply {
                    message_id: 101,
                    reply_to_message_id: 100,
                    text,
                },
            };
            assert_eq!(
                db.receive_assistant_im_update(&lease, event, now())
                    .unwrap(),
                IngressOutcome::Ignored,
                "{case}"
            );
            assert_eq!(
                db.assistant_im_poll_lease(now()).unwrap().unwrap().cursor,
                4
            );
        });
    }
}

#[test]
fn multiple_sent_items_require_an_explicit_index_and_research_uses_only_that_snapshot() {
    isolated(|db, root| {
        configure(&db, super::super::policy::AssistantNoticeRoute::Im);
        let lease = connected(&db, now());
        let first = source_finding(db.clone(), root);
        consent(&db, &first, true, &lease, now());
        db.request_assistant_source_review(&first.id, &uuid::Uuid::new_v4().to_string(), now())
            .unwrap();
        let pending = claim(&db);
        let execution = admit(&db, &pending);
        let mut alternate = finding(&first);
        alternate.topic_key = "separate-topic".into();
        alternate.fact = "Another independent observation".into();
        db.report_assistant_research(&execution, vec![alternate])
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE chat_stream_runs SET status='committed' WHERE run_id=?1",
                [&execution],
            )
            .unwrap();
        db.settle_assistant_research(now()).unwrap();
        let notice = claim_notice(&db, "im", now() + 61).unwrap();
        assert!(notice.body.contains("2."));
        db.finish_assistant_attention_im(&notice.id, Some(100), now() + 61)
            .unwrap();
        let event = |id, text: &str| TelegramInput {
            update_id: id,
            sent_at: Some(now()),
            user_id: 42,
            chat_id: 42,
            kind: TelegramInputKind::Reply {
                message_id: 100 + id,
                reply_to_message_id: 100,
                text: text.into(),
            },
        };
        db.receive_assistant_im_update(&lease, event(3, "Tell me more"), now())
            .unwrap();
        assert_eq!(
            db.read_conn()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM assistant_research_queries WHERE im_reply_id IS NOT NULL",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        let status = db.assistant_policy_status().unwrap();
        let mut settings = status.settings;
        settings.timezone = Some("Asia/Shanghai".into());
        db.request_assistant_policy(status.requested_revision, settings)
            .unwrap();
        db.reconcile_assistant_attention(now()).unwrap();
        assert_eq!(
            db.read_conn()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM assistant_attention_candidates WHERE kind='reply_status'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1,
            "temporary policy publication must retain the requested clarification"
        );
        db.conn.lock().unwrap().execute("UPDATE assistant_policy_gate SET applied_revision=requested_revision,applied_json=requested_json",[]).unwrap();
        let clarification = claim_notice(&db, "im", now()).unwrap();
        db.finish_assistant_attention_im(&clarification.id, Some(200), now())
            .unwrap();
        let response = TelegramInput {
            update_id: 4,
            sent_at: Some(now()),
            user_id: 42,
            chat_id: 42,
            kind: TelegramInputKind::Reply {
                message_id: 201,
                reply_to_message_id: 200,
                text: "2: Prepare another option".into(),
            },
        };
        db.receive_assistant_im_update(&lease, response, now())
            .unwrap();
        let conn = db.read_conn().unwrap();
        let prior:String=conn.query_row("SELECT prior_finding FROM assistant_research_queries WHERE im_reply_id IS NOT NULL",[],|row|row.get(0)).unwrap();
        let sent: String = conn
            .query_row(
                "SELECT body FROM assistant_attention_payloads WHERE delivery_id=?1 AND ordinal=2",
                [notice.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(prior, sent);
    });
}
#[test]
fn full_input_disclosure_and_time_sensitive_deadlines_gate_interruption() {
    isolated(|db, root| {
        configure(&db, super::super::policy::AssistantNoticeRoute::Im);
        let lease = connected(&db, now());
        let first = share(db.clone(), root, "one.md");
        let second = share(db.clone(), root, "two.md");
        // Discovery consumes both inputs even when the reported citation only
        // mentions the first one. Both inputs need separate IM consent.
        std::fs::write(root.join("one.md"), "Planning 09:00\nReview 09:00\n").unwrap();
        runtime()
            .block_on(refresh_owner_sources(db.clone()))
            .unwrap();
        let first = db
            .list_assistant_sources()
            .unwrap()
            .into_iter()
            .find(|source| source.id == first.id)
            .unwrap();
        let pending = claim(&db);
        let execution = admit(&db, &pending);
        publish(&db, &execution, &first);
        consent(&db, &first, true, &lease, now());
        assert!(claim_notice(&db, "im", now() + 61).is_none());
        consent(&db, &second, true, &lease, now());
        assert!(claim_notice(&db, "im", now() + 61).is_some());
    });
    isolated(|db, root| {
        configure(&db, super::super::policy::AssistantNoticeRoute::Desktop);
        let source = share(db.clone(), root, "agenda.md");
        db.request_assistant_source_review(&source.id, &uuid::Uuid::new_v4().to_string(), now())
            .unwrap();
        let pending = claim(&db);
        let execution = admit(&db, &pending);
        let mut expired = finding(&source);
        expired.attention_until =
            Some((chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339());
        db.report_assistant_research(&execution, vec![expired])
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE chat_stream_runs SET status='committed' WHERE run_id=?1",
                [&execution],
            )
            .unwrap();
        db.settle_assistant_research(now()).unwrap();
        assert!(claim_notice(&db, "desktop", now() + 61).is_none());
    });
}
#[test]
fn home_cannot_start_background_work_even_through_legacy_or_bypass_paths() {
    isolated(|db, root| {
        let home = seed_source(&db);
        let output = root.join("should-not-exist");
        for args in [
            serde_json::json!({"command":"unused","run_in_background":true}),
            serde_json::json!({"command":"unused","background":true}),
            serde_json::json!({"command":"unused","yield_ms":1}),
        ] {
            let ctx = crate::tool_defs::ToolExecContext {
                session_id: Some(home.id.clone()),
                turn_id: Some("owner-turn".into()),
                session_db: Some(crate::tools::SessionDbHandle(db.clone())),
                bypass_async_dispatch: true,
                ..Default::default()
            };
            let result =
                runtime().block_on(crate::tools::execute_tool_with_context("exec", &args, &ctx));
            assert!(result.unwrap_err().to_string().contains("Hope background"));
            assert!(!output.exists());
        }
    });
}
#[test]
fn followup_requests_keep_permanent_ownership_after_content_is_removed() {
    isolated(|db, _| {
        seed_item(&db);
        db.report_assistant_item(
            "target",
            "worker-stream",
            "continue",
            "Partial checked",
            Some("Check the remainder"),
        )
        .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE chat_stream_runs SET status='committed' WHERE run_id='worker-stream'",
                [],
            )
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE chat_turns SET status='completed' WHERE id='worker-turn'",
                [],
            )
            .unwrap();
        db.reconcile_personal_assistant_requests().unwrap();
        db.settle_assistant_execution("worker-stream").unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute("UPDATE assistant_followups SET due_at=0", [])
            .unwrap();
        db.queue_assistant_followups().unwrap();
        let conn = db.read_conn().unwrap();
        let request: String = conn
            .query_row(
                "SELECT request_id FROM assistant_followups WHERE request_id IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_attention_owned_requests WHERE request_id=?1)",[&request],|row|row.get::<_,bool>(0)).unwrap());
        drop(conn);
        db.delete_session("target").unwrap();
        assert!(db.read_conn().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM assistant_attention_owned_requests WHERE request_id=?1)",[request],|row|row.get::<_,bool>(0)).unwrap());
    });
}

#[test]
fn more_than_two_hundred_stale_candidates_do_not_hide_a_new_pending_question() {
    isolated(|db, _| {
        configure(&db, super::super::policy::AssistantNoticeRoute::Desktop);
        seed_item(&db);
        db.create_assistant_question("worker-stream", "A fresh decision", vec![])
            .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            let home = db
                .read_conn()
                .unwrap()
                .query_row(
                    "SELECT id FROM sessions WHERE kind='personal_assistant'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap();
            for index in 0..201 {
                conn.execute("INSERT INTO assistant_attention_candidates(logical_key,kind,subject_id,subject_revision,home_session_id,global_stop_epoch,home_stop_epoch,created_at,ready_at,expires_at) VALUES(?1,'question','removed-update',1,?2,0,0,?3,?3,?4)",params![format!("stale:{index:03}"),home,now()-10,now()+3600]).unwrap();
            }
        }
        let notice = claim_notice(&db, "desktop", now() + 1)
            .expect("invalid old candidates cannot starve the fresh question");
        assert!(notice.body.contains("等你回答"));
    });
}

#[test]
fn upgrade_seeds_a_stable_boundary_without_requiring_another_policy_save() {
    isolated(|db, _| {
        configure(&db, super::super::policy::AssistantNoticeRoute::Desktop);
        db.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM assistant_attention_boundary", [])
            .unwrap();
        super::migrate(&db.conn.lock().unwrap()).unwrap();
        let initial: i64 = db
            .read_conn()
            .unwrap()
            .query_row(
                "SELECT eligible_after FROM assistant_attention_boundary WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(initial > 0 && initial <= now());
        seed_item(&db);
        verified(&db);
        assert!(
            claim_notice(&db, "desktop", now() + 61).is_some(),
            "an existing applied policy works without another save after upgrade"
        );
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE assistant_attention_boundary SET eligible_after=123",
                [],
            )
            .unwrap();
        super::migrate(&db.conn.lock().unwrap()).unwrap();
        assert_eq!(
            db.read_conn()
                .unwrap()
                .query_row(
                    "SELECT eligible_after FROM assistant_attention_boundary WHERE singleton=1",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            123
        );
    });
}

#[test]
fn resuming_im_notices_discards_old_proactive_results_without_affecting_desktop_or_direct_replies()
{
    isolated(|db, root| {
        configure(&db, super::super::policy::AssistantNoticeRoute::Im);
        let lease = connected(&db, now());
        let source = source_finding(db.clone(), root);
        consent(&db, &source, true, &lease, now());
        db.set_assistant_im_notifications_paused(true).unwrap();
        db.set_assistant_im_notifications_paused(false).unwrap();
        // A controlled clock advances the same transactional resume boundary.
        let resume_at = now() + 10;
        note_im_notice_resume(&db.conn.lock().unwrap(), resume_at).unwrap();
        assert!(
            claim_notice(&db, "im", now() + 61).is_none(),
            "paused-period results are not replayed"
        );
        db.set_assistant_im_notifications_paused(false).unwrap();
        assert_eq!(
            db.read_conn()
                .unwrap()
                .query_row(
                    "SELECT im_eligible_after FROM assistant_attention_boundary",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            resume_at
        );
    });
    isolated(|db, _| {
        configure(&db, super::super::policy::AssistantNoticeRoute::Desktop);
        connected(&db, now());
        seed_item(&db);
        verified(&db);
        note_im_notice_resume(&db.conn.lock().unwrap(), now() + 10).unwrap();
        assert!(claim_notice(&db, "desktop", now() + 61).is_some());
    });
    isolated(|db, root| {
        configure(&db, super::super::policy::AssistantNoticeRoute::Im);
        let lease = connected(&db, now());
        let source = source_finding(db.clone(), root);
        consent(&db, &source, true, &lease, now());
        let notice = claim_notice(&db, "im", now() + 61).unwrap();
        db.finish_assistant_attention_im(&notice.id, Some(100), now() + 61)
            .unwrap();
        db.set_assistant_im_notifications_paused(true).unwrap();
        let event = TelegramInput {
            update_id: 3,
            sent_at: Some(now()),
            user_id: 42,
            chat_id: 42,
            kind: TelegramInputKind::Reply {
                message_id: 101,
                reply_to_message_id: 100,
                text: "Prepare another option".into(),
            },
        };
        assert_eq!(
            db.receive_assistant_im_update(&lease, event, now())
                .unwrap(),
            IngressOutcome::Queued
        );
        assert!(
            db.claim_assistant_research(now()).unwrap().is_some(),
            "a direct source question keeps its authority while proactive notices are muted"
        );
    });
}
