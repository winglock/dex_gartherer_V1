use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use crate::models::PoolData;
use super::{PoolSource, SourceError};

/// 1inch Aggregator - extracts pool addresses from route
pub struct OneInchSource {
    client: Client,
}

impl OneInchSource {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap(),
        }
    }

    async fn fetch_chain(&self, chain_id: u32, symbol: &str) -> Vec<PoolData> {
        let chain_name = match chain_id {
            1 => "ethereum",
            56 => "bsc",
            137 => "polygon",
            42161 => "arbitrum",
            10 => "optimism",
            8453 => "base",
            _ => return vec![],
        };

        // 1inch Quote API with protocols info
        let url = format!(
            "https://api.1inch.dev/swap/v6.0/{}/quote?src=0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE&dst=0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48&amount=1000000000000000000&includeProtocols=true",
            chain_id
        );

        match self.client.get(&url)
            .header("Authorization", "Bearer ")
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(data) = resp.json::<serde_json::Value>().await {
                    let mut pools = Vec::new();
                    
                    // Extract pool addresses from protocols
                    if let Some(protocols) = data["protocols"].as_array() {
                        for protocol_group in protocols {
                            if let Some(steps) = protocol_group.as_array() {
                                for step in steps {
                                    if let Some(swaps) = step.as_array() {
                                        for swap in swaps {
                                            let pool_addr = swap["name"].as_str()
                                                .map(|n| n.to_string())
                                                .unwrap_or_default();
                                            let dex_name = swap["part"].as_u64()
                                                .map(|_| "1inch-route".to_string())
                                                .unwrap_or_else(|| "1inch".to_string());
                                            
                                            if !pool_addr.is_empty() {
                                                let price = data["dstAmount"].as_str()
                                                    .and_then(|s| s.parse::<f64>().ok())
                                                    .map(|a| a / 1e6)
                                                    .unwrap_or(0.0);
                                                
                                                pools.push(PoolData::new(
                                                    symbol.to_string(),
                                                    chain_name.to_string(),
                                                    dex_name,
                                                    format!("1inch:{}:{}", chain_id, pool_addr),
                                                    format!("{}/USDC", symbol),
                                                    price,
                                                    0.0,
                                                    0.0,
                                                    "1inch".to_string(),
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    
                    // Fallback if no protocols found
                    if pools.is_empty() {
                        if let Some(dst_amount) = data["dstAmount"].as_str() {
                            if let Ok(amount) = dst_amount.parse::<f64>() {
                                pools.push(PoolData::new(
                                    symbol.to_string(),
                                    chain_name.to_string(),
                                    "1inch".to_string(),
                                    format!("1inch:{}:aggregated", chain_id),
                                    format!("{}/USDC", symbol),
                                    amount / 1e6,
                                    0.0,
                                    0.0,
                                    "1inch".to_string(),
                                ));
                            }
                        }
                    }
                    return pools;
                }
            }
            _ => {}
        }
        vec![]
    }
}

#[async_trait]
impl PoolSource for OneInchSource {
    fn name(&self) -> &'static str { "1inch" }

    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        let mut pools = Vec::new();
        for chain_id in [1, 56, 137, 42161, 10] {
            pools.extend(self.fetch_chain(chain_id, symbol).await);
        }
        Ok(pools)
    }
}

/// ParaSwap Aggregator - extracts pool from bestRoute
pub struct ParaSwapSource {
    client: Client,
}

impl ParaSwapSource {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap(),
        }
    }
}

#[async_trait]
impl PoolSource for ParaSwapSource {
    fn name(&self) -> &'static str { "ParaSwap" }

    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        let mut pools = Vec::new();
        
        for (chain_id, chain_name) in [(1, "ethereum"), (56, "bsc"), (137, "polygon")] {
            let url = format!(
                "https://apiv5.paraswap.io/prices?srcToken=0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE&destToken=0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48&amount=1000000000000000000&srcDecimals=18&destDecimals=6&network={}",
                chain_id
            );

            if let Ok(resp) = self.client.get(&url).send().await {
                if resp.status().is_success() {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        // Extract from bestRoute
                        if let Some(route) = data["priceRoute"]["bestRoute"].as_array() {
                            for step in route {
                                if let Some(swaps) = step["swaps"].as_array() {
                                    for swap in swaps {
                                        if let Some(exchanges) = swap["swapExchanges"].as_array() {
                                            for exchange in exchanges {
                                                let pool_addr = exchange["poolAddresses"].as_array()
                                                    .and_then(|arr| arr.first())
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("");
                                                let dex = exchange["exchange"].as_str().unwrap_or("paraswap");
                                                
                                                if !pool_addr.is_empty() {
                                                    let price = data["priceRoute"]["destAmount"].as_str()
                                                        .and_then(|s| s.parse::<f64>().ok())
                                                        .map(|a| a / 1e6)
                                                        .unwrap_or(0.0);
                                                    
                                                    pools.push(PoolData::new(
                                                        symbol.to_string(),
                                                        chain_name.to_string(),
                                                        dex.to_string(),
                                                        pool_addr.to_string(),
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
                        
                        // Fallback
                        if pools.is_empty() {
                            if let Some(amount) = data["priceRoute"]["destAmount"].as_str() {
                                if let Ok(val) = amount.parse::<f64>() {
                                    pools.push(PoolData::new(
                                        symbol.to_string(),
                                        chain_name.to_string(),
                                        "paraswap".to_string(),
                                        format!("paraswap:{}:aggregated", chain_id),
                                        format!("{}/USDC", symbol),
                                        val / 1e6,
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

/// KyberSwap Aggregator - extracts pool from route
pub struct KyberSwapSource {
    client: Client,
}

impl KyberSwapSource {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap(),
        }
    }
}

#[async_trait]
impl PoolSource for KyberSwapSource {
    fn name(&self) -> &'static str { "KyberSwap" }

    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        let mut pools = Vec::new();

        for (chain, chain_name) in [("ethereum", "ethereum"), ("bsc", "bsc"), ("polygon", "polygon")] {
            let url = format!(
                "https://aggregator-api.kyberswap.com/{}/api/v1/routes?tokenIn=0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE&tokenOut=0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48&amountIn=1000000000000000000",
                chain
            );

            if let Ok(resp) = self.client.get(&url).send().await {
                if resp.status().is_success() {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        // Extract pools from route
                        if let Some(route) = data["data"]["routeSummary"]["route"].as_array() {
                            for leg in route {
                                if let Some(sub_routes) = leg.as_array() {
                                    for sub in sub_routes {
                                        let pool_addr = sub["pool"].as_str().unwrap_or("");
                                        let dex = sub["exchange"].as_str().unwrap_or("kyberswap");
                                        
                                        if !pool_addr.is_empty() {
                                            let price = data["data"]["routeSummary"]["amountOut"].as_str()
                                                .and_then(|s| s.parse::<f64>().ok())
                                                .map(|a| a / 1e6)
                                                .unwrap_or(0.0);
                                            
                                            pools.push(PoolData::new(
                                                symbol.to_string(),
                                                chain_name.to_string(),
                                                dex.to_string(),
                                                pool_addr.to_string(),
                                                format!("{}/USDC", symbol),
                                                price,
                                                0.0,
                                                0.0,
                                                "kyberswap".to_string(),
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        
                        // Fallback
                        if pools.is_empty() {
                            if let Some(amount) = data["data"]["routeSummary"]["amountOut"].as_str() {
                                if let Ok(val) = amount.parse::<f64>() {
                                    pools.push(PoolData::new(
                                        symbol.to_string(),
                                        chain_name.to_string(),
                                        "kyberswap".to_string(),
                                        format!("kyber:{}:aggregated", chain),
                                        format!("{}/USDC", symbol),
                                        val / 1e6,
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
        }
        Ok(pools)
    }
}

/// OpenOcean Aggregator - extracts pool from path
pub struct OpenOceanSource {
    client: Client,
}

impl OpenOceanSource {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap(),
        }
    }
}

#[async_trait]
impl PoolSource for OpenOceanSource {
    fn name(&self) -> &'static str { "OpenOcean" }

    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        let mut pools = Vec::new();

        for (chain, chain_name) in [("eth", "ethereum"), ("bsc", "bsc"), ("polygon", "polygon")] {
            let url = format!(
                "https://open-api.openocean.finance/v3/{}/quote?inTokenAddress=0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE&outTokenAddress=0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48&amount=1000000000000000000&gasPrice=5",
                chain
            );

            if let Ok(resp) = self.client.get(&url).send().await {
                if resp.status().is_success() {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        // Extract pools from path
                        if let Some(path) = data["data"]["path"]["routes"].as_array() {
                            for route in path {
                                if let Some(sub_routes) = route["subRoutes"].as_array() {
                                    for sub in sub_routes {
                                        let pool_addr = sub["dexRouter"]["address"].as_str().unwrap_or("");
                                        let dex = sub["dexRouter"]["name"].as_str().unwrap_or("openocean");
                                        
                                        if !pool_addr.is_empty() {
                                            let price = data["data"]["outAmount"].as_str()
                                                .and_then(|s| s.parse::<f64>().ok())
                                                .map(|a| a / 1e6)
                                                .unwrap_or(0.0);
                                            
                                            pools.push(PoolData::new(
                                                symbol.to_string(),
                                                chain_name.to_string(),
                                                dex.to_string(),
                                                pool_addr.to_string(),
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
                        
                        // Fallback
                        if pools.is_empty() {
                            if let Some(amount) = data["data"]["outAmount"].as_str() {
                                if let Ok(val) = amount.parse::<f64>() {
                                    pools.push(PoolData::new(
                                        symbol.to_string(),
                                        chain_name.to_string(),
                                        "openocean".to_string(),
                                        format!("openocean:{}:aggregated", chain),
                                        format!("{}/USDC", symbol),
                                        val / 1e6,
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

/// Matcha (0x) Token Search - returns token contract addresses
pub struct MatchaSource {
    client: Client,
}

impl MatchaSource {
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
impl PoolSource for MatchaSource {
    fn name(&self) -> &'static str { "Matcha" }

    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        let chain_ids = "1,8453,56,42161,43114,10,137,534352,59144,5000,34443,130";
        let url = format!(
            "https://matcha.xyz/api/tokens/search?chainId={}&limit=15&query={}",
            chain_ids, symbol
        );

        let resp = self.client.get(&url)
            .header("accept", "*/*")
            .header("referer", "https://matcha.xyz/")
            .header("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .send()
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let data: serde_json::Value = resp.json()
            .await
            .map_err(|e| SourceError::Parse(e.to_string()))?;

        let mut pools = Vec::new();
        
        if let Some(tokens) = data["data"].as_array() {
            for token in tokens.iter().take(10) {
                let token_symbol = token["symbol"].as_str().unwrap_or("");
                let chain_id = token["chainId"].as_u64().unwrap_or(0);
                let address = token["address"].as_str().unwrap_or("");
                let name = token["name"].as_str().unwrap_or("");
                let decimals = token["decimals"].as_u64().unwrap_or(18);
                
                // Skip if not matching symbol (case insensitive)
                if !token_symbol.eq_ignore_ascii_case(symbol) && 
                   !token_symbol.to_uppercase().contains(&symbol.to_uppercase()) {
                    continue;
                }
                
                let chain_name = match chain_id {
                    1 => "ethereum",
                    56 => "bsc",
                    137 => "polygon",
                    42161 => "arbitrum",
                    8453 => "base",
                    10 => "optimism",
                    43114 => "avalanche",
                    534352 => "scroll",
                    59144 => "linea",
                    5000 => "mantle",
                    _ => "other",
                };

                if !address.is_empty() {
                    pools.push(PoolData::new(
                        token_symbol.to_string(),
                        chain_name.to_string(),
                        "matcha".to_string(),
                        address.to_string(), // Real token contract address
                        format!("{} ({})", name, decimals),
                        0.0,
                        0.0,
                        0.0,
                        "matcha".to_string(),
                    ));
                }
            }
        }

        Ok(pools)
    }
}
