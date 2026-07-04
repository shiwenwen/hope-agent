//! 设计空间（Design Space）子系统。
//!
//! agent 原生设计工作空间：自包含 HTML 产物 + 品牌设计系统 + 稳定预览 +
//! 可视化微调 + 一键导出。完整架构见 `docs/architecture/design-space.md`。
//!
//! **零 Tauri 依赖**：业务全在此，`src-tauri` / `ha-server` 只做薄壳。

pub mod db;
pub mod recipe;
pub mod renderer;
pub mod service;

pub use db::{DesignArtifact, DesignArtifactVersion, DesignProject, DesignSystemMeta};
pub use recipe::Recipe;
pub use renderer::{ArtifactKind, ArtifactParts};

use serde::{Deserialize, Serialize};

// ── Config（设置三件套，见 AGENTS.md 设置约定）──────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_auto_show")]
    pub auto_show: bool,
    #[serde(default)]
    pub default_system_id: Option<String>,
    #[serde(default)]
    pub auto_critique: bool,
    #[serde(default = "default_max_versions")]
    pub max_versions_per_artifact: i64,
    #[serde(default = "default_panel_width")]
    pub panel_width: u32,
    #[serde(default = "default_self_check")]
    pub self_check: bool,
}

fn default_enabled() -> bool {
    true
}
fn default_auto_show() -> bool {
    true
}
fn default_max_versions() -> i64 {
    50
}
fn default_panel_width() -> u32 {
    480
}
fn default_self_check() -> bool {
    true
}

impl Default for DesignConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            auto_show: default_auto_show(),
            default_system_id: None,
            auto_critique: false,
            max_versions_per_artifact: default_max_versions(),
            panel_width: default_panel_width(),
            self_check: default_self_check(),
        }
    }
}

/// 设计空间是否启用。
#[allow(dead_code)]
pub fn is_design_enabled() -> bool {
    crate::config::cached_config().design.enabled
}
