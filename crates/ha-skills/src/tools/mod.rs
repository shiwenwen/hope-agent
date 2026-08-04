//! 技能的 agent 工具面（阶段 5 第七刀，自 `ha_core::tools::skill` 迁出）。
//!
//! 只有一个 handler：`skill`（模型激活技能的首选入口，`context: fork` 派子
//! agent、否则内联 SKILL.md）。名字常量 `TOOL_SKILL` 与 `ToolDefinition`
//! schema **留 kernel**（`tool_defs::names` / `tools::definitions::core_tools`）
//! ——纯契约、不含任何 skills 类型，同第六刀 `note_*` 那批的处理。

pub mod skill;

use ha_core::tools::registry::BuiltinToolEntry;

/// `skill` 工具的分发条目。与迁出前 `tools/builtin_registry.rs` 里那一条逐字一致。
pub fn skill_dispatch_entries() -> Vec<BuiltinToolEntry> {
    use ha_core::tools::registry::tool_handler;
    vec![BuiltinToolEntry {
        name: ha_core::tools::TOOL_SKILL,
        aliases: &[],
        handler: tool_handler!(|args, ctx| skill::tool_skill(args, ctx).await),
    }]
}

#[cfg(test)]
mod tests {
    /// 迁出前 `TOOL_SKILL` 的 handler 在 kernel 的 `builtin_entries()` 里，由
    /// `every_builtin_canonical_name_has_a_definition` 强制「每个可调用名都有
    /// 一份 `ToolDefinition`」。handler 上浮后 kernel 那条断言不再覆盖它，
    /// 这里按同一强度接回——漏 definition 会让 `dispatch::resolve_tool_fate`
    /// 的 `tools.allow/deny` 兜底对该工具 no-op（能执行、不受约束），而
    /// `registry_freeze` 只在运行期记 warn，CI 不会红。
    #[test]
    fn skill_tool_has_a_definition() {
        let defined: std::collections::HashSet<&str> =
            ha_core::tools::dispatch::all_dispatchable_tools()
                .iter()
                .map(|d| d.name.as_str())
                .collect();
        let missing: Vec<&str> = super::skill_dispatch_entries()
            .iter()
            .map(|e| e.name)
            .filter(|name| !defined.contains(*name))
            .collect();
        assert!(
            missing.is_empty(),
            "skill tool lacking a ToolDefinition (resolve_tool_fate allow/deny fallback is a no-op for it): {missing:?}"
        );
    }
}
