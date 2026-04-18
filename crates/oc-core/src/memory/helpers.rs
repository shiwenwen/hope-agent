use super::types::*;

/// Clean each word (keep alphanumeric / `_` / `-`), wrap non-empty results in
/// double quotes for FTS5 MATCH literal matching, and OR-join them. Returns
/// `None` when no usable term remains — callers short-circuit to an empty
/// result set instead of running an unbounded full-index scan.
fn format_fts_terms<'a, I: Iterator<Item = &'a str>>(words: I) -> Option<String> {
    let terms: Vec<String> = words
        .filter_map(|w| {
            let clean: String = w
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            (!clean.is_empty()).then(|| format!("\"{}\"", clean))
        })
        .collect();

    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

/// Sanitize a user query for FTS5 MATCH syntax (no stopword filtering).
pub(crate) fn sanitize_fts_query(query: &str) -> Option<String> {
    format_fts_terms(query.split_whitespace())
}

/// Load dedup thresholds from config.json, falling back to defaults.
pub fn load_dedup_config() -> DedupConfig {
    crate::config::cached_config().dedup.clone()
}

/// Load LLM memory selection config from config.json.
pub fn load_memory_selection_config() -> MemorySelectionConfig {
    crate::config::cached_config().memory_selection.clone()
}

/// Load global extract config from config.json.
pub fn load_extract_config() -> MemoryExtractConfig {
    crate::config::cached_config().memory_extract.clone()
}

/// Load hybrid search config from config.json.
pub fn load_hybrid_search_config() -> HybridSearchConfig {
    crate::config::cached_config().hybrid_search.clone()
}

/// Load temporal decay config from config.json.
pub fn load_temporal_decay_config() -> TemporalDecayConfig {
    crate::config::cached_config().temporal_decay.clone()
}

/// Load MMR config from config.json.
pub fn load_mmr_config() -> MmrConfig {
    crate::config::cached_config().mmr.clone()
}

/// Load multimodal config from config.json.
pub fn load_multimodal_config() -> MultimodalConfig {
    crate::config::cached_config().multimodal.clone()
}

/// Load embedding cache config from config.json.
pub fn load_embedding_cache_config() -> EmbeddingCacheConfig {
    crate::config::cached_config().embedding_cache.clone()
}

/// Extract keywords from a query, filtering English + Chinese stopwords for
/// better FTS matching. Falls back to `sanitize_fts_query(query)` when every
/// word is a stopword so rare legitimate single-stopword queries still match.
pub(crate) fn expand_query(query: &str) -> Option<String> {
    use std::collections::HashSet;

    let stopwords_en: HashSet<&str> = [
        "the", "a", "an", "is", "are", "was", "were", "in", "on", "at", "to", "for", "of", "with",
        "by", "from", "this", "that", "it", "i", "you", "we", "they", "my", "your", "do", "does",
        "how", "what", "where", "when", "why", "which", "can", "could", "would", "should", "have",
        "has", "had", "be", "been", "being", "not", "no", "or", "and", "but", "if", "so", "as",
        "than", "too", "very", "about", "up", "out", "just", "also", "more", "some", "any", "all",
        "each",
    ]
    .into_iter()
    .collect();

    let stopwords_zh: HashSet<&str> = [
        "的", "了", "在", "是", "我", "有", "和", "的", "不", "人", "都", "一", "一个", "上", "也",
        "了", "到", "说", "要", "去", "你", "会", "着", "没有", "看", "好", "自己", "这", "那",
        "他", "她", "它", "们", "吗", "吧", "呢", "啊", "把", "被", "从", "对", "让", "给",
    ]
    .into_iter()
    .collect();

    format_fts_terms(query.split_whitespace().filter(|w| {
        let lower = w.to_lowercase();
        lower.len() > 1 && !stopwords_en.contains(lower.as_str()) && !stopwords_zh.contains(*w)
    }))
    .or_else(|| sanitize_fts_query(query))
}
