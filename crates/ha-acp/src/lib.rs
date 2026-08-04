//! ACP 特征 crate（阶段 3 自 ha-core 迁出）：
//!
//! - [`acp`] —— Hope Agent 自己作为 ACP stdio server（`hope-agent acp` 模式）
//! - [`acp_control`] —— ACP 控制面：外部 ACP agent（Claude Code / Codex 等）
//!   的注册表 / 健康探测 / 会话管理 / stdio runtime，`acp_spawn` 工具的实现
//!
//! 装配契约与 ha-updater / ha-weather 相同：每个调 `ha_core::init_runtime`
//! 的二进制必须先调 [`wire()`]。

// `app_*!` 系宏由 ha-base 导出（与 ha-core 同一接法）。
#[macro_use]
extern crate ha_base;

pub mod acp;
pub mod acp_control;
mod tool;

/// 装配入口（幂等）：
/// 1. `acp_spawn` 分发条目 → 工具注册表（ToolDefinition 留 ha-core）；
/// 2. SessionManager 创建 → init 任务（原 `init_runtime` 子系统装配位，
///    所有 role 执行；enabled 判定在任务内读 cached_config）；
/// 3. backend 自动发现 → PrimaryOnly startup task（原位在 primary 块内，
///    spawn 异步 fire-and-forget，档位语义一致）；
/// 4. system prompt ACP 段的 binary 可用性解析器钩子。
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        fn acp_spawn_handler<'a>(
            args: &'a serde_json::Value,
            ctx: &'a ha_core::tools::ToolExecContext,
        ) -> ha_core::tools::registry::BuiltinToolFuture<'a> {
            Box::pin(tool::tool_acp_spawn(args, ctx))
        }
        ha_core::tools::registry::register_external_tools(vec![
            ha_core::tools::registry::BuiltinToolEntry {
                name: ha_core::tools::TOOL_ACP_SPAWN,
                aliases: &[],
                handler: acp_spawn_handler,
            },
        ])
        .expect("ha_acp::wire() must run before ha_core::init_runtime freezes the tool registry");

        fn create_manager_if_enabled() {
            let store = ha_core::config::cached_config();
            if store.acp_control.enabled {
                let registry = std::sync::Arc::new(acp_control::AcpRuntimeRegistry::new());
                let manager = std::sync::Arc::new(acp_control::AcpSessionManager::new(registry));
                acp_control::set_acp_manager(manager);
            }
        }
        ha_core::app_init::register_init_task(create_manager_if_enabled)
            .expect("ha_acp::wire() must run before ha_core::init_runtime consumes init tasks");

        fn spawn_backend_auto_discover() {
            if let Some(acp_mgr) = acp_control::get_acp_manager() {
                let store = ha_core::config::cached_config();
                if store.acp_control.enabled {
                    let registry = acp_mgr.runtime_registry().clone();
                    let acp_config = store.acp_control.clone();
                    tokio::spawn(async move {
                        acp_control::registry::auto_discover_and_register(&registry, &acp_config)
                            .await;
                    });
                }
            }
        }
        ha_core::app_init::register_startup_task(
            ha_core::app_init::StartupStage::PrimaryOnly,
            spawn_backend_auto_discover,
        )
        .expect("ha_acp::wire() must run before start_background_tasks consumes startup tasks");

        fn binary_resolvable(binary: &str) -> bool {
            acp_control::registry::resolve_binary(binary).is_some()
        }
        ha_core::system_prompt::register_acp_binary_resolver(binary_resolvable)
            .expect("ha_acp::wire() registers the acp binary resolver exactly once");
    });
}
