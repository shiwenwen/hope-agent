//! Auto-extraction of memories from conversations.
//!
//! After a chat completion, this module can extract valuable information
//! (user facts, preferences, project context) and save them as memories.

use anyhow::{Context, Result};
use serde_json::Value;

use crate::agent::AssistantAgent;
use crate::memory::{AddResult, MemoryScope, MemoryType, NewMemory};

const MEMORY_EXTRACTION_SYSTEM: &str =
    "You are a memory extraction assistant. Respond ONLY with a JSON array, no markdown fences. \
Write every `content` (and free-form `tags`) field in the same language the user predominantly \
used in the conversation — if the user wrote in Chinese, the memory must be in Chinese; if they \
wrote in Japanese, write in Japanese; etc. Match the user's language, not the assistant's.";

fn memory_extraction_instruction(prompt: &str) -> String {
    format!("{}\n\n{}", MEMORY_EXTRACTION_SYSTEM, prompt)
}

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
- Write `content` in the SAME language the user predominantly used in the conversation
  (Chinese in → Chinese out, English in → English out). Do not translate.
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
- Write `content` in the SAME language the user predominantly used in the conversation
  (Chinese in → Chinese out, English in → English out). Do not translate.

"profile" rules (REFLECTIVE — user behavior / communication / work style):
- What did you LEARN about the user themselves in this conversation?
- Their preferences, communication style, expectations, work habits
- Skip if nothing new this turn; max 3 items
- MUST include "profile" as one of the tags
- Write `content` in the SAME language the user predominantly used in the conversation
- type = "user" for persona traits ("prefers terse answers", "native Chinese speaker")
-        "feedback" for behavior preferences toward AI ("wants confirmation before destructive ops")

If there's nothing new in either dimension, return {"facts":[],"profile":[]}.
Respond ONLY with the JSON object, no markdown fences.

Known memories:
{EXISTING}

Conversation (recent):
{MESSAGES}"#;

/// Next-gen Dreaming (beta) — combined extraction that ALSO emits structured
/// `claims`. Used when `extract_claims` is enabled (the default); users who
/// opt out keep the legacy prompt and skip the extra tokens. The `claims`
/// array is the structured long-term memory; legacy `facts`/`profile` continue
/// to write `MemoryEntry` rows.
const COMBINED_EXTRACT_WITH_CLAIMS_PROMPT: &str = r#"Output ONE JSON object with THREE arrays:
{
  "facts":    [{"content":"...","type":"user|feedback|project|reference","tags":["..."]}],
  "profile":  [{"content":"...","type":"user|feedback","tags":["profile", ...]}],
  "claims":   [{"claimType":"user_profile|preference|project_fact|standing_rule|reference|task_pattern",
                "subject":"user|agent|project|tool:<name>","predicate":"prefers|uses|works_on|avoid|completed|deprecated|...",
                "object":"...","content":"human-readable sentence, same language as the user",
                "scope":{"type":"global|agent|project"},
                "evidenceClass":"explicit_user_statement|user_confirmed|project_artifact_fact|assistant_inferred|behavioral_pattern",
                "salience":0.0-1.0,"temporal":{"validFrom":null,"validUntil":null},
                "tags":["..."]}]
}

"facts" rules — same as before:
- Extract NEW information not in "Known memories"; types user/feedback/project/reference; 1–2 sentences; max 5; same language as the user.

"profile" rules — reflective user traits; MUST include the "profile" tag; max 3; same language.

"claims" rules (structured long-term memory):
- One claim per durable, reusable fact. Decompose: a claim is (subject, predicate, object) + a human `content` sentence.
- `evidenceClass` describes HOW you know it (do NOT output a confidence number — the server derives it):
  * explicit_user_statement — the user said it directly
  * user_confirmed — the user confirmed something you proposed
  * project_artifact_fact — read from project files / tool output
  * assistant_inferred — you inferred it (default if unsure)
  * behavioral_pattern — inferred from repeated behavior
