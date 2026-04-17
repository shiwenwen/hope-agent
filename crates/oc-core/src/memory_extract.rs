//! Auto-extraction of memories from conversations.
//!
//! After a chat completion, this module can extract valuable information
//! (user facts, preferences, project context) and save them as memories.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use crate::agent::AssistantAgent;
use crate::memory::{AddResult, MemoryScope, MemoryType, NewMemory};

/// Pick the scope to use when auto-saving a memory extracted from `session_id`.
///
/// If the session belongs to a project (looked up via the global [`crate::get_session_db`]),
/// the memory is scoped to that project. Otherwise it falls back to the
/// agent's private scope, matching pre-project behavior.
fn resolve_extract_scope(session_id: &str, agent_id: &str) -> MemoryScope {
    if let Some(db) = crate::get_session_db() {
        if let Ok(Some(session)) = db.get_session(session_id) {
            if let Some(pid) = session.project_id {
                return MemoryScope::Project { id: pid };
            }
        }
    }
    MemoryScope::Agent {
        id: agent_id.to_string(),
    }
}

// ── Extraction Prompts ──────────────────────────────────────────
//
// Phase B'2 introduces the COMBINED prompt: one side_query returns both
// factual items AND reflective profile traits. The old legacy prompt is
// kept as a fallback for when `enable_reflection=false`, so operators can
// roll back without losing extraction quality.

const EXTRACTION_PROMPT: &str = r#"Extract any new, memorable facts from the conversation below.
Return a JSON array. Each item: {"content":"...","type":"user|feedback|project|reference","tags":["..."]}

Types:
- "user": facts about the user (name, location, preferences, expertise, role)
- "feedback": user preferences about AI behavior (response style, things to avoid)
- "project": technical/project facts (tech stack, architecture, goals, deadlines)
- "reference": URLs, docs, external resources mentioned

Rules:
- Only extract NEW information not in "Known memories" below
- Be concise — each content should be 1-2 sentences
- Return [] if nothing worth remembering
- Maximum 5 items

Known memories:
{EXISTING}

Conversation (recent):
{MESSAGES}"#;

const COMBINED_EXTRACT_PROMPT: &str = r#"Output ONE JSON object with TWO arrays:
{
  "facts":    [{"content":"...","type":"user|feedback|project|reference","tags":["..."]}],
  "profile":  [{"content":"...","type":"user|feedback","tags":["profile", ...]}]
}

"facts" rules (factual extraction — same as before):
- Extract NEW information not in "Known memories"
- Types:
  * "user" for facts ABOUT the user (name, role, location, skills)
  * "feedback" for preferences ABOUT AI behavior (response style, things to avoid)
  * "project" for technical/project facts (stack, architecture, goals)
  * "reference" for URLs / docs / external resources
- 1–2 sentences each, max 5 items
- tags are free-form keywords

"profile" rules (REFLECTIVE — user behavior / communication / work style):
- What did you LEARN about the user themselves in this conversation?
- Their preferences, communication style, expectations, work habits
- Skip if nothing new this turn; max 3 items
- MUST include "profile" as one of the tags
- type = "user" for persona traits ("prefers terse answers", "native Chinese speaker")
-        "feedback" for behavior preferences toward AI ("wants confirmation before destructive ops")

If there's nothing new in either dimension, return {"facts":[],"profile":[]}.
Respond ONLY with the JSON object, no markdown fences.

Known memories:
{EXISTING}

Conversation (recent):
{MESSAGES}"#;

// ── Public API ──────────────────────────────────────────────────

/// Run memory extraction from recent conversation history.
/// This is meant to be called from `tokio::spawn` after a successful chat.
/// When `main_agent` is provided, uses side_query() for prompt cache sharing.
pub async fn run_extraction(
    messages: &[Value],
    agent_id: &str,
    session_id: &str,
    provider_config: &crate::provider::ProviderConfig,
    model_id: &str,
    main_agent: Option<&AssistantAgent>,
) {
    if let Err(e) = do_extraction(
        messages,
        agent_id,
        session_id,
        provider_config,
        model_id,
        main_agent,
    )
    .await
    {
        app_warn!("memory", "auto_extract", "Extraction failed: {}", e);
    }
}

