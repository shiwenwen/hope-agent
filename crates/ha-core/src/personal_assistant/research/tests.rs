use super::*;
use crate::turn_durability::TurnDurabilitySink;
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

#[test]
fn actual_file_baseline_is_quiet_and_grant_retries_never_reenable_revoked_scope() {
    isolated(|db, root| {
        std::fs::write(root.join("agenda.md"), "Friday 09:00").unwrap();
        let request = input("agenda.md");
        let source = runtime()
            .block_on(create_source(db.clone(), request.clone()))
            .unwrap();
        assert_eq!(
            runtime()
                .block_on(create_source(db.clone(), request.clone()))
                .unwrap()
                .id,
            source.id
        );
        assert!(db
            .claim_assistant_research(chrono::Utc::now().timestamp())
            .unwrap()
            .is_none());
        assert_eq!(
            db.read_conn()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM assistant_source_events", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        db.revoke_assistant_source(&source.id, chrono::Utc::now().timestamp())
            .unwrap();
        assert!(runtime()
            .block_on(create_source(db.clone(), request))
            .is_err());
        assert_eq!(db.list_assistant_sources().unwrap()[0].state, "revoked");
    });
}

#[test]
fn actual_reader_rejects_escape_oversize_invalid_utf8_and_changed_root_before_reading_it() {
    isolated(|db, root| {
        let source = share(db.clone(), root, "agenda.md");
        assert!(read_shared_file(&db, "work", "../outside", None).is_err());
        std::fs::write(root.join("large.txt"), vec![b'a'; SOURCE_BYTES + 1]).unwrap();
        assert!(read_shared_file(&db, "work", "large.txt", None).is_err());
        std::fs::write(root.join("bad.txt"), [0xff, 0xfe]).unwrap();
        assert!(read_shared_file(&db, "work", "bad.txt", None).is_err());
        let key: String = db
            .read_conn()
            .unwrap()
            .query_row(
                "SELECT workspace_key FROM assistant_sources WHERE id=?1",
                [&source.id],
                |r| r.get(0),
            )
            .unwrap();
        let next = root.join("next");
        std::fs::create_dir(&next).unwrap();
        std::fs::write(next.join("agenda.md"), [0xff]).unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET working_dir=?1 WHERE id='work'",
                [next.to_str().unwrap()],
            )
            .unwrap();
        let error = read_shared_file(&db, "work", "agenda.md", Some(&key)).unwrap_err();
        assert!(error.downcast_ref::<sources::WorkspaceChanged>().is_some());
        assert_eq!(db.list_assistant_sources().unwrap()[0].state, "revoked");
    });
}

