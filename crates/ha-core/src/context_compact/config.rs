// ── Configuration (user-configurable, stored in config.json) ──
//
// 类型已下沉 ha-config-schema，此处原地再导出保持既有路径不变。

pub use ha_config_schema::context_compact::CompactConfig;

#[cfg(test)]
mod tests {
    use super::*;
    use ha_config_schema::context_compact::default_tool_policies;

    #[test]
    fn clamp_caps_injected_context_share_to_history_share() {
        let mut cfg = CompactConfig {
            max_history_share: 0.30,
            max_compaction_injected_context_share: 0.80,
            ..Default::default()
        };

        cfg.clamp();

        assert_eq!(cfg.max_history_share, 0.30);
        assert_eq!(cfg.max_compaction_injected_context_share, 0.30);
    }

    /// schema 层的 `default_tool_policies` 不能依赖 ha-core，只能写字面量；
    /// 本测试把它锁死在 `tools/mod.rs` 的 `TOOL_*` 常量上——任一侧改名或
    /// 增删默认策略工具，这里立刻红。
    #[test]
    fn default_tool_policies_match_tool_name_constants() {
        use crate::tools::{
            TOOL_AGENTS_LIST, TOOL_FIND, TOOL_GET_WEATHER, TOOL_GREP, TOOL_LS, TOOL_MEMORY_GET,
            TOOL_PROCESS, TOOL_RECALL_MEMORY, TOOL_SESSIONS_LIST, TOOL_SESSION_STATUS,
            TOOL_TOOL_SEARCH, TOOL_WEB_FETCH, TOOL_WEB_SEARCH,
        };

        let policies = default_tool_policies();
        let eager = [
            TOOL_LS,
            TOOL_GREP,
            TOOL_FIND,
            TOOL_PROCESS,
            TOOL_SESSIONS_LIST,
            TOOL_AGENTS_LIST,
            TOOL_SESSION_STATUS,
            TOOL_GET_WEATHER,
            TOOL_TOOL_SEARCH,
        ];
        let protect = [
            TOOL_WEB_SEARCH,
            TOOL_WEB_FETCH,
            TOOL_RECALL_MEMORY,
            TOOL_MEMORY_GET,
        ];
        for name in eager {
            assert_eq!(
                policies.get(name).map(String::as_str),
                Some("eager"),
                "tool `{name}` should default to eager"
            );
        }
        for name in protect {
            assert_eq!(
                policies.get(name).map(String::as_str),
                Some("protect"),
                "tool `{name}` should default to protect"
            );
        }
        assert_eq!(
            policies.len(),
            eager.len() + protect.len(),
            "schema default_tool_policies has entries not covered by TOOL_* constants"
        );
    }
}
