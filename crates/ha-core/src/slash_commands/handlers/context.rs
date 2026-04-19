// ── /context — Show context window breakdown ────────────────────
//
// Computes per-category token usage for the active session's context
// window: system prompt, tool schemas, memory, skills, messages, and
// reserved output. Returns a structured `ContextBreakdown` via the
// `ShowContextBreakdown` action for rich rendering on the desktop UI,
// plus a Unicode-bar markdown fallback in `content` for IM channels.

use crate::context_compact::CHARS_PER_TOKEN;
use crate::globals::AppState;
use crate::slash_commands::types::{CommandAction, CommandResult, ContextBreakdown};

fn chars_to_tokens(chars: usize) -> u32 {
    (chars / CHARS_PER_TOKEN) as u32
}

pub async fn handle_context(
    state: &AppState,
    session_id: Option<&str>,
    agent_id: &str,
    _args: &str,
) -> Result<CommandResult, String> {
    // ── Active agent snapshot ─────────────────────────────────────
    let agent_guard = state.agent.lock().await;
    let agent = agent_guard
        .as_ref()
        .ok_or_else(|| "No active agent".to_string())?;

    let context_window = agent.get_context_window();
    // Reserved output budget — matches the value used in run_compaction.
    let max_output_tokens: u32 = 16_384;

    // Conversation history (what actually gets sent to the API).
    let history = agent.get_conversation_history();
    let message_count = history.len() as u32;
    let messages_chars: usize = history
        .iter()
        .map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0))
        .sum();
    let messages_tokens = chars_to_tokens(messages_chars);

    // JSON tool schemas sent in the API `tools:` array.
    // Provider-agnostic estimate — Anthropic format is representative.
    let tool_schemas = agent.build_tool_schemas(crate::tools::ToolProvider::Anthropic);
    let tool_schemas_chars: usize = tool_schemas
        .iter()
        .map(|s| serde_json::to_string(s).map(|x| x.len()).unwrap_or(0))
        .sum();
    let tool_schemas_tokens = chars_to_tokens(tool_schemas_chars);

    // Last compaction timer — read directly while we still hold `agent`.
    let last_compact_secs_ago = agent
        .last_tier2_compaction_at
        .lock()
        .ok()
        .and_then(|guard| guard.map(|t| t.elapsed().as_secs()));

    drop(agent_guard);

    // ── Model / provider resolution ──────────────────────────────
    let (active_model, active_provider) = {
        let store = state.config.lock().await;
        if let Some(ref active) = store.active_model {
            let prov = store.providers.iter().find(|p| p.id == active.provider_id);
            let model_label = prov
                .and_then(|p| p.models.iter().find(|m| m.id == active.model_id))
                .map(|m| m.name.clone())
                .unwrap_or_else(|| active.model_id.clone());
            let provider_label = prov
                .map(|p| p.name.clone())
                .unwrap_or_else(|| active.provider_id.clone());
            (model_label, provider_label)
        } else {
            ("unknown".to_string(), "unknown".to_string())
        }
    };

    // ── System prompt breakdown (memory / skills / tool descriptions) ──
    let agent_def = crate::agent_loader::load_agent(agent_id)
        .map_err(|e| format!("Failed to load agent: {}", e))?;

    let memory_entries: Vec<crate::memory::MemoryEntry> = if agent_def.config.memory.enabled {
        crate::get_memory_backend()
            .and_then(|b| {
                b.load_prompt_candidates(agent_id, agent_def.config.memory.shared)
                    .ok()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let app_cfg = crate::config::cached_config();
    let memory_budget = crate::agent_config::effective_memory_budget(
        &agent_def.config.memory,
        &app_cfg.memory_budget,
    );

    let agent_home = crate::paths::agent_home_dir(agent_id)
        .ok()
        .map(|p| p.to_string_lossy().to_string());

    let breakdown_chars = crate::system_prompt::compute_breakdown(
        &agent_def,
        Some(&active_model),
        Some(&active_provider),
        &memory_entries,
        &memory_budget,
        agent_home.as_deref(),
    );

    let total_system_tokens = chars_to_tokens(breakdown_chars.total_chars);
    let memory_tokens = chars_to_tokens(breakdown_chars.memory_chars);
    let skill_tokens = chars_to_tokens(breakdown_chars.skills_chars);
    let tool_descriptions_tokens = chars_to_tokens(breakdown_chars.tool_descriptions_chars);
    // "Base" system prompt = everything in the prompt minus the sub-sections
    // we already attribute separately (memory, skills, tool descriptions).
    let system_prompt_tokens = total_system_tokens
        .saturating_sub(memory_tokens)
        .saturating_sub(skill_tokens)
        .saturating_sub(tool_descriptions_tokens);

    // ── Totals ───────────────────────────────────────────────────
    let used_total = system_prompt_tokens
        + tool_schemas_tokens
        + tool_descriptions_tokens
        + memory_tokens
        + skill_tokens
        + messages_tokens
        + max_output_tokens;
    let free_space = context_window.saturating_sub(used_total);
    let usage_pct = if context_window > 0 {
        (used_total as f32 / context_window as f32) * 100.0
    } else {
        0.0
    };

    // ── Compaction throttle countdown ───────────────────────────
    let cache_ttl_secs = crate::config::cached_config().compact.cache_ttl_secs;
    let next_compact_allowed_in_secs = match (last_compact_secs_ago, cache_ttl_secs) {
        (Some(ago), ttl) if ttl > 0 && ago < ttl => Some(ttl - ago),
        _ => None,
    };

    // ── Last tier (not currently tracked beyond "Tier 2+" timestamp) ──
    let last_compact_tier = last_compact_secs_ago.map(|_| 2u8);

    let breakdown = ContextBreakdown {
        context_window,
        max_output_tokens,
        system_prompt_tokens,
        tool_schemas_tokens,
        tool_descriptions_tokens,
        memory_tokens,
        skill_tokens,
        messages_tokens,
        used_total,
        free_space,
        usage_pct,
        last_compact_tier,
        last_compact_secs_ago,
        next_compact_allowed_in_secs,
        active_model,
        active_provider,
        active_agent: agent_id.to_string(),
        message_count,
    };

    // Silence the unused warning on server-only sessions — session_id is
    // informational only; the breakdown applies to whichever session owns
    // the currently-locked `AssistantAgent`.
    let _ = session_id;

    let content = render_markdown_fallback(&breakdown);

    Ok(CommandResult {
        content,
        action: Some(CommandAction::ShowContextBreakdown { breakdown }),
    })
}

/// Render a rich Unicode-bar markdown fallback for IM channels and
/// text-only renderers. The desktop UI uses the structured action
/// payload instead.
fn render_markdown_fallback(b: &ContextBreakdown) -> String {
    let ctx_k = (b.context_window as f32 / 1000.0).round() as u32;
    let used_k = (b.used_total as f32 / 1000.0).round() as u32;
    let pct = b.usage_pct.round() as u32;

    let bar = make_bar(b.usage_pct, 20);

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "📊 Context usage — {}k / {}k ({}%)",
        used_k, ctx_k, pct
    ));
    lines.push(format!("`{}`", bar));
    lines.push(String::new());

    lines.push(format_row(
        "System prompt",
        b.system_prompt_tokens,
        b.context_window,
    ));
    lines.push(format_row(
        "Tool schemas",
        b.tool_schemas_tokens,
        b.context_window,
    ));
    lines.push(format_row(
        "Tool descriptions",
        b.tool_descriptions_tokens,
        b.context_window,
    ));
    lines.push(format_row("Memory", b.memory_tokens, b.context_window));
    lines.push(format_row("Skills", b.skill_tokens, b.context_window));
    lines.push(format_row("Messages", b.messages_tokens, b.context_window));
    lines.push(format_row(
        "Reserved output",
        b.max_output_tokens,
        b.context_window,
    ));
    lines.push(format_row("Free space", b.free_space, b.context_window));
    lines.push(String::new());

    if let Some(tier) = b.last_compact_tier {
        if let Some(ago) = b.last_compact_secs_ago {
            lines.push(format!(
                "Last compact: Tier {} ({})",
                tier,
                format_duration(ago)
            ));
        }
    }
    if let Some(cooldown) = b.next_compact_allowed_in_secs {
        lines.push(format!(
            "Next compact allowed in: {}s (cache TTL protection)",
            cooldown
        ));
    }

    lines.push(format!(
        "Agent: `{}` · Model: `{}` · Messages: {}",
        b.active_agent, b.active_model, b.message_count
    ));
    lines.push("_Estimated (char÷4); may differ from billed usage by ~10–20%._".to_string());

    lines.join("\n")
}

fn format_row(label: &str, tokens: u32, total: u32) -> String {
    let pct = if total > 0 {
        (tokens as f32 / total as f32 * 100.0).round() as u32
    } else {
        0
    };
    let k = if tokens >= 1000 {
        format!("{:.1}k", tokens as f32 / 1000.0)
    } else {
        format!("{}", tokens)
    };
    format!("• **{:<18}** {:>7} ({:>3}%)", label, k, pct)
}

fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}

fn make_bar(pct: f32, width: usize) -> String {
    let filled = ((pct / 100.0) * width as f32).round() as usize;
    let filled = filled.min(width);
    let mut bar = String::with_capacity(width * 3);
    for _ in 0..filled {
        bar.push('█');
    }
    for _ in filled..width {
        bar.push('░');
    }
    bar
}
