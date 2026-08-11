//! 特征 crate 钩子：kernel ↔ MCP 客户端特征（ha-mcp）边界。
//!
//! kernel 在十个点需要 MCP 运行时的行为，倒转为装配期原子注册（单
//! OnceLock struct，部分注册不可能）。**未接线语义逐项镜像「manager 未
//! 初始化（`McpManager::global() == None`）」的既有行为**——迁移前每个
//! 调用点本来就要处理 None：
//!
//! - `init_subsystem` / `spawn_watchdog`：app_init 两档起动面（全 tier
//!   init + Primary watchdog；minimal/ACP 路径也调 init，与 browser
//!   broker 同型故走专用钩子而非 startup 档）。未接线 init 返 `false`
//!   并 `app_info` 跳过（等价于 mcpGlobal.enabled=false 的观感）。
//! - `tool_definitions`：动态 MCP 工具 schema 快照（agent 组包 /
//!   definitions / tool_search 消费）。未接线返空——与 manager None
//!   一致，MCP 工具整体缺席。
//! - `ensure_tool_catalogs`：`tool_search` 的 lazy catalog 自举入口，亦
//!   作为首个 provider 请求的 eager 完成屏障；未接线 no-op，保持特征
//!   缺席时 MCP 候选为空。
//! - `has_pending_catalogs`：同步目录就绪信号，用于在 lazy MCP 完成首
//!   次发现前保留本地 `tool_search`；未接线返回 `false`。
//! - `canonical_tool_name`：把历史 namespaced 名称归一到当前目录名称，
//!   未接线返 `None`（调用方保留原名）。
//! - `tool_server_name`：按原子 catalog 快照返回工具的真实 server owner；
//!   未接线返 `None`，scoped discovery 据此 fail closed。
//! - `tool_server_config`：按 namespaced 工具名反查所属 server 配置
//!   （execution 的 auto-approve 门消费；**trust-level 安全谓词
//!   `server_auto_approves_config` 留 kernel**，钩子只做运行时
//!   查表）。未接线返 `None` → auto-approve 恒 false（fail-closed）。
//! - `call_tool`：`mcp__<server>__<tool>` 分发（execution 逃逸口 +
//!   hooks runner）。未接线 `Err`——与 manager None 的既有报错一致。
//! - `system_prompt_snippet`：兼容命名；返回 MCP capability data 段，
//!   renderer 只能将其放入动态 user-data lane。未接线 `None`。
//! - `reconcile_from_config`：settings 热更（`mcp_global` 类目）冷启用
//!   /重协调。未接线 `Ok(())` + `app_warn` 审计（设置写路径不因特征
//!   缺席硬失败）。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::tool_defs::ToolDefinition;
use crate::tool_defs::ToolExecContext;
use ha_config_schema::mcp::McpServerConfig;

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub struct McpHooks {
    /// 读 cached_config 并幂等 init_global；返回「MCP 已启用且完成 init」。
    pub init_subsystem: fn() -> bool,
    /// Primary-only 长驻重连 watchdog（调用方决定 tier 门）。
    pub spawn_watchdog: fn(),
    /// 动态工具定义快照（ArcSwap 支撑，同步可调）。
    pub tool_definitions: fn() -> Arc<Vec<ToolDefinition>>,
    /// 为 MCP server 执行有界并发 catalog 发现；`eager_only=true` 只等待
    /// 明确配置为 eager 的 server；`server_name=Some` 时只自举指定服务，
    /// 否则供 `tool_search` 做全量发现。
    pub ensure_tool_catalogs:
        fn(eager_only: bool, server_name: Option<String>) -> BoxFut<'static, ()>,
    /// 是否仍有有效 server 未完成过一次 catalog round。
    pub has_pending_catalogs: fn() -> bool,
    /// 当前或历史 namespaced 工具名 → 当前 provider-facing 名称。
    pub canonical_tool_name: fn(&str) -> Option<String>,
    /// 当前或历史 namespaced 工具名 → catalog 中的真实 server owner。
    pub tool_server_name: fn(&str) -> Option<String>,
    /// namespaced 工具名 → 所属 server 的当前配置克隆。
    pub tool_server_config: for<'a> fn(&'a str) -> BoxFut<'a, Option<McpServerConfig>>,
    /// MCP 工具分发（approval/沙箱已在 kernel execution 层完成）。
    pub call_tool: for<'a> fn(
        &'a str,
        &'a serde_json::Value,
        &'a ToolExecContext,
    ) -> BoxFut<'a, anyhow::Result<String>>,
    /// MCP capability data 段（兼容字段名；不得提升为 system）。
    pub system_prompt_snippet: fn() -> Option<String>,
    /// settings 热更：从 config cache 重协调（含冷启用）。
    pub reconcile_from_config: fn() -> BoxFut<'static, anyhow::Result<()>>,
}

static MCP_HOOKS: std::sync::OnceLock<McpHooks> = std::sync::OnceLock::new();

/// 特征 crate 装配期注册。重复注册返回 `Err`。
pub fn register_mcp_hooks(hooks: McpHooks) -> Result<(), crate::AlreadyRegistered> {
    MCP_HOOKS
        .set(hooks)
        .map_err(|_| crate::AlreadyRegistered("mcp hooks"))
}

pub(crate) fn mcp_hooks() -> Option<&'static McpHooks> {
    MCP_HOOKS.get()
}