async fn do_extraction(
    messages: &[Value],
    agent_id: &str,
    session_id: &str,
    provider_config: &crate::provider::ProviderConfig,
    model_id: &str,
    main_agent: Option<&AssistantAgent>,
) -> Result<()> {
    let backend = crate::get_memory_backend()
        .ok_or_else(|| anyhow::anyhow!("Memory backend not initialized"))?;

    // Get existing memory summary to avoid re-extracting known info
    let existing_summary = backend
        .build_prompt_summary(agent_id, true, 2000)
        .unwrap_or_default();

    // Format recent messages (last 6) into a compact representation
    let recent: Vec<String> = messages
        .iter()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .filter_map(|msg| {
            let role = msg.get("role")?.as_str()?;
            let content = extract_text_content(msg)?;
            // Truncate very long messages
            let truncated = if content.len() > 500 {
                format!("{}...", crate::truncate_utf8(&content, 500))
            } else {
                content
            };
            Some(format!("[{}]: {}", role, truncated))
        })
        .collect();

    if recent.is_empty() {
        return Ok(());
    }

    let messages_text = recent.join("\n\n");

    // Phase B'2: single roundtrip returns facts + profile when reflection is on.
    // Fall back to the legacy facts-only prompt when the user disabled it.
    let global_extract = crate::memory::load_extract_config();
    let agent_def = crate::agent_loader::load_agent(agent_id);
    let agent_mem = agent_def.as_ref().ok().map(|d| &d.config.memory);
    let reflect_enabled = agent_mem
        .and_then(|m| m.enable_reflection)
        .unwrap_or(global_extract.enable_reflection);

    let prompt_template = if reflect_enabled {
        COMBINED_EXTRACT_PROMPT
    } else {
        EXTRACTION_PROMPT
    };
    let prompt = prompt_template
        .replace("{EXISTING}", &existing_summary)
        .replace("{MESSAGES}", &messages_text);

    // Make LLM call — prefer side_query for prompt cache sharing
    let response = if let Some(agent) = main_agent {
        let instruction = format!(
            "You are a memory extraction assistant. Respond ONLY with a JSON array, no markdown fences.\n\n{}",
            prompt
        );
        let result = agent.side_query(&instruction, 4096).await?;
        if let Some(logger) = crate::get_logger() {
            logger.log(
                "info",
                "memory",
                "side_query::extract",
                &format!(
                    "Memory extraction via side_query: cache_read={}",
                    result.usage.cache_read_input_tokens
                ),
                None,
                None,
                None,
            );
        }
        result.text
    } else {
        // Fallback: create temp agent (no cache sharing)
        let mut agent = AssistantAgent::new_from_provider(provider_config, model_id);
        agent.set_agent_id(agent_id);
        agent.set_session_id(session_id);
        agent.set_extra_system_context(
            "You are a memory extraction assistant. Respond ONLY with a JSON array, no markdown fences."
                .to_string(),
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let (resp, _thinking) = agent.chat(&prompt, &[], None, cancel, |_| {}).await?;
        resp
    };

    // Parse JSON response
    let extracted = parse_extraction_response(&response)?;

    if extracted.is_empty() {
        app_info!(
            "memory",
            "auto_extract",
            "No new memories extracted from session {}",
            session_id
        );
        return Ok(());
    }

    // If the session belongs to a project, write the new memory into
    // that project's scope so it stays local to the project. Otherwise
    // fall back to the agent's private scope (pre-project behavior).
    // Resolved once per extraction run (session/agent are constant inside a
    // turn, so no need to hit the session DB per extracted item).
    let scope = resolve_extract_scope(session_id, agent_id);

    // Save each extracted memory with dedup
    let mut saved_count = 0usize;
    for item in &extracted {
        // Phase B'2: profile-tagged items get a distinct `source` so they're
        // easy to filter in Dashboard queries and in the review UI.
        let source = if item.tags.iter().any(|t| t == "profile") {
            "auto-reflect".to_string()
        } else {
            "auto".to_string()
        };
        let entry = NewMemory {
            memory_type: item.memory_type.clone(),
            scope: scope.clone(),
            content: item.content.clone(),
            tags: item.tags.clone(),
            source,
            source_session_id: Some(session_id.to_string()),
            pinned: false,
            attachment_path: None,
            attachment_mime: None,
        };

        let dedup = crate::memory::load_dedup_config();
        match backend.add_with_dedup(entry, dedup.threshold_high, dedup.threshold_merge) {
            Ok(AddResult::Created { .. }) => saved_count += 1,
            Ok(AddResult::Updated { .. }) => saved_count += 1,
            Ok(AddResult::Duplicate { .. }) => {}
            Err(e) => {
                app_warn!(
                    "memory",
                    "auto_extract",
                    "Failed to save extracted memory: {}",
                    e
                );
            }
        }
    }

    app_info!(
        "memory",
        "auto_extract",
        "Extracted {} memories, saved {} new (session: {})",
        extracted.len(),
        saved_count,
        session_id
    );

    // Emit event for frontend notification
    if saved_count > 0 {
        if let Some(bus) = crate::get_event_bus() {
            bus.emit(
                "memory_extracted",
                serde_json::json!({
                    "count": saved_count,
                    "agentId": agent_id,
                    "sessionId": session_id,
                }),
            );
        }
    }

    Ok(())
}

// ── Flush Before Compact ────────────────────────────────────────

const FLUSH_PROMPT: &str = r#"The following conversation messages are about to be compressed and summarized.
Extract any important, durable information worth preserving as long-term memories.
Return a JSON array. Each item: {"content":"...","type":"user|feedback|project|reference","tags":["..."]}

Types:
- "user": facts about the user (name, location, preferences, expertise, role)
- "feedback": user preferences about AI behavior (response style, things to avoid)
- "project": technical/project facts (tech stack, architecture, goals, deadlines)
- "reference": URLs, docs, external resources mentioned

Rules:
- Only extract NEW information not in "Known memories" below
- Focus on information that would be lost after compression
- Be concise — each content should be 1-2 sentences
- Return [] if nothing worth remembering
- Maximum 8 items

Known memories:
{EXISTING}

Messages to be compressed:
{MESSAGES}"#;

/// Flush important memories before context compaction (Tier 3).
/// Called before summarization to prevent information loss.
/// Returns the number of memories saved.
pub async fn flush_before_compact(
    messages_to_discard: &[Value],
    agent_id: &str,
    session_id: &str,
    provider_config: &crate::provider::ProviderConfig,
    model_id: &str,
) -> Result<usize> {
    let backend = crate::get_memory_backend()
        .ok_or_else(|| anyhow::anyhow!("Memory backend not initialized"))?;

    let existing_summary = backend
        .build_prompt_summary(agent_id, true, 2000)
        .unwrap_or_default();

    // Format all messages to be discarded (more generous than auto_extract's 6-message limit)
    let mut total_chars = 0usize;
    let max_chars = 8000;
    let formatted: Vec<String> = messages_to_discard
        .iter()
        .filter_map(|msg| {
            if total_chars >= max_chars {
                return None;
            }
            let role = msg.get("role")?.as_str()?;
            let content = extract_text_content(msg)?;
            let truncated = if content.len() > 800 {
                format!("{}...", crate::truncate_utf8(&content, 800))
            } else {
                content
            };
            total_chars += truncated.len();
            Some(format!("[{}]: {}", role, truncated))
        })
        .collect();

    if formatted.is_empty() {
        return Ok(0);
    }

    let messages_text = formatted.join("\n\n");
    let prompt = FLUSH_PROMPT
        .replace("{EXISTING}", &existing_summary)
        .replace("{MESSAGES}", &messages_text);

    let mut agent = AssistantAgent::new_from_provider(provider_config, model_id);
    agent.set_agent_id(agent_id);
    agent.set_session_id(session_id);
    agent.set_extra_system_context(
        "You are a memory extraction assistant. Respond ONLY with a JSON array, no markdown fences."
            .to_string(),
    );

    let cancel = Arc::new(AtomicBool::new(false));
    let (response, _thinking) = agent.chat(&prompt, &[], None, cancel, |_| {}).await?;

    let extracted = parse_extraction_response(&response)?;
    if extracted.is_empty() {
        return Ok(0);
    }

    // Resolve once — session/agent are constant inside a flush run.
    let scope = resolve_extract_scope(session_id, agent_id);

    let mut saved_count = 0usize;
    for item in &extracted {
        let entry = NewMemory {
            memory_type: item.memory_type.clone(),
            scope: scope.clone(),
            content: item.content.clone(),
            tags: item.tags.clone(),
            source: "flush".to_string(),
            source_session_id: Some(session_id.to_string()),
            pinned: false,
            attachment_path: None,
            attachment_mime: None,
        };

        let dedup = crate::memory::load_dedup_config();
        match backend.add_with_dedup(entry, dedup.threshold_high, dedup.threshold_merge) {
            Ok(AddResult::Created { .. }) | Ok(AddResult::Updated { .. }) => saved_count += 1,
            _ => {}
        }
    }

    Ok(saved_count)
}

// ── Parsing ─────────────────────────────────────────────────────

struct ExtractedMemory {
    content: String,
    memory_type: MemoryType,
    tags: Vec<String>,
}

fn parse_extraction_response(response: &str) -> Result<Vec<ExtractedMemory>> {
    // Phase B'2: response may be either the legacy top-level array of items
    // OR the combined `{facts: [...], profile: [...]}` object. We prefer the
    // combined shape when the payload is an object (even if it also happens
    // to contain nested arrays that `extract_json_array` would match).
    let trimmed = response.trim();

    // Prefer combined-object shape.
    if let Some(obj_span) = crate::extract_json_span(trimmed, Some('{')) {
        if let Ok(obj) = serde_json::from_str::<Value>(obj_span) {
            if obj.get("facts").is_some() || obj.get("profile").is_some() {
                let facts = obj
                    .get("facts")
                    .and_then(|v| v.as_array())
                    .map(|arr| decode_items(arr, false, 5))
                    .unwrap_or_default();
                let profile = obj
                    .get("profile")
                    .and_then(|v| v.as_array())
                    .map(|arr| decode_items(arr, true, 3))
                    .unwrap_or_default();
                let mut all = facts;
                all.extend(profile);
                return Ok(all);
            }
        }
    }

    // Fall back to legacy top-level array shape. `extract_json_span` already
    // returns a bracket-balanced slice, so `serde_json::from_str` below is
    // the only validator we need — no extra "try parse, then span, then
    // parse again" dance.
    let span = crate::extract_json_span(trimmed, Some('['))
        .ok_or_else(|| anyhow::anyhow!("No JSON payload found in extraction response"))?;
    let items: Vec<Value> = serde_json::from_str(span)?;
    Ok(decode_items(&items, false, 5))
}

fn decode_items(items: &[Value], force_profile_tag: bool, limit: usize) -> Vec<ExtractedMemory> {
    let mut out = Vec::new();
    for item in items.iter().take(limit) {
        let content = match item.get("content").and_then(|v| v.as_str()) {
            Some(c) if !c.trim().is_empty() => c.trim().to_string(),
            _ => continue,
        };
        let memory_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("user");
        let mut tags: Vec<String> = item
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if force_profile_tag && !tags.iter().any(|t| t == "profile") {
            tags.push("profile".to_string());
        }
        out.push(ExtractedMemory {
            content,
            memory_type: MemoryType::from_str(memory_type),
            tags,
        });
    }
    out
}

// ── Idle Extraction ────────────────────────────────────────────

/// Cancel a pending idle extraction for a session.
fn cancel_idle_extract(session_id: &str) {
    if let Some(handles) = crate::globals::IDLE_EXTRACT_HANDLES.get() {
        if let Ok(mut map) = handles.lock() {
            if let Some((abort_handle, _, _)) = map.remove(session_id) {
                abort_handle.abort();
            }
        }
    }
}

/// Register an idle extraction handle for a session.
fn register_idle_extract(
    session_id: &str,
    abort_handle: tokio::task::AbortHandle,
    agent_id: &str,
    updated_at: &str,
) {
    if let Some(handles) = crate::globals::IDLE_EXTRACT_HANDLES.get() {
        if let Ok(mut map) = handles.lock() {
            map.insert(
                session_id.to_string(),
                (abort_handle, agent_id.to_string(), updated_at.to_string()),
            );
        }
    }
}

/// Schedule an idle extraction for a session. If no new messages arrive within
/// `idle_timeout_secs`, extraction will be triggered from DB history.
pub fn schedule_idle_extraction(
    agent_id: String,
    session_id: String,
    updated_at: String,
    idle_timeout_secs: u64,
) {
    if idle_timeout_secs == 0 {
        return;
    }

    cancel_idle_extract(&session_id);

    let sid = session_id.clone();
    let aid = agent_id.clone();
    let uat = updated_at.clone();

    let handle = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(idle_timeout_secs)).await;
        run_idle_extraction(&aid, &sid, &uat).await;
    });

    register_idle_extract(&session_id, handle.abort_handle(), &agent_id, &updated_at);
}

