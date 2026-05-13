//! Provider-native partial reconstruction.
//!
//! Two responsibilities:
//!
//! 1. **Forward rebuild** — [`rebuild_partial_assistant_blocks`] and
//!    [`synthesize_tool_results`] turn a [`PartialMeta`] into native
//!    `context_json` items matching whatever provider shape the last
//!    LLM call used.
//! 2. **Reverse rebuild** — [`collect_partial_from_messages`] reads
//!    `messages` table rows for an in-flight turn (everything after
//!    the latest user row) and synthesizes a `PartialMeta`. Used by
//!    the startup-sweep path where the runtime collector is no longer
//!    available.
//!
//! Why 4 provider shapes mattered enough to fork: Anthropic strictly
//! validates that every `tool_use` has a matching `tool_result` in the
//! next user message (otherwise 400); OpenAI Chat uses a top-level
//! `tool_calls` array and `role=tool` follow-ups; Responses / Codex
//! emit `function_call` / `function_call_output` items at the top level
//! (no nesting under a message). Mixing shapes in the wrong slot is
//! the most common source of "previous turn replayed but the model
//! 400-rejected the next request".

use std::sync::Arc;

use serde_json::{json, Value};

use crate::session::{MessageRole, SessionDB, SessionMessage};

use super::{ExecutedTool, PartialMeta, PendingToolCall, ProviderApiKind};

/// Synthetic `tool_result` body written when a tool_use never produced
/// a real result (interrupted by user / crash / shutdown / provider
/// failure). All four provider shapes accept this string verbatim; per-
/// reason wording is the model marker, not the tool result.
pub const INTERRUPTED_TOOL_RESULT: &str = "Tool execution was interrupted";

/// Resolve the provider shape used by a session by joining the session
/// row's `provider_id` against the cached provider catalog. Returns
/// `None` when the session has no recorded provider (history-only
/// rows) or the provider has been removed from config — callers fall
/// back to the rebuild path's default shape in that case.
///
/// Shared between the startup sweep and the shutdown signal handler;
/// both need the same lookup but neither can take the runtime
/// `last_provider_api_kind` (the engine is already returned or never
/// ran).
pub fn resolve_provider_kind_for_session(
    db: &Arc<SessionDB>,
    session_id: &str,
) -> Option<ProviderApiKind> {
    let provider_id = db
        .get_session(session_id)
        .ok()
        .flatten()
        .and_then(|sess| sess.provider_id)?;
    let config = crate::config::cached_config();
    let prov = config.providers.iter().find(|p| p.id == provider_id)?;
    Some(ProviderApiKind::from(prov.api_type.clone()))
}

// ── Forward rebuild ──────────────────────────────────────────────────

/// Emit one or more `context_json` items representing the assistant
/// side of the interrupted partial round.
///
/// **Anthropic**: a single `{role:assistant, content:[blocks…]}` item
/// with `thinking → text → tool_use…` order.
///
/// **OpenAI Chat**: a single `{role:assistant, content, reasoning_content,
/// tool_calls}` message (omitting fields that are empty).
///
/// **OpenAI Responses / Codex**: an output message item (with merged
/// text — Responses reasoning items require an `encrypted_content` we
/// don't have for runtime partials, so we fold thinking into the text),
/// followed by zero or more `function_call` items at top level.
///
/// Returns `[]` when there is genuinely nothing to emit (no text, no
/// thinking, no tool calls) — the marker step still runs so the model
/// still sees `[系统事件] ...`.
pub fn rebuild_partial_assistant_blocks(partial: &PartialMeta) -> Vec<Value> {
    let kind = partial.provider_kind.unwrap_or(ProviderApiKind::Anthropic);
    let text = partial
        .text
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let thinking = partial
        .thinking
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    match kind {
        ProviderApiKind::Anthropic => rebuild_anthropic(text, thinking, &partial.tool_calls),
        ProviderApiKind::OpenAIChat => rebuild_openai_chat(text, thinking, &partial.tool_calls),
        ProviderApiKind::OpenAIResponses | ProviderApiKind::Codex => {
            rebuild_openai_responses(text, thinking, &partial.tool_calls)
        }
    }
}

