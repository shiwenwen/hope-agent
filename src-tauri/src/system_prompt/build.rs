use crate::agent_config::AgentDefinition;
use crate::skills;
use crate::user_config;
use super::constants::MAX_FILE_CHARS;
use super::helpers::truncate;
use super::sections::*;

// ── Build System Prompt ──────────────────────────────────────────

/// Build the complete system prompt from an AgentDefinition.
///
/// Assembly order (13 sections):
/// ① Identity line
/// ② agent.md — what this agent does
/// ③ persona.md — personality
/// ④ User context — from user.json
/// ⑤ tools.md — custom tool guidance
/// ⑥ Tool definitions — per-tool descriptions (filtered by agent config)
/// ⑦ Skills — available skill descriptions (filtered)
/// ⑦b Behavior guidance — output efficiency, action safety, task execution
/// ⑧ Memory — injected from memory backend
/// ⑨ Runtime info — date, OS, etc.
/// ⑩ Sub-agent delegation (conditional)
/// ⑪ Sandbox mode (conditional)
/// ⑫ (reserved for project context — not yet implemented)
/// ⑬ ACP external agents (conditional)
pub fn build(
    definition: &AgentDefinition,
    model: Option<&str>,
    provider: Option<&str>,
    memory_context: Option<&str>,
    agent_home: Option<&str>,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    if definition.config.use_custom_prompt {
        // ── Custom prompt mode: use markdown files directly, skip structured config ──

        // Minimal identity line
        sections.push(format!(
            "You are {}, running in OpenComputer on {} {}.",
            definition.config.name, os, arch
        ));

        // agent.md — custom identity / instructions
        if let Some(md) = definition
            .agent_md
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            sections.push(truncate(md, MAX_FILE_CHARS));
        }

        // persona.md — custom personality
        if let Some(persona) = definition
            .persona
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            sections.push(truncate(persona, MAX_FILE_CHARS));
        }
    } else {
        // ── Structured mode: assemble from config fields + optional supplements ──

        // ① Identity
        let role_suffix = definition
            .config
            .personality
            .role
            .as_deref()
            .filter(|r| !r.is_empty())
            .map(|r| format!(", a {}", r))
            .unwrap_or_default();
        sections.push(format!(
            "You are {}{}, running in OpenComputer on {} {}.",
            definition.config.name, role_suffix, os, arch
        ));

        // ② Personality (structured)
        let personality_section = build_personality_section(&definition.config.personality);
        if !personality_section.is_empty() {
            sections.push(personality_section);
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

    // ⑤ tools.md
    if let Some(guide) = &definition.tools_guide {
        sections.push(truncate(guide, MAX_FILE_CHARS));
    }

    // ⑥ Tool definitions (filtered by agent config)
    sections.push(build_tools_section(&definition.config.tools));

    // ⑥b Deferred tools listing (when deferred loading is enabled)
    if let Some(deferred_section) = build_deferred_tools_section() {
        sections.push(deferred_section);
    }

    // ⑦ Skills (filtered by agent config)
    sections.push(build_skills_section(
        &definition.config.skills,
        definition.config.behavior.skill_env_check,
    ));

    // ⑦b Behavior guidance (output efficiency, action safety, task execution)
    sections.push(build_behavior_section());

    // ⑧ Memory
    if definition.config.memory.enabled {
        let mut memory_section = String::new();

        // 8a: Global Core Memory (shared across all agents)
        if let Some(md) = &definition.global_memory_md {
            if !md.trim().is_empty() {
                memory_section.push_str("## Core Memory (Global)\n\n");
                memory_section.push_str(&truncate(md, MAX_FILE_CHARS));
                memory_section.push_str("\n\n");
            }
        }

        // 8b: Agent Core Memory (specific to this agent)
        if let Some(md) = &definition.memory_md {
            if !md.trim().is_empty() {
                memory_section.push_str("## Core Memory (Agent)\n\n");
                memory_section.push_str(&truncate(md, MAX_FILE_CHARS));
                memory_section.push_str("\n\n");
            }
        }

        // 8c: SQLite memories (existing logic)
        if let Some(mem) = memory_context {
            if !mem.is_empty() {
                memory_section.push_str(mem);
                memory_section.push_str("\n\n");
            }
        }

        // 8d: Memory usage guidance
        memory_section.push_str(
            "## Memory Guidelines\n\
             Use update_core_memory when:\n\
             - The user gives a standing instruction (\"always\", \"never\", \"from now on\", \"remember to\")\n\
             - The user states a persistent preference or rule\n\
             - The user corrects a recurring behavior\n\n\
             Use save_memory when:\n\
             - You learn a fact about the user, project, or external resource\n\
             - The user mentions a deadline, event, or temporary context\n\
             - You discover something worth noting for future reference\n\n\
             Use recall_memory when:\n\
             - You need context about the user or project from prior conversations\n\
             - The user references something discussed before\n\
             - You want to check if preferences or constraints were previously established\n\n\
             Use recall_memory with include_history=true when:\n\
             - The user references a previous conversation (\"last time\", \"we discussed\", \"remember when\")\n\
             - You need to find what was said or decided in an earlier session\n\n\
             Do NOT save: ephemeral task details, code snippets, debugging steps, or anything derivable from the codebase."
        );

        if !memory_section.is_empty() {
            sections.push(memory_section);
        }
    }

    // ⑨ Runtime info
    sections.push(build_runtime_section(model, provider, agent_home));

    // ⑩ Sub-agent delegation (conditionally injected)
    if definition.config.subagents.enabled {
        let subagent_section =
            build_subagent_section(&definition.config.subagents, &definition.id, 0);
        if !subagent_section.is_empty() {
            sections.push(subagent_section);
        }
    }

    // ⑪ Sandbox mode (conditionally injected)
    if definition.config.behavior.sandbox {
        sections.push(
            "# Sandbox Mode\n\n\
             All commands you execute via the `exec` tool will automatically run inside a Docker sandbox container.\n\
             You do NOT need to pass `sandbox=true` — it is enforced by your agent configuration.\n\n\
             Sandbox properties:\n\
             - Read-only root filesystem (only /workspace, /tmp, /var/tmp, /run are writable)\n\
             - No network access (network mode: none)\n\
             - All Linux capabilities dropped\n\
             - Process count limited\n\
             - Your working directory is mounted at /workspace inside the container\n\n\
             If a command needs to write temporary files, use /tmp. \
             If a command requires network access or special privileges, inform the user that it cannot run in sandbox mode."
                .to_string(),
        );
    }

    // ⑬ ACP external agent delegation (conditionally injected)
    if definition.config.acp.enabled {
        let acp_section = build_acp_section();
        if !acp_section.is_empty() {
            sections.push(acp_section);
        }
    }

    // ⑫ Project context — not yet implemented

    // ⑭ Weather context (from cached weather data)
    if let Some(weather_text) = crate::weather::get_weather_for_prompt() {
        sections.push(weather_text);
    }

    // Join all non-empty sections
    let section_lengths: Vec<usize> = sections.iter().map(|s| s.len()).collect();
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
                    "section_count": section_lengths.len(),
                    "section_lengths": section_lengths,
                    "agent_name": &definition.config.name,
                    "custom_prompt_mode": definition.config.use_custom_prompt,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    prompt
}

/// Build a system prompt using the legacy path (no AgentDefinition).
/// This preserves backward compatibility during the transition.
pub fn build_legacy(model: Option<&str>, provider: Option<&str>) -> String {
    let store = crate::provider::load_store().unwrap_or_default();
    let available_skills =
        skills::load_all_skills_with_budget(&store.extra_skills_dirs, &store.skill_prompt_budget);
    let skills_section = skills::build_skills_prompt(
        &available_skills,
        &store.disabled_skills,
        store.skill_env_check,
        &store.skill_env,
        &store.skill_prompt_budget,
        &store.skill_allow_bundled,
    );

    let mut sections = Vec::new();

    // Identity
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    sections.push(format!(
        "You are OpenComputer, a personal AI assistant with deep system integration. \
         You help users interact with their computer naturally and efficiently. \
         Running on {} {}.",
        os, arch
    ));

    // User context
    if let Ok(user_cfg) = user_config::load_user_config() {
        if let Some(user_section) = user_config::build_user_context(&user_cfg) {
            sections.push(user_section);
        }
    }

    // Tools
    sections.push(build_all_tools_description());

    // Deferred tools listing
    if let Some(deferred_section) = build_deferred_tools_section() {
        sections.push(deferred_section);
    }

    // Skills
    if !skills_section.is_empty() {
        sections.push(skills_section);
    }

    // Behavior guidance
    sections.push(build_behavior_section());

    // Weather context
    if let Some(weather_text) = crate::weather::get_weather_for_prompt() {
        sections.push(weather_text);
    }

    // Runtime (legacy mode has no agent home)
    sections.push(build_runtime_section(model, provider, None));

    sections.join("\n\n")
}
