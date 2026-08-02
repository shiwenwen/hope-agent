//! 学习闭环**机器**的反向钩子——kernel → ha-improve 的唯一回调面。
//!
//! # 为什么只有一槽
//!
//! 第八刀把三个学习闭环模块（`coding_improvement` / `domain_eval` /
//! `domain_quality`）切成「台账留 kernel、机器上浮」两半之后，机器的入口
//! 几乎全部只被壳层（Tauri 命令 / HTTP 路由 / `ha-eval-runtime`）调用——
//! 那些是**正向**依赖，不需要钩子。真正 kernel 主动往上调的只有一处：
//! 工作流跑到终态时顺手记一条 coding retro。
//!
//! # 未装配语义
//!
//! `ensure_coding_workflow_retro_for_run` → `Ok(None)`，与迁出前
//! 「非终态 / 无痕会话」两个提前返回**逐字相同**，`transition_workflow_run`
//! 的 `if let Ok(Some(retro))` 分支据此跳过 `coding_retro_recorded` 事件。
//!
//! 这里刻意**不返 `Err`**：retro 是学习闭环的观察性副产物，工作流状态机
//! 本身不依赖它。让它把一次合法的终态转移变成失败，代价远大于少记一条
//! 复盘（同第七刀 `auto_review_post_turn` 的 no-op 语义）。装配契约仍要求
//! `wire()` 早于 `init_runtime`，正常路径下这条分支不会走到。

use std::sync::OnceLock;

use crate::coding_improvement::CodingWorkflowRetro;
use crate::session::SessionDB;
use crate::workflow::WorkflowRun;

pub struct ImproveHooks {
    /// `coding_improvement::ensure_coding_workflow_retro_for_run(db, run)`
    pub ensure_coding_workflow_retro_for_run:
        fn(&SessionDB, &WorkflowRun) -> anyhow::Result<Option<CodingWorkflowRetro>>,
}

static HOOKS: OnceLock<ImproveHooks> = OnceLock::new();

/// 装配期注册（幂等由 `ha_improve::wire()` 的 `Once` 保证）。
pub fn register_improve_hooks(hooks: ImproveHooks) -> Result<(), crate::AlreadyRegistered> {
    HOOKS
        .set(hooks)
        .map_err(|_| crate::AlreadyRegistered("improve machinery hooks"))
}

/// 工作流终态复盘。未装配 → `Ok(None)`（见模块文档）。
pub fn ensure_coding_workflow_retro_for_run(
    db: &SessionDB,
    run: &WorkflowRun,
) -> anyhow::Result<Option<CodingWorkflowRetro>> {
    match HOOKS.get() {
        Some(h) => (h.ensure_coding_workflow_retro_for_run)(db, run),
        None => Ok(None),
    }
}