fn rebuild_anthropic(
    text: Option<&str>,
    thinking: Option<&str>,
    tool_calls: &[PendingToolCall],
) -> Vec<Value> {
    let mut content: Vec<Value> = Vec::new();
    if let Some(t) = thinking {
        content.push(json!({"type": "thinking", "thinking": t}));
    }
    if let Some(t) = text {
        content.push(json!({"type": "text", "text": t}));
    }
    for tc in tool_calls {
        // Anthropic expects `input` as a parsed JSON object; fall back
        // to an empty object on malformed args so we don't poison the
        // next request with a string-shaped input.
        let input: Value = serde_json::from_str(&tc.arguments).unwrap_or_else(|_| json!({}));
        content.push(json!({
            "type": "tool_use",
            "id": tc.call_id,
            "name": tc.name,
            "input": input,
        }));
    }
    if content.is_empty() {
        return Vec::new();
    }
    vec![json!({"role": "assistant", "content": content})]
}

fn rebuild_openai_chat(
    text: Option<&str>,
    thinking: Option<&str>,
    tool_calls: &[PendingToolCall],
) -> Vec<Value> {
    if text.is_none() && thinking.is_none() && tool_calls.is_empty() {
        return Vec::new();
    }
    let mut msg = serde_json::Map::new();
    msg.insert("role".to_string(), json!("assistant"));
    if let Some(t) = text {
        msg.insert("content".to_string(), json!(t));
    }
    if let Some(t) = thinking {
        msg.insert("reasoning_content".to_string(), json!(t));
    }
    if !tool_calls.is_empty() {
        let tc_array: Vec<Value> = tool_calls
            .iter()
            .map(|tc| {
                json!({
                    "id": tc.call_id,
                    "type": "function",
                    "function": {
                        "name": tc.name,
                        // Chat keeps arguments as a JSON-encoded string
                        // (the API itself serializes them that way over
                        // SSE), so we preserve the raw string.
                        "arguments": tc.arguments,
                    }
                })
            })
            .collect();
        msg.insert("tool_calls".to_string(), json!(tc_array));
    }
    vec![Value::Object(msg)]
}

fn rebuild_openai_responses(
    text: Option<&str>,
    thinking: Option<&str>,
    tool_calls: &[PendingToolCall],
) -> Vec<Value> {
    let mut items: Vec<Value> = Vec::new();

    // Responses expects reasoning items with `encrypted_content`; we
    // can't recreate that from a runtime partial, so fold thinking
    // into the output_text body instead. The model still sees its own
    // chain-of-thought (which is the contract here) — only the
    // server-side encrypted reasoning is lost (and would be invalid
    // anyway).
    let merged = match (thinking, text) {
        (Some(t), Some(x)) => format!("{}\n\n{}", t, x),
        (Some(t), None) => t.to_string(),
        (None, Some(x)) => x.to_string(),
        (None, None) => String::new(),
    };
    if !merged.is_empty() {
        items.push(json!({
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": merged}],
            "status": "completed",
        }));
    }

    for tc in tool_calls {
        // Responses requires both `id` and `call_id` populated with
        // the same value — server returns 400 otherwise.
        items.push(json!({
            "type": "function_call",
            "id": tc.call_id,
            "call_id": tc.call_id,
            "name": tc.name,
            "arguments": tc.arguments,
        }));
    }

    items
}

// ── Tool-result synthesis ────────────────────────────────────────────

