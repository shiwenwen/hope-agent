pub mod activation;
pub mod author;
pub mod auto_review;
pub mod commands;
mod discovery;
mod embedded;
pub mod fork_helper;
mod frontmatter;
pub mod mention;
mod prompt;
mod requirements;
mod slash;
mod types;

#[cfg(test)]
mod tests;

pub use activation::{
    activate_skills_for_paths, activated_skill_names, clear_session_activation,
    reset_activation_cache,
};
pub use commands::{PresetCandidate, PresetSkillSource};
pub use discovery::*;
pub use fork_helper::{extract_fork_result, spawn_skill_fork, MAX_RESULT_CHARS};
pub use mention::{
    list_mentionable_skills, resolve_inline_skill_mentions, MentionableSkill, AT_MENTIONABLE_SKILLS,
};
pub use prompt::*;
pub use requirements::*;
pub use slash::*;
pub use types::*;

// 类型已下沉 ha-config-schema，原地再导出保持 `crate::skills::SkillsConfig` 路径不变。
pub use ha_config_schema::skills::SkillsConfig;

/// Wrap SKILL.md content with runtime package metadata so bundled resources can
/// be used without guessing where the skill lives on disk.
pub fn build_skill_context_payload(skill: &types::SkillEntry, content: &str) -> String {
    format!(
        "[SYSTEM: Skill package metadata]\n\
         - Skill name: `{}`\n\
         - Skill directory: `{}`\n\
         - Resolve bundled scripts, references, and assets relative to that directory.\n\
         [/SYSTEM]\n\n{}",
        skill.name, skill.base_dir, content
    )
}
