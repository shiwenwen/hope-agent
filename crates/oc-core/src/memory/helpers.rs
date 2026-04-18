use super::types::*;

/// Sanitize a user query for FTS5 MATCH syntax.
/// Wraps each word in double quotes to treat them as literal terms.
/// Returns `None` when the query is empty / entirely filtered — callers should
/// return an empty result set instead of running an unbounded full-index scan.
pub(crate) fn sanitize_fts_query(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .map(|w| {
            // Remove FTS5 special chars
            let clean: String = w
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if clean.is_empty() {
                String::new()
            } else {
                format!("\"{}\"", clean)
            }
        })
        .filter(|s| !s.is_empty())
        .collect();

    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
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

/// Extract keywords from a query, filtering stopwords for better FTS matching.
/// Supports English and Chinese stopwords. Returns `None` when nothing usable
/// remains after stopword stripping — callers should short-circuit to an empty
/// result.
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

    let terms: Vec<String> = query
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .filter(|w| {
            let lower = w.to_lowercase();
            lower.len() > 1 && !stopwords_en.contains(lower.as_str()) && !stopwords_zh.contains(*w)
        })
        .map(|w| {
            let clean: String = w
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if clean.is_empty() {
                String::new()
            } else {
                format!("\"{}\"", clean)
            }
        })
        .filter(|s| !s.is_empty())
        .collect();

    if terms.is_empty() {
        sanitize_fts_query(query)
    } else {
        Some(terms.join(" OR "))
    }
}
