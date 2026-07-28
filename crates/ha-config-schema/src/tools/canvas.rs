//! Canvas configuration (`AppConfig.canvas`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_auto_show")]
    pub auto_show: bool,
    #[serde(default = "default_content_type")]
    pub default_content_type: String,
    #[serde(default = "default_max_projects")]
    pub max_projects: u32,
    #[serde(default = "default_max_versions")]
    pub max_versions_per_project: i64,
    #[serde(default = "default_panel_width")]
    pub panel_width: u32,
}

fn default_enabled() -> bool {
    true
}
fn default_auto_show() -> bool {
    true
}
fn default_content_type() -> String {
    "html".to_string()
}
fn default_max_projects() -> u32 {
    100
}
fn default_max_versions() -> i64 {
    50
}
fn default_panel_width() -> u32 {
    480
}

impl Default for CanvasConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            auto_show: default_auto_show(),
            default_content_type: default_content_type(),
            max_projects: default_max_projects(),
            max_versions_per_project: default_max_versions(),
            panel_width: default_panel_width(),
        }
    }
}
