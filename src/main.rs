mod config;
mod models;
mod sources;
mod services;
mod api;

use std::sync::Arc;
use axum::{Router, routing::get, extract::State};
use axum::extract::ws::{WebSocket, WebSocketUpgrade, Message};
use axum::response::IntoResponse;
use tower_http::cors::CorsLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tokio::time::{interval, Duration};
use futures::{SinkExt, StreamExt};

use config::Config;
use services::{PoolCollector, ArbitrageDetector, PoolCache, PoolFilter};
use sources::upbit::UpbitClient;

pub struct AppState {
    pub collector: Arc<PoolCollector>,
    pub detector: Arc<ArbitrageDetector>,
    pub cache: Arc<PoolCache>,
    pub upbit: Arc<UpbitClient>,
    pub symbols: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 로깅 초기화
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // 설정 로드
    let config = Config::load().expect("Failed to load config.toml");
    tracing::info!("Config loaded: threshold={}%, update={}s", 
        config.arbitrage.threshold * 100.0,
        config.arbitrage.update_interval);

    // 업비트에서 KRW 코인 목록 가져오기
    let upbit = Arc::new(UpbitClient::new());
    let symbols = upbit.fetch_krw_coins().await.unwrap_or_else(|e| {
        tracing::error!("Failed to fetch Upbit coins: {}", e);
        vec!["BTC".to_string(), "ETH".to_string()]
    });
    tracing::info!("Loaded {} symbols from Upbit", symbols.len());

    // 업비트 WebSocket 시작
    if let Err(e) = upbit.start_websocket(symbols.clone()).await {
        tracing::warn!("Upbit WebSocket failed: {}", e);
    }

    // 서비스 초기화
    let cache = Arc::new(PoolCache::new(config.arbitrage.update_interval));
    let filter = PoolFilter::new(&config.filter);
    let collector = Arc::new(PoolCollector::new(cache.clone(), filter));
    let detector = Arc::new(ArbitrageDetector::new(config.arbitrage.threshold));

    let state = Arc::new(AppState {
        collector,
        detector,
        cache,
        upbit,
        symbols,
    });

    // 초기 수집은 백그라운드에서 (서버 빠른 시작)
    tracing::info!("Server starting, pool collection will happen on first request");

    // 라우터  
    let app = Router::new()
        .route("/pools", get(get_pools))
        .route("/pools/cached", get(get_cached_pools))
        .route("/arbitrage", get(get_arbitrage))
        .route("/health", get(health))
        .route("/ws", get(ws_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    tracing::info!("Starting server on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// REST handlers
async fn get_pools(State(state): State<Arc<AppState>>) -> axum::Json<Vec<models::PoolData>> {
    let pools = state.collector.collect_all(&state.symbols).await;
    axum::Json(pools)
}

async fn get_cached_pools(State(state): State<Arc<AppState>>) -> axum::Json<Vec<models::PoolData>> {
    axum::Json(state.cache.get_all())
}

async fn get_arbitrage(State(state): State<Arc<AppState>>) -> axum::Json<Vec<models::ArbitrageAlert>> {
    let pools = state.cache.get_all();
    let cex_prices = state.upbit.get_all_prices();
    let mut alerts = state.detector.detect_dex_dex(&pools);
    alerts.extend(state.detector.detect_dex_cex(&pools, &cex_prices));
    axum::Json(alerts)
}

async fn health() -> &'static str {
    "OK"
}

// WebSocket handler
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut ticker = interval(Duration::from_secs(30));

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let pools = state.collector.collect_all(&state.symbols).await;
                let cex_prices = state.upbit.get_all_prices();
                let mut alerts = state.detector.detect_dex_dex(&pools);
                alerts.extend(state.detector.detect_dex_cex(&pools, &cex_prices));

                let pool_msg = serde_json::json!({
                    "type": "pool_update",
                    "count": pools.len(),
                    "data": pools,
                });
                if sender.send(Message::Text(pool_msg.to_string())).await.is_err() {
                    break;
                }

                if !alerts.is_empty() {
                    let alert_msg = serde_json::json!({
                        "type": "arb_alert",
                        "count": alerts.len(),
                        "data": alerts,
                    });
                    if sender.send(Message::Text(alert_msg.to_string())).await.is_err() {
                        break;
                    }
                }
            }

            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(cmd) = serde_json::from_str::<serde_json::Value>(&text) {
                            if cmd["type"] == "ping" {
                                let _ = sender.send(Message::Text(r#"{"type":"pong"}"#.to_string())).await;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}
