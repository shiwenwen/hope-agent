//! 设计空间特征 crate（阶段 3 收官刀，ha-design + ha-artifacts 合并迁出——
//! artifacts 是 design 的存储层，2-crate 环随合并消解）：
//!
//! - [`design`] —— 设计空间服务层 / 编译 / 导出 / code sync / MCP provider
//! - [`artifacts`] —— durable Artifact 注册表与存储（design.db）
//! - [`canvas_db`] / [`ffmpeg`] —— legacy canvas 注册表 / 视频合成
//! - `design` / `canvas` / `artifact` 三个工具 adapter
//!
//! 装配契约与其它特征 crate 相同：每个调 `ha_core::init_runtime` 的二进制
//! 必须先调 [`wire()`]。

// `app_*!` 系宏由 ha-base 导出（与 ha-core 同一接法）。
#[macro_use]
extern crate ha_base;

pub mod artifacts;
pub mod canvas_db;
pub mod design;
pub mod ffmpeg;
mod tool_artifact;
// pub：canvas 的 owner 平面 API（项目列表/版本/导出）由 Tauri/HTTP 薄壳直调。
pub mod tool_canvas;
mod tool_design;

/// 装配入口（幂等）：
/// 1. `design` / `canvas` / `artifact` 三个分发条目 → 工具注册表；
/// 2. session 侧钩子原子注册（Design 线程工作目录派生 / 会话清理级联
///    purge·detach / incognito 开启守卫）；
/// 3. 代码仓库 watcher → `PrimaryOnly` 档启动任务（原 `app_init` 调用位
///    在 primary 块内——多进程各自建 watcher 会并发写 design.db，档位
///    必须保持 Primary；块内位置前移无碍，fs 监听 spawn 无序依赖）。
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        fn design_handler<'a>(
            args: &'a serde_json::Value,
            ctx: &'a ha_core::tools::ToolExecContext,
        ) -> ha_core::tools::registry::BuiltinToolFuture<'a> {
            Box::pin(tool_design::tool_design(args, ctx))
        }
        fn canvas_handler<'a>(
            args: &'a serde_json::Value,
            ctx: &'a ha_core::tools::ToolExecContext,
        ) -> ha_core::tools::registry::BuiltinToolFuture<'a> {
            Box::pin(tool_canvas::tool_canvas(args, ctx))
        }
        fn artifact_handler<'a>(
            args: &'a serde_json::Value,
            ctx: &'a ha_core::tools::ToolExecContext,
        ) -> ha_core::tools::registry::BuiltinToolFuture<'a> {
            Box::pin(tool_artifact::tool_artifact(args, ctx))
        }
        ha_core::tools::registry::register_external_tools(vec![
            ha_core::tools::registry::BuiltinToolEntry {
                name: ha_core::tools::TOOL_DESIGN,
                aliases: &[],
                handler: design_handler,
            },
            ha_core::tools::registry::BuiltinToolEntry {
                name: ha_core::tools::TOOL_CANVAS,
                aliases: &[],
                handler: canvas_handler,
            },
            ha_core::tools::registry::BuiltinToolEntry {
                name: ha_core::tools::TOOL_ARTIFACT,
                aliases: &[],
                handler: artifact_handler,
            },
        ])
        .expect(
            "ha_design::wire() must run before ha_core::init_runtime freezes the tool registry",
        );

        fn cleanup_artifacts(session_id: &str, purge: bool) -> anyhow::Result<usize> {
            let service = crate::artifacts::ArtifactService::open()?;
            if purge {
                service.purge_for_session(session_id)
            } else {
                service.detach_from_session(session_id)
            }
        }
        fn has_durable_artifacts(session_id: &str) -> anyhow::Result<bool> {
            crate::artifacts::ArtifactService::open()?.has_for_session(session_id)
        }
        ha_core::session::design_hooks::register_design_session_hooks(
            ha_core::session::design_hooks::DesignSessionHooks {
                bound_code_dir: crate::design::service::session_bound_code_dir,
                cleanup_artifacts,
                has_durable_artifacts,
            },
        )
        .expect("ha_design::wire() registers the design session hooks exactly once");

        ha_core::app_init::register_startup_task(
            ha_core::app_init::StartupStage::PrimaryOnly,
            crate::design::code_watcher::start_all_watchers,
        )
        .expect("ha_design::wire() must run before start_background_tasks consumes startup tasks");
    });
}

#[cfg(test)]
pub(crate) mod test_support {
    //! 本地最小版 env 隔离助手（ha-core 的 `test_support` 是 cfg(test)
    //! crate 内私有，跨 crate 不可见；此处只复刻用到的 `with_env_vars`）。
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    pub fn with_env_vars<T>(vars: &[(&str, &Path)], f: impl FnOnce() -> T) -> T {
        let guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous: Vec<_> = vars
            .iter()
            .map(|(key, _)| (*key, std::env::var_os(key)))
            .collect();
        for (key, value) in vars {
            std::env::set_var(key, value);
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        for (key, value) in previous {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
        drop(guard);
        match result {
            Ok(v) => v,
            Err(p) => std::panic::resume_unwind(p),
        }
    }
}
