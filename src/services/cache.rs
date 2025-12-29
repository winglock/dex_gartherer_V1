use dashmap::DashMap;
use std::time::{Duration, Instant};
use crate::models::PoolData;

pub struct PoolCache {
    cache: DashMap<String, CacheEntry>,
    ttl: Duration,
}

struct CacheEntry {
    data: PoolData,
    inserted_at: Instant,
}

impl PoolCache {
    pub fn new(ttl_seconds: u64) -> Self {
        Self {
            cache: DashMap::new(),
            ttl: Duration::from_secs(ttl_seconds),
        }
    }

    pub fn get(&self, key: &str) -> Option<PoolData> {
        if let Some(entry) = self.cache.get(key) {
            if entry.inserted_at.elapsed() < self.ttl {
                return Some(entry.data.clone());
            }
            drop(entry);
            self.cache.remove(key);
        }
        None
    }

    pub fn set(&self, key: &str, data: PoolData) {
        self.cache.insert(key.to_string(), CacheEntry {
            data,
            inserted_at: Instant::now(),
        });
    }

    pub fn get_all(&self) -> Vec<PoolData> {
        let now = Instant::now();
        self.cache.iter()
            .filter(|e| now.duration_since(e.inserted_at) < self.ttl)
            .map(|e| e.data.clone())
            .collect()
    }

    pub fn cleanup(&self) {
        let now = Instant::now();
        self.cache.retain(|_, entry| now.duration_since(entry.inserted_at) < self.ttl);
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }
}
