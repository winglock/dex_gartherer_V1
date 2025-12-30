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
    /// Create new price monitor with 50 concurrent requests for speed
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            pools: Vec::new(),
            semaphore: Arc::new(Semaphore::new(50)), // Increased for speed
        }
    }

    /// Load saved pool data from JSON files with validation
    pub fn load_pools(&mut self, pools_dir: &Path) -> Result<usize, std::io::Error> {
        let mut loaded = 0;
        let mut skipped = 0;
        
        for entry in std::fs::read_dir(pools_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(pools) = serde_json::from_str::<Vec<PoolData>>(&content) {
                        for pool in pools {
                            // Validation: skip invalid pools
                            if !Self::is_valid_pool(&pool) {
                                skipped += 1;
                                continue;
                            }
                            
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
        
        println!("   ({}개 비정상 풀 제외됨)", skipped);
        Ok(loaded)
    }

    /// Validate pool data
    fn is_valid_pool(pool: &PoolData) -> bool {
        // Must have valid 0x address
        if !pool.pool_address.starts_with("0x") || pool.pool_address.len() != 42 {
            return false;
        }
        
        // Skip synthetic/fake addresses
        if pool.pool_address.contains("kyber:") || 
           pool.pool_address.contains("openocean:") ||
           pool.pool_address.contains("paraswap:") {
            return false;
        }
        
        // Skip abnormal prices (too high = likely parsing error)
        if pool.price_usd > 1_000_000_000.0 {
            return false;
        }
        
        // Skip if symbol not in pair
        let pair_upper = pool.pair.to_uppercase();
        let symbol_upper = pool.symbol.to_uppercase();
        if !pair_upper.contains(&symbol_upper) {
            return false;
        }
        
        true
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

    /// Fetch prices using DexScreener API (much faster - by symbol)
    pub async fn fetch_all_prices(&self) -> Vec<PriceData> {
        let symbols = self.get_symbols();
        
        // Fetch prices by symbol using DexScreener (batch)
        let symbol_prices: Vec<(String, HashMap<String, f64>)> = stream::iter(symbols.into_iter())
            .map(|symbol| {
                let client = self.client.clone();
                let semaphore = self.semaphore.clone();
                
                async move {
                    let _permit = semaphore.acquire().await.unwrap();
                    let prices = Self::fetch_symbol_prices(&client, &symbol).await;
                    (symbol, prices)
                }
            })
            .buffer_unordered(50)
            .collect()
            .await;
        
        // Build price lookup
        let mut price_map: HashMap<String, HashMap<String, f64>> = HashMap::new();
        for (symbol, prices) in symbol_prices {
            price_map.insert(symbol, prices);
        }
        
        // Map pools to prices
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        self.pools.iter()
            .filter_map(|pool| {
                let price = price_map.get(&pool.symbol)
                    .and_then(|chain_prices| chain_prices.get(&pool.chain))
                    .copied()
                    .unwrap_or(0.0);
                
                if price > 0.0 {
                    Some(PriceData {
                        symbol: pool.symbol.clone(),
                        chain: pool.chain.clone(),
                        dex: pool.dex.clone(),
                        pool_address: pool.pool_address.clone(),
                        price_usd: price,
                        timestamp,
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    /// Fetch prices for a symbol from DexScreener
    async fn fetch_symbol_prices(client: &Client, symbol: &str) -> HashMap<String, f64> {
        let mut prices = HashMap::new();
        
        let url = format!("https://api.dexscreener.com/latest/dex/search?q={}", symbol);
        
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                if let Ok(data) = resp.json::<serde_json::Value>().await {
                    if let Some(pairs) = data["pairs"].as_array() {
                        for pair in pairs.iter().take(20) {
                            let chain_id = pair["chainId"].as_str().unwrap_or("");
                            let price_str = pair["priceUsd"].as_str().unwrap_or("0");
                            let base_symbol = pair["baseToken"]["symbol"].as_str().unwrap_or("");
                            
                            // Only use exact symbol match
                            if base_symbol.to_uppercase() == symbol.to_uppercase() {
                                if let Ok(price) = price_str.parse::<f64>() {
                                    if price > 0.0 && price < 1_000_000_000.0 {
                                        let chain = match chain_id {
                                            "ethereum" => "ethereum",
                                            "bsc" => "bsc",
                                            "polygon" => "polygon",
                                            "arbitrum" => "arbitrum",
                                            "base" => "base",
                                            "optimism" => "optimism",
                                            "avalanche" => "avalanche",
                                            _ => chain_id,
                                        };
                                        prices.entry(chain.to_string()).or_insert(price);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        
        prices
    }

    /// Run continuous price monitoring loop
    pub async fn run_continuous(&self, interval_secs: u64) {
        println!("\n🔄 가격 모니터링 시작 ({}초 간격)", interval_secs);
        println!("📊 총 {}개 풀, {}개 심볼 추적 중", self.pool_count(), self.get_symbols().len());
        println!("─────────────────────────────────────────");
        
        loop {
            let start = std::time::Instant::now();
            
            let prices = self.fetch_all_prices().await;
            
            // Group by symbol and calculate average
            let mut by_symbol: HashMap<String, Vec<&PriceData>> = HashMap::new();
            for price in prices.iter() {
                by_symbol.entry(price.symbol.clone()).or_default().push(price);
            }
            
            // Print summary
            let elapsed = start.elapsed();
            println!("\n⏱️  {} - {}개 가격 수집 ({:.2}초)", 
                chrono::Local::now().format("%H:%M:%S"),
                prices.len(),
                elapsed.as_secs_f64()
            );
            
            // Print symbols with prices (sorted by price descending)
            let mut symbol_prices: Vec<(String, f64, usize)> = by_symbol.iter()
                .map(|(sym, p)| {
                    let avg = p.iter().map(|x| x.price_usd).sum::<f64>() / p.len() as f64;
                    (sym.clone(), avg, p.len())
                })
                .collect();
            symbol_prices.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            
            for (symbol, avg_price, pool_count) in symbol_prices.iter().take(15) {
                println!("  {:8} ${:>12.4} ({:>2} 풀)", symbol, avg_price, pool_count);
            }
            
            if symbol_prices.len() > 15 {
                println!("  ... 외 {}개 심볼", symbol_prices.len() - 15);
            }
            
            // Wait for next interval
            let sleep_time = Duration::from_secs(interval_secs).saturating_sub(elapsed);
            if sleep_time > Duration::ZERO {
                tokio::time::sleep(sleep_time).await;
            }
        }
    }
}
