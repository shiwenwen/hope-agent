use super::types::*;

/// Sanitize a user query for FTS5 MATCH syntax.
/// Wraps each word in double quotes to treat them as literal terms.
pub(crate) fn sanitize_fts_query(query: &str) -> String {
    let terms: Vec<String> = query
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .map(|w| {
            // Remove FTS5 special chars
            let clean: String = w.chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-').collect();
            if clean.is_empty() {
                String::new()
            } else {
                format!("\"{}\"", clean)
            }
        })
        .filter(|s| !s.is_empty())
        .collect();

    if terms.is_empty() {
        // Fallback: match everything if query is empty/invalid
        "\"*\"".to_string()
    } else {
        terms.join(" OR ")
    }
}

/// Load dedup thresholds from config.json, falling back to defaults.
pub fn load_dedup_config() -> DedupConfig {
    crate::provider::load_store()
        .map(|s| s.dedup)
        .unwrap_or_default()
}

/// Load global extract config from config.json.
pub fn load_extract_config() -> MemoryExtractConfig {
    crate::provider::load_store()
        .map(|s| s.memory_extract)
        .unwrap_or_default()
}

/// Load hybrid search config from config.json.
pub fn load_hybrid_search_config() -> HybridSearchConfig {
    crate::provider::load_store()
        .map(|s| s.hybrid_search)
        .unwrap_or_default()
}

/// Load temporal decay config from config.json.
pub fn load_temporal_decay_config() -> TemporalDecayConfig {
    crate::provider::load_store()
        .map(|s| s.temporal_decay)
        .unwrap_or_default()
}

/// Load MMR config from config.json.
pub fn load_mmr_config() -> MmrConfig {
    crate::provider::load_store()
        .map(|s| s.mmr)
        .unwrap_or_default()
}

/// Load multimodal config from config.json.
pub fn load_multimodal_config() -> MultimodalConfig {
    crate::provider::load_store()
        .map(|s| s.multimodal)
        .unwrap_or_default()
}

/// Load embedding cache config from config.json.
pub fn load_embedding_cache_config() -> EmbeddingCacheConfig {
    crate::provider::load_store()
        .map(|s| s.embedding_cache)
        .unwrap_or_default()
}

/// Extract keywords from a query, filtering stopwords for better FTS matching.
/// Supports English and Chinese stopwords.
pub(crate) fn expand_query(query: &str) -> String {
    use std::collections::HashSet;

    let stopwords_en: HashSet<&str> = [
        "the", "a", "an", "is", "are", "was", "were", "in", "on", "at", "to", "for",
        "of", "with", "by", "from", "this", "that", "it", "i", "you", "we", "they",
        "my", "your", "do", "does", "how", "what", "where", "when", "why", "which",
        "can", "could", "would", "should", "have", "has", "had", "be", "been", "being",
        "not", "no", "or", "and", "but", "if", "so", "as", "than", "too", "very",
        "about", "up", "out", "just", "also", "more", "some", "any", "all", "each",
    ].into_iter().collect();

    let stopwords_zh: HashSet<&str> = [
        "的", "了", "在", "是", "我", "有", "和", "���", "不", "人", "都", "一",
        "一个", "上", "也", "��", "到", "说", "要", "去", "你", "会", "着",
        "没有", "看", "好", "自己", "这", "那", "他", "她", "它", "们",
        "吗", "吧", "呢", "啊", "把", "被", "从", "对", "让", "给",
    ].into_iter().collect();

    let terms: Vec<String> = query
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .filter(|w| {
            let lower = w.to_lowercase();
            lower.len() > 1
                && !stopwords_en.contains(lower.as_str())
                && !stopwords_zh.contains(*w)
        })
        .map(|w| {
            let clean: String = w.chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-').collect();
            if clean.is_empty() {
                String::new()
            } else {
                format!("\"{}\"", clean)
            }
        })
        .filter(|s| !s.is_empty())
        .collect();

    if terms.is_empty() {
        // Fallback to original sanitize
        sanitize_fts_query(query)
    } else {
        terms.join(" OR ")
    }
}
