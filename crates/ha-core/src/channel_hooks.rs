//! IM 渠道机器的**反向钩子**——kernel → ha-channel 的唯一回调面。
//!
//! # 为什么需要
//!
//! 阶段 5 第五刀把 12 个 IM 插件 / worker / feishu 业务 API 自 ha-core 迁入 ha-channel，
//! 但 kernel 侧仍有两类反向需求：
//!
//! 1. **撤掉 IM 侧待决窗口**——`ask_user` / 审批的决议路径遍布 kernel
//!    （`tools::approval` / `ask_user::questions` / `tools::enter_plan_mode` /
//!    `tools::ask_user_question` / `session::cleanup_watcher`），每条都要在
//!    决议后把 IM 上那张卡片撤掉。这是 AGENTS 的 IM 红线：**所有决议路径
//!    （submit / 超时 / 删会话 / eviction）必须 emit `approval:resolved`
//!    统一撤窗**，漏一条就留下点不动的僵尸卡片。
//! 2. **主对话的 IM 实时镜像**——`chat_engine` 要在 turn 开始时挂镜像、
//!    结束时收尾，而镜像实现依赖 worker 的流式投递管道。
//!
//! # 台账不在这里
//!
//! `channel_conversations` 的读写（[`crate::channel::db::ChannelDB`]）与内存
//! 取消注册表（[`crate::channel::cancel`]）**留在 kernel**，不走钩子：前者是
//! AGENTS「一 chat ↔ 一 session 双向 1:1、读写一律走 `channel/db.rs` helper」
//! 红线的执行点，且它持 `SessionDB` 的连接；后者绑着 `CHANNEL_CANCELS` 全局与
//! `AppState` 的 `ptr_eq_lock` 不变量。同 cron 那刀 `CronDB` + `cron/cancel.rs`
//! 留 kernel 的分法。
//!
//! # 未装配语义
//!
//! **撤窗一族返回 `()` 即静默 no-op**——等价于「这个进程里没有 IM 待决窗口」，
//! 与迁移前 `PENDING_*` 注册表为空时逐位相同（那些函数本就是「查不到就直接
//! 返回」）。这是唯一正确的降级：撤窗是**清理**动作，无窗可撤不是错误，
//! 让它 fail loud 会把 headless / ACP 进程的每一次审批决议都变成告警。
//!
//! **镜像一族返回 `None`**——等价于「本会话没有 attach 到任何 IM chat」，
//! 与迁移前 `attach_im_live_mirror` 查不到 attach 时的返回值相同。
//! 已 attach 镜像的异常终态返回 [`ImLiveMirrorAbortStatus`]：可能自动重放同一
//! 逻辑结果的调用方必须消费它，`Unsafe` 时禁止重试。

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Whether an abnormal IM mirror terminal is known to have settled. Callers
/// that may replay the same logical result must only do so after `Confirmed`;
/// `Unsafe` means a persistent provider mutation may still be visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "automatic replay is safe only after a confirmed IM mirror terminal"]
pub enum ImLiveMirrorAbortStatus {
    Confirmed,
    Unsafe,
}

impl ImLiveMirrorAbortStatus {
    pub fn is_confirmed(self) -> bool {
        matches!(self, Self::Confirmed)
    }

    pub fn from_confirmed(confirmed: bool) -> Self {
        if confirmed {
            Self::Confirmed
        } else {
            Self::Unsafe
        }
    }
}

/// 一次已挂起的 IM 实时镜像。
///
/// kernel 的 `chat_engine` **只**做 attach → 持有 → 收尾三件事，从不读它的
/// 字段（迁移前那个 `ImLiveMirrorState` 的 sink / pipeline / plugin / attach
/// 全是私有的，engine 也确实一个都没碰）。因此这里用 trait object 收口，
/// 镜像状态整体留在 ha-channel。
pub trait ImLiveMirror: Send {
    /// 正常收尾：把最终回答投递到 IM 会话。
    fn finalize<'a>(self: Box<Self>, response: &'a str) -> BoxFut<'a, ()>;
    /// 异常收尾（取消 / 失败）：`body` 为 `None` 时只撤引用、不追加正文。
    /// 返回值是后续重放同一逻辑结果的安全闸，不能忽略后继续自动重试。
    fn abort(self: Box<Self>, body: Option<String>) -> BoxFut<'static, ImLiveMirrorAbortStatus>;
}

/// 上一条用户消息的快照，用于在 IM 侧渲染引用前缀。字段与迁移前
/// `chat_engine::im_mirror::LastUserSnapshot` 逐个对应。
#[derive(Debug, Clone)]
pub struct LastUserSnapshot {
    pub source: String,
    pub text: String,
    pub attachment_count: usize,
}

