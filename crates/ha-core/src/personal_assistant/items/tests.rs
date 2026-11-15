use super::*;
use crate::personal_assistant::tests::{admit, input, isolated_test, seed_source, seed_target};
use crate::session::{CreateStreamRun, NewMessage};

fn seed_execution(db: &SessionDB) -> String {
    let home = seed_source(db);
    seed_target(db, "work");
    let request = db
        .enqueue_personal_assistant_request(input("work"))
        .unwrap();
    admit(db, &home, &request, "worker-turn").unwrap();
    db.create_assistant_work_stream(&CreateStreamRun {
        run_id: "worker-stream".into(),
        session_id: "work".into(),
        source: "session_tool".into(),
        stream_id: Some("stream".into()),
        turn_id: Some("worker-turn".into()),
        provider_shape: None,
    })
    .unwrap();
    request.id
}

fn terminal(db: &SessionDB, status: &str) {
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE chat_stream_runs SET status=?1 WHERE run_id='worker-stream'",
            [status],
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
}

#[test]
fn a_task_admits_atomically_without_goal_and_end_of_turn_is_not_delivery() {
    isolated_test(|db, _| {
        let item = seed_execution(&db);
        assert_eq!(
            db.assistant_item_for_turn("worker-turn")
                .unwrap()
                .as_deref(),
            Some(item.as_str())
        );
        let goals: i64 = db
            .read_conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM goals", [], |r| r.get(0))
            .unwrap();
        assert_eq!(goals, 0);
        terminal(&db, "committed");
        db.settle_assistant_execution("worker-stream").unwrap();
        assert!(matches!(
            db.list_assistant_items().unwrap()[0].state,
            ItemState::Waiting
        ));
        assert_eq!(db.list_assistant_updates().unwrap()[0].kind, "needs_review");
    });
}

#[test]
fn verified_report_settles_once_and_unrelated_session_work_does_not_block() {
    isolated_test(|db, _| {
        seed_execution(&db);
        db.report_assistant_item(
            "work",
            "worker-stream",
            "done",
            "Read back the requested output: verified.",
            None,
        )
        .unwrap();
        assert_eq!(
            db.assistant_pending_work("worker-stream").unwrap(),
            AssistantPendingWork::Ready
        );
        terminal(&db, "committed");
        db.settle_assistant_execution("worker-stream").unwrap();
        db.settle_assistant_execution("worker-stream").unwrap();
        assert!(matches!(
            db.list_assistant_items().unwrap()[0].state,
            ItemState::Done
        ));
        assert_eq!(db.list_assistant_updates().unwrap().len(), 1);
    });
}

#[test]
fn partial_report_enqueues_one_continuation_under_original_item() {
    isolated_test(|db, _| {
        let item = seed_execution(&db);
        db.report_assistant_item(
            "work",
            "worker-stream",
            "continue",
            "First half checked",
            Some("Read back the second half"),
        )
        .unwrap();
        terminal(&db, "committed");
        db.settle_assistant_execution("worker-stream").unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute("UPDATE assistant_followups SET due_at=0", [])
            .unwrap();
        assert_eq!(db.queue_assistant_followups().unwrap(), 1);
        assert_eq!(db.queue_assistant_followups().unwrap(), 0);
        assert_eq!(db.list_assistant_items().unwrap().len(), 1);
        assert_eq!(db.list_assistant_items().unwrap()[0].id, item);
    });
}

#[test]
fn durable_answer_after_restart_is_unique_and_never_reopens_question_execution() {
    isolated_test(|db, path| {
        let item = seed_execution(&db);
        let question = db
            .create_assistant_question("worker-stream", "Which output do you want?", vec![])
            .unwrap();
        let target = question.owner_response.unwrap().assistant_item.unwrap();
        let answer = crate::ask_user::AskUserQuestionAnswer {
            question_id: "decision".into(),
            selected: vec![],
            custom_input: Some("A short report".into()),
            files: vec![],
        };
        let reopened = SessionDB::open(&path.to_path_buf()).unwrap();
        reopened
            .answer_assistant_question(&question.request_id, &target, std::slice::from_ref(&answer))
            .unwrap();
        assert!(reopened
            .assistant_execution_waiting_for_owner("worker-stream")
            .unwrap());
        assert_eq!(
            reopened.queue_assistant_followups().unwrap(),
            0,
            "wait until old execution ends"
        );
        assert!(reopened
            .answer_assistant_question(&question.request_id, &target, &[answer])
            .is_err());
        terminal(&reopened, "committed");
        assert_eq!(reopened.queue_assistant_followups().unwrap(), 1);
        assert_eq!(reopened.list_assistant_items().unwrap()[0].id, item);
    });
}

