//! MCP 客户端的 kernel 面：wire 类型再导出、命名约定、hook trampoline。
//! 运行时（McpManager / client / transport / oauth / watchdog / 两工具）
//! 在特征 crate `ha-mcp`，经 [`crate::mcp_hooks`] 装配期注册。
//!
//! kernel 调用点（tools 分发 / agent 组包 / settings 热更 / app_init 起
//! 动面）路径不变：`crate::mcp::catalog::is_mcp_tool_name`、
//! `crate::mcp::invoke::call_tool`、`crate::mcp::McpServerConfig` 等照旧
//! 解析。未接线语义逐项见 `mcp_hooks` 模块 doc（镜像 manager-None 的既
//! 有行为）。

pub mod catalog;
pub mod config;
pub mod invoke;

pub use config::{
    McpGlobalSettings, McpOAuthConfig, McpServerConfig, McpTransportSpec, McpTrustLevel,
};

use std::sync::Arc;
use std::sync::OnceLock;

use crate::tool_defs::ToolDefinition;

/// Preserve snapshot identity when ha-mcp is not wired. Callers use Arc
/// identity as the catalog generation token; allocating a fresh empty Arc on
/// every read would look like a catalog refresh after every tool call.
static EMPTY_TOOL_DEFINITIONS: OnceLock<Arc<Vec<ToolDefinition>>> = OnceLock::new();

/// settings 热更（`mcp_global` 类目）：从 config cache 重协调（含冷启
/// 用）。未接线 `Ok(())` + `app_warn` 审计——设置写路径不因特征缺席硬
/// 失败，MCP 面等价于「未启用」。
pub(crate) async fn reconcile_from_config_cache() -> anyhow::Result<()> {
    match crate::mcp_hooks::mcp_hooks() {
        Some(hooks) => (hooks.reconcile_from_config)().await,
        None => {
            crate::app_warn!(
                "mcp",
                "reconcile_skipped",
                "ha-mcp not wired; skipping MCP reconcile from config cache"
            );
            Ok(())
        }
    }
}

/// 动态 MCP 工具定义快照（agent 组包 / definitions / tool_search 消费）。
/// 未接线返空 Vec——与 manager 未初始化一致，MCP 工具整体缺席。
pub fn tool_definitions() -> Arc<Vec<ToolDefinition>> {
    match crate::mcp_hooks::mcp_hooks() {
        Some(hooks) => (hooks.tool_definitions)(),
        None => EMPTY_TOOL_DEFINITIONS
            .get_or_init(|| Arc::new(Vec::new()))
            .clone(),
    }
}

/// Lazy MCP catalog 自举。`tool_search` 在读取动态目录前调用；指定
/// `server_name` 时只连接该服务，未指定时保持全量发现。未接线 no-op，
/// 等价于 manager 不存在且动态目录为空。
pub(crate) async fn ensure_tool_catalogs(server_name: Option<&str>) {
    if let Some(hooks) = crate::mcp_hooks::mcp_hooks() {
        (hooks.ensure_tool_catalogs)(false, server_name.map(ToString::to_string)).await;
    }
}

/// Await the one-shot startup contract of MCP servers configured with
/// `eager=true`. A failed startup attempt still completes the feature-side
/// barrier, so later chat turns never become synchronous reconnect loops.
#[doc(hidden)]
pub async fn ensure_initial_eager_tool_catalogs() {
    if let Some(hooks) = crate::mcp_hooks::mcp_hooks() {
        (hooks.ensure_tool_catalogs)(true, None).await;
    }
}

/// 有效 MCP server 中是否仍有未完成首轮目录发现的实例。未接线时
/// MCP 整体缺席，因此返回 false。
#[doc(hidden)]
pub fn has_pending_catalogs() -> bool {
    crate::mcp_hooks::mcp_hooks().is_some_and(|hooks| (hooks.has_pending_catalogs)())
}

/// 将历史 namespaced 工具名归一到当前 catalog 名称。未接线或未知名称
/// 保持 `None`，由调用方继续使用原值。
pub(crate) fn canonical_tool_name(name: &str) -> Option<String> {
    let hooks = crate::mcp_hooks::mcp_hooks()?;
    (hooks.canonical_tool_name)(name)
}

