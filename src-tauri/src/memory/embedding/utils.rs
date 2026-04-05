// ── Token Limit Management ──────────────────────────────────────

/// Maximum input tokens for known embedding models.
pub(crate) fn max_input_tokens(model: &str) -> usize {
    match model {
        "text-embedding-3-small" | "text-embedding-3-large" | "text-embedding-ada-002" => 8191,
        "gemini-embedding-001" | "text-embedding-004" => 2048,
        "gemini-embedding-2-preview" => 8192,
        "voyage-3" | "voyage-code-3" | "voyage-4-large" => 32000,
        "voyage-3-lite" => 16000,
        "mistral-embed" => 8192,
        "jina-embeddings-v3" => 8192,
        "embed-multilingual-v3.0" => 512,
        "nomic-embed-text" => 8192,
        m if m.starts_with("BAAI/bge") => 512,
        _ => 8192, // safe default
    }
}

/// Truncate texts that exceed the model's token limit (conservative: ~4 bytes/token).
pub(crate) fn truncate_for_model(texts: &[String], model: &str) -> Vec<String> {
    let max_bytes = max_input_tokens(model) * 4;
    texts
        .iter()
        .map(|t| {
            if t.len() > max_bytes {
                if let Some(logger) = crate::get_logger() {
                    logger.log(
                        "warn",
                        "memory",
                        "embedding::truncate",
                        &format!(
                            "Truncating text from {} to {} bytes for model {}",
                            t.len(),
                            max_bytes,
                            model
                        ),
                        None,
                        None,
                        None,
                    );
                }
                crate::truncate_utf8(t, max_bytes).to_string()
            } else {
                t.clone()
            }
        })
        .collect()
}

// ── L2 Vector Normalization ─────────────────────────────────────

/// L2-normalize an embedding vector in place for consistent cosine similarity.
pub(crate) fn l2_normalize(vec: &mut Vec<f32>) {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-12 {
        for v in vec.iter_mut() {
            *v /= norm;
        }
    }
}
