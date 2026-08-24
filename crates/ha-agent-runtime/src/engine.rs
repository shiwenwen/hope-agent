//! Feature-owned main-turn engine and failover/finalization machine.

use std::sync::Arc;

use anyhow::Context;

use ha_core::failover::{
    self,
    executor::{execute_with_failover_observed, ExecutorError, FailoverPolicy, RetryProgress},
};
use ha_core::provider::{ApiType, AuthProfile};
use ha_core::session;
use ha_core::turn_durability::{FlushReason, TurnDurabilitySink};

use super::streaming_loop::CurrentUserMessageState;
use ha_core::chat_engine::context::*;
use ha_core::chat_engine::finalize::{self, PartialMeta, TerminationReason};
use ha_core::chat_engine::*;
use ha_core::chat_engine::{stream_broadcast, stream_seq};

const CHAT_CANCEL_COOPERATIVE_GRACE: std::time::Duration = std::time::Duration::from_secs(6);
const CHAT_CANCELLED_BY_CALLER: &str = "chat cancelled by caller";

fn claim_non_cancelled_terminal(
    completion_claim: Option<&ha_core::chat_engine::TurnCompletionClaim>,
    cancel: &Arc<std::sync::atomic::AtomicBool>,
) -> bool {
    let claimed = completion_claim
        .map(ha_core::chat_engine::TurnCompletionClaim::try_claim)
        .unwrap_or(true);
    if !claimed {
        // Fail closed even if a future source adapter violates the callback
        // contract and rejects without first publishing its shared token.
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    claimed
}

pub async fn execute_admitted_params(
    params: ChatEngineParams,
) -> Result<ChatEngineResult, TurnFailure> {
    let ChatEngineParams {
        session_id,
        agent_id,
        turn_id,
        pre_admitted_stream,
        active_turn_guard: _active_turn_guard,
        ui_surface: _ui_surface,
        mut message,
        incoming_turn,
        display_text,
        mut attachments,
        session_db: db,
        model_chain,
        providers,
        config_revision: _,
        codex_token,
        resolved_temperature,
        compact_config,
        run_context,
        reasoning_effort,
        cancel,
        completion_claim,
        foreground_stop_admission,
        plan_context_override,
        mut skill_allowed_tools,
        denied_tools,
        tool_scope,
        subagent_depth,
        steer_run_id,
        auto_approve_tools,
        follow_global_reasoning_effort,
        post_turn_effects,
        abort_on_cancel,
        persist_final_error_event,
        source,
        origin_source,
        channel_kb_context,
        event_sink,
    } = params;

    // Atomically register execution against the lifecycle gate. Every desktop,
    // HTTP, channel, ACP, subagent, and parent-injection path must fail closed
    // once an Agent is disabled, and deletion must see admitted work even
    // before its durable activity rows have been written.
    let _agent_run_guard =
        ha_core::agent_lifecycle::begin_agent_run(&agent_id).map_err(|error| error.to_string())?;

    // Effective KB-access origin for this turn (design D10): top-level turns
    // have origin == source; a subagent carries its parent turn's origin so an
    // IM-origin chain can't reacquire KB access via the neutral Subagent source.
    let kb_origin = origin_source.unwrap_or_else(|| kb_access_source(source));

    // Typed skill discovery is scoped to this session's effective workspace.
    // A daemon may serve unrelated projects concurrently, so the process cwd
    // and process-global DB are not valid substitutes here.
    let has_typed_skill_mentions = incoming_turn.as_ref().is_some_and(|wire| {
        wire.mentions
            .iter()
            .any(|mention| mention.kind == ha_core::prompt_context::MentionKind::Skill)
    });
    let skill_working_dir = if has_typed_skill_mentions {
        let snapshot_db = db.clone();
        let snapshot_session_id = session_id.clone();
        ha_core::blocking::run_blocking(move || {
            let session = snapshot_db
                .get_session(&snapshot_session_id)?
                .with_context(|| "typed skill mention session no longer exists")?;
            anyhow::Ok(ha_core::session::effective_working_dir_for_meta(&session))
        })
        .await
        .map_err(|error| format!("Cannot resolve typed skill workspace: {error}"))?
    } else {
        None
    };

    // Freeze the typed composer contract once, before any provider/profile
    // attempt. Every failover serializes the same resolved bindings and user
    // envelope; no attempt re-reads display labels or reparses pasted tokens.
    let canonical_user_message = message.clone();
    validate_engine_typed_resource_boundary(
        &canonical_user_message,
        incoming_turn.as_ref(),
        &attachments,
    )?;
    let (
        mut turn_context_builder,
        agent_binding_refs,
        mut mention_receipts,
        mention_wire_version,
        mut legacy_compatibility,
    ) = if let Some(ref wire) = incoming_turn {
        let (builder, bindings, receipts) = ha_core::prompt_context::resolve_typed_turn_context(
            &canonical_user_message,
            wire,
            &session_id,
            turn_id.as_deref(),
            &agent_id,
        )
        .map_err(|error| format!("Invalid typed mention context: {error}"))?;
        (
            builder,
            bindings,
            receipts,
            Some(wire.mention_wire_version),
            false,
        )
    } else {
        (
            ha_core::prompt_context::TurnContextBuilder::default(),
            Vec::new(),
            Vec::new(),
            None,
            true,
        )
    };

    // A typed file binding is only resolved when the same turn also carries a
    // matching attachment. Resolve the canonical target beneath the session
    // working directory and read its bytes exactly once. This phase is
    // deliberately read-only: durable publication happens only after the
    // stream run exists and owns a staged materialization journal fact.
    let mut prepared_resource_mentions = None;
    if let Some(ref wire) = incoming_turn {
        let file_targets = wire
            .mentions
            .iter()
            .filter(|mention| mention.kind == ha_core::prompt_context::MentionKind::File)
            .map(|mention| mention.target_id.clone())
            .collect::<Vec<_>>();
        let plan_targets = wire
            .mentions
            .iter()
            .filter(|mention| mention.kind == ha_core::prompt_context::MentionKind::Plan)
            .map(|mention| mention.target_id.clone())
            .collect::<Vec<_>>();
        if !file_targets.is_empty() || !plan_targets.is_empty() {
            let snapshot_db = db.clone();
            let snapshot_session_id = session_id.clone();
            let snapshot_attachments = attachments;
            let (prepared_attachments, session_incognito, prepared) =
                ha_core::blocking::run_blocking(move || {
                    let session = snapshot_db
                        .get_session(&snapshot_session_id)?
                        .with_context(|| "typed resource mention session no longer exists")?;
                    let prepared = prepare_typed_resource_mentions_for_session(
                        &session,
                        &file_targets,
                        &plan_targets,
                        &snapshot_attachments,
                    )?;
                    anyhow::Ok((snapshot_attachments, session.incognito, prepared))
                })
                .await
                .map_err(|error| format!("Cannot freeze typed resource mentions: {error}"))?;
            attachments = prepared_attachments;
            prepared_resource_mentions = Some((session_incognito, prepared));
        }
    }
    if let Some(ref wire) = incoming_turn {
        let skill_ids = wire
            .mentions
            .iter()
            .filter(|mention| {
                mention.kind == ha_core::prompt_context::MentionKind::Skill
                    && mention.origin
                        != ha_core::prompt_context::StructuredMentionOrigin::SlashCommandAst
            })
            .map(|mention| mention.target_id.clone())
            .collect::<Vec<_>>();
        if !skill_ids.is_empty() {
            let activation = require_explicit_mention_skill_activation(
                &skill_ids,
                ha_core::skills_hooks::resolve_named_skill_mentions(
                    &skill_ids,
                    Some(&agent_id),
                    skill_working_dir.as_deref().map(std::path::Path::new),
                ),
            )
            .map_err(|error| format!("Invalid typed mention context: {error}"))?;
            turn_context_builder.user_instruction(
                ha_core::prompt_context::UserInstructionSource::ExplicitSkillMention,
                activation.content,
            );
            merge_explicit_skill_ceiling(&mut skill_allowed_tools, activation.tool_ceiling.clone());
            for receipt in &mut mention_receipts {
                if receipt.kind == ha_core::prompt_context::MentionKind::Skill
                    && activation
                        .resolved_names
                        .iter()
                        .any(|name| name == &receipt.target_id)
                {
                    receipt.status = ha_core::prompt_context::MentionResolutionStatus::Resolved;
                } else if receipt.kind == ha_core::prompt_context::MentionKind::Skill
                    && activation
                        .rejected_names
                        .iter()
                        .any(|name| name == &receipt.target_id)
                {
                    receipt.status = ha_core::prompt_context::MentionResolutionStatus::Rejected;
                }
            }
        }

        // Slash skills use the same typed binding/receipt channel but are not
        // restricted to the composer's curated @skill allowlist. Re-resolve
        // the canonical skill id against the live invocable catalog and render
        // it here, so a client cannot smuggle prompt content or tool grants.
        let slash_skill_mentions = wire
            .mentions
            .iter()
            .filter(|mention| {
                mention.kind == ha_core::prompt_context::MentionKind::Skill
                    && mention.origin
                        == ha_core::prompt_context::StructuredMentionOrigin::SlashCommandAst
            })
            .collect::<Vec<_>>();
        if !slash_skill_mentions.is_empty() {
            let cfg = ha_core::config::cached_config();
            let env_check = ha_core::skills::skill_env_check_enabled_for_agent(
                Some(&agent_id),
                cfg.skill_env_check,
            );
            let skill_env = cfg.skill_env.clone();
            let entries = ha_core::skills_hooks::invocable_skills(
                &cfg.extra_skills_dirs,
                &cfg.disabled_skills,
                skill_working_dir.as_deref().map(std::path::Path::new),
            );
            // Typed ownership must be rebuilt from the same globally surfaced
            // catalog as list/help/dispatch. Agent-specific requirement checks
            // still run after the collision-resolved binding is matched.
            let entries = ha_core::skills::filter_catalog_eligible_skills(
                entries,
                cfg.skill_env_check,
                &cfg.skill_env,
            );
            drop(cfg);

            for mention in slash_skill_mentions {
                let Some(entry) = resolve_slash_skill_binding(
                    &entries,
                    &mention.target_id,
                    &mention.display_label,
                ) else {
                    return Err(format!(
                        "Invalid typed mention context: slash command '{}' is not owned by skill '{}'",
                        mention.display_label, mention.target_id
                    )
                    .into());
                };
                ensure_explicit_slash_skill_requirements(entry, env_check, &skill_env)
                    .map_err(|error| format!("Invalid typed mention context: {error}"))?;
                let args =
                    ha_core::prompt_context::slash_skill_args(&canonical_user_message, mention);
                let rendered = match args.as_deref() {
                    Some(args) => {
                        match ha_core::skills::resolve_skill_slash_dispatch(entry, args) {
                            ha_core::skills::SkillSlashDispatch::ModelTemplate { message } => {
                                Ok(message)
                            }
                            ha_core::skills::SkillSlashDispatch::ModelInline => {
                                ha_core::skills_hooks::render_skill_inline(entry, args).await
                            }
                            ha_core::skills::SkillSlashDispatch::Fork
                            | ha_core::skills::SkillSlashDispatch::Tool => Err(anyhow::anyhow!(
                                "typed slash binding targets a non-model Skill dispatch"
                            )),
                        }
                    }
                    None => Err(anyhow::anyhow!(
                        "validated slash command binding has no canonical arguments"
                    )),
                };
                let activation =
                    require_explicit_slash_skill_materialization(entry, args, rendered)
                        .map_err(|error| format!("Invalid typed mention context: {error}"))?;
                turn_context_builder.user_instruction(
                    ha_core::prompt_context::UserInstructionSource::ExplicitSlashSkill,
                    activation.content,
                );
                merge_explicit_skill_ceiling(&mut skill_allowed_tools, activation.tool_ceiling);
                if let Some(receipt) = mention_receipts
                    .iter_mut()
                    .find(|receipt| receipt.mention_id == mention.id)
                {
                    receipt.status = ha_core::prompt_context::MentionResolutionStatus::Resolved;
                }
            }
        }
    }

    if model_chain.is_empty() {
        return Err("No model configured for chat execution".to_string().into());
    }

    // Resolve the Plan-mode bundle once at turn start. Spawn-supplied
    // overrides win (their child sessions have backend `plan_mode = Off`
    // even though they're meant to run as PlanAgent); otherwise read this
    // session's backend state. The `plan_context_locked` flag rides along
    // so configure_agent picks the right setter and the streaming loop's
    // mid-turn probe knows whether to leave the bundle alone.
    //
    // Plan's fixed platform contract and user/model-authored document occupy
    // separate slots. A mid-turn state flip can replace both without losing
    // caller framing, and adapters keep the document out of developer roles.
    let plan_context_locked = plan_context_override.is_some();
    let plan_resolved = match plan_context_override {
        Some(o) => o,
        None => ha_core::chat_engine::resolve_plan_context_for_session(&session_id).await,
    };

    let mut stream_lifecycle = match pre_admitted_stream.as_ref() {
        Some(admitted) => StreamLifecycle::from_admission(
            &session_id,
            source,
            turn_id.clone(),
            admitted.stream_id.clone(),
        ),
        None => StreamLifecycle::begin(&session_id, source, turn_id.clone())?,
    };

    // Every conversation-producing entry receives a persistence run, even
    // when it has no user-visible chat_turn id. Incognito registrations stay
    // memory-only inside the coordinator.
    let durability_result = match pre_admitted_stream {
        Some(admitted) => {
            ha_core::chat_engine::durability::StreamCoordinator::from_admission(
                db.clone(),
                session_id.clone(),
                source,
                Some(admitted.stream_id),
                turn_id.clone(),
                event_sink.clone(),
                cancel.clone(),
                admitted.registration,
            )
            .await
        }
        None => {
            ha_core::chat_engine::durability::StreamCoordinator::create(
                db.clone(),
                session_id.clone(),
                source,
                stream_lifecycle.stream_id.clone(),
                turn_id.clone(),
                event_sink.clone(),
                cancel.clone(),
                foreground_stop_admission,
            )
            .await
        }
    };
    let durability = match durability_result {
        Ok(coordinator) => coordinator,
        Err(error) => {
            let message = format!("Cannot initialize durable chat stream: {error}");
            let stopped_by_fence = error
                .to_string()
                .contains(session::FOREGROUND_STOP_FENCE_ERROR);
            let terminal_status = if stopped_by_fence {
                session::ChatTurnStatus::Interrupted
            } else {
                session::ChatTurnStatus::Failed
            };
            let interrupt_reason = if stopped_by_fence {
                session::ChatTurnInterruptReason::UserStop
            } else {
                session::ChatTurnInterruptReason::Unknown
            };
            if let Some(turn_id) = turn_id.as_deref() {
                if let Err(finish_error) = db.finish_chat_turn_once(
                    turn_id,
                    terminal_status,
                    Some(interrupt_reason),
                    Some(&message),
                    None,
                ) {
                    app_error!(
                        "chat",
                        "stream_durability",
                        "failed to converge turn {} after coordinator initialization error: {}",
                        turn_id,
                        finish_error
                    );
                }
            }
            stream_lifecycle.set_terminal(
                terminal_status,
                Some(interrupt_reason),
                Some(message.clone()),
            );
            stream_lifecycle.finish();
            return Err(if stopped_by_fence {
                TurnFailure::cancelled(message)
            } else {
                message.into()
            });
        }
    };
    stream_lifecycle
        .arm_abandoned_recovery(db.clone(), durability.persistence_run_id().to_string());

    // Network-capable OAuth refresh and title generation are downstream of
    // durable admission/manual-retry convergence. Nothing may issue side
    // requests while an earlier SendUnknown request is still unresolved.
    let chain_needs_codex = model_chain.iter().any(|m| {
        providers
            .iter()
            .any(|p| p.id == m.provider_id && p.api_type == ApiType::Codex)
    });
    let mut codex_token = codex_token;
    if chain_needs_codex {
        let current = codex_token.as_ref().map(|(t, _)| t.as_str()).unwrap_or("");
        if let Some(pair) = ha_core::oauth::ensure_fresh_codex_token(current).await {
            codex_token = Some(pair);
        }
    }

    {
        let title_db = db.clone();
        let title_session_id = session_id.clone();
        let title_agent_id = agent_id.clone();
        let title_model = model_chain[0].clone();
        ha_core::blocking::run_blocking(move || {
            ha_core::session_title::maybe_schedule_autonomous_start(
                title_db,
                title_session_id,
                title_agent_id,
                title_model,
            )
        })
        .await;
    }

    // Durable basenames are owned by the already-persisted stream run. Crash
    // recovery reconciles this exact backend UUID prefix against every
    // durable Initial Context event for the run; Incognito writes no file.
    let mut durable_snapshot_names = Vec::new();
    if let Some((session_incognito, prepared)) = prepared_resource_mentions.as_mut() {
        if !*session_incognito {
            prepared.bind_persistence_run(durability.persistence_run_id())?;
            durable_snapshot_names = prepared.durable_snapshot_names()?;
        }
    }
    if !durable_snapshot_names.is_empty() {
        // Ownership must be durable before filesystem publication. The ledger
        // deliberately survives later run deletion, letting GC/edit retries
        // unlink the exact backend-minted basenames before acknowledging them.
        let ownership_db = db.clone();
        let ownership_run_id = durability.persistence_run_id().to_string();
        let ownership_session_id = session_id.clone();
        let ownership_snapshot_names = durable_snapshot_names.clone();
        ownership_db
            .run(move |db| {
                db.register_typed_resource_snapshots(
                    &ownership_run_id,
                    &ownership_session_id,
                    &ownership_snapshot_names,
                )
            })
            .await
            .map_err(|error| format!("Cannot register typed resource snapshots: {error}"))?;
    }

    let (published_attachments, frozen_resource_mentions) =
        if let Some((session_incognito, prepared)) = prepared_resource_mentions.take() {
            let publication_db = db.clone();
            let publication_run_id = durability.persistence_run_id().to_string();
            let snapshot_session_id = session_id.clone();
            let publication_snapshot_names = durable_snapshot_names;
            let mut snapshot_attachments = attachments;
            publication_db
                .run(move |db| {
                    let publish_files = || {
                        ha_core::attachments::publish_typed_resource_snapshot_files(
                            &snapshot_session_id,
                            prepared,
                            session_incognito,
                        )
                    };
                    let published = if session_incognito {
                        publish_files()?
                    } else {
                        db.publish_registered_typed_resource_snapshots(
                            &publication_run_id,
                            &snapshot_session_id,
                            &publication_snapshot_names,
                            publish_files,
                        )?
                    };
                    let frozen = ha_core::attachments::finalize_typed_resource_mentions(
                        published,
                        &mut snapshot_attachments,
                    );
                    anyhow::Ok((snapshot_attachments, frozen))
                })
                .await
                .map_err(|error| format!("Cannot publish typed resource mentions: {error}"))?
        } else {
            (attachments, Vec::new())
        };
    attachments = published_attachments;
    let frozen_resource_mentions = Arc::new(frozen_resource_mentions);
    let snapshot_names = frozen_resource_mentions
        .iter()
        .filter_map(|snapshot| snapshot.snapshot_name.clone())
        .collect::<Vec<_>>();
    let snapshot_refs_committed = Arc::new(std::sync::atomic::AtomicBool::new(
        snapshot_names.is_empty(),
    ));
    let _pending_snapshot_cleanup = PendingTypedResourceSnapshots {
        session_id: session_id.clone(),
        snapshot_names,
        refs_committed: snapshot_refs_committed.clone(),
    };

    let context_resource_turn_budget =
        Arc::new(ha_core::prompt_context::ContextResourceTurnBudget::default());
    let context_resource_refs = frozen_resource_mentions
        .iter()
        .filter_map(|snapshot| {
            let mention_id = incoming_turn.as_ref()?.mentions.iter().find(|mention| {
                matches!(
                    mention.kind,
                    ha_core::prompt_context::MentionKind::File
                        | ha_core::prompt_context::MentionKind::Plan
                ) && mention.target_id == snapshot.target_id
            })?;
            turn_context_builder.untrusted_data(
                ha_core::prompt_context::UntrustedDataSource::FileAttachment,
                serde_json::json!({
                    "mentionId": mention_id.id,
                    "resourceRef": snapshot.resource_ref,
                    "path": snapshot.target_id,
                    "sourceBytes": snapshot.source_bytes,
                    "continuationTool": ha_core::tool_defs::TOOL_READ_CONTEXT_RESOURCE,
                })
                .to_string(),
            );
            Some(ha_core::prompt_context::ContextResourceRef {
                resource_ref: snapshot.resource_ref.clone(),
                mention_id: mention_id.id.clone(),
                target_id: snapshot.target_id.clone(),
                file_name: snapshot.file_name.clone(),
                mime_type: snapshot.mime_type.clone(),
                parent_session_id: session_id.clone(),
                parent_turn_id: turn_id.clone(),
                principal_agent_id: agent_id.clone(),
                bytes: snapshot.bytes.clone(),
                turn_budget: context_resource_turn_budget.clone(),
            })
        })
        .collect::<Vec<_>>();

    if incoming_turn.is_some() {
        for receipt in &mut mention_receipts {
            if !matches!(
                receipt.kind,
                ha_core::prompt_context::MentionKind::File
                    | ha_core::prompt_context::MentionKind::Plan
            ) {
                continue;
            }
            let snapshot = frozen_resource_mentions
                .iter()
                .find(|snapshot| snapshot.target_id == receipt.target_id);
            receipt.status = if snapshot.is_some() {
                ha_core::prompt_context::MentionResolutionStatus::Resolved
            } else {
                ha_core::prompt_context::MentionResolutionStatus::Unavailable
            };
            receipt.materialization = snapshot.map(|snapshot| {
                ha_core::prompt_context::MentionMaterialization::FrozenSnapshot {
                    source_bytes: snapshot.source_bytes,
                    persistence: if snapshot.durable {
                        ha_core::prompt_context::ContextPersistence::DurableSnapshot
                    } else {
                        ha_core::prompt_context::ContextPersistence::IncognitoMemoryOnly
                    },
                }
            });
        }
    }

    // Wrap attachments in Arc<[T]> only after the staged typed-resource batch
    // has been published and its attachment paths/data have been frozen.
    // Failover closure clones are then pointer bumps even for MB-sized data.
    let attachments: std::sync::Arc<[ha_core::agent::Attachment]> =
        std::sync::Arc::from(attachments);

    // Idle/busy tracking (R2 — §5.4 fix). Mark this session active for the whole
    // turn so background-job / sub-agent completion injection yields to the live
    // turn instead of splicing into it. Created here at the shared engine entry
    // so all foreground-policy sources are covered uniformly — desktop, HTTP,
    // ACP, IM channel, cron, and SessionTool. Previously only the
    // Tauri shell created the guard (`commands/chat.rs`), so on server / IM the
    // gate `ACTIVE_CHAT_SESSIONS` stayed at 0 and injection fired immediately
    // against a running turn. The Tauri shell keeps its own earlier guard (to
    // cancel an in-flight injection the moment the user hits send, before this
    // turn's preflight); the refcount in `ChatSessionGuard` makes the overlap
    // safe — the engine guard drops first, the shell guard last, so idle/flush
    // fires exactly once after the whole command. `ParentInjection` / `Subagent`
    // are excluded by `holds_foreground_idle_guard` (the former is the injection
    // itself; the latter is a distinct child session). ACP now enters through
    // this same boundary.
    let _idle_guard = source
        .holds_foreground_idle_guard()
        .then(|| ha_core::subagent::ChatSessionGuard::new(&session_id));

    if let (Some(ref turn_id), Some(ref stream_id)) =
        (turn_id.as_ref(), stream_lifecycle.stream_id.as_ref())
    {
        let _ = ha_core::chat_engine::active_turn::set_stream_id(&session_id, turn_id, stream_id);
        if let Err(e) = db.update_chat_turn_stream_id(turn_id, stream_id) {
            app_warn!(
                "chat",
                "turn",
                "Failed to persist stream id for turn {}: {}",
                turn_id,
                e
            );
        }
        if source.broadcasts_to_user_ui() {
            stream_broadcast::broadcast_turn_started(&session_id, turn_id, Some(stream_id));
        }
    }

    // SessionStart hook (startup / resume). Observation output is frozen into
    // this turn's untrusted data envelope and survives failover retries (which
    // rebuild the agent from this same local). ACP also enters this engine, so
    // every interactive source fires SessionStart and resolves cwd identically.
    //
    // Gate on `source.fires_user_lifecycle_hooks()`: subagent / parent-injection
    // runs are internal workers, not user-visible sessions, so they MUST NOT
    // fire SessionStart. Without this gate an `agent` handler on `SessionStart`
    // spawns a sub-agent on every run, whose own chat-engine pass fires another
    // `SessionStart` (new session id ⇒ per-session `claim_session_start` doesn't
    // dedupe), and so on — a single global SessionStart agent hook would burn
    // tokens until concurrency or external limits intervene. Subagent
    // observability lives on `SubagentStart` / `SubagentStop` instead, also
    // gated against hook-spawned children in `subagent::spawn`.
    if source.fires_user_lifecycle_hooks() {
        if let Some(extra) = ha_core::hooks::fire_session_start_observation(
            &session_id,
            &agent_id,
            model_chain
                .first()
                .map(|m| m.model_id.as_str())
                .unwrap_or_default(),
        )
        .await
        {
            turn_context_builder.untrusted_data(
                ha_core::prompt_context::UntrustedDataSource::HookContext,
                extra,
            );
        }
    }

    // UserPromptSubmit hook context: the preflight chokepoint stashed any
    // `additionalContext` from the UserPromptSubmit hook keyed by session;
    // drain it here so it rides this turn's user-owned context next to SessionStart
    // (and survives failover for the same reason — it lives in this run-local).
    // Drained exactly once per turn.
    if let Some(extra) = ha_core::hooks::take_user_prompt_context(&session_id) {
        turn_context_builder.untrusted_data(
            ha_core::prompt_context::UntrustedDataSource::HookContext,
            extra,
        );
    }

    // Knowledge read bridge channel ① (D7): deterministically inject notes the
    // user referenced inline with `[[ ]]`, scoped by `effective_kb_access` (D10)
    // and wrapped as untrusted external data (#7). Skipped for incognito inside
    // the resolver (zero KB access).
    let bound_notes = incoming_turn
        .as_ref()
        .map(ha_core::prompt_context::bound_note_refs)
        .unwrap_or_default();
    let typed_notes = if !bound_notes.is_empty() {
        let per_note_budget = typed_note_byte_budget(&model_chain, &providers, bound_notes.len());
        ha_core::knowledge_hooks::resolve_bound_notes(
            &bound_notes,
            &session_id,
            kb_access_source(source),
            kb_origin,
            channel_kb_context.clone(),
            per_note_budget,
        )
    } else {
        None
    };
    let legacy_note_message = incoming_turn
        .as_ref()
        .map(|wire| message_without_typed_note_spans(&canonical_user_message, wire))
        .unwrap_or_else(|| canonical_user_message.clone());
    let legacy_note_slots = 5usize.saturating_sub(bound_notes.len().min(5));
    let legacy_notes = (legacy_note_slots > 0)
        .then(|| {
            ha_core::knowledge_hooks::resolve_inline_injections(
                &legacy_note_message,
                &session_id,
                kb_access_source(source),
                kb_origin,
                channel_kb_context.clone(),
                legacy_note_slots,
            )
        })
        .flatten();
    if legacy_notes.is_some() {
        // A current typed wire may legitimately coexist with the explicit
        // read-only `[[note]]` compatibility syntax (for example a typed
        // @agent plus a manually entered wikilink). Record that the legacy
        // parser actually contributed context.
        legacy_compatibility = true;
    }
    let referenced_notes = match (typed_notes, legacy_notes) {
        (Some(mut typed), Some(legacy)) => {
            typed.content.push_str("\n\n");
            typed.content.push_str(&legacy);
            Some(typed)
        }
        (Some(typed), None) => Some(typed),
        (None, Some(content)) => Some(ha_core::knowledge_hooks::ResolvedBoundNotes {
            content,
            resolved_refs: Vec::new(),
        }),
        (None, None) => None,
    };
    if let Some(resolved_notes) = referenced_notes {
        turn_context_builder.untrusted_data(
            ha_core::prompt_context::UntrustedDataSource::KnowledgeNote,
            resolved_notes.content,
        );
        for receipt in &mut mention_receipts {
            if receipt.kind == ha_core::prompt_context::MentionKind::Note
                && receipt
                    .target_id
                    .split_once("::")
                    .is_some_and(|(kb_id, rel_path)| {
                        resolved_notes.resolved_refs.iter().any(|resolved| {
                            resolved.kb_id == kb_id && resolved.rel_path == rel_path
                        })
                    })
            {
                receipt.status = ha_core::prompt_context::MentionResolutionStatus::Resolved;
                if let Some(resolved) = resolved_notes.resolved_refs.iter().find(|resolved| {
                    receipt
                        .target_id
                        .split_once("::")
                        .is_some_and(|(kb_id, rel_path)| {
                            resolved.kb_id == kb_id && resolved.rel_path == rel_path
                        })
                }) {
                    receipt.materialization =
                        Some(if resolved.source_bytes == resolved.delivered_bytes {
                            ha_core::prompt_context::MentionMaterialization::Complete {
                                source_bytes: resolved.source_bytes,
                                delivered_bytes: resolved.delivered_bytes,
                            }
                        } else {
                            ha_core::prompt_context::MentionMaterialization::Preview {
                                source_bytes: resolved.source_bytes,
                                delivered_bytes: resolved.delivered_bytes,
                                continuation_tool: ha_core::tools::TOOL_NOTE_READ.to_string(),
                            }
                        });
                }
            }
        }
    }

    // Raw/pasted markdown that merely resembles `@skill` or `@agent` remains
    // ordinary user text. Only a validated typed binding above may activate a
    // Skill or mint an opaque Agent reference. `[[note]]` is the deliberately
    // retained read-only legacy syntax handled by the knowledge bridge.

    ha_core::prompt_context::append_unresolved_mention_statuses(
        &mut turn_context_builder,
        &mention_receipts,
    );

    let resolved_turn_context = ha_core::prompt_context::finalize_turn_context(
        &canonical_user_message,
        turn_context_builder,
        agent_binding_refs,
        mention_wire_version,
        legacy_compatibility,
        mention_receipts,
    );
    message = resolved_turn_context.model_message.clone();
    let agent_binding_refs = resolved_turn_context.agent_bindings.clone();
    let prompt_context_receipt = std::sync::Arc::new(resolved_turn_context.receipt);

    // IM-mirror prefers the friendly `display_text` (e.g. `Using skill **X**...`
    // rendered for `/skill` invocations) so attached IM chats see what the
    // desktop user saw, not the internal structured turn envelope.
    // A normal Desktop / HTTP turn has a durable `turn_id`; the stream id is
    // the stable per-run fallback for internal callers that do not create a
    // chat-turn row. Pass it explicitly so the channel layer never has to
    // race the active-turn registry to infer this mirror generation.
    let im_mirror_generation = turn_id
        .clone()
        .map(ha_core::channel_hooks::ImLiveMirrorGeneration::Turn)
        .or_else(|| {
            stream_lifecycle
                .stream_id
                .clone()
                .map(ha_core::channel_hooks::ImLiveMirrorGeneration::Stream)
        })
        .unwrap_or_else(|| {
            ha_core::channel_hooks::ImLiveMirrorGeneration::Stream(
                durability.persistence_run_id().to_string(),
            )
        });
    let mut im_mirror = ha_core::channel_hooks::attach_live_mirror(
        &session_id,
        source,
        im_mirror_generation,
        Some(ha_core::channel_hooks::LastUserSnapshot {
            source: source.as_str().to_string(),
            text: ha_core::util::non_empty_trim_or(
                display_text.as_deref(),
                &canonical_user_message,
            )
            .to_owned(),
            attachment_count: attachments.len(),
        }),
    )
    .await;

    let total_models = model_chain.len();
    let mut last_error: Option<String> = None;
    // Preserve the executor's typed verdict from `ExecutorError::Exhausted`
    // so the IM mirror abort path can render a per-class friendly notice
    // (`🔐 Authentication failed`, `⏱️ Rate limited`, …). Re-classifying
    // `last_error` at the abort site is lossy — provider-specific
    // wrapping can drop the original 4xx/5xx markers that
    // `failover::classify_error` keys off.
    let mut last_reason: Option<failover::FailoverReason> = None;
    // Pinned to `true` only when the failing model's provider is Codex
    // *and* its failure reason is Auth — drives the "re-authorize via
    // desktop app" headline. Tracked per-failure rather than derived from
    // primary-only because the failover chain may have rotated through
    // multiple providers, and the user-facing hint depends on which one
    // actually erred.
    let mut last_is_codex_auth = false;
    // Set when emergency compaction was attempted but still failed to
    // bring history below the model's context window — promoted into
    // `TerminationReason::CompactionFailed` by `derive_termination_reason`
    // so the marker classifies the failure correctly instead of folding
    // it into a generic provider error.
    let mut compaction_failed: Option<String> = None;
    // True when the most recent model attempt bailed with
    // `ExecutorError::NoProfileAvailable`. We still fill `last_reason`
    // / `last_error` in that branch so logs include the model id, but
    // the unified finalize taxonomy needs to surface this as the
    // explicit `NoProfileAvailable` reason (not generic `ProviderFailed`)
    // so the user-facing copy can say "configure provider" instead of
    // "all models failed".
    let mut last_was_no_profile = false;

    // Build primary model display name for fallback events
    let primary_display = {
        let first = &model_chain[0];
        let prov_name = providers
            .iter()
            .find(|p| p.id == first.provider_id)
            .map(|p| p.name.as_str())
            .unwrap_or(&first.provider_id);
        format!("{} / {}", prov_name, first.model_id)
    };

    let effort_str = reasoning_effort.clone();

    // A complete second pass is reserved for timeout/unknown failures that may
    // self-heal after every configured model has had a chance. Rate-limit and
    // overload already consume the larger per-profile retry budget and rotate
    // keys; auth/billing/model-not-found are deterministic. Never replay a
    // whole chain after any tool boundary, where another pass could duplicate
    // an external side effect.
    const MAX_MODEL_CHAIN_ROUNDS: u32 = 2;
    const MODEL_CHAIN_RETRY_BASE_MS: u64 = 4_000;
    const MODEL_CHAIN_RETRY_MAX_MS: u64 = 10_000;
    let mut model_chain_round = 1_u32;
    let mut model_index = 0_usize;
    // The ordinary attempt base is the pre-turn context. Tier-4 recovery
    // adopts a compacted post-user checkpoint, after which retries and model
    // fallback must preserve rather than append the current user message.
    let mut current_user_message_state = CurrentUserMessageState::MissingFromHistory;

    loop {
        if model_index >= model_chain.len() {
            let can_retry_whole_chain = should_retry_model_chain(
                model_chain_round,
                MAX_MODEL_CHAIN_ROUNDS,
                last_reason,
                last_was_no_profile,
                compaction_failed.is_some(),
                durability.had_tool_activity(),
            ) && !cancel.load(std::sync::atomic::Ordering::SeqCst);
            if !can_retry_whole_chain {
                break;
            }

            let delay_ms = failover::retry_delay_ms(
                model_chain_round - 1,
                MODEL_CHAIN_RETRY_BASE_MS,
                MODEL_CHAIN_RETRY_MAX_MS,
            );
            let next_round = model_chain_round + 1;
            app_info!(
                "provider",
                "retry_chain",
                "Restarting model fallback chain for session {} (round {}/{}, delay={}ms)",
                session_id,
                next_round,
                MAX_MODEL_CHAIN_ROUNDS,
                delay_ms
            );
            let recovery_wait = ha_core::recovery_control::register(&session_id);
            if let Ok(json_str) = serde_json::to_string(&serde_json::json!({
                "type": "model_chain_retry",
                "reason": last_reason,
                "attempt": next_round,
                "total": MAX_MODEL_CHAIN_ROUNDS,
                "delay_ms": delay_ms,
                "recovery_id": recovery_wait.id(),
                "can_switch_model": false,
            })) {
                emit_stream_event(
                    &db,
                    &event_sink,
                    &session_id,
                    source,
                    turn_id.as_deref(),
                    &json_str,
                );
            }
            match recovery_wait
                .wait(std::time::Duration::from_millis(delay_ms), Some(&cancel))
                .await
            {
                ha_core::recovery_control::RecoveryWaitOutcome::Cancelled => {
                    last_reason = None;
                    last_error = Some(CHAT_CANCELLED_BY_CALLER.to_string());
                    break;
                }
                ha_core::recovery_control::RecoveryWaitOutcome::Elapsed
                | ha_core::recovery_control::RecoveryWaitOutcome::SkipWait
                | ha_core::recovery_control::RecoveryWaitOutcome::SwitchModel => {}
            }
            model_chain_round = next_round;
            model_index = 0;
            continue;
        }

        let idx = model_index;
        let model_ref = &model_chain[idx];
        let mut manual_model_switch = false;
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            last_error = Some(CHAT_CANCELLED_BY_CALLER.to_string());
            break;
        }
        // Look up provider once per model. Skip the model if missing — same
        // semantics as the pre-Phase-3 build_agent_from_snapshot None path.
        let current_provider = providers.iter().find(|p| p.id == model_ref.provider_id);
        let prov = match current_provider {
            Some(p) => p,
            None => {
                let msg = format!(
                    "Provider not found: {} for model {}",
                    model_ref.provider_id, model_ref.model_id
                );
                // A stale fallback is deterministic, but it must not erase a
                // transient failure from an earlier usable model. Reaching the
                // chain boundary should still give that model its bounded
                // second-round opportunity.
                last_reason = Some(chain_reason_after_missing_provider(last_reason));
                last_error = Some(msg);
                model_index += 1;
                continue;
            }
        };

        // Build the fallback event now, but enqueue it only after the next
        // attempt has been opened. Otherwise it would land in the previous
        // attempt and be correctly discarded together with that superseded
        // output during replay/materialization.
        let fallback_event_json = if idx > 0 {
            let display = format!("{} / {}", prov.name, model_ref.model_id);
            let reason_str = fallback_event_reason(last_reason, last_error.as_deref());
            ha_core::eval_context::record_model_retry(&session_id, true, reason_str.as_str(), 0);
            let event = serde_json::json!({
                "type": "model_fallback",
                "model": display,
                "from_model": primary_display,
                "provider_id": model_ref.provider_id,
                "model_id": model_ref.model_id,
                "reason": reason_str,
                "attempt": idx + 1,
                "total": total_models,
                "error": last_error.as_deref().unwrap_or(""),
            });
            serde_json::to_string(&event).ok()
        } else {
            None
        };

        // ── Outer compaction-retry loop ─────────────────────────
        // The executor (execute_with_failover) handles profile rotation +
        // retry-with-backoff in one call. Context overflow is the only
        // signal that needs to escape and re-enter — emergency_compact
        // borrows the agent mutably so it can't run inside the closure
        // while the operation is still holding the agent. After compact,
        // we write the failed profile back to PROFILE_STICKY so the next
        // executor call's select_profile picks it (preserves prompt cache
        // prefix that compaction did NOT invalidate).
        let mut compaction_attempts: u32 = 0;
        const MAX_COMPACTION_RETRIES: u32 = 1;
        let model_provider_id = model_ref.provider_id.clone();
        let model_id = model_ref.model_id.clone();

        loop {
            // Build the on-rotation callback that emits profile_rotation
            // events. Borrows event_sink + session_id + provider/model ids;
            // executor calls it inline so no Send/Sync gymnastics needed.
            let on_rotate =
                |from: &AuthProfile, to: &AuthProfile, reason: &failover::FailoverReason| {
                    app_info!(
                        "provider",
                        "failover",
                        "Rotating auth profile for {}::{}: {} -> {} (reason: {:?})",
                        model_provider_id,
                        model_id,
                        from.label,
                        to.label,
                        reason
                    );
                    if let Ok(json_str) = serde_json::to_string(&serde_json::json!({
                        "type": "profile_rotation",
                        "provider_id": model_provider_id,
                        "model_id": model_id,
                        "from_profile": from.label,
                        "to_profile": to.label,
                        "reason": reason,
                    })) {
                        emit_stream_event(
                            &db,
                            &event_sink,
                            &session_id,
                            source,
                            turn_id.as_deref(),
                            &json_str,
                        );
                    }
                };

            let retry_model_display = format!("{} / {}", prov.name, model_ref.model_id);
            let can_switch_model = has_resolvable_fallback(&model_chain, &providers, idx);
            let on_retry = |progress: &RetryProgress| {
                app_info!(
                    "provider",
                    "retry",
                    "Retrying {}::{} after {:?} (attempt {}/{}, delay={}ms)",
                    model_provider_id,
                    model_id,
                    progress.reason,
                    progress.attempt,
                    progress.max_attempts,
                    progress.delay_ms
                );
                if let Ok(json_str) = serde_json::to_string(&serde_json::json!({
                    "type": "model_retry",
                    "provider_id": model_provider_id,
                    "model_id": model_id,
                    "model": retry_model_display,
                    "reason": progress.reason,
                    "attempt": progress.attempt,
                    "total": progress.max_attempts,
                    "delay_ms": progress.delay_ms,
                    "recovery_id": progress.recovery_id,
                    "can_switch_model": can_switch_model,
                })) {
                    emit_stream_event(
                        &db,
                        &event_sink,
                        &session_id,
                        source,
                        turn_id.as_deref(),
                        &json_str,
                    );
                }
            };
            let can_replay_operation = || !durability.had_tool_activity();

            // Capture refs / clones the closure needs. `move` consumes per-
            // call clones; the original chat_engine values stay borrowable
            // for the next compaction-retry iteration.
            let providers_ref = &providers;
            let compact_config_ref = &compact_config;
            let agent_id_ref = &agent_id;
            let session_id_ref = &session_id;
            let channel_kb_context_ref = &channel_kb_context;
            let run_context_ref = &run_context;
            let agent_binding_refs_ref = &agent_binding_refs;
            let context_resource_refs_ref = &context_resource_refs;
            let skill_allowed_tools_ref = &skill_allowed_tools;
            let plan_resolved_ref = &plan_resolved;
            let message_ref = &message;
            let canonical_user_message_ref = &canonical_user_message;
            let attachments_ref = &attachments;
            let effort_str_ref = &effort_str;
            let cancel_ref = &cancel;
            let event_sink_ref = &event_sink;
            let db_ref = &db;
            let model_ref_for_op = model_ref;
            let codex_token_ref = &codex_token;
            let durability_ref = durability.clone();
            let prompt_context_receipt_ref = prompt_context_receipt.clone();
            let frozen_resource_mentions_ref = frozen_resource_mentions.clone();
            let snapshot_refs_committed_ref = snapshot_refs_committed.clone();
            let fallback_event_ref = fallback_event_json.as_deref();
            let current_user_message_state_for_attempt = current_user_message_state;

            let exec_result = execute_with_failover_observed(
                prov,
                &session_id,
                FailoverPolicy::chat_engine_default().with_cancel(cancel.clone()),
                Some(&on_rotate),
                Some(&on_retry),
                Some(&can_replay_operation),
                |profile| {
                    let profile_owned = profile.cloned();
                    // Sync setup: build + configure + restore. If build
                    // fails (e.g. Codex without token), surface as Unknown
                    // so the executor exhausts and we move to next model.
                    // Per-call clones for the streaming callback's `move ||`.
                    let event_sink_for_cb = event_sink_ref.clone();
                    let session_for_cb = session_id_ref.clone();
                    let source_for_cb = source;
                    let cancel_for_op = cancel_ref.clone();
                    let cancel_for_check = cancel_for_op.clone();
                    let cancel_for_wait = cancel_for_op.clone();
                    let turn_id_for_cb = turn_id.clone();

                    let agent_id_owned = agent_id_ref.clone();
                    let session_id_owned = session_id_ref.clone();
                    let run_context_owned = run_context_ref.clone();
                    let agent_bindings_owned = agent_binding_refs_ref.clone();
                    let context_resources_owned = context_resource_refs_ref.clone();
                    let skill_tools_owned = skill_allowed_tools_ref.clone();
                    let denied_tools_owned = denied_tools.clone();
                    let steer_run_id_owned = steer_run_id.clone();
                    let plan_resolved_owned = plan_resolved_ref.clone();
                    let channel_kb_context_owned = channel_kb_context_ref.clone();
                    let message_owned = message_ref.clone();
                    let canonical_user_message_owned = canonical_user_message_ref.clone();
                    // Arc<[Attachment]> clone is a pointer bump regardless
                    // of attachment size. See param destructure for the wrap.
                    let attachments_owned = attachments_ref.clone();
                    let effort_owned = effort_str_ref.clone();
                    let db_owned = db_ref.clone();
                    let provider_id_for_err = model_ref_for_op.provider_id.clone();
                    let model_id_for_err = model_ref_for_op.model_id.clone();
                    let codex_token_owned = codex_token_ref.clone();
                    let durability_owned = durability_ref.clone();
                    let prompt_context_receipt_owned = prompt_context_receipt_ref.clone();
                    let frozen_resource_mentions_owned = frozen_resource_mentions_ref.clone();
                    let snapshot_refs_committed_owned = snapshot_refs_committed_ref.clone();
                    let fallback_event_owned = fallback_event_ref.map(ToOwned::to_owned);
                    async move {
                        let provider_shape = match &prov.api_type {
                            ApiType::Anthropic => "anthropic",
                            ApiType::OpenaiChat => "openai_chat",
                            ApiType::OpenaiResponses => "openai_responses",
                            ApiType::Codex => "codex",
                        };
                        let attempt_no = durability_owned
                            .begin_attempt(
                                Some(&model_ref_for_op.provider_id),
                                Some(&model_ref_for_op.model_id),
                                Some(provider_shape),
                            )
                            .await?;
                        let current_user_message_state_for_op = if durability_owned
                            .attempt_base_contains_current_user()
                        {
                            CurrentUserMessageState::AlreadyInHistory
                        } else {
                            current_user_message_state_for_attempt
                        };
                        // Attempts are separate recovery prefixes. Re-commit a
                        // reference to the exact same frozen revision in every
                        // attempt so superseding attempt 1 cannot orphan the
                        // Agent/resource bindings. No resolver, Hook, or source
                        // read is repeated here.
                        let event = serde_json::json!({
                            "type": "initial_context_committed",
                            "revision": 0,
                            "attemptNo": attempt_no,
                            "replayed": attempt_no > 1,
                            "receipt": &*prompt_context_receipt_owned,
                            "agentBindings": &agent_bindings_owned,
                            // Compatibility key retained for journal/readers;
                            // v2 entries may represent typed Plan resources too.
                            "fileSnapshots": &*frozen_resource_mentions_owned,
                            "resourceSnapshotVersion": 2,
                            "skillAllowedTools": &skill_tools_owned,
                            "runContextSource": run_context_owned.as_ref().map(|context| context.source()),
                        })
                        .to_string();
                        durability_owned.accept_event(&event)?;
                        let source_journal_seq = durability_owned
                            .flush(ha_core::turn_durability::FlushReason::RoleSwitch)
                            .await?;
                        snapshot_refs_committed_owned
                            .store(true, std::sync::atomic::Ordering::Release);
                        if let (Some(turn_id), Some(projection)) = (
                            turn_id_for_cb.as_deref(),
                            ha_core::prompt_context::resolved_typed_mention_receipt_projection(
                                &canonical_user_message_owned,
                                &prompt_context_receipt_owned,
                                source_journal_seq,
                            ),
                        ) {
                            let receipt_db = db_owned.clone();
                            let receipt_session_id = session_for_cb.clone();
                            let receipt_turn_id = turn_id.to_string();
                            if let Err(error) = receipt_db
                                .run(move |db| {
                                    db.merge_chat_turn_typed_mention_receipt(
                                        &receipt_session_id,
                                        &receipt_turn_id,
                                        &projection,
                                    )
                                })
                                .await
                            {
                                // This projection is UI provenance, not model
                                // authority. A persistence failure must never
                                // fabricate a chip or fail an otherwise valid
                                // model turn; history simply has no receipt.
                                ha_core::app_warn!(
                                    "chat_engine",
                                    "typed_mention_receipt_projection",
                                    "failed to persist typed mention receipt: {}",
                                    error
                                );
                            }
                        }
                        if let Some(fallback_event) = fallback_event_owned.as_deref() {
                            emit_stream_event(
                                &db_owned,
                                &event_sink_for_cb,
                                &session_for_cb,
                                source_for_cb,
                                turn_id_for_cb.as_deref(),
                                fallback_event,
                            );
                        }
                        let mut agent = build_agent_from_snapshot(
                            model_ref_for_op,
                            providers_ref,
                            codex_token_owned,
                            compact_config_ref,
                            profile_owned.as_ref(),
                            session_id_ref,
                        )
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Cannot build agent for {}::{}: {}",
                                provider_id_for_err,
                                model_id_for_err,
                                e
                            )
                        })?;
                        configure_agent(
                            &mut agent,
                            &agent_id_owned,
                            &session_id_owned,
                            turn_id_for_cb.as_deref(),
                            db_owned.clone(),
                            resolved_temperature,
                            run_context_owned.as_ref(),
                            &agent_bindings_owned,
                            &context_resources_owned,
                            &skill_tools_owned,
                            &denied_tools_owned,
                            tool_scope,
                            subagent_depth,
                            steer_run_id_owned,
                            plan_resolved_owned,
                            plan_context_locked,
                            auto_approve_tools,
                            follow_global_reasoning_effort,
                            source,
                            kb_origin,
                            Some(durability_owned.stop_admission()),
                            channel_kb_context_owned,
                        );
                        agent.set_retrieval_query(canonical_user_message_owned);
                        agent.set_turn_durability(durability_owned.clone());
                        restore_agent_context(&db_owned, &session_id_owned, &agent);

                        let history_len_before = agent.get_conversation_history().len();
                        let chat_start = std::time::Instant::now();
                        let allow_hard_cancel = Arc::new(std::sync::atomic::AtomicBool::new(true));
                        let allow_hard_cancel_for_cb = allow_hard_cancel.clone();

                        let mut chat_future = Box::pin(crate::run_agent_chat(
                            &agent,
                            &message_owned,
                            &attachments_owned,
                            current_user_message_state_for_op,
                            effort_owned.as_deref(),
                            cancel_for_op,
                            move |delta| {
                                if !turn_accepts_stream_event(
                                    &db_owned,
                                    &session_for_cb,
                                    turn_id_for_cb.as_deref(),
                                ) {
                                    return;
                                }
                                if event_enters_runtime_loop(delta) {
                                    allow_hard_cancel_for_cb
                                        .store(false, std::sync::atomic::Ordering::SeqCst);
                                }
                                // Guard already checked above this tick — skip
                                // the redundant turn_accepts lock + snapshot.
                                emit_stream_event_unchecked(
                                    &event_sink_for_cb,
                                    &session_for_cb,
                                    source_for_cb,
                                    turn_id_for_cb.as_deref(),
                                    delta,
                                );
                            },
                        ));
                        let chat_result = match tokio::select! {
                            biased;
                            _ = wait_for_chat_cancel(cancel_for_wait) => None,
                            result = &mut chat_future => Some(result),
                        } {
                            Some(result) => result,
                            None if allow_hard_cancel.load(std::sync::atomic::Ordering::SeqCst) => {
                                Err(anyhow::anyhow!(CHAT_CANCELLED_BY_CALLER))
                            }
                            None => match tokio::time::timeout(
                                CHAT_CANCEL_COOPERATIVE_GRACE,
                                chat_future.as_mut(),
                            )
                            .await
                            {
                                Ok(result) => result,
                                Err(_) => {
                                    app_warn!(
                                        "chat",
                                        "cancel",
                                        "Force-dropping session {} model/tool loop after {}ms cancellation grace",
                                        session_id_owned,
                                        CHAT_CANCEL_COOPERATIVE_GRACE.as_millis()
                                    );
                                    Err(anyhow::anyhow!(CHAT_CANCELLED_BY_CALLER))
                                }
                            },
                        };
                        drop(chat_future);

                        if abort_on_cancel
                            && cancel_for_check.load(std::sync::atomic::Ordering::SeqCst)
                        {
                            return Err(anyhow::anyhow!("chat cancelled by caller"));
                        }

                        match chat_result {
                            Ok((response, thinking)) => Ok(ChatRoundOk {
                                response,
                                thinking,
                                agent,
                                history_len_before,
                                chat_start,
                            }),
                            Err(e) => Err(e),
                        }
                    }
                },
            )
            .await;

            match exec_result {
                Ok(ok) => {
                    let ChatRoundOk {
                        response,
                        thinking,
                        agent,
                        history_len_before,
                        chat_start,
                    } = ok;
                    let duration_ms = chat_start.elapsed().as_millis() as u64;

                    if let Some(ref tid) = turn_id {
                        if let Ok(Some(turn)) = db.get_chat_turn(tid) {
                            if turn.status.is_terminal() {
                                // A watchdog/request guard may have finalized
                                // chat_turns while the provider future was
                                // still unwinding. The journal must still be
                                // materialized atomically; merely marking the
                                // run terminal would strand already displayed
                                // bytes outside canonical messages/context.
                                let terminal = if turn.status == session::ChatTurnStatus::Completed
                                {
                                    session::ChatTurnStatus::Failed
                                } else {
                                    turn.status
                                };
                                let interrupt = turn
                                    .interrupt_reason
                                    .unwrap_or(session::ChatTurnInterruptReason::Unknown);
                                let convergence: Result<(), String> = async {
                                    let final_seq = durability
                                        .flush(FlushReason::Failure)
                                        .await
                                        .map_err(|error| error.to_string())?;
                                    durability
                                        .reconcile_spool_to_sqlite()
                                        .await
                                        .map_err(|error| error.to_string())?;
                                    let mut partial_text = durability.trailing_text();
                                    if partial_text.is_empty() && !durability.had_text_output() {
                                        partial_text = response.clone();
                                    }
                                    let assistant = durability.had_text_output().then(|| {
                                        build_durable_assistant_message(
                                            &durability,
                                            &partial_text,
                                            thinking.clone(),
                                            duration_ms,
                                            source,
                                        )
                                    });
                                    let context_json =
                                        serde_json::to_string(&agent.get_conversation_history())
                                            .map_err(|error| error.to_string())?;
                                    let commit = session::CommitInterruptedTurn {
                                        run_id: durability
                                            .is_persistent()
                                            .then(|| durability.persistence_run_id().to_string()),
                                        attempt_no: durability.current_attempt_no(),
                                        session_id: session_id.clone(),
                                        assistant,
                                        context_json,
                                        expected_context_revision: durability.context_revision(),
                                        turn_id: turn_id.clone(),
                                        final_seq,
                                        status: terminal,
                                        interrupt_reason: Some(interrupt.as_str().to_string()),
                                        error: turn.error.clone(),
                                        recovery_event: None,
                                        request_plan: durability.interrupted_request_plan_commit(
                                            session::RequestPlanResponseOutcome::ResponseIncomplete,
                                        ),
                                    };
                                    let db_for_commit = db.clone();
                                    db_for_commit
                                        .run(move |db| db.commit_interrupted_turn(&commit))
                                        .await
                                        .map_err(|error| error.to_string())?;
                                    durability
                                        .finalize_interrupted_request_after_turn_commit(
                                            session::RequestPlanResponseOutcome::ResponseIncomplete,
                                        )
                                        .await
                                        .map_err(|error| error.to_string())?;
                                    Ok(())
                                }
                                .await;
                                if let Err(error) = convergence {
                                    let message = format!(
                                        "externally-terminal stream convergence failed: {error}"
                                    );
                                    app_error!(
                                        "chat",
                                        "stream_durability",
                                        "run {}: {}",
                                        durability.persistence_run_id(),
                                        message
                                    );
                                    // Keep the DB run in `running` state so a
                                    // restart can replay its durable journal.
                                    durability.mark_interrupted("persistence_unavailable");
                                    stream_lifecycle.set_terminal(
                                        session::ChatTurnStatus::Failed,
                                        Some(session::ChatTurnInterruptReason::Unknown),
                                        Some(message.clone()),
                                    );
                                    stream_lifecycle.finish();
                                    let _ = abort_im_mirror_after_internal_error(
                                        &mut im_mirror,
                                        &session_id,
                                        &message,
                                    );
                                    return Err(message.into());
                                }
                                durability.mark_interrupted(terminal.as_str());
                                let mirror_reason = mirror_reason_from_terminal_state(
                                    terminal,
                                    Some(interrupt),
                                    turn.error.as_deref(),
                                );
                                stream_lifecycle.set_terminal(
                                    terminal,
                                    Some(interrupt),
                                    turn.error.clone(),
                                );
                                stream_lifecycle.finish();
                                schedule_browser_turn_finalize(source, &session_id);
                                let _ = abort_im_mirror_in_background(
                                    &mut im_mirror,
                                    &session_id,
                                    &mirror_reason,
                                );
                                retain_desktop_agent(source, agent).await;
                                return Ok(ChatEngineResult {
                                    response,
                                    model_used: Some(model_ref.clone()),
                                    usage: durability.usage(),
                                    terminal: TurnTerminal::from_chat_status(terminal),
                                });
                            }
                        }
                    }

                    // A provider can finish before the 100ms durability writer
                    // publishes its last batch. Publishing that batch may be
                    // the moment the UI observes the first delta and requests
                    // Stop, so check cancellation only after this barrier as
                    // well as inside the provider loop. Otherwise a late Stop
                    // races through the normal completed transaction.
                    if !abort_on_cancel && persist_final_error_event {
                        if let Err(error) = durability.flush(FlushReason::FinalEnd).await {
                            let message = format!("pre-final durability barrier failed: {error}");
                            let _ = abort_im_mirror_after_internal_error(
                                &mut im_mirror,
                                &session_id,
                                &message,
                            );
                            return Err(message.into());
                        }
                        if let Err(error) = durability.reconcile_spool_to_sqlite().await {
                            let message = format!("pre-final spool import failed: {error}");
                            let _ = abort_im_mirror_after_internal_error(
                                &mut im_mirror,
                                &session_id,
                                &message,
                            );
                            return Err(message.into());
                        }
                    }

                    if !abort_on_cancel
                        && cancel.load(std::sync::atomic::Ordering::SeqCst)
                        && persist_final_error_event
                    {
                        // Reuse the common journal-replay convergence below. It
                        // appends the user-stop marker to provider context and
                        // writes the matching UI event in the same transaction;
                        // the former inline branch omitted both.
                        last_reason = None;
                        last_error = Some(CHAT_CANCELLED_BY_CALLER.to_string());
                        last_was_no_profile = false;
                        break;
                    }

                    // Emit usage event with duration
                    let usage_event = serde_json::json!({
                        "type": "usage",
                        "duration_ms": duration_ms,
                    });
                    if let Ok(json_str) = serde_json::to_string(&usage_event) {
                        emit_stream_event(
                            &db,
                            &event_sink,
                            &session_id,
                            source,
                            turn_id.as_deref(),
                            &json_str,
                        );
                    }

                    // Freeze the complete durable prefix before deriving the
                    // canonical assistant. Reading `trailing_text()` before
                    // this barrier can miss the final <100ms pending batch and
                    // would commit a truncated assistant despite the journal
                    // containing (and the UI receiving) the full response.
                    let final_seq = match durability.flush(FlushReason::FinalEnd).await {
                        Ok(seq) => seq,
                        Err(error) => {
                            let message = format!("final durability barrier failed: {error}");
                            stream_lifecycle.set_terminal(
                                session::ChatTurnStatus::Failed,
                                Some(session::ChatTurnInterruptReason::Unknown),
                                Some(message.clone()),
                            );
                            stream_lifecycle.finish();
                            let _ = abort_im_mirror_after_internal_error(
                                &mut im_mirror,
                                &session_id,
                                &message,
                            );
                            return Err(message.into());
                        }
                    };
                    if let Err(error) = durability.reconcile_spool_to_sqlite().await {
                        let message = format!("cannot import emergency stream spool: {error}");
                        stream_lifecycle.set_terminal(
                            session::ChatTurnStatus::Failed,
                            Some(session::ChatTurnInterruptReason::Unknown),
                            Some(message.clone()),
                        );
                        stream_lifecycle.finish();
                        let _ = abort_im_mirror_after_internal_error(
                            &mut im_mirror,
                            &session_id,
                            &message,
                        );
                        return Err(message.into());
                    }

                    let mut trailing_text = durability.trailing_text();
                    let trailing_placeholder_id = None;
                    if trailing_text.is_empty()
                        && !durability.had_text_output()
                        && !response.is_empty()
                    {
                        // Defensive fallback for provider adapters that return
                        // terminal text without emitting text_delta.
                        trailing_text = response.clone();
                    }
                    let mut assistant_msg = build_durable_assistant_message(
                        &durability,
                        &trailing_text,
                        thinking,
                        duration_ms,
                        source,
                    );
                    let active_trace = agent.current_active_memory_trace();
                    let used_refs = agent.current_used_memory_refs();
                    let retrieval_planner_trace = agent.current_retrieval_planner_trace(&used_refs);
                    if active_trace.is_some()
                        || !used_refs.is_empty()
                        || retrieval_planner_trace.is_some()
                    {
                        let mut meta = serde_json::Map::new();
                        if let Some(trace) = active_trace {
                            meta.insert(
                                session::ATTACHMENT_META_KEY_ACTIVE_MEMORY.to_string(),
                                serde_json::to_value(&*trace).unwrap_or(serde_json::Value::Null),
                            );
                        }
                        if !used_refs.is_empty() {
                            meta.insert(
                                session::ATTACHMENT_META_KEY_USED_MEMORY_REFS.to_string(),
                                serde_json::to_value(used_refs).unwrap_or(serde_json::Value::Null),
                            );
                        }
                        if let Some(trace) = retrieval_planner_trace {
                            meta.insert(
                                session::ATTACHMENT_META_KEY_RETRIEVAL_PLANNER.to_string(),
                                serde_json::to_value(trace).unwrap_or(serde_json::Value::Null),
                            );
                        }
                        assistant_msg.attachments_meta =
                            serde_json::to_string(&serde_json::Value::Object(meta)).ok();
                    }
                    let usage = durability.usage();
                    let mut ledger_event =
                        ha_core::model_usage::ModelUsageEvent::new(ha_core::model_usage::KIND_CHAT);
                    if let Some(input_tokens) = usage.input_tokens {
                        ledger_event.input_tokens = Some(input_tokens.max(0) as u64);
                        ledger_event.cache_creation_input_tokens = usage
                            .cache_creation_input_tokens
                            .map(|value| value.max(0) as u64);
                        ledger_event.cache_read_input_tokens = usage
                            .cache_read_input_tokens
                            .map(|value| value.max(0) as u64);
                        ledger_event.context_input_tokens = usage
                            .context_input_tokens
                            .or(usage.input_tokens)
                            .map(|value| value.max(0) as u64);
                        ledger_event.fresh_input_tokens = usage
                            .fresh_input_tokens
                            .or(usage.input_tokens)
                            .map(|value| value.max(0) as u64);
                    }
                    ledger_event.output_tokens =
                        usage.output_tokens.map(|value| value.max(0) as u64);
                    ledger_event.metadata = Some(serde_json::json!({
                        "tokenAccounting": {
                            "inputCoverage": usage.input_coverage,
                            "outputCoverage": usage.output_coverage,
                            "observations": usage.token_accounting_observations,
                        }
                    }));
                    ledger_event.timestamp = Some(chrono::Utc::now().to_rfc3339());
                    ledger_event.operation = Some("chat".to_string());
                    ledger_event.source = Some(source.as_str().to_string());
                    ledger_event.provider_id = Some(model_ref.provider_id.clone());
                    ledger_event.provider_name = Some(prov.name.clone());
                    ledger_event.model_id = Some(
                        usage
                            .model
                            .clone()
                            .unwrap_or_else(|| model_ref.model_id.clone()),
                    );
                    ledger_event.session_id = Some(session_id.clone());
                    ledger_event.agent_id = Some(agent_id.clone());
                    ledger_event.duration_ms = Some(duration_ms);
                    ledger_event.ttft_ms = usage.ttft_ms.map(|value| value.max(0) as u64);
                    // Per-Provider-round accounting is recorded inside the
                    // streaming adapter. The durable aggregate still carries
                    // evaluation identity for traceability without counting a
                    // second model call.
                    ha_core::eval_context::enrich_usage_metadata(&mut ledger_event);

                    let context_json =
                        match serde_json::to_string(&agent.get_conversation_history()) {
                            Ok(context_json) => context_json,
                            Err(error) => {
                                let message = format!("serialize final context failed: {error}");
                                let _ = abort_im_mirror_after_internal_error(
                                    &mut im_mirror,
                                    &session_id,
                                    &message,
                                );
                                return Err(message.into());
                            }
                        };
                    let commit = session::CommitAssistantTurn {
                        run_id: durability
                            .is_persistent()
                            .then(|| durability.persistence_run_id().to_string()),
                        attempt_no: durability.current_attempt_no(),
                        session_id: session_id.clone(),
                        assistant: assistant_msg,
                        trailing_placeholder_id,
                        context_json,
                        expected_context_revision: durability.context_revision(),
                        turn_id: turn_id.clone(),
                        usage: Some(ledger_event),
                        final_seq,
                        tier3_recovery: if agent.tier3_summary_applied_this_turn() {
                            session::Tier3RecoveryCommit::ClearAfterSummary
                        } else {
                            session::Tier3RecoveryCommit::Unchanged
                        },
                        request_plan: durability.successful_request_plan_commit()?,
                    };
                    // ACP cancellation is observed on a separate stdin reader
                    // thread. Linearize that request against completion before
                    // the durable terminal transaction: cancel-first falls
                    // through the normal UserStop finalizer, while
                    // completion-first disarms this prompt generation so the
                    // reader cannot publish a late session pause.
                    if !claim_non_cancelled_terminal(completion_claim.as_ref(), &cancel) {
                        last_reason = None;
                        last_error = Some(CHAT_CANCELLED_BY_CALLER.to_string());
                        last_was_no_profile = false;
                        break;
                    }
                    let committed = {
                        let db = db.clone();
                        db.run(move |db| db.commit_assistant_turn(&commit)).await
                    };
                    let committed = match committed {
                        Ok(committed) => committed,
                        Err(_) if cancel.load(std::sync::atomic::Ordering::SeqCst) => {
                            // Stop may win after the final in-memory cancel
                            // check but before the atomic success transaction.
                            // The DB refuses to overwrite `cancelling`; converge
                            // the durable journal through the normal UserStop
                            // finalizer instead of misclassifying that CAS as a
                            // persistence failure.
                            last_reason = None;
                            last_error = Some(CHAT_CANCELLED_BY_CALLER.to_string());
                            last_was_no_profile = false;
                            break;
                        }
                        Err(error) => {
                            let message = format!("final assistant transaction failed: {error}");
                            // Do not terminalize the persistence run here.
                            // Its journal is the only recovery source after a
                            // failed final transaction; startup must still see
                            // the run as recoverable.
                            durability.mark_interrupted("failed");
                            stream_lifecycle.set_terminal(
                                session::ChatTurnStatus::Failed,
                                Some(session::ChatTurnInterruptReason::Unknown),
                                Some(message.clone()),
                            );
                            stream_lifecycle.finish();
                            let _ = abort_im_mirror_after_internal_error(
                                &mut im_mirror,
                                &session_id,
                                &message,
                            );
                            return Err(message.into());
                        }
                    };
                    durability
                        .finalize_successful_request_after_turn_commit()
                        .await?;
                    let assistant_id = Some(committed.assistant_message_id);
                    durability.mark_committed(committed.committed_seq);

                    // GUI / HTTP turns mirror into the attached IM chat via
                    // the live stream sink. Kick the final IM flush before
                    // ending the frontend lifecycle and before running
                    // post-turn side effects so title/memory work cannot
                    // delay the remote chat's finalization. It runs in the
                    // background so slow IM network calls never hold the GUI
                    // path open.
                    let _ = finalize_im_mirror_in_background(&mut im_mirror, response.clone());

                    // The user-visible response is complete once the final
                    // assistant row is durable. End the frontend stream here;
                    // memory extraction and other follow-ups below must not
                    // keep the stop button/sidebar spinner alive.
                    let terminal_status = session::ChatTurnStatus::Completed;
                    let interrupt_reason = None;
                    stream_lifecycle.set_terminal(terminal_status, interrupt_reason, None);
                    stream_lifecycle.finish();
                    schedule_browser_turn_finalize(source, &session_id);

                    // Stop hook: the agent finished responding. `terminal_status`
                    // distinguishes a natural `completed` from an interrupt —
                    // block-to-continue is honored ONLY on `completed`
                    // (fire_stop guards on it), never on a user interrupt.
                    // `response` is the turn's final assistant text
                    // (`last_assistant_message`), so a Stop hook can inspect it.
                    ha_core::hooks::fire_stop(
                        &session_id,
                        Some(&agent_id),
                        terminal_status.as_str(),
                        Some(&response),
                    );

                    if terminal_status == session::ChatTurnStatus::Completed {
                        let continuation = {
                            let session_id = session_id.clone();
                            let agent_id = agent_id.clone();
                            let turn_id = turn_id.clone();
                            db.run(move |db| {
                                ha_core::goal::maybe_schedule_goal_continuation(
                                    db,
                                    &session_id,
                                    &agent_id,
                                    source,
                                    turn_id.as_deref(),
                                    assistant_id,
                                )
                            })
                            .await
                        };
                        if let Err(e) = continuation {
                            app_warn!(
                                "goal",
                                "auto_continue",
                                "Failed to schedule goal continuation for session {}: {}",
                                session_id,
                                e
                            );
                        }
                    }

                    if post_turn_effects {
                        ha_core::session_title::maybe_schedule_after_success(
                            db.clone(),
                            session_id.clone(),
                            agent_id.clone(),
                            model_ref.clone(),
                        );
                        {
                            let usage_snapshot = durability.usage();
                            let round_tokens = usage_snapshot
                                .best_effort_total_tokens()
                                .min(u64::from(u32::MAX))
                                as u32;
                            let round_messages = agent
                                .get_conversation_history()
                                .len()
                                .saturating_sub(history_len_before)
                                as u32;
                            agent.accumulate_extraction_stats(round_tokens, round_messages);
                        }

                        let idle_timeout = schedule_memory_extraction_after_turn(
                            &agent_id,
                            &session_id,
                            model_ref,
                            &agent,
                        )
                        .await;

                        // Skill auto-review trigger (gate 1 of the five-gate
                        // waterfall). Feed tool_use_count from this round's
                        // conversation slice — pure-chat turns yield 0 and
                        // are filtered by `require_tool_use` in the config.
                        // `history_tail_stats` walks the slice under one lock
                        // without cloning the whole history.
                        {
                            let round_tokens = {
                                let u = durability.usage();
                                u.best_effort_total_tokens().min(usize::MAX as u64) as usize
                            };
                            let (round_messages, tool_use_count) =
                                agent.history_tail_stats(history_len_before);
                            let cfg = ha_core::config::cached_config()
                                .skills
                                .auto_review
                                .clone()
                                .sanitize();
                            // Two user messages within 30 seconds is the
                            // "user is correcting themselves" signal — cheap
                            // DB read, only consulted when the master
                            // toggle is on.
                            let user_correction = cfg.correction_signal_enabled
                                && db.user_messages_within(&session_id, 30).unwrap_or(false);
                            // 闸 1 起的整条瀑布（trigger → spawn(run_review_cycle)
                            // → sweep_stale）在 ha-skills；kernel 只算这四个
                            // 信号标量——`user_correction` 需要 SessionDB。
                            ha_core::skills_hooks::auto_review_post_turn(
                                &session_id,
                                &cfg,
                                round_tokens,
                                round_messages,
                                tool_use_count,
                                user_correction,
                            );
                        }

                        if idle_timeout > 0 {
                            let (tokens_remain, msgs_remain) = agent.extraction_tracking_counts();
                            if tokens_remain > 0 || msgs_remain > 0 {
                                let updated_at = db
                                    .get_session(&session_id)
                                    .ok()
                                    .flatten()
                                    .map(|s| s.updated_at)
                                    .unwrap_or_default();
                                ha_core::memory_extract::schedule_idle_extraction(
                                    agent_id.clone(),
                                    session_id.clone(),
                                    updated_at,
                                    idle_timeout,
                                );
                            }
                        }
                    }

                    retain_desktop_agent(source, agent).await;
                    return Ok(ChatEngineResult {
                        response,
                        model_used: Some(model_ref.clone()),
                        usage: durability.usage(),
                        terminal: TurnTerminal::Completed,
                    });
                }

                Err(ExecutorError::NeedsCompaction {
                    last_profile,
                    evidence,
                }) => {
                    if !evidence.is_high_confidence() {
                        let msg = format!(
                            "Refusing emergency compaction without high-confidence overflow evidence: {evidence:?}"
                        );
                        app_warn!("context", "compact_evidence", "{}", msg);
                        last_reason = Some(failover::FailoverReason::Unknown);
                        last_error = Some(msg);
                        break;
                    }
                    let capacity_proof = match &evidence {
                        failover::ContextOverflowEvidence::LocalPreflight {
                            input_tokens,
                            max_input_tokens,
                            capacity_proof: Some(proof),
                            ..
                        } if proof.original_local_upper_bound == *input_tokens
                            && proof.max_input_tokens == *max_input_tokens =>
                        {
                            proof.clone()
                        }
                        failover::ContextOverflowEvidence::LocalPreflight { .. }
                        | failover::ContextOverflowEvidence::StructuredProvider { .. }
                        | failover::ContextOverflowEvidence::TextHint { .. } => {
                            let msg = format!(
                                "Refusing emergency compaction on {}::{} without an immutable complete-request capacity proof",
                                model_ref.provider_id, model_ref.model_id,
                            );
                            app_warn!("context", "compact_capacity_unproven", "{}", msg);
                            last_reason = Some(failover::FailoverReason::ContextOverflow);
                            last_error = Some(msg.clone());
                            compaction_failed.get_or_insert(msg);
                            break;
                        }
                    };
                    last_reason = Some(failover::FailoverReason::ContextOverflow);
                    if let Some((status, interrupt, error)) =
                        terminal_turn_state(&db, turn_id.as_deref())
                    {
                        let mirror_reason =
                            mirror_reason_from_terminal_state(status, interrupt, error.as_deref());
                        stream_lifecycle.set_terminal(status, interrupt, error);
                        stream_lifecycle.finish();
                        schedule_browser_turn_finalize(source, &session_id);
                        let _ = abort_im_mirror_in_background(
                            &mut im_mirror,
                            &session_id,
                            &mirror_reason,
                        );
                        return Ok(ChatEngineResult {
                            response: String::new(),
                            model_used: Some(model_ref.clone()),
                            usage: Default::default(),
                            terminal: TurnTerminal::from_chat_status(status),
                        });
                    }

                    if durability.had_non_replayable_tool_activity() {
                        let msg = format!(
                            "Context overflow on {}::{} after non-replayable tool activity; refusing to replay the turn",
                            model_ref.provider_id, model_ref.model_id
                        );
                        app_warn!("provider", "recovery_blocked", "{}", msg);
                        last_reason = Some(failover::FailoverReason::ContextOverflow);
                        last_error = Some(msg);
                        break;
                    }

                    if compaction_attempts >= MAX_COMPACTION_RETRIES {
                        app_warn!(
                            "context",
                            "compact",
                            "Context overflow on {}::{} persists after compaction, moving to next model",
                            model_ref.provider_id,
                            model_ref.model_id
                        );
                        let msg = format!(
                            "Context overflow on {}::{} after emergency compaction",
                            model_ref.provider_id, model_ref.model_id
                        );
                        last_reason = Some(failover::FailoverReason::ContextOverflow);
                        last_error = Some(msg.clone());
                        compaction_failed.get_or_insert(msg);
                        break;
                    }
                    compaction_attempts += 1;

                    app_info!(
                        "context",
                        "compact",
                        "Context overflow on {}::{}, attempting emergency compaction (evidence={:?})",
                        model_ref.provider_id,
                        model_ref.model_id,
                        evidence
                    );

                    let mut progress_extra = serde_json::Map::new();
                    progress_extra.insert(
                        "attempt".to_string(),
                        serde_json::json!(compaction_attempts),
                    );
                    progress_extra.insert(
                        "max_attempts".to_string(),
                        serde_json::json!(MAX_COMPACTION_RETRIES),
                    );
                    progress_extra.insert(
                        "provider_id".to_string(),
                        serde_json::json!(model_ref.provider_id),
                    );
                    progress_extra.insert(
                        "model_id".to_string(),
                        serde_json::json!(model_ref.model_id),
                    );
                    let _ = emit_context_compaction_progress(
                        &db,
                        &event_sink,
                        &session_id,
                        source,
                        turn_id.as_deref(),
                        "preparing",
                        "emergency",
                        Some(progress_extra),
                    );

                    // Build a temporary agent to run the compaction. Same
                    // profile that just hit overflow so the cache prefix is
                    // identical.
                    let mut compact_agent = match build_agent_from_snapshot(
                        model_ref,
                        &providers,
                        codex_token.clone(),
                        &compact_config,
                        last_profile.as_ref(),
                        &session_id,
                    )
                    .await
                    {
                        Ok(a) => a,
                        Err(e) => {
                            // The "preparing"/emergency spinner was already emitted
                            // above; emit a terminal "failed" so the GUI banner
                            // resolves instead of spinning forever on this break.
                            let _ = emit_context_compaction_progress(
                                &db,
                                &event_sink,
                                &session_id,
                                source,
                                turn_id.as_deref(),
                                "failed",
                                "emergency",
                                None,
                            );
                            let msg = format!(
                                "Cannot build agent for emergency compaction on {}::{}: {}",
                                model_ref.provider_id, model_ref.model_id, e
                            );
                            last_reason = Some(failover::FailoverReason::ContextOverflow);
                            last_error = Some(msg);
                            break;
                        }
                    };
                    configure_agent(
                        &mut compact_agent,
                        &agent_id,
                        &session_id,
                        turn_id.as_deref(),
                        db.clone(),
                        resolved_temperature,
                        run_context.as_ref(),
                        &agent_binding_refs,
                        &context_resource_refs,
                        &skill_allowed_tools,
                        &denied_tools,
                        tool_scope,
                        subagent_depth,
                        steer_run_id.clone(),
                        plan_resolved.clone(),
                        plan_context_locked,
                        auto_approve_tools,
                        follow_global_reasoning_effort,
                        source,
                        kb_origin,
                        Some(durability.stop_admission()),
                        channel_kb_context.clone(),
                    );
                    restore_agent_context(&db, &session_id, &compact_agent);

                    let mut history = compact_agent.get_conversation_history();
                    let original_history_for_capacity =
                        ha_core::context_compact::prepare_messages_for_api(&history);
                    let current_user_anchor =
                        ha_core::context_compact::latest_user_request_anchor(&history);
                    // Incognito parity with the Tier-3 path (agent/context.rs): an
                    // incognito session must NOT have its runtime ledger (job /
                    // subagent ids) built or injected into history — that history is
                    // both sent to the model and persisted via save_agent_context
                    // below. Fail-closed: a missing/burned session row counts as
                    // incognito. Gating lives in `emergency_runtime_ledger` (unit-tested).
                    let emergency_ledger = ha_core::agent::emergency_runtime_ledger(
                        &session_id,
                        ha_core::session::is_session_incognito(Some(&session_id)),
                    );
                    let emergency_ctx = ha_core::context_compact::EmergencyCompactionContext {
                        config: &compact_config,
                        runtime_ledger: emergency_ledger.as_ref(),
                    };
                    let compact_result = compact_agent
                        .context_engine()
                        .emergency_compact(&mut history, &emergency_ctx);
                    if !current_user_anchor
                        .as_ref()
                        .is_some_and(|anchor| anchor.is_preserved_exactly_once(&history))
                    {
                        let msg = format!(
                            "Emergency compaction could not preserve the current user request exactly once on {}::{}; refusing to publish or retry",
                            model_ref.provider_id, model_ref.model_id,
                        );
                        app_warn!("context", "compact_user_anchor_lost", "{}", msg);
                        let _ = emit_context_compaction_progress(
                            &db,
                            &event_sink,
                            &session_id,
                            source,
                            turn_id.as_deref(),
                            "failed",
                            "emergency",
                            None,
                        );
                        last_reason = Some(failover::FailoverReason::ContextOverflow);
                        last_error = Some(msg.clone());
                        compaction_failed.get_or_insert(msg);
                        break;
                    }
                    if compact_result.messages_affected == 0
                        || compact_result.tokens_after >= compact_result.tokens_before
                    {
                        let msg = format!(
                            "Emergency compaction made no measurable progress on {}::{} (before={}, after={}, affected={}); refusing to publish or retry the same oversized request",
                            model_ref.provider_id,
                            model_ref.model_id,
                            compact_result.tokens_before,
                            compact_result.tokens_after,
                            compact_result.messages_affected,
                        );
                        app_warn!("context", "compact_no_progress", "{}", msg);
                        let _ = emit_context_compaction_progress(
                            &db,
                            &event_sink,
                            &session_id,
                            source,
                            turn_id.as_deref(),
                            "failed",
                            "emergency",
                            None,
                        );
                        last_reason = Some(failover::FailoverReason::ContextOverflow);
                        last_error = Some(msg.clone());
                        compaction_failed.get_or_insert(msg);
                        break;
                    }
                    let compacted_history_for_capacity =
                        ha_core::context_compact::prepare_messages_for_api(&history);
                    let projected_input_upper = match ha_core::token_accounting::service()
                        .verify_compacted_capacity(
                            &capacity_proof,
                            &original_history_for_capacity,
                            &compacted_history_for_capacity,
                        ) {
                        Ok(projected_input_upper) => projected_input_upper,
                        Err(error) => {
                            let msg = format!(
                                "Emergency compaction capacity proof failed on {}::{}: {}; refusing to publish or retry",
                                model_ref.provider_id, model_ref.model_id, error,
                            );
                            app_warn!("context", "compact_capacity_unproven", "{}", msg);
                            let _ = emit_context_compaction_progress(
                                &db,
                                &event_sink,
                                &session_id,
                                source,
                                turn_id.as_deref(),
                                "failed",
                                "emergency",
                                None,
                            );
                            last_reason = Some(failover::FailoverReason::ContextOverflow);
                            last_error = Some(msg.clone());
                            compaction_failed.get_or_insert(msg);
                            break;
                        }
                    };
                    app_info!(
                        "context",
                        "compact_capacity_proven",
                        "Emergency compaction complete-request capacity proven on {}::{}: input_upper={} max_input={}",
                        model_ref.provider_id,
                        model_ref.model_id,
                        projected_input_upper,
                        capacity_proof.max_input_tokens,
                    );
                    compact_agent.set_conversation_history(history);
                    if let Some((status, interrupt, error)) =
                        terminal_turn_state(&db, turn_id.as_deref())
                    {
                        let mirror_reason =
                            mirror_reason_from_terminal_state(status, interrupt, error.as_deref());
                        stream_lifecycle.set_terminal(status, interrupt, error);
                        stream_lifecycle.finish();
                        schedule_browser_turn_finalize(source, &session_id);
                        let _ = abort_im_mirror_in_background(
                            &mut im_mirror,
                            &session_id,
                            &mirror_reason,
                        );
                        return Ok(ChatEngineResult {
                            response: String::new(),
                            model_used: Some(model_ref.clone()),
                            usage: Default::default(),
                            terminal: TurnTerminal::from_chat_status(status),
                        });
                    }
                    let compact_history = compact_agent.get_conversation_history();
                    if let Err(error) = durability
                        .checkpoint_emergency_context(
                            &compact_history,
                            durability.context_revision(),
                        )
                        .await
                    {
                        let _ = emit_context_compaction_progress(
                            &db,
                            &event_sink,
                            &session_id,
                            source,
                            turn_id.as_deref(),
                            "failed",
                            "emergency",
                            None,
                        );
                        last_error =
                            Some(format!("Emergency compaction context CAS failed: {error}"));
                        break;
                    }
                    if let Err(error) = durability.adopt_attempt_base_context(&compact_history) {
                        last_error =
                            Some(format!("Emergency compaction retry base failed: {error}"));
                        break;
                    }
                    current_user_message_state = CurrentUserMessageState::AlreadyInHistory;

                    let mut progress_extra = serde_json::Map::new();
                    progress_extra.insert(
                        "attempt".to_string(),
                        serde_json::json!(compaction_attempts),
                    );
                    progress_extra.insert(
                        "max_attempts".to_string(),
                        serde_json::json!(MAX_COMPACTION_RETRIES),
                    );
                    let _ = emit_context_compaction_progress(
                        &db,
                        &event_sink,
                        &session_id,
                        source,
                        turn_id.as_deref(),
                        "finalizing",
                        "emergency",
                        Some(progress_extra),
                    );

                    // Manual snake_case shape — `CompactResult` itself is
                    // `rename_all="camelCase"`, but the frontend / IM
                    // formatter / persister all key off snake_case fields
                    // (matching `agent/context.rs`'s pre-LLM compaction
                    // emit). Direct `"data": compact_result` would silently
                    // skip every consumer's tier filter.
                    if let Ok(event_str) = serde_json::to_string(&serde_json::json!({
                        "type": "context_compacted",
                        "data": {
                            "tier_applied": compact_result.tier_applied,
                            "tokens_before": compact_result.tokens_before,
                            "tokens_after": compact_result.tokens_after,
                            "messages_affected": compact_result.messages_affected,
                            "description": compact_result.description,
                            "manifest": compact_result.manifest,
                        },
                    })) {
                        // The coordinator journals this event and materializes
                        // it exactly once with the final turn transaction.
                        emit_stream_event(
                            &db,
                            &event_sink,
                            &session_id,
                            source,
                            turn_id.as_deref(),
                            &event_str,
                        );
                    }

                    // Write the just-failed profile back to PROFILE_STICKY
                    // so the next executor call's select_profile picks it
                    // first (compaction reduces tokens but doesn't change
                    // the cached prefix → same key avoids a cache miss).
                    if let Some(ref p) = last_profile {
                        failover::PROFILE_STICKY.set(&model_ref.provider_id, &session_id, &p.id);
                    }
                    continue;
                }

                Err(ExecutorError::Cancelled) => {
                    last_reason = None;
                    last_error = Some(CHAT_CANCELLED_BY_CALLER.to_string());
                    last_was_no_profile = false;
                    break;
                }

                Err(ExecutorError::SwitchModel {
                    last_reason: r,
                    last_error: err_str,
                }) => {
                    app_info!(
                        "provider",
                        "manual_model_switch",
                        "Skipping remaining retries for {}::{} at user request",
                        model_ref.provider_id,
                        model_ref.model_id
                    );
                    last_reason = Some(r);
                    last_error = Some(err_str);
                    last_was_no_profile = false;
                    manual_model_switch = true;
                    break;
                }

                Err(ExecutorError::Exhausted {
                    last_reason: r,
                    last_error: err_str,
                }) => {
                    app_warn!(
                        "provider",
                        "failover",
                        "Giving up on {}::{} (reason {:?}), moving to next model in chain",
                        model_ref.provider_id,
                        model_ref.model_id,
                        r
                    );

                    // Codex Auth → emit codex_auth_expired so frontend can
                    // prompt the user to re-authorize.
                    let is_codex_auth =
                        matches!(r, failover::FailoverReason::Auth) && prov.api_type.is_codex();
                    if is_codex_auth {
                        if let Ok(json_str) = serde_json::to_string(&serde_json::json!({
                            "type": "codex_auth_expired",
                            "error": &err_str,
                        })) {
                            emit_stream_event(
                                &db,
                                &event_sink,
                                &session_id,
                                source,
                                turn_id.as_deref(),
                                &json_str,
                            );
                        }
                    }

                    last_is_codex_auth = is_codex_auth;
                    last_reason = Some(r);
                    last_error = Some(err_str);
                    last_was_no_profile = false;
                    break;
                }

                Err(ExecutorError::NoProfileAvailable) => {
                    app_warn!(
                        "provider",
                        "failover",
                        "No auth profile available for {}::{}",
                        model_ref.provider_id,
                        model_ref.model_id
                    );
                    let msg = format!(
                        "No auth profile available for {}::{}",
                        model_ref.provider_id, model_ref.model_id
                    );
                    last_reason = Some(failover::classify_error(&msg));
                    last_error = Some(msg);
                    last_was_no_profile = true;
                    break;
                }
            }
        }

        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }

        // Every model/profile retry rebuilds the Agent from the turn's stable
        // base context. Once a tool ran, doing so would replay its external
        // side effects instead of resuming after its result.
        if durability.had_non_replayable_tool_activity() {
            app_warn!(
                "provider",
                "recovery_blocked",
                "Not switching models for session {} after tool activity",
                session_id
            );
            break;
        }

        if last_reason.is_some_and(|reason| reason.is_terminal()) {
            break;
        }

        model_index += 1;
        // "Switch model" means leave the current model immediately. If there
        // is no later configured model, do not reinterpret it as permission to
        // restart the same chain from its first model.
        if model_index >= model_chain.len() && manual_model_switch {
            break;
        }
    }

    // All non-success paths (cancel, exhausted, no-profile, compaction
    // give-up) converge here.
    let final_error = last_error
        .clone()
        .unwrap_or_else(|| "All models in the fallback chain failed.".to_string());
    app_error!(
        "provider",
        "failover",
        "All {} models exhausted for session {}: {}",
        total_models,
        session_id,
        final_error
    );

    // Provider exhaustion and every other non-success outcome commit their
    // own durable terminal below. They must participate in the same ACP
    // completion/cancel ordering as success: cancel-first becomes UserStop;
    // terminal-first disarms the prompt generation before its failed commit.
    let _ = claim_non_cancelled_terminal(completion_claim.as_ref(), &cancel);
    let reason = derive_termination_reason(
        abort_on_cancel,
        &cancel,
        last_reason,
        last_error.as_deref(),
        last_is_codex_auth,
        compaction_failed.as_deref(),
        last_was_no_profile,
    );

    // The journal, rather than legacy placeholder rows, is the truth source
    // for failed/aborted turns. Keep the last visible attempt and converge the
    // partial assistant + context + turn status atomically.
    let terminal_status = reason.to_chat_turn_status();
    let terminal_interrupt = reason.to_chat_turn_interrupt_reason();
    let durability_result: anyhow::Result<()> = async {
        let durable_seq = durability.flush(FlushReason::Stop).await?;
        if durability.is_persistent() {
            durability.reconcile_spool_to_sqlite().await?;
        }

        let (attempt_no, commit_seq, visible_events, integrity_error, provider_kind) =
            if durability.is_persistent() {
                let run_id = durability.persistence_run_id().to_string();
                let db_for_snapshot = db.clone();
                let snapshot = db_for_snapshot
                    .run(move |db| db.stream_run_snapshot(&run_id))
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("persistence run disappeared"))?;
                let (attempt_no, commit_seq, events, integrity_error) =
                    session::select_recoverable_attempt_prefix(&snapshot);
                let attempt = snapshot
                    .attempts
                    .iter()
                    .find(|attempt| attempt.attempt_no == attempt_no);
                let provider_kind = attempt
                    .and_then(|attempt| attempt.provider_shape.as_deref())
                    .or(snapshot.run.provider_shape.as_deref())
                    .and_then(finalize::ProviderApiKind::from_shape);
                (
                    attempt_no,
                    commit_seq,
                    events,
                    integrity_error,
                    provider_kind,
                )
            } else {
                let snapshot = durability.snapshot();
                (
                    durability.current_attempt_no(),
                    durable_seq,
                    snapshot.events,
                    None,
                    durability
                        .current_provider_shape()
                        .as_deref()
                        .and_then(finalize::ProviderApiKind::from_shape),
                )
            };
        let trailing_text = session::trailing_text_from_journal_events(&visible_events);
        let assistant = session::journal_events_have_assistant_output(&visible_events).then(|| {
            let mut message = session::NewMessage::assistant(&trailing_text);
            message.source = Some(source.as_str().to_string());
            message
        });
        let (context_json, context_checkpoint_seq, context_revision, has_context_checkpoint) =
            if durability.is_persistent() {
                let run_id = durability.persistence_run_id().to_string();
                db.clone()
                    .run(move |db| {
                        let (context, checkpoint_seq, revision) =
                            db.recovery_context_for_prefix(&run_id, attempt_no, commit_seq)?;
                        let has_checkpoint =
                            db.stream_context_checkpoint_exists(&run_id, attempt_no, commit_seq)?;
                        Ok::<_, anyhow::Error>((context, checkpoint_seq, revision, has_checkpoint))
                    })
                    .await?
            } else {
                let session_id_for_context = session_id.clone();
                let (context, revision) = db
                    .clone()
                    .run(move |db| db.load_context_with_revision(&session_id_for_context))
                    .await?;
                (context, 0, revision, false)
            };
        let mut history: Vec<serde_json::Value> = context_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default();
        if !has_context_checkpoint {
            let user_message = message.trim();
            if !user_message.is_empty() {
                history.push(serde_json::json!({
                    "role": "user",
                    "content": user_message,
                }));
            }
        }
        finalize::rebuild::append_journal_suffix_to_history(
            &mut history,
            &visible_events,
            context_checkpoint_seq,
            provider_kind,
        )?;
        history.push(serde_json::json!({
            "role": "assistant",
            "content": finalize::copy::model_marker(&reason),
        }));
        let context_json = serde_json::to_string(&history)?;
        let recovery_event = persist_final_error_event.then(|| {
            let mut event = if terminal_status == session::ChatTurnStatus::Failed {
                session::NewMessage::error_event(&finalize::copy::user_notice(&reason))
            } else {
                session::NewMessage::event(&finalize::copy::user_notice(&reason))
            };
            event.source = Some(source.as_str().to_string());
            event
        });
        let commit = session::CommitInterruptedTurn {
            run_id: durability
                .is_persistent()
                .then(|| durability.persistence_run_id().to_string()),
            attempt_no,
            session_id: session_id.clone(),
            assistant,
            context_json,
            expected_context_revision: context_revision,
            turn_id: turn_id.clone(),
            final_seq: commit_seq,
            status: terminal_status,
            interrupt_reason: Some(terminal_interrupt.as_str().to_string()),
            error: integrity_error.or_else(|| {
                (terminal_status == session::ChatTurnStatus::Failed).then(|| final_error.clone())
            }),
            recovery_event,
            request_plan: durability.interrupted_request_plan_commit(
                if terminal_interrupt == session::ChatTurnInterruptReason::UserStop {
                    session::RequestPlanResponseOutcome::CancelledAfterResponse
                } else {
                    session::RequestPlanResponseOutcome::ResponseIncomplete
                },
            ),
        };
        let db_for_commit = db.clone();
        db_for_commit
            .run(move |db| db.commit_interrupted_turn(&commit))
            .await?;
        durability
            .finalize_interrupted_request_after_turn_commit(
                if terminal_interrupt == session::ChatTurnInterruptReason::UserStop {
                    session::RequestPlanResponseOutcome::CancelledAfterResponse
                } else {
                    session::RequestPlanResponseOutcome::ResponseIncomplete
                },
            )
            .await?;
        durability.mark_interrupted(terminal_status.as_str());
        Ok(())
    }
    .await;

    if let Err(error) = durability_result {
        app_error!(
            "chat",
            "stream_durability",
            "failed to converge terminal stream {}: {}",
            durability.persistence_run_id(),
            error
        );
        // Leave the DB run recoverable, but release the live coordinator so
        // the UI is not reported as indefinitely active in this process.
        durability.mark_interrupted("persistence_unavailable");
    }
    let _ = abort_im_mirror_in_background(&mut im_mirror, &session_id, &reason);
    stream_lifecycle.set_terminal(
        terminal_status,
        Some(terminal_interrupt),
        (terminal_status == session::ChatTurnStatus::Failed).then(|| final_error.clone()),
    );

    if matches!(reason, TerminationReason::UserStop) && !abort_on_cancel {
        stream_lifecycle.finish();
        schedule_browser_turn_finalize(source, &session_id);
        return Ok(ChatEngineResult {
            response: String::new(),
            model_used: None,
            usage: Default::default(),
            terminal: TurnTerminal::Cancelled,
        });
    }

    schedule_browser_turn_finalize(source, &session_id);
    stream_lifecycle.finish();
    let (failure_kind, failure_reason, failure_is_codex_auth) = classify_turn_failure(&reason);
    Err(
        TurnFailure::classified(failure_kind, failure_reason, final_error)
            .with_route_all_codex(failure_is_codex_auth),
    )
}

