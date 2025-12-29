use crate::models::{PoolData, ArbitrageAlert, alert::ArbType};
use crate::sources::upbit::CexPrice;
use std::collections::HashMap;
use std::sync::Arc;

pub struct ArbitrageDetector {
    threshold: f64,
}

impl ArbitrageDetector {
    pub fn new(threshold: f64) -> Self {
        Self { threshold }
    }

    /// DEX-DEX arbitrage detection (Arc optimized)
    pub fn detect_dex_dex(&self, pools: &[Arc<PoolData>]) -> Vec<ArbitrageAlert> {
        let mut alerts = Vec::new();

        let mut by_symbol: HashMap<&str, Vec<&PoolData>> = HashMap::new();
        for pool in pools {
            by_symbol.entry(&pool.symbol).or_default().push(pool.as_ref());
        }

        for (_symbol, symbol_pools) in by_symbol {
            if symbol_pools.len() < 2 {
                continue;
            }

            let mut min_pool = symbol_pools[0];
            let mut max_pool = symbol_pools[0];

            for pool in &symbol_pools {
                if pool.price_usd < min_pool.price_usd {
                    min_pool = pool;
                }
                if pool.price_usd > max_pool.price_usd {
                    max_pool = pool;
                }
            }

            if min_pool.price_usd <= 0.0 {
                continue;
            }

            let diff_pct = (max_pool.price_usd - min_pool.price_usd) / min_pool.price_usd;

            if diff_pct >= self.threshold {
                alerts.push(ArbitrageAlert::from_pools(min_pool, max_pool));
            }
        }

        alerts
    }

    /// DEX-CEX arbitrage detection (Arc optimized)
    pub fn detect_dex_cex(&self, pools: &[Arc<PoolData>], cex_prices: &[CexPrice]) -> Vec<ArbitrageAlert> {
        let mut alerts = Vec::new();

        let cex_map: HashMap<&str, &CexPrice> = cex_prices.iter()
            .map(|p| (p.symbol.as_str(), p))
            .collect();

        for pool in pools {
            if let Some(cex) = cex_map.get(pool.symbol.as_str()) {
                if pool.price_usd <= 0.0 || cex.price_usd <= 0.0 {
                    continue;
                }

                let (low, high, low_source, high_source) = if pool.price_usd < cex.price_usd {
                    (pool.price_usd, cex.price_usd, 
                     format!("{}:{}", pool.dex, pool.pool_address),
                     "upbit".to_string())
                } else {
                    (cex.price_usd, pool.price_usd,
                     "upbit".to_string(),
                     format!("{}:{}", pool.dex, pool.pool_address))
                };

                let diff_pct = (high - low) / low;

                if diff_pct >= self.threshold {
                    alerts.push(ArbitrageAlert {
                        symbol: pool.symbol.clone(),
                        arb_type: ArbType::DexToCex,
                        low_price: low,
                        low_source,
                        high_price: high,
                        high_source,
                        diff_pct: diff_pct * 100.0,
                        timestamp: chrono::Utc::now().timestamp(),
                    });
                }
            }
        }

        alerts
    }

    #[allow(dead_code)]
    pub fn set_threshold(&mut self, threshold: f64) {
        self.threshold = threshold;
    }
}