#[test]
fn stop_invalidates_pending_answer_and_all_queued_continuations() {
    isolated_test(|db, _| {
        let item = seed_execution(&db);
        let question = db
            .create_assistant_question("worker-stream", "Continue?", vec![])
            .unwrap();
        let target = question.owner_response.unwrap().assistant_item.unwrap();
        db.stop_assistant_item(&item).unwrap();
        let answer = crate::ask_user::AskUserQuestionAnswer {
            question_id: "decision".into(),
            selected: vec![],
            custom_input: Some("yes".into()),
            files: vec![],
        };
        assert!(db
            .answer_assistant_question(&question.request_id, &target, &[answer])
            .is_err());
        assert!(db
            .report_assistant_item("work", "worker-stream", "done", "late result", None)
            .is_err());
        assert_eq!(db.queue_assistant_followups().unwrap(), 0);
    });
}

#[test]
fn deleting_content_does_not_refund_owner_turns_or_unknown_time() {
    isolated_test(|db, _| {
        let item = seed_execution(&db);
        let now = chrono::Utc::now().timestamp();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("INSERT INTO assistant_budget_reservations(token,item_id,root_turn_id,execution_id,
                kind,started_at,reserved_seconds) VALUES('unknown',?1,'worker-turn','worker-stream','tool',?2,900)",params![item,now]).unwrap();
            conn.execute("DELETE FROM sessions WHERE id='work'", [])
                .unwrap();
        }
        let conn = db.read_conn().unwrap();
        assert_eq!(owner_turns_with_conn(&conn, now).unwrap(), 1);
        let unknown: i64 = conn
            .query_row(
                "SELECT SUM(reserved_seconds) FROM assistant_budget_reservations
            WHERE settled_at IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(unknown, 900);
    });
}

#[test]
fn a_completion_claim_can_move_after_pure_cancel_but_not_after_unknown_action() {
    isolated_test(|db, _| {
        let item = seed_execution(&db);
        db.bind_assistant_work_source("job", "job-id", "work", Some("worker-stream"))
            .unwrap();
        let proof = provenance::test_proof(item.clone());
        terminal(&db, "committed");
        let make = |id: &str| CreateStreamRun {
            run_id: id.into(),
            session_id: "work".into(),
            source: "parent_injection".into(),
            stream_id: Some(format!("s-{id}")),
            turn_id: None,
            provider_shape: None,
        };
        db.create_assistant_injection_stream(&make("followup1"), &proof)
            .unwrap();
        assert!(db
            .create_assistant_injection_stream(&make("duplicate-live"), &proof)
            .is_err());
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE chat_stream_runs SET status='interrupted' WHERE run_id='followup1'",
                [],
            )
            .unwrap();
        db.create_assistant_injection_stream(&make("followup2"), &proof)
            .unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE chat_stream_runs SET status='interrupted' WHERE run_id='followup2'",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO assistant_budget_reservations(token,item_id,root_turn_id,execution_id,kind,
            started_at,reserved_seconds) VALUES('uncertain',?1,'worker-turn','followup2','tool',?2,1)",
            params![item,chrono::Utc::now().timestamp()]).unwrap();
        drop(conn);
        assert!(db
            .create_assistant_injection_stream(&make("unsafe-replay"), &proof)
            .is_err());
    });
}

#[test]
fn invalid_worker_identity_cannot_report_or_create_child_work() {
    isolated_test(|db, _| {
        seed_execution(&db);
        assert!(db
            .report_assistant_item("other", "worker-stream", "done", "forged", None)
            .is_err());
        assert!(db
            .bind_assistant_work_source("job", "escape", "work", None)
            .is_err());
        let message = NewMessage::assistant("ordinary");
        assert!(db.append_message("work", &message).is_ok());
    });
}

#[test]
fn inline_completion_settles_preallocated_source_without_a_job_row() {
    isolated_test(|db, _| {
        seed_execution(&db);
        db.bind_assistant_work_source("job", "inline-id", "work", Some("worker-stream"))
            .unwrap();
        db.settle_assistant_inline_source("inline-id", "worker-stream")
            .unwrap();
        terminal(&db, "committed");
        assert_eq!(
            db.assistant_pending_work("worker-stream").unwrap(),
            AssistantPendingWork::Ready
        );
    });
}

