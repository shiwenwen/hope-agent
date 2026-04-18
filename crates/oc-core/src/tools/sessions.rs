use anyhow::Result;
use serde_json::Value;

use crate::session::{MessageRole, NewMessage};

/// Tool: sessions_list — list all chat sessions with metadata.
pub(crate) async fn tool_sessions_list(args: &Value) -> Result<String> {
    let agent_id = args.get("agent_id").and_then(|v| v.as_str());
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .min(100) as usize;
    let include_cron = args
        .get("include_cron")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let db = crate::get_session_db()
        .ok_or_else(|| anyhow::anyhow!("Session database not initialized"))?;

    let sessions = db.list_sessions(agent_id)?;

    let filtered: Vec<_> = sessions
        .into_iter()
        .filter(|s| include_cron || !s.is_cron)
        .take(limit)
        .collect();

    if filtered.is_empty() {
        return Ok("No sessions found.".to_string());
    }

    let mut output = format!("Sessions ({}):\n", filtered.len());

    for (i, s) in filtered.iter().enumerate() {
        let title = s.title.as_deref().unwrap_or("(untitled)");
        let model = s.model_id.as_deref().unwrap_or("unknown");
        output.push_str(&format!(
            "\n{}. [{}] \"{}\" (agent: {})\n   Model: {} | Messages: {} | Unread: {} | Updated: {}\n",
            i + 1, s.id, title, s.agent_id, model, s.message_count, s.unread_count, s.updated_at,
        ));

        if s.is_cron {
            output.push_str("   [cron]\n");
        }
        if let Some(parent) = &s.parent_session_id {
            output.push_str(&format!("   Parent: {}\n", parent));
        }
    }

    Ok(output)
}

/// Tool: session_status — query detailed status of a specific session.
pub(crate) async fn tool_session_status(args: &Value) -> Result<String> {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'session_id' parameter"))?;

    let db = crate::get_session_db()
        .ok_or_else(|| anyhow::anyhow!("Session database not initialized"))?;

    match db.get_session(session_id)? {
        Some(s) => {
            let title = s.title.as_deref().unwrap_or("(untitled)");
            let provider = s.provider_name.as_deref().unwrap_or("unknown");
            let model = s.model_id.as_deref().unwrap_or("unknown");
            let parent = s.parent_session_id.as_deref().unwrap_or("none");

            Ok(format!(
                "Session: {}\n\
                 Title: \"{}\"\n\
                 Agent: {}\n\
                 Provider: {} ({})\n\
                 Messages: {} ({} unread)\n\
                 Created: {}\n\
                 Updated: {}\n\
                 Is Cron: {}\n\
                 Parent Session: {}",
                s.id,
                title,
                s.agent_id,
                provider,
                model,
                s.message_count,
                s.unread_count,
                s.created_at,
                s.updated_at,
                s.is_cron,
                parent,
            ))
        }
        None => Ok(format!("Session '{}' not found.", session_id)),
    }
}

