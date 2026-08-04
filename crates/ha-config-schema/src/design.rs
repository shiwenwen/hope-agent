//! Design Space configuration (`AppConfig.design`).

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
    /// 首页 / 涉图入口模型选择器的「上次使用」记忆。行为记忆非设置项（GUI 选择器
    /// 隐式更新，照 `default_system_id` 先例挂 config，跨会话一致）；弱引用，
    /// provider / 模型已删则消费端回退默认链。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_model: Option<crate::provider::ActiveModel>,
    // 后台任务（critique / 大纲等一次性调用）的模型不再由 design 自持覆盖，
    // 走统一 `function_models.automation`（见 design::run_design_task）。
    // 生成 / 涉图路径可被用户在 GUI 显式选择的模型覆盖（单模型、不降级）。
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
            last_model: None,
        }
    }
}