#[test]
fn adding_allowance_does_not_duplicate_a_child_continuation() {
    isolated_test(|db, _| {
        let item = seed_execution(&db);
        db.bind_assistant_work_source("job", "pending-child", "work", Some("worker-stream"))
            .unwrap();
        terminal(&db, "committed");
        db.conn.lock().unwrap().execute("UPDATE assistant_items SET state='waiting',reason='work_state_unknown' WHERE id=?1",[&item]).unwrap();
        db.continue_assistant_item(&item).unwrap();
        let conn = db.read_conn().unwrap();
        let followups: i64 = conn
            .query_row("SELECT COUNT(*) FROM assistant_followups", [], |r| r.get(0))
            .unwrap();
        assert_eq!(followups, 0);
        let turns: i64 = conn
            .query_row(
                "SELECT turn_limit FROM assistant_items WHERE id=?1",
                [&item],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(turns, 16);
    });
}

#[test]
fn nested_child_injection_never_acquires_whole_item_reporting_rights() {
    isolated_test(|db, _| {
        let item = seed_execution(&db);
        seed_target(&db, "child");
        db.create_stream_run(&CreateStreamRun {
            run_id: "child-execution".into(),
            session_id: "child".into(),
            source: "subagent".into(),
            stream_id: Some("child-stream".into()),
            turn_id: None,
            provider_shape: None,
        })
        .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("INSERT INTO assistant_execution_contexts(execution_id,root_turn_id,item_id,item_revision,is_subtask)
                VALUES('child-execution','worker-turn',?1,1,1)",[&item]).unwrap();
            conn.execute("INSERT INTO subagent_runs(run_id,parent_session_id,parent_agent_id,child_agent_id,
                child_session_id,task,status,started_at,owner_kind,owner_id,delivery_kind)
                VALUES('grandchild','child','ha-main','ha-main','grandchild-session','read data','completed',?1,
                    'parent_session','child','parent')",[chrono::Utc::now().to_rfc3339()]).unwrap();
        }
        db.bind_assistant_work_source("subagent", "grandchild", "child", Some("child-execution"))
            .unwrap();
        db.create_assistant_question("worker-stream", "Which output?", vec![])
            .unwrap();
        let proof = db
            .assistant_injection_proof("child", &[("subagent".into(), "grandchild".into())])
            .unwrap()
            .unwrap();
        assert!(proof.matches_source(crate::chat_engine::ChatSource::ParentInjection));
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE chat_stream_runs SET status='committed' WHERE run_id='child-execution'",
                [],
            )
            .unwrap();
        db.create_assistant_injection_stream(
            &CreateStreamRun {
                run_id: "nested-injection".into(),
                session_id: "child".into(),
                source: "parent_injection".into(),
                stream_id: Some("nested-stream".into()),
                turn_id: None,
                provider_shape: None,
            },
            &proof,
        )
        .unwrap();
        assert_eq!(
            db.assistant_execution_role("nested-injection").unwrap(),
            Some(true)
        );
        assert_eq!(
            db.list_assistant_items().unwrap()[0].reason.as_deref(),
            Some("needs_input")
        );

        assert!(db
            .report_assistant_item("child", "nested-injection", "done", "all done", None)
            .is_err());
        assert!(db
            .create_assistant_question("nested-injection", "approve everything?", vec![])
            .is_err());
    });
}

#[test]
fn derived_turn_usage_blocks_a_new_root_and_explains_the_wait() {
    isolated_test(|db, _| {
        seed_execution(&db);
        seed_target(&db, "second");
        {
            let conn = db.conn.lock().unwrap();
            for n in 0..15 {
                conn.execute(
                    "INSERT INTO assistant_owner_turns(execution_id,started_at) VALUES(?1,?2)",
                    params![format!("derived-{n}"), chrono::Utc::now().timestamp()],
                )
                .unwrap();
            }
        }
        let home = db.ensure_personal_assistant_home().unwrap();
        let request = db
            .enqueue_personal_assistant_request(input("second"))
            .unwrap();
        assert!(admit(&db, &home, &request, "second-turn").is_err());
        db.fail_personal_assistant_request(&request.id, "not persisted raw")
            .unwrap();
        let item = db
            .list_assistant_items()
            .unwrap()
            .into_iter()
            .find(|i| i.id == request.id)
            .unwrap();
        assert!(matches!(item.state, ItemState::Waiting));
        assert_eq!(item.reason.as_deref(), Some("budget_exhausted"));
    });
}

#[test]
fn owner_supplement_reuses_item_and_waits_for_the_existing_execution() {
    isolated_test(|db, _| {
        let item = seed_execution(&db);
        let event = db
            .request_assistant_item_followup(
                &item,
                "owner-turn",
                "more-info",
                "work",
                "Use the updated outline",
                0,
            )
            .unwrap();
        assert_eq!(
            db.request_assistant_item_followup(
                &item,
                "owner-turn",
                "more-info",
                "work",
                "Use the updated outline",
                0
            )
            .unwrap(),
            event
        );
        assert_eq!(db.queue_assistant_followups().unwrap(), 0);
        db.report_assistant_item(
            "work",
            "worker-stream",
            "done",
            "Original output verified",
            None,
        )
        .unwrap();
        terminal(&db, "committed");
        db.settle_assistant_execution("worker-stream").unwrap();
        assert!(matches!(
            db.list_assistant_items().unwrap()[0].state,
            ItemState::Open
        ));
        assert_eq!(db.queue_assistant_followups().unwrap(), 1);
        assert_eq!(db.list_assistant_items().unwrap().len(), 1);
    });
}

fn seed_child_result(db: &SessionDB, id: &str, delivery: &str) {
    db.conn.lock().unwrap().execute("INSERT INTO subagent_runs(run_id,parent_session_id,parent_agent_id,
        child_agent_id,child_session_id,task,status,started_at,owner_kind,owner_id,delivery_kind)
        VALUES(?1,'work','ha-main','ha-main',?2,'read data','completed',?3,'parent_session','work',?4)",
        params![id,format!("session-{id}"),chrono::Utc::now().to_rfc3339(),delivery]).unwrap();
    db.bind_assistant_work_source("subagent", id, "work", Some("worker-stream"))
        .unwrap();
}