/// Tool: sessions_history — get paginated chat history from a session.
pub(crate) async fn tool_sessions_history(args: &Value) -> Result<String> {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'session_id' parameter"))?;

    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .min(200) as u32;

    let before_id = args.get("before_id").and_then(|v| v.as_i64());

    let include_tools = args
        .get("include_tools")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let db = crate::get_session_db()
        .ok_or_else(|| anyhow::anyhow!("Session database not initialized"))?;

    // Verify session exists
    let session = db
        .get_session(session_id)?
        .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", session_id))?;

    let (messages, total) = if let Some(bid) = before_id {
        let (msgs, _has_more) = db.load_session_messages_before(session_id, bid, limit)?;
        let len = msgs.len() as u32;
        (msgs, len) // approximate; before_id mode doesn't return total
    } else {
        let (msgs, total, _has_more) = db.load_session_messages_latest(session_id, limit)?;
        (msgs, total)
    };

    // Filter tool/text_block messages unless requested
    let filtered: Vec<_> = messages
        .into_iter()
        .filter(|m| {
            if include_tools {
                return true;
            }
            !matches!(m.role, MessageRole::Tool | MessageRole::TextBlock)
        })
        .collect();

    let title = session.title.as_deref().unwrap_or("(untitled)");
    let mut output = format!(
        "Session \"{}\" — {} messages (total: {}):\n",
        title,
        filtered.len(),
        total,
    );

    const MAX_OUTPUT_BYTES: usize = 80 * 1024; // 80KB cap
    const TOOL_RESULT_MAX: usize = 500;
    const TOOL_ARGS_MAX: usize = 200;

    for msg in &filtered {
        let entry = match msg.role {
            MessageRole::User => {
                let content = truncate_str(&msg.content, 2000);
                format!("\n[#{}] user ({}):\n  {}\n", msg.id, msg.timestamp, content)
            }
            MessageRole::Assistant => {
                let model_str = msg.model.as_deref().unwrap_or("");
                let model_suffix = if model_str.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", model_str)
                };
                let content = truncate_str(&msg.content, 4000);
                format!(
                    "\n[#{}] assistant ({}){}:\n  {}\n",
                    msg.id, msg.timestamp, model_suffix, content
                )
            }
            MessageRole::Tool => {
                let name = msg.tool_name.as_deref().unwrap_or("unknown");
                let duration = msg
                    .tool_duration_ms
                    .map(|d| format!(" [{}ms]", d))
                    .unwrap_or_default();
                let args_str = msg
                    .tool_arguments
                    .as_deref()
                    .map(|a| format!("\n  Args: {}", truncate_str(a, TOOL_ARGS_MAX)))
                    .unwrap_or_default();
                let result_str = msg
                    .tool_result
                    .as_deref()
                    .map(|r| format!("\n  Result: {}", truncate_str(r, TOOL_RESULT_MAX)))
                    .unwrap_or_default();
                format!(
                    "\n[#{}] tool: {} ({}){}{}{}\n",
                    msg.id, name, msg.timestamp, duration, args_str, result_str
                )
            }
            MessageRole::Event => {
                format!(
                    "\n[#{}] event ({}): {}\n",
                    msg.id,
                    msg.timestamp,
                    truncate_str(&msg.content, 500)
                )
            }
            MessageRole::TextBlock => {
                format!(
                    "\n[#{}] text ({}):\n  {}\n",
                    msg.id,
                    msg.timestamp,
                    truncate_str(&msg.content, 2000)
                )
            }
            MessageRole::ThinkingBlock => {
                format!(
                    "\n[#{}] thinking ({}):\n  {}\n",
                    msg.id,
                    msg.timestamp,
                    truncate_str(&msg.content, 2000)
                )
            }
        };

        if output.len() + entry.len() > MAX_OUTPUT_BYTES {
            output.push_str(&format!(
                "\n... output truncated at {}KB. Use before_id={} to load earlier messages.",
                MAX_OUTPUT_BYTES / 1024,
                filtered.last().map(|m| m.id).unwrap_or(0),
            ));
            break;
        }
        output.push_str(&entry);
    }

    Ok(output)
}

/// Tool: sessions_send — send a message to another session.
pub(crate) async fn tool_sessions_send(
    args: &Value,
    ctx: &super::execution::ToolExecContext,
) -> Result<String> {
    let target_session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'session_id' parameter"))?;

    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'message' parameter"))?;

    let wait = args.get("wait").and_then(|v| v.as_bool()).unwrap_or(false);

    let timeout_secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(60)
        .min(300);

    // Prevent sending to self (infinite loop)
    if let Some(ref self_session) = ctx.session_id {
        if self_session == target_session_id {
            return Ok(
                "Error: Cannot send a message to your own session (would create a loop)."
                    .to_string(),
            );
        }
    }

    let db = crate::get_session_db()
        .ok_or_else(|| anyhow::anyhow!("Session database not initialized"))?;

    // Verify target session exists
    let session = db
        .get_session(target_session_id)?
        .ok_or_else(|| anyhow::anyhow!("Target session '{}' not found", target_session_id))?;

    // Append user message to target session
    let new_msg = NewMessage::user(message);
    db.append_message(target_session_id, &new_msg)?;

    if !wait {
        // Non-blocking: emit event for frontend to pick up, return immediately
        if let Some(bus) = crate::globals::get_event_bus() {
            bus.emit(
                "session_message_injected",
                serde_json::json!({
                    "session_id": target_session_id,
                }),
            );
        }

        return Ok(format!(
            "Message delivered to session [{}] (\"{}\"). The agent will process it asynchronously.",
            target_session_id,
            session.title.as_deref().unwrap_or("untitled"),
        ));
    }

    // Blocking: build agent inline and execute.
    // We inline the agent construction here (similar to cron::build_and_run_agent)
    // to avoid async recursion issues (sessions_send → build_and_run_agent → chat → tools → sessions_send).
    let agent_id = session.agent_id.clone();
    let session_id_owned = target_session_id.to_string();
    let message_owned = message.to_string();

    let agent_task = run_agent_for_session(&agent_id, &message_owned, &session_id_owned);

    let response =
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), agent_task).await;

    match response {
        Ok(Ok(reply)) => Ok(format!(
            "Message sent to session [{}]. Agent response:\n\n{}",
            target_session_id, reply,
        )),
        Ok(Err(e)) => Ok(format!(
            "Message delivered to session [{}], but agent execution failed: {}",
            target_session_id, e,
        )),
        Err(_) => Ok(format!(
            "Message delivered to session [{}], but agent did not respond within {} seconds.",
            target_session_id, timeout_secs,
        )),
    }
}