fn classify_turn_failure(
    reason: &TerminationReason,
) -> (TurnFailureKind, Option<failover::FailoverReason>, bool) {
    match reason {
        TerminationReason::UserStop | TerminationReason::RuntimeCancel => {
            (TurnFailureKind::Cancelled, None, false)
        }
        TerminationReason::ProviderFailed {
            last_kind,
            is_codex_auth,
            ..
        } => (
            if last_kind.is_terminal() {
                TurnFailureKind::Terminal
            } else {
                TurnFailureKind::ProviderExhausted
            },
            Some(*last_kind),
            *is_codex_auth,
        ),
        TerminationReason::NoProfileAvailable => (TurnFailureKind::ProviderExhausted, None, false),
        TerminationReason::CompactionFailed { .. } => (
            TurnFailureKind::Infrastructure,
            Some(failover::FailoverReason::ContextOverflow),
            false,
        ),
        TerminationReason::Other { .. }
        | TerminationReason::Shutdown
        | TerminationReason::Crash => (TurnFailureKind::Infrastructure, None, false),
    }
}

pub fn build_durable_assistant_message(
    durability: &ha_core::chat_engine::durability::StreamCoordinator,
    content: &str,
    thinking: Option<String>,
    duration_ms: u64,
    source: stream_seq::ChatSource,
) -> session::NewMessage {
    let usage = durability.usage();
    let mut message = session::NewMessage::assistant(content);
    message.tool_duration_ms = Some(duration_ms.min(i64::MAX as u64) as i64);
    if !durability.had_thinking() {
        message.thinking = thinking;
    }
    message.tokens_in = usage.input_tokens;
    message.tokens_out = usage.output_tokens;
    message.tokens_in_last = usage.last_context_input_tokens.or(usage.last_input_tokens);
    message.model = usage.model;
    message.ttft_ms = usage.ttft_ms;
    message.tokens_cache_creation = usage
        .last_cache_creation_input_tokens
        .or(usage.cache_creation_input_tokens);
    message.tokens_cache_read = usage
        .last_cache_read_input_tokens
        .or(usage.cache_read_input_tokens);
    message.source = Some(source.as_str().to_string());
    message
}

