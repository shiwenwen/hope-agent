//! `/recap` 的**分发钩子**——slash 装配层 → ha-dash 的唯一回调面。
//!
//! # 为什么是钩子
//!
//! `/recap` 的 handler 干的全是 recap 领域的事：解析 `--range` / `--agent`、
//! 建 `RecapContext`、后台 spawn 生成、往 `recap_progress` 事件上打进度。这
//! 些随 recap 一起进了 ha-dash，而 [`crate::slash_commands`] 是 ha-core 内的
//! 装配层——ha-core 不依赖任何特征 crate，直接调就是反向边。故装配层只留一
//! 行 trampoline，实现由 `ha_dash::wire()` 注册。
//!
//! 跨边界的只有 [`CommandResult`]（`slash_defs` 的 kernel wire 类型），recap
//! 自己的 `GenerateMode` / `RecapFilters` / `RecapProgress` **不必下沉**。
//!
//! # 未装配语义
//!
//! `Err`——与 [`crate::slash_hooks::dispatch`] 未装配时同款：无装配的进程里
//! 根本没有 handler 可跑，返回错误让调用方走既有错误分支，**不会**把命令
//! 文本当普通消息喂给模型。

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use crate::slash_defs::types::CommandResult;

/// `/recap` handler 的返回体。生命周期绑在入参 `args` 上。
pub type BoxSlashFut<'a> = Pin<Box<dyn Future<Output = Result<CommandResult, String>> + Send + 'a>>;

type SlashRecapHook = for<'a> fn(&'a str) -> BoxSlashFut<'a>;

static SLASH_RECAP: OnceLock<SlashRecapHook> = OnceLock::new();

/// 装配期注册 `/recap` 实现。重复注册返回 `Err`。
pub fn register_slash_recap(hook: SlashRecapHook) -> Result<(), crate::AlreadyRegistered> {
    SLASH_RECAP
        .set(hook)
        .map_err(|_| crate::AlreadyRegistered("/recap slash hook"))
}

/// 执行 `/recap`。未装配即 `Err`（见模块文档「未装配语义」）。
pub async fn run_slash_recap(args: &str) -> Result<CommandResult, String> {
    match SLASH_RECAP.get() {
        Some(hook) => hook(args).await,
        None => Err("recap is not available in this build".to_string()),
    }
}
