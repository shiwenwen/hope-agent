// ── /context — Show context window breakdown ────────────────────
//
// Computes per-category token usage for the active session's context
// window: system prompt, tool schemas, memory, skills, messages, and
// reserved output. Returns a structured `ContextBreakdown` via the
// `ShowContextBreakdown` action for rich rendering on the desktop UI,
// plus a Unicode-bar markdown fallback in `content` for IM channels.

use crate::context_compact::CHARS_PER_TOKEN;
use crate::slash_commands::types::{CommandAction, CommandResult, ContextBreakdown};

fn chars_to_tokens(chars: usize) -> u32 {
    chars.div_ceil(CHARS_PER_TOKEN).min(u32::MAX as usize) as u32
}

pub async fn handle_context(
    session_id: Option<&str>,
    agent_id: &str,
    _args: &str,
) -> Result<CommandResult, String> {
    // ── Active agent snapshot ─────────────────────────────────────
    // `/context` reports against whichever agent was last built (set via
    // `/model`, desktop login, etc.). No cached agent → nothing to
    // measure against, fail loudly.
    let cached = crate::require_cached_agent().map_err(|e| e.to_string())?;
    let agent_guard = cached.lock().await;
    let agent = agent_guard
        .as_ref()
        .ok_or_else(|| "No active agent".to_string())?;

    let context_window = agent.get_context_window();
    // Reserved output budget — matches the value used in run_compaction.
    let max_output_tokens: u32 = 16_384;

    // Conversation history fallback. When a real round exists below, its
    // provider-shaped manifest replaces these heuristic values.
    let history = agent.get_conversation_history();
    let message_count = history.len() as u32;
    let fallback_messages_tokens = history
        .iter()
        .map(crate::context_compact::estimate_tokens)
        .sum();

    // Build the same provider-specific prompt and live-gated tool inventory as
    // a chat round, including deferred tools already activated for the session.
    let (provider_key, model_id) = agent.current_model_for_compaction();
    let tool_provider = if provider_key == "Anthropic" {
        crate::tools::ToolProvider::Anthropic
    } else {
        crate::tools::ToolProvider::OpenAI
    };
    let activated = agent.load_activated_tool_names();
    let inventory = agent.build_tool_inventory(tool_provider, &activated);
    let tool_schemas = inventory.schemas;
    let fallback_tool_schemas_tokens = tool_schemas
        .iter()
        .map(crate::context_compact::estimate_tokens)
        .sum();
    let actual_system_prompt = agent.build_full_system_prompt(&model_id, provider_key);
    let fallback_system_tokens =
        crate::system_prompt::conservative_core_token_estimate(&actual_system_prompt)
            .min(u32::MAX as usize) as u32;
    let static_memory = agent
        .static_memory_manifest
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let latest_round = session_id.and_then(crate::agent::token_manifest::latest_round_context);

    // Last compaction timer — read directly while we still hold `agent`.
    let last_compact_secs_ago = agent
        .last_tier2_compaction_at
        .lock()
        .ok()
        .and_then(|guard| guard.map(|t| t.elapsed().as_secs()));

    drop(agent_guard);

    // ── Model / provider resolution ──────────────────────────────
    let (configured_model, configured_provider) = {
        let store = crate::config::cached_config();
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

    let agent_home = crate::paths::agent_home_dir(agent_id)
        .ok()
        .map(|p| p.to_string_lossy().to_string());

    let breakdown_chars = crate::system_prompt::compute_breakdown(
        &agent_def,
        Some(&configured_model),
        Some(&configured_provider),
        &[],
        &crate::memory::MemoryBudgetConfig::default(),
        agent_home.as_deref(),
    );

    // Memory attribution comes from the exact prompt-build manifest, not a
    // second SQLite/Core reconstruction. Dynamic recall remains in the
    // separate dynamic suffix category below, so categories do not overlap.
    let memory_tokens = [
        static_memory.agent_core.tokens_estimate,
        static_memory.global_core.tokens_estimate,
        static_memory.project_index.tokens_estimate,
        static_memory.legacy_static_block.tokens_estimate,
    ]
    .into_iter()
    .fold(0u32, u32::saturating_add);
    let skill_tokens = chars_to_tokens(breakdown_chars.skills_chars);
    let tool_descriptions_tokens = chars_to_tokens(breakdown_chars.tool_descriptions_chars);
    let stable_prompt_tokens = latest_round
        .as_ref()
        .map(|round| round.stable_prompt_tokens_estimate)
        .unwrap_or(fallback_system_tokens);
    let dynamic_prompt_tokens = latest_round
        .as_ref()
        .map(|round| round.dynamic_prompt_tokens_estimate)
        .unwrap_or(0);
    let messages_tokens = latest_round
        .as_ref()
        .map(|round| round.history_tokens_estimate)
        .unwrap_or(fallback_messages_tokens);
    let tool_schemas_tokens = latest_round
        .as_ref()
        .map(|round| round.tool_schema_tokens_estimate)
        .unwrap_or(fallback_tool_schemas_tokens);
    // Base stable prompt = the exact stable prefix minus its attributed
    // memory/skills/tool-description sections.
    let system_prompt_tokens = stable_prompt_tokens
        .saturating_sub(memory_tokens)
        .saturating_sub(skill_tokens)
        .saturating_sub(tool_descriptions_tokens);

    // ── Totals ───────────────────────────────────────────────────
    let request_input_tokens_estimate = latest_round
        .as_ref()
        .map(|round| round.request_input_tokens_estimate)
        .unwrap_or_else(|| {
            stable_prompt_tokens
                .saturating_add(dynamic_prompt_tokens)
                .saturating_add(tool_schemas_tokens)
                .saturating_add(messages_tokens)
        });
    let actual_context_input = latest_round
        .as_ref()
        .and_then(|round| round.context_input_tokens);
    let input_for_window = actual_context_input
        .unwrap_or(u64::from(request_input_tokens_estimate))
        .min(u64::from(u32::MAX)) as u32;
    let used_total = input_for_window.saturating_add(max_output_tokens);
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
        active_model: latest_round
            .as_ref()
            .map(|round| round.model.clone())
            .unwrap_or(configured_model),
        active_provider: latest_round
            .as_ref()
            .map(|round| round.provider.clone())
            .unwrap_or(configured_provider),
        active_agent: agent_id.to_string(),
        message_count,
        dynamic_prompt_tokens,
        context_input_tokens: actual_context_input,
        fresh_input_tokens: latest_round
            .as_ref()
            .and_then(|round| round.fresh_input_tokens),
        cache_read_tokens: latest_round
            .as_ref()
            .and_then(|round| round.cache_read_tokens),
        cache_write_tokens: latest_round
            .as_ref()
            .and_then(|round| round.cache_write_tokens),
        output_tokens: latest_round.as_ref().and_then(|round| round.output_tokens),
        ttft_ms: latest_round.as_ref().and_then(|round| round.ttft_ms),
        request_input_tokens_estimate: Some(request_input_tokens_estimate),
        cacheable_stable_tokens_estimate: latest_round
            .as_ref()
            .map(|round| round.cacheable_stable_tokens_estimate),
        eager_tool_schema_tokens: latest_round
            .as_ref()
            .map(|round| round.eager_tool_schema_tokens_estimate),
        activated_tool_schema_tokens: latest_round
            .as_ref()
            .map(|round| round.activated_tool_schema_tokens_estimate),
        deferred_tool_schema_tokens: latest_round
            .as_ref()
            .map(|round| round.deferred_tool_schema_tokens_estimate),
        eager_tool_count: latest_round
            .as_ref()
            .map(|round| round.eager_tool_count.min(u32::MAX as usize) as u32),
        deferred_tool_count: latest_round
            .as_ref()
            .map(|round| round.deferred_tool_count.min(u32::MAX as usize) as u32),
        activated_tool_count: latest_round
            .as_ref()
            .map(|round| round.activated_tool_count.min(u32::MAX as usize) as u32),
        native_deferred: latest_round.as_ref().map(|round| round.native_deferred),
        stable_prompt_fingerprint: latest_round
            .as_ref()
            .map(|round| round.stable_prompt_fingerprint.clone()),
        dynamic_prompt_fingerprint: latest_round
            .as_ref()
            .map(|round| round.dynamic_prompt_fingerprint.clone()),
    };

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
    lines.push(format_row(
        "Dynamic prompt",
        b.dynamic_prompt_tokens,
        b.context_window,
    ));
    lines.push(format_row("Messages", b.messages_tokens, b.context_window));
    lines.push(format_row(
        "Reserved output",
        b.max_output_tokens,
        b.context_window,
    ));
    lines.push(format_row("Free space", b.free_space, b.context_window));
    lines.push(String::new());

    if let Some(context_input) = b.context_input_tokens {
        lines.push(format!("Provider input: {context_input} tokens"));
        if let (Some(fresh), Some(cache_read)) = (b.fresh_input_tokens, b.cache_read_tokens) {
            lines.push(format!(
                "Fresh input: {fresh} · Cache read: {cache_read} (cache does not reduce context-window usage)"
            ));
        }
    } else if let Some(estimate) = b.request_input_tokens_estimate {
        lines.push(format!("Provider input estimate: {estimate} tokens"));
    }

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
    if b.context_input_tokens.is_none() {
        lines.push("_Estimated from the exact Provider request manifest; actual usage is shown after the Provider reports it._".to_string());
    }

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
    // Markdown list item — `-` so consecutive rows render one-per-line as a
    // real list, and `**label**` with NO padding inside the markers so bold
    // actually parses (fixed-width column padding broke it: `**label   **`
    // has a trailing space before `**`, which Markdown won't bold, and the
    // alignment collapses in HTML anyway).
    format!("- **{}**: {} ({}%)", label, k, pct)
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