- `scope`: "project" for project-specific facts, "global" for cross-project user preferences, "agent" for how this agent should behave.
- Set `temporal.validUntil` (ISO8601) ONLY for time-bounded facts ("next week", "until the launch"); else null.
- `salience` = long-term usefulness (0–1). Skip low-value chit-chat. Max 6 claims.
- Write `content` in the SAME language the user predominantly used.

If there's nothing new in any dimension, return {"facts":[],"profile":[],"claims":[]}.
Respond ONLY with the JSON object, no markdown fences.

Known memories:
{EXISTING}

Conversation (recent):
{MESSAGES}"#;

/// Next-gen Dreaming (beta) with reflection OFF: `facts` + `claims`, but NO
/// `profile` array. When the user/agent disabled `enable_reflection`, opting
/// into claims must NOT silently re-enable reflective user-profile extraction
/// into the legacy `memories` table — so this variant drops `profile` entirely
/// (the write path also filters any stray profile-tagged item as
/// defense-in-depth).
const COMBINED_EXTRACT_FACTS_CLAIMS_PROMPT: &str = r#"Output ONE JSON object with TWO arrays:
{
  "facts":    [{"content":"...","type":"user|feedback|project|reference","tags":["..."]}],
  "claims":   [{"claimType":"user_profile|preference|project_fact|standing_rule|reference|task_pattern",
                "subject":"user|agent|project|tool:<name>","predicate":"prefers|uses|works_on|avoid|completed|deprecated|...",
                "object":"...","content":"human-readable sentence, same language as the user",
                "scope":{"type":"global|agent|project"},
                "evidenceClass":"explicit_user_statement|user_confirmed|project_artifact_fact|assistant_inferred|behavioral_pattern",
                "salience":0.0-1.0,"temporal":{"validFrom":null,"validUntil":null},
                "tags":["..."]}]
}

"facts" rules — same as before:
- Extract NEW information not in "Known memories"; types user/feedback/project/reference; 1–2 sentences; max 5; same language as the user.
- Do NOT emit reflective user-profile traits here; the user disabled reflection.

"claims" rules (structured long-term memory):
- One claim per durable, reusable fact. Decompose: a claim is (subject, predicate, object) + a human `content` sentence.
- `evidenceClass` describes HOW you know it (do NOT output a confidence number — the server derives it):
  * explicit_user_statement — the user said it directly
  * user_confirmed — the user confirmed something you proposed
  * project_artifact_fact — read from project files / tool output
  * assistant_inferred — you inferred it (default if unsure)
  * behavioral_pattern — inferred from repeated behavior
- `scope`: "project" for project-specific facts, "global" for cross-project user preferences, "agent" for how this agent should behave.
- Set `temporal.validUntil` (ISO8601) ONLY for time-bounded facts ("next week", "until the launch"); else null.
- `salience` = long-term usefulness (0–1). Skip low-value chit-chat. Max 6 claims.
- Write `content` in the SAME language the user predominantly used.

If there's nothing new in either dimension, return {"facts":[],"claims":[]}.
Respond ONLY with the JSON object, no markdown fences.

Known memories:
{EXISTING}

Conversation (recent):
{MESSAGES}"#;

