mod config;
mod models;
mod sources;
mod services;

use std::sync::Arc;
use std::path::Path;
use crate::sources::PoolSource;
use std::sync::atomic::Ordering;
use axum::{
    Router, 
    routing::get,
    extract::{State, ws::{WebSocket, WebSocketUpgrade, Message}},
    response::IntoResponse,
};
use tower_http::cors::CorsLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tokio::time::{interval, Duration};
use futures::{SinkExt, StreamExt};

use config::Config;
use services::{PoolCollector, ArbitrageDetector, PoolCache, PoolFilter, PriceMonitor};
use sources::upbit::UpbitClient;

pub struct AppState {
    pub collector: Arc<PoolCollector>,
    pub detector: Arc<ArbitrageDetector>,
    pub cache: Arc<PoolCache>,
    pub upbit: Arc<UpbitClient>,
    pub symbols: Vec<String>,
}

// main() 함수 바로 위에 추가
async fn debug_single_token(symbol: &str) {
    println!("\n🔍 테스트 중: {}\n", symbol);
    
    let sources: Vec<Arc<dyn PoolSource>> = vec![
        Arc::new(sources::gecko::GeckoTerminal::new()),
        Arc::new(sources::aggregators::DexScreenerSource::new()),
        Arc::new(sources::aggregators::MatchaSource::new()),
    ];
    
    for source in sources {
        print!("  {} ... ", source.name());
        match source.fetch_pools(symbol).await {
            Ok(pools) => {
                println!("✅ {}개 풀", pools.len());
                for pool in pools.iter().take(2) {
                    println!("    - ${:.4} @ {} (LP: ${})", 
                        pool.price_usd, pool.dex, pool.lp_reserve_usd);
                }
            }
            Err(e) => println!("❌ {}", e),
        }
    }
}

