use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use parking_lot::RwLock;
use crate::models::PoolData;
use super::{PoolSource, SourceError};

/// Shared token address cache (symbol -> chain_id -> address)
pub type TokenCache = Arc<RwLock<HashMap<String, HashMap<u32, String>>>>;

/// Get USDC address for chain
fn get_usdc_address(chain_id: u32) -> &'static str {
    match chain_id {
        1 => "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
        56 => "0x8AC76a51cc950d9822D68b83fE1Ad97B32Cd580d",
        137 => "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174",
        42161 => "0xaf88d065e77c8cC2239327C5EDb3A432268e5831",
        10 => "0x7F5c764cBc14f9669B88837ca1490cCa17c31607",
        8453 => "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
        43114 => "0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E",
        _ => "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
    }
}

/// Load consolidated token data from JSON file
fn load_token_data() -> HashMap<String, HashMap<u32, String>> {
    let json_str = include_str!("../../matcha_tokens_consolidated.json");
    
    // Parse JSON: {"SYMBOL": {"chainId": "address", ...}, ...}
    let raw: HashMap<String, HashMap<String, String>> = 
        serde_json::from_str(json_str).unwrap_or_default();
    
    // Convert string chain IDs to u32
    raw.into_iter()
        .map(|(symbol, chains)| {
            let converted: HashMap<u32, String> = chains.into_iter()
                .filter_map(|(chain_str, addr)| {
                    chain_str.parse::<u32>().ok().map(|id| (id, addr))
                })
                .collect();
            (symbol, converted)
        })
        .collect()
}

/// Create a new shared token cache pre-loaded with Matcha data
pub fn new_token_cache() -> TokenCache {
    Arc::new(RwLock::new(load_token_data()))
}

/// Matcha Token Resolver - uses pre-loaded data from consolidated JSON
pub struct MatchaTokenResolver {
    cache: TokenCache,
}

impl MatchaTokenResolver {
    pub fn new(cache: TokenCache) -> Self {
        Self { cache }
    }

    pub fn resolve(&self, symbol: &str) -> HashMap<u32, String> {
        let cache = self.cache.read();
        cache.get(&symbol.to_uppercase())
            .cloned()
            .unwrap_or_default()
    }
}

#[async_trait]
impl PoolSource for MatchaTokenResolver {
    fn name(&self) -> &'static str { "Matcha" }

    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        let addresses = self.resolve(symbol);
        
        let pools: Vec<PoolData> = addresses.iter()
            .map(|(chain_id, address)| {
                let chain_name = match chain_id {
                    1 => "ethereum",
                    56 => "bsc",
                    137 => "polygon",
                    42161 => "arbitrum",
                    10 => "optimism",
                    8453 => "base",
                    43114 => "avalanche",
                    130 => "unichain",
                    _ => "other",
                };
                
                PoolData::new(
                    symbol.to_string(),
                    chain_name.to_string(),
                    "matcha-data".to_string(),
                    address.clone(),
                    format!("{} token", symbol),
                    0.0, 0.0, 0.0,
                    "matcha".to_string(),
                )
            })
            .collect();
        
        Ok(pools)
    }
}

/// KyberSwap with static token data
pub struct KyberSwapDirectSource {
    client: Client,
    cache: TokenCache,
}

impl KyberSwapDirectSource {
    pub fn new(cache: TokenCache) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
            cache,
        }
    }

    fn get_token_address(&self, symbol: &str, chain_id: u32) -> Option<String> {
        let cache = self.cache.read();
        cache.get(&symbol.to_uppercase())
            .and_then(|chains| chains.get(&chain_id))
            .cloned()
    }
}

#[async_trait]
impl PoolSource for KyberSwapDirectSource {
    fn name(&self) -> &'static str { "KyberSwap" }

    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        let mut pools = Vec::new();
        
