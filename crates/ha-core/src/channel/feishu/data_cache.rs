//! In-flight cache for sharded Feishu WS event payloads.
//!
//! When a single event JSON exceeds the per-frame cap, Feishu's gateway splits
//! it into N frames sharing the same `message_id` but with `seq` 0..N-1 and
//! `sum=N`. This cache buffers shards by `message_id` and returns the merged
//! bytes once all `sum` slots are filled. Mirrors the official SDK's
//! `data-cache.ts` (10s TTL, single-shot delivery, GC by background task).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// Magic number from node-sdk: shards live at most 10 seconds before being
/// dropped by the GC sweep. Real-world events arrive within sub-second; the
/// generous TTL just guards against pathological network reordering.
const ENTRY_TTL: Duration = Duration::from_secs(10);
const GC_INTERVAL: Duration = Duration::from_secs(10);

/// Resource caps that prevent a misbehaving / hostile gateway from
/// exhausting memory by opening many concurrent message streams or
/// advertising huge shard counts. Real Feishu events sit well below all
/// three.
const MAX_ENTRIES: usize = 1024;
const MAX_SUM: usize = 64;
const MAX_TOTAL_BYTES_PER_ENTRY: usize = 16 * 1024 * 1024;

struct CacheEntry {
    buffer: Vec<Option<Vec<u8>>>,
    created_at: Instant,
}

pub struct DataCache {
    inner: Mutex<HashMap<String, CacheEntry>>,
}

impl DataCache {
    /// Construct a cache plus a background GC task. The GC task holds a `Weak`
    /// reference and exits cleanly once the last `Arc<DataCache>` is dropped.
    pub fn new() -> Arc<Self> {
        let cache = Arc::new(Self {
            inner: Mutex::new(HashMap::new()),
        });
        let weak = Arc::downgrade(&cache);
        tokio::spawn(gc_loop(weak));
        cache
    }

    /// Add a shard. Returns `Some(bytes)` when the final shard arrives —
    /// caller should parse + dispatch + ack. Returns `None` while
    /// accumulating; caller should ack but not dispatch.
    ///
    /// Inputs that violate `sum == 0`, `seq >= sum`, `sum > MAX_SUM`, or
    /// would exceed `MAX_ENTRIES` / `MAX_TOTAL_BYTES_PER_ENTRY` are rejected
    /// (returns `None`, possibly evicting a partial entry).
    pub fn merge(
        &self,
        message_id: &str,
        sum: usize,
        seq: usize,
        data: Vec<u8>,
    ) -> Option<Vec<u8>> {
        if sum == 0 || seq >= sum || sum > MAX_SUM {
            return None;
        }
        let mut guard = self.inner.lock().unwrap();

        if !guard.contains_key(message_id) && guard.len() >= MAX_ENTRIES {
            return None;
        }

        let entry = guard
            .entry(message_id.to_string())
            .or_insert_with(|| CacheEntry {
                buffer: vec![None; sum],
                created_at: Instant::now(),
            });
        if entry.buffer.len() != sum {
            *entry = CacheEntry {
                buffer: vec![None; sum],
                created_at: Instant::now(),
            };
        }
        entry.buffer[seq] = Some(data);

        let total: usize = entry
            .buffer
            .iter()
            .map(|b| b.as_ref().map(|v| v.len()).unwrap_or(0))
            .sum();
        if total > MAX_TOTAL_BYTES_PER_ENTRY {
            guard.remove(message_id);
            return None;
        }

        if entry.buffer.iter().all(|s| s.is_some()) {
            let removed = guard.remove(message_id).unwrap();
            let mut out = Vec::with_capacity(total);
            for shard in removed.buffer {
                if let Some(bytes) = shard {
                    out.extend_from_slice(&bytes);
                }
            }
            return Some(out);
        }
        None
    }