#[tokio::main(worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check for --monitor flag
    let args: Vec<String> = std::env::args().collect();
    if args.contains(&"--monitor".to_string()) || args.contains(&"-m".to_string()) {
        println!("\n🔄 DEX Price Monitor Mode\n");
        
        let mut monitor = PriceMonitor::new();
        let pools_path = Path::new("./data/pools");
        
        let loaded = monitor.load_pools(pools_path)?;
        println!("✓ {} 풀 로드 완료", loaded);
        
        // Run continuous monitoring with 30 second interval
        monitor.run_continuous(30).await;
        return Ok(());
    }

    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,dex_gatherer=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    println!("\n🚀 DEX Pool Monitor Starting...\n");

    // Load configuration
    let config = Config::load()?;
    tracing::info!("✓ Configuration loaded");

    // Initialize Upbit client
    println!("📡 Connecting to Upbit...");
    let upbit = Arc::new(UpbitClient::new());
    let symbols = upbit.fetch_krw_coins().await?;
    tracing::info!("✓ Loaded {} KRW pairs", symbols.len());

    // Start Upbit WebSocket
    upbit.start_websocket(symbols.clone()).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    tracing::info!("✓ Upbit WebSocket connected");

    // Initialize services
    let cache = Arc::new(PoolCache::new(120));
    let filter = PoolFilter::new(&config.filter);
    let collector = Arc::new(PoolCollector::new(cache.clone(), filter));
    let detector = Arc::new(ArbitrageDetector::new(config.arbitrage.threshold));

    // Initialize storage
    let storage = if config.storage.enabled {
        Some(Arc::new(services::LocalStorage::new(&config.storage.data_dir)))
    } else {
        None
    };

    // Background: Pool collection with storage (1 minute cycle)
    println!("\n📥 Starting pool collection (1 min cycle)...\n");
    let collector_clone = collector.clone();
    let symbols_clone = symbols.clone();
    let storage_clone = storage.clone();
    let cache_clone2 = cache.clone();
    tokio::spawn(async move {
        loop {
            let result = collector_clone.collect_all(&symbols_clone).await;
            
            // Save to local storage
            if let Some(ref storage) = storage_clone {
                let pools: Vec<models::PoolData> = cache_clone2.get_all()
                    .into_iter()
                    .map(|arc| (*arc).clone())
                    .collect();
                storage.save_all_by_symbol(&pools);
                storage.save_snapshot(&pools);
            }
            
            tracing::info!(
                "✓ Cycle complete: {} pools | {}/{} requests | saved to ./data",
                result.total,
                result.successful,
                result.successful + result.failed
            );
            tokio::time::sleep(Duration::from_secs(60)).await; // 1 minute
        }
    });

    // Background: Cache cleanup
    let cache_clone = cache.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(60));
        loop {
            ticker.tick().await;
            cache_clone.cleanup_if_needed();
        }
    });

    // Application state
    let state = Arc::new(AppState {
        collector,
        detector,
        cache,
        upbit,
        symbols,
    });

    // Router
    let app = Router::new()
        .route("/pools/cached", get(get_cached_pools))
        .route("/arbitrage", get(get_arbitrage))
        .route("/health", get(health))
        .route("/stats", get(get_stats))
        .route("/ws", get(ws_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    println!("\n✓ Server ready on http://{}\n", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// REST Handlers
async fn get_cached_pools(
    State(state): State<Arc<AppState>>
) -> axum::Json<Vec<models::PoolData>> {
    // Arc를 벗겨서 PoolData 직접 반환
    let pools: Vec<models::PoolData> = state.cache.get_all()
        .into_iter()
        .map(|arc_pool| (*arc_pool).clone())
        .collect();
    axum::Json(pools)
}

async fn get_arbitrage(
    State(state): State<Arc<AppState>>
) -> axum::Json<Vec<models::ArbitrageAlert>> {
    let pools = state.cache.get_all();
    let cex_prices = state.upbit.get_all_prices();
    
    let mut alerts = state.detector.detect_dex_dex(&pools);
    alerts.extend(state.detector.detect_dex_cex(&pools, &cex_prices));
    
    axum::Json(alerts)
}

async fn get_stats(
    State(state): State<Arc<AppState>>
) -> axum::Json<serde_json::Value> {
    let stats = state.collector.get_stats();
    
    axum::Json(serde_json::json!({
        "cache_pools": state.cache.len(),
        "symbols": state.symbols.len(),
        "total_requests": stats.total_requests.load(Ordering::Relaxed),
        "successful": stats.successful.load(Ordering::Relaxed),
        "failed": stats.failed.load(Ordering::Relaxed),
        "pools_collected": stats.pools_collected.load(Ordering::Relaxed),
        "upbit_prices": state.upbit.get_all_prices().len(),
    }))
}

async fn health() -> &'static str {
    "OK"
}

// WebSocket Handler
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut update_ticker = interval(Duration::from_secs(30));
    let mut heartbeat_ticker = interval(Duration::from_secs(10));

    loop {
        tokio::select! {
            _ = update_ticker.tick() => {
                let pools = state.cache.get_all();
                
                // Send in chunks - convert Arc to references for serialization
                for chunk in pools.chunks(50) {
                    let chunk_data: Vec<&models::PoolData> = chunk.iter()
                        .map(|arc| arc.as_ref())
                        .collect();
                    
                    let msg = serde_json::json!({
                        "type": "pool_update",
                        "data": chunk_data,
                    });
                    
                    match tokio::time::timeout(
                        Duration::from_secs(5),
                        sender.send(Message::Text(msg.to_string()))
                    ).await {
                        Ok(Ok(_)) => {},
                        _ => return,
                    }
                }

                // Arbitrage alerts
                let cex_prices = state.upbit.get_all_prices();
                let alerts = state.detector.detect_dex_dex(&pools);
                
                if !alerts.is_empty() {
                    let msg = serde_json::json!({
                        "type": "arb_alert",
                        "data": alerts,
                    });
                    let _ = sender.send(Message::Text(msg.to_string())).await;
                }
            }

            _ = heartbeat_ticker.tick() => {
                if sender.send(Message::Ping(vec![])).await.is_err() {
                    return;
                }
            }

            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => return,
                    Some(Ok(Message::Pong(_))) => {},
                    _ => {}
                }
            }
        }
    }
}

