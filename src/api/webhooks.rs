//! Webhook layer: shared deduplication cache and per-type secret validation.
//! Used by POST /webhooks/frigate, /webhooks/homeassistant, etc.

#![cfg(feature = "webhooks")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

const DEFAULT_MAX_ENTRIES: usize = 1000;
const DEFAULT_TTL_SECS: u64 = 24 * 3600; // 24h

/// In-memory dedup cache for webhook event IDs / pointers.
/// Evicts oldest entries when full; entries expire after TTL.
#[derive(Clone)]
pub struct DedupCache {
    inner: Arc<RwLock<DedupCacheInner>>,
    ttl: Duration,
    max_entries: usize,
}

struct DedupCacheInner {
    entries: HashMap<String, Instant>,
}

impl DedupCache {
    pub fn new() -> Self {
        Self::with_capacity_and_ttl(DEFAULT_MAX_ENTRIES, Duration::from_secs(DEFAULT_TTL_SECS))
    }

    pub fn with_capacity_and_ttl(max_entries: usize, ttl: Duration) -> Self {
        Self {
            inner: Arc::new(RwLock::new(DedupCacheInner {
                entries: HashMap::new(),
            })),
            ttl,
            max_entries,
        }
    }

    /// Returns true if key was already seen (duplicate), false if new (and inserted).
    pub async fn seen_or_insert(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut guard = self.inner.write().await;
        guard.evict_expired(now, self.ttl);
        if guard.entries.contains_key(key) {
            return true;
        }
        while guard.entries.len() >= self.max_entries {
            guard.evict_oldest();
        }
        guard.entries.insert(key.to_string(), now);
        false
    }
}

impl Default for DedupCache {
    fn default() -> Self {
        Self::new()
    }
}

impl DedupCacheInner {
    fn evict_expired(&mut self, now: Instant, ttl: Duration) {
        self.entries.retain(|_, t| now.duration_since(*t) < ttl);
    }

    fn evict_oldest(&mut self) {
        let oldest = self
            .entries
            .iter()
            .min_by_key(|(_, t)| *t)
            .map(|(k, _)| k.clone());
        if let Some(k) = oldest {
            self.entries.remove(&k);
        }
    }
}

/// Checks webhook secret from header `X-Webhook-Secret` or query `secret`.
/// Returns true only if a secret is configured and the provided value matches.
/// If no secret is configured, returns false (webhook disabled for security).
pub fn check_webhook_secret(
    expected: Option<&str>,
    header_secret: Option<&str>,
    query_secret: Option<&str>,
) -> bool {
    let Some(expected) = expected else {
        return false; // no secret configured => reject (webhook disabled)
    };
    if expected.is_empty() {
        return false;
    }
    let provided = header_secret.or(query_secret);
    match provided {
        Some(s) => s == expected,
        None => false,
    }
}