#[test]
fn terminal_group_children_do_not_block_the_group_consumers_next_step() {
    isolated_test(|db, _| {
        seed_execution(&db);
        seed_child_result(&db, "group-child", "group");
        db.bind_assistant_work_source("job", "group-job", "work", Some("worker-stream"))
            .unwrap();
        db.settle_assistant_inline_source("group-job", "worker-stream")
            .unwrap();
        db.report_assistant_item(
            "work",
            "worker-stream",
            "continue",
            "Group output checked",
            Some("Finish the output"),
        )
        .unwrap();
        terminal(&db, "committed");
        assert_eq!(
            db.assistant_pending_work("worker-stream").unwrap(),
            AssistantPendingWork::Ready
        );
        db.settle_assistant_execution("worker-stream").unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute("UPDATE assistant_followups SET due_at=0", [])
            .unwrap();
        assert_eq!(db.queue_assistant_followups().unwrap(), 1);
    });
}

#[test]
fn an_owner_answer_moves_to_the_child_consumer_without_a_duplicate_root() {
    isolated_test(|db, _| {
        seed_execution(&db);
        seed_child_result(&db, "result", "parent");
        let question = db
            .create_assistant_question("worker-stream", "Which format?", vec![])
            .unwrap();
        assert!(db
            .assistant_injection_proof("work", &[("subagent".into(), "result".into())])
            .is_err());
        let answer = crate::ask_user::AskUserQuestionAnswer {
            question_id: "decision".into(),
            selected: vec![],
            custom_input: Some("Use Markdown".into()),
            files: vec![],
        };
        db.answer_assistant_question(
            &question.request_id,
            &question.owner_response.unwrap().assistant_item.unwrap(),
            &[answer],
        )
        .unwrap();
        terminal(&db, "committed");
        assert_eq!(db.queue_assistant_followups().unwrap(), 0);
        let proof = db
            .assistant_injection_proof("work", &[("subagent".into(), "result".into())])
            .unwrap()
            .unwrap();
        db.create_assistant_injection_stream(
            &CreateStreamRun {
                run_id: "owner-answer-consumer".into(),
                session_id: "work".into(),
                source: "parent_injection".into(),
                stream_id: Some("answer-stream".into()),
                turn_id: None,
                provider_shape: None,
            },
            &proof,
        )
        .unwrap();
        assert!(db
            .assistant_execution_owner_events("owner-answer-consumer")
            .unwrap()
            .contains("Use Markdown"));
        assert_eq!(db.queue_assistant_followups().unwrap(), 0);
        db.report_assistant_item(
            "work",
            "owner-answer-consumer",
            "done",
            "Read back the Markdown result",
            None,
        )
        .unwrap();
        db.conn.lock().unwrap().execute("UPDATE chat_stream_runs SET status='committed' WHERE run_id='owner-answer-consumer'",[]).unwrap();
        db.settle_assistant_execution("owner-answer-consumer")
            .unwrap();
        assert!(matches!(
            db.list_assistant_items().unwrap()[0].state,
            ItemState::Done
        ));
        assert_eq!(db.queue_assistant_followups().unwrap(), 0);
    });
}

#[test]
fn an_aborted_claim_is_still_pending_and_inline_reads_are_exactly_bound() {
    isolated_test(|db, _| {
        seed_execution(&db);
        seed_child_result(&db, "result", "parent");
        terminal(&db, "committed");
        let proof = db
            .assistant_injection_proof("work", &[("subagent".into(), "result".into())])
            .unwrap()
            .unwrap();
        db.create_assistant_injection_stream(
            &CreateStreamRun {
                run_id: "aborted-claim".into(),
                session_id: "work".into(),
                source: "parent_injection".into(),
                stream_id: Some("aborted-stream".into()),
                turn_id: None,
                provider_shape: None,
            },
            &proof,
        )
        .unwrap();
        assert!(db
            .consume_subagent_result_in_execution("result", None)
            .is_err());
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE chat_stream_runs SET status='interrupted' WHERE run_id='aborted-claim'",
                [],
            )
            .unwrap();
        assert_eq!(
            db.assistant_pending_work("worker-stream").unwrap(),
            AssistantPendingWork::Running
        );
        let conn = db.read_conn().unwrap();
        let item = execution_item(&conn, "worker-stream").unwrap().unwrap().0;
        assert!(has_internal_continuation(&conn, &item).unwrap());
    });
}

