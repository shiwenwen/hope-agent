use super::constants::{
    build_permission_mode_guidance, build_tool_budget_guidance, APP_INTRO,
    HUMAN_IN_THE_LOOP_GUIDANCE, MARKDOWN_PATH_LINKS_GUIDANCE, MAX_FILE_CHARS, MEMORY_GUIDELINES,
    SOUL_EMBODIMENT_GUIDANCE, TOOL_CALL_NARRATION_GUIDANCE,
};
use super::helpers::{truncate, truncate_retained_ranges};
use super::sections::*;
use super::working_dir_instructions::collect_working_dir_instructions;
use crate::agent_config::AgentDefinition;
use crate::execution_mode::ExecutionMode;
use crate::memory::{MemoryBudgetConfig, MemoryEntry};
use crate::permission::SessionMode;
use crate::project::Project;
use crate::skills;
use crate::user_config;

// ── Build System Prompt ──────────────────────────────────────────

// Per-section cap for the Pinned Claims segment. Constant for now (not a
// user-config field): it `.min(remaining)` downstream, so claims still share
// the one `effective_memory_budget` pool — this only bounds how much a single
// segment can take before the rest of the budget flows to legacy memory.
const PINNED_CLAIMS_CHARS: usize = 2500;

/// Build the complete system prompt from an AgentDefinition.
///
/// Assembly order (13 sections):
/// ① Identity line
/// ② agent.md — what this agent does
/// ③ persona.md — personality
/// ④ User context — from user.json
/// ⑤ tools.md — custom tool guidance
/// ⑥ Tool definitions — per-tool descriptions (filtered by agent config)
/// ⑥b Deferred tools listing (conditional)
/// ⑥c Tool-call narration guidance (hardcoded, always injected)
/// ⑥c³ Markdown path links guidance (desktop runtime only)
/// ⑥d Human-in-the-loop guidance (conditional, hardcoded)
/// ⑦ Skills — available skill descriptions (filtered)
/// ⑧ Memory — injected from memory backend
/// ⑨ Runtime info — date, OS, etc.
/// ⑩ Sub-agent delegation (conditional)
/// ⑪ Sandbox mode (conditional)
/// ⑦b Current Project (conditional — when session belongs to a project)
/// ⑦d Session working directory + a top-level file listing + auto-injected
///     AGENTS.md/CLAUDE.md and transitive `@`-includes (conditional)
/// ⑬ ACP external agents (conditional)
pub fn build(
    definition: &AgentDefinition,
    model: Option<&str>,
    provider: Option<&str>,
    memory_entries: &[MemoryEntry],
    memory_budget: &MemoryBudgetConfig,
    profile_snapshot: Option<&str>,
    context_pack: Option<&crate::memory::dreaming::MemoryContextPack>,
    agent_home: Option<&str>,
    project: Option<&Project>,
    session_id: Option<&str>,
    incognito: bool,
    session_working_dir: Option<&str>,
    channel_info: Option<&crate::session::ChannelSessionInfo>,
    permission_mode: SessionMode,
    execution_mode: ExecutionMode,
    workflow_mode: crate::workflow_mode::WorkflowMode,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    if definition.config.openclaw_mode {
        // ── 4-file markdown prompt mode (AGENTS.md, SOUL.md, IDENTITY.md, TOOLS.md) ──

        // Minimal identity line
        sections.push(format!(
            "You are {}, running in Hope Agent on {} {}.",
            definition.config.name, os, arch
        ));
        push_avatar_line(&mut sections, definition.config.avatar.as_deref());
        sections.push(APP_INTRO.to_string());

        // # Project Context — fixed 4-file order
        let mut project_ctx = String::from(
            "# Project Context\n\nThe following project context files have been loaded:",
        );

        let project_files: [(&str, &Option<String>); 4] = [
            ("AGENTS.md", &definition.agents_md),
            ("SOUL.md", &definition.soul_md),
            ("IDENTITY.md", &definition.identity_md),
            ("TOOLS.md", &definition.tools_guide),
        ];
        let mut has_soul = false;
        for (name, content) in &project_files {
            if let Some(md) = content.as_deref().filter(|s| !s.trim().is_empty()) {
                project_ctx.push_str(&format!("\n\n## {}\n\n", name));
                project_ctx.push_str(&truncate(md, MAX_FILE_CHARS));
                if *name == "SOUL.md" {
                    has_soul = true;
                }
            }
        }

        sections.push(project_ctx);

        // SOUL.md embodiment guidance
        if has_soul {
            sections.push(SOUL_EMBODIMENT_GUIDANCE.to_string());
        }
    } else {
        // ── Structured mode: assemble from config fields + optional supplements ──

        let soul_md_mode = matches!(
            definition.config.personality.mode,
            crate::agent_config::PersonaMode::SoulMd
        );

        // ① Identity — omit role_suffix in SOUL.md mode so the markdown's
        //    self-declared identity is not double-declared with the structured role.
        let role_suffix = if soul_md_mode {
            String::new()
        } else {
            definition
                .config
                .personality
                .role
                .as_deref()
                .filter(|r| !r.is_empty())
                .map(|r| format!(", a {}", r))
                .unwrap_or_default()
        };
        sections.push(format!(
            "You are {}{}, running in Hope Agent on {} {}.",
            definition.config.name, role_suffix, os, arch
        ));
        push_avatar_line(&mut sections, definition.config.avatar.as_deref());
        sections.push(APP_INTRO.to_string());

        // ② Personality — SoulMd mode injects soul.md verbatim + embodiment
        //    guidance; Structured mode (default) assembles from role/tone/values.
        //    Structured fields remain persisted in agent.json either way so the
        //    user can switch back without data loss.
        if soul_md_mode {
            if let Some(md) = definition
                .soul_md
                .as_deref()
                .filter(|s| !s.trim().is_empty())
            {
                sections.push(truncate(md, MAX_FILE_CHARS));
                sections.push(SOUL_EMBODIMENT_GUIDANCE.to_string());
            }
        } else {
            let personality_section = build_personality_section(&definition.config.personality);
            if !personality_section.is_empty() {
                sections.push(personality_section);
            }
        }

        // ③ agent.md — supplementary identity notes
        if let Some(md) = definition
            .agent_md
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            sections.push(truncate(md, MAX_FILE_CHARS));
        }

        // ④ persona.md — supplementary personality notes
        if let Some(persona) = definition
            .persona
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            sections.push(truncate(persona, MAX_FILE_CHARS));
        }
    }

    // ④ User context
    if let Ok(user_cfg) = user_config::load_user_config() {
        if let Some(user_section) = user_config::build_user_context(&user_cfg) {
            sections.push(user_section);
        }
    }

    // ⑤ tools.md (skip in 4-file mode — already included in Project Context)
    if !definition.config.openclaw_mode {
        if let Some(guide) = &definition.tools_guide {
            sections.push(truncate(guide, MAX_FILE_CHARS));
        }
    }

    // ⑥ Tool definitions (driven by dispatch::resolve_tool_fate)
    sections.push(build_tools_section(
        &definition.id,
        &definition.config,
        incognito,
    ));

    // ⑥b Deferred tools listing (when deferred loading is enabled)
    if let Some(deferred_section) =
        build_deferred_tools_section(&definition.id, &definition.config, incognito)
    {
        sections.push(deferred_section);
    }

    // ⑥b² Async tool execution guide (when the feature is enabled)
    if let Some(async_section) = build_async_tools_section() {
        sections.push(async_section);
    }

    // ⑥c Tool-call narration guidance — opt-in via `AppConfig.tool_call_narration_enabled`.
    if crate::config::cached_config().tool_call_narration_enabled {
        sections.push(TOOL_CALL_NARRATION_GUIDANCE.to_string());
    }

    // ⑥c³ Desktop only — server's browser can't reach the local FS, ACP
    // routes paths through external editors.
    if crate::app_init::is_desktop() {
        sections.push(MARKDOWN_PATH_LINKS_GUIDANCE.to_string());
    }

    // ⑥c¹ Permission-mode guidance. Living near the prompt tail keeps mode
    // flips from invalidating the larger static prefix cache.
    sections.push(build_permission_mode_guidance(permission_mode));

    // ⑥c¹½ Execution mode policy. Session-scoped and intentionally near other
    // dynamic execution controls so /mode flips do not churn the larger prefix.
    if let Some(section) = execution_mode.system_prompt_section() {
        sections.push(section.to_string());
    }

    // ⑥c¹¾ Workflow Mode policy. Session-scoped and placed near the other
    // dynamic execution controls so mode flips avoid churning the static
    // prefix cache.
    if let Some(section) = workflow_mode.system_prompt_section() {
        sections.push(section.to_string());
    }

    if let Some(section) = build_active_goal_section(session_id, incognito) {
        sections.push(section);
    }

    // ⑥c² Tool-call budget reminder — always injected when rounds are bounded,
    // so the model can produce a graceful handoff instead of a cut-off mid-call.
    if let Some(budget) = build_tool_budget_guidance(definition.config.capabilities.max_tool_rounds)
    {
        sections.push(budget);
    }

    // ⑥d Human-in-the-loop guidance — hardcoded so it cannot be overridden by
    // a user-customized agent.md. `ask_user_question` is a Core Interaction
    // tool, so this guidance is always available alongside its schema.
    sections.push(HUMAN_IN_THE_LOOP_GUIDANCE.to_string());

    // ⑦ Skills (filtered by agent config + per-session `paths:` activation)
    sections.push(build_skills_section(
        &definition.config.capabilities.skills,
        definition.config.capabilities.skill_env_check,
        session_id,
    ));

    // ⑦b Current Project — injected before Memory so the LLM knows which
    // project context it's in before reading project-scoped memories.
    // Only in non-openclaw mode (openclaw already uses a "Project Context"
    // heading for its 4-file markdown pack).
    if !definition.config.openclaw_mode {
        if let Some(proj) = project {
            sections.push(build_project_context_section(proj));
        }
    }

    // ⑦d User-selected working directory for this session. Injected after
    //     project context so the model treats it as the operational focus
    //     before it reads memory / runtime info.
    //
    //     If the working directory contains an AGENTS.md (or fallback
    //     CLAUDE.md), it (and its transitively `@`-included files) is
    //     loaded synchronously and rendered inside the same section, so
    //     project conventions reach the model on every turn without the
    //     user having to repeat them. See
    //     `system_prompt::working_dir_instructions` for the discovery rules.
    if let Some(wd) = session_working_dir.map(str::trim).filter(|s| !s.is_empty()) {
        let instructions = collect_working_dir_instructions(wd);
        sections.push(build_session_working_dir_section(wd, &instructions));
    }

    // ⑦e IM channel attachment — injected for sessions attached to an IM chat,
    // including desktop / HTTP turns whose replies may be mirrored to IM.
    if let Some(info) = channel_info {
        sections.push(build_im_channel_attachment_section(info));
    }

    // ⑧ Memory — layered budget negotiation (see `build_memory_section`).
    let long_term_memory_enabled = crate::memory::load_extract_config().enabled;
    if long_term_memory_enabled && definition.config.memory.enabled && !incognito {
        let section = build_memory_section(
            definition.memory_md.as_deref(),
            definition.global_memory_md.as_deref(),
            memory_entries,
            memory_budget,
            profile_snapshot,
            context_pack,
        );
        if !section.is_empty() {
            sections.push(section);
        }
    }

    if incognito {
        sections.push(build_incognito_section());
    }

    // ⑨ Runtime info
    sections.push(build_runtime_section(model, provider, agent_home));

    // ⑩ Sub-agent delegation (conditionally injected — gated by Tier 3 toggle)
    if crate::tools::subagent::subagent_capability_enabled(&definition.id, &definition.config) {
        let subagent_section =
            build_subagent_section(&definition.config.subagents, &definition.id, 0);
        if !subagent_section.is_empty() {
            sections.push(subagent_section);
        }
    }

    // ⑩½ Agent Team (conditionally injected)
    if definition.config.team.enabled {
        let team_section = build_team_section();
        if !team_section.is_empty() {
            sections.push(team_section);
        }
    }

    // ⑪ Sandbox mode (conditionally injected)
    let sandbox_mode = session_id
        .and_then(|sid| crate::session::lookup_session_meta(Some(sid)).map(|m| m.sandbox_mode))
        .unwrap_or_else(|| {
            definition
                .config
                .capabilities
                .effective_default_sandbox_mode()
        });
    if sandbox_mode.enabled() {
        let sandbox_config = crate::sandbox::load_sandbox_config().unwrap_or_default();
        sections.push(build_sandbox_mode_section(sandbox_mode, &sandbox_config));
    }

    // ⑬ ACP external agent delegation (conditionally injected)
    if definition.config.acp.enabled {
        let acp_section = build_acp_section();
        if !acp_section.is_empty() {
            sections.push(acp_section);
        }
    }

    // ⑭ Weather context (from cached weather data)
    if let Some(weather_text) = crate::weather::get_weather_for_prompt() {
        sections.push(weather_text);
    }

    // ⑮ Working-directory file listing — emitted LAST so adding/removing a
    //    top-level file only invalidates this trailing block; the larger prefix
    //    (tools / skills / memory / …) stays cache-stable across turns. Same
    //    gating as the working-dir section above.
    if let Some(wd) = session_working_dir.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(files_section) = build_working_dir_files_section(wd) {
            sections.push(files_section);
        }
    }

    // Join all non-empty sections
    let section_lengths: Vec<usize> = sections.iter().map(|s| s.len()).collect();
    let section_debug: Vec<_> = sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            serde_json::json!({
                "index": index,
                "label": section_debug_label(index),
                "chars": section.len(),
                "fingerprint": short_fingerprint(section),
            })
        })
        .collect();
    let prompt = sections
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    // Log system prompt build result
    if let Some(logger) = crate::get_logger() {
        logger.log(
            "debug",
            "agent",
            "system_prompt::build",
            &format!(
                "System prompt built: {} chars, {} sections",
                prompt.len(),
                section_lengths.len()
            ),
            Some(
                serde_json::json!({
                    "total_length": prompt.len(),
                    "prompt_fingerprint": short_fingerprint(&prompt),
                    "section_count": section_lengths.len(),
                    "section_lengths": section_lengths,
                    "section_debug": section_debug,
                    "agent_name": &definition.config.name,
                    "openclaw_mode": definition.config.openclaw_mode,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    prompt
}

fn short_fingerprint(text: &str) -> String {
    let mut hash = blake3::hash(text.as_bytes()).to_hex().to_string();
    hash.truncate(16);
    hash
}

fn section_debug_label(index: usize) -> String {
    format!("section_{index}")
}

/// Build the Memory section with layered budget negotiation.
///
/// Priority: Guidelines (always reserved) > Agent Core Memory > Global Core
/// Memory > SQLite summary. Each core-memory layer trims to
/// `min(per_layer_cap, remaining_total)`; low-priority layers drop out first
/// when the total budget is tight. SQLite's 5 sub-sections are proportionally
/// scaled into whatever budget remains after Layer 1/2.
pub(super) fn build_memory_section(
    agent_memory_md: Option<&str>,
    global_memory_md: Option<&str>,
    memory_entries: &[MemoryEntry],
    budget: &MemoryBudgetConfig,
    profile_snapshot: Option<&str>,
    context_pack: Option<&crate::memory::dreaming::MemoryContextPack>,
) -> String {
    if budget.total_chars == 0 {
        return String::new();
    }
    let mut out = String::new();
    // Reserve Guidelines up-front so a large memory.md cannot crowd it out.
    let mut remaining = budget.total_chars.saturating_sub(MEMORY_GUIDELINES.len());

    push_core_memory_layer(
        &mut out,
        &mut remaining,
        agent_memory_md,
        "## Core Memory (Agent)\n\n",
        budget.core_memory_file_chars,
    );
    push_core_memory_layer(
        &mut out,
        &mut remaining,
        global_memory_md,
        "## Core Memory (Global)\n\n",
        budget.core_memory_file_chars,
    );

    // Context Pack — Pinned Claims (design §4.8): high-salience active claims
    // fold into this static prefix and share the same `remaining` budget pool as
    // Core Memory. Priority Core > Pinned > (Profile + legacy SQLite): claim
    // facts outrank legacy memory (the single-source goal). Each line is
    // sanitized at render time (context_pack.rs) and rides the existing prefix
    // cache block — Anthropic's 4 cache_control breakpoints are full, so no new
    // dynamic block. (Strict §4.8 orders Profile ahead of Pinned; Profile stays
    // inside the SQLite block to avoid splitting its snapshot/legacy-fallback
    // dual path — claim facts still outrank legacy memory either way.)
    if let Some(pack) = context_pack {
        push_core_memory_layer(
            &mut out,
            &mut remaining,
            Some(pack.pinned_claims_md.as_str()),
            "## Pinned Memory\n\n",
            PINNED_CLAIMS_CHARS,
        );
    }

    // A profile snapshot renders the `## User Profile` section even when there
    // are no legacy SQLite memory entries, so it must not be gated out by an
    // empty entry list.
    let has_profile_snapshot = profile_snapshot
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    if (!memory_entries.is_empty() || has_profile_snapshot) && remaining > 0 {
        let sqlite_cap = remaining.min(budget.sqlite_sections.total());
        let scaled = budget.sqlite_sections.scaled_to(sqlite_cap);
        let sqlite_block = crate::memory::sqlite::format_prompt_summary_v2(
            memory_entries,
            &scaled,
            sqlite_cap,
            budget.sqlite_entry_max_chars,
            profile_snapshot,
        );
        if !sqlite_block.is_empty() {
            out.push_str(&sqlite_block);
            out.push_str("\n\n");
        }
    }

    out.push_str(MEMORY_GUIDELINES);
    out
}

/// Return the exact SQLite-memory character cap left after the cache-stable
/// static memory layers consume their priority budget. This mirrors
/// [`build_memory_section`] so Retrieval Planner trace code can ask the SQLite
/// renderer which legacy memory rows actually made it into the prompt.
pub(crate) fn sqlite_memory_budget_after_static_layers(
    agent_memory_md: Option<&str>,
    global_memory_md: Option<&str>,
    budget: &MemoryBudgetConfig,
    context_pack: Option<&crate::memory::dreaming::MemoryContextPack>,
) -> usize {
    if budget.total_chars == 0 {
        return 0;
    }
    let mut remaining = budget.total_chars.saturating_sub(MEMORY_GUIDELINES.len());
    debit_core_memory_layer(
        &mut remaining,
        agent_memory_md,
        "## Core Memory (Agent)\n\n",
        budget.core_memory_file_chars,
    );
    debit_core_memory_layer(
        &mut remaining,
        global_memory_md,
        "## Core Memory (Global)\n\n",
        budget.core_memory_file_chars,
    );
    if let Some(pack) = context_pack {
        debit_core_memory_layer(
            &mut remaining,
            Some(pack.pinned_claims_md.as_str()),
            "## Pinned Memory\n\n",
            PINNED_CLAIMS_CHARS,
        );
    }
    remaining.min(budget.sqlite_sections.total())
}

/// Return only the Context Pack sources whose complete bullet lines survive the
/// exact static-memory budget and head/tail truncation used by
/// [`build_memory_section`]. Partial lines fail closed: trace code must never
/// report a claim as injected when the model did not receive its complete line.
pub(crate) fn rendered_pinned_memory_sources(
    agent_memory_md: Option<&str>,
    global_memory_md: Option<&str>,
    budget: &MemoryBudgetConfig,
    context_pack: &crate::memory::dreaming::MemoryContextPack,
) -> Vec<crate::memory::dreaming::SourceRef> {
    if budget.total_chars == 0 {
        return Vec::new();
    }
    let mut remaining = budget.total_chars.saturating_sub(MEMORY_GUIDELINES.len());
    debit_core_memory_layer(
        &mut remaining,
        agent_memory_md,
        "## Core Memory (Agent)\n\n",
        budget.core_memory_file_chars,
    );
    debit_core_memory_layer(
        &mut remaining,
        global_memory_md,
        "## Core Memory (Global)\n\n",
        budget.core_memory_file_chars,
    );
    let md = context_pack.pinned_claims_md.as_str();
    let Some(body_cap) = core_memory_layer_body_cap(
        remaining,
        Some(md),
        "## Pinned Memory\n\n",
        PINNED_CLAIMS_CHARS,
    ) else {
        return Vec::new();
    };
    let retained_ranges = truncate_retained_ranges(md, body_cap);
    let mut cursor = 0;
    let mut rendered = Vec::new();

    for source in context_pack
        .source_digest
        .iter()
        .filter(|source| source.section == "pinned")
    {
        let rendered_line = format!("- {}\n", source.preview);
        let Some(relative_start) = md[cursor..].find(&rendered_line) else {
            continue;
        };
        let start = cursor + relative_start;
        let end = start + rendered_line.len();
        cursor = end;
        if retained_ranges
            .iter()
            .any(|&(range_start, range_end)| start >= range_start && end <= range_end)
        {
            rendered.push(source.clone());
        }
    }

    rendered
}

/// Append one heading + truncated body + trailer block, debiting `remaining`.
/// No-op when `md` is `None` / blank, when `remaining` is already 0, or when
/// the heading alone wouldn't fit.
fn push_core_memory_layer(
    out: &mut String,
    remaining: &mut usize,
    md: Option<&str>,
    heading: &str,
    per_layer_cap: usize,
) {
    let Some(md) = md.filter(|s| !s.trim().is_empty()) else {
        return;
    };
    const TRAILER: &str = "\n\n";
    let overhead = heading.len() + TRAILER.len();
    let Some(body_cap) = core_memory_layer_body_cap(*remaining, Some(md), heading, per_layer_cap)
    else {
        return;
    };
    let chunk = truncate(md, body_cap);
    out.push_str(heading);
    out.push_str(&chunk);
    out.push_str(TRAILER);
    *remaining = remaining.saturating_sub(chunk.len() + overhead);
}

fn core_memory_layer_body_cap(
    remaining: usize,
    md: Option<&str>,
    heading: &str,
    per_layer_cap: usize,
) -> Option<usize> {
    let _md = md.filter(|s| !s.trim().is_empty())?;
    if remaining == 0 {
        return None;
    }
    const TRAILER: &str = "\n\n";
    let overhead = heading.len() + TRAILER.len();
    let body_cap = per_layer_cap.min(remaining.saturating_sub(overhead));
    if body_cap == 0 {
        return None;
    }
    Some(body_cap)
}

fn debit_core_memory_layer(
    remaining: &mut usize,
    md: Option<&str>,
    heading: &str,
    per_layer_cap: usize,
) {
    let Some(md) = md.filter(|s| !s.trim().is_empty()) else {
        return;
    };
    if *remaining == 0 {
        return;
    }
    const TRAILER: &str = "\n\n";
    let overhead = heading.len() + TRAILER.len();
    let body_cap = per_layer_cap.min(remaining.saturating_sub(overhead));
    if body_cap == 0 {
        return;
    }
    let chunk = truncate(md, body_cap);
    *remaining = remaining.saturating_sub(chunk.len() + overhead);
}

/// Append an avatar line right after the identity sentence so the model knows
/// where to find its avatar image (local path or URL). The frontend renders
/// avatars from the same string, so a markdown image reference produced by the
/// model will resolve to the user-configured avatar.
///
/// Skips `data:` URLs (OpenClaw import accepts them — see
/// `openclaw_import::agents::is_remote_avatar`) because base64-embedded images
/// can run tens to hundreds of KB and would bloat every turn's system prompt.
/// Also caps total length defensively in case some other string ever slips in.
fn push_avatar_line(sections: &mut Vec<String>, avatar: Option<&str>) {
    const MAX_AVATAR_LEN: usize = 1024;
    let Some(avatar) = avatar.map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    if avatar.starts_with("data:") || avatar.len() > MAX_AVATAR_LEN {
        return;
    }
    sections.push(format!("Your avatar image is at: {}", avatar));
}

fn build_incognito_section() -> String {
    "# Incognito Session\n\n\
     This session is running in incognito mode.\n\
     - Do not use memory or awareness.\n\
     - Do not infer, recall, update, delete, or store long-term memory in this session.\n\
     - Long-term memory tools are unavailable here; answer from the current conversation only.\n\
     - Treat this as a forward-looking rule for the current session only."
        .to_string()
}

fn build_active_goal_section(session_id: Option<&str>, incognito: bool) -> Option<String> {
    if incognito {
        return None;
    }
    let session_id = session_id?;
    let db = crate::get_session_db()?;
    let snapshot = db.active_goal_for_session(session_id).ok()??;
    Some(render_active_goal_section(&snapshot))
}

fn render_active_goal_section(snapshot: &crate::goal::GoalSnapshot) -> String {
    let goal = &snapshot.goal;
    let mut lines = vec![
        "# Active Goal".to_string(),
        String::new(),
        "The user has set a durable goal for this session. Treat it as the current north star until the user changes, pauses, clears, or completes it.".to_string(),
        "Goal Runtime Contract: autonomously keep working toward this goal across long tasks; do not stop merely because one turn is long or a subtask is hard. Maintain evidence, recover from interruptions, and only finish after the current revision's completion criteria are actually satisfied.".to_string(),
        "Autonomous execution rules: when the next in-scope action is already determined, perform it in this turn instead of ending with a plan, promise, or list of next steps. If the user asks a side question while the Goal remains active, answer it and then continue unless they changed, paused, or cancelled the Goal.".to_string(),
        "Scope rules: reversible work that directly advances the Goal should proceed without needless reconfirmation, while irreversible, outward-facing, or genuinely scope-changing actions still require the normal permission or user decision. If the user's Goal is an assessment or answer rather than a requested change, the assessment is the deliverable; do not mutate merely because Goal Mode is active. Do not add unrelated features, cleanup, or abstractions.".to_string(),
        "Use the Goal tools when useful: `goal_status` to re-read the latest objective/revision/budget; `goal_prepare_contract` early to form a gradeable rubric and viability preflight when the objective is underspecified (without expanding scope, and while preserving explicit criteria exactly); `goal_checkpoint` after meaningful milestones or handoffs; `goal_record_evidence` for truthful general-domain evidence; `goal_evaluate` before any completion claim; `goal_finish_request` only when the audit should pass; and `goal_block_request` only after repeated failed attempts or a real user/external blocker.".to_string(),
        format!("- State: {}", goal.state.as_str()),
        format!("- Revision: {}", goal.revision),
        format!("- Objective: {}", truncate(&goal.objective, 1200)),
    ];
    if snapshot.audit_stale {
        lines.push("- Latest audit: stale because the goal revision or linked evidence changed; do not rely on the old completion conclusion until a new evaluation runs.".to_string());
    }
    if let Some(domain) = goal.domain.as_deref().filter(|s| !s.trim().is_empty()) {
        lines.push(format!("- Domain: {}", truncate(domain, 200)));
    }
    if let Some(template_id) = goal
        .workflow_template_id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        let mut template = template_id.to_string();
        if let Some(version) = goal
            .workflow_template_version
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            template.push('@');
            template.push_str(version);
        }
        lines.push(format!("- Workflow template: {}", truncate(&template, 300)));
    }
    if let Some(task_type) = goal
        .workflow_task_type
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        lines.push(format!(
            "- Workflow task type: {}",
            truncate(task_type, 200)
        ));
    }
    if !goal.completion_criteria.trim().is_empty() {
        lines.push(format!(
            "- Completion criteria: {}",
            truncate(&goal.completion_criteria, 1600)
        ));
    }
    if !snapshot.criteria_items.is_empty() {
        lines.push(
            "- Criteria ids for traceability; when creating a workflow run for a specific criterion, pass the matching `goalCriterionId`:".to_string(),
        );
        for criterion in snapshot.criteria_items.iter().take(12) {
            let check = criterion
                .check_kind
                .map(|kind| format!(", check={}", kind.as_str()))
                .unwrap_or_default();
            let expected = if criterion.expected_evidence.is_empty() {
                String::new()
            } else {
                format!(", evidence={}", criterion.expected_evidence.join("|"))
            };
            lines.push(format!(
                "  - {} [{}{}{}]: {}",
                criterion.id,
                criterion.kind.as_str(),
                check,
                expected,
                truncate(&criterion.text, 240)
            ));
        }
    }
    let required_missing = snapshot
        .criteria
        .iter()
        .filter(|criterion| {
            criterion.kind.as_str() == "required" && criterion.status.as_str() != "satisfied"
        })
        .map(|criterion| {
            format!(
                "{} ({})",
                truncate(&criterion.text, 220),
                criterion.status.as_str()
            )
        })
        .take(8)
        .collect::<Vec<_>>();
    if !required_missing.is_empty() {
        lines.push("- Required criteria still needing evidence:".to_string());
        for criterion in required_missing {
            lines.push(format!("  - {criterion}"));
        }
    }
    let follow_ups = goal
        .follow_up_items
        .iter()
        .map(|item| truncate(&item.text, 220))
        .take(6)
        .collect::<Vec<_>>();
    if !follow_ups.is_empty() {
        lines.push("- Follow-up pool, not current blockers:".to_string());
        for item in follow_ups {
            lines.push(format!("  - {item}"));
        }
    }
    if let Some(decision) = goal.closure_decision {
        lines.push(format!("- User closure decision: {}", decision.as_str()));
        if decision.as_str() != "accepted_v1" {
            lines.push("- Do not claim the goal is closed; continue only toward the requested evidence or ask the user before broadening scope.".to_string());
        }
    } else if goal.state.as_str() == "completed" {
        lines.push("- Closure is not accepted by the user yet. Present the evidence and ask whether to accept v1 closure or require stricter evidence.".to_string());
    }
    if let Some(reason) = goal
        .blocked_reason
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        lines.push(format!("- Blocked reason: {}", truncate(reason, 600)));
        if matches!(
            reason,
            "goal_evidence_incomplete" | "goal_blocked_by_evidence"
        ) {
            lines.push("- This blocked reason means the audit needs more evidence; continue producing concrete evidence unless a real user/external blocker exists.".to_string());
        }
    }
    if let Some(summary) = goal
        .final_summary
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        lines.push(format!("- Latest audit: {}", truncate(summary, 800)));
    }
    if snapshot.budget.warning || snapshot.budget.exhausted {
        lines.push(format!(
            "- Goal budget: tokens {}/{:?}, time {}s/{:?}, turns {}/{:?}; warnings={:?}; exceeded={:?}",
            snapshot.budget.tokens_used,
            snapshot.budget.token_limit,
            snapshot.budget.elapsed_secs,
            snapshot.budget.time_limit_secs,
            snapshot.budget.turns_used,
            snapshot.budget.turn_limit,
            snapshot.budget.warnings,
            snapshot.budget.exceeded
        ));
    }
    append_goal_handoff(&mut lines, snapshot);
    lines.push(
        "Behavior rules: prefer actions that create concrete evidence toward the completion criteria; keep user-visible tasks current; if the user updates the goal, immediately follow the latest revision; if you complete it, call `goal_finish_request` before the final user summary; if you cannot proceed, call `goal_block_request` with concrete attempts instead of silently stopping.".to_string(),
    );
    lines.join("\n")
}

