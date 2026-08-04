//! IM 渠道特征 crate（阶段 5 第五刀，自 ha-core 迁出）：12 个聊天平台插件
//! （telegram / wechat / slack / feishu / discord / qqbot / irc / signal /
//! imessage / whatsapp / googlechat / line）、入站 worker 分发与媒体管道、
//! 账号生命周期与启动重试 watchdog、主对话的 IM 实时镜像，以及飞书业务工具
//! adapter。
//!
//! # 分法：台账 + 契约留 kernel，机器上浮
//!
//! 留在 [`ha_core::channel`] 的四件东西**都不是「渠道行为」**：
//!
//! - **`db`（`ChannelDB` / `ChannelConversation`）**——它持
//!   `SessionDB` 的连接直接读写 `channel_conversations`（表在 kernel 自己的
//!   `sessions.db` 里），更是 AGENTS 红线「一 chat ↔ 一 session **双向 1:1**，
//!   读写一律走 `channel/db.rs` helper」的**执行点**。同 cron 那刀 `CronDB`
//!   留 kernel。
//! - **`cancel`（`ChannelCancelRegistry`）**——纯内存取消注册表，绑着
//!   `CHANNEL_CANCELS` 全局、`AppState.channel_cancels` 与 `app_init` 里那条
//!   「两者必须共享同一 `Arc`」的 `ptr_eq_lock` 断言。
//! - **`traits`（`ChannelPlugin`）/ `registry`（`ChannelRegistry`）**——契约与
//!   持有者，不是机器：trait 零实现，registry 只按 `ChannelId` 存
//!   `Arc<dyn ChannelPlugin>` 并转发 start/stop/restart。**正因为它们留下，
//!   `CHANNEL_REGISTRY` 全局、`AppState` 字段与壳层的 **registry** 调用点
//!   （src-tauri 9 处 / ha-server 1 处 / ha-cron 1 处）一处未改**——
//!   非 registry 的 API（`accounts` / `wechat::login` / `attach_sync`）壳层
//!   照常改成 `ha_channel::`——12 个插件实现由本 crate 在 [`wire()`]
//!   里装进去。
//! - **`types` / `config`**——`AppConfig` 可达的 wire 类型，定义处本就在
//!   ha-config-schema。
//!
//! 名字常量同理：35 个 `feishu_*` 已下沉 `ha_core::tool_defs::feishu_names`
//! （kernel 的 `permission::engine::classify_external_connector_action` 要按名字
//! 裁决「改动了另一个记录系统」），[`tools::feishu`] 对它 glob 再导出。
//!
//! # 反向回调
//!
//! kernel → 本 crate 的**唯一**回调面是 [`ha_core::channel_hooks`]（十六槽
//! 原子注册）：撤窗 5 + IM 实时镜像 2 + 账号开关 1 + watchdog 2 + 装配 6。
//! 未装配语义逐槽在该模块文档里论证——撤窗一族是 no-op（无窗可撤不是错误），
//! 镜像一族是 `None`（本会话未 attach IM），账号开关是 `Err`（写入不能静默失败）。
//!
//! 装配契约与其它特征 crate 相同：每个调 `ha_core::init_runtime` 的二进制
//! 必须先调 [`wire()`]。

// `app_*!` 系宏由 ha-base 导出（与 ha-core / ha-cron / ha-media 同一接法）。
#[macro_use]
extern crate ha_base;

pub mod channel;
pub mod im_mirror;
pub mod tools;

/// 幂等装配：三处接线。
///
/// 1. **`channel_hooks` 十六槽原子注册**——见 crate 文档「反向回调」。
/// 2. **35 个飞书工具的分发条目**（`register_external_tools`）。
/// 3. **35 个飞书工具的 schema 提供者**（`register_external_tool_definitions`）
///    ——与 2 配对是 `tools::registry` 的明文契约：只注册 handler 不给
///    `ToolDefinition`，工具就能执行却不受 `tools.allow/deny` 约束，
///    `freeze_now()` 会记 warn。
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        register_hooks();
        register_feishu_tools();
    });
}

