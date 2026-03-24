//! Auto-extraction of memories from conversations.
//!
//! After a chat completion, this module can extract valuable information
//! (user facts, preferences, project context) and save them as memories.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use serde_json::Value;

use crate::agent::AssistantAgent;
use crate::memory::{
    AddResult, MemoryScope, MemoryType, NewMemory,
};

// ── Extraction Prompt ───────────────────────────────────────────

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

// ── Public API ──────────────────────────────────────────────────

/// Run memory extraction from recent conversation history.
/// This is meant to be called from `tokio::spawn` after a successful chat.
pub async fn run_extraction(
    messages: &[Value],
    agent_id: &str,
    session_id: &str,
    provider_config: &crate::provider::ProviderConfig,
    model_id: &str,
) {
    if let Err(e) = do_extraction(messages, agent_id, session_id, provider_config, model_id).await {
        app_warn!("memory", "auto_extract", "Extraction failed: {}", e);
    }
}

async fn do_extraction(
    messages: &[Value],
    agent_id: &str,
    session_id: &str,
    provider_config: &crate::provider::ProviderConfig,
    model_id: &str,
) -> Result<()> {
    let backend = crate::get_memory_backend()
        .ok_or_else(|| anyhow::anyhow!("Memory backend not initialized"))?;

    // Get existing memory summary to avoid re-extracting known info
    let existing_summary = backend.build_prompt_summary(agent_id, true, 2000)
        .unwrap_or_default();

    // Format recent messages (last 6) into a compact representation
    let recent: Vec<String> = messages.iter()
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
                format!("{}...", &content[..500])
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

    // Build extraction prompt
    let prompt = EXTRACTION_PROMPT
        .replace("{EXISTING}", &existing_summary)
        .replace("{MESSAGES}", &messages_text);

    // Make a simple LLM call (no tool loop, no system prompt injection)
    let mut agent = AssistantAgent::new_from_provider(provider_config, model_id);
    agent.set_agent_id(agent_id);
    agent.set_session_id(session_id);
    agent.set_extra_system_context(
        "You are a memory extraction assistant. Respond ONLY with a JSON array, no markdown fences."
            .to_string(),
    );

    let cancel = Arc::new(AtomicBool::new(false));
    let (response, _thinking) = agent.chat(&prompt, &[], None, cancel, |_| {}).await?;

    // Parse JSON response
    let extracted = parse_extraction_response(&response)?;

    if extracted.is_empty() {
        app_info!("memory", "auto_extract", "No new memories extracted from session {}", session_id);
        return Ok(());
    }

    // Save each extracted memory with dedup
    let mut saved_count = 0usize;
    for item in &extracted {
        let scope = MemoryScope::Agent { id: agent_id.to_string() };
        let entry = NewMemory {
            memory_type: item.memory_type.clone(),
            scope,
            content: item.content.clone(),
            tags: item.tags.clone(),
            source: "auto".to_string(),
            source_session_id: Some(session_id.to_string()),
        };

        let dedup = crate::memory::load_dedup_config();
        match backend.add_with_dedup(entry, dedup.threshold_high, dedup.threshold_merge) {
            Ok(AddResult::Created { .. }) => saved_count += 1,
            Ok(AddResult::Updated { .. }) => saved_count += 1,
            Ok(AddResult::Duplicate { .. }) => {}
            Err(e) => {
                app_warn!("memory", "auto_extract", "Failed to save extracted memory: {}", e);
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
        if let Some(handle) = crate::get_app_handle() {
            use tauri::Emitter;
            let _ = handle.emit("memory_extracted", serde_json::json!({
                "count": saved_count,
                "agentId": agent_id,
                "sessionId": session_id,
            }));
        }
    }

    Ok(())
}

// ── Parsing ─────────────────────────────────────────────────────

struct ExtractedMemory {
    content: String,
    memory_type: MemoryType,
    tags: Vec<String>,
}

fn parse_extraction_response(response: &str) -> Result<Vec<ExtractedMemory>> {
    // Try to find JSON array in the response (handle markdown fences, extra text)
    let json_str = extract_json_array(response)
        .ok_or_else(|| anyhow::anyhow!("No JSON array found in extraction response"))?;

    let items: Vec<Value> = serde_json::from_str(&json_str)?;
    let mut result = Vec::new();

    for item in items.iter().take(5) {
        let content = match item.get("content").and_then(|v| v.as_str()) {
            Some(c) if !c.trim().is_empty() => c.trim().to_string(),
            _ => continue,
        };

        let memory_type = item.get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("user");

        let tags: Vec<String> = item.get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        result.push(ExtractedMemory {
            content,
            memory_type: MemoryType::from_str(memory_type),
            tags,
        });
    }

    Ok(result)
}

fn extract_json_array(text: &str) -> Option<String> {
    // Try direct parse first
    if serde_json::from_str::<Vec<Value>>(text.trim()).is_ok() {
        return Some(text.trim().to_string());
    }

    // Try to find [...] in the text
    let start = text.find('[')?;
    let mut depth = 0;
    let mut end = None;
    for (i, ch) in text[start..].char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(start + i + 1);
                    break;
                }
            }
            _ => {}
        }
    }

    let end = end?;
    let candidate = &text[start..end];
    // Validate it's valid JSON
    if serde_json::from_str::<Vec<Value>>(candidate).is_ok() {
        Some(candidate.to_string())
    } else {
        None
    }
}

fn extract_text_content(msg: &Value) -> Option<String> {
    // Handle string content
    if let Some(s) = msg.get("content").and_then(|v| v.as_str()) {
        return Some(s.to_string());
    }
    // Handle array content (Anthropic format)
    if let Some(arr) = msg.get("content").and_then(|v| v.as_array()) {
        let texts: Vec<&str> = arr.iter()
            .filter_map(|block| {
                if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                    block.get("text").and_then(|t| t.as_str())
                } else {
                    None
                }
            })
            .collect();
        if !texts.is_empty() {
            return Some(texts.join("\n"));
        }
    }
    None
}
