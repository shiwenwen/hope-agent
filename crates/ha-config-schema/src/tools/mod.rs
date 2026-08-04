//! `AppConfig` 下的工具类配置（`web_search` / `web_fetch` / `canvas` /
//! `image` / `pdf`），子模块路径镜像 `ha-core::tools::*`。
//!
//! 只放配置 wire 类型与 serde default helper；工具执行逻辑、schema 构建、
//! HTTP 调用全部留在 ha-core。

pub mod canvas;
pub mod image;
pub mod pdf;
pub mod web_fetch;
pub mod web_search;
