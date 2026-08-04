//! Hope Agent 配置 wire 类型：`AppConfig` 及其全部传递闭包。
//!
//! 模块路径**镜像 ha-core**（`memory` / `config` / `tools::web_search` …），
//! 因此被搬入的代码里 `crate::memory::X` 这类内部引用原样成立；ha-core 侧
//! 各子系统 `pub use ha_config_schema::<mod>::X;` 原地再导出，调用方路径不变。
//!
//! 根部全量再导出 ha-base（与 ha-core 的 `pub use ha_base::*` 同构），使
//! `crate::default_true` / `crate::paths::is_valid_pet_id` 等 ha-base 引用
//! 在本 crate 内继续解析。

pub use ha_base::*;

pub mod acp_control;
pub mod awareness;
pub mod browser;
pub mod channel;
pub mod config;
pub mod context_compact;
pub mod design;
pub mod hooks;
pub mod issue_reporting;
pub mod knowledge;
pub mod mcp;
pub mod media_gen;
pub mod memory;
pub mod permission;
pub mod pet;
pub mod provider;
pub mod session_title;
pub mod skills;
pub mod sprite;
pub mod stt;
pub mod tools;
pub mod updater;
