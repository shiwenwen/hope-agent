use super::constants::{
    HUMAN_IN_THE_LOOP_GUIDANCE, MAX_FILE_CHARS, SOUL_EMBODIMENT_GUIDANCE,
    TOOL_CALL_NARRATION_GUIDANCE,
};
use super::helpers::truncate;
use super::sections::*;
use crate::agent_config::AgentDefinition;
use crate::project::{Project, ProjectFile};
use crate::skills;
use crate::user_config;

/// Default inline budget for small project-file contents, in bytes.
/// Kept conservative so the "Project Files" section never blows up the
/// prompt size on its own.
const DEFAULT_PROJECT_FILES_INLINE_BUDGET: usize = 8 * 1024;

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
/// ⑥b Deferred tools listing (conditional)
/// ⑥c Tool-call narration guidance (hardcoded, always injected)
/// ⑥d Human-in-the-loop guidance (conditional, hardcoded)
/// ⑦ Skills — available skill descriptions (filtered)
/// ⑧ Memory — injected from memory backend
/// ⑨ Runtime info — date, OS, etc.
/// ⑩ Sub-agent delegation (conditional)
/// ⑪ Sandbox mode (conditional)
/// ⑦b Current Project (conditional — when session belongs to a project)
/// ⑦c Project Files catalog + small-file inlining (conditional)
/// ⑬ ACP external agents (conditional)
pub fn build(
    definition: &AgentDefinition,
    model: Option<&str>,
    provider: Option<&str>,
    memory_context: Option<&str>,
    agent_home: Option<&str>,
    project: Option<&Project>,
    project_files: &[ProjectFile],
    session_id: Option<&str>,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    if definition.config.openclaw_mode {
        // ── 4-file markdown prompt mode (AGENTS.md, SOUL.md, IDENTITY.md, TOOLS.md) ──

        // Minimal identity line
        sections.push(format!(
            "You are {}, running in OpenComputer on {} {}.",
            definition.config.name, os, arch
        ));

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
    } else if definition.config.use_custom_prompt {
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
            "You are {}{}, running in OpenComputer on {} {}.",
            definition.config.name, role_suffix, os, arch
        ));

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

    // ⑥ Tool definitions (filtered by agent config)
    sections.push(build_tools_section(&definition.config.capabilities.tools));

    // ⑥b Deferred tools listing (when deferred loading is enabled)
    if let Some(deferred_section) = build_deferred_tools_section() {
        sections.push(deferred_section);
    }

    // ⑥b² Async tool execution guide (when the feature is enabled)
    if let Some(async_section) = build_async_tools_section() {
        sections.push(async_section);
    }

    // ⑥c Tool-call narration guidance — always injected so the model previews
    // each tool call with one short natural-language sentence (mirrors Claude
    // Code's "Before your first tool call, briefly state what you're about to
    // do"). Hardcoded constant so custom agent.md cannot drop it.
    sections.push(TOOL_CALL_NARRATION_GUIDANCE.to_string());

    // ⑥d Human-in-the-loop guidance — hardcoded so it cannot be overridden by
    // a user-customized agent.md. Only emitted when the agent has access to
    // the `ask_user_question` tool (agents with no interactive surface skip it).
    if crate::tools::agent_tool_filter_allows(
        crate::tools::TOOL_ASK_USER_QUESTION,
        &definition.config.capabilities.tools,
    ) {
        sections.push(HUMAN_IN_THE_LOOP_GUIDANCE.to_string());
    }

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

            // ⑦c Project Files — catalog + inlined small files
            if !project_files.is_empty() {
                let files_section = build_project_files_section(
                    &proj.id,
                    project_files,
                    DEFAULT_PROJECT_FILES_INLINE_BUDGET,
                );
                if !files_section.is_empty() {
                    sections.push(files_section);
                }
            }
        }
    }

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

    // ⑩½ Agent Team (conditionally injected)
    if definition.config.team.enabled {
        let team_section = build_team_section();
        if !team_section.is_empty() {
            sections.push(team_section);
        }
    }

    // ⑪ Sandbox mode (conditionally injected)
    if definition.config.capabilities.sandbox {
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

/// Build a system prompt using the legacy path (no AgentDefinition).
/// This preserves backward compatibility during the transition.
pub fn build_legacy(model: Option<&str>, provider: Option<&str>) -> String {
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
    sections.push(build_all_tools_description());

    // Deferred tools listing
    if let Some(deferred_section) = build_deferred_tools_section() {
        sections.push(deferred_section);
    }

    // Async tool execution guide
    if let Some(async_section) = build_async_tools_section() {
        sections.push(async_section);
    }

    // Tool-call narration guidance (see build() for rationale)
    sections.push(TOOL_CALL_NARRATION_GUIDANCE.to_string());

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

    sections.join("\n\n")
}