/// Flush all pending idle extractions immediately (e.g., when creating a new session).
/// Spawns extraction tasks without waiting for timeout.
pub fn flush_all_idle_extractions() {
    let entries = if let Some(handles) = crate::globals::IDLE_EXTRACT_HANDLES.get() {
        if let Ok(mut map) = handles.lock() {
            let entries: Vec<(String, String, String)> = map
                .drain()
                .map(|(sid, (abort_handle, aid, uat))| {
                    abort_handle.abort(); // Cancel the delayed task
                    (sid, aid, uat)
                })
                .collect();
            entries
        } else {
            return;
        }
    } else {
        return;
    };

    for (session_id, agent_id, updated_at) in entries {
        tokio::spawn(async move {
            run_idle_extraction(&agent_id, &session_id, &updated_at).await;
        });
    }
}

/// Execute idle extraction: load history from DB and run extraction without agent cache.
async fn run_idle_extraction(agent_id: &str, session_id: &str, expected_updated_at: &str) {
    // Remove our handle entry immediately (task is running, abort handle is stale).
    // This prevents cleanup from accidentally removing a newer entry registered
    // by a concurrent schedule_idle_extraction() call.
    if let Some(handles) = crate::globals::IDLE_EXTRACT_HANDLES.get() {
        if let Ok(mut map) = handles.lock() {
            map.remove(session_id);
        }
    }

    let db = match crate::get_session_db() {
        Some(db) => db,
        None => return,
    };

    let session_meta = match db.get_session(session_id) {
        Ok(Some(s)) => s,
        _ => return,
    };
    if session_meta.updated_at != expected_updated_at {
        return; // New messages arrived, skip
    }

    // Check auto_extract is enabled
    let global_extract = crate::memory::load_extract_config();
    let agent_def = crate::agent_loader::load_agent(agent_id);
    let agent_mem = agent_def.as_ref().ok().map(|d| &d.config.memory);
    let auto_extract = agent_mem
        .and_then(|m| m.auto_extract)
        .unwrap_or(global_extract.auto_extract);
    if !auto_extract {
        return;
    }

    // Load conversation history from DB
    let history = match db.load_context(session_id) {
        Ok(Some(json)) => serde_json::from_str::<Vec<Value>>(&json).unwrap_or_default(),
        _ => return,
    };
    if history.is_empty() {
        return;
    }

    // Resolve provider/model
    let extract_provider_id = agent_mem
        .and_then(|m| m.extract_provider_id.clone())
        .or_else(|| global_extract.extract_provider_id.clone())
        .or(session_meta.provider_id.clone())
        .unwrap_or_default();
    let extract_model_id = agent_mem
        .and_then(|m| m.extract_model_id.clone())
        .or_else(|| global_extract.extract_model_id.clone())
        .or(session_meta.model_id.clone())
        .unwrap_or_default();

    let store = crate::config::cached_config();
    if let Some(prov) = crate::provider::find_provider(&store.providers, &extract_provider_id) {
        app_info!(
            "memory",
            "idle_extract",
            "Running idle extraction for session {} (agent: {})",
            session_id,
            agent_id
        );
        run_extraction(
            &history,
            agent_id,
            session_id,
            prov,
            &extract_model_id,
            None,
        )
        .await;
    }
}

