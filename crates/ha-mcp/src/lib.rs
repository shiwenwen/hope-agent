//! MCP 客户端特征 crate（阶段 4 第三刀，自 ha-core 迁出）：McpManager
//! 注册表 / client / transport / oauth / credentials / watchdog /
//! `mcp_resource`·`mcp_prompt` 两工具 / owner API。
//!
//! kernel 侧留存：`ha_core::mcp`（wire 类型再导出 + `mcp__` 命名约定 +
//! auto-approve 信任谓词 + trampoline）。kernel 边界经
//! [`ha_core::mcp_hooks::McpHooks`] 九件套原子注册，未接线语义镜像
//! manager-None 的既有行为（见该模块 doc）。
//!
//! 装配契约与其它特征 crate 相同：每个调 `ha_core::init_runtime` 的二进
//! 制必须先调 [`wire()`]。
//!
//! Hard rule (enforced by code review, not the compiler): **no `use
//! tauri::*` anywhere in this crate.** The Tauri and axum shells talk to
//! this crate only through the public API re-exported below.

// `app_*!` 系宏由 ha-base 导出（与 ha-core 同一接法）。
#[macro_use]
extern crate ha_base;

pub mod api;
pub mod catalog;
pub mod client;
pub mod config;
pub mod credentials;
pub mod errors;
pub mod events;
pub mod invoke;
pub mod oauth;
pub mod prompts;
pub mod registry;
pub mod resources;
pub mod transport;
pub mod watchdog;

pub use config::{
    McpGlobalSettings, McpOAuthConfig, McpServerConfig, McpTransportSpec, McpTrustLevel,
};
pub use credentials::McpCredentials;
pub use errors::{McpError, McpResult};
pub use registry::{McpManager, ServerHandle, ServerState, ServerStatusSnapshot, ToolIndexEntry};

/// Hot-sync the MCP runtime from the current cached app config.
///
/// This handles both steady-state edits (`McpManager` already exists) and
/// the important cold-enable case where the app started with
/// `mcpGlobal.enabled=false` and the user turns MCP on later without a
/// restart.
pub async fn reconcile_from_config_cache() -> anyhow::Result<()> {
    let cfg = ha_core::config::cached_config();
    if let Some(mgr) = McpManager::global() {
        mgr.reconcile(cfg.mcp_global.clone(), cfg.mcp_servers.clone())
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        client::spawn_reconciled_eager_catalog_warmup();
        return Ok(());
    }

    if cfg.mcp_global.enabled {
        McpManager::init_global(cfg.mcp_global.clone(), cfg.mcp_servers.clone());
        client::spawn_initial_eager_catalog_warmup();
        if ha_core::runtime_lock::is_primary() {
            watchdog::spawn_watchdog_loop();
        }
        events::emit_servers_changed();
    }
    Ok(())
}

/// Look up a server by id or name, returning an `anyhow`-flavored
/// error so tool handlers can propagate it directly. Wrapper over
/// [`McpManager::locate`] that also turns "manager not initialized"
/// and "server not found" into distinct messages.
pub(crate) async fn locate_server(
    name_or_id: &str,
) -> anyhow::Result<std::sync::Arc<ServerHandle>> {
    let mgr =
        McpManager::global().ok_or_else(|| anyhow::anyhow!("MCP subsystem not initialized"))?;
    if !mgr.is_enabled().await {
        anyhow::bail!(
            "MCP subsystem is disabled in config (mcpGlobal.enabled=false); \
             server '{}' is unavailable",
            name_or_id
        );
    }
    mgr.locate(name_or_id)
        .await
        .ok_or_else(|| anyhow::anyhow!("MCP server '{name_or_id}' not found"))
}

/// Resolve a configured server and lazily populate its catalog/connection.
/// Resource and prompt meta-tools use this path so they can bootstrap an Idle
/// server instead of requiring a prior visit to Settings.
pub(crate) async fn ensure_server_connected(
    name_or_id: &str,
) -> anyhow::Result<std::sync::Arc<ServerHandle>> {
    let manager =
        McpManager::global().ok_or_else(|| anyhow::anyhow!("MCP subsystem not initialized"))?;
    let handle = locate_server(name_or_id).await?;
    client::ensure_connected(manager, handle.clone())
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    Ok(handle)
}