// ── Helpers ──────────────────────────────────────────────────────

/// Build and run an agent for a target session (used by sessions_send wait mode).
/// This is similar to cron::build_and_run_agent but with a different system context.
async fn run_agent_for_session(agent_id: &str, message: &str, session_id: &str) -> Result<String> {
    use crate::agent::AssistantAgent;
    use crate::failover;
    use crate::provider;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let store = crate::config::cached_config();
    let agent_model_config = crate::agent_loader::load_agent(agent_id)
        .map(|def| def.config.model)
        .unwrap_or_default();

    let (primary, fallbacks) = provider::resolve_model_chain(&agent_model_config, &store);

    let mut model_chain = Vec::new();
    if let Some(p) = primary {
        model_chain.push(p);
    }
    for fb in fallbacks {
        if !model_chain
            .iter()
            .any(|m| m.provider_id == fb.provider_id && m.model_id == fb.model_id)
        {
            model_chain.push(fb);
        }
    }

    if model_chain.is_empty() {
        return Err(anyhow::anyhow!(
            "No model configured for agent '{}'",
            agent_id
        ));
    }

    let mut last_error = String::new();
    for (idx, model_ref) in model_chain.iter().enumerate() {
        let prov = match provider::find_provider(&store.providers, &model_ref.provider_id) {
            Some(p) => p,
            None => continue,
        };

        let model_label = format!("{}::{}", model_ref.provider_id, model_ref.model_id);
        let mut retry_count: u32 = 0;

        loop {
            let mut agent = AssistantAgent::new_from_provider(prov, &model_ref.model_id);
            agent.set_agent_id(agent_id);
            agent.set_session_id(session_id);
            agent.set_extra_system_context(
                "## Execution Context\n\
                 You are responding to a cross-session message. Another agent or session sent you this message.\n\
                 - Respond concisely and directly to the message content.\n\
                 - This is an isolated execution with no prior conversation history."
                .to_string()
            );

            let cancel = Arc::new(AtomicBool::new(false));
            match agent.chat(message, &[], None, cancel, |_delta| {}).await {
                Ok((response, _thinking)) => {
                    if idx > 0 {
                        app_info!(
                            "tool",
                            "sessions_send",
                            "Fallback model {} succeeded",
                            model_label
                        );
                    }
                    return Ok(response);
                }
                Err(e) => {
                    last_error = e.to_string();
                    let reason = failover::classify_error(&last_error);

                    if reason.is_terminal() {
                        return Err(anyhow::anyhow!("{}", last_error));
                    }

                    if reason.is_retryable() && retry_count < 2 {
                        retry_count += 1;
                        let delay = failover::retry_delay_ms(retry_count - 1, 1000, 10_000);
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue;
                    }

                    break;
                }
            }
        }
    }

    Err(anyhow::anyhow!(
        "All models failed. Last error: {}",
        last_error
    ))
}

fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    // Find a valid UTF-8 boundary near max
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