/// Emit the matching `tool_result` payload for every tool_use that
/// didn't get a real result. Already-completed tool calls (where
/// `partial.executed_tools` has the call_id) emit their *real* result
/// instead of the synthetic marker.
///
/// For **Anthropic**, returns a single `{role:user, content:[…]}` item
/// containing one `tool_result` block per call — Anthropic groups all
/// tool_results into one user message that immediately follows the
/// assistant.
///
/// For **OpenAI Chat**, returns one `{role:tool, tool_call_id, content}`
/// message per call.
///
/// For **Responses / Codex**, returns one `{type:function_call_output,
/// call_id, output}` item per call.
///
/// The order matches `partial.tool_calls`. When there are no tool calls,
/// returns `[]`.
pub fn synthesize_tool_results(partial: &PartialMeta, marker: &str) -> Vec<Value> {
    if partial.tool_calls.is_empty() {
        return Vec::new();
    }
    let kind = partial.provider_kind.unwrap_or(ProviderApiKind::Anthropic);

    match kind {
        ProviderApiKind::Anthropic => {
            let blocks: Vec<Value> = partial
                .tool_calls
                .iter()
                .map(|tc| {
                    let (content, is_error) =
                        resolve_tool_result(tc, &partial.executed_tools, marker);
                    let mut obj = serde_json::Map::new();
                    obj.insert("type".to_string(), json!("tool_result"));
                    obj.insert("tool_use_id".to_string(), json!(tc.call_id));
                    obj.insert("content".to_string(), json!(content));
                    if is_error {
                        obj.insert("is_error".to_string(), json!(true));
                    }
                    Value::Object(obj)
                })
                .collect();
            vec![json!({"role": "user", "content": blocks})]
        }
        ProviderApiKind::OpenAIChat => partial
            .tool_calls
            .iter()
            .map(|tc| {
                let (content, _) = resolve_tool_result(tc, &partial.executed_tools, marker);
                json!({
                    "role": "tool",
                    "tool_call_id": tc.call_id,
                    "content": content,
                })
            })
            .collect(),
        ProviderApiKind::OpenAIResponses | ProviderApiKind::Codex => partial
            .tool_calls
            .iter()
            .map(|tc| {
                let (content, _) = resolve_tool_result(tc, &partial.executed_tools, marker);
                json!({
                    "type": "function_call_output",
                    "call_id": tc.call_id,
                    "output": content,
                })
            })
            .collect(),
    }
}

fn resolve_tool_result(
    tc: &PendingToolCall,
    executed: &[ExecutedTool],
    marker: &str,
) -> (String, bool) {
    if let Some(e) = executed.iter().find(|e| e.call_id == tc.call_id) {
        return (e.result.clone(), e.is_error);
    }
    (marker.to_string(), true)
}

// ── Reverse rebuild (startup sweep path) ─────────────────────────────

/// Reverse-engineer a [`PartialMeta`] for an in-flight turn from the
/// `messages` table. Used by the startup sweep to finalize stale turns
/// that died before their runtime collector got a chance to run.
///
/// Reads every row after the latest `role=user` row (the in-flight
/// turn slice) via [`SessionDB::load_current_turn_tail`] and folds:
/// - all `text_block` content → `text`
/// - all `thinking_block` content → `thinking`
/// - every `tool` row → `tool_calls` (and `executed_tools` when there
///   is a real `tool_result` already on disk)
///
/// `provider_kind` must be supplied by the caller; the sweep path
/// resolves it from `sessions.provider_id` + the provider catalog
/// (because `messages.model` is just a string and round-trip from that
/// to a `ProviderApiKind` would re-introduce string parsing here).
pub fn collect_partial_from_messages(
    db: &Arc<SessionDB>,
    session_id: &str,
    provider_kind: Option<ProviderApiKind>,
) -> PartialMeta {
    let user_message = db
        .last_user_message(session_id)
        .ok()
        .flatten()
        .map(|m| m.content);
    let tail = db.load_current_turn_tail(session_id).unwrap_or_default();

    let mut text_parts: Vec<String> = Vec::new();
    let mut thinking_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<PendingToolCall> = Vec::new();
    let mut executed_tools: Vec<ExecutedTool> = Vec::new();
    let mut assistant_message_id: Option<i64> = None;

    for msg in tail {
        match msg.role {
            MessageRole::TextBlock => {
                let c = msg.content.trim();
                if !c.is_empty() {
                    text_parts.push(c.to_string());
                }
            }
            MessageRole::ThinkingBlock => {
                let c = msg.content.trim();
                if !c.is_empty() {
                    thinking_parts.push(c.to_string());
                }
            }
            MessageRole::Tool => collect_tool_row(&msg, &mut tool_calls, &mut executed_tools),
            MessageRole::Assistant => {
                // A partial assistant row already merged via
                // `persist_failed_partial_assistant`. Record its id so
                // `chat_turns.assistant_message_id` lands on it; the
                // content itself is already in `context_json` (the
                // failed-attempt persister writes both).
                assistant_message_id = Some(msg.id);
                let c = msg.content.trim();
                if !c.is_empty() {
                    text_parts.push(c.to_string());
                }
            }
            _ => {}
        }
    }

    PartialMeta {
        user_message,
        provider_kind,
        text: opt_join(&text_parts),
        thinking: opt_join(&thinking_parts),
        tool_calls,
        executed_tools,
        round_id: None,
        turn_id: None,
        assistant_message_id,
    }
}

