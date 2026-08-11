//! 技能机器层（阶段 5 第七刀，自 ha-core 迁出）。
//!
//! 契约（`types`）、台账（`activation`）与纯谓词 / 纯渲染（`requirements` /
//! `prompt` / `slash`）**留在 [`ha_core::skills`]**，本模块下方原名再导出，
//! crate 外的 `skills::SkillEntry` / `skills::build_skills_prompt` 等路径逐字
//! 不变。分法论据见 [`ha_core::skills`] 与 [`ha_core::skills_hooks`] 的模块文档。

pub mod author;
pub mod auto_review;
pub mod commands;
mod discovery;
mod embedded;
pub mod fork_helper;
mod frontmatter;
pub mod mention;

#[cfg(test)]
mod tests;

pub use commands::{PresetCandidate, PresetSkillSource};
pub use discovery::*;
pub use fork_helper::{extract_fork_result, spawn_skill_fork, MAX_RESULT_CHARS};
pub use mention::{
    list_mentionable_skills, resolve_named_skill_mentions, MentionableSkill, AT_MENTIONABLE_SKILLS,
};

/// Verify that the bytes just read for an activation still carry the same
/// control metadata as the catalog entry that selected the command/mention.
/// The body may change freely, but a concurrent frontmatter edit must not pair
/// new instructions with a stale (and potentially wider) tool ceiling or stale
/// requirements snapshot.
pub(crate) fn validate_materialized_skill_snapshot(
    entry: &SkillEntry,
    content: &str,
) -> anyhow::Result<()> {
    let parsed = frontmatter::parse_frontmatter(content)
        .ok_or_else(|| anyhow::anyhow!("materialized SKILL.md has invalid frontmatter"))?;
    let control_metadata_matches = parsed.name == entry.name
        && parsed.aliases == entry.aliases
        && parsed.requires == entry.requires
        && parsed.skill_key == entry.skill_key
        && parsed.user_invocable == entry.user_invocable
        && parsed.disable_model_invocation == entry.disable_model_invocation
        && parsed.command_dispatch == entry.command_dispatch
        && parsed.command_tool == entry.command_tool
        && parsed.command_arg_mode == entry.command_arg_mode
        && parsed.command_arg_placeholder == entry.command_arg_placeholder
        && parsed.command_arg_options == entry.command_arg_options
        && parsed.command_prompt_template == entry.command_prompt_template
        && parsed.allowed_tools_declared == entry.allowed_tools_declared
        && parsed.allowed_tools == entry.allowed_tools
        && parsed.context_mode == entry.context_mode
        && parsed.agent == entry.agent
        && parsed.effort == entry.effort
        && parsed.paths == entry.paths
        && parsed.status == entry.status;
    if !control_metadata_matches {
        anyhow::bail!(
            "SKILL.md activation metadata changed during materialization; retry the activation"
        );
    }
    Ok(())
}

// —— kernel 留存部分：原名再导出，保住迁出前的模块路径 ——
// glob 同时带上 `activation` / `prompt` / `requirements` / `slash` / `types`
// 五个子模块与它们的全部 `pub use`，故 `skills::types::SkillStatus`、
// `skills::activated_skill_names`、`skills::SkillsConfig` 等一处未变。
pub use ha_core::skills::*;
