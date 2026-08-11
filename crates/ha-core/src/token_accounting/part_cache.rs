use std::num::NonZeroUsize;
use std::sync::Mutex;

use lru::LruCache;

use super::TokenizerId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PartCacheKind {
    Text,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PartCacheKey {
    tokenizer_id: TokenizerId,
    registry_version: u32,
    kind: PartCacheKind,
    digest: [u8; 32],
    bytes: usize,
}

#[derive(Debug)]
pub struct TokenizedPartCache {
    entries: Mutex<LruCache<PartCacheKey, u64>>,
}

impl Default for TokenizedPartCache {
    fn default() -> Self {
        Self {
            entries: Mutex::new(LruCache::new(
                NonZeroUsize::new(2_048).expect("token part cache size is non-zero"),
            )),
        }
    }
}

impl TokenizedPartCache {
    pub fn get(
        &self,
        tokenizer_id: TokenizerId,
        registry_version: u32,
        kind: PartCacheKind,
        bytes: &[u8],
    ) -> Option<u64> {
        let key = cache_key(tokenizer_id, registry_version, kind, bytes);
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&key)
            .copied()
    }

    pub fn put(
        &self,
        tokenizer_id: TokenizerId,
        registry_version: u32,
        kind: PartCacheKind,
        bytes: &[u8],
        tokens: u64,
    ) {
        let key = cache_key(tokenizer_id, registry_version, kind, bytes);
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .put(key, tokens);
    }
}

fn cache_key(
    tokenizer_id: TokenizerId,
    registry_version: u32,
    kind: PartCacheKind,
    bytes: &[u8],
) -> PartCacheKey {
    PartCacheKey {
        tokenizer_id,
        registry_version,
        kind,
        digest: *blake3::hash(bytes).as_bytes(),
        bytes: bytes.len(),
    }
}
