use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Semaphore;
use futures::stream::{self, StreamExt};
use std::time::Duration;
use crate::models::PoolData;
use crate::sources::{
    PoolSource, 
    gecko::GeckoTerminal, 
    aggregators::{DexScreenerSource, MatchaSource},
    meta_agg::{self, OpenOceanDirectSource, ParaSwapDirectSource},
};
use super::{PoolCache, PoolFilter};

const MAX_RETRIES: usize = 3;

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
        let token_cache = meta_agg::new_token_cache();
        
        // Sources in priority order: DexScreener → GeckoTerminal → Matcha → OpenOcean → ParaSwap
        Self {
            sources: vec![
                Arc::new(DexScreenerSource::new()),
                Arc::new(GeckoTerminal::new()),
                Arc::new(MatchaSource::new()),
                Arc::new(OpenOceanDirectSource::new(token_cache.clone())),
                Arc::new(ParaSwapDirectSource::new(token_cache.clone())),
            ],
            cache,
            filter,
            semaphore: Arc::new(Semaphore::new(20)),
            stats: Arc::new(CollectorStats::default()),
        }
    }

    /// Collect data from all sources sequentially with retry
    pub async fn collect_all(&self, symbols: &[String]) -> CollectorResult {
        let total_pools = Arc::new(AtomicUsize::new(0));
        let successful = Arc::new(AtomicUsize::new(0));
        let failed = Arc::new(AtomicUsize::new(0));

        println!("\n📊 데이터 수집 시작 ({} 심볼)", symbols.len());
        println!("─────────────────────────────────────────");

        // Process each source sequentially
        for source in self.sources.iter() {
            let source_name = source.name();
            println!("\n🔍 {} 조회 중...", source_name);
            
            let start = std::time::Instant::now();
            let mut source_pools = 0usize;
            let mut source_failed = 0usize;

            // Process symbols in parallel for this source
            let results: Vec<(String, Result<Vec<PoolData>, ()>)> = stream::iter(symbols.iter().cloned())
                .map(|symbol| {
                    let source = source.clone();
                    let semaphore = self.semaphore.clone();
                    
                    async move {
                        let _permit = semaphore.acquire().await.unwrap();
                        
                        // Retry logic
                        for attempt in 0..MAX_RETRIES {
                            match tokio::time::timeout(
                                Duration::from_secs(10),
                                source.fetch_pools(&symbol)
                            ).await {
                                Ok(Ok(pools)) => return (symbol, Ok(pools)),
                                Ok(Err(_)) | Err(_) => {
                                    if attempt < MAX_RETRIES - 1 {
                                        tokio::time::sleep(Duration::from_millis(500)).await;
                                    }
                                }
                            }
                        }
                        (symbol, Err(()))
                    }
                })
                .buffer_unordered(20)
                .collect()
                .await;

            // Process results
            for (symbol, result) in results {
                match result {
                    Ok(pools) => {
                        let filtered: Vec<_> = pools.into_iter()
                            .filter(|p| self.filter.is_valid(p))
                            .collect();
                        
                        for pool in filtered {
                            let key = format!("{}:{}:{}", pool.source, pool.chain, pool.pool_address);
                            self.cache.insert(key, pool);
                            source_pools += 1;
                        }
                        successful.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        source_failed += 1;
                        failed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }

            total_pools.fetch_add(source_pools, Ordering::Relaxed);
            
            let elapsed = start.elapsed();
            println!("   ✓ {} - {}개 풀 ({} 실패) [{:.2}초]",
                source_name, source_pools, source_failed, elapsed.as_secs_f64());
        }

        let total = total_pools.load(Ordering::Relaxed);
        println!("\n─────────────────────────────────────────");
        println!("✅ 완료: 총 {}개 풀 수집", total);

        CollectorResult {
            total,
            successful: successful.load(Ordering::Relaxed),
            failed: failed.load(Ordering::Relaxed),
        }
    }

    /// Get all cached pools
    pub fn get_cached_pools(&self) -> Vec<Arc<PoolData>> {
        self.cache.get_all()
    }

    /// Get collection statistics
    pub fn get_stats(&self) -> Arc<CollectorStats> {
        self.stats.clone()
    }
}