use super::*;
use crate::personal_assistant::AssistantDispatchInput;
use crate::personal_assistant_im::*;
use crate::session::{ChatTurnStatus, SessionKind};

fn isolated(test: impl FnOnce(Arc<SessionDB>, i64)) {
    let dir = tempfile::tempdir().unwrap();
    crate::test_support::with_env_vars(&[("HA_DATA_DIR", dir.path())], || {
        let db = Arc::new(SessionDB::open(&dir.path().join("sessions.db")).unwrap());
        crate::channel::db::ChannelDB::new(db.clone())
            .migrate()
            .unwrap();
        db.with_conn_for_test(|conn| { conn.execute("INSERT INTO sessions(id,agent_id,title,created_at,updated_at) VALUES ('work','ha-main','Release','now','now')",[])?; Ok(()) }).unwrap();
        test(db, chrono::Utc::now().timestamp());
    });
}

fn command(db: &SessionDB, now: i64) -> AssistantImContext {
    db.save_assistant_im_connection(700, "private_bot", &uuid::Uuid::new_v4().to_string(), now)
        .unwrap();
    let code = db.begin_assistant_im_pairing(now).unwrap();
    let lease = db.assistant_im_poll_lease(now).unwrap().unwrap();
    let input = |id, kind| TelegramInput {
        update_id: id,
        user_id: 42,
        chat_id: 42,
        sent_at: Some(now),
        kind,
    };
    db.receive_assistant_im_update(
        &lease,
        input(1, TelegramInputKind::PairCode(code.code)),
        now,
    )
    .unwrap();
    let activation = db
        .confirm_assistant_im_pairing(lease.generation, 42, 42, now)
        .unwrap();
    db.receive_assistant_im_update(
        &lease,
        input(
            2,
            TelegramInputKind::Activate(
                activation
                    .callback_data
                    .strip_prefix(ACTIVATE_PREFIX)
                    .unwrap()
                    .to_owned(),
            ),
        ),
        now,
    )
    .unwrap();
    db.set_assistant_im_grant(
        SessionGrant {
            session_id: "work".into(),
            revision: 0,
            query_status: true,
            delegate: true,
            stop: true,
            completion_notice: true,
            expires_at: now + 3600,
        },
        now,
    )
    .unwrap();
    let lease = db.assistant_im_poll_lease(now).unwrap().unwrap();
    db.receive_assistant_im_update(
        &lease,
        input(3, TelegramInputKind::Message("Check release".into())),
        now,
    )
    .unwrap();
    let command = db.assistant_im_pending_commands().unwrap().remove(0);
    let context = db
        .prepare_assistant_im_context(&command.request_id, now)
        .unwrap();
    db.claim_assistant_im_command(&command.request_id, now)
        .unwrap()
        .unwrap();
    context
}

fn admit(db: &SessionDB, context: &AssistantImContext) {
    let mut message = NewMessage::user(&context.text);
    message.source = Some("channel".into());
    db.append_message_and_create_chat_turn_with_id_surface_dispatch(
        &context.request_id,
        &context.view_session_id,
        "channel",
        None,
        &message,
        None,
        None,
        None,
        Some(
            db.foreground_stop_admission(Some(&context.view_session_id))
                .unwrap(),
        ),
    )
    .unwrap();
}

