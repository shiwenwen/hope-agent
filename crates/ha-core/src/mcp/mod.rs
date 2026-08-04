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

use crate::tool_defs::ToolDefinition;

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
        None => Arc::new(Vec::new()),
    }
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
