//! Bounded in-memory cache with TTL evaluated at lookup time.
//!
//! Two call sites in the codebase wanted a bounded `HashMap` that auto-
//! expires entries:
//!
//! - [`crate::permission::judge`] — process-wide judge verdict cache
//!   (capacity 256, TTL 60 s). Repeat tool calls within a chat turn
//!   shouldn't trigger a fresh ~5 s LLM call.
//! - [`crate::agent::active_memory`] — per-agent recall cache (capacity
//!   32, TTL configurable). Re-asking the exact same user phrasing
//!   inside the cache window reuses the previous recall.
//!
//! Both implementations independently rolled the same `Mutex<HashMap<K,
//! (V, Instant)>>` + capacity sweep + lookup-time expiry pattern. This
//! module provides a single `TtlCache<K, V>` so future cache sites can
//! reuse it instead of forking a third copy.
//!
//! ## Eviction policy
//!
//! - `put` checks capacity; if at the cap, drops the single entry with
//!   the oldest `created_at` (O(n) scan, n ≤ cap, called at most once
//!   per `put`).
//! - `get` checks the supplied TTL; if elapsed, removes the entry lazily
//!   and returns `None`.
//! - There is **no background sweep** — callers don't need a runtime.
//!   Expired entries that are never looked up sit until eviction-on-put.
//!   With small caps (≤ 256) and lookup-driven workloads this is fine.
//!
//! ## TTL at lookup time
//!
//! The TTL is **not** stored on the entry. Each `get` call passes its
//! own `ttl: Duration`, comparing against `created_at.elapsed()`. This
//! lets config-driven TTL changes take effect immediately without
//! restamping existing entries — useful for [`active_memory`](crate::agent::active_memory)
//! where users edit `cache_ttl_secs` in settings, and harmless for
//! [`judge`](crate::permission::judge) which always passes the same
//! `JUDGE_CACHE_TTL`.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct TtlCache<K: Eq + Hash + Clone, V: Clone> {
    capacity: usize,
    inner: Mutex<HashMap<K, Entry<V>>>,
}

struct Entry<V: Clone> {
    value: V,
    created_at: Instant,
}

impl<K: Eq + Hash + Clone, V: Clone> TtlCache<K, V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Look up `key`. If present and not yet expired (per `ttl` measured
    /// from `created_at`), returns a clone of the value. Expired entries
    /// are removed in-place. Returns `None` for both "missing" and
    /// "expired and removed" cases — callers that need to distinguish
    /// "no entry" from "tombstone" should encode that in the value type
    /// (e.g. `Option<T>` so a cached `None` survives lookup).
    pub fn get(&self, key: &K, ttl: Duration) -> Option<V> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = guard.get(key) {
            if entry.created_at.elapsed() <= ttl {
                return Some(entry.value.clone());
            }
            guard.remove(key);
        }
        None
    }

    /// Insert `(key, value)`. If the cache is at capacity, evicts the
    /// single entry with the oldest `created_at` first (the LRU-by-age
    /// policy — not LRU-by-access, since `get` doesn't touch the entry).
    pub fn put(&self, key: K, value: V) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.len() >= self.capacity {
            if let Some(oldest_key) = guard
                .iter()
                .min_by_key(|(_, e)| e.created_at)
                .map(|(k, _)| k.clone())
            {
                guard.remove(&oldest_key);
            }
        }
        guard.insert(
            key,
            Entry {
                value,
                created_at: Instant::now(),
            },
        );
    }

    /// Drop every entry. Useful when an agent-config change invalidates
    /// the entire cache scope (e.g. switching agents in active_memory).
    pub fn clear(&self) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Current entry count, observable in tests. Includes expired entries
    /// that haven't been swept yet — callers shouldn't rely on this for
    /// correctness, only as a soft observability signal.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_then_get_within_ttl_returns_value() {
        let c: TtlCache<u64, String> = TtlCache::new(8);
        c.put(1, "a".into());
        assert_eq!(c.get(&1, Duration::from_secs(60)), Some("a".into()));
    }

    #[test]
    fn get_with_zero_ttl_treats_immediate_lookup_as_expired() {
        let c: TtlCache<u64, String> = TtlCache::new(8);
        c.put(1, "a".into());
        // ttl=0 means "any age fails". Entry is also removed lazily.
        assert_eq!(c.get(&1, Duration::from_nanos(0)), None);
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn put_evicts_oldest_when_at_capacity() {
        let c: TtlCache<u64, &'static str> = TtlCache::new(2);
        c.put(1, "first");
        std::thread::sleep(Duration::from_millis(2));
        c.put(2, "second");
        std::thread::sleep(Duration::from_millis(2));
        c.put(3, "third"); // should evict key 1 (oldest)
        assert_eq!(c.len(), 2);
        assert!(c.get(&1, Duration::from_secs(60)).is_none());
        assert_eq!(c.get(&2, Duration::from_secs(60)), Some("second"));
        assert_eq!(c.get(&3, Duration::from_secs(60)), Some("third"));
    }

    #[test]
    fn cached_none_value_survives_lookup() {
        // Sanity: callers that store Option<T> for "we computed but the
        // answer was empty" still get back Some(None) on hit, distinct
        // from the cache-miss None.
        let c: TtlCache<u64, Option<String>> = TtlCache::new(4);
        c.put(1, None);
        assert_eq!(c.get(&1, Duration::from_secs(60)), Some(None));
    }

    #[test]
    fn clear_drops_all_entries() {
        let c: TtlCache<u64, &'static str> = TtlCache::new(4);
        c.put(1, "a");
        c.put(2, "b");
        c.clear();
        assert_eq!(c.len(), 0);
        assert!(c.get(&1, Duration::from_secs(60)).is_none());
    }
}