#[test]
fn personal_assistant_im_view_is_separate_fixed_and_rejects_owner_impersonation() {
    isolated(|db, now| {
        let context = command(&db, now);
        db.append_message(
            &context.home_session_id,
            &NewMessage::user("PRIVATE_HOME_SENTINEL"),
        )
        .unwrap();
        let view = db.get_session(&context.view_session_id).unwrap().unwrap();
        assert_eq!(view.kind, SessionKind::PersonalAssistantIm);
        assert!(db.has_personal_assistant_private_data().unwrap());
        assert!(db.is_personal_assistant_private_session(&view.id).unwrap());
        assert!(db.load_session_messages(&view.id).unwrap().is_empty());
        let agent_dir = crate::paths::agent_dir("private-prompt-test").unwrap();
        std::fs::create_dir_all(&agent_dir).unwrap();
        for name in [
            "AGENT.md",
            "PERSONA.md",
            "TOOLS.md",
            "MEMORY.md",
            "AGENTS.md",
        ] {
            std::fs::write(
                agent_dir.join(name),
                "PRIVATE_INSTRUCTION_MEMORY_WORKSPACE_SENTINEL",
            )
            .unwrap();
        }
        let prompt = crate::agent::config::build_system_prompt_bundle_with_session_db(
            "private-prompt-test",
            "model",
            "provider",
            Some(&view.id),
            Some(&db),
            None,
        );
        assert_eq!(prompt.prompt, SYSTEM_PROMPT);
        assert!(prompt.static_memory_refs.is_empty());
        assert!(!prompt.prompt.contains("PRIVATE_HOME_SENTINEL"));
        assert!(!prompt
            .prompt
            .contains("PRIVATE_INSTRUCTION_MEMORY_WORKSPACE_SENTINEL"));
        crate::user_config::save_user_config_to_disk(&crate::user_config::UserConfig {
            name: Some("PRIVATE_PROFILE_SENTINEL".into()),
            custom_info: Some("PRIVATE_PROFILE_SENTINEL".into()),
            ..Default::default()
        })
        .unwrap();
        let mut restricted_agent = crate::agent::AssistantAgent::new_anthropic("test-key");
        restricted_agent.set_agent_id("private-prompt-test");
        restricted_agent.set_session_db(db.clone());
        restricted_agent.set_session_id(&view.id);
        restricted_agent.set_tool_scope(Some(crate::tool_defs::ToolScope::PersonalAssistantIm));
        *restricted_agent.active_memory_suffix.lock().unwrap() =
            Some(Arc::new("PRIVATE_CACHED_CONTEXT_SENTINEL".into()));
        assert!(restricted_agent.current_active_memory_suffix().is_none());
        *restricted_agent.legacy_memory_suffix.lock().unwrap() =
            Some(Arc::new("PRIVATE_CACHED_CONTEXT_SENTINEL".into()));
        assert!(restricted_agent.current_legacy_memory_suffix().is_none());
        *restricted_agent.procedure_memory_suffix.lock().unwrap() =
            Some(Arc::new("PRIVATE_CACHED_CONTEXT_SENTINEL".into()));
        assert!(restricted_agent.current_procedure_memory_suffix().is_none());
        *restricted_agent.related_notes_suffix.lock().unwrap() =
            Some(Arc::new("PRIVATE_CACHED_CONTEXT_SENTINEL".into()));
        assert!(restricted_agent.current_related_notes_suffix().is_none());
        *restricted_agent.coding_profile_suffix.lock().unwrap() =
            Some(Arc::new("PRIVATE_CACHED_CONTEXT_SENTINEL".into()));
        assert!(restricted_agent.current_coding_profile_suffix().is_none());
        *restricted_agent.awareness_suffix.lock().unwrap() =
            Some(Arc::new("PRIVATE_CACHED_CONTEXT_SENTINEL".into()));
        assert!(restricted_agent.current_awareness_suffix().is_none());
        let context_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        context_runtime.block_on(async {
            assert!(restricted_agent.prepare_user_profile_data().await.is_none());
            assert!(restricted_agent
                .prepare_attached_knowledge_section()
                .await
                .is_none());
            assert_eq!(
                restricted_agent.prepare_session_policy_context().await,
                (None, None)
            );
            assert!(restricted_agent
                .current_capability_catalog_suffix()
                .is_none());
        });

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            use crate::turn_kernel::{
                InteractiveAdmission, TurnKernel, TurnRequest, TurnSubmission,
            };
            let request = |id: &str, cancel: Arc<AtomicBool>| {
                TurnRequest::new(
                    view.id.clone(),
                    crate::agent_loader::DEFAULT_AGENT_ID.to_owned(),
                    "forged".into(),
                    db.clone(),
                    Default::default(),
                    cancel,
                    Arc::new(crate::chat_engine::NoopEventSink),
                )
                .with_turn_id(id.to_owned())
            };
            let channel = TurnSubmission::channel(
                request("forged-channel", Arc::new(AtomicBool::new(false))),
                crate::knowledge::ChannelKbContext {
                    channel_id: "telegram".into(),
                    account_id: "ordinary".into(),
                    chat_id: "42".into(),
                    is_group: false,
                },
            );
            let failure = TurnKernel::admit(channel).await.err().unwrap();
            assert!(failure
                .to_string()
                .contains("cannot attach a persisted chat-turn identity"));
            let cancel = Arc::new(AtomicBool::new(false));
            let foreground = crate::chat_engine::active_turn::begin_durable_foreground_request(
                &db,
                Some(&view.id),
            )
            .unwrap();
            let lease = TurnKernel::begin_desktop(
                foreground,
                &view.id,
                "forged-desktop".into(),
                None,
                cancel.clone(),
            )
            .unwrap();
            let admission = InteractiveAdmission::desktop(
                lease,
                NewMessage::user("forged"),
                None,
                None,
                None,
                false,
                false,
            );
            let failure = TurnKernel::admit(TurnSubmission::desktop(
                request("forged-desktop", cancel),
                admission,
            ))
            .await
            .err()
            .unwrap();
            assert!(failure.to_string().contains("paired source proof"));
        });
        assert!(db
            .append_message_and_create_chat_turn_with_id_surface_dispatch(
                &context.request_id,
                &view.id,
                "desktop",
                None,
                &NewMessage::user("forged"),
                None,
                None,
                None,
                None
            )
            .is_err());
        assert!(db.load_session_messages(&view.id).unwrap().is_empty());
        admit(&db, &context);
        assert!(db
            .assistant_im_turn_targets(&view.id, "wrong-turn", now)
            .is_err());
        assert!(db
            .list_sessions_for_model(None, false, 100)
            .unwrap()
            .iter()
            .all(|s| s.id != view.id));
    });
}

