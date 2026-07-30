//! 特征 crate 钩子：kernel ↔ VCS/沙箱特征（ha-vcs）边界。
//!
//! kernel 在两组点需要 ha-vcs 的行为，倒转为装配期原子注册（单 OnceLock
//! struct，部分注册不可能）。缺失接线语义逐项、全部 fail-closed：
//!
//! - 沙箱三口（exec 工具 / cron 预检 / 状态查询）：`ensure` 与
//!   `exec_mode` 未 wire 即 `Err`——沙箱红线是「不可用即终止、绝不回落
//!   宿主机」，缺接线与 Docker 不可用同一语义；`check` 未 wire 返
//!   `installed:false` 的状态对象（只读展示面，不 gate 执行）。
//! - git 操作对账（Primary 启动期，`init_runtime` 内唯一生产调用点；进程
//!   tier 生命周期内固定、无晋升路径）：未 wire 记 `app_warn` 跳过——
//!   `git_operation_runs` 里的 `running` 行保持原状直到接线进程接手，
//!   簿记不损坏；但 handoff 中断后的源
//!   checkout 恢复不会发生，故必须审计可见。
//!
//! 时序契约：对账钩子由 `recover_startup_session_state` **同步内联**调用
//! （见 app_init.rs 的 ordering invariant——`replay_pending_jobs` 依赖它
//! 先完成），不得改为后台任务注册。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::sandbox::{DockerStatus, SandboxConfig, SandboxResult};
use crate::session::SessionDB;

type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// `exec_in_sandbox_mode` 的 owned 参数包（fn-pointer 钩子须 `'static`，
/// kernel trampoline 保持原借用签名、进钩子前克隆为 owned）。
pub struct SandboxExecRequest {
    pub command: String,
    pub cwd: String,
    pub env: Option<serde_json::Map<String, serde_json::Value>>,
    pub config: SandboxConfig,
    pub timeout_secs: u64,
    pub cancellation_token: Option<tokio_util::sync::CancellationToken>,
    pub mode: crate::permission::SandboxMode,
}

pub struct VcsHooks {
    /// Docker/WSL 沙箱后端状态探测（壳层状态端点经 kernel trampoline）。
    pub sandbox_check: fn() -> BoxFut<DockerStatus>,
    /// 沙箱可用性预检（exec 工具与 cron 执行器的 fail-closed 门）。
    pub sandbox_ensure: fn() -> BoxFut<anyhow::Result<()>>,
    /// 在选定沙箱模式内执行命令（`Isolated` 语义见 ha-vcs::sandbox）。
    pub sandbox_exec_mode: fn(SandboxExecRequest) -> BoxFut<anyhow::Result<SandboxResult>>,
    /// Primary 启动期对账中断的 git 操作（handoff 回滚 + 标记
    /// interrupted）；返回处理的行数。
    pub git_ops_reconciler: fn(&Arc<SessionDB>) -> anyhow::Result<usize>,
}

static VCS_HOOKS: std::sync::OnceLock<VcsHooks> = std::sync::OnceLock::new();

/// 特征 crate 装配期注册。重复注册返回 `Err`。
pub fn register_vcs_hooks(hooks: VcsHooks) -> Result<(), crate::AlreadyRegistered> {
    VCS_HOOKS
        .set(hooks)
        .map_err(|_| crate::AlreadyRegistered("vcs hooks"))
}

pub(crate) fn vcs_hooks() -> Option<&'static VcsHooks> {
    VCS_HOOKS.get()
}
