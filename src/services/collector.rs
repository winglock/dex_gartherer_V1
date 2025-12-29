use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Semaphore;
use futures::stream::{self, StreamExt};
use indicatif::{ProgressBar, ProgressStyle, MultiProgress};
use std::time::Duration;
use crate::models::PoolData;
use crate::sources::{
    PoolSource, 
    gecko::GeckoTerminal, 
    aggregators::{DexScreenerSource, MatchaSource},
    meta_agg::MetaAggSource,
};
use super::{PoolCache, PoolFilter};

/// Collection statistics for monitoring
#[derive(Default)]
pub struct CollectorStats {
    pub total_requests: AtomicUsize,
    pub successful: AtomicUsize,
    pub failed: AtomicUsize,
    pub pools_collected: AtomicUsize,
}

#[derive(Debug)]
pub struct CollectorResult {
    pub total: usize,
    pub successful: usize,
    pub failed: usize,
}

pub struct PoolCollector {
    sources: Vec<Arc<dyn PoolSource>>,
    cache: Arc<PoolCache>,
    filter: PoolFilter,
    semaphore: Arc<Semaphore>,
    stats: Arc<CollectorStats>,
}

impl PoolCollector {
    pub fn new(cache: Arc<PoolCache>, filter: PoolFilter) -> Self {
        Self {
            // All active sources (4 APIs)
            sources: vec![
                Arc::new(GeckoTerminal::new()),      // Pool data with LP/volume
                Arc::new(DexScreenerSource::new()), // Reliable DEX data
                Arc::new(MatchaSource::new()),      // Token search (16 chains)
                Arc::new(MetaAggSource::new("http://localhost:8000")), // 1inch, ParaSwap, KyberSwap, OpenOcean
            ],
            cache,
            filter,
            semaphore: Arc::new(Semaphore::new(10)),
            stats: Arc::new(CollectorStats::default()),
        }
    }

    /// Progressive collection with visual feedback
    pub async fn collect_progressive(&self, symbols: &[String]) -> CollectorResult {
        let multi = MultiProgress::new();
        let overall_pb = multi.add(ProgressBar::new(symbols.len() as u64));
        overall_pb.set_style(
            ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} symbols | {msg}")
                .unwrap()
        );

        // Phase 1: Top 20 coins (high priority)
        let high_priority = &symbols[..20.min(symbols.len())];
        overall_pb.set_message("Phase 1: High Priority");
        self.collect_batch(high_priority, &multi, &overall_pb).await;

        // Phase 2: Next 30 coins
        if symbols.len() > 20 {
            let medium_priority = &symbols[20..50.min(symbols.len())];
            overall_pb.set_message("Phase 2: Medium Priority");
            self.collect_batch(medium_priority, &multi, &overall_pb).await;
        }

        // Phase 3: Remaining coins
        if symbols.len() > 50 {
            let low_priority = &symbols[50..];
            overall_pb.set_message("Phase 3: Remaining");
            self.collect_batch(low_priority, &multi, &overall_pb).await;
        }

        overall_pb.finish_with_message("✓ Complete");

        CollectorResult {
            total: self.stats.pools_collected.load(Ordering::Relaxed),
            successful: self.stats.successful.load(Ordering::Relaxed),
            failed: self.stats.failed.load(Ordering::Relaxed),
        }
    }

    async fn collect_batch(
        &self,
        symbols: &[String],
        multi: &MultiProgress,
        overall_pb: &ProgressBar,
    ) {
        let pb = multi.add(ProgressBar::new(symbols.len() as u64));
        pb.set_style(
            ProgressStyle::default_bar()
                .template("  └─ {spinner:.green} [{bar:30}] {pos}/{len} | {msg}")
                .unwrap()
        );

        stream::iter(symbols.iter().cloned())
            .map(|symbol| {
                let pb = pb.clone();
                let sources = self.sources.clone();
                let cache = self.cache.clone();
                let filter = self.filter.clone();
                let semaphore = self.semaphore.clone();
                let stats = self.stats.clone();

                async move {
                    let _permit = semaphore.acquire().await.unwrap();
                    pb.set_message(format!("Fetching {}", symbol));

                    let start = std::time::Instant::now();
                    let mut count = 0;

                    for source in sources.iter() {
                        stats.total_requests.fetch_add(1, Ordering::Relaxed);

                        match tokio::time::timeout(
                            Duration::from_secs(10),
                            source.fetch_pools(&symbol)
                        ).await {
                            Ok(Ok(pools)) => {
                                stats.successful.fetch_add(1, Ordering::Relaxed);
                                for pool in pools {
                                    if filter.is_valid(&pool) {
                                        cache.insert(pool.pool_address.clone(), pool);
                                        count += 1;
                                        stats.pools_collected.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                            }
                            Ok(Err(e)) => {
                                stats.failed.fetch_add(1, Ordering::Relaxed);
                                tracing::warn!("❌ {} | {}: {}", symbol, source.name(), e);
                            }
                            Err(_) => {
                                stats.failed.fetch_add(1, Ordering::Relaxed);
                                tracing::warn!("⏱️ {} timeout", symbol);
                            }
                        }
                    }

                    if count > 0 {
                        pb.println(format!(
                            "  ✓ {} | {} pools in {:.1}s",
                            symbol, count, start.elapsed().as_secs_f32()
                        ));
                    }

                    pb.inc(1);
                }
            })
            .buffer_unordered(5)
            .collect::<Vec<_>>()
            .await;

        pb.finish_and_clear();
        overall_pb.inc(symbols.len() as u64);
    }

    pub fn get_stats(&self) -> &Arc<CollectorStats> {
        &self.stats
    }

    #[allow(dead_code)]
    pub fn get_cached(&self, address: &str) -> Option<Arc<PoolData>> {
        self.cache.get(address)
    }
}