fn append_goal_handoff(lines: &mut Vec<String>, snapshot: &crate::goal::GoalSnapshot) {
    let current_task = snapshot
        .tasks
        .iter()
        .find(|task| task.status == "in_progress")
        .or_else(|| snapshot.tasks.iter().find(|task| task.status == "pending"));
    let active_workflow = snapshot.workflow_runs.iter().find(|run| {
        matches!(
            run.state,
            crate::workflow::WorkflowRunState::Running
                | crate::workflow::WorkflowRunState::Recovering
        )
    });
    let waiting_workflow = snapshot.workflow_runs.iter().find(|run| {
        matches!(
            run.state,
            crate::workflow::WorkflowRunState::AwaitingApproval
                | crate::workflow::WorkflowRunState::AwaitingUser
                | crate::workflow::WorkflowRunState::Paused
        )
    });
    let next_evidence = snapshot
        .goal
        .last_evaluator_result
        .get("nextEvidenceNeeded")
        .or_else(|| {
            snapshot
                .goal
                .last_evaluator_result
                .get("next_evidence_needed")
        })
        .and_then(|value| value.as_array())
        .and_then(|items| items.iter().find_map(|item| item.as_str()))
        .filter(|value| !value.trim().is_empty());
    let checkpoint_next = snapshot
        .events
        .iter()
        .rev()
        .filter(|event| event.kind == "goal_checkpoint")
        .find_map(|event| event.payload.get("next").and_then(|value| value.as_str()))
        .filter(|value| !value.trim().is_empty());

    if current_task.is_none()
        && active_workflow.is_none()
        && waiting_workflow.is_none()
        && next_evidence.is_none()
        && checkpoint_next.is_none()
    {
        return;
    }

    lines.push("- Goal handoff packet:".to_string());
    if let Some(task) = current_task {
        lines.push(format!(
            "  - Current task: {} ({})",
            truncate(&task.content, 240),
            task.status
        ));
    }
    if let Some(run) = active_workflow {
        lines.push(format!(
            "  - Active workflow: {} ({})",
            truncate(&run.kind, 180),
            run.state.as_str()
        ));
    }
    if let Some(run) = waiting_workflow {
        lines.push(format!(
            "  - Waiting on workflow: {} ({})",
            truncate(&run.kind, 180),
            run.state.as_str()
        ));
    }
    if let Some(next) = next_evidence.or(checkpoint_next) {
        lines.push(format!("  - One next action: {}", truncate(next, 320)));
    }
}

