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
    meta_agg::{self, MatchaTokenResolver, KyberSwapDirectSource, OpenOceanDirectSource, ParaSwapDirectSource},
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
        // Shared token address cache for aggregators
        let token_cache = meta_agg::new_token_cache();
        
        Self {
            // All 7 active sources with shared token cache
            sources: vec![
                // Primary sources (always work)
                Arc::new(GeckoTerminal::new()),
                Arc::new(DexScreenerSource::new()),
                // Token resolver (populates cache for others)
                Arc::new(MatchaTokenResolver::new(token_cache.clone())),
                // Aggregators using shared cache
                Arc::new(KyberSwapDirectSource::new(token_cache.clone())),
                Arc::new(OpenOceanDirectSource::new(token_cache.clone())),
                Arc::new(ParaSwapDirectSource::new(token_cache.clone())),
                // Original Matcha (for comparison)
                Arc::new(MatchaSource::new()),
            ],
            cache,
            filter,
            semaphore: Arc::new(Semaphore::new(10)),
            stats: Arc::new(CollectorStats::default()),
        }
    }

    /// Progressive collection with visual feedback
    pub async fn collect_progressive(&self, symbols: &[String]) -> CollectorResult {
        let mp = MultiProgress::new();
        
        let pb_total = mp.add(ProgressBar::new(symbols.len() as u64));
        pb_total.set_style(ProgressStyle::default_bar()
            .template("{prefix:.bold.blue} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("█▓▒░"));
        pb_total.set_prefix("📊 Collecting");

        let total_pools = Arc::new(AtomicUsize::new(0));
        let successful = Arc::new(AtomicUsize::new(0));
        let failed = Arc::new(AtomicUsize::new(0));

        stream::iter(symbols.iter().cloned())
            .map(|symbol| {
                let pb = pb_total.clone();
                let sources = self.sources.clone();
                let cache = self.cache.clone();
                let filter = self.filter.clone();
                let semaphore = self.semaphore.clone();
                let total_pools = total_pools.clone();
                let successful = successful.clone();
                let failed = failed.clone();

                async move {
                    let _permit = semaphore.acquire().await.unwrap();
                    
                    // 🔥 디버그: 토큰 처리 시작
                    tracing::debug!("\n🔍 처리 중: {}", symbol);
                    
                    let mut source_results: Vec<String> = Vec::new();
                    
                    for source in sources.iter() {
                        let source_name = source.name();
                        pb.set_message(format!("{} ← {}", symbol, source_name));
                        
                        match tokio::time::timeout(
                            Duration::from_secs(10),
                            source.fetch_pools(&symbol)
                        ).await {
                            Ok(Ok(pools)) => {
                                let pool_count = pools.len();
                                
                                // 🔥 디버그: 소스별 결과
                                tracing::info!("  ✅ {}: {}에서 {}개 풀 수신", 
                                    symbol, source_name, pool_count);
                                
                                let before_filter = pools.len();
                                let filtered: Vec<_> = pools.into_iter()
                                    .filter(|p| {
                                        let valid = filter.is_valid(p);
                                        // 🔥 디버그: 필터링된 풀 상세
                                        if !valid {
                                            tracing::debug!("    ⏭️ 필터됨: {} @ {} (가격=${:.4}, LP=${:.0}, Vol=${:.0})",
                                                p.symbol, p.dex, p.price_usd, p.lp_reserve_usd, p.volume_24h);
                                        }
                                        valid
                                    })
                                    .collect();
                                
                                let after_filter = filtered.len();
                                // 🔥 디버그: 필터 결과 요약
                                tracing::info!("    → 필터: {}/{} 통과 ({}개 제거)", 
                                    after_filter, before_filter, before_filter - after_filter);
                                
                                for pool in filtered {
                                    let key = format!("{}:{}:{}", pool.source, pool.chain, pool.pool_address);
                                    cache.insert(key, pool);
                                    total_pools.fetch_add(1, Ordering::Relaxed);
                                }
                                successful.fetch_add(1, Ordering::Relaxed);
                                
                                if pool_count > 0 {
                                    source_results.push(format!("{}:{}", source_name, pool_count));
                                }
                            }
                            Ok(Err(e)) => {
                                // 🔥 디버그: API 에러
                                tracing::warn!("  ❌ {}: {} 실패 - {}", 
                                    symbol, source_name, e);
                                failed.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(_) => {
                                // 🔥 디버그: 타임아웃
                                tracing::warn!("  ⏱️ {}: {} 타임아웃 (10초)", 
                                    symbol, source_name);
                                failed.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }

                    pb.inc(1);
                    let sources_info = if source_results.is_empty() {
                        "없음".to_string()
                    } else {
                        source_results.join(" | ")
                    };
                    pb.set_message(format!("{} [{}]", symbol, sources_info));
                }
            })
            .buffer_unordered(5)
            .collect::<Vec<_>>()
            .await;

        pb_total.finish_with_message(format!("✓ {}개 풀 수집 완료", total_pools.load(Ordering::Relaxed)));

        CollectorResult {
            total: total_pools.load(Ordering::Relaxed),
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