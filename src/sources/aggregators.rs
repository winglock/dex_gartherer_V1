use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use crate::models::PoolData;
use super::{PoolSource, SourceError};

// L1 tokens that need W-prefix search for wrapped versions
const L1_TOKENS: &[&str] = &[
    "BTC", "ETH", "SOL", "ADA", "XRP", "DOGE", "DOT", "AVAX", "ATOM", "NEAR",
    "ALGO", "FIL", "HBAR", "VET", "FLOW", "XLM", "THETA", "EGLD", "NEO", "QTUM",
    "WAVES", "IOTA", "CKB", "KAVA", "ARK", "LSK", "ASTR", "BCH", "ETC", "TRX",
    "SUI", "APT", "SEI", "CELO", "CRO", "RVN", "TT", "OM", "AKT", "G"
];

/// Check if symbol is L1 token
pub fn is_l1_token(symbol: &str) -> bool {
    L1_TOKENS.contains(&symbol.to_uppercase().as_str())
}

/// Get wrapped symbol variants
pub fn get_search_variants(symbol: &str) -> Vec<String> {
    let upper = symbol.to_uppercase();
    if is_l1_token(&upper) {
        vec![
            upper.clone(),
            format!("W{}", upper),  // WBTC, WETH, etc.
        ]
    } else {
        vec![upper]
    }
}

/// DexScreener - Most reliable DEX data API
pub struct DexScreenerSource {
    client: Client,
}

impl DexScreenerSource {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DexScreenerResponse {
    pairs: Option<Vec<DexScreenerPair>>,
}

#[derive(Debug, Deserialize)]
struct DexScreenerPair {
    #[serde(rename = "chainId")]
    chain_id: Option<String>,
    #[serde(rename = "dexId")]
    dex_id: Option<String>,
    #[serde(rename = "pairAddress")]
    pair_address: Option<String>,
    #[serde(rename = "baseToken")]
    base_token: Option<DexScreenerToken>,
    #[serde(rename = "priceUsd")]
    price_usd: Option<String>,
    liquidity: Option<DexScreenerLiquidity>,
    volume: Option<DexScreenerVolume>,
}

#[derive(Debug, Deserialize)]
struct DexScreenerToken {
    symbol: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DexScreenerLiquidity {
    usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DexScreenerVolume {
    h24: Option<f64>,
}

#[async_trait]
impl PoolSource for DexScreenerSource {
    fn name(&self) -> &'static str { "DexScreener" }

    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        let mut all_pools = Vec::new();
        
        // Search both original and W-prefixed
        for variant in get_search_variants(symbol) {
            let url = format!(
                "https://api.dexscreener.com/latest/dex/search?q={}",
                variant
            );

            match self.client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(data) = resp.json::<DexScreenerResponse>().await {
                        if let Some(pairs) = data.pairs {
                            for pair in pairs.into_iter().take(10) {
                                let pool_addr = pair.pair_address.unwrap_or_default();
                                let chain = pair.chain_id.unwrap_or_else(|| "unknown".to_string());
                                let dex = pair.dex_id.unwrap_or_else(|| "unknown".to_string());
                                let token_symbol = pair.base_token
                                    .as_ref()
                                    .and_then(|t| t.symbol.clone())
                                    .unwrap_or_else(|| variant.clone());
                                let price = pair.price_usd
                                    .as_ref()
                                    .and_then(|p| p.parse::<f64>().ok())
                                    .unwrap_or(0.0);
                                let lp = pair.liquidity
                                    .as_ref()
                                    .and_then(|l| l.usd)
                                    .unwrap_or(0.0);
                                let volume = pair.volume
                                    .as_ref()
                                    .and_then(|v| v.h24)
                                    .unwrap_or(0.0);

                                if !pool_addr.is_empty() && price > 0.0 {
                                    all_pools.push(PoolData::new(
                                        token_symbol,
                                        chain,
                                        dex,
                                        pool_addr,
                                        format!("{}/USD", variant),
                                        price,
                                        lp,
                                        volume,
                                        "dexscreener".to_string(),
                                    ));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(all_pools)
    }
}

/// Matcha (0x) Token Search - with proper headers
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
        let mut all_pools = Vec::new();
        let chain_ids = "1,8453,56,42161,43114,10,137,534352,59144,5000,34443,130,143,9745";
        
        // Search both original and W-prefixed
        for variant in get_search_variants(symbol) {
            let url = format!(
                "https://matcha.xyz/api/tokens/search?chainId={}&limit=15&page=1&query={}",
                chain_ids, variant
            );

            let resp = self.client.get(&url)
                .header("accept", "*/*")
                .header("accept-language", "ko-KR,ko;q=0.8")
                .header("referer", "https://matcha.xyz/")
                .header("sec-fetch-dest", "empty")
                .header("sec-fetch-mode", "cors")
                .header("sec-fetch-site", "same-origin")
                .header("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36")
                .send()
                .await;

            match resp {
                Ok(r) if r.status().is_success() => {
                    if let Ok(data) = r.json::<serde_json::Value>().await {
                        if let Some(tokens) = data["data"].as_array() {
                            for token in tokens.iter().take(10) {
                                let token_symbol = token["symbol"].as_str().unwrap_or("");
                                let chain_id = token["chainId"].as_u64().unwrap_or(0);
                                let address = token["address"].as_str().unwrap_or("");
                                let name = token["name"].as_str().unwrap_or("");
                                let decimals = token["decimals"].as_u64().unwrap_or(18);

                                // Match symbol (case insensitive)
                                if !token_symbol.eq_ignore_ascii_case(&variant) &&
                                   !token_symbol.to_uppercase().contains(&variant.to_uppercase()) {
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
                                    143 => "monad",
                                    9745 => "sonic",
                                    _ => "other",
                                };

                                if !address.is_empty() {
                                    all_pools.push(PoolData::new(
                                        token_symbol.to_string(),
                                        chain_name.to_string(),
                                        "matcha".to_string(),
                                        address.to_string(),
                                        format!("{} ({} dec)", name, decimals),
                                        0.0,
                                        0.0,
                                        0.0,
                                        "matcha".to_string(),
                                    ));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(all_pools)
    }
}

/// 1inch Aggregator - simplified
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
}

#[async_trait]
impl PoolSource for OneInchSource {
    fn name(&self) -> &'static str { "1inch" }

    async fn fetch_pools(&self, _symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        // 1inch requires API key, skip for now
        Ok(vec![])
    }
}

/// ParaSwap - simplified
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

    async fn fetch_pools(&self, _symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        // ParaSwap needs specific token addresses, skip for now
        Ok(vec![])
    }
}

/// KyberSwap - simplified
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

    async fn fetch_pools(&self, _symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        // KyberSwap needs specific token addresses, skip for now
        Ok(vec![])
    }
}

/// OpenOcean - simplified
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

    async fn fetch_pools(&self, _symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        // OpenOcean needs specific token addresses, skip for now
        Ok(vec![])
    }
}
