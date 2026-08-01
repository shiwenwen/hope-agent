//! 技能系统的**契约 + 台账 + 纯谓词**层（阶段 5 第七刀后 kernel 留存部分）。
//!
//! 机器——内置技能解包（`embedded`）、SKILL.md 扫描与 YAML 解析（`discovery` /
//! `frontmatter`）、创作写盘（`author`）、自动复盘流水线（`auto_review`）、
//! `@skill` 提及、fork 派发、GUI / HTTP 命令面与 `skill` 工具——已迁入
//! [`ha-skills`](https://docs.rs/ha-skills) crate，经 [`crate::skills_hooks`] 反向回调。
//!
//! 留在这里的五个模块**都不是「技能行为」**：
//!
//! - [`types`]——`SkillEntry` / `SkillStatus` / `SkillSummary` 等 wire 契约，
//!   连同 `skill_cache_version` / `bump_skill_version` 这对目录版本计数器。
//!   slash 命令表（`slash_commands`）与 GUI / HTTP 命令面共用它们，是跨 crate
//!   的公共词汇表。
//! - [`activation`]——**台账**：`session_skill_activation` 表 + 进程内热缓存的
//!   真相源。三个 kernel 调用点读写它（`tools::execution` 写、
//!   `system_prompt::sections` 读、`session::cleanup_watcher` 清），
//!   `SessionDB` 也在 kernel，没有理由让它出去再钩回来。
//! - [`requirements`] / [`prompt`] / [`slash`]——对契约类型的**纯谓词与纯渲染**
//!   （环境依赖检查、prompt 段拼装、slash 名字归一 / 健康度）。不碰文件系统、
//!   不调 LLM、不出网，`slash_commands` 与 `system_prompt` 直接用。
//!
//! 这一层留下的直接后果：`tools::execution` 的条件技能激活块、
//! `session::cleanup_watcher` 的清理、`system_prompt` 的 `build_skills_prompt`
//! 与 `activated_skill_names` 全部**一行未改**，只有「取目录」那一步走钩子。

pub mod activation;
// 以下四个模块在迁出前是私有 `mod` + `pub use …::*` 再导出；本刀改 `pub mod`，
// 好让 ha-skills 以原名再导出（`ha_skills::skills::types::…`），并让迁出的
// 单元测试保持 `skills::slash::check_all_skills_status` 这类原路径。
pub mod prompt;
pub mod requirements;
pub mod slash;
pub mod types;

pub use activation::{
    activate_skills_for_paths, activated_skill_names, clear_session_activation,
    reset_activation_cache,
};
pub use prompt::*;
pub use requirements::*;
pub use slash::*;
pub use types::*;

// 类型已下沉 ha-config-schema，原地再导出保持 `crate::skills::SkillsConfig` 路径不变。
pub use ha_config_schema::skills::SkillsConfig;

/// Wrap SKILL.md content with runtime package metadata so bundled resources can
/// be used without guessing where the skill lives on disk.
///
/// 纯格式化，留 kernel 与 [`types::SkillEntry`] 同处——三个消费者
/// （`@skill` 提及 / fork / `skill` 工具内联）都在 ha-skills，但它自身
/// 只读契约字段。
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