/// Build a system prompt using the legacy path (no AgentDefinition).
/// This preserves backward compatibility during the transition.
pub fn build_legacy(model: Option<&str>, provider: Option<&str>, incognito: bool) -> String {
    let store = crate::config::cached_config();
    let available_skills =
        skills::load_all_skills_with_budget(&store.extra_skills_dirs, &store.skill_prompt_budget);
    // Legacy path has no session context — conditional skills stay hidden.
    let activated_conditional = std::collections::HashSet::new();
    let skills_section = skills::build_skills_prompt(
        &available_skills,
        &store.disabled_skills,
        store.skill_env_check,
        &store.skill_env,
        &store.skill_prompt_budget,
        &store.skill_allow_bundled,
        &activated_conditional,
    );

    let mut sections = Vec::new();

    // Identity + behavior guidance (from agent.md template)
    let locale = crate::agent_loader::detect_system_locale();
    sections.push(crate::agent_loader::default_agent_md(&locale).to_string());

    // User context
    if let Ok(user_cfg) = user_config::load_user_config() {
        if let Some(user_section) = user_config::build_user_context(&user_cfg) {
            sections.push(user_section);
        }
    }

    // Tools
    sections.push(build_all_tools_description(incognito));

    // Deferred tools listing — legacy path uses default agent + default config.
    let legacy_agent_config = crate::agent_config::AgentConfig::default();
    if let Some(deferred_section) = build_deferred_tools_section(
        crate::agent_loader::DEFAULT_AGENT_ID,
        &legacy_agent_config,
        incognito,
    ) {
        sections.push(deferred_section);
    }

    // Async tool execution guide
    if let Some(async_section) = build_async_tools_section() {
        sections.push(async_section);
    }

    // Tool-call narration guidance — gated on AppConfig flag (see build())
    if crate::config::cached_config().tool_call_narration_enabled {
        sections.push(TOOL_CALL_NARRATION_GUIDANCE.to_string());
    }

    // Tool-call budget reminder — legacy path has no AgentDefinition, so fall
    // back to the CapabilitiesConfig default.
    let legacy_max_rounds = crate::agent_config::CapabilitiesConfig::default().max_tool_rounds;
    if let Some(budget) = build_tool_budget_guidance(legacy_max_rounds) {
        sections.push(budget);
    }

    // Skills
    if !skills_section.is_empty() {
        sections.push(skills_section);
    }

    // Weather context
    if let Some(weather_text) = crate::weather::get_weather_for_prompt() {
        sections.push(weather_text);
    }

    // Runtime (legacy mode has no agent home)
    sections.push(build_runtime_section(model, provider, None));

    if incognito {
        sections.push(build_incognito_section());
    }

    sections.join("\n\n")
}