#[test]
fn personal_assistant_im_dispatch_rechecks_revocation_after_source_completion() {
    isolated(|db, now| {
        let context = command(&db, now);
        admit(&db, &context);
        let input = AssistantDispatchInput {
            source_turn_id: context.request_id.clone(),
            item_key: "send-1".into(),
            target_session_id: "work".into(),
            instruction: "Report release status".into(),
            global_stop_epoch: db.global_stop_epoch().unwrap(),
        };
        let request = db
            .enqueue_personal_assistant_request(input.clone())
            .unwrap();
        assert_eq!(
            db.enqueue_personal_assistant_request(input).unwrap().id,
            request.id
        );
        db.finish_chat_turn_once(
            &context.request_id,
            ChatTurnStatus::Completed,
            None,
            None,
            None,
        )
        .unwrap();
        db.settle_assistant_im_command(&context.request_id, CommandTerminal::Completed)
            .unwrap();
        db.with_conn_for_test(|conn| {
            crate::personal_assistant_im::revalidate_assistant_im_dispatch(
                conn,
                &context.request_id,
                "work",
                now,
            )
        })
        .unwrap();
        db.set_assistant_im_grant(
            SessionGrant {
                session_id: "work".into(),
                revision: 1,
                query_status: true,
                delegate: false,
                stop: false,
                completion_notice: false,
                expires_at: now + 3600,
            },
            now,
        )
        .unwrap();
        let mut message = NewMessage::user("Report release status");
        message.attachments_meta =
            Some(serde_json::json!({"personal_assistant_request_id":request.id}).to_string());
        assert!(db
            .append_message_and_create_session_tool_turn_with_id(
                "dispatch-target",
                "work",
                Some(&context.home_session_id),
                &message,
                request.global_stop_epoch
            )
            .is_err());
        assert!(db.get_chat_turn("dispatch-target").unwrap().is_none());
        assert!(db.load_session_messages("work").unwrap().is_empty());
    });
}

