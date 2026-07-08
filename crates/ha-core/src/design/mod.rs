//! 设计空间（Design Space）子系统。
//!
//! agent 原生设计工作空间：自包含 HTML 产物 + 品牌设计系统 + 稳定预览 +
//! 可视化微调 + 一键导出。完整架构见 `docs/architecture/design-space.md`。
//!
//! **零 Tauri 依赖**：业务全在此，`src-tauri` / `ha-server` 只做薄壳。

pub mod audio;
mod brands;
pub mod compile;
pub mod critique;
pub mod db;
pub mod design_md;
pub mod export;
pub mod extract;
pub mod generate;
pub mod image;
pub mod patch;
pub mod recipe;
pub mod render_native;
pub mod renderer;
pub mod selfcheck;
pub mod service;
pub mod kit;
pub mod system;
pub mod threads;
pub mod token_export;
pub mod vision;

pub use critique::CritiqueResult;
pub use db::{
    DesignArtifact, DesignArtifactVersion, DesignCodeBinding, DesignComment, DesignProject,
    DesignSystemMeta,
};
pub use recipe::Recipe;
pub use renderer::{ArtifactKind, ArtifactParts};
pub use system::DesignSystemFull;
pub use threads::DesignChatThread;

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
    /// 反向提取（截图/设计图）读取的图片文件大小上限（MB）。`0` = 不限。默认 24。
    #[serde(default = "default_max_extract_image_mb")]
    pub max_extract_image_mb: u32,
    /// 导出栅格化倍率（清晰度）。越大越清晰、文件越大。读时钳 `[1,4]`。默认 2（retina）。
    #[serde(default = "default_export_scale")]
    pub export_scale: u32,
    /// PDF 导出的 JPEG 压缩质量（1–100）。读时钳 `[40,100]`。默认 92。
    #[serde(default = "default_export_jpeg_quality")]
    pub export_jpeg_quality: u32,
    /// 反向提取截图的专用视觉模型（`providerId:modelId`）。空 = 复用当前激活模型。
    #[serde(default)]
    pub extract_vision_model: Option<String>,
    /// 质量评审的专用模型（`providerId:modelId`）。空 = 复用默认分析 agent 模型。
    #[serde(default)]
    pub critique_model: Option<String>,
}

/// 导出倍率安全钳（`[1,4]`）。
pub fn clamp_export_scale(v: u32) -> u32 {
    v.clamp(1, 4)
}

/// 导出 JPEG 质量安全钳（`[40,100]`）。
pub fn clamp_export_jpeg_quality(v: u32) -> u32 {
    v.clamp(40, 100)
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
fn default_max_extract_image_mb() -> u32 {
    24
}
fn default_export_scale() -> u32 {
    2
}
fn default_export_jpeg_quality() -> u32 {
    92
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
            max_extract_image_mb: default_max_extract_image_mb(),
            export_scale: default_export_scale(),
            export_jpeg_quality: default_export_jpeg_quality(),
            extract_vision_model: None,
            critique_model: None,
        }
    }
}

/// 设计空间是否启用。
#[allow(dead_code)]
pub fn is_design_enabled() -> bool {
    crate::config::cached_config().design.enabled
}