#[cfg(test)]
mod memory_section_tests {
    use super::*;
    use crate::agent_config::{AgentConfig, AgentDefinition};
    use crate::goal::{
        Goal, GoalBudgetSnapshot, GoalClosureDecision, GoalCriterionAudit, GoalCriterionItem,
        GoalCriterionKind, GoalCriterionStatus, GoalFollowUpItem, GoalSnapshot, GoalState,
    };
    use crate::memory::{MemoryEntry, MemoryScope, MemoryType, SqliteSectionBudgets};
    use serde_json::json;
    use std::path::PathBuf;

    fn mk_definition() -> AgentDefinition {
        AgentDefinition {
            id: crate::agent_loader::DEFAULT_AGENT_ID.into(),
            dir: PathBuf::from("/tmp/default"),
            config: AgentConfig::default(),
            agent_md: None,
            persona: None,
            tools_guide: None,
            agents_md: None,
            identity_md: None,
            soul_md: None,
            global_memory_md: Some("Global memory".into()),
            memory_md: Some("Agent memory".into()),
        }
    }

    fn mk_entry(id: i64, ty: MemoryType, content: &str) -> MemoryEntry {
        MemoryEntry {
            id,
            memory_type: ty,
            scope: MemoryScope::Global,
            content: content.to_string(),
            tags: Vec::new(),
            source: "user".into(),
            source_session_id: None,
            pinned: false,
            created_at: "2026-04-18T00:00:00Z".into(),
            updated_at: "2026-04-18T00:00:00Z".into(),
            relevance_score: None,
            attachment_path: None,
            attachment_mime: None,
        }
    }