/// Resolve a dynamic MCP tool through the atomically published catalog rather
/// than inferring ownership from an ambiguous namespaced string.
pub(crate) fn tool_server_name(name: &str) -> Option<String> {
    let hooks = crate::mcp_hooks::mcp_hooks()?;
    (hooks.tool_server_name)(name)
}

/// Normalize persisted Agent/Skill/Plan tool filters against the live MCP
/// catalog. This runs at schema/context construction time rather than config
/// load time because lazy servers do not publish their legacy alias map until
/// discovery completes.
pub(crate) fn canonicalize_tool_filter_names(names: &[String]) -> Vec<String> {
    canonicalize_tool_filter_names_with(names, canonical_tool_name)
}

fn canonicalize_tool_filter_names_with(
    names: &[String],
    mut resolve: impl FnMut(&str) -> Option<String>,
) -> Vec<String> {
    names
        .iter()
        .map(|name| resolve(name).unwrap_or_else(|| name.clone()))
        .collect()
}

/// Defense-in-depth matcher for execution contexts that were constructed by
/// a non-Agent caller and may still contain historical MCP identifiers.
pub(crate) fn tool_filter_contains(names: &[String], canonical_name: &str) -> bool {
    tool_filter_contains_with(names, canonical_name, canonical_tool_name)
}

fn tool_filter_contains_with(
    names: &[String],
    canonical_name: &str,
    mut resolve: impl FnMut(&str) -> Option<String>,
) -> bool {
    names
        .iter()
        .any(|name| name == canonical_name || resolve(name).as_deref() == Some(canonical_name))
}

/// namespaced 工具名 → 所属 server 的当前配置克隆（execution 的
/// auto-approve 门消费）。未接线 `None` → auto-approve 恒 false。
pub async fn tool_server_config(name: &str) -> Option<McpServerConfig> {
    let hooks = crate::mcp_hooks::mcp_hooks()?;
    (hooks.tool_server_config)(name).await
}

/// MCP auto-approve 的信任谓词（**安全语义，留 kernel**）：仅
/// `auto_approve=true` 且 `trust_level=Trusted` 才跳常规审批。
/// `validate_server_config`（ha-mcp）在保存期拒绝 Untrusted+auto_approve
/// 组合，本谓词是执行层的第二道防线。
pub fn server_auto_approves_config(cfg: &McpServerConfig) -> bool {
    cfg.auto_approve && matches!(cfg.trust_level, McpTrustLevel::Trusted)
}

/// app_init 起动面：读 cached_config 并幂等 init_global。返回「MCP 已
/// 启用且完成 init」（调用方据此决定是否再起 Primary watchdog）。
pub(crate) fn init_subsystem() -> bool {
    match crate::mcp_hooks::mcp_hooks() {
        Some(hooks) => (hooks.init_subsystem)(),
        None => {
            app_info!(
                "mcp",
                "init",
                "ha-mcp not wired in this process; MCP subsystem unavailable"
            );
            false
        }
    }
}

/// Primary-only 长驻 watchdog（调用方守 tier 门）。未接线 no-op。
pub(crate) fn spawn_watchdog() {
    if let Some(hooks) = crate::mcp_hooks::mcp_hooks() {
        (hooks.spawn_watchdog)();
    }
}

#[cfg(test)]
mod tests {
    use super::{canonicalize_tool_filter_names_with, tool_filter_contains_with};

    #[test]
    fn persisted_legacy_filter_names_follow_live_catalog_aliases() {
        let legacy = "mcp__alpha__historical_name".to_string();
        let builtin = "read".to_string();
        let canonical = "mcp__alpha__current_full_name";

        let normalized =
            canonicalize_tool_filter_names_with(&[legacy.clone(), builtin.clone()], |name| {
                (name == legacy).then(|| canonical.to_string())
            });

        assert_eq!(normalized, vec![canonical.to_string(), builtin]);
        assert!(tool_filter_contains_with(
            std::slice::from_ref(&legacy),
            canonical,
            |name| (name == legacy).then(|| canonical.to_string())
        ));
        assert!(!tool_filter_contains_with(
            std::slice::from_ref(&legacy),
            "mcp__alpha__other_tool",
            |name| (name == legacy).then(|| canonical.to_string())
        ));
    }
}
