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
