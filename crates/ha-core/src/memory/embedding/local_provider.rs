use anyhow::{Context, Result};
use std::sync::Mutex;

use super::utils::l2_normalize;
use crate::memory::traits::EmbeddingProvider;

// ── Local Embedding Provider ────────────────────────────────────

/// Local ONNX-based embedding provider using fastembed-rs.
pub struct LocalEmbeddingProvider {
    model: Mutex<fastembed::TextEmbedding>,
    model_id: String,
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
            model_id: model_id.to_string(),
            dims,
        })
    }

    fn record_local_usage(&self, operation: &'static str, text_count: usize, duration_ms: u64) {
        let mut event =
            crate::model_usage::ModelUsageEvent::new(crate::model_usage::KIND_EMBEDDING);
        event.operation = Some(operation.to_string());
        event.source = Some("embedding".to_string());
        event.provider_name = Some("local".to_string());
        event.model_id = Some(self.model_id.clone());
        event.duration_ms = Some(duration_ms);
        event.metadata = Some(serde_json::json!({
            "text_count": text_count,
            "dimensions": self.dims,
        }));
        crate::model_usage::record_model_usage_best_effort(event);
    }
}

impl EmbeddingProvider for LocalEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let started = std::time::Instant::now();
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
        self.record_local_usage("embedding.local", 1, started.elapsed().as_millis() as u64);
        Ok(vec)
    }

    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let started = std::time::Instant::now();
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
        self.record_local_usage(
            "embedding.local_batch",
            texts.len(),
            started.elapsed().as_millis() as u64,
        );
        Ok(results)
    }

    fn dimensions(&self) -> u32 {
        self.dims
    }
}
