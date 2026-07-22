//! Provider-agnostic streaming chat orchestration.
//!
//! [`AssistantAgent::run_streaming_chat`] runs the full tool loop using a
//! [`StreamingChatAdapter`] for provider-specific concerns (body / SSE /
//! history). All compaction, tool dispatch, microcompact, steer mailbox
//! drain, and event emission live here — provider files become thin
//! adapters owning only body construction + SSE decoding + history shape.
//!
//! See [`super::streaming_adapter`] for the trait that adapters implement.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use futures_util::future::join_all;
use serde_json::json;

use super::api_types::FunctionCallItem;
use super::content::build_user_content_for_provider;
use super::context::MidLoopCompactionState;
use super::events::{
    emit_max_rounds_notice, emit_round_limit_event, emit_tool_call, emit_tool_call_args_rewritten,
    emit_tool_result, emit_usage, extract_media_items,
};
use super::streaming_adapter::{ExecutedTool, RoundRequest, StreamingChatAdapter};
use super::types::{AssistantAgent, ChatUsage};
use crate::tools::{self, ToolExecContext};

/// All four providers share the same max_tokens budget: it caps Anthropic's
/// `max_tokens` request field and feeds the compaction / microcompact token
/// budget estimator. OpenAI Chat / Responses / Codex don't put this in the
/// request body — only in the budget calculator.
const MAX_OUTPUT_TOKENS: u32 = 16384;
const TOOL_CANCEL_CLEANUP_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Max concurrent-safe (read-only) tools allowed to run at once within one
/// assistant turn. Bounds fd / outbound-request fan-out when a single message
/// emits many read-only calls (e.g. N `web_fetch`). An internal guardrail (peer
/// to the IM-inbound concurrency const), not a user-facing knob.
const MAX_CONCURRENT_SAFE_TOOLS: usize = 8;

fn terminal_assistant_text_for_history<'a>(
    cancelled: bool,
    final_assistant_text: &'a str,
    pending_terminal_text: &'a str,
) -> &'a str {
    if cancelled && final_assistant_text.is_empty() {
        pending_terminal_text
    } else {
        final_assistant_text
    }
}

async fn wait_for_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

/// Run `futs` concurrently with at most `max` in flight at any time, returning
/// results in the SAME order as the input. Order preservation lets callers pair
/// results to inputs positionally. The semaphore is never closed, so permit
/// acquisition cannot fail (`.ok()` degrades to unbounded only in that
/// impossible closed case).
async fn run_bounded_in_order<T, Fut>(max: usize, futs: Vec<Fut>) -> Vec<T>
where
    Fut: std::future::Future<Output = T>,
{
    // `max.max(1)`: a degenerate cap of 0 would make `Semaphore::new(0)` +
    // `acquire_owned()` park forever (acquire only errors on a *closed*
    // semaphore), so clamp it to single-flight. Today both callers pass 8;
    // this guards future reuse with a config-derived bound.
    let sem = Arc::new(tokio::sync::Semaphore::new(max.max(1)));
    let wrapped = futs.into_iter().map(|f| {
        let sem = sem.clone();
        async move {
            let _permit = sem.acquire_owned().await.ok();
            f.await
        }
    });
    join_all(wrapped).await
}

fn final_round_handoff_guidance(max_rounds: u32) -> String {
    format!(
        "# Tool-Call Limit Reached\n\n\
         This is the final allowed response for this user turn: the tool-call \
         limit of {} rounds has been reached and tools are now unavailable. \
         Tell the user this limit was reached, summarize what is done, list \
         what remains, and ask them to send a new message such as \"继续\" \
         if they want you to continue. Do not claim the whole task is complete \
         unless every required item has actually been verified.",
        max_rounds
    )
}

// ── Tool execution helpers (private to streaming_loop, no other caller).