/// Pick the extraction prompt by feature flags. Pure + unit-tested so the
/// privacy-critical invariant is locked: when reflection is OFF, no prompt
/// variant (claims on or off) asks the model for the reflective `profile`
/// array, so opting into claims can never silently re-open profile extraction.
fn select_extract_prompt(claims_enabled: bool, reflect_enabled: bool) -> &'static str {
    match (claims_enabled, reflect_enabled) {
        // Claims beta on + reflection on: facts + profile + claims.
        (true, true) => COMBINED_EXTRACT_WITH_CLAIMS_PROMPT,
        // Claims beta on + reflection OFF: facts + claims, NO profile.
        (true, false) => COMBINED_EXTRACT_FACTS_CLAIMS_PROMPT,
        // Claims beta off + reflection on: facts + profile.
        (false, true) => COMBINED_EXTRACT_PROMPT,
        // Both off: legacy facts-only.
        (false, false) => EXTRACTION_PROMPT,
    }
}

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
    if crate::session::is_session_incognito(Some(session_id)) {
        app_info!(
            "memory",
            "auto_extract",
            "Skipping extraction for incognito session {}",
            session_id
        );
        return;
    }
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

    // Next-gen claim dual-write (beta, on by default). When on, use the claims-
    // augmented combined prompt; users who opt out keep the existing prompts
    // and skip the extra tokens.
    let claims_enabled = global_extract.extract_claims;
    let prompt_template = select_extract_prompt(claims_enabled, reflect_enabled);
    let prompt = prompt_template
        .replace("{EXISTING}", &existing_summary)
        .replace("{MESSAGES}", &messages_text);

    // Make LLM call — prefer side_query for prompt cache sharing
    let response = if let Some(agent) = main_agent {
        let instruction = memory_extraction_instruction(&prompt);
        let result = agent
            .side_query(&instruction, 4096)
            .await
            .with_context(|| {
                format!(
                    "memory extraction side_query failed (source=main_agent, provider_id={}, api_type={}, model={}, session={})",
                    provider_config.id,
                    provider_config.api_type.display_name(),
                    model_id,
                    session_id
                )
            })?;
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
        // Fallback: create temp agent (no cache sharing). Use side_query so
        // Codex hydrates OAuth from disk through build_llm_provider instead
        // of sending the placeholder provider token from config.
        let mut agent = AssistantAgent::try_new_from_provider(provider_config, model_id)
            .await?
            .with_failover_context(provider_config);
        agent.set_agent_id(agent_id);
        agent.set_session_id(session_id);
        let instruction = memory_extraction_instruction(&prompt);
        agent
            .side_query(&instruction, 4096)
            .await
            .with_context(|| {
                format!(
                    "memory extraction side_query failed (source=temp_agent, provider_id={}, api_type={}, model={}, session={})",
                    provider_config.id,
                    provider_config.api_type.display_name(),
                    model_id,
                    session_id
                )
            })?
            .text
    };

    // Parse JSON response
    let extracted = parse_extraction_response(&response)?;

    // Parse claim candidates up-front (beta). A claims-only turn — the model
    // surfaced a durable claim but no loose fact/profile — must still
    // dual-write, so claims are parsed BEFORE the empty-legacy early return.
    let claim_candidates = if claims_enabled {
        parse_claim_candidates(&response)
    } else {
        Vec::new()
    };

    if extracted.is_empty() && claim_candidates.is_empty() {
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
        // Defense-in-depth: when reflection is off, never persist reflective
        // profile-tagged items even if the model emitted them unprompted (the
        // facts-only / facts+claims prompts don't ask for `profile`, but a
        // model could still surface one). Mirrors the prompt selection above.
        if !reflect_enabled && item.tags.iter().any(|t| t == "profile") {
            continue;
        }
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

    // Next-gen claim dual-write (beta). For each candidate (parsed above, so a
    // claims-only turn still reaches here), write its legacy shadow via
    // add_with_dedup (consuming the 3-state) + the structured claim (rule-only
    // canonicalize) + the link.
    if !claim_candidates.is_empty() {
        let sid = session_id.to_string();
        let scope_for_claims = scope.clone();
        let linked = tokio::task::spawn_blocking(move || {
            dual_write_claims(claim_candidates, &sid, &scope_for_claims)
        })
        .await
        .unwrap_or(0);
        app_info!(
            "memory",
            "claim_extract",
            "dual-wrote {} claim(s) (session: {})",
            linked,
            session_id
        );
    }

    Ok(())
}

/// Map a claim type to the legacy `MemoryType` used for the dual-write shadow.
fn claim_type_to_memory_type(claim_type: &str) -> MemoryType {
    let s = match claim_type {
        "user_profile" => "user",
        "preference" | "standing_rule" => "feedback",
        "project_fact" | "task_pattern" => "project",
        "reference" => "reference",
        _ => "user",
    };
    MemoryType::from_str(s)
}

