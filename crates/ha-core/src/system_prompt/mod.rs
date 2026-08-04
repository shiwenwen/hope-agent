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
            packages.insert(crate::tool_defs::TOOL_SUBAGENT.to_string(), section);
        }
    }
    if definition.config.team.enabled {
        let section = sections::build_team_section();
        if !section.is_empty() {
            packages.insert(crate::tool_defs::TOOL_TEAM.to_string(), section);
        }
    }
    if definition.config.acp.enabled {
        let section = sections::build_acp_section();
        if !section.is_empty() {
            packages.insert(crate::tool_defs::TOOL_ACP_SPAWN.to_string(), section);
        }
    }

    packages
}

// ── 特征 crate 钩子：天气 prompt 段 ──────────────────────────────
//
// weather 已迁出为特征 crate（依赖方向 ha-weather → ha-core），kernel 构建
// system prompt 时经此钩子取天气段：未装配（未 wire）＝ None ＝ 无天气段，
// 与「特征不存在」语义一致，fail-soft。装配在 `ha_weather::wire()`。

static WEATHER_PROMPT_SOURCE: std::sync::OnceLock<fn() -> Option<String>> =
    std::sync::OnceLock::new();

/// 特征 crate 装配期注册天气 prompt 段来源。重复注册返回 `Err`（fail-loud，
/// 静默顶替＝来源不可预测）。
pub fn register_weather_prompt_source(
    source: fn() -> Option<String>,
) -> Result<(), crate::AlreadyRegistered> {
    WEATHER_PROMPT_SOURCE
        .set(source)
        .map_err(|_| crate::AlreadyRegistered("system_prompt weather source"))
}

/// 当前天气 prompt 段（未注册来源即 `None`）。
pub(crate) fn weather_prompt_text() -> Option<String> {
    WEATHER_PROMPT_SOURCE.get().and_then(|f| f())
}

// ── 特征 crate 钩子：ACP backend binary 可用性 ───────────────────
//
// acp 迁出为特征 crate 后，ACP prompt 段判断「相对路径 binary 是否可解析」
// 经此钩子（PATH / 注册表探测在 ha-acp）。未装配（未 wire）＝一律不可用
// ＝相对路径 backend 不进清单——绝对路径分支不受影响；backends 清单为空
// 时整段为空，与特征不存在语义一致。

static ACP_BINARY_RESOLVER: std::sync::OnceLock<fn(&str) -> bool> = std::sync::OnceLock::new();

/// 特征 crate 装配期注册 ACP binary 解析器。重复注册返回 `Err`。
pub fn register_acp_binary_resolver(
    resolver: fn(&str) -> bool,
) -> Result<(), crate::AlreadyRegistered> {
    ACP_BINARY_RESOLVER
        .set(resolver)
        .map_err(|_| crate::AlreadyRegistered("system_prompt acp binary resolver"))
}

pub(crate) fn acp_binary_resolvable(binary: &str) -> bool {
    ACP_BINARY_RESOLVER
        .get()
        .map(|f| f(binary))
        .unwrap_or(false)
}
