use anyhow::{Context, Result};
use std::sync::Mutex;

use super::utils::l2_normalize;
use crate::memory::traits::EmbeddingProvider;

// ── Local Embedding Provider ────────────────────────────────────

/// Local ONNX-based embedding provider using fastembed-rs.
pub struct LocalEmbeddingProvider {
    model: Mutex<fastembed::TextEmbedding>,
    dims: u32,
}

impl LocalEmbeddingProvider {
    /// Initialize with a model ID from the built-in presets.
    pub fn new(model_id: &str) -> Result<Self> {
        let (fe_model, dims) = match model_id {
            "bge-small-zh-v1.5" => (fastembed::EmbeddingModel::BGESmallZHV15, 384),
            "multilingual-e5-small" => (fastembed::EmbeddingModel::MultilingualE5Small, 384),
            "bge-large-en-v1.5" => (fastembed::EmbeddingModel::BGELargeENV15, 1024),
            _ => (fastembed::EmbeddingModel::BGESmallENV15, 384), // default
        };

        let cache_dir = crate::paths::models_cache_dir()?;

        let model = fastembed::TextEmbedding::try_new(
            fastembed::InitOptions::new(fe_model)
                .with_cache_dir(cache_dir)
                .with_show_download_progress(false),
        )
        .context("Failed to initialize local embedding model")?;

        Ok(Self {
            model: Mutex::new(model),
            dims,
        })
    }
}

impl EmbeddingProvider for LocalEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut model = self
            .model
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let results = model
            .embed(vec![text.to_string()], None)
            .map_err(|e| anyhow::anyhow!("Local embedding failed: {}", e))?;
        let mut vec = results
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding result"))?;
        l2_normalize(&mut vec);
        Ok(vec)
    }

    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut model = self
            .model
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut results = model
            .embed(texts.to_vec(), None)
            .map_err(|e| anyhow::anyhow!("Local batch embedding failed: {}", e))?;
        for vec in &mut results {
            l2_normalize(vec);
        }
        Ok(results)
    }

    fn dimensions(&self) -> u32 {
        self.dims
    }
}