#[test]
fn unrelated_completions_cannot_take_events_from_an_unsafe_consumer() {
    isolated_test(|db, _| {
        let item = seed_execution(&db);
        seed_child_result(&db, "a", "parent");
        seed_child_result(&db, "b", "parent");
        db.request_assistant_item_followup(
            &item,
            "owner-turn",
            "followup",
            "work",
            "Finish this output",
            0,
        )
        .unwrap();
        terminal(&db, "committed");
        let make = |id: &str| CreateStreamRun {
            run_id: id.into(),
            session_id: "work".into(),
            source: "parent_injection".into(),
            stream_id: Some(format!("stream-{id}")),
            turn_id: None,
            provider_shape: None,
        };
        let a = db
            .assistant_injection_proof("work", &[("subagent".into(), "a".into())])
            .unwrap()
            .unwrap();
        db.create_assistant_injection_stream(&make("consumer-a"), &a)
            .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("INSERT INTO assistant_budget_reservations(token,item_id,root_turn_id,execution_id,kind,
                started_at,reserved_seconds) VALUES('unknown-write',?1,'worker-turn','consumer-a','tool',?2,10)",
                params![item,chrono::Utc::now().timestamp()]).unwrap();
            conn.execute(
                "UPDATE chat_stream_runs SET status='interrupted' WHERE run_id='consumer-a'",
                [],
            )
            .unwrap();
        }
        let b = db
            .assistant_injection_proof("work", &[("subagent".into(), "b".into())])
            .unwrap()
            .unwrap();
        db.create_assistant_injection_stream(&make("consumer-b"), &b)
            .unwrap();
        assert!(!db
            .assistant_execution_owner_events("consumer-b")
            .unwrap()
            .contains("Finish this output"));
        let conn = db.read_conn().unwrap();
        let consumer: String = conn
            .query_row(
                "SELECT consumer_execution_id FROM assistant_followups
            WHERE id LIKE 'owner-followup:%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(consumer, "consumer-a");
    });
}

#[test]
fn only_the_delivered_owner_event_batch_is_claimed_and_the_answer_has_priority() {
    isolated_test(|db, _| {
        let item = seed_execution(&db);
        seed_child_result(&db, "result", "parent");
        for n in 0..8 {
            db.request_assistant_item_followup(
                &item,
                "owner-turn",
                &format!("extra-{n}"),
                "work",
                &format!("Detail {n}"),
                0,
            )
            .unwrap();
        }
        let question = db
            .create_assistant_question("worker-stream", "Which format?", vec![])
            .unwrap();
        let target = question.owner_response.unwrap().assistant_item.unwrap();
        db.answer_assistant_question(
            &question.request_id,
            &target,
            &[crate::ask_user::AskUserQuestionAnswer {
                question_id: "decision".into(),
                selected: vec![],
                custom_input: Some("Markdown".into()),
                files: vec![],
            }],
        )
        .unwrap();
        terminal(&db, "committed");
        let proof = db
            .assistant_injection_proof("work", &[("subagent".into(), "result".into())])
            .unwrap()
            .unwrap();
        db.create_assistant_injection_stream(
            &CreateStreamRun {
                run_id: "batch-consumer".into(),
                session_id: "work".into(),
                source: "parent_injection".into(),
                stream_id: Some("batch-stream".into()),
                turn_id: None,
                provider_shape: None,
            },
            &proof,
        )
        .unwrap();
        let data: serde_json::Value = serde_json::from_str(
            &db.assistant_execution_owner_events("batch-consumer")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(data["ownerFollowupData"].as_array().unwrap().len(), 8);
        assert!(data.to_string().contains("Markdown"));
        let pending: i64 = db
            .read_conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM assistant_followups WHERE state='pending'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            pending, 1,
            "unclaimed ninth input remains durable for the next batch"
        );
    });
}

#[test]
fn dropping_a_live_operation_keeps_its_unknown_reservation_and_missing_identity_is_denied() {
    isolated_test(|db, _| {
        seed_execution(&db);
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let lease = begin_model_budget(db.clone(), "worker-stream".into())
                .await
                .unwrap()
                .unwrap();
            drop(lease);
            tokio::task::yield_now().await;
            let ctx = crate::tool_defs::ToolExecContext {
                session_id: Some("work".into()),
                session_db: Some(crate::tool_defs::SessionDbHandle(db.clone())),
                turn_id: Some("worker-turn".into()),
                ..Default::default()
            };
            assert!(check_execution(&ctx).await.is_err());
        });
        let conn = db.read_conn().unwrap();
        let (held, count): (i64, i64) = conn
            .query_row(
                "SELECT SUM(reserved_seconds),COUNT(*)
            FROM assistant_budget_reservations WHERE settled_at IS NULL",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((held, count), (90, 1));
    });
}

#[test]
fn custom_hook_guard_is_scoped_to_the_exact_execution_and_survives_commit() {
    isolated_test(|db, _| {
        seed_execution(&db);
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            assert!(!custom_hooks_blocked());
            execution_scope(db.clone(), Some("worker-stream".into()), async {
                assert!(custom_hooks_blocked());
                terminal(&db, "committed");
                tokio::task::yield_now().await;
                assert!(
                    custom_hooks_blocked(),
                    "terminal hooks retain the sealed execution guard"
                );
            })
            .await;
            assert!(
                !custom_hooks_blocked(),
                "ordinary owner turns retain custom hooks"
            );
        });
    });
}