    fn mk_goal_snapshot() -> GoalSnapshot {
        GoalSnapshot {
            goal: Goal {
                id: "goal-1".to_string(),
                session_id: "s1".to_string(),
                objective: "Ship Goal v2 review".to_string(),
                completion_criteria: "[required] status machine is stable".to_string(),
                revision: 7,
                domain: Some("coding".to_string()),
                workflow_template_id: Some("goal-v2-review".to_string()),
                workflow_template_version: Some("1".to_string()),
                workflow_task_type: Some("review".to_string()),
                state: GoalState::Blocked,
                mode_snapshot: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:10:00Z".to_string(),
                completed_at: None,
                final_summary: Some("Old audit completed before new evidence".to_string()),
                final_evidence: json!({}),
                blocked_reason: Some("needs_strict_evidence".to_string()),
                last_evaluator_result: json!({}),
                closure_decision: Some(GoalClosureDecision::NeedsStrictEvidence),
                closure_reason: Some("User asked for strict proof".to_string()),
                closed_at: None,
                follow_up_items: vec![GoalFollowUpItem {
                    id: "followup-1".to_string(),
                    text: "manual browser profile".to_string(),
                    created_at: "2026-01-01T00:11:00Z".to_string(),
                    source: Some("closure".to_string()),
                }],
            },
            links: Vec::new(),
            events: Vec::new(),
            audit_stale: true,
            criteria_items: vec![GoalCriterionItem {
                id: "criterion-1".to_string(),
                text: "status machine is stable".to_string(),
                kind: GoalCriterionKind::Required,
                check_kind: None,
                expected_evidence: Vec::new(),
                inferred: false,
            }],
            criteria: vec![GoalCriterionAudit {
                id: "criterion-1".to_string(),
                text: "status machine is stable".to_string(),
                kind: GoalCriterionKind::Required,
                status: GoalCriterionStatus::Blocked,
                evidence_ids: Vec::new(),
                reason: Some("accepted closure cannot reopen".to_string()),
            }],
            evidence: Vec::new(),
            timeline: Vec::new(),
            budget: GoalBudgetSnapshot::default(),
            workflow_runs: Vec::new(),
            tasks: Vec::new(),
            grader_runs: Vec::new(),
        }
    }

