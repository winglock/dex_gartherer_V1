use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use crate::models::PoolData;
use super::{PoolSource, SourceError};

/// 1inch Aggregator
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

        // 1inch Quote API
        let url = format!(
            "https://api.1inch.dev/swap/v6.0/{}/quote?src=0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE&dst=0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48&amount=1000000000000000000",
            chain_id
        );

        match self.client.get(&url)
            .header("Authorization", "Bearer ")  // API key needed
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(data) = resp.json::<serde_json::Value>().await {
                    if let Some(dst_amount) = data["dstAmount"].as_str() {
                        if let Ok(amount) = dst_amount.parse::<f64>() {
                            let price = amount / 1e6; // USDC 6 decimals
                            return vec![PoolData::new(
                                symbol.to_string(),
                                chain_name.to_string(),
                                "1inch".to_string(),
                                format!("1inch-{}-{}", chain_id, symbol),
                                format!("{}/USDC", symbol),
                                price,
                                0.0,
                                0.0,
                                "1inch".to_string(),
                            )];
                        }
                    }
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

/// ParaSwap Aggregator
pub struct ParaSwapSource {
    client: Client,
}

#[derive(Debug, Deserialize)]
struct ParaSwapResponse {
    #[serde(rename = "priceRoute")]
    price_route: Option<ParaSwapPriceRoute>,
}

#[derive(Debug, Deserialize)]
struct ParaSwapPriceRoute {
    #[serde(rename = "destAmount")]
    dest_amount: Option<String>,
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
                    if let Ok(data) = resp.json::<ParaSwapResponse>().await {
                        if let Some(route) = data.price_route {
                            if let Some(amount) = route.dest_amount {
                                if let Ok(val) = amount.parse::<f64>() {
                                    let price = val / 1e6;
                                    pools.push(PoolData::new(
                                        symbol.to_string(),
                                        chain_name.to_string(),
                                        "paraswap".to_string(),
                                        format!("paraswap-{}-{}", chain_id, symbol),
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

/// KyberSwap Aggregator
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
                        if let Some(amount) = data["data"]["routeSummary"]["amountOut"].as_str() {
                            if let Ok(val) = amount.parse::<f64>() {
                                let price = val / 1e6;
                                pools.push(PoolData::new(
                                    symbol.to_string(),
                                    chain_name.to_string(),
                                    "kyberswap".to_string(),
                                    format!("kyber-{}-{}", chain, symbol),
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
        }
        Ok(pools)
    }
}

/// OpenOcean Aggregator
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
                        if let Some(amount) = data["data"]["outAmount"].as_str() {
                            if let Ok(val) = amount.parse::<f64>() {
                                let price = val / 1e6;
                                pools.push(PoolData::new(
                                    symbol.to_string(),
                                    chain_name.to_string(),
                                    "openocean".to_string(),
                                    format!("openocean-{}-{}", chain, symbol),
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
        Ok(pools)
    }
}

/// Matcha (0x) Token Search
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
        let chain_ids = "1,8453,56,42161,43114,10,137";
        let url = format!(
            "https://matcha.xyz/api/tokens/search?chainId={}&limit=10&query={}",
            chain_ids, symbol
        );

        let resp = self.client.get(&url)
            .header("accept", "*/*")
            .header("referer", "https://matcha.xyz/")
            .header("user-agent", "Mozilla/5.0")
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
            for token in tokens.iter().take(5) {
                let token_symbol = token["symbol"].as_str().unwrap_or("");
                let chain_id = token["chainId"].as_u64().unwrap_or(0);
                let address = token["address"].as_str().unwrap_or("");
                
                let chain_name = match chain_id {
                    1 => "ethereum",
                    56 => "bsc",
                    137 => "polygon",
                    42161 => "arbitrum",
                    8453 => "base",
                    10 => "optimism",
                    _ => "unknown",
                };

                pools.push(PoolData::new(
                    token_symbol.to_string(),
                    chain_name.to_string(),
                    "matcha".to_string(),
                    address.to_string(),
                    format!("{}/USD", token_symbol),
                    0.0, // Matcha search doesn't return price
                    0.0,
                    0.0,
                    "matcha".to_string(),
                ));
            }
        }

        Ok(pools)
    }
}