#[test]
fn archived_then_restored_target_never_declassifies_the_old_execution_or_sources() {
    isolated_test(|db, _| {
        let item = seed_execution(&db);
        seed_child_result(&db, "late-owned-child", "parent");
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET archived_at=?1 WHERE id='work'",
                [chrono::Utc::now().to_rfc3339()],
            )
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET archived_at=NULL WHERE id='work'", [])
            .unwrap();
        assert!(db.list_assistant_items().unwrap().is_empty());
        assert_eq!(
            db.assistant_execution_role("worker-stream").unwrap(),
            Some(false)
        );
        assert_eq!(
            db.assistant_source_owner("subagent", "late-owned-child")
                .unwrap(),
            Some(item.clone())
        );
        assert!(db
            .assistant_injection_proof("work", &[("subagent".into(), "late-owned-child".into())])
            .is_err());
        assert!(db
            .assistant_subagent_proof("session-late-owned-child")
            .is_err());
        assert!(db.assistant_stopped_work().unwrap().contains(&item));
        let conn = db.read_conn().unwrap();
        assert!(consume_source_with_conn(&conn, "subagent", "late-owned-child", None).is_err());
        drop(conn);
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            assert!(begin_model_budget(db.clone(), "worker-stream".into())
                .await
                .is_err());
            let ctx = crate::tool_defs::ToolExecContext {
                session_id: Some("work".into()),
                session_db: Some(crate::tool_defs::SessionDbHandle(db.clone())),
                persistence_run_id: Some("worker-stream".into()),
                ..Default::default()
            };
            assert!(check_execution(&ctx).await.is_err());
            assert!(
                check_tool_capability(&ctx, "exec", &serde_json::json!({"command":"true"}))
                    .await
                    .is_err()
            );
            let missing = crate::tool_defs::ToolExecContext {
                persistence_run_id: None,
                ..ctx
            };
            assert!(
                check_execution(&missing).await.is_err(),
                "removing the exact identity cannot bypass an active owned receipt"
            );
        });
    });
}

#[test]
fn deleting_home_or_worker_content_keeps_exact_identity_denial() {
    for delete_home in [true, false] {
        isolated_test(|db, _| {
            seed_execution(&db);
            seed_child_result(&db, "deleted-owned-child", "parent");
            let target = if delete_home {
                db.read_conn()
                    .unwrap()
                    .query_row(
                        "SELECT home_session_id FROM personal_assistant_requests LIMIT 1",
                        [],
                        |r| r.get::<_, String>(0),
                    )
                    .unwrap()
            } else {
                "work".into()
            };
            db.delete_session(&target).unwrap();
            assert_eq!(
                db.assistant_execution_role("worker-stream").unwrap(),
                Some(false)
            );
            assert!(db
                .assistant_subagent_proof("session-deleted-owned-child")
                .is_err());
            let conn = db.read_conn().unwrap();
            assert!(execution_item(&conn, "worker-stream").is_err());
            assert!(
                consume_source_with_conn(&conn, "subagent", "deleted-owned-child", None).is_err()
            );
        });
    }
}

#[test]
fn identity_receipts_cannot_be_recreated_after_their_canonical_scope_is_removed() {
    isolated_test(|db, _| {
        seed_execution(&db);
        db.conn
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM assistant_execution_contexts WHERE execution_id='worker-stream'",
                [],
            )
            .unwrap();
        assert!(db.conn.lock().unwrap().execute("INSERT INTO assistant_execution_contexts(execution_id,root_turn_id,item_id,item_revision)
            SELECT 'worker-stream','worker-turn',id,revision FROM assistant_items",[]).is_err());
        assert!(execution_item(&db.read_conn().unwrap(), "worker-stream").is_err());
    });
}

#[test]
fn archived_scope_cancels_durable_questions_and_keeps_completed_parents_sources_owned() {
    isolated_test(|db, _| {
        let item = seed_execution(&db);
        seed_child_result(&db, "orphan-child", "parent");
        let question = db
            .create_assistant_question("worker-stream", "Which result?", vec![])
            .unwrap();
        terminal(&db, "committed");
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET archived_at=?1 WHERE id='work'",
                [chrono::Utc::now().to_rfc3339()],
            )
            .unwrap();
        let status: String = db
            .read_conn()
            .unwrap()
            .query_row(
                "SELECT status FROM ask_user_questions WHERE request_id=?1",
                [&question.request_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "cancelled");
        assert!(db.assistant_stopped_work().unwrap().contains(&item));
        assert!(db
            .assistant_injection_proof("work", &[("subagent".into(), "orphan-child".into())])
            .is_err());
    });
}

#[test]
fn supplement_waits_for_queued_original_and_completes_only_after_its_own_delivery() {
    isolated_test(|db, _| {
        let home = seed_source(&db);
        seed_target(&db, "work");
        let original = db
            .enqueue_personal_assistant_request(input("work"))
            .unwrap();
        db.request_assistant_item_followup(
            &original.id,
            "owner-turn",
            "queued-extra",
            "work",
            "Include the updated outline",
            0,
        )
        .unwrap();
        assert_eq!(db.queue_assistant_followups().unwrap(), 0);
        admit(&db, &home, &original, "worker-turn").unwrap();
        db.create_assistant_work_stream(&CreateStreamRun {
            run_id: "worker-stream".into(),
            session_id: "work".into(),
            source: "session_tool".into(),
            stream_id: Some("original-stream".into()),
            turn_id: Some("worker-turn".into()),
            provider_shape: None,
        })
        .unwrap();
        db.report_assistant_item("work", "worker-stream", "done", "Original verified", None)
            .unwrap();
        terminal(&db, "committed");
        db.settle_assistant_execution("worker-stream").unwrap();
        assert_eq!(db.queue_assistant_followups().unwrap(), 1);
        // Reconciliation can revisit the original after the supplement queues.
        db.settle_assistant_execution("worker-stream").unwrap();
        assert!(matches!(
            db.list_assistant_items().unwrap()[0].state,
            ItemState::Open
        ));
        let next = db
            .list_personal_assistant_requests()
            .unwrap()
            .into_iter()
            .find(|r| r.id != original.id)
            .unwrap();
        admit(&db, &home, &next, "supplement-turn").unwrap();
        db.create_assistant_work_stream(&CreateStreamRun {
            run_id: "supplement-stream".into(),
            session_id: "work".into(),
            source: "session_tool".into(),
            stream_id: Some("supplement".into()),
            turn_id: Some("supplement-turn".into()),
            provider_shape: None,
        })
        .unwrap();
        db.report_assistant_item(
            "work",
            "supplement-stream",
            "done",
            "Updated outline verified",
            None,
        )
        .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE chat_stream_runs SET status='committed' WHERE run_id='supplement-stream'",
                [],
            )
            .unwrap();
        db.settle_assistant_execution("supplement-stream").unwrap();
        assert!(matches!(
            db.list_assistant_items().unwrap()[0].state,
            ItemState::Done
        ));
    });
}

