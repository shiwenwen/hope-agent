use std::sync::Arc;

use crate::agent::AssistantAgent;
use crate::context_compact::CompactConfig;
use crate::provider::{self, ActiveModel, AuthProfile, ProviderConfig};
use crate::session::SessionDB;

// ── Agent Construction ──────────────────────────────────────────────

/// Build an AssistantAgent from provider configs (no State dependency).
///
/// When `profile` is `Some`, the agent is constructed with that specific
/// auth profile's API key and base_url override. When `None`, the first
/// effective profile (or legacy `api_key`) is used.
pub(super) async fn build_agent_from_snapshot(
    model: &ActiveModel,
    providers: &[ProviderConfig],
    codex_token_hint: Option<(String, String)>,
    compact_config: &CompactConfig,
    profile: Option<&AuthProfile>,
    session_id: &str,
) -> anyhow::Result<AssistantAgent> {
    let prov = provider::find_provider(providers, &model.provider_id)
        .ok_or_else(|| anyhow::anyhow!("Provider {} not found", model.provider_id))?;

    let agent = AssistantAgent::try_new_from_provider_with_codex_hint(
        prov,
        &model.model_id,
        profile,
        codex_token_hint,
    )
    .await?;

    let mut agent = agent.with_failover_context(prov);
    agent.set_compact_config(compact_config.clone());

    if let Some(model_ref) = compact_config.effective_summarization_model_ref() {
        if let Some(cp) = crate::agent::build_compaction_provider(&model_ref, providers, session_id)
        {
            agent.set_compaction_provider(Some(std::sync::Arc::new(cp)));
        }
    }

    Ok(agent)
}

// ── Conversation history load/save ──────────────────────────────────

/// Restore conversation history from DB into the agent.
///
/// Reverse-rebuild of partial/interrupted turns is no longer the
/// restore path's responsibility — the unified `finalize` system
/// (`chat_engine::finalize`) writes a coherent `[系统事件]` marker plus
/// provider-native partial reconstruction directly into `context_json`
/// at turn-termination time (either runtime convergence in `engine.rs`
/// or the startup sweep in `app_init`). Restore just loads what's
/// there and hands it to the agent.
pub fn restore_agent_context_from_json(
    session_id: &str,
    json_str: &str,
    agent: &AssistantAgent,
) -> bool {
    let history: Vec<serde_json::Value> = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if history.is_empty() {
        return false;
    }
    app_debug!(
        "session",
        "chat_engine",
        "Restored {} messages for session {}",
        history.len(),
        session_id
    );
    agent.set_conversation_history(history);
    true
}

pub fn restore_agent_context(db: &Arc<SessionDB>, session_id: &str, agent: &AssistantAgent) {
    let Ok(Some(json_str)) = db.load_context(session_id) else {
        return;
    };
    restore_agent_context_from_json(session_id, &json_str, agent);
}

// ── Auto memory extraction scheduling ───────────────────────────────