fn register_hooks() {
    use ha_core::channel_hooks as h;
    use std::sync::Arc;

    type Fut<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;

    fn drop_ask_user_by_request_id(id: &str) -> Fut<'_> {
        Box::pin(channel::worker::ask_user::drop_pending_by_request_id(id))
    }
    fn drop_ask_user_for_session(sid: &str) -> Fut<'_> {
        Box::pin(channel::worker::ask_user::drop_pending_for_session(sid))
    }
    fn drop_approval_by_request_id(id: &str) -> Fut<'_> {
        Box::pin(channel::worker::approval::drop_pending_by_request_id(id))
    }
    fn drop_approval_for_session(sid: &str) -> Fut<'_> {
        Box::pin(channel::worker::approval::drop_pending_for_session(sid))
    }
    fn drop_approval_for_chat<'a>(account_id: &'a str, chat_id: &'a str) -> Fut<'a> {
        Box::pin(channel::worker::approval::drop_pending_for_chat(
            account_id, chat_id,
        ))
    }
    fn watchdog_mark_success(id: &str) -> Fut<'_> {
        Box::pin(channel::start_watchdog::mark_success(id))
    }
    fn watchdog_cancel_pending(id: &str) -> Fut<'_> {
        Box::pin(channel::start_watchdog::cancel_pending(id))
    }
    fn watchdog_register_failure<'a>(
        account: &'a ha_core::channel::types::ChannelAccountConfig,
        error: &'a anyhow::Error,
    ) -> Fut<'a> {
        Box::pin(channel::start_watchdog::register_failure(account, error))
    }
    fn set_auto_transcribe(id: &str, on: bool, source: &'static str) -> anyhow::Result<bool> {
        channel::accounts::set_account_auto_transcribe_voice(id, on, source)
    }

    /// 12 个内置插件。顺序与迁移前 `app_init` 里那段逐行一致。
    fn install_plugins(registry: &mut ha_core::channel::ChannelRegistry) {
        registry.register_plugin(Arc::new(channel::telegram::TelegramPlugin::new()));
        registry.register_plugin(Arc::new(channel::wechat::WeChatPlugin::new()));
        registry.register_plugin(Arc::new(channel::slack::SlackPlugin::new()));
        registry.register_plugin(Arc::new(channel::feishu::FeishuPlugin::new()));
        registry.register_plugin(Arc::new(channel::discord::DiscordPlugin::new()));
        registry.register_plugin(Arc::new(channel::qqbot::QqBotPlugin::new()));
        registry.register_plugin(Arc::new(channel::irc::IrcPlugin::new()));
        registry.register_plugin(Arc::new(channel::signal::SignalPlugin::new()));
        registry.register_plugin(Arc::new(channel::imessage::IMessagePlugin::new()));
        registry.register_plugin(Arc::new(channel::whatsapp::WhatsAppPlugin::new()));
        registry.register_plugin(Arc::new(channel::googlechat::GoogleChatPlugin::new()));
        registry.register_plugin(Arc::new(channel::line::LinePlugin::new()));
    }

    fn spawn_dispatcher(
        registry: Arc<ha_core::channel::ChannelRegistry>,
        channel_db: Arc<ha_core::channel::ChannelDB>,
        inbound_rx: tokio::sync::mpsc::Receiver<ha_core::channel::types::InboundEvent>,
    ) {
        channel::worker::spawn_dispatcher(registry, channel_db, inbound_rx);
    }

    /// 前三个监听器，顺序与迁移前 `spawn_channel_listeners()` 一致。
    /// 菜单重同步（kernel）夹在这三个与 startup notifier 之间，故后者单独一槽。
    fn spawn_listeners(
        channel_db: Arc<ha_core::channel::ChannelDB>,
        registry: Arc<ha_core::channel::ChannelRegistry>,
    ) {
        channel::worker::approval::spawn_channel_approval_listener(
            channel_db.clone(),
            registry.clone(),
        );
        channel::worker::ask_user::spawn_channel_ask_user_listener(channel_db, registry.clone());
        channel::worker::spawn_channel_eviction_watcher(registry);
    }

    fn spawn_startup_notifier(registry: Arc<ha_core::channel::ChannelRegistry>) {
        channel::worker::spawn_startup_notifier(registry);
    }

    fn spawn_start_watchdog_loop(registry: Arc<ha_core::channel::ChannelRegistry>) {
        channel::start_watchdog::spawn_loop(registry);
    }

    h::register_channel_hooks(h::ChannelHooks {
        drop_ask_user_by_request_id,
        drop_ask_user_for_session,
        drop_approval_by_request_id,
        drop_approval_for_session,
        drop_approval_for_chat,
        attach_live_mirror: im_mirror::attach_live_hook,
        attach_injection_mirror: im_mirror::attach_injection_hook,
        set_account_auto_transcribe_voice: set_auto_transcribe,
        start_watchdog_mark_success: watchdog_mark_success,
        start_watchdog_cancel_pending: watchdog_cancel_pending,
        install_plugins,
        spawn_dispatcher,
        spawn_listeners,
        spawn_startup_notifier,
        spawn_start_watchdog_loop,
        start_watchdog_register_failure: watchdog_register_failure,
    })
    .expect("ha_channel::wire() registers the IM channel hooks exactly once");
}

fn register_feishu_tools() {
    ha_core::tools::registry::register_external_tools(tools::feishu_dispatch_entries())
        .expect("ha_channel::wire() registers the Feishu tool handlers before registry freeze");
    ha_core::tools::registry::register_external_tool_definitions(tools::feishu::get_feishu_tools)
        .expect("ha_channel::wire() registers the Feishu tool schemas exactly once");
}
