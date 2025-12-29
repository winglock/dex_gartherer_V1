use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use chrono::{Utc, Datelike};
use crate::models::PoolData;

pub struct LocalStorage {
    data_dir: PathBuf,
    pools_dir: PathBuf,
    snapshots_dir: PathBuf,
}

impl LocalStorage {
    pub fn new(base_dir: &str) -> Self {
        let data_dir = PathBuf::from(base_dir);
        let pools_dir = data_dir.join("pools");
        let snapshots_dir = data_dir.join("snapshots");

        // Create directories if not exist
        fs::create_dir_all(&pools_dir).ok();
        fs::create_dir_all(&snapshots_dir).ok();

        Self {
            data_dir,
            pools_dir,
            snapshots_dir,
        }
    }

    /// Save pools for a specific symbol
    pub fn save_symbol_pools(&self, symbol: &str, pools: &[PoolData]) {
        let now = Utc::now();
        let filename = format!("{}_{}-{:02}-{:02}.json", 
            symbol, now.year(), now.month(), now.day());
        let path = self.pools_dir.join(&filename);

        if let Ok(file) = File::create(&path) {
            let writer = BufWriter::new(file);
            if serde_json::to_writer_pretty(writer, pools).is_ok() {
                tracing::debug!("💾 Saved {} pools for {} -> {}", pools.len(), symbol, filename);
            }
        }
    }

    /// Load pools for a specific symbol (today's file)
    pub fn load_symbol_pools(&self, symbol: &str) -> Vec<PoolData> {
        let now = Utc::now();
        let filename = format!("{}_{}-{:02}-{:02}.json", 
            symbol, now.year(), now.month(), now.day());
        let path = self.pools_dir.join(&filename);

        if let Ok(file) = File::open(&path) {
            let reader = BufReader::new(file);
            if let Ok(pools) = serde_json::from_reader(reader) {
                return pools;
            }
        }
        vec![]
    }

    /// Save full snapshot of all pools
    pub fn save_snapshot(&self, all_pools: &[PoolData]) {
        let now = Utc::now();
        let filename = format!("full_{}.json", now.format("%Y-%m-%dT%H-%M"));
        let path = self.snapshots_dir.join(&filename);

        if let Ok(file) = File::create(&path) {
            let writer = BufWriter::new(file);
            if serde_json::to_writer_pretty(writer, all_pools).is_ok() {
                tracing::info!("📦 Snapshot saved: {} ({} pools)", filename, all_pools.len());
            }
        }
    }

    /// Save pools grouped by symbol
    pub fn save_all_by_symbol(&self, pools: &[PoolData]) {
        use std::collections::HashMap;
        
        let mut by_symbol: HashMap<&str, Vec<&PoolData>> = HashMap::new();
        for pool in pools {
            by_symbol.entry(&pool.symbol).or_default().push(pool);
        }

        for (symbol, symbol_pools) in by_symbol {
            let owned: Vec<PoolData> = symbol_pools.into_iter().cloned().collect();
            self.save_symbol_pools(symbol, &owned);
        }
    }

    /// Get storage stats
    pub fn get_stats(&self) -> StorageStats {
        let pool_files = fs::read_dir(&self.pools_dir)
            .map(|entries| entries.count())
            .unwrap_or(0);
        
        let snapshot_files = fs::read_dir(&self.snapshots_dir)
            .map(|entries| entries.count())
            .unwrap_or(0);

        let total_size = Self::dir_size(&self.data_dir);

        StorageStats {
            pool_files,
            snapshot_files,
            total_size_mb: total_size as f64 / 1024.0 / 1024.0,
        }
    }

    fn dir_size(path: &PathBuf) -> u64 {
        let mut size = 0;
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let metadata = entry.metadata().ok();
                if let Some(meta) = metadata {
                    if meta.is_file() {
                        size += meta.len();
                    } else if meta.is_dir() {
                        size += Self::dir_size(&entry.path());
                    }
                }
            }
        }
        size
    }
}

#[derive(Debug, Clone)]
pub struct StorageStats {
    pub pool_files: usize,
    pub snapshot_files: usize,
    pub total_size_mb: f64,
}