// ── Termination reason derivation ────────────────────────────────────

/// Map runtime convergence state to a [`TerminationReason`].
///
/// A set cancel flag is the positive signal for `UserStop`; user-facing
/// desktop / HTTP / IM paths all preserve partial state and converge through
/// the same interrupted finalizer. `last_reason == None` after a non-cancel
/// path means we never even reached an executor call → `NoProfileAvailable`.
/// Everything else is `ProviderFailed` carrying the classified reason.
pub fn derive_termination_reason(
    _abort_on_cancel: bool,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    last_reason: Option<failover::FailoverReason>,
    last_error: Option<&str>,
    last_is_codex_auth: bool,
    compaction_failed: Option<&str>,
    last_was_no_profile: bool,
) -> TerminationReason {
    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return TerminationReason::UserStop;
    }
    if let Some(detail) = compaction_failed {
        return TerminationReason::CompactionFailed {
            detail: detail.to_string(),
        };
    }
    // Profile-availability failure is configuration-class, not API-class.
    // The `Err(NoProfileAvailable)` branch fills `last_reason`/`last_error`
    // for logging, but the unified taxonomy surfaces this distinctly.
    if last_was_no_profile {
        return TerminationReason::NoProfileAvailable;
    }
    match (last_reason, last_error) {
        (Some(kind), Some(msg)) => TerminationReason::ProviderFailed {
            last_kind: kind,
            last_message: msg.to_string(),
            is_codex_auth: last_is_codex_auth,
        },
        (Some(kind), None) => TerminationReason::ProviderFailed {
            last_kind: kind,
            last_message: String::new(),
            is_codex_auth: last_is_codex_auth,
        },
        (None, Some(msg)) => TerminationReason::Other {
            message: msg.to_string(),
        },
        (None, None) => TerminationReason::NoProfileAvailable,
    }
}