/// 十六槽**原子注册**：撤窗 5 + IM 实时镜像 2 + 账号开关 1 + watchdog 2 + 装配 6。
///
/// 部分注册＝一部分决议路径能撤窗、另一部分不能，正是那条 IM 红线最难查的
/// 破法，故与 `cron_hooks` / `slash_hooks` 同样一次性注册整组。
pub struct ChannelHooks {
    /// `worker::ask_user::drop_pending_by_request_id`
    pub drop_ask_user_by_request_id: for<'a> fn(&'a str) -> BoxFut<'a, ()>,
    /// `worker::ask_user::drop_pending_for_session`
    pub drop_ask_user_for_session: for<'a> fn(&'a str) -> BoxFut<'a, ()>,
    /// `worker::approval::drop_pending_by_request_id`
    pub drop_approval_by_request_id: for<'a> fn(&'a str) -> BoxFut<'a, ()>,
    /// `worker::approval::drop_pending_for_session`
    pub drop_approval_for_session: for<'a> fn(&'a str) -> BoxFut<'a, ()>,
    /// `worker::approval::drop_pending_for_chat`
    pub drop_approval_for_chat: for<'a> fn(&'a str, &'a str) -> BoxFut<'a, ()>,
    /// `chat_engine::im_mirror::attach_im_live_mirror`。`source` 传
    /// [`crate::chat_engine::stream_seq::ChatSource`]——**门在特征侧**
    /// （只有 Desktop / Http 才镜像），与迁移前逐位一致。
    pub attach_live_mirror: for<'a> fn(
        &'a str,
        crate::chat_engine::stream_seq::ChatSource,
        Option<LastUserSnapshot>,
    ) -> BoxFut<'a, Option<Box<dyn ImLiveMirror>>>,
    /// `chat_engine::im_mirror::attach_im_injection_mirror`
    pub attach_injection_mirror: for<'a> fn(&'a str) -> BoxFut<'a, Option<Box<dyn ImLiveMirror>>>,
    /// `channel::accounts::set_account_auto_transcribe_voice`
    pub set_account_auto_transcribe_voice: fn(&str, bool, &'static str) -> anyhow::Result<bool>,
    /// `channel::start_watchdog::mark_success`——账号起来了，撤掉重试退避。
    pub start_watchdog_mark_success: for<'a> fn(&'a str) -> BoxFut<'a, ()>,
    /// `channel::start_watchdog::cancel_pending`——账号被显式停掉，撤掉在排的重试。
    ///
    /// **AGENTS 红线「user 操作永远胜过 watchdog」的执行点**：`stop_account`
    /// 必须能取消 watchdog 正在排的自动重试，否则用户刚停掉的账号会被拉起来。
    pub start_watchdog_cancel_pending: for<'a> fn(&'a str) -> BoxFut<'a, ()>,

    // ── 装配槽（`app_init` 在建好 registry / ChannelDB 之后逐个调用）──
    /// 把 12 个内置插件装进 registry。registry 与其 `ChannelId` 键都是 kernel
    /// 契约，只有插件实现在特征侧。
    pub install_plugins: fn(&mut crate::channel::ChannelRegistry),
    /// `worker::spawn_dispatcher`——入站事件分发线程（自带 runtime，可从 sync
    /// 栈调用）。
    pub spawn_dispatcher: fn(
        std::sync::Arc<crate::channel::ChannelRegistry>,
        std::sync::Arc<crate::channel::ChannelDB>,
        tokio::sync::mpsc::Receiver<crate::channel::types::InboundEvent>,
    ),
    /// approval / ask_user / eviction 三个监听器。**需要环境 tokio runtime**，
    /// 故由 `start_background_tasks()` 调用而非 `init_runtime`（server / acp
    /// 从 sync 栈进 init）。
    ///
    /// startup-notifier **单独一槽**（下一项）：迁移前它排在 kernel 的
    /// `spawn_channel_menu_resync_listener` **之后**，合成一槽会把菜单重同步
    /// 从第 4 位挤到第 5 位。这一刀不改任何运行期顺序。
    pub spawn_listeners: fn(
        std::sync::Arc<crate::channel::ChannelDB>,
        std::sync::Arc<crate::channel::ChannelRegistry>,
    ),
    /// `worker::spawn_startup_notifier`——「重新上线」通知，自带
    /// `is_primary()` + `startup_notification.enabled` 双重自门控。
    pub spawn_startup_notifier: fn(std::sync::Arc<crate::channel::ChannelRegistry>),
    /// `start_watchdog::spawn_loop`——启动失败的后台重试循环。
    pub spawn_start_watchdog_loop: fn(std::sync::Arc<crate::channel::ChannelRegistry>),
    /// `start_watchdog::register_failure`——记一次启动失败并排重试。
    pub start_watchdog_register_failure: for<'a> fn(
        &'a crate::channel::types::ChannelAccountConfig,
        &'a anyhow::Error,
    ) -> BoxFut<'a, ()>,
}