#[test]
fn personal_assistant_im_scope_blocks_general_tools_and_continue_bypass() {
    isolated(|_, _| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let context = crate::tool_defs::ToolExecContext {
            tool_scope: Some(crate::tool_defs::ToolScope::PersonalAssistantIm),
            ..Default::default()
        };
        for name in [
            "exec",
            "sessions_history",
            "sessions_search",
            "sessions_create",
            "recall_memory",
            "read_context_resource",
            "session_continue",
            "skill",
            "channel_send",
        ] {
            assert!(!context.is_tool_visible(name), "{name}");
            assert!(
                runtime
                    .block_on(context.tool_visibility_error(name))
                    .is_some(),
                "{name}"
            );
        }
        assert!(context.is_tool_visible("sessions_send"));
        assert!(context.is_tool_visible("session_status"));
        let result=runtime.block_on(crate::tools::tool_search::tool_search(&serde_json::json!({"query":"select:exec,sessions_history,recall_memory,read_context_resource,session_continue,sessions_send"}),&context)).unwrap();
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert!(!tools.is_empty());
        assert!(tools.iter().all(|tool| tool["name"] == "sessions_send"));
    });
}

#[test]
fn personal_assistant_im_changed_scope_gets_empty_history_and_old_turn_loses_tools() {
    isolated(|db, now| {
        let old = command(&db, now);
        admit(&db, &old);
        let mut private_reply = NewMessage::user("OLD_SCOPE_PRIVATE_SENTINEL");
        private_reply.role = MessageRole::Assistant;
        db.append_message(&old.view_session_id, &private_reply)
            .unwrap();
        db.set_assistant_im_grant(
            SessionGrant {
                session_id: "work".into(),
                revision: 1,
                query_status: true,
                delegate: false,
                stop: false,
                completion_notice: false,
                expires_at: now + 3600,
            },
            now,
        )
        .unwrap();
        assert!(db
            .assistant_im_turn_targets(&old.view_session_id, &old.request_id, now)
            .is_err());
        let lease = db.assistant_im_poll_lease(now).unwrap().unwrap();
        db.receive_assistant_im_update(
            &lease,
            TelegramInput {
                update_id: 4,
                user_id: 42,
                chat_id: 42,
                sent_at: Some(now),
                kind: TelegramInputKind::Message("Status only".into()),
            },
            now,
        )
        .unwrap();
        let next = db
            .assistant_im_pending_commands()
            .unwrap()
            .into_iter()
            .find(|command| command.request_id != old.request_id)
            .unwrap();
        let next = db
            .prepare_assistant_im_context(&next.request_id, now)
            .unwrap();
        assert_ne!(old.view_session_id, next.view_session_id);
        assert!(db
            .load_session_messages(&next.view_session_id)
            .unwrap()
            .is_empty());
        db.revoke_assistant_im_pairing().unwrap();
        assert!(db
            .assistant_im_turn_targets(&next.view_session_id, &next.request_id, now)
            .is_err());
    });
}

