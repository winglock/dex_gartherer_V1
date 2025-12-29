use parking_lot::RwLock;
use std::sync::Arc;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use crate::models::PoolData;

pub struct PoolCache {
    cache: Arc<RwLock<HashMap<String, Arc<PoolData>>>>,
    ttl: Duration,
    last_cleanup: Arc<RwLock<Instant>>,
}

impl PoolCache {
    pub fn new(ttl_seconds: u64) -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            ttl: Duration::from_secs(ttl_seconds),
            last_cleanup: Arc::new(RwLock::new(Instant::now())),
        }
    }

    /// Zero-copy retrieval - returns Arc clone (pointer only)
    pub fn get(&self, key: &str) -> Option<Arc<PoolData>> {
        let cache = self.cache.read();
        cache.get(key).cloned()
    }

    /// Single insertion
    pub fn insert(&self, key: String, pool: PoolData) {
        let mut cache = self.cache.write();
        cache.insert(key, Arc::new(pool));
    }

    /// Zero-copy all pools
    pub fn get_all(&self) -> Vec<Arc<PoolData>> {
        let cache = self.cache.read();
        cache.values().cloned().collect()
    }

    /// Smart cleanup (only when needed)
    pub fn cleanup_if_needed(&self) {
        let mut last_cleanup = self.last_cleanup.write();
        if last_cleanup.elapsed() < Duration::from_secs(60) {
            return;
        }

        let now = Instant::now();
        let mut cache = self.cache.write();
        let before = cache.len();

        cache.retain(|_, pool| {
            let age = now.duration_since(
                Instant::now() - Duration::from_secs(
                    now.elapsed().as_secs().saturating_sub(pool.timestamp as u64)
                )
            );
            age < self.ttl
        });

        let removed = before - cache.len();
        if removed > 0 {
            tracing::info!("🧹 Cleaned {} expired pools", removed);
        }

        *last_cleanup = now;
    }

    pub fn len(&self) -> usize {
        self.cache.read().len()
    }
}