    #[test]
    fn active_goal_section_exposes_goal_v2_control_state() {
        let out = render_active_goal_section(&mk_goal_snapshot());

        assert!(out.contains("# Active Goal"));
        assert!(out.contains("Goal Runtime Contract"));
        assert!(out.contains("perform it in this turn instead of ending with a plan"));
        assert!(out.contains("If the user's Goal is an assessment or answer"));
        assert!(out.contains("goal_status"));
        assert!(out.contains("goal_finish_request"));
        assert!(out.contains("- Revision: 7"));
        assert!(out.contains("stale because the goal revision or linked evidence changed"));
        assert!(out.contains("pass the matching `goalCriterionId`"));
        assert!(out.contains("criterion-1 [required]: status machine is stable"));
        assert!(out.contains("Required criteria still needing evidence"));
        assert!(out.contains("status machine is stable (blocked)"));
        assert!(out.contains("Follow-up pool, not current blockers"));
        assert!(out.contains("manual browser profile"));
        assert!(out.contains("User closure decision: needs_strict_evidence"));
        assert!(out.contains("Do not claim the goal is closed"));
        assert!(out.contains("Blocked reason: needs_strict_evidence"));
    }

    #[test]
    fn active_goal_section_includes_durable_handoff_next_action() {
        let mut snapshot = mk_goal_snapshot();
        snapshot.goal.last_evaluator_result = json!({
            "nextEvidenceNeeded": ["Run the release smoke test"]
        });

        let out = render_active_goal_section(&snapshot);

        assert!(out.contains("Goal handoff packet"));
        assert!(out.contains("One next action: Run the release smoke test"));
    }

    #[test]
    fn memory_section_respects_total_budget_with_oversized_core_files() {
        let agent_md = "a".repeat(20_000);
        let global_md = "g".repeat(20_000);
        let budget = MemoryBudgetConfig {
            total_chars: 10_000,
            core_memory_file_chars: 8_000,
            sqlite_entry_max_chars: 500,
            sqlite_sections: SqliteSectionBudgets::default(),
        };
        let out = build_memory_section(Some(&agent_md), Some(&global_md), &[], &budget, None, None);
        // Guidelines always present.
        assert!(out.contains("## Memory Guidelines"));
        // Total stays under budget (±5% slack for heading overhead rounding).
        assert!(
            out.len() <= budget.total_chars + 200,
            "section {} chars exceeds total_chars {} too far",
            out.len(),
            budget.total_chars
        );
    }

    #[test]
    fn agent_memory_wins_when_combined_exceeds_budget() {
        // Agent-specific rules are higher priority — Agent.md 8k + Global.md
        // 8k + guidelines should leave Global truncated.
        let agent_md = "A".repeat(8_000);
        let global_md = "G".repeat(8_000);
        let budget = MemoryBudgetConfig::default();
        let out = build_memory_section(Some(&agent_md), Some(&global_md), &[], &budget, None, None);

        let agent_a_count = out.matches('A').count();
        let global_g_count = out.matches('G').count();
        // Agent.md fully preserved (head/tail truncate may keep all 8000 A's
        // when the cap equals the content length).
        assert!(
            agent_a_count >= 7_000,
            "Agent.md should be mostly intact, got {} A's",
            agent_a_count
        );
        // Global.md should have been heavily truncated since remaining budget
        // after Guidelines (~800) + Agent.md (~8000) is roughly 1200 minus
        // heading overhead.
        assert!(
            global_g_count < agent_a_count,
            "Global.md should be truncated more than Agent.md (A={} G={})",
            agent_a_count,
            global_g_count
        );
        assert!(out.contains("## Memory Guidelines"));
    }

    #[test]
    fn sqlite_gets_residual_budget_only() {
        // Small core memory leaves SQLite plenty of room.
        let agent_md = "a".repeat(1_000);
        let global_md = "g".repeat(1_000);
        let entries: Vec<MemoryEntry> = (0..5)
            .map(|i| mk_entry(i, MemoryType::User, &format!("user fact #{}", i)))
            .collect();
        let budget = MemoryBudgetConfig::default();
        let out = build_memory_section(
            Some(&agent_md),
            Some(&global_md),
            &entries,
            &budget,
            None,
            None,
        );

        assert!(out.contains("## Core Memory (Agent)"));
        assert!(out.contains("## Core Memory (Global)"));
        assert!(
            out.contains("## About the User"),
            "SQLite section should render when budget allows: {out}"
        );
        assert!(out.contains("## Memory Guidelines"));
    }

    fn pinned_source(id: &str, preview: &str) -> crate::memory::dreaming::SourceRef {
        crate::memory::dreaming::SourceRef {
            claim_id: id.to_string(),
            scope_type: "global".to_string(),
            scope_id: None,
            claim_type: "preference".to_string(),
            section: "pinned".to_string(),
            preview: preview.to_string(),
        }
    }