/// Parse the `claims` array from a combined extraction response into validated
/// candidates. Tolerant: skips malformed items; caps at 8.
fn parse_claim_candidates(response: &str) -> Vec<crate::memory::claims::ClaimCandidate> {
    let trimmed = response.trim();
    let Some(span) = crate::extract_json_span(trimmed, Some('{')) else {
        return Vec::new();
    };
    let Ok(obj) = serde_json::from_str::<Value>(span) else {
        return Vec::new();
    };
    let Some(arr) = obj.get("claims").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| {
            serde_json::from_value::<crate::memory::claims::ClaimCandidate>(v.clone()).ok()
        })
        .filter(|c| {
            !c.subject.trim().is_empty()
                && !c.predicate.trim().is_empty()
                && !c.object.trim().is_empty()
                && !c.content.trim().is_empty()
        })
        .take(8)
        .collect()
}

/// Blocking dual-write of claim candidates: shadow `memories` row (3-state) +
/// structured claim + link. Returns the number of claims linked. Best-effort:
/// a per-item failure logs a warning and continues.
fn dual_write_claims(
    candidates: Vec<crate::memory::claims::ClaimCandidate>,
    session_id: &str,
    scope: &MemoryScope,
) -> usize {
    let Some(backend) = crate::get_memory_backend() else {
        return 0;
    };
    let dedup = crate::memory::load_dedup_config();
    let mut linked = 0usize;
    for c in &candidates {
        let shadow = NewMemory {
            memory_type: claim_type_to_memory_type(&c.claim_type),
            scope: scope.clone(),
            content: c.content.clone(),
            tags: c.tags.clone(),
            source: "auto-claim".to_string(),
            source_session_id: Some(session_id.to_string()),
            pinned: false,
            attachment_path: None,
            attachment_mime: None,
        };
        // `managed` link = the claim OWNS this shadow row, so the shadow's
        // injection visibility follows the claim's lifecycle (hidden once the
        // claim expires/supersedes). Only a freshly Created shadow is owned.
        // When dedup MERGES into a pre-existing memory (Updated/Duplicate) —
        // possibly a manual or independent auto memory the user already had —
        // record a `detached` link instead: the claim detail still shows the
        // association, but the claim's lifecycle must never hide that
        // pre-existing memory (the hidden-set query only considers `managed`
        // links). Fixes the over-reach where a dedup hit bound an unrelated
        // memory's visibility to a claim it never belonged to.
        let (memory_id, sync_mode) =
            match backend.add_with_dedup(shadow, dedup.threshold_high, dedup.threshold_merge) {
                Ok(AddResult::Created { id }) => (id, "managed"),
                Ok(AddResult::Updated { id }) => (id, "detached"),
                Ok(AddResult::Duplicate { existing_id, .. }) => (existing_id, "detached"),
                Err(e) => {
                    app_warn!(
                        "memory",
                        "claim_extract",
                        "shadow memory write failed: {}",
                        e
                    );
                    continue;
                }
            };
        match crate::memory::claims::write_claim_candidate(c, scope, session_id, None) {
            Ok(outcome) => {
                if let Err(e) = crate::memory::claims::link_claim_memory(
                    &outcome.claim_id,
                    memory_id,
                    sync_mode,
                ) {
                    app_warn!("memory", "claim_extract", "claim link failed: {}", e);
                } else {
                    linked += 1;
                }
            }
            Err(e) => app_warn!("memory", "claim_extract", "claim write failed: {}", e),
        }
    }
    linked
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
- Write `content` in the SAME language the user predominantly used in the conversation
  (Chinese in → Chinese out, English in → English out). Do not translate.
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
    if crate::session::is_session_incognito(Some(session_id)) {
        app_info!(
            "memory",
            "flush",
            "Skipping flush_before_compact for incognito session {}",
            session_id
        );
        return Ok(0);
    }
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

    let mut agent = AssistantAgent::try_new_from_provider(provider_config, model_id)
        .await?
        .with_failover_context(provider_config);
    agent.set_agent_id(agent_id);
    agent.set_session_id(session_id);
    let instruction = memory_extraction_instruction(&prompt);
    let response = agent
        .side_query(&instruction, 4096)
        .await
        .with_context(|| {
            format!(
                "flush_before_compact side_query failed (provider_id={}, api_type={}, model={}, session={})",
                provider_config.id,
                provider_config.api_type.display_name(),
                model_id,
                session_id
            )
        })?
        .text;

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

    // Prefer combined-object shape. `claims` is included so a claims-only
    // object (`{"claims":[...]}`, common when the model drops empty
    // facts/profile arrays) enters this branch and returns NO legacy items —
    // rather than falling through to the bracket scan below, which would match
    // the `claims` array's `[` and mis-decode each claim's `content` as a
    // bogus legacy memory (double-write). Claims are handled solely by
    // `parse_claim_candidates`.
    if let Some(obj_span) = crate::extract_json_span(trimmed, Some('{')) {
        if let Ok(obj) = serde_json::from_str::<Value>(obj_span) {
            if obj.get("facts").is_some()
                || obj.get("profile").is_some()
                || obj.get("claims").is_some()
            {
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
pub fn cancel_idle_extraction(session_id: &str) {
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

    if crate::session::is_session_incognito(Some(&session_id)) {
        app_info!(
            "memory",
            "idle_extract",
            "Not scheduling idle extraction for incognito session {}",
            session_id
        );
        return;
    }

    cancel_idle_extraction(&session_id);

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
    // Remove our handle entry — but only if it still matches the updated_at we
    // were scheduled for. A concurrent `schedule_idle_extraction()` may have
    // cancelled the old abort handle and registered a fresh one while this
    // task was already running; blindly removing would drop the new entry and
    // leak it from future `cancel_idle_extract()` paths.
    if let Some(handles) = crate::globals::IDLE_EXTRACT_HANDLES.get() {
        if let Ok(mut map) = handles.lock() {
            if let Some(entry) = map.get(session_id) {
                if entry.2 == expected_updated_at {
                    map.remove(session_id);
                }
            }
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
    if session_meta.incognito {
        app_info!(
            "memory",
            "idle_extract",
            "Skipping idle extraction for incognito session {}",
            session_id
        );
        return;
    }
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

    #[test]
    fn claims_only_object_yields_no_legacy_items() {
        // Model dropped empty facts/profile and returned only claims. The
        // legacy parser must NOT fall through to the bracket scan and decode
        // claim `content` as bogus legacy memories (the double-write bug).
        let text = r#"{"claims":[{"claimType":"preference","subject":"user","predicate":"prefers","object":"bun","content":"User prefers Bun.","evidenceClass":"explicit_user_statement"}]}"#;
        let items = parse_extraction_response(text).unwrap();
        assert!(
            items.is_empty(),
            "claims-only object must yield 0 legacy items"
        );
    }

    #[test]
    fn claims_only_object_still_parses_claim_candidates() {
        // The same claims-only payload must still produce claim candidates, so
        // a claims-only turn dual-writes (not silently dropped).
        let text = r#"{"facts":[],"profile":[],"claims":[{"claimType":"preference","subject":"user","predicate":"prefers","object":"bun","content":"User prefers Bun.","evidenceClass":"explicit_user_statement"}]}"#;
        let candidates = parse_claim_candidates(text);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].object, "bun");
    }

    #[test]
    fn claims_on_with_reflection_off_never_requests_profile() {
        // Privacy invariant: opting into claims while reflection is OFF must
        // pick a prompt that does NOT ask the model for the reflective
        // `profile` array (Codex adversarial finding #1).
        let prompt = select_extract_prompt(true, false);
        assert!(
            !prompt.contains("\"profile\""),
            "claims-on + reflection-off prompt must not request a profile array"
        );
        assert!(prompt.contains("\"claims\""), "must still request claims");
        assert!(prompt.contains("\"facts\""), "must still request facts");
    }

    #[test]
    fn extract_prompt_selection_matrix() {
        // reflection on → profile present; reflection off → profile absent.
        assert!(select_extract_prompt(true, true).contains("\"profile\""));
        assert!(select_extract_prompt(true, true).contains("\"claims\""));
        assert!(select_extract_prompt(false, true).contains("\"profile\""));
        assert!(!select_extract_prompt(false, true).contains("\"claims\""));
        // Both off → legacy facts-only: neither profile nor claims.
        let legacy = select_extract_prompt(false, false);
        assert!(!legacy.contains("\"profile\""));
        assert!(!legacy.contains("\"claims\""));
    }
}
