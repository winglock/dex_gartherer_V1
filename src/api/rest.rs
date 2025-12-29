use axum::{
    extract::State,
    response::Json,
    routing::get,
    Router,
};
use std::sync::Arc;
use crate::models::{PoolData, ArbitrageAlert};
use crate::services::{PoolCollector, ArbitrageDetector, PoolCache};
use crate::sources::upbit::UpbitClient;

pub struct AppState {
    pub collector: Arc<PoolCollector>,
    pub detector: Arc<ArbitrageDetector>,
    pub cache: Arc<PoolCache>,
    pub upbit: Arc<UpbitClient>,
    pub symbols: Vec<String>,
}

/// GET /pools - 모든 풀 데이터
async fn get_pools(State(state): State<Arc<AppState>>) -> Json<Vec<PoolData>> {
    let pools = state.collector.collect_all(&state.symbols).await;
    Json(pools)
}

/// GET /pools/cached - 캐시된 풀 데이터 (빠름)
async fn get_cached_pools(State(state): State<Arc<AppState>>) -> Json<Vec<PoolData>> {
    Json(state.cache.get_all())
}

/// GET /arbitrage - 현재 아비트라지 기회
async fn get_arbitrage(State(state): State<Arc<AppState>>) -> Json<Vec<ArbitrageAlert>> {
    let pools = state.cache.get_all();
    let cex_prices = state.upbit.get_all_prices();
    
    let mut alerts = state.detector.detect_dex_dex(&pools);
    alerts.extend(state.detector.detect_dex_cex(&pools, &cex_prices));
    
    Json(alerts)
}

/// GET /health
async fn health() -> &'static str {
    "OK"
}

/// GET /stats
async fn stats(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "cached_pools": state.cache.len(),
        "symbols": state.symbols.len(),
    }))
}

pub fn create_rest_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/pools", get(get_pools))
        .route("/pools/cached", get(get_cached_pools))
        .route("/arbitrage", get(get_arbitrage))
        .route("/health", get(health))
        .route("/stats", get(stats))
        .with_state(state)
}