    #[test]
    fn pinned_memory_trace_only_returns_complete_rendered_lines() {
        let previews = [
            "first-claim-1234567890",
            "second-claim-123456789",
            "third-claim-1234567890",
        ];
        let pack = crate::memory::dreaming::MemoryContextPack {
            pinned_claims_md: previews
                .iter()
                .map(|preview| format!("- {preview}\n"))
                .collect(),
            source_digest: previews
                .iter()
                .enumerate()
                .map(|(index, preview)| pinned_source(&format!("c{}", index + 1), preview))
                .collect(),
        };
        let budget = MemoryBudgetConfig::default();
        let all_sources = rendered_pinned_memory_sources(None, None, &budget, &pack);
        assert_eq!(
            all_sources
                .iter()
                .map(|source| source.claim_id.as_str())
                .collect::<Vec<_>>(),
            vec!["c1", "c2", "c3"]
        );

        let first_line_len = format!("- {}\n", previews[0]).len();
        let body_cap = first_line_len * 2;
        let pinned_overhead = "## Pinned Memory\n\n".len() + "\n\n".len();
        let tight_budget = MemoryBudgetConfig {
            total_chars: MEMORY_GUIDELINES.len() + pinned_overhead + body_cap,
            core_memory_file_chars: 8_000,
            sqlite_entry_max_chars: 500,
            sqlite_sections: SqliteSectionBudgets::default(),
        };
        let rendered_sources = rendered_pinned_memory_sources(None, None, &tight_budget, &pack);
        assert_eq!(
            rendered_sources
                .iter()
                .map(|source| source.claim_id.as_str())
                .collect::<Vec<_>>(),
            vec!["c1"]
        );
    }

    #[test]
    fn zero_total_chars_emits_nothing() {
        let budget = MemoryBudgetConfig {
            total_chars: 0,
            ..MemoryBudgetConfig::default()
        };
        let out = build_memory_section(Some("agent"), Some("global"), &[], &budget, None, None);
        assert_eq!(out, "");
    }

    #[test]
    fn guidelines_always_reserved_even_under_pressure() {
        // total_chars just big enough for Guidelines + small headings.
        let agent_md = "x".repeat(100_000);
        let budget = MemoryBudgetConfig {
            total_chars: MEMORY_GUIDELINES.len() + 50,
            core_memory_file_chars: 8_000,
            sqlite_entry_max_chars: 500,
            sqlite_sections: SqliteSectionBudgets::default(),
        };
        let out = build_memory_section(Some(&agent_md), None, &[], &budget, None, None);
        assert!(
            out.contains("## Memory Guidelines"),
            "Guidelines must survive under budget pressure"
        );
    }

    #[test]
    fn sandbox_prompt_explains_isolated_persistence_boundary() {
        let config = crate::sandbox::SandboxConfig::default();
        let out = build_sandbox_mode_section(crate::permission::SandboxMode::Isolated, &config);
        assert!(
            out.contains("Current session sandbox mode: `isolated`"),
            "current mode should be explicit: {out}"
        );
        assert!(
            out.contains("temporary workspace copy"),
            "isolated mode must explain the temp workspace: {out}"
        );
        assert!(
            out.contains("command-created file changes are not durable"),
            "isolated mode must warn about discarded command-created files: {out}"
        );
    }

    #[test]
    fn sandbox_prompt_explains_file_tools_are_host_side() {
        let config = crate::sandbox::SandboxConfig::default();
        let out = build_sandbox_mode_section(crate::permission::SandboxMode::Workspace, &config);
        assert!(
            out.contains("Current session sandbox mode: `workspace`"),
            "current mode should be explicit: {out}"
        );
        assert!(
            out.contains("Direct file tools such as `write`, `edit`, and `apply_patch`"),
            "prompt must name durable host-side file tools: {out}"
        );
        assert!(
            out.contains("not automatically sandboxed by the mode"),
            "prompt must prevent over-generalizing sandbox mode to file tools: {out}"
        );
    }

    #[test]
    fn sandbox_prompt_reflects_current_docker_config() {
        let config = crate::sandbox::SandboxConfig {
            image: "custom:latest".to_string(),
            read_only: false,
            network_mode: "bridge".to_string(),
            cap_drop_all: false,
            no_new_privileges: false,
            pids_limit: None,
            tmpfs: Vec::new(),
            ..crate::sandbox::SandboxConfig::default()
        };
        let out = build_sandbox_mode_section(crate::permission::SandboxMode::Trusted, &config);
        assert!(out.contains("Container image: `custom:latest`"), "{out}");
        assert!(out.contains("Docker network mode: `bridge`"), "{out}");
        assert!(out.contains("Container root filesystem: writable"), "{out}");
        assert!(
            out.contains("Linux capabilities are not globally dropped"),
            "{out}"
        );
        assert!(out.contains("no-new-privileges is disabled"), "{out}");
        assert!(out.contains("PID limit: unlimited"), "{out}");
        assert!(
            !out.contains("no network, a read-only root filesystem"),
            "prompt must not hard-code the default sandbox constraints: {out}"
        );
    }

    #[test]
    fn working_dir_section_injected_when_path_provided() {
        let definition = mk_definition();
        let budget = MemoryBudgetConfig::default();
        let out = build(
            &definition,
            Some("gpt-5.4"),
            Some("OpenAI"),
            &[],
            &budget,
            None,
            None,
            None,
            None,
            None,
            false,
            Some("/srv/projects/demo"),
            None,
            SessionMode::Default,
            ExecutionMode::Off,
            crate::workflow_mode::WorkflowMode::Off,
        );
        assert!(
            out.contains("# Working Directory"),
            "expected working directory heading to appear: {out}"
        );
        assert!(
            out.contains("/srv/projects/demo"),
            "expected selected path to appear in prompt: {out}"
        );
    }

    #[test]
    fn working_dir_section_omitted_when_missing_or_blank() {
        let definition = mk_definition();
        let budget = MemoryBudgetConfig::default();
        let out_none = build(
            &definition,
            Some("gpt-5.4"),
            Some("OpenAI"),
            &[],
            &budget,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            SessionMode::Default,
            ExecutionMode::Off,
            crate::workflow_mode::WorkflowMode::Off,
        );
        let out_blank = build(
            &definition,
            Some("gpt-5.4"),
            Some("OpenAI"),
            &[],
            &budget,
            None,
            None,
            None,
            None,
            None,
            false,
            Some("   "),
            None,
            SessionMode::Default,
            ExecutionMode::Off,
            crate::workflow_mode::WorkflowMode::Off,
        );
        assert!(
            !out_none.contains("# Working Directory"),
            "no working_dir should omit section"
        );
        assert!(
            !out_blank.contains("# Working Directory"),
            "blank working_dir should omit section"
        );
    }

    #[test]
    fn execution_mode_prompt_injected_only_when_enabled() {
        let definition = mk_definition();
        let budget = MemoryBudgetConfig::default();
        let out_off = build(
            &definition,
            Some("gpt-5.4"),
            Some("OpenAI"),
            &[],
            &budget,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            SessionMode::Default,
            ExecutionMode::Off,
            crate::workflow_mode::WorkflowMode::Off,
        );
        let out_guarded = build(
            &definition,
            Some("gpt-5.4"),
            Some("OpenAI"),
            &[],
            &budget,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            SessionMode::Default,
            ExecutionMode::Guarded,
            crate::workflow_mode::WorkflowMode::Off,
        );

        assert!(
            !out_off.contains("# Execution Mode"),
            "off mode should not inject execution policy: {out_off}"
        );
        assert!(out_guarded.contains("# Execution Mode: Guarded"));
        assert!(out_guarded.contains("observe -> plan -> edit -> targeted validate -> report"));
        assert!(out_guarded.contains("Stop and ask the user"));
    }