/// Schedule memory extraction after a successful turn. Returns the resolved
/// idle_timeout_secs so the caller can schedule idle extraction without
/// re-loading config.
///
/// Trigger logic (since last extraction):
/// - Cooldown: elapsed time must >= time threshold (prevents too-frequent extraction)
/// - Trigger: token count >= token threshold OR message count >= message threshold
///
/// Both cooldown AND trigger must be satisfied.
pub(super) async fn schedule_memory_extraction_after_turn(
    agent_id: &str,
    session_id: &str,
    model_ref: &ActiveModel,
    agent: &AssistantAgent,
) -> u64 {
    let sid = session_id.to_string();
    if crate::blocking::run_blocking(move || crate::session::is_session_incognito(Some(&sid))).await
    {
        return 0;
    }
    if !crate::memory::automatic_memory_learning_allowed(
        Some(session_id),
        agent.session_db.as_deref(),
    ) {
        return 0;
    }
    let global_extract = crate::memory::load_extract_config();
    let aid = agent_id.to_string();
    let agent_def =
        crate::blocking::run_blocking(move || crate::agent_loader::load_agent(&aid)).await;
    let agent_mem = agent_def.as_ref().ok().map(|d| &d.config.memory);

    // Preserve an explicit per-Agent legacy override throughout the V2
    // compatibility window. The V2 global learning mode remains the outer
    // gate above, so `manual` still wins over `Some(true)`.
    let auto_extract = agent_mem
        .and_then(|m| m.auto_extract)
        .unwrap_or(global_extract.auto_extract);
    let idle_timeout = agent_mem
        .and_then(|m| m.extract_idle_timeout_secs)
        .unwrap_or(global_extract.extract_idle_timeout_secs);

    if !auto_extract {
        return 0;
    }

    if agent
        .manual_memory_saved
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        app_info!(
            "memory",
            "auto_extract",
            "Skipping extraction: manual save_memory called this round"
        );
        return idle_timeout;
    }

    let token_threshold = agent_mem
        .and_then(|m| m.extract_token_threshold)
        .unwrap_or(global_extract.extract_token_threshold);
    let cooldown_secs = agent_mem
        .and_then(|m| m.extract_time_threshold_secs)
        .unwrap_or(global_extract.extract_time_threshold_secs);
    let message_threshold = agent_mem
        .and_then(|m| m.extract_message_threshold)
        .unwrap_or(global_extract.extract_message_threshold);

    let tokens_acc = agent
        .tokens_since_extraction
        .load(std::sync::atomic::Ordering::SeqCst) as usize;
    let messages_acc = agent
        .messages_since_extraction
        .load(std::sync::atomic::Ordering::SeqCst) as usize;
    let elapsed_secs = agent
        .last_extraction_at
        .lock()
        .map(|t| t.elapsed().as_secs())
        .unwrap_or(0);

    if elapsed_secs < cooldown_secs {
        return idle_timeout;
    }

    let token_met = tokens_acc >= token_threshold;
    let message_met = messages_acc >= message_threshold;

    if !token_met && !message_met {
        return idle_timeout;
    }

    app_info!(
        "memory",
        "auto_extract",
        "Extraction scheduled: tokens={}/{} msgs={}/{} cooldown={}s/{}s (session: {})",
        tokens_acc,
        token_threshold,
        messages_acc,
        message_threshold,
        elapsed_secs,
        cooldown_secs,
        session_id
    );

    // Resolve provider/model for extraction: per-agent override (unchanged)
    // → global `modelOverride` (new) → deprecated global pair → this turn's
    // own model.
    let extract_provider_id = agent_mem
        .and_then(|m| m.extract_provider_id.clone())
        .or_else(|| {
            global_extract
                .model_override
                .as_ref()
                .map(|m| m.provider_id.clone())
        })
        .or_else(|| global_extract.extract_provider_id.clone())
        .unwrap_or_else(|| model_ref.provider_id.clone());
    let extract_model_id = agent_mem
        .and_then(|m| m.extract_model_id.clone())
        .or_else(|| {
            global_extract
                .model_override
                .as_ref()
                .map(|m| m.model_id.clone())
        })
        .or_else(|| global_extract.extract_model_id.clone())
        .unwrap_or_else(|| model_ref.model_id.clone());

    let history = agent.get_conversation_history();
    let store = crate::config::cached_config();
    if let Some(prov) = provider::find_provider(&store.providers, &extract_provider_id).cloned() {
        let agent_id = agent_id.to_string();
        let session_id = session_id.to_string();
        let session_db = agent.session_db.clone();
        tokio::spawn(async move {
            crate::memory_extract::run_extraction(
                &history,
                &agent_id,
                &session_id,
                &prov,
                &extract_model_id,
                None,
                session_db,
            )
            .await;
        });
        agent.reset_extraction_tracking();
    } else {
        app_warn!(
            "memory",
            "auto_extract",
            "Extraction provider {} not found for session {}",
            extract_provider_id,
            session_id
        );
    }
    idle_timeout
}
