use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use futures::stream::{self, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use crate::models::PoolData;

/// Pool info loaded from saved JSON files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedPool {
    pub symbol: String,
    pub chain: String,
    pub dex: String,
    pub pool_address: String,
    pub pair: String,
    pub source: String,
}

/// Price data from DEX
#[derive(Debug, Clone)]
pub struct PriceData {
    pub symbol: String,
    pub chain: String,
    pub dex: String,
    pub pool_address: String,
    pub price_usd: f64,
    pub timestamp: u64,
}

/// Price monitor for real-time price tracking
pub struct PriceMonitor {
    client: Client,
    pools: Vec<SavedPool>,
    semaphore: Arc<Semaphore>,
}

impl PriceMonitor {
    /// Create new price monitor with 20 concurrent requests
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            pools: Vec::new(),
            semaphore: Arc::new(Semaphore::new(20)),
        }
    }

    /// Load saved pool data from JSON files
    pub fn load_pools(&mut self, pools_dir: &Path) -> Result<usize, std::io::Error> {
        let mut loaded = 0;
        
        for entry in std::fs::read_dir(pools_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(pools) = serde_json::from_str::<Vec<PoolData>>(&content) {
                        for pool in pools {
                            // Only add pools with valid addresses (not synthetic)
                            if pool.pool_address.starts_with("0x") && pool.pool_address.len() == 42 {
                                self.pools.push(SavedPool {
                                    symbol: pool.symbol,
                                    chain: pool.chain,
                                    dex: pool.dex,
                                    pool_address: pool.pool_address,
                                    pair: pool.pair,
                                    source: pool.source,
                                });
                                loaded += 1;
                            }
                        }
                    }
                }
            }
        }
        
        Ok(loaded)
    }

    /// Get unique symbols
    pub fn get_symbols(&self) -> Vec<String> {
        let mut symbols: Vec<String> = self.pools.iter()
            .map(|p| p.symbol.clone())
            .collect();
        symbols.sort();
        symbols.dedup();
        symbols
    }

    /// Get pool count
    pub fn pool_count(&self) -> usize {
        self.pools.len()
    }

    /// Fetch prices for all pools in parallel
    pub async fn fetch_all_prices(&self) -> Vec<PriceData> {
        let prices: Vec<PriceData> = stream::iter(self.pools.iter().cloned())
            .map(|pool| {
                let client = self.client.clone();
                let semaphore = self.semaphore.clone();
                
                async move {
                    let _permit = semaphore.acquire().await.unwrap();
                    
                    // Use GeckoTerminal for on-chain price
                    let price = Self::fetch_pool_price(&client, &pool).await;
                    
                    PriceData {
                        symbol: pool.symbol,
                        chain: pool.chain,
                        dex: pool.dex,
                        pool_address: pool.pool_address,
                        price_usd: price.unwrap_or(0.0),
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                    }
                }
            })
            .buffer_unordered(20)
            .collect()
            .await;
        
        prices
    }

    /// Fetch price for a single pool from GeckoTerminal
    async fn fetch_pool_price(client: &Client, pool: &SavedPool) -> Option<f64> {
        let network = match pool.chain.as_str() {
            "ethereum" | "eth" => "eth",
            "bsc" => "bsc",
            "polygon" => "polygon_pos",
            "arbitrum" => "arbitrum",
            "base" => "base",
            "optimism" => "optimism",
            "avalanche" => "avax",
            _ => return None,
        };
        
        let url = format!(
            "https://api.geckoterminal.com/api/v2/networks/{}/pools/{}",
            network, pool.pool_address
        );
        
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                if let Ok(data) = resp.json::<serde_json::Value>().await {
                    if let Some(price_str) = data["data"]["attributes"]["base_token_price_usd"].as_str() {
                        return price_str.parse::<f64>().ok();
                    }
                }
            }
        }
        
        None
    }

    /// Run continuous price monitoring loop
    pub async fn run_continuous(&self, interval_secs: u64) {
        println!("\n🔄 가격 모니터링 시작 ({}초 간격)", interval_secs);
        println!("📊 총 {}개 풀, {}개 심볼 추적 중", self.pool_count(), self.get_symbols().len());
        println!("─────────────────────────────────────────");
        
        loop {
            let start = std::time::Instant::now();
            
            let prices = self.fetch_all_prices().await;
            let valid_prices: Vec<_> = prices.iter().filter(|p| p.price_usd > 0.0).collect();
            
            // Group by symbol
            let mut by_symbol: HashMap<String, Vec<&PriceData>> = HashMap::new();
            for price in valid_prices.iter() {
                by_symbol.entry(price.symbol.clone()).or_default().push(price);
            }
            
            // Print summary
            let elapsed = start.elapsed();
            println!("\n⏱️  {} - {}개 가격 수집 ({:.2}초)", 
                chrono::Local::now().format("%H:%M:%S"),
                valid_prices.len(),
                elapsed.as_secs_f64()
            );
            
            // Print top symbols with prices
            let mut symbols: Vec<_> = by_symbol.keys().collect();
            symbols.sort();
            
            for symbol in symbols.iter().take(10) {
                if let Some(prices) = by_symbol.get(*symbol) {
                    let avg_price: f64 = prices.iter().map(|p| p.price_usd).sum::<f64>() / prices.len() as f64;
                    println!("  {} ${:.4} ({} 풀)", symbol, avg_price, prices.len());
                }
            }
            
            if symbols.len() > 10 {
                println!("  ... 외 {}개 심볼", symbols.len() - 10);
            }
            
            // Wait for next interval
            let sleep_time = Duration::from_secs(interval_secs).saturating_sub(elapsed);
            if sleep_time > Duration::ZERO {
                tokio::time::sleep(sleep_time).await;
            }
        }
    }
}