    #[test]
    fn workflow_mode_prompt_injected_only_when_enabled() {
        let definition = mk_definition();
        let budget = MemoryBudgetConfig::default();
        let out_off = build(
            &definition,
            Some("gpt-5.4"),
            Some("OpenAI"),
            &[],
            &budget,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            SessionMode::Default,
            ExecutionMode::Off,
            crate::workflow_mode::WorkflowMode::Off,
        );
        let out_on = build(
            &definition,
            Some("gpt-5.4"),
            Some("OpenAI"),
            &[],
            &budget,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            SessionMode::Default,
            ExecutionMode::Off,
            crate::workflow_mode::WorkflowMode::On,
        );
        let out_ultracode = build(
            &definition,
            Some("gpt-5.4"),
            Some("OpenAI"),
            &[],
            &budget,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            SessionMode::Default,
            ExecutionMode::Off,
            crate::workflow_mode::WorkflowMode::Ultracode,
        );

        assert!(
            !out_off.contains("# Workflow Mode"),
            "off mode should not inject workflow policy: {out_off}"
        );
        assert!(out_on.contains("# Workflow Mode: On"));
        assert!(out_on.contains("workflow` with `action=create"));
        assert!(out_on.contains("workflow.task.create"));
        assert!(out_on.contains("Workflow is not coding-only"));
        assert!(out_ultracode.contains("# Workflow Mode: Ultracode"));
        assert!(out_ultracode.contains("Use `workflow` with `action=create` by default"));
    }

    #[test]
    fn runtime_section_labels_agent_home_separately_from_session_working_dir() {
        let out = build_runtime_section(
            Some("gpt-5.4"),
            Some("OpenAI"),
            Some("/tmp/hope-agent/coder-home"),
        );

        assert!(
            out.contains("- Agent home: /tmp/hope-agent/coder-home"),
            "agent home should be named as agent home: {out}"
        );
        assert!(
            !out.contains("- Working directory: /tmp/hope-agent/coder-home"),
            "agent home should not be presented as the session working directory: {out}"
        );
    }

    #[test]
    fn markdown_path_links_guidance_skipped_outside_desktop_runtime() {
        // ha-core tests share a single binary and `init_runtime("test")`
        // pins `runtime_role()` to a non-desktop value, so the guidance
        // section must not appear. (We can't directly test the desktop
        // injection here for the same reason — it's covered end-to-end.)
        let definition = mk_definition();
        let budget = MemoryBudgetConfig::default();
        let out = build(
            &definition,
            Some("gpt-5.4"),
            Some("OpenAI"),
            &[],
            &budget,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            SessionMode::Default,
            ExecutionMode::Off,
            crate::workflow_mode::WorkflowMode::Off,
        );
        assert!(
            !out.contains("# File Path Formatting"),
            "non-desktop runtime should skip path-links guidance: {out}"
        );
    }

    #[test]
    fn avatar_line_injected_when_configured() {
        let mut definition = mk_definition();
        definition.config.avatar = Some("/Users/me/.hope-agent/avatars/foo.png".into());
        let budget = MemoryBudgetConfig::default();
        let out = build(
            &definition,
            Some("gpt-5.4"),
            Some("OpenAI"),
            &[],
            &budget,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            SessionMode::Default,
            ExecutionMode::Off,
            crate::workflow_mode::WorkflowMode::Off,
        );
        assert!(
            out.contains("Your avatar image is at: /Users/me/.hope-agent/avatars/foo.png"),
            "structured-mode prompt should include avatar line: {out}"
        );
    }

    #[test]
    fn avatar_line_omitted_when_blank_or_missing() {
        let mut definition = mk_definition();
        definition.config.avatar = Some("   ".into());
        let budget = MemoryBudgetConfig::default();
        let out_blank = build(
            &definition,
            Some("gpt-5.4"),
            Some("OpenAI"),
            &[],
            &budget,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            SessionMode::Default,
            ExecutionMode::Off,
            crate::workflow_mode::WorkflowMode::Off,
        );
        definition.config.avatar = None;
        let out_none = build(
            &definition,
            Some("gpt-5.4"),
            Some("OpenAI"),
            &[],
            &budget,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            SessionMode::Default,
            ExecutionMode::Off,
            crate::workflow_mode::WorkflowMode::Off,
        );
        assert!(!out_blank.contains("Your avatar image is at:"));
        assert!(!out_none.contains("Your avatar image is at:"));
    }

    #[test]
    fn avatar_line_skipped_for_data_url() {
        let mut definition = mk_definition();
        definition.config.avatar = Some(
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNgAAIAAAUAAeImBZsAAAAASUVORK5CYII="
                .into(),
        );
        let budget = MemoryBudgetConfig::default();
        let out = build(
            &definition,
            Some("gpt-5.4"),
            Some("OpenAI"),
            &[],
            &budget,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            SessionMode::Default,
            ExecutionMode::Off,
            crate::workflow_mode::WorkflowMode::Off,
        );
        assert!(
            !out.contains("Your avatar image is at:"),
            "data: URLs must not be injected (would bloat prompt with base64): {out}"
        );
        assert!(!out.contains("data:image/png"));
    }

    #[test]
    fn avatar_line_skipped_when_path_is_oversized() {
        let mut definition = mk_definition();
        definition.config.avatar = Some(format!("https://example.com/{}.png", "a".repeat(2_000)));
        let budget = MemoryBudgetConfig::default();
        let out = build(
            &definition,
            Some("gpt-5.4"),
            Some("OpenAI"),
            &[],
            &budget,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            SessionMode::Default,
            ExecutionMode::Off,
            crate::workflow_mode::WorkflowMode::Off,
        );
        assert!(
            !out.contains("Your avatar image is at:"),
            "oversized avatar string must not be injected: prompt len {}",
            out.len()
        );
    }

    #[test]
    fn avatar_line_injected_in_openclaw_mode() {
        let mut definition = mk_definition();
        definition.config.openclaw_mode = true;
        definition.config.avatar = Some("https://example.com/a.png".into());
        let budget = MemoryBudgetConfig::default();
        let out = build(
            &definition,
            Some("gpt-5.4"),
            Some("OpenAI"),
            &[],
            &budget,
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            SessionMode::Default,
            ExecutionMode::Off,
            crate::workflow_mode::WorkflowMode::Off,
        );
        assert!(
            out.contains("Your avatar image is at: https://example.com/a.png"),
            "openclaw-mode prompt should include avatar line: {out}"
        );
    }

    #[test]
    fn incognito_prompt_omits_memory_and_includes_policy() {
        let definition = mk_definition();
        let budget = MemoryBudgetConfig::default();
        let entries = vec![mk_entry(1, MemoryType::User, "Prefers concise responses")];

        let out = build(
            &definition,
            Some("gpt-5.4"),
            Some("OpenAI"),
            &entries,
            &budget,
            None,
            None,
            None,
            None,
            None,
            true,
            None,
            None,
            SessionMode::Default,
            ExecutionMode::Off,
            crate::workflow_mode::WorkflowMode::Off,
        );

        assert!(out.contains("# Incognito Session"));
        assert!(out.contains("Long-term memory tools are unavailable"));
        assert!(
            !out.contains("# Memory\n"),
            "incognito prompt should omit the memory section: {out}"
        );
        assert!(
            !out.contains("## Memory Guidelines"),
            "incognito prompt should omit memory guidelines: {out}"
        );
        assert!(
            !out.contains("save_memory:"),
            "incognito prompt should omit memory tool descriptions: {out}"
        );
        assert!(
            !out.contains("recall_memory:"),
            "incognito prompt should omit memory tool descriptions: {out}"
        );
    }

    #[test]
    fn prompt_render_debug_helpers_are_stable_and_bounded() {
        assert_eq!(short_fingerprint("abc").len(), 16);
        assert_eq!(short_fingerprint("abc"), short_fingerprint("abc"));
        assert_ne!(short_fingerprint("abc"), short_fingerprint("abcd"));

        let label = section_debug_label(3);
        assert_eq!(label, "section_3");
        assert!(!label.contains("Available Tools"));
    }
}
