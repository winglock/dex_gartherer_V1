use serde::{Deserialize, Serialize};
use super::PoolData;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitrageAlert {
    pub symbol: String,
    pub arb_type: ArbType,
    pub low_price: f64,
    pub low_source: String,
    pub high_price: f64,
    pub high_source: String,
    pub diff_pct: f64,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ArbType {
    DexToDex,
    DexToCex,
}

impl ArbitrageAlert {
    pub fn from_pools(low: &PoolData, high: &PoolData) -> Self {
        let diff_pct = (high.price_usd - low.price_usd) / low.price_usd * 100.0;
        Self {
            symbol: low.symbol.clone(),
            arb_type: ArbType::DexToDex,
            low_price: low.price_usd,
            low_source: format!("{}:{}", low.dex, low.pool_address),
            high_price: high.price_usd,
            high_source: format!("{}:{}", high.dex, high.pool_address),
            diff_pct,
            timestamp: chrono::Utc::now().timestamp(),
        }
    }
}
