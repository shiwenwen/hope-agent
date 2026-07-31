pub mod accounts;
pub mod attach_sync;
pub mod cancel;
pub mod config;
pub mod db;
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
pub mod registry;
pub mod signal;
pub mod slack;
pub mod start_watchdog;
pub mod telegram;
pub mod traits;
pub mod types;
pub mod webhook_server;
pub mod wechat;
pub mod whatsapp;
pub mod worker;
pub mod ws;

pub use cancel::ChannelCancelRegistry;
pub use config::ChannelStoreConfig;
// WS8 的 KB 闸门已随 `effective_kb_access` 归位 `crate::knowledge::access`
// （它是 KB 访问裁决的一部分、不是渠道行为；留在这里会让 ha-knowledge 反向
// 依赖 ha-channel）。**原路径兼容再导出**——`ha_core::channel::
// im_kb_access_allowed` 迁移前是公开 API，拆分方案的兼容承诺覆盖它；
// 方向上也不亏：channel 本就经 dispatcher 的 `ChannelKbContext` 依赖
// knowledge（单向、不成环），这行别名不新增任何 Cargo 依赖。
pub use crate::knowledge::im_kb_access_allowed;
pub use db::ChannelDB;
pub use registry::ChannelRegistry;
pub use traits::ChannelPlugin;
pub use types::*;
