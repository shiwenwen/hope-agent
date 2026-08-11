//! MCP 客户端特征 crate（阶段 4 第三刀，自 ha-core 迁出）：McpManager
//! 注册表 / client / transport / oauth / credentials / watchdog /
//! `mcp_resource`·`mcp_prompt` 两工具 / owner API。
//!
//! kernel 侧留存：`ha_core::mcp`（wire 类型再导出 + `mcp__` 命名约定 +
//! auto-approve 信任谓词 + trampoline）。kernel 边界经
//! [`ha_core::mcp_hooks::McpHooks`] 十件套原子注册，未接线语义镜像
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

/// Preserve catalog generation identity while MCP is globally disabled and no
/// manager exists. Kernel callers compare this `Arc` by pointer to detect real
/// catalog refreshes between tool rounds.
static EMPTY_TOOL_DEFINITIONS: std::sync::OnceLock<
    std::sync::Arc<Vec<ha_core::tools::ToolDefinition>>,
> = std::sync::OnceLock::new();

fn tool_definitions_snapshot() -> std::sync::Arc<Vec<ha_core::tools::ToolDefinition>> {
    match McpManager::global() {
        Some(manager) => manager.mcp_tool_definitions(),
        None => EMPTY_TOOL_DEFINITIONS
            .get_or_init(|| std::sync::Arc::new(Vec::new()))
            .clone(),
    }
}

fn connector_mention_config_is_available(
    server: &McpServerConfig,
    denied_servers: &[String],
) -> bool {
    server.enabled
        && !denied_servers.contains(&server.name)
        && config::validate_server_config(server).is_ok()
}

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
        fn list_connector_mentions(
            principal_agent_id: &str,
        ) -> Vec<ha_core::mention_hooks::MentionCapabilityCandidate> {
            if !ha_core::agent_loader::load_agent(principal_agent_id)
                .map(|agent| agent.config.capabilities.mcp_enabled)
                .unwrap_or(false)
            {
                return Vec::new();
            }
            let store = ha_core::config::cached_config();
            if !store.mcp_global.enabled {
                return Vec::new();
            }
            store
                .mcp_servers
                .iter()
                .filter(|server| {
                    connector_mention_config_is_available(
                        server,
                        &store.mcp_global.denied_servers,
                    )
                })
                .map(|server| ha_core::mention_hooks::MentionCapabilityCandidate {
                    kind: ha_core::prompt_context::MentionKind::Connector,
                    target_id: server.id.clone(),
                    display_label: server.name.clone(),
                    namespace: String::new(),
                    summary: server.description.clone().unwrap_or_else(|| {
                        "Configured MCP connector; live tool, authentication, scope, disclosure, and approval checks apply at use time.".to_string()
                    }),
                })
                .collect()
        }
        fn resolve_connector_mention(
            kind: ha_core::prompt_context::MentionKind,
            target_id: &str,
            principal_agent_id: &str,
        ) -> Option<ha_core::mention_hooks::ResolvedCapabilityMention> {
            if kind != ha_core::prompt_context::MentionKind::Connector {
                return None;
            }
            if !ha_core::agent_loader::load_agent(principal_agent_id)
                .map(|agent| agent.config.capabilities.mcp_enabled)
                .unwrap_or(false)
            {
                return None;
            }
            let store = ha_core::config::cached_config();
            if !store.mcp_global.enabled {
                return None;
            }
            let server = store
                .mcp_servers
                .iter()
                .find(|server| {
                    connector_mention_config_is_available(
                        server,
                        &store.mcp_global.denied_servers,
                    )
                        && server.id == target_id
                })?;
            Some(ha_core::mention_hooks::ResolvedCapabilityMention {
                namespace: format!("mcp:{}", server.id),
                display_alias: server.name.clone(),
                capability_summary: "Configured MCP connector. Discover and call only the tools/resources needed for the user's request; live authentication, scope, disclosure, and approval checks still apply.".to_string(),
            })
        }
        ha_core::mention_hooks::register_mention_provider(
            ha_core::mention_hooks::MentionProvider {
                namespace: "mcp",
                list: list_connector_mentions,
                resolve: resolve_connector_mention,
            },
        )
        .expect("ha_mcp::wire() registers the MCP mention provider once");

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
        fn ensure_tool_catalogs(
            eager_only: bool,
            server_name: Option<String>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
            if eager_only {
                Box::pin(client::ensure_initial_eager_tool_catalogs())
            } else {
                Box::pin(async move {
                    client::ensure_tool_catalogs(server_name.as_deref()).await;
                })
            }
        }
        fn has_pending_catalogs() -> bool {
            catalog::has_pending_catalogs()
        }
        fn canonical_tool_name(name: &str) -> Option<String> {
            McpManager::global()?.canonical_tool_name(name)
        }
        fn tool_server_name(name: &str) -> Option<String> {
            McpManager::global()?.tool_server_name(name)
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
            tool_definitions: tool_definitions_snapshot,
            ensure_tool_catalogs,
            has_pending_catalogs,
            canonical_tool_name,
            tool_server_name,
            tool_server_config,
            call_tool,
            system_prompt_snippet,
            reconcile_from_config,
        })
        .expect("ha_mcp::wire() registers the mcp hooks exactly once");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_server() -> McpServerConfig {
        McpServerConfig {
            id: "id-1".into(),
            name: "connector".into(),
            enabled: true,
            transport: McpTransportSpec::Stdio {
                command: "true".into(),
                args: vec![],
                cwd: None,
            },
            env: Default::default(),
            headers: Default::default(),
            oauth: None,
            allowed_tools: vec![],
            denied_tools: vec![],
            connect_timeout_secs: 30,
            call_timeout_secs: 120,
            health_check_interval_secs: 60,
            max_concurrent_calls: 4,
            auto_approve: false,
            trust_level: McpTrustLevel::Untrusted,
            eager: false,
            deferred_tools: false,
            project_paths: vec![],
            description: None,
            icon: None,
            created_at: 0,
            updated_at: 0,
            trust_acknowledged_at: None,
        }
    }

    #[test]
    fn disabled_catalog_snapshot_keeps_pointer_identity() {
        assert!(McpManager::global().is_none());
        let first = tool_definitions_snapshot();
        let second = tool_definitions_snapshot();
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        assert!(first.is_empty());
    }

    #[test]
    fn connector_mentions_exclude_invalid_server_configs() {
        let valid = valid_server();
        assert!(connector_mention_config_is_available(&valid, &[]));

        let mut invalid = valid;
        invalid.transport = McpTransportSpec::Stdio {
            command: " ".into(),
            args: vec![],
            cwd: None,
        };
        assert!(config::validate_server_config(&invalid).is_err());
        assert!(!connector_mention_config_is_available(&invalid, &[]));
    }
}