static CHANNEL_HOOKS: OnceLock<ChannelHooks> = OnceLock::new();

/// 装配期注册 IM 渠道机器钩子（十六槽原子）。重复注册返回 `Err`。
pub fn register_channel_hooks(hooks: ChannelHooks) -> Result<(), crate::AlreadyRegistered> {
    CHANNEL_HOOKS
        .set(hooks)
        .map_err(|_| crate::AlreadyRegistered("channel machinery hooks"))
}

fn hooks() -> Option<&'static ChannelHooks> {
    CHANNEL_HOOKS.get()
}

/// 撤掉某个 `ask_user` 请求在 IM 侧的待决卡片。未装配即 no-op。
pub async fn drop_ask_user_by_request_id(request_id: &str) {
    if let Some(h) = hooks() {
        (h.drop_ask_user_by_request_id)(request_id).await;
    }
}

/// 撤掉某会话全部 `ask_user` 待决卡片。未装配即 no-op。
pub async fn drop_ask_user_for_session(session_id: &str) {
    if let Some(h) = hooks() {
        (h.drop_ask_user_for_session)(session_id).await;
    }
}

/// 撤掉某个审批请求在 IM 侧的待决卡片。未装配即 no-op。
///
/// **测试注入点**（`#[cfg(test)]` only）：kernel 测试可以经 `test_seam::install`
/// 装一个自定回调（例如阻塞回调），覆写默认的 hook 分派——用于验证
/// `deny_pending_for_session` / `deny_all_pending` 的终态事件排序（即
/// AGENTS.md IM channel 「审批一致性 + fail-closed」红线）。生产构建里
/// `#[cfg(test)]` 分支根本不编译，无副作用。
pub async fn drop_approval_by_request_id(request_id: &str) {
    #[cfg(test)]
    {
        let hook = test_seam::override_slot().read().unwrap().clone();
        if let Some(hook) = hook {
            hook(request_id).await;
            return;
        }
    }
    if let Some(h) = hooks() {
        (h.drop_approval_by_request_id)(request_id).await;
    }
}

/// 撤掉某会话全部审批待决卡片。未装配即 no-op。
pub async fn drop_approval_for_session(session_id: &str) {
    if let Some(h) = hooks() {
        (h.drop_approval_for_session)(session_id).await;
    }
}

/// 撤掉某 IM chat 下全部审批待决卡片（chat 接管 / eviction 用）。未装配即 no-op。
pub async fn drop_approval_for_chat(account_id: &str, chat_id: &str) {
    if let Some(h) = hooks() {
        (h.drop_approval_for_chat)(account_id, chat_id).await;
    }
}

/// 为主对话 turn 挂一次 IM 实时镜像。未装配 / 本会话未 attach 即 `None`。
pub async fn attach_live_mirror(
    session_id: &str,
    source: crate::chat_engine::stream_seq::ChatSource,
    last_user: Option<LastUserSnapshot>,
) -> Option<Box<dyn ImLiveMirror>> {
    match hooks() {
        Some(h) => (h.attach_live_mirror)(session_id, source, last_user).await,
        None => None,
    }
}

/// 为注入回投挂一次 IM 镜像。未装配 / 本会话未 attach 即 `None`。
pub async fn attach_injection_mirror(session_id: &str) -> Option<Box<dyn ImLiveMirror>> {
    match hooks() {
        Some(h) => (h.attach_injection_mirror)(session_id).await,
        None => None,
    }
}

/// 改某 IM 账号的「语音自动转写」开关（`ha-settings` 能力面用）。
///
/// **未装配即 `Err`**——与撤窗一族的 no-op 不同：这是一次**写入**，
/// 悄悄不生效会让模型以为改成功了。返回错误让 `settings` 工具如实回报。
pub fn set_account_auto_transcribe_voice(
    account_id: &str,
    on: bool,
    source: &'static str,
) -> anyhow::Result<bool> {
    match hooks() {
        Some(h) => (h.set_account_auto_transcribe_voice)(account_id, on, source),
        None => Err(anyhow::anyhow!(
            "IM channel accounts are not available in this build"
        )),
    }
}

/// 账号启动成功，撤掉 watchdog 的退避状态。未装配即 no-op（没有 watchdog 可撤）。
pub async fn start_watchdog_mark_success(account_id: &str) {
    if let Some(h) = hooks() {
        (h.start_watchdog_mark_success)(account_id).await;
    }
}

