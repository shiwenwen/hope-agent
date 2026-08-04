//! 知识空间的 agent 工具面（阶段 5 第六刀，自 `ha_core::tools::note` 迁出）。
//!
//! 24 个 handler：22 个 `note_*` + `knowledge_recall` + `session_to_note`。
//! 名字常量与 `ToolDefinition` schema **留 kernel**（`tool_defs::names` /
//! `tools::definitions::core_tools`）——它们是纯契约，不含任何 knowledge 类型，
//! 且 `is_kb_scoped_tool` / `ToolScope::Knowledge` 的收窄逻辑本就在 kernel。
//! 因此这里只需注册分发条目，不需要 `register_external_tool_definitions`。

pub mod note;

use ha_core::tools::registry::BuiltinToolEntry;

/// 24 个知识空间工具的分发条目。顺序与迁出前 `tools/builtin_registry.rs`
/// 里那段逐行一致。
pub fn note_dispatch_entries() -> Vec<BuiltinToolEntry> {
    use ha_core::tools::registry::tool_handler;
    vec![
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_CREATE,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_create(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_READ,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_read(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_UPDATE,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_update(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_PATCH,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_patch(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_APPEND,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_append(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_DELETE,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_delete(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_SEARCH,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_search(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_LINK,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_link(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_BACKLINKS,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_backlinks(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_BY_TAG,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_by_tag(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_TAGS,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_tags(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_RENAME,
            aliases: &[ha_core::tools::TOOL_NOTE_MOVE],
            handler: tool_handler!(|args, ctx| note::tool_note_rename(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_SET_FRONTMATTER,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_set_frontmatter(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_ASSIGN_BLOCK,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_assign_block(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_BROKEN_LINKS,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_broken_links(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_ORPHANS,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_orphans(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_GRAPH,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_graph(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_SIMILAR,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_similar(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_RELATED,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_related(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_SUGGEST_LINKS,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_suggest_links(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_DISTILL,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_distill(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_NOTE_MOC,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_note_moc(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_KNOWLEDGE_RECALL,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_knowledge_recall(args, ctx).await),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_SESSION_TO_NOTE,
            aliases: &[],
            handler: tool_handler!(|args, ctx| note::tool_session_to_note(args, ctx).await),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::note_dispatch_entries;

    /// 外部注册的别名也必须归一到规范名——执行门的 fate / deny / allowlist
    /// 判定都走这条链路，漏一个别名 = 该别名滑过 `resolve_tool_fate` 兜底。
    /// 迁出前这条断言在 kernel `tools/registry.rs` 的
    /// `aliases_resolve_to_canonical_names` 里（`note_move` 那一项）；handler
    /// 随本 crate 上浮后，别名只在 `wire()` 之后才进注册表，故断言随之下移。
    ///
    /// `canonical_name_for_test` 会冻结注册表——本 crate 若将来加了别的测试在
    /// `wire()` 之前触发工具 lookup，`register_external_tools` 会返 `Err` 并被
    /// `wire()` 里的 `.expect` 打成 panic。届时改用串行化守卫，别放任它按测试
    /// 执行序碰运气。
    /// **规范名侧的强制门禁**：迁出前这条契约由 kernel 的
    /// `every_builtin_canonical_name_has_a_definition` 守着（它只遍历
    /// `builtin_entries()`，24 个 handler 移出后就不再覆盖它们）。`freeze_now`
    /// 的运行期 warn 不会让 CI 红，而漏 `ToolDefinition` 会让
    /// `resolve_tool_fate` 的 `tools.allow/deny` 可见性兜底对该工具 no-op——
    /// 工具照样能执行，却不受任何 allow/deny 约束。故在此按同一强度补回。
    ///
    /// 只查**规范名**：历史别名本就无独立 definition，随规范名归一（与 kernel
    /// 那条测试同一口径）。别名的归一链路由下面那条测试守。
    #[test]
    fn every_note_tool_has_a_definition() {
        let defined: std::collections::HashSet<&str> =
            ha_core::tools::dispatch::all_dispatchable_tools()
                .iter()
                .map(|d| d.name.as_str())
                .collect();
        let missing: Vec<&str> = note_dispatch_entries()
            .iter()
            .map(|e| e.name)
            .filter(|name| !defined.contains(*name))
            .collect();
        assert!(
            missing.is_empty(),
            "note tools lacking a ToolDefinition (resolve_tool_fate allow/deny fallback is a no-op for them): {missing:?}"
        );
    }

    #[test]
    fn note_alias_resolves_to_canonical_name() {
        crate::wire();
        assert_eq!(
            ha_core::tools::registry::canonical_name_for_test(ha_core::tools::TOOL_NOTE_MOVE),
            Some(ha_core::tools::TOOL_NOTE_RENAME),
        );
    }
}