fn collect_tool_row(
    msg: &SessionMessage,
    tool_calls: &mut Vec<PendingToolCall>,
    executed_tools: &mut Vec<ExecutedTool>,
) {
    let Some(call_id) = msg.tool_call_id.clone() else {
        return;
    };
    let name = msg.tool_name.clone().unwrap_or_default();
    let arguments = msg.tool_arguments.clone().unwrap_or_default();
    // `tool_duration_ms` is the durable signal that
    // `update_tool_result_with_metadata` actually wrote a result —
    // an empty result string + non-null duration is a legitimate
    // completed MCP tool (some servers return empty content); only
    // a missing duration means the row never finalized.
    let has_result = msg.tool_duration_ms.is_some();

    tool_calls.push(PendingToolCall {
        call_id: call_id.clone(),
        name: name.clone(),
        arguments: arguments.clone(),
        has_result,
    });

    if has_result {
        executed_tools.push(ExecutedTool {
            call_id,
            name,
            arguments,
            result: msg.tool_result.clone().unwrap_or_default(),
            is_error: msg.is_error.unwrap_or(false),
        });
    }
}

fn opt_join(parts: &[String]) -> Option<String> {
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("\n\n"))
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(id: &str, name: &str, args: &str) -> PendingToolCall {
        PendingToolCall {
            call_id: id.into(),
            name: name.into(),
            arguments: args.into(),
            has_result: false,
        }
    }

    fn meta_with(
        kind: ProviderApiKind,
        text: Option<&str>,
        thinking: Option<&str>,
        tool_calls: Vec<PendingToolCall>,
        executed: Vec<ExecutedTool>,
    ) -> PartialMeta {
        PartialMeta {
            user_message: None,
            provider_kind: Some(kind),
            text: text.map(str::to_owned),
            thinking: thinking.map(str::to_owned),
            tool_calls,
            executed_tools: executed,
            round_id: None,
            turn_id: None,
            assistant_message_id: None,
        }
    }

    #[test]
    fn rebuild_anthropic_with_thinking_text_and_tool_use() {
        let partial = meta_with(
            ProviderApiKind::Anthropic,
            Some("hello world"),
            Some("let me think"),
            vec![pending("call_1", "exec", r#"{"cmd":"ls"}"#)],
            vec![],
        );
        let items = rebuild_partial_assistant_blocks(&partial);
        assert_eq!(items.len(), 1);
        let content = &items[0]["content"];
        assert_eq!(content[0]["type"], "thinking");
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[2]["type"], "tool_use");
        assert_eq!(content[2]["id"], "call_1");
        assert_eq!(content[2]["input"]["cmd"], "ls");
    }

    #[test]
    fn rebuild_anthropic_empty_returns_no_items() {
        let partial = meta_with(ProviderApiKind::Anthropic, None, None, vec![], vec![]);
        assert!(rebuild_partial_assistant_blocks(&partial).is_empty());
    }

    #[test]
    fn rebuild_openai_chat_omits_empty_fields_and_keeps_args_as_string() {
        let partial = meta_with(
            ProviderApiKind::OpenAIChat,
            Some("ok"),
            None,
            vec![pending("c1", "exec", r#"{"a":1}"#)],
            vec![],
        );
        let items = rebuild_partial_assistant_blocks(&partial);
        assert_eq!(items.len(), 1);
        let msg = &items[0];
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["content"], "ok");
        assert!(msg.get("reasoning_content").is_none());
        let tc = &msg["tool_calls"][0];
        assert_eq!(tc["id"], "c1");
        // arguments must remain a string per OpenAI spec.
        assert!(tc["function"]["arguments"].is_string());
    }

    #[test]
    fn rebuild_openai_responses_merges_thinking_into_output_text() {
        let partial = meta_with(
            ProviderApiKind::OpenAIResponses,
            Some("answer"),
            Some("reason"),
            vec![pending("c1", "exec", r#"{}"#)],
            vec![],
        );
        let items = rebuild_partial_assistant_blocks(&partial);
        // First item is the assistant message, second is the function_call.
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["type"], "message");
        let text = items[0]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("reason"));
        assert!(text.contains("answer"));
        assert!(text.find("reason").unwrap() < text.find("answer").unwrap());
        assert_eq!(items[1]["type"], "function_call");
        assert_eq!(items[1]["id"], "c1");
        assert_eq!(items[1]["call_id"], "c1");
    }

    #[test]
    fn synthesize_tool_results_anthropic_groups_into_one_user_message() {
        let partial = meta_with(
            ProviderApiKind::Anthropic,
            None,
            None,
            vec![
                pending("c1", "exec", r#"{}"#),
                pending("c2", "read", r#"{}"#),
            ],
            vec![],
        );
        let items = synthesize_tool_results(&partial, INTERRUPTED_TOOL_RESULT);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["role"], "user");
        let blocks = items[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "tool_result");
        assert_eq!(blocks[0]["tool_use_id"], "c1");
        assert_eq!(blocks[0]["content"], INTERRUPTED_TOOL_RESULT);
        assert_eq!(blocks[0]["is_error"], true);
        assert_eq!(blocks[1]["tool_use_id"], "c2");
    }

    #[test]
    fn synthesize_tool_results_chat_one_msg_per_call() {
        let partial = meta_with(
            ProviderApiKind::OpenAIChat,
            None,
            None,
            vec![pending("c1", "exec", r#"{}"#), pending("c2", "x", r#"{}"#)],
            vec![],
        );
        let items = synthesize_tool_results(&partial, INTERRUPTED_TOOL_RESULT);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["role"], "tool");
        assert_eq!(items[0]["tool_call_id"], "c1");
        assert_eq!(items[0]["content"], INTERRUPTED_TOOL_RESULT);
    }

    #[test]
    fn synthesize_tool_results_responses_emits_function_call_output() {
        let partial = meta_with(
            ProviderApiKind::OpenAIResponses,
            None,
            None,
            vec![pending("c1", "exec", r#"{}"#)],
            vec![],
        );
        let items = synthesize_tool_results(&partial, INTERRUPTED_TOOL_RESULT);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "function_call_output");
        assert_eq!(items[0]["call_id"], "c1");
        assert_eq!(items[0]["output"], INTERRUPTED_TOOL_RESULT);
    }

    #[test]
    fn synthesize_uses_real_result_when_executed_tool_present() {
        let mut partial = meta_with(
            ProviderApiKind::Anthropic,
            None,
            None,
            vec![pending("c1", "exec", r#"{}"#)],
            vec![ExecutedTool {
                call_id: "c1".into(),
                name: "exec".into(),
                arguments: "{}".into(),
                result: "real output".into(),
                is_error: false,
            }],
        );
        partial.tool_calls[0].has_result = true;
        let items = synthesize_tool_results(&partial, INTERRUPTED_TOOL_RESULT);
        let blocks = items[0]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["content"], "real output");
        assert!(blocks[0].get("is_error").is_none());
    }

    #[test]
    fn synthesize_returns_empty_when_no_tool_calls() {
        let partial = meta_with(ProviderApiKind::Anthropic, None, None, vec![], vec![]);
        assert!(synthesize_tool_results(&partial, INTERRUPTED_TOOL_RESULT).is_empty());
    }
}