#[test]
fn terminal_model_allocations_keep_charges_without_blocking_explicit_continuation() {
    isolated_test(|db, _| {
        let item = seed_execution(&db);
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            for _ in 0..2 {
                drop(begin_model_budget(db.clone(), "worker-stream".into()).await.unwrap().unwrap());
            }
            terminal(&db, "interrupted");
            db.settle_assistant_execution("worker-stream").unwrap();
            db.continue_assistant_item(&item).unwrap();
            assert_eq!(db.queue_assistant_followups().unwrap(), 1);
            let home = db.ensure_personal_assistant_home().unwrap();
            let next = db.list_personal_assistant_requests().unwrap().into_iter().find(|r| r.id != item).unwrap();
            admit(&db, &home, &next, "resumed-turn").unwrap();
            db.create_assistant_work_stream(&CreateStreamRun {
                run_id: "resumed-stream".into(), session_id: "work".into(), source: "session_tool".into(),
                stream_id: Some("resumed".into()), turn_id: Some("resumed-turn".into()), provider_shape: None,
            }).unwrap();
            let resumed = tokio::time::timeout(std::time::Duration::from_secs(2), begin_model_budget(db.clone(), "resumed-stream".into())).await.unwrap().unwrap().unwrap();
            resumed.finish().await.unwrap();
            let conn = db.read_conn().unwrap();
            let retained: i64 = conn.query_row("SELECT SUM(reserved_seconds) FROM assistant_budget_reservations WHERE execution_id='worker-stream' AND settled_at IS NULL", [], |r| r.get(0)).unwrap();
            assert_eq!(retained, 180, "unknown charges must not be refunded");
        });
    });
}

#[test]
fn explicit_continuation_hands_failed_supplement_to_the_verifying_consumer() {
    isolated_test(|db, _| {
        let item = seed_execution(&db);
        db.request_assistant_item_followup(
            &item,
            "owner-turn",
            "failed-extra",
            "work",
            "Verify the updated outline before writing",
            0,
        )
        .unwrap();
        db.report_assistant_item("work", "worker-stream", "done", "Original verified", None)
            .unwrap();
        terminal(&db, "committed");
        db.settle_assistant_execution("worker-stream").unwrap();
        assert_eq!(db.queue_assistant_followups().unwrap(), 1);
        let home = db.ensure_personal_assistant_home().unwrap();
        let next = db
            .list_personal_assistant_requests()
            .unwrap()
            .into_iter()
            .find(|r| r.status == "queued")
            .unwrap();
        admit(&db, &home, &next, "failed-extra-turn").unwrap();
        db.create_assistant_work_stream(&CreateStreamRun {
            run_id: "failed-extra-stream".into(),
            session_id: "work".into(),
            source: "session_tool".into(),
            stream_id: Some("failed-extra".into()),
            turn_id: Some("failed-extra-turn".into()),
            provider_shape: None,
        })
        .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE chat_stream_runs SET status='failed' WHERE run_id='failed-extra-stream'",
                [],
            )
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE chat_turns SET status='failed' WHERE id='failed-extra-turn'",
                [],
            )
            .unwrap();
        db.reconcile_personal_assistant_requests().unwrap();
        db.settle_assistant_execution("failed-extra-stream")
            .unwrap();
        db.continue_assistant_item(&item).unwrap();
        for n in 0..7 {
            db.request_assistant_item_followup(
                &item,
                "owner-turn",
                &format!("extra-{n}"),
                "work",
                &format!("New owner detail {n}"),
                0,
            )
            .unwrap();
        }
        assert_eq!(db.queue_assistant_followups().unwrap(), 1);
        let resume = db
            .list_personal_assistant_requests()
            .unwrap()
            .into_iter()
            .find(|r| r.status == "queued")
            .unwrap();
        assert!(resume.instruction.contains("Verify unknown prior outcomes"));
        admit(&db, &home, &resume, "verify-turn").unwrap();
        db.create_assistant_work_stream(&CreateStreamRun {
            run_id: "verify-stream".into(),
            session_id: "work".into(),
            source: "session_tool".into(),
            stream_id: Some("verify".into()),
            turn_id: Some("verify-turn".into()),
            provider_shape: None,
        })
        .unwrap();
        let delivered = db
            .assistant_execution_owner_events("verify-stream")
            .unwrap();
        assert!(delivered.contains("updated outline"));
        for n in 0..7 {
            assert!(delivered.contains(&format!("New owner detail {n}")));
        }
        db.report_assistant_item(
            "work",
            "verify-stream",
            "done",
            "Verified the prior outcome and updated outline",
            None,
        )
        .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE chat_stream_runs SET status='committed' WHERE run_id='verify-stream'",
                [],
            )
            .unwrap();
        db.settle_assistant_execution("verify-stream").unwrap();
        assert!(matches!(
            db.list_assistant_items().unwrap()[0].state,
            ItemState::Done
        ));
    });
}

