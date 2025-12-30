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

/// Gap monitor: Upbit vs DEX price comparison
async fn run_gap_monitor(upbit: &UpbitClient, symbols: &[String], threshold: f64) {
    use reqwest::Client;
    
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    
    println!("\n🔍 갭 모니터링 시작 (30초 간격)");
    println!("─────────────────────────────────────────────────────────");
    
    loop {
        let start = std::time::Instant::now();
        
        // Fetch Upbit prices and convert to HashMap for lookup
        let upbit_prices: std::collections::HashMap<String, f64> = upbit.get_all_prices()
            .into_iter()
            .map(|p| (p.symbol, p.price_usd))
            .collect();
        
        // Fetch DEX prices from DexScreener
        let mut gaps: Vec<(String, f64, f64, f64, String)> = Vec::new(); // (symbol, upbit, dex, gap%, source)
        
        for symbol in symbols.iter().take(50) { // Limit to 50 for speed
            let url = format!("https://api.dexscreener.com/latest/dex/search?q={}", symbol);
            
            if let Ok(resp) = client.get(&url).send().await {
                if resp.status().is_success() {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        if let Some(pairs) = data["pairs"].as_array() {
                            for pair in pairs.iter().take(5) {
                                let base_symbol = pair["baseToken"]["symbol"].as_str().unwrap_or("");
                                let price_str = pair["priceUsd"].as_str().unwrap_or("0");
                                let dex = pair["dexId"].as_str().unwrap_or("unknown");
                                let chain = pair["chainId"].as_str().unwrap_or("unknown");
                                
                                if base_symbol.to_uppercase() == symbol.to_uppercase() {
                                    if let Ok(dex_price) = price_str.parse::<f64>() {
                                        if dex_price > 0.0 && dex_price < 1_000_000_000.0 {
                                            // Get Upbit price
                                            if let Some(upbit_price) = upbit_prices.get(symbol) {
                                                let gap_pct = (*upbit_price - dex_price) / dex_price * 100.0;
                                                
                                                if gap_pct.abs() >= threshold * 100.0 {
                                                    gaps.push((
                                                        symbol.clone(),
                                                        *upbit_price,
                                                        dex_price,
                                                        gap_pct,
                                                        format!("{}:{}", dex, chain),
                                                    ));
                                                }
                                            }
                                            break; // One price per symbol is enough
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            
            // Small delay to avoid rate limiting
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        
        // Sort by gap percentage (descending by absolute value)
        gaps.sort_by(|a, b| b.3.abs().partial_cmp(&a.3.abs()).unwrap());
        
        let elapsed = start.elapsed();
        println!("\n⏱️  {} [{}개 갭 발견] ({:.2}초)",
            chrono::Local::now().format("%H:%M:%S"),
            gaps.len(),
            elapsed.as_secs_f64()
        );
        
        // Print gaps
        if gaps.is_empty() {
            println!("   갭 없음 (임계값 {:.1}% 이상)", threshold * 100.0);
        } else {
            println!("   {:8} {:>12} {:>12} {:>8} {}", "심볼", "업비트($)", "DEX($)", "갭(%)", "소스");
            println!("   ──────── ──────────── ──────────── ──────── ─────────────");
            for (symbol, upbit_price, dex_price, gap_pct, source) in gaps.iter().take(20) {
                let arrow = if *gap_pct > 0.0 { "↗️" } else { "↘️" };
                println!("   {:8} {:12.4} {:12.4} {:>+7.2}% {} {}",
                    symbol, upbit_price, dex_price, gap_pct, arrow, source);
            }
        }
        
        // Wait for next interval (30 seconds)
        let sleep_time = Duration::from_secs(30).saturating_sub(elapsed);
        if sleep_time > Duration::ZERO {
            tokio::time::sleep(sleep_time).await;
        }
    }
}

#[tokio::main(worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    
    // Check for --monitor flag
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
    
    // Check for --gap flag (Upbit vs DEX gap monitoring)
    if args.contains(&"--gap".to_string()) || args.contains(&"-g".to_string()) {
        println!("\n📊 Gap Monitor Mode (Upbit vs DEX)\n");
        
        // Load pools from saved data
        let mut monitor = PriceMonitor::new();
        let pools_path = Path::new("./data/pools");
        let loaded = monitor.load_pools(pools_path)?;
        println!("✓ {} 풀 로드 완료", loaded);
        
        // Initialize Upbit
        println!("📡 Connecting to Upbit...");
        let upbit = UpbitClient::new();
        let symbols = upbit.fetch_krw_coins().await?;
        println!("✓ {} KRW 페어 로드", symbols.len());
        
        // Get threshold from args (default 1.0%)
        let threshold = args.iter()
            .position(|a| a == "--threshold" || a == "-t")
            .and_then(|i| args.get(i + 1))
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(1.0) / 100.0;
        
        println!("✓ 갭 임계값: {:.1}%", threshold * 100.0);
        
        // Run gap monitoring loop
        run_gap_monitor(&upbit, &symbols, threshold).await;
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

