use crate::models::PoolData;
use crate::config::FilterConfig;

#[derive(Clone)]
pub struct PoolFilter {
    min_lp: f64,
    min_volume: f64,
    min_tx_count: u32,
}

impl PoolFilter {
    pub fn new(config: &FilterConfig) -> Self {
        Self {
            min_lp: config.min_lp,
            min_volume: config.min_volume,
            min_tx_count: config.min_tx_count,
        }
    }

    /// 풀 유효성 검사 (스캠/허니팟 필터)
    pub fn is_valid(&self, pool: &PoolData) -> bool {
        // LP가 충분한 경우
        if pool.lp_reserve_usd >= self.min_lp {
            return pool.volume_24h >= self.min_volume;
        }

        // LP가 낮아도 거래량이 있으면 거래 가능
        // (동적 LP 계산: LP의 2% 이상 거래 가능 시 유효)
        let tradeable = pool.lp_reserve_usd * 0.02;
        if tradeable >= 100.0 && pool.volume_24h >= self.min_volume {
            return true;
        }

        false
    }

    /// 거래 가능 금액 계산
    pub fn calculate_max_trade(&self, pool: &PoolData) -> f64 {
        // LP의 2% (슬리피지 고려)
        pool.lp_reserve_usd * 0.02
    }

    pub fn set_min_lp(&mut self, min_lp: f64) {
        self.min_lp = min_lp;
    }

    pub fn set_min_volume(&mut self, min_volume: f64) {
        self.min_volume = min_volume;
    }
}
