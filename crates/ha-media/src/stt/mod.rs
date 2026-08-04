//! STT 执行机器（provider 协议实现 / 流式会话 / failover 转写）。
//! 配置面（crud / 链解析 / 类型 / 本地 catalog）在 `ha_core::stt`。

pub mod engine;
pub mod providers;
pub mod session;

pub use ha_core::stt::*;
pub use session::SttSessionManager;
