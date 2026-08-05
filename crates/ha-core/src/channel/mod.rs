//! IM 渠道的**台账与契约**——机器（12 个插件 / worker / 媒体管道 / 账号生命
//! 周期）已随阶段 5 第五刀迁出 `ha-channel`，此处只留 kernel 必须自持的部分。
//!
//! # 为什么这四个留 kernel
//!
//! - [`db`]（`ChannelDB` + `ChannelConversation`）——它持 [`crate::session::SessionDB`]
//!   的连接直接读写 `channel_conversations`，而那张表在 kernel 自己的
//!   `sessions.db` 里；更重要的是它是 AGENTS 红线「一 chat ↔ 一 session
//!   **双向 1:1**，读写一律走 `channel/db.rs` helper、禁止直接写表」的**执行点**。
//!   把它交给特征 crate 等于把红线的守门人放到装配之后。
//! - [`cancel`]（`ChannelCancelRegistry`）——纯内存取消注册表，绑着
//!   `CHANNEL_CANCELS` 全局与 `AppState.channel_cancels`，`app_init` 还有一条
//!   `ptr_eq_lock` 不变量断言两者共享同一 `Arc`。同 cron 那刀把
//!   `cron/cancel.rs` 留 kernel 的分法。
//! - [`types`] / [`config`]——`AppConfig` 可达的 wire 类型，定义处本就在
//!   ha-config-schema，这里只是转发。
//! - [`traits`]（`ChannelPlugin`）/ [`registry`]（`ChannelRegistry`）——**契约与
//!   持有者，不是机器**：trait 本身零实现，registry 只是按 `ChannelId` 存放
//!   `Arc<dyn ChannelPlugin>` 并转发 start/stop/restart。12 个插件实现随
//!   ha-channel 上浮，靠 [`crate::channel_hooks`] 的插件注册槽在 `app_init`
//!   建好 registry 之后装入。这样 `CHANNEL_REGISTRY` 全局、`AppState`
//!   字段与壳层的 **registry** 调用点（src-tauri 9 处 / ha-server 1 处 /
//!   ha-cron 1 处）**一处未改**。
//!
//! 反向回调（撤窗 / IM 实时镜像 / 账号开关）走 [`crate::channel_hooks`]。

pub mod cancel;
pub mod config;
pub mod db;
pub mod registry;
pub mod traits;
pub mod types;

pub use cancel::ChannelCancelRegistry;
pub use config::ChannelStoreConfig;
// WS8 的 KB 闸门已随 `effective_kb_access` 归位 `crate::knowledge::access`
// （它是 KB 访问裁决的一部分、不是渠道行为）。**原路径兼容再导出**——
// `ha_core::channel::im_kb_access_allowed` 迁移前是公开 API。
pub use crate::knowledge::im_kb_access_allowed;
pub use db::ChannelDB;
pub use registry::ChannelRegistry;
pub use traits::{ChannelPlugin, ChannelReplyStream};
pub use types::*;
