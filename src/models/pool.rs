use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolData {
    pub symbol: String,
    pub chain: String,
    pub dex: String,
    pub pool_address: String,
    pub pair: String,
    pub price_usd: f64,
    pub lp_reserve_usd: f64,
    pub volume_24h: f64,
    pub fee_tier: Option<f64>,
    pub source: String,
    pub timestamp: i64,
}

impl PoolData {
    pub fn new(
        symbol: String,
        chain: String,
        dex: String,
        pool_address: String,
        pair: String,
        price_usd: f64,
        lp_reserve_usd: f64,
        volume_24h: f64,
        source: String,
    ) -> Self {
        Self {
            symbol,
            chain,
            dex,
            pool_address,
            pair,
            price_usd,
            lp_reserve_usd,
            volume_24h,
            fee_tier: None,
            source,
            timestamp: chrono::Utc::now().timestamp(),
        }
    }
}