fn extract_text_content(msg: &Value) -> Option<String> {
    // Skip OpenAI Responses API reasoning items (encrypted, no readable text)
    if msg.get("type").and_then(|t| t.as_str()) == Some("reasoning") {
        return None;
    }
    // Handle string content (Chat Completions / simple Anthropic)
    if let Some(s) = msg.get("content").and_then(|v| v.as_str()) {
        return Some(s.to_string());
    }
    // Handle array content (Anthropic format / Responses API message format)
    if let Some(arr) = msg.get("content").and_then(|v| v.as_array()) {
        let texts: Vec<&str> = arr
            .iter()
            .filter_map(|block| {
                let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match block_type {
                    "text" => block.get("text").and_then(|t| t.as_str()),
                    "output_text" => block.get("text").and_then(|t| t.as_str()),
                    _ => None,
                }
            })
            .collect();
        if !texts.is_empty() {
            return Some(texts.join("\n"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_legacy_array_response() {
        let text = r#"[{"content":"User prefers Chinese","type":"user","tags":["lang"]}]"#;
        let items = parse_extraction_response(text).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].content, "User prefers Chinese");
        assert!(!items[0].tags.iter().any(|t| t == "profile"));
    }

    #[test]
    fn parse_combined_response() {
        let text = r#"{
          "facts": [{"content":"Lives in Shanghai","type":"user","tags":[]}],
          "profile": [{"content":"Prefers terse replies","type":"user","tags":[]}]
        }"#;
        let items = parse_extraction_response(text).unwrap();
        assert_eq!(items.len(), 2);
        // profile item should have "profile" tag injected.
        let profile_item = items
            .iter()
            .find(|i| i.content.contains("terse"))
            .expect("profile item present");
        assert!(profile_item.tags.iter().any(|t| t == "profile"));
        let fact_item = items
            .iter()
            .find(|i| i.content.contains("Shanghai"))
            .expect("fact item present");
        assert!(!fact_item.tags.iter().any(|t| t == "profile"));
    }

    #[test]
    fn parse_combined_response_with_fences() {
        let text = r#"Here's the JSON:
```json
{"facts":[],"profile":[{"content":"Speaks English fluently","type":"user","tags":["lang"]}]}
```"#;
        let items = parse_extraction_response(text).unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].tags.iter().any(|t| t == "profile"));
    }

    #[test]
    fn parse_empty_combined() {
        let text = r#"{"facts":[],"profile":[]}"#;
        let items = parse_extraction_response(text).unwrap();
        assert!(items.is_empty());
    }
}
