use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lru::LruCache;

/// Endpoint-shape capability cache. Only protocol support is cached here.
/// Authentication and transport failures are deliberately kept out of the
/// endpoint capability map. They use a separate, short-lived credential
/// fingerprint suppression so one bad profile cannot poison another profile.
#[derive(Debug)]
pub struct ProviderCountCapabilityCache {
    entries: Mutex<LruCache<String, bool>>,
    in_flight: Mutex<HashSet<String>>,
    profile_suppressions: Mutex<LruCache<String, Instant>>,
}

impl Default for ProviderCountCapabilityCache {
    fn default() -> Self {
        Self {
            entries: Mutex::new(LruCache::new(
                NonZeroUsize::new(128).expect("capability cache size is non-zero"),
            )),
            in_flight: Mutex::new(HashSet::new()),
            profile_suppressions: Mutex::new(LruCache::new(
                NonZeroUsize::new(256).expect("profile suppression cache size is non-zero"),
            )),
        }
    }
}

impl ProviderCountCapabilityCache {
    pub fn should_attempt(&self, key: &str) -> bool {
        !matches!(
            self.entries
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(key),
            Some(false)
        )
    }

    pub fn record_supported(&self, key: String) {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .put(key, true);
    }

    pub fn record_unsupported(&self, key: String) {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .put(key, false);
    }

    pub fn begin_attempt(&self, key: &str) -> Option<ProviderCountAttempt<'_>> {
        if !self.should_attempt(key) {
            return None;
        }
        let mut in_flight = self
            .in_flight
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !in_flight.insert(key.to_string()) {
            return None;
        }
        Some(ProviderCountAttempt {
            cache: self,
            key: key.to_string(),
        })
    }

    pub fn profile_allowed(&self, key: &str) -> bool {
        let mut suppressions = self
            .profile_suppressions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match suppressions.get(key).copied() {
            Some(until) if until > Instant::now() => false,
            Some(_) => {
                suppressions.pop(key);
                true
            }
            None => true,
        }
    }

    pub fn suppress_profile(&self, key: String, duration: Duration) {
        self.profile_suppressions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .put(key, Instant::now() + duration);
    }
}

static PROFILE_FINGERPRINT_KEY: LazyLock<[u8; 32]> = LazyLock::new(rand::random);

pub fn profile_suppression_key(capability_key: &str, credential: &str) -> String {
    let digest = blake3::keyed_hash(&PROFILE_FINGERPRINT_KEY, credential.as_bytes());
    format!("{capability_key}:{}", &digest.to_hex()[..16])
}

pub struct ProviderCountAttempt<'a> {
    cache: &'a ProviderCountCapabilityCache,
    key: String,
}

impl Drop for ProviderCountAttempt<'_> {
    fn drop(&mut self) {
        self.cache
            .in_flight
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_suppression_does_not_poison_endpoint_capability() {
        let cache = ProviderCountCapabilityCache::default();
        let profile_a = profile_suppression_key("count:https://example.test", "key-a");
        let profile_b = profile_suppression_key("count:https://example.test", "key-b");
        cache.suppress_profile(profile_a.clone(), Duration::from_secs(60));
        assert!(!cache.profile_allowed(&profile_a));
        assert!(cache.profile_allowed(&profile_b));
        assert!(cache.should_attempt("count:https://example.test"));
    }

    #[test]
    fn endpoint_probe_is_single_flight() {
        let cache = ProviderCountCapabilityCache::default();
        let first = cache.begin_attempt("count:https://example.test").unwrap();
        assert!(cache.begin_attempt("count:https://example.test").is_none());
        drop(first);
        assert!(cache.begin_attempt("count:https://example.test").is_some());
    }
}
