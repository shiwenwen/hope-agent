//! Cron **机器**的反向钩子——kernel → ha-cron 的唯一回调面。
//!
//! # 为什么是钩子
//!
//! 台账（`cron::CronDB`）留 kernel、机器（调度器 / 执行器 / 投递）上浮
//! ha-cron 之后，仍有三处 kernel 代码需要机器行为：`loop_control` 的托管
//! `/loop` 要真正把 cron 任务跑起来、`runtime_tasks` 要取消在跑的任务、
//! `subagent::injection` 完成后要按 cron 的投递白名单回投。ha-core 不依赖
//! 任何特征 crate，故倒转为注册钩子，由 `ha_cron::wire()` 装配。
//!
//! # 未装配语义（逐项镜像迁移前的「cron 不可用」分支）
//!
//! - `spawn_job_execution` → `app_warn!` 后 no-op。迁移前这条路径要求
//!   `CRON_DB` 已就绪；没有装配的进程本就跑不了 cron 任务。
//! - `cancel_running_job` → `Ok(None)`，与迁移前 `get_cron_db()` 返回
//!   `None` 时的返回值**逐字相同**（调用方据此报「任务不存在 / 未在跑」）。
//! - `deliver_injection_for_session` → 静默 no-op，与迁移前 cron db 缺席时
//!   的提前返回一致（投递是尽力而为，缺席不影响注入本身）。

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use crate::cron::{CronDB, CronJob};
use crate::session::SessionDB;

type BoxFut<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

pub struct CronHooks {
    /// `cron::start_scheduler(cron_db, session_db)`
    pub start_scheduler: fn(Arc<CronDB>, Arc<SessionDB>),
    /// `cron::spawn_job_execution(cron_db, session_db, job)`
    pub spawn_job_execution: fn(Arc<CronDB>, Arc<SessionDB>, CronJob),
    /// `cron::cancel_running_job(job_id)`
    pub cancel_running_job: fn(&str) -> anyhow::Result<Option<bool>>,
    /// `cron::delivery::deliver_injection_for_session(session_id, text)`
    pub deliver_injection_for_session: for<'a> fn(&'a str, &'a str) -> BoxFut<'a>,
}

static HOOKS: OnceLock<CronHooks> = OnceLock::new();

/// 装配期注册（四槽原子——部分注册＝调度能起但取消不掉，是难查的半瘫痪）。
pub fn register_cron_hooks(hooks: CronHooks) -> Result<(), crate::AlreadyRegistered> {
    HOOKS
        .set(hooks)
        .map_err(|_| crate::AlreadyRegistered("cron machinery hooks"))
}

/// 起调度线程。**必须在 `loop_control` 的事件 watcher 之前调用**——调度器
/// 启动会跑 `recover_orphaned_runs()` + `clear_all_running()`，后者**无差别
/// 清除所有 running 标记**；若 watcher 先跑并 claim 了任务，那个合法在途任务
/// 会被当成上一次会话的遗留标记清掉，从而可能被重复 claim、重复产生副作用。
///
/// 正因为这条顺序约束，调度器**没有**做成 startup task（那样会被排到
/// PrimaryOnly 队列里、落在 watcher 之后），而是保留在 `app_init` 的原调用位
/// 经本钩子转发——迁移前后时序逐位相同。
///
/// 未装配 → warn + no-op（没有装配的进程本就跑不了 cron）。
pub fn start_scheduler(cron_db: Arc<CronDB>, session_db: Arc<SessionDB>) {
    match HOOKS.get() {
        Some(h) => (h.start_scheduler)(cron_db, session_db),
        None => app_warn!(
            "cron",
            "hooks",
            "cron machinery not wired — scheduler not started"
        ),
    }
}

/// 把一个已 claim 的 cron 任务真正跑起来。未装配 → warn + no-op。
pub fn spawn_job_execution(cron_db: Arc<CronDB>, session_db: Arc<SessionDB>, job: CronJob) {
    match HOOKS.get() {
        Some(h) => (h.spawn_job_execution)(cron_db, session_db, job),
        None => app_warn!(
            "cron",
            "hooks",
            "cron machinery not wired — job {} not started",
            job.id
        ),
    }
}

/// 取消在跑的任务。未装配 → `Ok(None)`（同迁移前 cron db 缺席）。
pub fn cancel_running_job(job_id: &str) -> anyhow::Result<Option<bool>> {
    match HOOKS.get() {
        Some(h) => (h.cancel_running_job)(job_id),
        None => Ok(None),
    }
}

/// subagent 注入完成后按 cron 投递白名单回投。未装配 → 静默 no-op。
pub async fn deliver_injection_for_session(session_id: &str, text: &str) {
    if let Some(h) = HOOKS.get() {
        (h.deliver_injection_for_session)(session_id, text).await;
    }
}
