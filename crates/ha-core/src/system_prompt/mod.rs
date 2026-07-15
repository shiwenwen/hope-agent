mod breakdown;
mod build;
mod constants;
mod helpers;
mod sections;
mod working_dir_instructions;

pub use breakdown::{compute_breakdown, SystemPromptBreakdown};
pub use build::{build, build_legacy};
pub(crate) use build::{
    build_with_resolved_session, conservative_core_token_estimate,
    render_core_memory_v2_for_context, rendered_core_memory_bodies, rendered_pinned_memory_sources,
    sqlite_memory_budget_after_static_layers,
};
pub use sections::build_subagent_section_with_depth;

/// Build dynamic guidance for deferred tools that became callable during the
/// session. This suffix is intentionally outside the stable prompt prefix: it
/// appears only after activation and disappears if the live inventory revokes
/// the tool. Eager tools keep using the static sections assembled by `build`.
pub(crate) fn build_tool_activation_guidance_packages(
    agent_id: &str,
    subagent_depth: u32,
) -> std::collections::HashMap<String, String> {
    let Ok(definition) = crate::agent_loader::load_agent(agent_id) else {
        return std::collections::HashMap::new();
    };
    let mut packages = std::collections::HashMap::new();

    if crate::tools::subagent::subagent_capability_enabled(&definition.id, &definition.config) {
        let section = sections::build_subagent_section(
            &definition.config.subagents,
            &definition.id,
            subagent_depth,
        );
        if !section.is_empty() {
            packages.insert(crate::tools::TOOL_SUBAGENT.to_string(), section);
        }
    }
    if definition.config.team.enabled {
        let section = sections::build_team_section();
        if !section.is_empty() {
            packages.insert(crate::tools::TOOL_TEAM.to_string(), section);
        }
    }
    if definition.config.acp.enabled {
        let section = sections::build_acp_section();
        if !section.is_empty() {
            packages.insert(crate::tools::TOOL_ACP_SPAWN.to_string(), section);
        }
    }

    packages
}
