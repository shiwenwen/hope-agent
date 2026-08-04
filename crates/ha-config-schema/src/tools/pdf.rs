//! PDF tool configuration (`AppConfig.pdf`).

use serde::{Deserialize, Serialize};

/// Default maximum number of PDFs per single tool call.
const DEFAULT_MAX_PDFS: usize = 5;
/// Default maximum pages to render in vision mode.
const DEFAULT_MAX_VISION_PAGES: usize = 10;

/// Persistent PDF tool configuration, stored in config.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfToolConfig {
    /// Maximum number of PDFs per single tool call
    #[serde(default = "default_max_pdfs")]
    pub max_pdfs: usize,
    /// Maximum pages to render in vision mode
    #[serde(default = "default_max_vision_pages")]
    pub max_vision_pages: usize,
}

fn default_max_pdfs() -> usize {
    DEFAULT_MAX_PDFS
}
fn default_max_vision_pages() -> usize {
    DEFAULT_MAX_VISION_PAGES
}

impl Default for PdfToolConfig {
    fn default() -> Self {
        Self {
            max_pdfs: DEFAULT_MAX_PDFS,
            max_vision_pages: DEFAULT_MAX_VISION_PAGES,
        }
    }
}