#[test]
fn failed_owner_claims_over_one_batch_leave_a_schedulable_remainder() {
    isolated_test(|db, _| {
        let item = seed_execution(&db);
        for n in 0..9 {
            // Preserve the old attempts as a real failed-queue history would.
            db.conn.lock().unwrap().execute("INSERT INTO personal_assistant_requests(id,home_session_id,source_turn_id,source_message_id,item_key,target_session_id,instruction,status,global_stop_epoch,source_stop_epoch,target_stop_epoch,created_at,updated_at)
                SELECT ?1,home_session_id,source_turn_id,source_message_id,?2,target_session_id,?3,'failed',global_stop_epoch,source_stop_epoch,target_stop_epoch,created_at,updated_at FROM personal_assistant_requests WHERE id=?4",
                params![format!("old-request-{n}"),format!("owner-followup:old:{n}"),format!("Old owner detail {n}"),item]).unwrap();
            db.conn.lock().unwrap().execute("INSERT INTO assistant_followups(id,item_id,item_revision,source_turn_id,instruction,state,consumer_execution_id,due_at)
                VALUES(?1,?2,1,'owner-turn',?3,'claimed','worker-stream',0)", params![format!("owner-followup:old:{n}"),item,format!("Old owner detail {n}")]).unwrap();
            db.conn
                .lock()
                .unwrap()
                .execute(
                    "UPDATE assistant_followups SET request_id=?2 WHERE id=?1",
                    params![
                        format!("owner-followup:old:{n}"),
                        format!("old-request-{n}")
                    ],
                )
                .unwrap();
        }
        terminal(&db, "failed");
        db.settle_assistant_execution("worker-stream").unwrap();
        db.continue_assistant_item(&item).unwrap();
        let home = db.ensure_personal_assistant_home().unwrap();
        for batch in 0..2 {
            assert_eq!(db.queue_assistant_followups().unwrap(), 1);
            let request = db
                .list_personal_assistant_requests()
                .unwrap()
                .into_iter()
                .find(|r| r.status == "queued")
                .unwrap();
            assert!(request
                .instruction
                .contains("Verify unknown prior outcomes"));
            let turn = format!("batch-turn-{batch}");
            let execution = format!("batch-stream-{batch}");
            admit(&db, &home, &request, &turn).unwrap();
            db.create_assistant_work_stream(&CreateStreamRun {
                run_id: execution.clone(),
                session_id: "work".into(),
                source: "session_tool".into(),
                stream_id: Some(format!("batch-{batch}")),
                turn_id: Some(turn.clone()),
                provider_shape: None,
            })
            .unwrap();
            let delivered: serde_json::Value =
                serde_json::from_str(&db.assistant_execution_owner_events(&execution).unwrap())
                    .unwrap();
            assert_eq!(
                delivered["ownerFollowupData"].as_array().unwrap().len(),
                if batch == 0 { 8 } else { 1 }
            );
            db.report_assistant_item(
                "work",
                &execution,
                "done",
                "Verified delivered owner details",
                None,
            )
            .unwrap();
            db.conn
                .lock()
                .unwrap()
                .execute(
                    "UPDATE chat_stream_runs SET status='committed' WHERE run_id=?1",
                    [&execution],
                )
                .unwrap();
            db.conn
                .lock()
                .unwrap()
                .execute(
                    "UPDATE chat_turns SET status='completed' WHERE id=?1",
                    [&turn],
                )
                .unwrap();
            db.reconcile_personal_assistant_requests().unwrap();
            db.settle_assistant_execution(&execution).unwrap();
            assert!(
                matches!(db.list_assistant_items().unwrap()[0].state, ItemState::Done)
                    == (batch == 1)
            );
        }
    });
}
