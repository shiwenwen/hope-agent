// ── System Prompt Breakdown ──────────────────────────────────────
//
// Instrumented helper for `/context` — computes per-section char counts
// of the built system prompt without changing `build()` output.
//
// Memory section is measured via a diff trick (full − without-memory)
// because the memory section's exact boundaries are trickier to isolate
// than skills/tools, which have dedicated section builders we can call
// directly.

use super::build::build;
use super::sections::{build_skills_section, build_tools_section};
use crate::agent_config::AgentDefinition;

/// Per-section char counts for a built system prompt.
/// All values are character counts, not tokens — the caller
/// applies the `char / 4` heuristic.
#[derive(Debug, Clone)]
pub struct SystemPromptBreakdown {
    /// Total chars of the full system prompt (`build()` output).
    pub total_chars: usize,
    /// Chars attributable to the memory section (core + SQLite + guidelines).
    /// Computed via diff: `full - build_without_memory`.
    pub memory_chars: usize,
    /// Chars of the skills section (`# Available Skills ...`).
    pub skills_chars: usize,
    /// Chars of the tool-descriptions section (`# Available Tools ...`).
    /// This is the per-tool prose inside the system prompt, NOT the JSON
    /// tool schemas sent in the API request `tools:` array.
    pub tool_descriptions_chars: usize,
}

/// Compute a per-section breakdown of the system prompt.
///
/// The `memory_context` argument should match what will actually be
/// injected at chat time (from `memory_backend.build_prompt_summary`).
pub fn compute_breakdown(
    definition: &AgentDefinition,
    model: Option<&str>,
    provider: Option<&str>,
    memory_context: Option<&str>,
    agent_home: Option<&str>,
) -> SystemPromptBreakdown {
    // Breakdown is not project-aware (it's used by the /context dashboard,
    // which measures prompt size outside the chat loop). Pass empty project
    // context so the output matches the non-project case.
    let full = build(
        definition,
        model,
        provider,
        memory_context,
        agent_home,
        None,
        &[],
    );
    let without_memory = build(definition, model, provider, None, agent_home, None, &[]);
    let memory_chars = full.len().saturating_sub(without_memory.len());

    let skills_chars = build_skills_section(
        &definition.config.capabilities.skills,
        definition.config.capabilities.skill_env_check,
    )
    .len();

    let tool_descriptions_chars =
        build_tools_section(&definition.config.capabilities.tools).len();

    SystemPromptBreakdown {
        total_chars: full.len(),
        memory_chars,
        skills_chars,
        tool_descriptions_chars,
    }
}
