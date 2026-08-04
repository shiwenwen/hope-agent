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
    list_mentionable_skills, resolve_inline_skill_mentions, MentionableSkill, AT_MENTIONABLE_SKILLS,
};

// —— kernel 留存部分：原名再导出，保住迁出前的模块路径 ——
// glob 同时带上 `activation` / `prompt` / `requirements` / `slash` / `types`
// 五个子模块与它们的全部 `pub use`，故 `skills::types::SkillStatus`、
// `skills::activated_skill_names`、`skills::SkillsConfig` 等一处未变。
pub use ha_core::skills::*;