/// 账号被显式停掉，取消 watchdog 在排的自动重试。未装配即 no-op。
pub async fn start_watchdog_cancel_pending(account_id: &str) {
    if let Some(h) = hooks() {
        (h.start_watchdog_cancel_pending)(account_id).await;
    }
}

/// 把内置 IM 插件装进 registry。未装配即空 registry——该构建不含 IM 渠道，
/// 后续 `start_account` 会按既有的「未知 channel」分支返错。
pub fn install_plugins(registry: &mut crate::channel::ChannelRegistry) {
    if let Some(h) = hooks() {
        (h.install_plugins)(registry);
    }
}

/// 起入站事件分发器。未装配即不起——没有插件也就没有入站事件。
pub fn spawn_dispatcher(
    registry: std::sync::Arc<crate::channel::ChannelRegistry>,
    channel_db: std::sync::Arc<crate::channel::ChannelDB>,
    inbound_rx: tokio::sync::mpsc::Receiver<crate::channel::types::InboundEvent>,
) {
    if let Some(h) = hooks() {
        (h.spawn_dispatcher)(registry, channel_db, inbound_rx);
    }
}

/// 起 approval / ask_user / eviction 三个监听器。未装配即不起。
pub fn spawn_listeners(
    channel_db: std::sync::Arc<crate::channel::ChannelDB>,
    registry: std::sync::Arc<crate::channel::ChannelRegistry>,
) {
    if let Some(h) = hooks() {
        (h.spawn_listeners)(channel_db, registry);
    }
}

/// 起「重新上线」通知。未装配即不起。
pub fn spawn_startup_notifier(registry: std::sync::Arc<crate::channel::ChannelRegistry>) {
    if let Some(h) = hooks() {
        (h.spawn_startup_notifier)(registry);
    }
}

/// 起启动失败重试循环。未装配即不起。
pub fn spawn_start_watchdog_loop(registry: std::sync::Arc<crate::channel::ChannelRegistry>) {
    if let Some(h) = hooks() {
        (h.spawn_start_watchdog_loop)(registry);
    }
}

/// 记一次账号启动失败。未装配即 no-op。
pub async fn start_watchdog_register_failure(
    account: &crate::channel::types::ChannelAccountConfig,
    error: &anyhow::Error,
) {
    if let Some(h) = hooks() {
        (h.start_watchdog_register_failure)(account, error).await;
    }
}

/// 单一目的：让 kernel 侧的审批顺序测试可以在 kernel 内部就地写完，不必
/// 反向依赖 ha-channel 的 `pub(crate) hold_text_pending_lock_for_test`
/// 或搬到 `crates/ha-channel/tests/`（那要把大量 kernel `pub(crate)` 面
/// 放开为 `pub`）。全部 `#[cfg(test)]`，生产不编。
///
/// 位置说明：本 module 放在文件末尾以避免触发 `clippy::items_after_test_module`
/// ——`#[cfg(test)]` module 后不得再放生产项。
///
/// 测试用法：
/// ```ignore
/// let _serial = channel_hooks::test_seam::serial_guard().lock().await;
/// let _hook = channel_hooks::test_seam::install(Arc::new(|_id| Box::pin(async {
///     // 阻塞 / 记录 / 计数 …
/// })));
/// deny_pending_for_session(&session_id, source).await;
/// // 断言 …
/// ```
#[cfg(test)]
pub(crate) mod test_seam {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, OnceLock, RwLock};

    pub(crate) type BoxedTestHook =
        Arc<dyn Fn(&str) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

    static OVERRIDE_SLOT: OnceLock<RwLock<Option<BoxedTestHook>>> = OnceLock::new();
    static TEST_SERIAL: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

    pub(crate) fn override_slot() -> &'static RwLock<Option<BoxedTestHook>> {
        OVERRIDE_SLOT.get_or_init(|| RwLock::new(None))
    }

    /// 多个测试共用同一 override slot，必须先取这把锁再 install，否则会互相
    /// 覆盖（cargo test 默认并行）。
    pub(crate) fn serial_guard() -> &'static tokio::sync::Mutex<()> {
        TEST_SERIAL.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    /// RAII：drop 时清 override，无论测试成功还是 panic 都不会留残留状态。
    pub(crate) struct ApprovalHookGuard;
    impl Drop for ApprovalHookGuard {
        fn drop(&mut self) {
            if let Some(slot) = OVERRIDE_SLOT.get() {
                *slot.write().unwrap() = None;
            }
        }
    }

    pub(crate) fn install(hook: BoxedTestHook) -> ApprovalHookGuard {
        *override_slot().write().unwrap() = Some(hook);
        ApprovalHookGuard
    }
}