/// Log tool execution input.
fn log_tool_input(tc: &FunctionCallItem, round: u32) {
    if let Some(logger) = crate::get_logger() {
        let args_str = tc.arguments.as_str();
        let args_preview = if args_str.len() > 2048 {
            format!(
                "{}...(truncated, total {}B)",
                crate::truncate_utf8(args_str, 2048),
                args_str.len()
            )
        } else {
            args_str.to_string()
        };
        logger.log(
            "debug",
            "agent",
            "agent::tool_exec::input",
            &format!("Tool exec [{}] id={}", tc.name, tc.call_id),
            Some(
                json!({
                    "tool_name": tc.name,
                    "call_id": tc.call_id,
                    "arguments": args_preview,
                    "round": round,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }
}

/// Log tool execution output.
fn log_tool_output(call_id: &str, name: &str, result: &str, elapsed_ms: u64, round: u32) {
    if let Some(logger) = crate::get_logger() {
        let result_preview = if result.len() > 2048 {
            format!(
                "{}...(truncated, total {}B)",
                crate::truncate_utf8(result, 2048),
                result.len()
            )
        } else {
            result.to_string()
        };
        let is_error = result.starts_with("Tool error:");
        logger.log(
            if is_error { "warn" } else { "debug" },
            "agent",
            "agent::tool_exec::output",
            &format!(
                "Tool result [{}] {}B, {}ms{}",
                name,
                result.len(),
                elapsed_ms,
                if is_error { " (ERROR)" } else { "" }
            ),
            Some(
                json!({
                    "tool_name": name,
                    "call_id": call_id,
                    "result_size_bytes": result.len(),
                    "elapsed_ms": elapsed_ms,
                    "is_error": is_error,
                    "result_preview": result_preview,
                    "round": round,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }
}

/// Fire `PostToolUse` / `PostToolUseFailure` for one settled tool call and fold
/// any `additionalContext` the hooks return into `clean_result`, so it rides
/// into history attached to this tool's result on the next round (design
/// §5.2.2). Observation events — non-blocking. No-op cost is near-zero when no
/// hooks are configured (the dispatcher short-circuits on an empty registry).
async fn fire_post_tool_use_hook(
    ctx: &ToolExecContext,
    call_id: &str,
    name: &str,
    arguments: &str,
    clean_result: &mut String,
    is_error: bool,
    elapsed_ms: u64,
) {
    use crate::hooks::{HookDispatcher, HookEvent, HookInput};

    let event = if is_error {
        HookEvent::PostToolUseFailure
    } else {
        HookEvent::PostToolUse
    };
    // Hot-path gate: this fires per tool per round. Skip all input building
    // (two serde parses, one over a possibly-large clean_result) when no hook
    // listens for this event — multi-scope (project/local for this session's
    // working dir too).
    if !crate::hooks::scopes::any_handlers_for(
        event,
        ctx.session_working_dir.as_deref().map(std::path::Path::new),
    ) {
        return;
    }
    let common = ctx.common_hook_input(event.as_str());
    let tool_input = serde_json::from_str(arguments).unwrap_or(serde_json::Value::Null);
    let input = if is_error {
        HookInput::PostToolUseFailure {
            common,
            tool_name: name.to_string(),
            tool_input,
            tool_use_id: call_id.to_string(),
            error: clean_result.clone(),
            // Phase 1.1: interrupt vs error not distinguished at this site.
            is_interrupt: false,
            duration_ms: elapsed_ms,
            // Synchronous (foreground) settle — async-job terminals fill this
            // via `fire_async_job_terminal`.
            job_id: None,
        }
    } else {
        let tool_response = serde_json::from_str(clean_result)
            .unwrap_or_else(|_| serde_json::Value::String(clean_result.clone()));
        HookInput::PostToolUse {
            common,
            tool_name: name.to_string(),
            tool_input,
            tool_response,
            tool_use_id: call_id.to_string(),
            job_id: None,
        }
    };
    let outcome = HookDispatcher::dispatch(event, input).await;
    // `updatedToolOutput` (official): a `PostToolUse` hook may rewrite the tool
    // result before it re-enters history (e.g. redact secrets). Applied on the
    // success path only. A JSON string replaces verbatim; any other JSON value
    // is stringified.
    if let Some(updated) = outcome.updated_mcp_output.as_ref() {
        *clean_result = match updated {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
    }
    if let Some(extra) = outcome.merged_additional_context() {
        // Frame the injected context so the model can tell hook output apart
        // from the tool's own result.
        clean_result.push_str("\n\n<hook-context>\n");
        clean_result.push_str(&extra);
        clean_result.push_str("\n</hook-context>");
    }
}

async fn drain_queued_turn_user_messages<F>(
    agent: &AssistantAgent,
    adapter: &dyn StreamingChatAdapter,
    messages: &mut Vec<serde_json::Value>,
    on_delta: &F,
) where
    F: Fn(&str) + Send + Sync,
{
    let Some(session_id) = agent.session_id.as_deref() else {
        return;
    };
    let Some(active) = crate::chat_engine::active_turn::current(session_id) else {
        return;
    };
    if !matches!(
        active.source,
        crate::chat_engine::stream_seq::ChatSource::Desktop
            | crate::chat_engine::stream_seq::ChatSource::Http
    ) {
        return;
    }

    let Some(db) = crate::get_session_db() else {
        return;
    };
    let queued = crate::chat_engine::turn_injection::drain(session_id, &active.turn_id);
    if queued.is_empty() {
        return;
    }

    for mut item in queued {
        if active.cancel.load(Ordering::SeqCst) {
            break;
        }
        let raw_prompt =
            crate::util::non_empty_trim_or(item.display_text.as_deref(), &item.message);
        let effective_prompt = match crate::agent::preflight::user_prompt_preflight(
            crate::agent::preflight::PreflightArgs {
                session_id,
                agent_id: Some(&agent.agent_id),
                raw_prompt,
            },
        )
        .await
        {
            crate::agent::preflight::PreflightOutcome::Proceed { effective_prompt } => {
                if let Some(extra) = crate::hooks::take_user_prompt_context(session_id) {
                    agent.push_pending_hook_context(extra);
                }
                effective_prompt
            }
            crate::agent::preflight::PreflightOutcome::Block { reason } => {
                let notice = if reason.trim().is_empty() {
                    "🚫 Prompt blocked by a UserPromptSubmit hook.".to_string()
                } else {
                    format!("🚫 {reason}")
                };
                let _ = db.append_message(
                    session_id,
                    &crate::session::NewMessage::event(&notice).with_source(active.source),
                );
                let _ = db.remove_claimed_turn_message(session_id, &item.request_id);
                if let Ok(event) = serde_json::to_string(&json!({
                    "type": "queued_user_message_blocked",
                    "request_id": item.request_id,
                    "session_id": item.session_id,
                    "turn_id": item.turn_id,
                    "reason": notice,
                })) {
                    on_delta(&event);
                }
                continue;
            }
        };

        let attachment_meta = match crate::attachments::persist_chat_user_attachments_meta(
            session_id,
            &mut item.attachments,
        ) {
            Ok(meta) => meta,
            Err(err) => {
                let notice = format!("🚫 Failed to insert queued message attachments: {err}");
                let _ = db.append_message(
                    session_id,
                    &crate::session::NewMessage::event(&notice).with_source(active.source),
                );
                let _ = db.remove_claimed_turn_message(session_id, &item.request_id);
                if let Ok(event) = serde_json::to_string(&json!({
                    "type": "queued_user_message_blocked",
                    "request_id": item.request_id,
                    "session_id": item.session_id,
                    "turn_id": item.turn_id,
                    "reason": notice,
                })) {
                    on_delta(&event);
                }
                continue;
            }
        };
        let attachments_meta = crate::session::build_chat_user_attachments_meta(
            item.is_plan_trigger,
            item.plan_comment.as_ref(),
            item.goal_trigger,
            attachment_meta,
        );
        let mut user_msg =
            crate::session::NewMessage::user(&effective_prompt).with_source(active.source);
        user_msg.attachments_meta = attachments_meta.clone();
        let message_id = match db.complete_inserted_turn_message(&item, &user_msg) {
            Ok(id) => id,
            Err(err) => {
                let notice = format!("🚫 Failed to insert queued message: {err}");
                let _ = db.append_message(
                    session_id,
                    &crate::session::NewMessage::event(&notice).with_source(active.source),
                );
                let _ = db.remove_claimed_turn_message(session_id, &item.request_id);
                if let Ok(event) = serde_json::to_string(&json!({
                    "type": "queued_user_message_blocked",
                    "request_id": item.request_id,
                    "session_id": item.session_id,
                    "turn_id": item.turn_id,
                    "reason": notice,
                })) {
                    on_delta(&event);
                }
                continue;
            }
        };

        let user_content = build_user_content_for_provider(
            adapter.provider_format(),
            &item.message,
            &item.attachments,
        );
        AssistantAgent::push_user_message(messages, user_content);

        if let Ok(event) = serde_json::to_string(&json!({
            "type": "queued_user_message_inserted",
            "request_id": item.request_id,
            "session_id": item.session_id,
            "turn_id": item.turn_id,
            "message_id": message_id,
            "content": effective_prompt,
            "attachments_meta": attachments_meta,
            "is_plan_trigger": item.is_plan_trigger,
            "plan_comment": item.plan_comment,
        })) {
            on_delta(&event);
        }
    }
}

/// Pull the `job_id` out of a synthetic `{"status":"started","job_id":...}`
/// background-tool result, if that's what the string is. Returns `None` for
/// any non-JSON / non-started payload (safe fallback — nothing to cancel).
/// Used by the turn-cancel grace window to reap a job that a just-approved
/// background tool spawned (MISC-2).
fn extract_started_job_id(tool_result: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(tool_result.trim()).ok()?;
    if v.get("status").and_then(|s| s.as_str()) != Some("started") {
        return None;
    }
    v.get("job_id")
        .and_then(|j| j.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Stable, content-only identifier used by evaluation telemetry to recognize
/// repeated tool work without retaining potentially sensitive arguments.
fn eval_tool_arguments_digest(args: &serde_json::Value) -> String {
    ha_eval_spec::canonical_json(args)
        .map(|bytes| ha_eval_spec::sha256_bytes(&bytes))
        .unwrap_or_else(|_| ha_eval_spec::sha256_bytes(args.to_string().as_bytes()))
}

fn eval_raw_tool_arguments_digest(arguments: &str) -> String {
    ha_eval_spec::sha256_bytes(arguments.as_bytes())
}

fn eval_tool_result_digest(result: &str) -> String {
    ha_eval_spec::sha256_bytes(result.as_bytes())
}

/// Execute a tool with cancel-flag racing. Returns `(result_string,
/// elapsed_ms, side_output)`. The side output carries structured metadata
/// (file change before/after snapshots, line deltas, etc.) emitted by the
/// tool through [`ToolExecContext::emit_metadata`]; one fresh sink is
/// constructed per call so concurrent peers cannot clobber each other.
async fn execute_tool_with_cancel(
    name: &str,
    call_id: &str,
    args: &serde_json::Value,
    ctx: &ToolExecContext,
    cancel: &Arc<AtomicBool>,
    durability: Option<Arc<dyn crate::turn_durability::TurnDurabilitySink>>,
    on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
) -> Result<(
    String,
    u64,
    super::streaming_adapter::ToolDispatchSideOutput,
)> {
    let sink: Arc<tokio::sync::Mutex<Option<serde_json::Value>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    // Handshake sink for effective arguments. Tool execution pauses at the
    // rewrite point until this task journals and flushes the update, so a
    // side effect can never race ahead of its durable invocation record.
    let effective_args_sink = Arc::new(tools::EffectiveArgsSink::default());
    let mut local_ctx = ctx.clone();
    local_ctx.metadata_sink = Some(sink.clone());
    local_ctx.effective_args_sink = Some(effective_args_sink.clone());
    local_ctx.tool_call_id = Some(call_id.to_string());
    let cancellation_token = tokio_util::sync::CancellationToken::new();
    local_ctx.cancellation_token = Some(cancellation_token.clone());
    let tool_start = std::time::Instant::now();
    if let Err(error) = crate::eval_context::ensure_tool_budget(ctx.session_id.as_deref()) {
        let elapsed_ms = tool_start.elapsed().as_millis() as u64;
        let rendered = tools::ToolRejection::render_error(&error);
        crate::eval_context::record_tool_result_with_digest(
            ctx.session_id.as_deref(),
            name,
            call_id,
            &eval_tool_arguments_digest(args),
            Some(&eval_tool_result_digest(&rendered)),
            crate::eval_context::EvalToolOutcome::Failed,
            elapsed_ms,
        );
        return Ok((rendered, elapsed_ms, Default::default()));
    }
    if let Some(fault) = crate::eval_context::tool_fault_action(ctx.session_id.as_deref(), name) {
        if fault.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(fault.delay_ms)).await;
        }
        if let Some(error_class) = fault.error_class {
            let elapsed_ms = tool_start.elapsed().as_millis() as u64;
            let error = anyhow::anyhow!(
                "controlled evaluation fault {} ({error_class})",
                fault.fault_id
            );
            let rendered = tools::ToolRejection::render_error(&error);
            crate::eval_context::record_tool_result_with_digest(
                ctx.session_id.as_deref(),
                name,
                call_id,
                &eval_tool_arguments_digest(args),
                Some(&eval_tool_result_digest(&rendered)),
                crate::eval_context::EvalToolOutcome::Failed,
                elapsed_ms,
            );
            return Ok((rendered, elapsed_ms, Default::default()));
        }
    }
    let cancel_clone = cancel.clone();
    let mut dispatch = Box::pin(tools::execute_tool_with_context(name, args, &local_ctx));
    let mut effective_arguments = None;
    let mut eval_outcome = crate::eval_context::EvalToolOutcome::Succeeded;
    let result = loop {
        tokio::select! {
            biased;
            update = effective_args_sink.next() => {
                let patched = update.value.to_string();
                emit_tool_call_args_rewritten(on_delta, call_id, &patched);
                let barrier = match durability.as_ref() {
                    Some(sink) => sink
                        .flush(crate::turn_durability::FlushReason::ToolBoundary)
                        .await
                        .map(|_| ()),
                    None => Ok(()),
                };
                match barrier {
                    Ok(()) => {
                        effective_arguments = Some(patched);
                        let _ = update.acknowledged.send(Ok(()));
                    }
                    Err(error) => {
                        let message = error.to_string();
                        let _ = update.acknowledged.send(Err(message));
                        cancellation_token.cancel();
                        return Err(error);
                    }
                }
            }
            res = &mut dispatch => {
                break match res {
                    Ok(r) => r,
                    Err(e) => {
                        eval_outcome = crate::eval_context::EvalToolOutcome::Failed;
                        tools::ToolRejection::render_error(&e)
                    }
                };
            }
            _ = async {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    if cancel_clone.load(Ordering::SeqCst) { break; }
                }
            } => {
                cancellation_token.cancel();
                // Grace window: let the dispatch wind down. If the user approved a
                // background-capable tool (exec / web_search / …) inside this
                // window, the dispatch returns a synthetic `{job_id,status:"started"}`
                // and has ALREADY detached a runner with its own fresh cancel token —
                // the turn cancel never reaches it, so the job would run on as an
                // orphan while the model is told "cancelled" (MISC-2). Capture that
                // result and cancel the freshly-spawned job so the verdict stays
                // truthful. (Sync inline tools that don't finish in time are dropped
                // here as before; their exec process group is reaped by
                // `ProcessGroupGuard::drop`.)
                if let Ok(Ok(grace_result)) =
                    tokio::time::timeout(TOOL_CANCEL_CLEANUP_GRACE, &mut dispatch).await
                {
                    if let Some(job_id) = extract_started_job_id(&grace_result) {
                        app_info!(
                            "async_jobs",
                            "cancel",
                            "Reaping job {} spawned by tool '{}' inside the turn-cancel grace window",
                            job_id,
                            name
                        );
                        let _ = crate::async_jobs::JobManager::cancel(&job_id);
                    }
                }
                eval_outcome = crate::eval_context::EvalToolOutcome::Cancelled;
                break tools::ToolRejection::cancelled(name).to_tool_result();
            }
        }
    };
    let elapsed_ms = tool_start.elapsed().as_millis() as u64;
    let metadata = sink.lock().await.take();
    let arguments_digest = effective_arguments
        .as_deref()
        .and_then(|arguments| serde_json::from_str(arguments).ok())
        .map(|arguments| eval_tool_arguments_digest(&arguments))
        .unwrap_or_else(|| eval_tool_arguments_digest(args));
    crate::eval_context::record_tool_result_with_digest(
        ctx.session_id.as_deref(),
        name,
        call_id,
        &arguments_digest,
        Some(&eval_tool_result_digest(&result)),
        eval_outcome,
        elapsed_ms,
    );
    Ok((
        result,
        elapsed_ms,
        super::streaming_adapter::ToolDispatchSideOutput {
            metadata,
            effective_arguments,
        },
    ))
}

fn invalid_tool_arguments_result(
    name: &str,
    raw_arguments: &str,
    err: serde_json::Error,
) -> String {
    let preview = if raw_arguments.len() > 500 {
        format!(
            "{}...(truncated, total {}B)",
            crate::truncate_utf8(raw_arguments, 500),
            raw_arguments.len()
        )
    } else {
        raw_arguments.to_string()
    };
    format!(
        "{}Invalid JSON arguments for tool '{}': {}. Raw arguments: {}",
        tools::TOOL_ERROR_PREFIX,
        name,
        err,
        preview
    )
}

fn collect_tool_search_activations(
    side: &super::streaming_adapter::ToolDispatchSideOutput,
    out: &mut Vec<String>,
) {
    let Some(metadata) = side.metadata.as_ref() else {
        return;
    };
    if metadata.get("kind").and_then(|v| v.as_str()) != Some("tool_search_activation") {
        return;
    }
    let Some(names) = metadata
        .get("activatedToolNames")
        .and_then(|value| value.as_array())
    else {
        return;
    };
    for name in names.iter().filter_map(|value| value.as_str()) {
        if !out.iter().any(|existing| existing == name) {
            out.push(name.to_string());
        }
    }
}

fn prompt_cache_key(
    agent: &AssistantAgent,
    provider: super::types::ProviderFormat,
    model: &str,
    stable_prompt: &str,
) -> String {
    const PROMPT_CONTRACT_VERSION: &str = "v2";
    let prompt_hash = blake3::hash(stable_prompt.as_bytes()).to_hex();
    let scope = if agent.session_is_incognito() {
        agent.session_id.as_deref().unwrap_or("incognito")
    } else {
        agent.agent_id.as_str()
    };
    let scope_hash = blake3::hash(scope.as_bytes()).to_hex();
    format!(
        "ha:{}:{model}:{PROMPT_CONTRACT_VERSION}:{}:{}",
        provider.label(),
        &scope_hash[..12],
        &prompt_hash[..16]
    )
}

impl AssistantAgent {
    fn record_eval_model_attempt(
        &self,
        model: &str,
        provider_label: &str,
        round: u32,
        usage: Option<&ChatUsage>,
        ttft_ms: Option<u64>,
        duration_ms: u64,
        error: Option<&anyhow::Error>,
    ) {
        let Some(session_id) = self.session_id.as_deref() else {
            return;
        };
        if crate::eval_context::context_for_session(session_id).is_none() {
            return;
        }
        let mut event = crate::model_usage::ModelUsageEvent::new(crate::model_usage::KIND_CHAT);
        event.request_key = Some(format!("eval:{session_id}:round:{round}"));
        event.operation = Some("chat_round".to_string());
        event.provider_id = self
            .provider_config
            .as_deref()
            .map(|provider| provider.id.clone());
        event.provider_name = Some(provider_label.to_string());
        event.model_id = Some(model.to_string());
        event.session_id = Some(session_id.to_string());
        event.agent_id = Some(self.agent_id.clone());
        event.duration_ms = Some(duration_ms);
        event.ttft_ms = ttft_ms;
        if let Some(usage) = usage {
            event.input_tokens = Some(usage.input_tokens);
            event.output_tokens = Some(usage.output_tokens);
            event.cache_creation_input_tokens = Some(usage.cache_creation_input_tokens);
            event.cache_read_input_tokens = Some(usage.cache_read_input_tokens);
            event.context_input_tokens = Some(usage.context_input_tokens);
            event.fresh_input_tokens = Some(usage.fresh_input_tokens);
        }
        if let Some(error) = error {
            event.success = false;
            event.error = Some(error.to_string());
        }
        crate::eval_context::record_model_usage(&event);
    }

    /// Provider-agnostic streaming chat with tool loop.
    ///
    /// All four `chat_<provider>` entry points delegate here, passing a
    /// provider-specific [`StreamingChatAdapter`] and a pre-built user-content
    /// `Value` (because content shape differs per provider).
    ///
    /// The orchestrator owns:
    ///   - reset_chat_flags / refresh_awareness / refresh_active_memory
    ///   - tool schema build + history normalize + push_user_message
    ///   - system prompt build + compaction + memory selection + cache snapshot
    ///   - per-round: cancel check, touch_active_session, drain steer mailbox,
    ///     prepare_messages_for_api, dispatch tools (concurrent + sequential),
    ///     manual_memory_save check, truncate_tool_results, reactive_microcompact
    ///   - max-rounds notice, final assistant persist, emit_usage
    ///
    /// The adapter owns: normalize_history, chat_round (body+SSE),
    /// append_round_to_history, append_final_assistant, loop_should_exit.
    pub(crate) async fn run_streaming_chat<F>(
        &self,
        adapter: &dyn StreamingChatAdapter,
        model: &str,
        message: &str,
        user_content_for_history: serde_json::Value,
        reasoning_effort: Option<&str>,
        cancel: &Arc<AtomicBool>,
        on_delta: &F,
    ) -> Result<(String, Option<String>)>
    where
        F: Fn(&str) + Send + Sync,
    {
        let provider_label = adapter.provider_format().label();

        self.reset_chat_flags();
        self.refresh_coding_profile_suffix(message);

        // The user item is the turn's crash-recovery base, not merely transient
        // request state. Persist its exact provider-native shape at seq=0 before
        // slow dynamic-context work or provider IO can expose assistant output.
        // A failed attempt rolls this snapshot back and the next provider writes
        // its independently normalized user shape.
        let mut messages = {
            let history_guard = self
                .conversation_history
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let mut h = history_guard.clone();
            drop(history_guard);
            adapter.normalize_history(&mut h);
            h
        };
        Self::push_user_message(&mut messages, user_content_for_history);
        *self
            .conversation_history
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = messages.clone();
        if let Some(sink) = self.turn_durability.as_ref() {
            sink.checkpoint_context(&messages, sink.context_revision())
                .await?;
        }

        self.warm_kb_access().await;
        self.warm_memory_agent_config().await;
        self.configure_retrieval_planner_context(message);
        // Dynamic context refreshers write independent slots / trace ledgers
        // and never read each other; run them concurrently so the worst case
        // stays bounded by the slowest refresher instead of their sum.
        let refresh_turn_context = async {
            tokio::join!(
                self.refresh_awareness_suffix(message),
                self.refresh_active_memory_suffix(message),
                self.refresh_related_notes_suffix(message),
                self.refresh_experience_memory_trace(message),
                self.refresh_graph_memory_trace(message),
                self.prepare_full_system_prompt(model, provider_label),
            )
        };
        let (_, _, _, _, _, prepared_system_prompt) = tokio::select! {
            refreshed = refresh_turn_context => refreshed,
            _ = wait_for_cancel(cancel) => return Ok((String::new(), None)),
        };

        let client =
            crate::provider::apply_proxy(reqwest::Client::builder().user_agent(&self.user_agent))
                .build()
                .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;

        let mut activated_tool_names = self.load_activated_tool_names();
        let mut tool_inventory =
            self.build_tool_inventory(adapter.tool_provider(), &activated_tool_names);
        activated_tool_names = tool_inventory.activated_names.clone();
        let mut eager_tool_count = tool_inventory.eager_count;
        let mut deferred_tool_count = tool_inventory.deferred_count;
        let mut deferred_tool_schemas = tool_inventory.deferred_schemas;
        let mut tool_schemas = tool_inventory.schemas;

        // Static system prompt prefix (cache-friendly). Dynamic suffixes are
        // sent as independent provider-level blocks when supported.
        let system_prompt = prepared_system_prompt;
        let mut system_prompt_for_budget = self.merge_dynamic_system_prompt(system_prompt.clone());

        let compaction = self
            .run_compaction(
                &mut messages,
                &system_prompt_for_budget,
                &tool_schemas,
                model,
                MAX_OUTPUT_TOKENS,
                Some(cancel.clone()),
                on_delta,
            )
            .await;
        if compaction.summary_applied && compaction.tier_applied >= 3 {
            // Tier 3 intentionally severs detailed history references. Drop
            // the session's activation ledger so large, now-unreferenced
            // schemas do not remain pinned forever; every capability stays in
            // deferred inventory and can be rediscovered immediately.
            self.clear_tool_activations_after_summary();
            activated_tool_names.clear();
            tool_inventory = self.build_tool_inventory(adapter.tool_provider(), &[]);
            eager_tool_count = tool_inventory.eager_count;
            deferred_tool_count = tool_inventory.deferred_count;
            deferred_tool_schemas = tool_inventory.deferred_schemas;
            tool_schemas = tool_inventory.schemas;
        }

        let mut system_prompt = system_prompt;
        self.select_memories_if_needed(&mut system_prompt, message)
            .await;
        self.apply_engine_prompt_addition(&mut system_prompt);

        // Snapshot cache-safe params for side_query reuse (prompt cache sharing).
        // Must run AFTER compaction + memory selection so the snapshot matches
        // what the next API request actually sends.
        self.save_cache_safe_params(
            system_prompt.clone(),
            tool_schemas.clone(),
            messages.clone(),
            model,
        );

        let max_rounds_cfg = super::config::get_max_tool_rounds(&self.agent_id);
        let max_rounds = if max_rounds_cfg == 0 {
            u32::MAX
        } else {
            max_rounds_cfg
        };
        let round_limit_enabled = max_rounds_cfg != 0;
        let mut round_count: u32 = 0;
        let mut natural_exit = false;
        let mut collected_text = String::new();
        // Text from the terminal no-tool round only. Earlier tool-round text is
        // already persisted by append_round_to_history so replaying it as the
        // final assistant message would make the model see duplicate narration.
        let mut final_assistant_text = String::new();
        let mut terminal_round_persisted = false;
        // Text from the latest round that has not yet been committed to provider
        // history. If the user stops before the round reaches a normal exit or
        // tool-history append, this becomes the model-visible partial assistant.
        let mut pending_terminal_text = String::new();
        let mut collected_thinking = String::new();
        let mut last_round_thinking = String::new();
        let mut total_usage = ChatUsage::default();
        let mut first_ttft_ms: Option<u64> = None;
        let mut mid_loop_compaction_state = MidLoopCompactionState::default();

        // Coerce the generic `&F` to a `&dyn` once for trait method calls.
        // Generic emit_* helpers continue to use `on_delta` directly (zero
        // dispatch overhead in the hot SSE path).
        let on_delta_dyn: &(dyn Fn(&str) + Send + Sync) = on_delta;

        // Vision bridge (issue #434): when the main model can't see images and a
        // vision model is configured, prepare it once for the turn. Per round
        // (below, at api_messages build) we transcribe any not-yet-cached images
        // and rewrite the ephemeral api_messages copy so a text-only model gets
        // text descriptions instead of raw images. `None` when the main model is
        // vision-capable or no bridge is configured → existing behavior.
        let vision_bridge = if self
            .provider_config
            .as_deref()
            .map(|pc| !pc.model_supports_vision(model))
            .unwrap_or(false)
        {
            super::vision_bridge::prepare(self.session_id.as_deref(), self.session_is_incognito())
        } else {
            None
        };
        let mut vision_notice_sent = false;

        // LSP diagnostics injection. The workspace dir is resolved lazily and
        // memoized (at most one SQL lookup per turn), but the cheap global gate
        // `has_any_diagnostics()` is re-checked EVERY round: a diagnostic first
        // introduced mid-turn by write/edit/apply_patch populates the cache, so a
        // later round must still surface it — a once-per-turn snapshot taken
        // before any edit would miss it. `messages` here ends the pre-turn
        // history; only tool activity appended from now counts as "touched this
        // turn". Incognito never surfaces diagnostics.
        let lsp_incognito = self.session_is_incognito();
        let lsp_turn_start_index = messages.len();
        let mut lsp_working_dir_memo: Option<Option<String>> = None;

        let mut round: u32 = 0;
        let mut effective_max_rounds = max_rounds;
        let mut activation_grace_used = false;
        while round < effective_max_rounds {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            round_count = round + 1;

            // Keep this session marked as "active" during long tool loops
            // so peer sessions see it in the registry.
            if let Some(ref sid) = self.session_id {
                crate::awareness::touch_active_session(sid);
            }

            // Mid-turn plan-state probe (round head): catches transitions
            // that happened between rounds. `maybe_resync_plan_mode_from_backend`
            // updates ALL plan slots together (mode, allow_paths,
            // plan_extra_context) so when it returns true we just have to
            // rebuild dependent artifacts: tool_schemas (LLM sees new
            // tools next round) and the round's system_prompt mut local
            // (LLM sees new plan contract next round). Prompt cache may
            // miss for this round — acceptable cost since plan-state
            // changes happen 1-2× per turn at most.
            //
            // Honors the externally-locked flag: spawn-supplied PlanAgent
            // child sessions (plan_subagent) skip the probe entirely.
            if self.maybe_resync_plan_mode_from_backend().await {
                tool_inventory =
                    self.build_tool_inventory(adapter.tool_provider(), &activated_tool_names);
                activated_tool_names = tool_inventory.activated_names.clone();
                eager_tool_count = tool_inventory.eager_count;
                deferred_tool_count = tool_inventory.deferred_count;
                deferred_tool_schemas = tool_inventory.deferred_schemas;
                tool_schemas = tool_inventory.schemas;
                system_prompt = self.prepare_full_system_prompt(model, provider_label).await;
                system_prompt_for_budget = self.merge_dynamic_system_prompt(system_prompt.clone());
                self.select_memories_if_needed(&mut system_prompt, message)
                    .await;
                self.apply_engine_prompt_addition(&mut system_prompt);
            }

            // Drain steer mailbox: inject any pending steer messages as user msgs.
            if let Some(ref rid) = self.steer_run_id {
                for msg in crate::subagent::SUBAGENT_MAILBOX.drain(rid) {
                    Self::push_user_message(
                        &mut messages,
                        json!(format!("[Steer from parent agent]: {}", msg)),
                    );
                }
            }

            let is_final_round = round + 1 == effective_max_rounds;
            let final_round_system_prompt;
            let round_system_prompt = if round_limit_enabled && is_final_round {
                final_round_system_prompt = format!(
                    "{}\n\n{}",
                    system_prompt,
                    final_round_handoff_guidance(max_rounds)
                );
                final_round_system_prompt.as_str()
            } else {
                system_prompt.as_str()
            };
            let mut api_messages = crate::context_compact::prepare_messages_for_api(&messages);
            // Vision bridge: transcribe not-yet-cached images (once each) and
            // rewrite this round's ephemeral api_messages in place. Round 0
            // covers user images; round N covers tool images appended by the
            // previous round. `conversation_history` is untouched (reversible).
            if let Some(ref bridge) = vision_bridge {
                let report = bridge
                    .apply(&mut api_messages, adapter.provider_format(), cancel)
                    .await;
                if !vision_notice_sent && report != super::vision_bridge::ApplyReport::Idle {
                    let status = if report == super::vision_bridge::ApplyReport::Engaged {
                        "engaged"
                    } else {
                        "unavailable"
                    };
                    on_delta(
                        &json!({
                            "type": "vision_bridge",
                            "status": status,
                            "model_id": bridge.vision_model_id(),
                        })
                        .to_string(),
                    );
                    vision_notice_sent = true;
                }
            }
            let effort_live = self.effective_reasoning_effort(reasoning_effort).await;
            let awareness_suffix = self.current_awareness_suffix();
            let active_suffix = self.current_active_memory_suffix();
            let coding_profile_suffix = self.current_coding_profile_suffix();
            let procedure_suffix = self.current_procedure_memory_suffix();
            let related_notes_suffix = self.current_related_notes_suffix();
            // Two-step: cheap existence probe first (one SQL row, no Vec
            // alloc), then list+format only when there's actually an active
            // task. Skips a full task list deserialize on every round of
            // every chat that's never used `task_create` (the common case).
            let task_reminder = self.session_id.as_deref().and_then(|sid| {
                let db = crate::get_session_db()?;
                if !db.has_active_tasks(sid).unwrap_or(false) {
                    return None;
                }
                let tasks = db.list_tasks(sid).ok()?;
                tools::task_reminder_text(&tasks)
            });
            // Fold any pending hook context (PostCompact / SessionStart(compact)
            // / Notification additionalContext, queued outside a round) into
            // this round's reminder suffix so it reaches the LLM as a system
            // reminder block. Drained once — subsequent rounds see it cleared.
            let task_reminder = match (task_reminder, self.drain_pending_hook_context()) {
                (Some(t), Some(h)) => Some(format!("{t}\n\n{h}")),
                (None, Some(h)) => Some(h),
                (other, None) => other,
            };
            // Hybrid LSP diagnostics over files touched during THIS turn only
            // (P2: slice from the turn-start index so prior-turn edits don't
            // crowd out current global diagnostics). The cheap global gate is
            // re-checked every round; the working dir is resolved at most once
            // per turn (memoized) and only once diagnostics exist — so a
            // diagnostic introduced by an earlier round's edit is picked up on
            // the next round even if the turn started clean (P1).
            let lsp_diagnostics_suffix = if !lsp_incognito && crate::lsp::has_any_diagnostics() {
                let working_dir = lsp_working_dir_memo.get_or_insert_with(|| {
                    self.lookup_session_meta()
                        .as_ref()
                        .and_then(crate::session::effective_working_dir_for_meta)
                });
                working_dir.as_deref().and_then(|wd| {
                    let start = lsp_turn_start_index.min(messages.len());
                    let touched: Vec<String> =
                        crate::context_compact::extract_file_touches(&messages[start..])
                            .into_iter()
                            .rev()
                            .take(crate::lsp::MAX_TOUCHED_FILES_FOR_DIAGNOSTICS)
                            .map(|touch| touch.path)
                            .collect();
                    crate::lsp::diagnostics_prompt_suffix_hybrid(
                        self.session_id.as_deref(),
                        Some(wd),
                        &touched,
                    )
                })
            } else {
                None
            };
            let round_prompt_cache_key =
                prompt_cache_key(self, adapter.provider_format(), model, round_system_prompt);

            crate::eval_context::ensure_model_budget(self.session_id.as_deref())?;
            let eval_max_tokens =
                crate::eval_context::remaining_output_tokens(self.session_id.as_deref())
                    .map(|remaining| remaining.clamp(1, u64::from(MAX_OUTPUT_TOKENS)) as u32)
                    .unwrap_or(MAX_OUTPUT_TOKENS);
            let req = RoundRequest {
                session_id: self.session_id.as_deref(),
                system_prompt: round_system_prompt,
                awareness_suffix: awareness_suffix.as_deref().map(|s| s.as_str()),
                active_memory_suffix: active_suffix.as_deref().map(|s| s.as_str()),
                coding_profile_suffix: coding_profile_suffix.as_deref().map(|s| s.as_str()),
                procedure_memory_suffix: procedure_suffix.as_deref().map(|s| s.as_str()),
                related_notes_suffix: related_notes_suffix.as_deref().map(|s| s.as_str()),
                lsp_diagnostics_suffix: lsp_diagnostics_suffix.as_deref(),
                task_reminder_suffix: task_reminder.as_deref(),
                tool_schemas: &tool_schemas,
                deferred_tool_schemas: &deferred_tool_schemas,
                eager_tool_count,
                deferred_tool_count,
                activated_tool_count: activated_tool_names.len(),
                prompt_cache_key: Some(round_prompt_cache_key.as_str()),
                history_for_api: &api_messages,
                reasoning_effort: effort_live.as_deref(),
                temperature: self.temperature,
                max_tokens: eval_max_tokens,
                is_final_round,
                round,
            };

            self.log_memory_context_manifest(
                adapter.provider_format().label(),
                model,
                round,
                round_system_prompt,
            );

            let model_attempt_started = std::time::Instant::now();
            if let Some(fault) =
                crate::eval_context::provider_fault_action(self.session_id.as_deref())
            {
                if fault.delay_ms > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(fault.delay_ms)).await;
                }
                if let Some(error_class) = fault.error_class {
                    let error = anyhow::anyhow!(
                        "controlled evaluation fault {} ({error_class})",
                        fault.fault_id
                    );
                    self.record_eval_model_attempt(
                        model,
                        provider_label,
                        round,
                        None,
                        None,
                        model_attempt_started.elapsed().as_millis() as u64,
                        Some(&error),
                    );
                    return Err(error);
                }
            }
            let mut outcome = match adapter.chat_round(&client, req, cancel, on_delta_dyn).await {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.record_eval_model_attempt(
                        model,
                        provider_label,
                        round,
                        None,
                        None,
                        model_attempt_started.elapsed().as_millis() as u64,
                        Some(&error),
                    );
                    return Err(error);
                }
            };
            self.record_eval_model_attempt(
                model,
                provider_label,
                round,
                Some(&outcome.usage),
                outcome.ttft_ms,
                model_attempt_started.elapsed().as_millis() as u64,
                None,
            );

            // Compact callVariants are a model-facing schema optimization
            // only. Canonicalize before concurrency classification, live
            // visibility, permission, hooks, audit, history, or dispatch.
            for tool_call in &mut outcome.tool_calls {
                let Some((canonical, _)) = tools::split_call_variant_name(&tool_call.name) else {
                    continue;
                };
                let canonical = canonical.to_string();
                if let Ok(arguments) =
                    serde_json::from_str::<serde_json::Value>(&tool_call.arguments)
                {
                    if let Some((_, normalized)) =
                        tools::normalize_call_variant(&tool_call.name, &arguments)
                    {
                        tool_call.arguments = normalized.to_string();
                    }
                }
                tool_call.name = canonical;
            }

            let raw_round_estimate = crate::context_compact::estimate_request_tokens_with_tools(
                &system_prompt_for_budget,
                &api_messages,
                &tool_schemas,
                0,
            );
            let calibrator_key = format!("{}:{model}", adapter.provider_format().label());
            if outcome.usage.context_input_tokens > 0 {
                self.token_calibrator
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .update(
                        &calibrator_key,
                        raw_round_estimate,
                        outcome.usage.context_input_tokens.min(u64::from(u32::MAX)) as u32,
                    );
            }

            if first_ttft_ms.is_none() {
                first_ttft_ms = outcome.ttft_ms;
            }
            collected_text.push_str(&outcome.text);
            pending_terminal_text = outcome.text.clone();
            collected_thinking.push_str(&outcome.thinking);
            last_round_thinking = outcome.thinking.clone();
            total_usage.accumulate_round(&outcome.usage);

            if cancel.load(Ordering::SeqCst) {
                if outcome.tool_calls.is_empty() && !outcome.provider_history_items.is_empty() {
                    adapter.append_round_to_history(&mut messages, round, &outcome, &[]);
                    terminal_round_persisted = true;
                    pending_terminal_text.clear();
                }
                break;
            }

            if adapter.loop_should_exit(&outcome) {
                natural_exit = true;
                final_assistant_text = std::mem::take(&mut pending_terminal_text);
                if !outcome.provider_history_items.is_empty() {
                    adapter.append_round_to_history(&mut messages, round, &outcome, &[]);
                    terminal_round_persisted = true;
                }
                break;
            }

            // The turn will run at least one more round (tool calls are
            // pending). Emit an interim usage snapshot so the context-usage
            // gauge reflects this round's input immediately, instead of only
            // updating once the whole tool loop finishes at `emit_usage` below.
            // `ttft` is omitted — it is a turn-level metric surfaced once with
            // the final usage event. The streaming assistant message is still
            // the latest message at this point, so the frontend (which only
            // applies usage onto a trailing assistant) picks it up.
            emit_usage(on_delta, &total_usage, model, None, false);

            // Estimate current token usage for adaptive tool output sizing.
            let raw_estimated_prompt = crate::context_compact::estimate_request_tokens_with_tools(
                &system_prompt,
                &messages,
                &tool_schemas,
                0,
            );
            let estimated_used = self
                .token_calibrator
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .calibrated_estimate(&calibrator_key, raw_estimated_prompt)
                // Compaction/tool-output sizing must fail safe when the
                // heuristic is slightly optimistic.
                .saturating_add(raw_estimated_prompt / 10)
                .saturating_add(MAX_OUTPUT_TOKENS);

            // Partition tool calls by concurrent-safety:
            //   Phase 1: parallel concurrent-safe tools (read-only)
            //   Phase 2: sequential write/exec tools
            let (concurrent_tcs, sequential_tcs): (Vec<_>, Vec<_>) = outcome
                .tool_calls
                .iter()
                .partition(|tc| tools::is_concurrent_safe(&tc.name));

            let mut executed: Vec<ExecutedTool> = Vec::new();
            let mut pending_tool_activations: Vec<String> = Vec::new();
            // A provider response can stream for minutes. Refresh agent-level
            // tool filters and approval policy immediately before execution so
            // a user revocation made while the model was responding takes
            // effect in this batch.
            self.warm_memory_agent_config().await;
            let tool_ctx = self.tool_context_with_usage(Some(estimated_used));
            let tool_durability = self.turn_durability.clone();

            // Phase 1: concurrent-safe in parallel, but BOUNDED — a single
            // assistant message with many read-only calls (e.g. N `web_fetch`)
            // must not fire N concurrent operations at once (fd / outbound-request
            // flood). A semaphore caps the in-flight count; `join_all` still
            // preserves result order, and each result tuple self-describes via
            // its own call_id so completion order never affects correctness.
            if !concurrent_tcs.is_empty() && !cancel.load(Ordering::SeqCst) {
                let futures: Vec<_> = concurrent_tcs
                    .iter()
                    .map(|tc| {
                        let cancel_clone = cancel.clone();
                        let tool_ctx = tool_ctx.clone();
                        let durability = tool_durability.clone();
                        let call_id = tc.call_id.clone();
                        let name = tc.name.clone();
                        let arguments = tc.arguments.clone();
                        async move {
                            let (result, elapsed_ms, side) = match serde_json::from_str(&arguments)
                            {
                                Ok(args) => {
                                    execute_tool_with_cancel(
                                        &name,
                                        &call_id,
                                        &args,
                                        &tool_ctx,
                                        &cancel_clone,
                                        durability,
                                        on_delta_dyn,
                                    )
                                    .await?
                                }
                                Err(e) => (
                                    {
                                        crate::eval_context::record_tool_result(
                                            tool_ctx.session_id.as_deref(),
                                            &name,
                                            &call_id,
                                            &eval_raw_tool_arguments_digest(&arguments),
                                            crate::eval_context::EvalToolOutcome::ParseError,
                                            0,
                                        );
                                        invalid_tool_arguments_result(&name, &arguments, e)
                                    },
                                    0,
                                    Default::default(),
                                ),
                            };
                            Ok::<_, anyhow::Error>((
                                call_id, name, arguments, result, elapsed_ms, side,
                            ))
                        }
                    })
                    .collect();

                // Emit all tool_call events before parallel execution starts so
                // the UI shows the in-flight set immediately.
                for tc in &concurrent_tcs {
                    emit_tool_call(on_delta, &tc.call_id, &tc.name, &tc.arguments);
                    log_tool_input(tc, round);
                }

                // A tool may have external side effects. Do not start any
                // executor until every preceding tool_call frame is durable.
                self.flush_turn_durability(crate::turn_durability::FlushReason::ToolBoundary)
                    .await?;

                // Bounded fan-out: at most MAX_CONCURRENT_SAFE_TOOLS in flight at
                // once (order preserved; each result self-describes via call_id).
                let results = run_bounded_in_order(MAX_CONCURRENT_SAFE_TOOLS, futures).await;

                for result in results {
                    let (call_id, name, arguments, result, elapsed_ms, side) = result?;
                    collect_tool_search_activations(&side, &mut pending_tool_activations);
                    log_tool_output(&call_id, &name, &result, elapsed_ms, round);
                    let is_error = result.starts_with("Tool error:");
                    let (mut clean_result, media_items) = extract_media_items(&result);
                    // Same `effective_arguments` plumbing as the sequential
                    // branch — concurrent-safe tools (read / ls / grep / find /
                    // web_fetch / MCP) also honor `PreToolUse` `updatedInput`
                    // rewrites, and dropping them here would silently
                    // audit-roll-back the rewrite in the UI, history, and
                    // `PostToolUse` hook input (the actual exec saw the patched
                    // args, but everything else saw the model's pre-rewrite
                    // shape). Mirror lines 644-675 verbatim.
                    let effective_args: &str = side
                        .effective_arguments
                        .as_deref()
                        .unwrap_or(arguments.as_str());
                    emit_tool_result(
                        on_delta,
                        &call_id,
                        &name,
                        &clean_result,
                        elapsed_ms,
                        is_error,
                        &media_items,
                        side.metadata.as_ref(),
                    );
                    // PostToolUse / PostToolUseFailure (observation): fold any
                    // hook additionalContext into the result so the LLM sees it
                    // attached to this tool on the next round. Pass the
                    // *effective* args so a validating PostToolUse hook can't
                    // be fooled by the pre-rewrite shape.
                    fire_post_tool_use_hook(
                        &tool_ctx,
                        &call_id,
                        &name,
                        effective_args,
                        &mut clean_result,
                        is_error,
                        elapsed_ms,
                    )
                    .await;
                    let persisted_arguments = effective_args.to_string();
                    executed.push(ExecutedTool {
                        call_id,
                        name,
                        arguments: persisted_arguments,
                        clean_result,
                    });
                }
            }

            // Phase 2: sequential write/exec tools.
            //
            // Per-tool plan-mode resync: a sequential tool earlier in this
            // same batch could have flipped backend plan state — most
            // importantly `enter_plan_mode` (Off → Planning after the user
            // accepts the dialog). Without re-reading state per tool the
            // remaining sequential calls would run under the batch-start
            // Off snapshot, which only blocks `write/edit/apply_patch/canvas`
            // via the live-state fallback in `resolve_tool_permission`.
            // Anything else outside the PlanAgent allow-list — `update_settings`,
            // `manage_cron`, `delete_memory`, etc. — would slip through.
            // Re-syncing here puts the live PlanAgent allow-list (and ask
            // tools, allow paths) into `ToolExecContext` for every tool.
            //
            // Concurrent phase doesn't need this hook: it only contains
            // `is_concurrent_safe` tools (read-only) which by definition
            // can't mutate plan state.
            for tc in &sequential_tcs {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }

                // Sequential tools may span user approvals and long-running
                // work. Re-check both agent.json and plan state before every
                // execution, then rebuild one coherent permission snapshot.
                self.warm_memory_agent_config().await;
                let _plan_changed = self.maybe_resync_plan_mode_from_backend().await;
                let tool_ctx = self.tool_context_with_usage(Some(estimated_used));

                emit_tool_call(on_delta, &tc.call_id, &tc.name, &tc.arguments);
                log_tool_input(tc, round);

                self.flush_turn_durability(crate::turn_durability::FlushReason::ToolBoundary)
                    .await?;

                let (result, elapsed_ms, side) = match serde_json::from_str(&tc.arguments) {
                    Ok(args) => {
                        execute_tool_with_cancel(
                            &tc.name,
                            &tc.call_id,
                            &args,
                            &tool_ctx,
                            cancel,
                            tool_durability.clone(),
                            on_delta_dyn,
                        )
                        .await?
                    }
                    Err(e) => (
                        {
                            crate::eval_context::record_tool_result(
                                tool_ctx.session_id.as_deref(),
                                &tc.name,
                                &tc.call_id,
                                &eval_raw_tool_arguments_digest(&tc.arguments),
                                crate::eval_context::EvalToolOutcome::ParseError,
                                0,
                            );
                            invalid_tool_arguments_result(&tc.name, &tc.arguments, e)
                        },
                        0,
                        Default::default(),
                    ),
                };
                collect_tool_search_activations(&side, &mut pending_tool_activations);

                // If a `PreToolUse` hook rewrote the tool input via
                // `updatedInput`, execute_tool_with_cancel has already emitted
                // and durably flushed the rewrite before acknowledging dispatch.
                // Carry the same effective args into history and PostToolUse.
                let effective_args: &str = side
                    .effective_arguments
                    .as_deref()
                    .unwrap_or(tc.arguments.as_str());

                log_tool_output(&tc.call_id, &tc.name, &result, elapsed_ms, round);
                let is_error = result.starts_with("Tool error:");
                let (mut clean_result, media_items) = extract_media_items(&result);
                emit_tool_result(
                    on_delta,
                    &tc.call_id,
                    &tc.name,
                    &clean_result,
                    elapsed_ms,
                    is_error,
                    &media_items,
                    side.metadata.as_ref(),
                );
                // PostToolUse / PostToolUseFailure (observation): fold any hook
                // additionalContext into the result so the LLM sees it attached
                // to this tool on the next round. Pass the *effective* args
                // (post-PreToolUse rewrite) so a validating PostToolUse hook
                // can't be fooled by the pre-rewrite shape.
                fire_post_tool_use_hook(
                    &tool_ctx,
                    &tc.call_id,
                    &tc.name,
                    effective_args,
                    &mut clean_result,
                    is_error,
                    elapsed_ms,
                )
                .await;
                executed.push(ExecutedTool {
                    call_id: tc.call_id.clone(),
                    name: tc.name.clone(),
                    arguments: effective_args.to_string(),
                    clean_result,
                });
            }

            // A discovery result becomes callable only after its provider
            // schema is present. Rebuild from the live-gated inventory now so
            // the very next API round can call it. Persist only names that
            // survived all current gates.
            if !pending_tool_activations.is_empty() {
                let mut requested = activated_tool_names.clone();
                for name in pending_tool_activations {
                    if !requested.contains(&name) {
                        requested.push(name);
                    }
                }
                tool_inventory = self.build_tool_inventory(adapter.tool_provider(), &requested);
                let valid_new: Vec<String> = tool_inventory
                    .activated_names
                    .iter()
                    .filter(|name| !activated_tool_names.contains(name))
                    .cloned()
                    .collect();
                if !valid_new.is_empty() {
                    self.record_tool_activations(&valid_new);
                    activated_tool_names = tool_inventory.activated_names.clone();
                    eager_tool_count = tool_inventory.eager_count;
                    deferred_tool_count = tool_inventory.deferred_count;
                    deferred_tool_schemas = tool_inventory.deferred_schemas;
                    tool_schemas = tool_inventory.schemas;
                    // Successful discovery on the penultimate configured
                    // round must still leave one tool-capable round. This is
                    // bounded to one extension per turn.
                    if round_limit_enabled && !activation_grace_used {
                        effective_max_rounds = effective_max_rounds.saturating_add(1);
                        activation_grace_used = true;
                    }
                }
            }

            // PostToolBatch (observation): fires once per API round after every
            // tool call in the round settles, before the round lands in
            // history. Skipped for pure-text rounds (no tools). Any
            // additionalContext is queued for the next round's reminder.
            let post_tool_batch_wd =
                crate::session::effective_session_working_dir(self.session_id.as_deref());
            if !executed.is_empty()
                && crate::hooks::scopes::any_handlers_for(
                    crate::hooks::HookEvent::PostToolBatch,
                    post_tool_batch_wd.as_deref().map(std::path::Path::new),
                )
            {
                let input = crate::hooks::HookInput::PostToolBatch {
                    common: self.hook_common_input("PostToolBatch"),
                    round,
                    tool_names: executed.iter().map(|e| e.name.clone()).collect(),
                    tool_calls: executed
                        .iter()
                        .map(|e| crate::hooks::types::ToolCallSummary {
                            tool_name: e.name.clone(),
                            tool_input: serde_json::from_str(&e.arguments)
                                .unwrap_or(serde_json::Value::Null),
                            tool_response: serde_json::Value::String(e.clean_result.clone()),
                        })
                        .collect(),
                };
                let outcome = crate::hooks::HookDispatcher::dispatch(
                    crate::hooks::HookEvent::PostToolBatch,
                    input,
                )
                .await;
                if let Some(extra) = outcome.merged_additional_context() {
                    self.push_pending_hook_context(extra);
                }
            }

            // A later model round must never observe a tool result which is
            // absent from crash recovery. Flush the whole completed batch
            // before appending it to provider-native history.
            if !executed.is_empty() {
                self.flush_turn_durability(crate::turn_durability::FlushReason::ToolResultBoundary)
                    .await?;
            }

            // Adapter writes assistant + tool_results into history in its
            // native shape (Anthropic content blocks / OpenAI tool_calls /
            // Responses function_call+function_call_output items).
            adapter.append_round_to_history(&mut messages, round, &outcome, &executed);
            pending_terminal_text.clear();

            drain_queued_turn_user_messages(self, adapter, &mut messages, on_delta).await;

            self.check_manual_memory_save(&outcome.tool_calls);

            self.maybe_compact_between_tool_rounds(
                &mut messages,
                &system_prompt_for_budget,
                &system_prompt,
                &tool_schemas,
                model,
                MAX_OUTPUT_TOKENS,
                cancel.clone(),
                &mut mid_loop_compaction_state,
                round,
                on_delta,
            )
            .await?;
            round = round.saturating_add(1);
        }

        let cancelled = cancel.load(Ordering::SeqCst);
        let hit_round_limit =
            round_limit_enabled && !cancelled && round_count >= effective_max_rounds;
        let rounds_exhausted = hit_round_limit && !natural_exit;
        if rounds_exhausted {
            let notice = emit_max_rounds_notice(on_delta, max_rounds);
            collected_text.push_str(&notice);
            final_assistant_text.push_str(&notice);
            emit_round_limit_event(on_delta, max_rounds);
        }
        if collected_text.is_empty() && !cancelled {
            return Err(anyhow::anyhow!(
                "No content received from {} API",
                provider_label
            ));
        }

        // Persist the terminal assistant message in this provider's native
        // shape. Tool-round narration was already written with its tool calls.
        let terminal_text = terminal_assistant_text_for_history(
            cancelled,
            &final_assistant_text,
            &pending_terminal_text,
        );
        if !terminal_round_persisted {
            adapter.append_final_assistant(&mut messages, terminal_text, &last_round_thinking);
        }

        *self
            .conversation_history
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = messages;

        emit_usage(on_delta, &total_usage, model, first_ttft_ms, true);

        // Log chat completion summary.
        if let Some(logger) = crate::get_logger() {
            let history_len = self
                .conversation_history
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len();
            logger.log(
                "info",
                "agent",
                "agent::chat::done",
                &format!(
                    "{} chat complete: {}chars, {} rounds, usage in={}/out={}",
                    provider_label,
                    collected_text.len(),
                    round_count,
                    total_usage.input_tokens,
                    total_usage.output_tokens
                ),
                Some(
                    json!({
                        "provider": provider_label,
                        "text_length": collected_text.len(),
                        "total_rounds": round_count,
                        "hit_round_limit": hit_round_limit,
                        "history_length": history_len,
                        "cancelled": cancelled,
                        "rounds_exhausted": rounds_exhausted,
                        "model": model,
                        "usage": {
                            "input_tokens": total_usage.input_tokens,
                            "output_tokens": total_usage.output_tokens,
                            "cache_creation": total_usage.cache_creation_input_tokens,
                            "cache_read": total_usage.cache_read_input_tokens,
                            "last_cache_creation": total_usage.last_cache_creation_input_tokens,
                            "last_cache_read": total_usage.last_cache_read_input_tokens,
                        }
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        let thinking_result = if collected_thinking.is_empty() {
            None
        } else {
            Some(collected_thinking)
        };
        let user_visible_response = if terminal_text.is_empty() {
            collected_text
        } else {
            terminal_text.to_string()
        };

        Ok((user_visible_response, thinking_result))
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_started_job_id, terminal_assistant_text_for_history};
    use crate::async_jobs::{synthetic_started_result, JobOrigin};

    #[test]
    fn extract_started_job_id_reads_synthetic_started_payload() {
        let body = synthetic_started_result("job_abc", "exec", JobOrigin::Explicit);
        assert_eq!(extract_started_job_id(&body).as_deref(), Some("job_abc"));

        let auto = synthetic_started_result("job_xyz", "web_search", JobOrigin::AutoBackgrounded);
        assert_eq!(extract_started_job_id(&auto).as_deref(), Some("job_xyz"));
    }

    #[test]
    fn terminal_history_text_preserves_cancelled_partial_reply() {
        assert_eq!(
            terminal_assistant_text_for_history(true, "", "partial before stop"),
            "partial before stop"
        );
        assert_eq!(
            terminal_assistant_text_for_history(true, "final answer", "partial before stop"),
            "final answer"
        );
        assert_eq!(
            terminal_assistant_text_for_history(false, "final answer", "partial before stop"),
            "final answer"
        );
        assert_eq!(
            terminal_assistant_text_for_history(false, "", "partial"),
            ""
        );
    }

    #[tokio::test]
    async fn run_bounded_in_order_caps_concurrency_and_preserves_order() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let inflight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let n = 20usize;
        let max = 8usize;

        let futs: Vec<_> = (0..n)
            .map(|i| {
                let inflight = inflight.clone();
                let peak = peak.clone();
                async move {
                    let cur = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(cur, Ordering::SeqCst);
                    // Yield + brief sleep so calls actually overlap in time.
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    inflight.fetch_sub(1, Ordering::SeqCst);
                    i
                }
            })
            .collect();

        let results = super::run_bounded_in_order(max, futs).await;

        let observed_peak = peak.load(Ordering::SeqCst);
        assert!(
            observed_peak <= max,
            "peak in-flight {} exceeded cap {}",
            observed_peak,
            max
        );
        assert!(
            observed_peak > 1,
            "concurrency never overlapped (peak {}); test is not exercising the bound",
            observed_peak
        );
        // Order must match input despite out-of-order completion.
        assert_eq!(results, (0..n).collect::<Vec<_>>());
    }

    #[test]
    fn extract_started_job_id_ignores_non_started_and_non_json() {
        // Plain tool output — not JSON.
        assert_eq!(extract_started_job_id("command finished, exit 0"), None);
        // JSON, but a completed/terminal result, not a backgrounded "started".
        assert_eq!(
            extract_started_job_id(r#"{"status":"completed","job_id":"j1"}"#),
            None
        );
        // Started but no job id (defensive — nothing to cancel).
        assert_eq!(extract_started_job_id(r#"{"status":"started"}"#), None);
        assert_eq!(
            extract_started_job_id(r#"{"status":"started","job_id":""}"#),
            None
        );
    }
}
