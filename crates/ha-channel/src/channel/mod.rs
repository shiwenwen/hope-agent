//! IM 渠道**机器**：12 个插件 + worker 分发 + 媒体管道 + 账号生命周期。
//!
//! 台账与契约留在 kernel 的 [`ha_core::channel`]（`db` / `types` / `cancel` /
//! `config`），见 crate 根文档。

pub mod accounts;
pub mod attach_sync;
pub mod discord;
pub mod feishu;
pub mod googlechat;
pub mod imessage;
pub mod inbound_media_common;
pub mod irc;
pub mod line;
pub mod media_helpers;
pub mod process_manager;
pub mod qqbot;
pub mod rate_limit;
pub mod signal;
pub mod slack;
pub mod start_watchdog;
pub mod telegram;
pub mod webhook_server;
pub mod wechat;
pub mod whatsapp;
pub mod worker;
pub mod ws;

pub use ha_core::channel::{ChannelPlugin, ChannelRegistry};

// 台账 / 契约在 kernel——原路径兼容再导出，让 `ha_channel::channel::<X>` 与
// 迁移前的 `ha_core::channel::<X>` 符号集尽量一致。
pub use ha_core::channel::types::*;
pub use ha_core::channel::{cancel, config, db, types};
pub use ha_core::channel::{ChannelCancelRegistry, ChannelDB, ChannelStoreConfig};
