//! Image tool configuration (`AppConfig.image`).

use serde::{Deserialize, Serialize};

/// Default maximum number of images per single tool call.
const DEFAULT_MAX_IMAGES: usize = 10;

/// Persistent image tool configuration, stored in config.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageToolConfig {
    /// Maximum number of images per single tool call
    #[serde(default = "default_max_images")]
    pub max_images: usize,
}

fn default_max_images() -> usize {
    DEFAULT_MAX_IMAGES
}

impl Default for ImageToolConfig {
    fn default() -> Self {
        Self {
            max_images: DEFAULT_MAX_IMAGES,
        }
    }
}