        for (chain_name, chain_id) in [("ethereum", 1u32), ("bsc", 56), ("polygon", 137), ("arbitrum", 42161)] {
            let token_addr = match self.get_token_address(symbol, chain_id) {
                Some(addr) => addr,
                None => continue,
            };
            
            let usdc = get_usdc_address(chain_id);
            let url = format!(
                "https://aggregator-api.kyberswap.com/{}/api/v1/routes?tokenIn={}&tokenOut={}&amountIn=1000000000000000000",
                chain_name, token_addr, usdc
            );

            if let Ok(resp) = self.client.get(&url).send().await {
                if resp.status().is_success() {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        if let Some(route_summary) = data["data"]["routeSummary"].as_object() {
                            let amount_out = route_summary.get("amountOut")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<f64>().ok())
                                .map(|v| v / 1e6)
                                .unwrap_or(0.0);
                            
                            if let Some(route) = data["data"]["routeSummary"]["route"].as_array() {
                                for (i, leg) in route.iter().enumerate() {
                                    if let Some(swaps) = leg.as_array() {
                                        for swap in swaps {
                                            let pool = swap["pool"].as_str().unwrap_or("");
                                            let exchange = swap["exchange"].as_str().unwrap_or("kyberswap");
                                            
                                            if !pool.is_empty() {
                                                pools.push(PoolData::new(
                                                    symbol.to_string(),
                                                    chain_name.to_string(),
                                                    exchange.to_string(),
                                                    pool.to_string(),
                                                    format!("{}/USDC (hop {})", symbol, i + 1),
                                                    amount_out,
                                                    0.0, 0.0,
                                                    "kyberswap".to_string(),
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                            
                            if pools.is_empty() && amount_out > 0.0 {
                                pools.push(PoolData::new(
                                    symbol.to_string(),
                                    chain_name.to_string(),
                                    "kyberswap".to_string(),
                                    format!("kyber:{}:{}", chain_id, symbol),
                                    format!("{}/USDC", symbol),
                                    amount_out,
                                    0.0, 0.0,
                                    "kyberswap".to_string(),
                                ));
                            }
                        }
                    }
                }
            }
        }
        
        Ok(pools)
    }
}

/// OpenOcean with static token data
pub struct OpenOceanDirectSource {
    client: Client,
    cache: TokenCache,
}

impl OpenOceanDirectSource {
    pub fn new(cache: TokenCache) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
            cache,
        }
    }

    fn get_token_address(&self, symbol: &str, chain_id: u32) -> Option<String> {
        let cache = self.cache.read();
        cache.get(&symbol.to_uppercase())
            .and_then(|chains| chains.get(&chain_id))
            .cloned()
    }
}

#[async_trait]
impl PoolSource for OpenOceanDirectSource {
    fn name(&self) -> &'static str { "OpenOcean" }

    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        let mut pools = Vec::new();
        
        for (chain, chain_id, chain_name) in [
            ("eth", 1u32, "ethereum"), 
            ("bsc", 56, "bsc"), 
            ("polygon", 137, "polygon"),
            ("arbitrum", 42161, "arbitrum"),
            ("base", 8453, "base"),
        ] {
            let token_addr = match self.get_token_address(symbol, chain_id) {
                Some(addr) => addr,
                None => continue,
            };
            
            let usdc = get_usdc_address(chain_id);
            let url = format!(
                "https://open-api.openocean.finance/v3/{}/quote?inTokenAddress={}&outTokenAddress={}&amount=1000000000000000000&gasPrice=5",
                chain, token_addr, usdc
            );

            if let Ok(resp) = self.client.get(&url).send().await {
                if resp.status().is_success() {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        if let Some(out_amount) = data["data"]["outAmount"].as_str() {
                            if let Ok(price) = out_amount.parse::<f64>() {
                                let price = price / 1e6;
                                
                                if price > 0.0 {
                                    pools.push(PoolData::new(
                                        symbol.to_string(),
                                        chain_name.to_string(),
                                        "openocean".to_string(),
                                        format!("openocean:{}:{}", chain_id, symbol),
                                        format!("{}/USDC", symbol),
                                        price,
                                        0.0, 0.0,
                                        "openocean".to_string(),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        
        Ok(pools)
    }
}

/// ParaSwap with static token data
pub struct ParaSwapDirectSource {
    client: Client,
    cache: TokenCache,
}

impl ParaSwapDirectSource {
    pub fn new(cache: TokenCache) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
            cache,
        }
    }

    fn get_token_address(&self, symbol: &str, chain_id: u32) -> Option<String> {
        let cache = self.cache.read();
        cache.get(&symbol.to_uppercase())
            .and_then(|chains| chains.get(&chain_id))
            .cloned()
    }
}

#[async_trait]
impl PoolSource for ParaSwapDirectSource {
    fn name(&self) -> &'static str { "ParaSwap" }

    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        let mut pools = Vec::new();
        
        for (chain_id, chain_name) in [(1u32, "ethereum"), (56, "bsc"), (137, "polygon"), (42161, "arbitrum")] {
            let token_addr = match self.get_token_address(symbol, chain_id) {
                Some(addr) => addr,
                None => continue,
            };
            
            let usdc = get_usdc_address(chain_id);
            let url = format!(
                "https://apiv5.paraswap.io/prices?srcToken={}&destToken={}&amount=1000000000000000000&srcDecimals=18&destDecimals=6&network={}",
                token_addr, usdc, chain_id
            );

            if let Ok(resp) = self.client.get(&url).send().await {
                if resp.status().is_success() {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        if let Some(dest_amount) = data["priceRoute"]["destAmount"].as_str() {
                            if let Ok(price) = dest_amount.parse::<f64>() {
                                let price = price / 1e6;
                                
                                if let Some(best_route) = data["priceRoute"]["bestRoute"].as_array() {
                                    for step in best_route {
                                        if let Some(swaps) = step["swaps"].as_array() {
                                            for swap in swaps {
                                                if let Some(exchanges) = swap["swapExchanges"].as_array() {
                                                    for exchange in exchanges {
                                                        let dex = exchange["exchange"].as_str().unwrap_or("paraswap");
                                                        if let Some(addrs) = exchange["poolAddresses"].as_array() {
                                                            for addr in addrs {
                                                                if let Some(a) = addr.as_str() {
                                                                    pools.push(PoolData::new(
                                                                        symbol.to_string(),
                                                                        chain_name.to_string(),
                                                                        dex.to_string(),
                                                                        a.to_string(),
                                                                        format!("{}/USDC", symbol),
                                                                        price,
                                                                        0.0, 0.0,
                                                                        "paraswap".to_string(),
                                                                    ));
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                
                                if pools.is_empty() && price > 0.0 {
                                    pools.push(PoolData::new(
                                        symbol.to_string(),
                                        chain_name.to_string(),
                                        "paraswap".to_string(),
                                        format!("paraswap:{}:{}", chain_id, symbol),
                                        format!("{}/USDC", symbol),
                                        price,
                                        0.0, 0.0,
                                        "paraswap".to_string(),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        
        Ok(pools)
    }
}