/// 幂等装配：注册 kernel 的 MCP 钩子九件套 + `mcp_resource` / `mcp_prompt`
/// 两个工具分发条目（ToolDefinition 仍在 kernel schema 目录）。
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        fn mcp_resource_handler<'a>(
            args: &'a serde_json::Value,
            _ctx: &'a ha_core::tools::ToolExecContext,
        ) -> ha_core::tools::registry::BuiltinToolFuture<'a> {
            Box::pin(resources::tool_mcp_resource(args))
        }
        fn mcp_prompt_handler<'a>(
            args: &'a serde_json::Value,
            _ctx: &'a ha_core::tools::ToolExecContext,
        ) -> ha_core::tools::registry::BuiltinToolFuture<'a> {
            Box::pin(prompts::tool_mcp_prompt(args))
        }
        ha_core::tools::registry::register_external_tools(vec![
            ha_core::tools::registry::BuiltinToolEntry {
                name: ha_core::tools::TOOL_MCP_RESOURCE,
                aliases: &[],
                handler: mcp_resource_handler,
            },
            ha_core::tools::registry::BuiltinToolEntry {
                name: ha_core::tools::TOOL_MCP_PROMPT,
                aliases: &[],
                handler: mcp_prompt_handler,
            },
        ])
        .expect("ha_mcp::wire() must run before ha_core::init_runtime freezes the tool registry");

        fn init_subsystem() -> bool {
            let store = ha_core::config::cached_config();
            let global = store.mcp_global.clone();
            let servers = store.mcp_servers.clone();
            if global.enabled {
                let enabled_count = servers.iter().filter(|s| s.enabled).count();
                McpManager::init_global(global, servers);
                client::spawn_initial_eager_catalog_warmup();
                app_info!(
                    "mcp",
                    "init",
                    "MCP subsystem initialized ({} enabled server(s))",
                    enabled_count
                );
                true
            } else {
                app_info!(
                    "mcp",
                    "init",
                    "MCP subsystem disabled via mcpGlobal.enabled=false"
                );
                false
            }
        }
        fn spawn_watchdog() {
            watchdog::spawn_watchdog_loop();
        }
        fn tool_definitions() -> std::sync::Arc<Vec<ha_core::tools::ToolDefinition>> {
            match McpManager::global() {
                Some(mgr) => mgr.mcp_tool_definitions(),
                None => std::sync::Arc::new(Vec::new()),
            }
        }
        fn ensure_tool_catalogs(
            eager_only: bool,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
            if eager_only {
                Box::pin(client::ensure_initial_eager_tool_catalogs())
            } else {
                Box::pin(client::ensure_tool_catalogs())
            }
        }
        fn has_pending_catalogs() -> bool {
            catalog::has_pending_catalogs()
        }
        fn tool_server_config(
            name: &str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<config::McpServerConfig>> + Send + '_>,
        > {
            Box::pin(async move {
                let manager = McpManager::global()?;
                let entry = manager.lookup_tool(name).await?;
                let handle = manager.get_by_id(&entry.server_id).await?;
                let cfg = handle.config.read().await;
                Some(cfg.clone())
            })
        }
        fn call_tool<'a>(
            name: &'a str,
            args: &'a serde_json::Value,
            ctx: &'a ha_core::tools::ToolExecContext,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + 'a>>
        {
            Box::pin(invoke::call_tool(name, args, ctx))
        }
        fn system_prompt_snippet() -> Option<String> {
            catalog::system_prompt_snippet()
        }
        fn reconcile_from_config(
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>> {
            Box::pin(reconcile_from_config_cache())
        }
        ha_core::mcp_hooks::register_mcp_hooks(ha_core::mcp_hooks::McpHooks {
            init_subsystem,
            spawn_watchdog,
            tool_definitions,
            ensure_tool_catalogs,
            has_pending_catalogs,
            tool_server_config,
            call_tool,
            system_prompt_snippet,
            reconcile_from_config,
        })
        .expect("ha_mcp::wire() registers the mcp hooks exactly once");
    });
}
