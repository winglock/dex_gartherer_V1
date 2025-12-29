use std::sync::Arc;
use tokio::sync::Semaphore;
use crate::models::PoolData;
use crate::sources::{PoolSource, gecko::GeckoTerminal, dexguru::DexGuruSource};
use super::{PoolCache, PoolFilter};

pub struct PoolCollector {
    sources: Vec<Arc<dyn PoolSource>>,
    cache: Arc<PoolCache>,
    filter: PoolFilter,
    semaphore: Arc<Semaphore>,
}

impl PoolCollector {
    pub fn new(cache: Arc<PoolCache>, filter: PoolFilter) -> Self {
        let sources: Vec<Arc<dyn PoolSource>> = vec![
            Arc::new(GeckoTerminal::new()),
            Arc::new(DexGuruSource::new()),
        ];

        Self {
            sources,
            cache,
            filter,
            semaphore: Arc::new(Semaphore::new(20)),
        }
    }

    pub async fn collect_all(&self, symbols: &[String]) -> Vec<PoolData> {
        let mut all_pools = Vec::new();
        
        // 순차 처리로 변경 (lifetime 문제 해결)
        for symbol in symbols {
            let pools = self.collect_symbol(symbol).await;
            all_pools.extend(pools);
        }

        // 캐시에 저장
        for pool in &all_pools {
            self.cache.set(&pool.pool_address, pool.clone());
        }

        all_pools
    }

    async fn collect_symbol(&self, symbol: &str) -> Vec<PoolData> {
        let _permit = self.semaphore.acquire().await.unwrap();
        let mut pools = Vec::new();

        for source in &self.sources {
            match source.fetch_pools(symbol).await {
                Ok(source_pools) => {
                    for pool in source_pools {
                        if self.filter.is_valid(&pool) {
                            pools.push(pool);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Source {} error for {}: {}", source.name(), symbol, e);
                }
            }
        }

        pools
    }

    #[allow(dead_code)]
    pub fn get_cached(&self, address: &str) -> Option<PoolData> {
        self.cache.get(address)
    }
}