/// Build [`PartialMeta`] from runtime convergence state.
///
/// The text / thinking / tool_use rebuild is reverse-engineered from
/// the `messages` table by [`finalize::rebuild::collect_partial_from_messages`]
/// — `persist_failed_partial_assistant` has already written the
/// assistant row that links text/thinking blocks, and the tool rows
/// persist independently. Runtime only needs to overlay metadata that
/// the table doesn't carry (user_message text for the early-persist
/// gap, provider shape from the last attempt, turn id, persisted
/// assistant id).
#[allow(dead_code)] // legacy placeholder finalize compatibility
pub fn collect_partial_meta_from_runtime(
    db: &std::sync::Arc<session::SessionDB>,
    session_id: &str,
    user_message: &str,
    api_type: Option<ha_core::provider::ApiType>,
    assistant_message_id: Option<i64>,
    turn_id: Option<&str>,
) -> PartialMeta {
    let provider_kind = api_type.map(finalize::ProviderApiKind::from);
    let mut meta = finalize::rebuild::collect_partial_from_messages(db, session_id, provider_kind);
    meta.user_message = Some(user_message.to_string());
    meta.turn_id = turn_id.map(str::to_owned);
    if assistant_message_id.is_some() {
        meta.assistant_message_id = assistant_message_id;
    }
    meta
}