#[test]
fn personal_assistant_im_completion_is_status_only_once_and_pause_cannot_replay() {
    isolated(|db, now| {
        let context = command(&db, now);
        admit(&db, &context);
        db.set_assistant_im_notifications_paused(false).unwrap();
        let request = db
            .enqueue_personal_assistant_request(AssistantDispatchInput {
                source_turn_id: context.request_id.clone(),
                item_key: "send-complete".into(),
                target_session_id: "work".into(),
                instruction: "PRIVATE_OWNER_INSTRUCTION".into(),
                global_stop_epoch: db.global_stop_epoch().unwrap(),
            })
            .unwrap();
        let mut user = NewMessage::user("PRIVATE_OWNER_INSTRUCTION")
            .with_source(crate::chat_engine::ChatSource::SessionTool);
        user.attachments_meta =
            Some(serde_json::json!({"personal_assistant_request_id":request.id}).to_string());
        db.append_message_and_create_session_tool_turn_with_id(
            "completion-target",
            "work",
            Some(&context.home_session_id),
            &user,
            request.global_stop_epoch,
        )
        .unwrap();
        let mut answer = NewMessage::user("PRIVATE_TARGET_RESULT_SENTINEL");
        answer.role = MessageRole::Assistant;
        let result = db.append_message("work", &answer).unwrap();
        db.finish_chat_turn_once(
            &context.request_id,
            ChatTurnStatus::Completed,
            None,
            None,
            None,
        )
        .unwrap();
        db.settle_assistant_im_command(&context.request_id, CommandTerminal::Completed)
            .unwrap();
        db.finish_chat_turn_once(
            "completion-target",
            ChatTurnStatus::Completed,
            None,
            None,
            Some(result),
        )
        .unwrap();
        assert!(db
            .enqueue_assistant_im_request_completion(&request.id, now)
            .unwrap());
        assert!(!db
            .enqueue_assistant_im_request_completion(&request.id, now)
            .unwrap());
        db.with_conn_for_test(|conn| {
            let (body, count): (String, i64) = conn.query_row(
                "SELECT text,COUNT(*) FROM assistant_im_deliveries WHERE request_id=?1",
                [format!("{}:complete", request.id)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            assert_eq!(count, 1);
            assert!(body.contains("已完成"));
            assert!(!body.contains("PRIVATE_"));
            Ok(())
        })
        .unwrap();
        db.set_assistant_im_notifications_paused(true).unwrap();
        db.set_assistant_im_notifications_paused(false).unwrap();
        assert!(!db
            .enqueue_assistant_im_request_completion(&request.id, now)
            .unwrap());
        assert!(db.prepare_assistant_im_delivery(now).unwrap().is_none());
        db.revoke_assistant_im_grant("work").unwrap();
        assert!(!db
            .enqueue_assistant_im_request_completion(&request.id, now)
            .unwrap());
    });
}

#[test]
fn personal_assistant_im_view_scope_and_memory_are_fixed_but_deletion_is_allowed() {
    isolated(|db, now| {
        let context = command(&db, now);
        db.with_conn_for_test(|conn| {
            assert!(conn
                .execute(
                    "UPDATE sessions SET kind='regular' WHERE id=?1",
                    [&context.view_session_id]
                )
                .is_err());
            assert!(conn
                .execute(
                    "UPDATE session_memory_policy SET use_memories='allow' WHERE session_id=?1",
                    [&context.view_session_id]
                )
                .is_err());
            assert!(conn
                .execute(
                    "DELETE FROM session_memory_policy WHERE session_id=?1",
                    [&context.view_session_id]
                )
                .is_err());
            conn.execute(
                "DELETE FROM sessions WHERE id=?1",
                [&context.view_session_id],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(db.get_session(&context.view_session_id).unwrap().is_none());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut agent = crate::agent::AssistantAgent::new_anthropic("test-key");
        agent.set_tool_scope(Some(crate::tool_defs::ToolScope::PersonalAssistantIm));
        assert_eq!(
            agent.build_full_system_prompt("model", "provider"),
            SYSTEM_PROMPT
        );
        assert_eq!(
            runtime.block_on(agent.prepare_full_system_prompt("model", "provider")),
            SYSTEM_PROMPT
        );
        agent.set_session_db(db.clone());
        agent.set_session_id(&context.view_session_id);
        assert_eq!(
            runtime
                .block_on(agent.prepare_full_system_prompt("fallback-model", "fallback-provider")),
            SYSTEM_PROMPT
        );
    });
}

#[test]
fn personal_assistant_im_deleted_view_drops_pending_reply_without_blocking_later_commands() {
    isolated(|db, now| {
        let context = command(&db, now);
        admit(&db, &context);
        db.finish_chat_turn_once(
            &context.request_id,
            ChatTurnStatus::Completed,
            None,
            None,
            None,
        )
        .unwrap();
        db.with_conn_for_test(|conn| {
            conn.execute(
                "UPDATE assistant_im_commands SET state='reply_pending' WHERE request_id=?1",
                [&context.request_id],
            )?;
            conn.execute(
                "DELETE FROM sessions WHERE id=?1",
                [&context.view_session_id],
            )?;
            Ok(())
        })
        .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(queue_terminal_reply(db.clone(), context.request_id))
            .unwrap();
        assert!(db
            .assistant_im_commands_awaiting_reply()
            .unwrap()
            .is_empty());
        assert!(db.prepare_assistant_im_delivery(now).unwrap().is_none());
    });
}

#[test]
fn personal_assistant_im_removed_connection_keeps_pairing_history_private_without_home() {
    isolated(|db, now| {
        assert!(!db.has_personal_assistant_private_data().unwrap());
        db.save_assistant_im_connection(
            701,
            "new_private_bot",
            &uuid::Uuid::new_v4().to_string(),
            now,
        )
        .unwrap();
        let code = db.begin_assistant_im_pairing(now).unwrap();
        let lease = db.assistant_im_poll_lease(now).unwrap().unwrap();
        db.receive_assistant_im_update(
            &lease,
            TelegramInput {
                update_id: 1,
                user_id: 42,
                chat_id: 42,
                sent_at: Some(now),
                kind: TelegramInputKind::PairCode(code.code),
            },
            now,
        )
        .unwrap();
        db.confirm_assistant_im_pairing(lease.generation, 42, 42, now)
            .unwrap();
        db.remove_assistant_im_connection().unwrap();
        assert!(db.assistant_im_connection().unwrap().is_none());
        db.with_conn_for_test(|conn| {
            assert_eq!(conn.query_row("SELECT COUNT(*) FROM sessions WHERE kind IN ('personal_assistant','personal_assistant_im')", [], |row| row.get::<_,i64>(0))?,0);
            assert_eq!(conn.query_row("SELECT COUNT(*) FROM assistant_im_activations", [], |row| row.get::<_,i64>(0))?,1);
            Ok(())
        }).unwrap();
        assert!(db.has_personal_assistant_private_data().unwrap());
    });
}

#[test]
fn personal_assistant_im_sealed_source_admits_durable_identity_and_forces_scope() {
    isolated(|db, now| {
        let context = command(&db, now);
        admit(&db, &context);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let request = crate::turn_kernel::TurnRequest::new(
            context.view_session_id.clone(),
            crate::agent_loader::DEFAULT_AGENT_ID.to_owned(),
            "Status".into(),
            db.clone(),
            Default::default(),
            Arc::new(AtomicBool::new(false)),
            Arc::new(crate::chat_engine::NoopEventSink),
        )
        .with_turn_id(context.request_id.clone());
        let admitted = runtime
            .block_on(
                crate::chat_engine::turn_kernel::admit_assistant_im_source_for_test(
                    request,
                    context.request_id.clone(),
                ),
            )
            .unwrap();
        assert_eq!(admitted.source, crate::chat_engine::ChatSource::Channel);
        assert_eq!(
            admitted.tool_scope,
            Some(crate::tool_defs::ToolScope::PersonalAssistantIm)
        );
        assert!(!admitted.post_turn_effects);
        assert!(!admitted.auto_approve_tools);
        assert!(admitted.channel_kb_context.is_none());
        db.revoke_assistant_im_grant("work").unwrap();
        let request = crate::turn_kernel::TurnRequest::new(
            context.view_session_id,
            crate::agent_loader::DEFAULT_AGENT_ID.to_owned(),
            "Status".into(),
            db,
            Default::default(),
            Arc::new(AtomicBool::new(false)),
            Arc::new(crate::chat_engine::NoopEventSink),
        )
        .with_turn_id(context.request_id.clone());
        assert!(runtime
            .block_on(
                crate::chat_engine::turn_kernel::admit_assistant_im_source_for_test(
                    request,
                    context.request_id
                )
            )
            .is_err());
    });
}