    /// Remove entries older than `ENTRY_TTL`. Public to the module so the
    /// background loop and tests can both invoke it.
    fn gc_once(&self) {
        let mut guard = self.inner.lock().unwrap();
        let now = Instant::now();
        guard.retain(|_, e| now.duration_since(e.created_at) < ENTRY_TTL);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}

async fn gc_loop(weak: Weak<DataCache>) {
    let mut ticker = tokio::time::interval(GC_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ticker.tick().await; // consume immediate first tick
    loop {
        ticker.tick().await;
        let Some(cache) = weak.upgrade() else {
            return;
        };
        cache.gc_once();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache() -> Arc<DataCache> {
        // Tests can construct a cache without a runtime if they don't need GC;
        // we only invoke gc_once directly. tokio::spawn requires a runtime so
        // tests run under #[tokio::test].
        DataCache::new()
    }

    #[tokio::test]
    async fn single_shard_round_trip() {
        let c = cache();
        let merged = c.merge("msg-1", 1, 0, b"hello".to_vec());
        assert_eq!(merged, Some(b"hello".to_vec()));
        assert_eq!(c.len(), 0); // entry removed on completion
    }

    #[tokio::test]
    async fn multi_shard_out_of_order() {
        let c = cache();
        // 3 shards, arrive in order 2, 0, 1
        assert_eq!(c.merge("m", 3, 2, b"three".to_vec()), None);
        assert_eq!(c.merge("m", 3, 0, b"one".to_vec()), None);
        let merged = c.merge("m", 3, 1, b"two".to_vec());
        assert_eq!(merged, Some(b"onetwothree".to_vec()));
        assert_eq!(c.len(), 0);
    }

    #[tokio::test]
    async fn multi_shard_partial_held() {
        let c = cache();
        assert_eq!(c.merge("m", 2, 0, b"a".to_vec()), None);
        assert_eq!(c.len(), 1);
    }

    #[tokio::test]
    async fn gc_evicts_old_entries() {
        let c = cache();
        c.merge("m", 2, 0, b"a".to_vec());
        assert_eq!(c.len(), 1);
        // Force the entry's created_at to look ancient.
        {
            let mut g = c.inner.lock().unwrap();
            g.get_mut("m").unwrap().created_at = Instant::now() - Duration::from_secs(20);
        }
        c.gc_once();
        assert_eq!(c.len(), 0);
    }

    #[tokio::test]
    async fn malformed_input_rejected() {
        let c = cache();
        assert!(c.merge("m", 0, 0, b"x".to_vec()).is_none());
        assert!(c.merge("m", 2, 5, b"x".to_vec()).is_none());
        assert_eq!(c.len(), 0);
    }

    #[tokio::test]
    async fn duplicate_seq_overwrites() {
        let c = cache();
        c.merge("m", 2, 0, b"first".to_vec());
        c.merge("m", 2, 0, b"second".to_vec()); // overwrite
        let merged = c.merge("m", 2, 1, b"end".to_vec());
        assert_eq!(merged, Some(b"secondend".to_vec()));
    }

    #[tokio::test]
    async fn rejects_sum_above_max() {
        let c = cache();
        assert!(c.merge("m", MAX_SUM + 1, 0, b"x".to_vec()).is_none());
        assert_eq!(c.len(), 0);
    }

    #[tokio::test]
    async fn rejects_when_entries_at_cap() {
        let c = cache();
        for i in 0..MAX_ENTRIES {
            assert_eq!(c.merge(&format!("m{i}"), 2, 0, vec![0u8; 1]), None);
        }
        assert_eq!(c.len(), MAX_ENTRIES);
        // New message_id refused; existing one still accepts shards.
        assert!(c.merge("overflow", 2, 0, b"x".to_vec()).is_none());
        assert_eq!(c.len(), MAX_ENTRIES);
        assert_eq!(c.merge("m0", 2, 1, vec![0u8; 1]).map(|v| v.len()), Some(2));
    }

    #[tokio::test]
    async fn evicts_when_payload_exceeds_per_entry_cap() {
        let c = cache();
        // First shard well within the cap.
        assert!(c
            .merge("m", 2, 0, vec![0u8; MAX_TOTAL_BYTES_PER_ENTRY / 2])
            .is_none());
        assert_eq!(c.len(), 1);
        // Second shard pushes total above the cap → evicted.
        assert!(c
            .merge("m", 2, 1, vec![0u8; MAX_TOTAL_BYTES_PER_ENTRY])
            .is_none());
        assert_eq!(c.len(), 0);
    }
}
