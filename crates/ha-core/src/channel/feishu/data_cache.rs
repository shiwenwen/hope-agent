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

struct CacheEntry {
    buffer: Vec<Option<Vec<u8>>>,
    // Retained so future GC logging can identify expired traces without
    // re-parsing payloads. Currently only consumed by tests.
    #[allow(dead_code)]
    trace_id: String,
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
    /// Malformed inputs (`sum == 0`, `seq >= sum`) are rejected silently.
    pub fn merge(
        &self,
        message_id: &str,
        sum: usize,
        seq: usize,
        trace_id: &str,
        data: Vec<u8>,
    ) -> Option<Vec<u8>> {
        if sum == 0 || seq >= sum {
            return None;
        }
        let mut guard = self.inner.lock().unwrap();
        let entry = guard.entry(message_id.to_string()).or_insert_with(|| {
            CacheEntry {
                buffer: vec![None; sum],
                trace_id: trace_id.to_string(),
                created_at: Instant::now(),
            }
        });
        // Defensive: if a stale entry exists with a different shard count,
        // treat the new arrival as the start of a fresh stream.
        if entry.buffer.len() != sum {
            *entry = CacheEntry {
                buffer: vec![None; sum],
                trace_id: trace_id.to_string(),
                created_at: Instant::now(),
            };
        }
        entry.buffer[seq] = Some(data);

        if entry.buffer.iter().all(|s| s.is_some()) {
            let removed = guard.remove(message_id).unwrap();
            let total: usize = removed
                .buffer
                .iter()
                .map(|b| b.as_ref().map(|v| v.len()).unwrap_or(0))
                .sum();
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
        let merged = c.merge("msg-1", 1, 0, "trace-1", b"hello".to_vec());
        assert_eq!(merged, Some(b"hello".to_vec()));
        assert_eq!(c.len(), 0); // entry removed on completion
    }

    #[tokio::test]
    async fn multi_shard_out_of_order() {
        let c = cache();
        // 3 shards, arrive in order 2, 0, 1
        assert_eq!(c.merge("m", 3, 2, "t", b"three".to_vec()), None);
        assert_eq!(c.merge("m", 3, 0, "t", b"one".to_vec()), None);
        let merged = c.merge("m", 3, 1, "t", b"two".to_vec());
        assert_eq!(merged, Some(b"onetwothree".to_vec()));
        assert_eq!(c.len(), 0);
    }

    #[tokio::test]
    async fn multi_shard_partial_held() {
        let c = cache();
        assert_eq!(c.merge("m", 2, 0, "t", b"a".to_vec()), None);
        assert_eq!(c.len(), 1);
    }

    #[tokio::test]
    async fn gc_evicts_old_entries() {
        let c = cache();
        c.merge("m", 2, 0, "t", b"a".to_vec());
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
        assert!(c.merge("m", 0, 0, "t", b"x".to_vec()).is_none());
        assert!(c.merge("m", 2, 5, "t", b"x".to_vec()).is_none());
        assert_eq!(c.len(), 0);
    }

    #[tokio::test]
    async fn duplicate_seq_overwrites() {
        let c = cache();
        c.merge("m", 2, 0, "t", b"first".to_vec());
        c.merge("m", 2, 0, "t", b"second".to_vec()); // overwrite
        let merged = c.merge("m", 2, 1, "t", b"end".to_vec());
        assert_eq!(merged, Some(b"secondend".to_vec()));
    }
}
