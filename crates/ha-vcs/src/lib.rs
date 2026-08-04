//! VCS / 本地执行基建特征 crate（阶段 4 第二刀，自 ha-core 迁出）：
//! Git 控制平面操作面（index/branch/commit/push/PR/handoff + 启动期对账）、
//! Docker/WSL 沙箱执行机器、SearxNG Docker 部署。
//!
//! kernel 侧留存（类型随表下沉 + 配置面）：`ha_core::git_control`
//! （`git_operation_runs` 簿记与 `repository_revision`）、
//! `ha_core::sandbox`（`SandboxConfig` 持久化与 trampoline）；
//! `worktree` / `project_bootstrap` 是 session 生命周期簿记，整体留 kernel。
//!
//! 装配契约与其它特征 crate 相同：每个调 `ha_core::init_runtime` 的二进制
//! 必须先调 [`wire()`]。未接线语义（fail-closed / 审计跳过）见
//! `ha_core::vcs_hooks`。

// `app_*!` 系宏由 ha-base 导出（与 ha-core 同一接法）。
#[macro_use]
extern crate ha_base;

pub mod docker;
pub mod git_control;
pub mod sandbox;

/// 幂等装配：注册 kernel 的 VCS 钩子（沙箱三口 + git 操作对账）。
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        fn sandbox_check() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = ha_core::sandbox::DockerStatus> + Send>,
        > {
            Box::pin(sandbox::check_sandbox_available())
        }
        fn sandbox_ensure(
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>>
        {
            Box::pin(sandbox::ensure_sandbox_available())
        }
        fn sandbox_exec_mode(
            req: ha_core::vcs_hooks::SandboxExecRequest,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = anyhow::Result<ha_core::sandbox::SandboxResult>>
                    + Send,
            >,
        > {
            Box::pin(async move {
                sandbox::exec_in_sandbox_mode(
                    &req.command,
                    &req.cwd,
                    req.env.as_ref(),
                    &req.config,
                    req.timeout_secs,
                    req.cancellation_token,
                    req.mode,
                )
                .await
            })
        }
        fn git_ops_reconciler(
            db: &std::sync::Arc<ha_core::session::SessionDB>,
        ) -> anyhow::Result<usize> {
            git_control::reconcile_interrupted_git_operations(db)
        }
        ha_core::vcs_hooks::register_vcs_hooks(ha_core::vcs_hooks::VcsHooks {
            sandbox_check,
            sandbox_ensure,
            sandbox_exec_mode,
            git_ops_reconciler,
        })
        .expect("ha_vcs::wire() registers the vcs hooks exactly once");
    });
}