/// Schedule turn-end browser cleanup, skipping `ParentInjection` turns.
///
/// Background-job / wakeup completions inject into the PARENT session and run a
/// turn under that session_id. Running the turn-end finalize there would tear
/// down the parent's live browser scope (close agent tabs, drop claim leases)
/// mid-task while the user may still be working in that session. The parent's
/// own foreground turns and session teardown handle cleanup, so injection turns
/// must skip it. Other sources (`Desktop`/`Http`/`Channel`/`Subagent`/`Cron`)
/// finalize their own session scope, which matches the documented turn-end
/// release.
pub fn schedule_browser_turn_finalize(source: stream_seq::ChatSource, session_id: &str) {
    if matches!(source, stream_seq::ChatSource::ParentInjection) {
        return;
    }
    // 特征钩子（未 wire no-op：无 extension tab 可 finalize；wrapper 首次未
    // 命中打一次 warn，避免每轮刷屏）。
    ha_core::browser_hooks::schedule_turn_finalize(session_id);
}

#[cfg(test)]
mod terminal_claim_tests {
    use super::*;

    #[test]
    fn rejected_completion_claim_converts_failed_terminal_to_user_stop() {
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let claim = ha_core::chat_engine::TurnCompletionClaim::new(|| false);

        assert!(!claim_non_cancelled_terminal(Some(&claim), &cancel));
        assert!(matches!(
            derive_termination_reason(
                false,
                &cancel,
                Some(ha_core::failover::FailoverReason::Unknown),
                Some("provider failed"),
                false,
                None,
                false,
            ),
            TerminationReason::UserStop
        ));
    }

    #[test]
    fn accepted_completion_claim_preserves_failed_terminal() {
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let claim = ha_core::chat_engine::TurnCompletionClaim::new(|| true);

        assert!(claim_non_cancelled_terminal(Some(&claim), &cancel));
        assert!(matches!(
            derive_termination_reason(
                false,
                &cancel,
                Some(ha_core::failover::FailoverReason::Unknown),
                Some("provider failed"),
                false,
                None,
                false,
            ),
            TerminationReason::ProviderFailed { .. }
        ));
    }
}
