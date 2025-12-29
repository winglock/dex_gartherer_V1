use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use crate::models::PoolData;
use super::{PoolSource, SourceError};

// Token address map: symbol -> chain_id -> address
lazy_static::lazy_static! {
    static ref TOKEN_MAP: HashMap<&'static str, HashMap<u32, &'static str>> = {
        let mut m = HashMap::new();
        
        // BTC
        let mut btc = HashMap::new();
        btc.insert(1u32, "0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"); // WBTC ETH
        btc.insert(56u32, "0x7130d2A12B9BCbFAe4f2634d864A1Ee1Ce3Ead9c"); // BTCB BSC
        m.insert("BTC", btc);
        
        // ETH
        let mut eth = HashMap::new();
        eth.insert(1u32, "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"); // WETH
        eth.insert(56u32, "0x2170Ed0880ac9A755fd29B2688956BD959F933F8"); // ETH BSC
        eth.insert(137u32, "0x7ceB23fD6bC0adD59E62ac25578270cFf1b9f619"); // WETH Polygon
        m.insert("ETH", eth);
        
        // USDT
        let mut usdt = HashMap::new();
        usdt.insert(1u32, "0xdAC17F958D2ee523a2206206994597C13D831ec7");
        usdt.insert(56u32, "0x55d398326f99059fF775485246999027B3197955");
        usdt.insert(137u32, "0xc2132D05D31c914a87C6611C10748AEb04B58e8F");
        m.insert("USDT", usdt);
        
        // Add more common tokens
        let mut link = HashMap::new();
        link.insert(1u32, "0x514910771AF9Ca656af840dff83E8264EcF986CA");
        m.insert("LINK", link);
        
        let mut uni = HashMap::new();
        uni.insert(1u32, "0x1f9840a85d5aF5bf1D1762F925BDADdC4201F984");
        m.insert("UNI", uni);
        
        let mut aave = HashMap::new();
        aave.insert(1u32, "0x7Fc66500c84A76Ad7e9c93437bFc5Ac33E2DDaE9");
        m.insert("AAVE", aave);
        
        m
    };
}

fn get_token_address(symbol: &str, chain_id: u32) -> Option<&'static str> {
    TOKEN_MAP.get(symbol.to_uppercase().as_str())
        .and_then(|chains| chains.get(&chain_id))
        .copied()
}

fn get_usdc_address(chain_id: u32) -> &'static str {
    match chain_id {
        1 => "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
        56 => "0x8AC76a51cc950d9822D68b83fE1Ad97B32Cd580d",
        137 => "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174",
        _ => "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
    }
}

/// KyberSwap Direct API
pub struct KyberSwapDirectSource {
    client: Client,
}

impl KyberSwapDirectSource {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
        }
    }
}

#[async_trait]
impl PoolSource for KyberSwapDirectSource {
    fn name(&self) -> &'static str { "KyberSwap" }

    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        let mut pools = Vec::new();
        
        for (chain_name, chain_id) in [("ethereum", 1u32), ("bsc", 56), ("polygon", 137)] {
            let token_addr = match get_token_address(symbol, chain_id) {
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
                        // Extract route info
                        if let Some(route_summary) = data["data"]["routeSummary"].as_object() {
                            let amount_out = route_summary.get("amountOut")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<f64>().ok())
                                .map(|v| v / 1e6)
                                .unwrap_or(0.0);
                            
                            // Get pools from route
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
                                                    0.0,
                                                    0.0,
                                                    "kyberswap".to_string(),
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                            
                            // Fallback if no specific pools
                            if pools.is_empty() && amount_out > 0.0 {
                                pools.push(PoolData::new(
                                    symbol.to_string(),
                                    chain_name.to_string(),
                                    "kyberswap".to_string(),
                                    format!("kyber:{}:{}", chain_id, symbol),
                                    format!("{}/USDC", symbol),
                                    amount_out,
                                    0.0,
                                    0.0,
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

/// OpenOcean Direct API (free, no API key)
pub struct OpenOceanDirectSource {
    client: Client,
}

impl OpenOceanDirectSource {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
        }
    }
}

#[async_trait]
impl PoolSource for OpenOceanDirectSource {
    fn name(&self) -> &'static str { "OpenOcean" }

    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        let mut pools = Vec::new();
        
        for (chain, chain_id, chain_name) in [("eth", 1u32, "ethereum"), ("bsc", 56, "bsc"), ("polygon", 137, "polygon")] {
            let token_addr = match get_token_address(symbol, chain_id) {
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
                                
                                // Get path/routes if available
                                if let Some(path) = data["data"]["path"].as_object() {
                                    if let Some(routes) = path.get("routes").and_then(|r| r.as_array()) {
                                        for route in routes {
                                            if let Some(subs) = route["subRoutes"].as_array() {
                                                for sub in subs {
                                                    let dex = sub["dexRouter"]["name"].as_str().unwrap_or("openocean");
                                                    let addr = sub["dexRouter"]["address"].as_str().unwrap_or("");
                                                    
                                                    if !addr.is_empty() {
                                                        pools.push(PoolData::new(
                                                            symbol.to_string(),
                                                            chain_name.to_string(),
                                                            dex.to_string(),
                                                            addr.to_string(),
                                                            format!("{}/USDC", symbol),
                                                            price,
                                                            0.0,
                                                            0.0,
                                                            "openocean".to_string(),
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                
                                // Fallback
                                if pools.is_empty() && price > 0.0 {
                                    pools.push(PoolData::new(
                                        symbol.to_string(),
                                        chain_name.to_string(),
                                        "openocean".to_string(),
                                        format!("openocean:{}:{}", chain_id, symbol),
                                        format!("{}/USDC", symbol),
                                        price,
                                        0.0,
                                        0.0,
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

/// ParaSwap Direct API (free)
pub struct ParaSwapDirectSource {
    client: Client,
}

impl ParaSwapDirectSource {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
        }
    }
}

#[async_trait]
impl PoolSource for ParaSwapDirectSource {
    fn name(&self) -> &'static str { "ParaSwap" }

    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        let mut pools = Vec::new();
        
        for (chain_id, chain_name) in [(1u32, "ethereum"), (56, "bsc"), (137, "polygon")] {
            let token_addr = match get_token_address(symbol, chain_id) {
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
                                
                                // Get best route pools
                                if let Some(best_route) = data["priceRoute"]["bestRoute"].as_array() {
                                    for step in best_route {
                                        if let Some(swaps) = step["swaps"].as_array() {
                                            for swap in swaps {
                                                if let Some(exchanges) = swap["swapExchanges"].as_array() {
                                                    for exchange in exchanges {
                                                        let dex = exchange["exchange"].as_str().unwrap_or("");
                                                        let pool_addrs = exchange["poolAddresses"].as_array();
                                                        
                                                        if let Some(addrs) = pool_addrs {
                                                            for addr in addrs {
                                                                if let Some(a) = addr.as_str() {
                                                                    pools.push(PoolData::new(
                                                                        symbol.to_string(),
                                                                        chain_name.to_string(),
                                                                        dex.to_string(),
                                                                        a.to_string(),
                                                                        format!("{}/USDC", symbol),
                                                                        price,
                                                                        0.0,
                                                                        0.0,
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
                                
                                // Fallback
                                if pools.is_empty() && price > 0.0 {
                                    pools.push(PoolData::new(
                                        symbol.to_string(),
                                        chain_name.to_string(),
                                        "paraswap".to_string(),
                                        format!("paraswap:{}:{}", chain_id, symbol),
                                        format!("{}/USDC", symbol),
                                        price,
                                        0.0,
                                        0.0,
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