#[test]
fn source_change_uses_a_private_formal_stream_and_publication_waits_for_commit() {
    isolated(|db, root| {
        let source = share(db.clone(), root, "agenda.md");
        std::fs::write(
            root.join("agenda.md"),
            "Planning: Friday 09:00\nReview: Friday 09:00\n",
        )
        .unwrap();
        runtime()
            .block_on(refresh_owner_sources(db.clone()))
            .unwrap();
        let current = db.list_assistant_sources().unwrap().remove(0);
        assert_ne!(source.version, current.version);
        let pending = claim(&db);
        let execution = admit(&db, &pending);
        assert_eq!(
            db.get_session_kind(&pending.session_id).unwrap(),
            Some(crate::session::SessionKind::PersonalAssistantResearch)
        );
        publish(&db, &execution, &current);
        assert_eq!(
            db.list_assistant_findings(chrono::Utc::now().timestamp())
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            db.read_conn()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM goals", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    });
}

#[test]
fn full_input_scope_revocation_invalidates_a_finding_even_when_it_cites_only_one_file() {
    isolated(|db, root| {
        let first = share(db.clone(), root, "agenda.md");
        let other = share(db.clone(), root, "travel.md");
        std::fs::write(root.join("agenda.md"), "Planning moved to 10:00\n").unwrap();
        runtime()
            .block_on(refresh_owner_sources(db.clone()))
            .unwrap();
        let current = db
            .list_assistant_sources()
            .unwrap()
            .into_iter()
            .find(|s| s.id == first.id)
            .unwrap();
        let pending = claim(&db);
        let execution = admit(&db, &pending);
        publish(&db, &execution, &current);
        db.revoke_assistant_source(&other.id, chrono::Utc::now().timestamp())
            .unwrap();
        assert!(db
            .list_assistant_findings(chrono::Utc::now().timestamp())
            .unwrap()
            .is_empty());
        assert!(db
            .report_assistant_research(&execution, vec![finding(&current)])
            .is_err());
        assert_eq!(db.read_conn().unwrap().query_row("SELECT COUNT(*) FROM assistant_research_inputs WHERE run_id=?1 AND body IS NOT NULL",[&pending.run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    });
}

#[test]
fn actual_dispatch_allocations_count_repair_attempts_and_survive_home_deletion() {
    isolated(|db, root| {
        let source = share(db.clone(), root, "agenda.md");
        db.request_assistant_source_review(
            &source.id,
            &uuid::Uuid::new_v4().to_string(),
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
        let pending = claim(&db);
        let execution = admit(&db, &pending);
        runtime().block_on(async {
            let lease = super::super::items::begin_model_budget(db.clone(), execution.clone())
                .await
                .unwrap()
                .unwrap();
            reserve_model_attempt(db.clone(), execution.clone())
                .await
                .unwrap();
            reserve_model_attempt(db.clone(), execution.clone())
                .await
                .unwrap();
            assert!(reserve_model_attempt(db.clone(), execution.clone())
                .await
                .is_err());
            drop(lease);
        });
        let home = db.ensure_personal_assistant_home().unwrap();
        db.delete_session(&home.id).unwrap();
        assert_eq!(
            db.read_conn()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM assistant_research_usage", [], |r| r
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            2
        );
        assert_eq!(db.read_conn().unwrap().query_row("SELECT SUM(reserved_seconds) FROM assistant_research_reservations WHERE settled_at IS NULL",[],|r|r.get::<_,i64>(0)).unwrap(),90);
    });
}

#[test]
fn private_body_cannot_leave_through_generic_history_search_export_recovery_or_hooks() {
    isolated(|db, root| {
        let source = share(db.clone(), root, "agenda.md");
        db.request_assistant_source_review(
            &source.id,
            &uuid::Uuid::new_v4().to_string(),
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
        let pending = claim(&db);
        let execution = admit(&db, &pending);
        let marker = "PRIVATE_RESEARCH_BODY_936";
        let id = {
            let conn = db.conn.lock().unwrap();
            conn.execute("INSERT INTO messages(session_id,role,content,timestamp) VALUES(?1,'assistant',?2,'now')",params![pending.session_id,marker]).unwrap();
            conn.last_insert_rowid()
        };
        assert!(db
            .load_session_messages(&pending.session_id)
            .unwrap()
            .is_empty());
        assert!(db.get_message(id).unwrap().is_none());
        assert!(db
            .search_message_content(marker, None, Some(&pending.session_id), None, 20)
            .unwrap()
            .is_empty());
        assert!(db.load_context(&pending.session_id).unwrap().is_none());
        assert!(db.save_context(&pending.session_id, marker).is_err());
        assert!(db.stream_run_snapshot(&execution).unwrap().is_none());
        assert!(db.create_side_chat(&pending.session_id).is_err());
        assert!(
            !crate::hooks::transcript::TranscriptMirror::backfill_session(&db, &pending.session_id)
                .unwrap()
        );
        assert!(crate::session::export::export_session(
            &db,
            &pending.session_id,
            crate::session::export::ExportOptions::full(crate::session::export::ExportFormat::Json)
        )
        .is_err());
    });
}

#[test]
fn pending_owner_pause_and_archive_restore_do_not_revive_old_research() {
    isolated(|db, root| {
        let source = share(db.clone(), root, "agenda.md");
        db.request_assistant_source_review(
            &source.id,
            &uuid::Uuid::new_v4().to_string(),
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
        let pending = claim(&db);
        let execution = admit(&db, &pending);
        db.request_assistant_policy(
            1,
            super::super::policy::AssistantPolicySettings {
                paused: true,
                research_enabled: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(db
            .report_assistant_research(&execution, vec![finding(&source)])
            .is_err());
        assert!(db.assistant_policy_status().unwrap().pending);
        db.conn
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET archived_at='now' WHERE id='work'", [])
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET archived_at=NULL WHERE id='work'", [])
            .unwrap();
        assert_eq!(db.list_assistant_sources().unwrap()[0].state, "revoked");
        assert!(db
            .list_assistant_findings(chrono::Utc::now().timestamp())
            .unwrap()
            .is_empty());
    });
}

#[test]
fn a_stopped_background_reader_cannot_borrow_a_fresh_owner_read() {
    isolated(|db, root| {
        let source = share(db.clone(), root, "agenda.md");
        db.request_assistant_source_review(
            &source.id,
            &uuid::Uuid::new_v4().to_string(),
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
        let pending = claim(&db);
        let home = db.ensure_personal_assistant_home().unwrap();
        db.prepare_session_autonomy_pause(&home.id).unwrap();
        std::fs::write(root.join("agenda.md"), "DO_NOT_READ_FROM_OLD_RUN\n").unwrap();
        assert!(runtime()
            .block_on(sources::refresh_research_sources(
                db.clone(),
                pending.run_id
            ))
            .is_err());
        let version = db.list_assistant_sources().unwrap().remove(0).version;
        assert_eq!(version, source.version);
        runtime()
            .block_on(refresh_owner_sources(db.clone()))
            .unwrap();
        assert_ne!(
            db.list_assistant_sources().unwrap().remove(0).version,
            source.version,
            "an explicit owner read retains separate, bounded read permission"
        );
    });
}

fn prepared_plan(
    db: &SessionDB,
    session: &str,
    execution: &str,
) -> crate::session::context_projection::PreparedRequestBodyIdentity {
    use crate::session::context_projection::{
        ExactPayloadAvailability, NewRequestProjectionPlan, RequestProjectionRole,
    };
    let version = db.get_request_projection_version(session).unwrap();
    let plan = NewRequestProjectionPlan {
        request_plan_id: "private-plan".into(),
        session_id: session.into(),
        branch_id: "main".into(),
        run_id: execution.into(),
        attempt_no: 1,
        request_role: RequestProjectionRole::MainContinuation,
        expected_canonical_generation: version.canonical_generation,
        expected_context_revision: version.context_revision,
        projection_epoch_id: None,
        cache_identity_hash: "cache".into(),
        provider_id: "mock".into(),
        provider_profile_id: None,
        model_id: "mock".into(),
        request_shape: "messages-v1".into(),
        writer_version: 1,
        renderer_version: 1,
        policy_fingerprint: "policy".into(),
        counter_profile: "counter".into(),
        exact_payload_id: None,
        exact_payload_reservation_id: None,
        exact_payload_keyed_digest: None,
        exact_payload_storage_kind: None,
        exact_payload_stored_bytes: None,
        payload_availability: ExactPayloadAvailability::Unavailable,
        projection_bytes: 2,
        expires_at: None,
        final_capacity_count_json: "{}".into(),
        prepared_body_fingerprint: "a".repeat(64),
        prepared_body_bytes: 2,
        endpoint_kind: "chat".into(),
        content_type: "application/json".into(),
    };
    db.create_context_committed_request_projection_plan(&plan)
        .unwrap();
    crate::session::context_projection::PreparedRequestBodyIdentity {
        fingerprint: "a".repeat(64),
        bytes: 2,
        endpoint_kind: "chat".into(),
        content_type: "application/json".into(),
    }
}

#[test]
fn dispatch_claim_rechecks_policy_stop_and_expiry_after_reserving_the_attempt() {
    for boundary in ["policy", "stop", "expiry", "current"] {
        isolated(|db, root| {
            let source = share(db.clone(), root, "agenda.md");
            db.request_assistant_source_review(
                &source.id,
                &uuid::Uuid::new_v4().to_string(),
                chrono::Utc::now().timestamp(),
            )
            .unwrap();
            let pending = claim(&db);
            let execution = admit(&db, &pending);
            runtime()
                .block_on(reserve_model_attempt(db.clone(), execution.clone()))
                .unwrap();
            let body = prepared_plan(&db, &pending.session_id, &execution);
            match boundary {
                "policy" => {
                    db.request_assistant_policy(1, Default::default()).unwrap();
                }
                "stop" => {
                    let home = db.ensure_personal_assistant_home().unwrap();
                    db.prepare_session_autonomy_pause(&home.id).unwrap();
                }
                "expiry" => {
                    db.conn
                        .lock()
                        .unwrap()
                        .execute("UPDATE assistant_sources SET expires_at=0", [])
                        .unwrap();
                }
                _ => {}
            }
            let claimed = db.claim_request_dispatch(
                &pending.session_id,
                "private-plan",
                "wire-attempt",
                None,
                &body,
            );
            if boundary == "current" {
                assert!(claimed.unwrap());
            } else {
                assert!(claimed.is_err(), "{boundary}");
            }
        });
    }
}

#[test]
fn public_live_snapshot_never_exposes_research_even_after_source_or_session_deletion() {
    isolated(|db, root| {
        let source = share(db.clone(), root, "agenda.md");
        db.request_assistant_source_review(
            &source.id,
            &uuid::Uuid::new_v4().to_string(),
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
        let pending = claim(&db);
        let proof = db.research_proof(&pending.run_id).unwrap();
        let execution = uuid::Uuid::new_v4().to_string();
        let (registration, _) = db
            .admit_research_stream(
                &proof,
                &crate::session::CreateStreamRun {
                    run_id: execution,
                    session_id: pending.session_id.clone(),
                    source: "assistant_research".into(),
                    stream_id: None,
                    turn_id: None,
                    provider_shape: None,
                },
            )
            .unwrap();
        runtime().block_on(async {
            let coordinator=crate::chat_engine::durability::StreamCoordinator::from_admission(
                db.clone(),pending.session_id.clone(),crate::chat_engine::ChatSource::AssistantResearch,None,None,
                Arc::new(crate::chat_engine::NoopEventSink),Arc::new(std::sync::atomic::AtomicBool::new(false)),registration,
            ).await.unwrap();
            coordinator.begin_attempt(Some("mock"),Some("model"),Some("anthropic")).await.unwrap();
            coordinator.accept_event(&serde_json::json!({"type":"text_delta","content":"PRIVATE_LIVE_RESEARCH_SENTINEL"}).to_string()).unwrap();
            coordinator.flush(crate::turn_durability::FlushReason::ToolBoundary).await.unwrap();
            assert!(crate::chat_engine::durability::active_snapshot(&pending.session_id).unwrap().events.iter().any(|e|e.event.contains("PRIVATE_LIVE_RESEARCH_SENTINEL")));
            assert!(crate::chat_engine::session_stream_snapshot(&pending.session_id).unwrap().is_none());
            db.revoke_assistant_source(&source.id,chrono::Utc::now().timestamp()).unwrap();
            assert!(crate::chat_engine::session_stream_snapshot(&pending.session_id).unwrap().is_none());
            db.delete_session(&pending.session_id).unwrap();
            assert!(crate::chat_engine::session_stream_snapshot(&pending.session_id).unwrap().is_none());
        });
    });
}

#[test]
fn returning_to_a_consumed_content_version_creates_a_new_research_event() {
    isolated(|db, root| {
        let source = share(db.clone(), root, "agenda.md");
        let original = std::fs::read_to_string(root.join("agenda.md")).unwrap();
        let changed = "Planning 09:00\nReview 09:00\n";
        let mut runs = Vec::new();
        for body in [changed, original.as_str(), changed] {
            std::fs::write(root.join("agenda.md"), body).unwrap();
            runtime()
                .block_on(refresh_owner_sources(db.clone()))
                .unwrap();
            let pending = claim(&db);
            runs.push(pending.run_id.clone());
            let execution = admit(&db, &pending);
            db.report_assistant_research(&execution, vec![]).unwrap();
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
        assert_eq!(
            runs.into_iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            3
        );
        let count:i64=db.read_conn().unwrap().query_row("SELECT COUNT(*) FROM assistant_source_events WHERE source_id=?1 AND kind='change' AND run_id IS NOT NULL",[source.id],|row|row.get(0)).unwrap();
        assert_eq!(
            count, 3,
            "A → B → A → B has three observations, including the second B"
        );
    });
}

#[test]
fn active_sources_stay_visible_after_more_than_sixty_four_historical_grants() {
    isolated(|db, root| {
        let current = share(db.clone(), root, "agenda.md");
        {
            let conn = db.conn.lock().unwrap();
            for index in 0..70 {
                conn.execute("INSERT INTO assistant_sources(id,home_session_id,session_id,relative_path,workspace_key,focus,revision,state,expires_at,global_stop_epoch,home_stop_epoch,next_scan_at,next_review_at)
                    SELECT ?1,home_session_id,session_id,?2,workspace_key,focus,1,'revoked',0,0,0,0,0 FROM assistant_sources WHERE id=?3",
                    params![format!("!historical-{index:02}"),format!("revoked-{index}.txt"),current.id]).unwrap();
            }
        }
        let listed = db.list_assistant_sources().unwrap();
        assert!(listed
            .iter()
            .any(|source| source.id == current.id && source.state == "active"));
        assert_eq!(
            listed
                .iter()
                .filter(|source| source.state == "revoked")
                .count(),
            64
        );
        db.revoke_assistant_source(&current.id, chrono::Utc::now().timestamp())
            .unwrap();
        assert!(db
            .list_assistant_sources()
            .unwrap()
            .iter()
            .all(|source| source.state != "active"));
    });
}
