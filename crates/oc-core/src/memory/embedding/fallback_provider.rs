use anyhow::Result;
use std::sync::Arc;

use crate::memory::traits::{EmbeddingProvider, MultimodalInput};

// ── Fallback Embedding Provider ─────────────────────────────────

/// Provider wrapper that falls back to a secondary provider on error.
pub struct FallbackEmbeddingProvider {
    pub(super) primary: Arc<dyn EmbeddingProvider>,
    pub(super) fallback: Arc<dyn EmbeddingProvider>,
}

impl EmbeddingProvider for FallbackEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        match self.primary.embed(text) {
            Ok(v) => Ok(v),
            Err(e) => {
                if let Some(logger) = crate::get_logger() {
                    logger.log(
                        "warn",
                        "memory",
                        "embedding::fallback",
                        &format!("Primary embed failed, trying fallback: {}", e),
                        None,
                        None,
                        None,
                    );
                }
                self.fallback.embed(text)
            }
        }
    }

    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        match self.primary.embed_batch(texts) {
            Ok(v) => Ok(v),
            Err(e) => {
                if let Some(logger) = crate::get_logger() {
                    logger.log(
                        "warn",
                        "memory",
                        "embedding::fallback",
                        &format!("Primary embed_batch failed, trying fallback: {}", e),
                        None,
                        None,
                        None,
                    );
                }
                self.fallback.embed_batch(texts)
            }
        }
    }

    fn dimensions(&self) -> u32 {
        self.primary.dimensions()
    }

    fn supports_multimodal(&self) -> bool {
        self.primary.supports_multimodal()
    }

    fn embed_multimodal(&self, input: &MultimodalInput) -> Result<Vec<f32>> {
        match self.primary.embed_multimodal(input) {
            Ok(v) => Ok(v),
            Err(e) => {
                if let Some(logger) = crate::get_logger() {
                    logger.log(
                        "warn",
                        "memory",
                        "embedding::fallback",
                        &format!("Primary embed_multimodal failed, trying fallback: {}", e),
                        None,
                        None,
                        None,
                    );
                }
                self.fallback.embed_multimodal(input)
            }
        }
    }

    fn supports_batch_api(&self) -> bool {
        self.primary.supports_batch_api()
    }

    fn embed_batch_async(
        &self,
        texts: &[(String, String)],
    ) -> Result<std::collections::HashMap<String, Vec<f32>>> {
        match self.primary.embed_batch_async(texts) {
            Ok(v) => Ok(v),
            Err(e) => {
                if let Some(logger) = crate::get_logger() {
                    logger.log(
                        "warn",
                        "memory",
                        "embedding::fallback",
                        &format!("Primary embed_batch_async failed, trying fallback: {}", e),
                        None,
                        None,
                        None,
                    );
                }
                self.fallback.embed_batch_async(texts)
            }
        }
    }
}
