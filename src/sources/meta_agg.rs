use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use crate::models::PoolData;
use super::{PoolSource, SourceError};

/// Meta Aggregation API Source (local server http://localhost:8000)
/// Supports: 1inch, ParaSwap, KyberSwap, OpenOcean
pub struct MetaAggSource {
    client: Client,
    base_url: String,
    token_map: HashMap<String, HashMap<u32, String>>,
}

#[derive(Debug, Deserialize)]
struct MetaQuoteResponse {
    result: Option<MetaQuoteResult>,
}

#[derive(Debug, Deserialize)]
struct MetaQuoteResult {
    #[serde(rename = "dstAmount")]
    dst_amount: Option<String>,
    protocols: Option<serde_json::Value>,
    #[serde(rename = "estimatedGas")]
    estimated_gas: Option<String>,
}

impl MetaAggSource {
    pub fn new(base_url: &str) -> Self {
        let token_map = Self::load_token_map();
        
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap(),
            base_url: base_url.to_string(),
            token_map,
        }
    }

    fn load_token_map() -> HashMap<String, HashMap<u32, String>> {
        // Load from token_addresses.json or use embedded map
        let json_str = include_str!("../../token_addresses.json");
        serde_json::from_str(json_str).unwrap_or_default()
    }

    fn get_token_address(&self, symbol: &str, chain_id: u32) -> Option<String> {
        self.token_map
            .get(&symbol.to_uppercase())
            .and_then(|chains| chains.get(&chain_id))
            .cloned()
    }

    async fn fetch_chain(&self, symbol: &str, chain_id: u32) -> Vec<PoolData> {
        let chain_name = match chain_id {
            1 => "ethereum",
            56 => "bsc",
            137 => "polygon",
            42161 => "arbitrum",
            10 => "optimism",
            _ => return vec![],
        };

        // Get token address from map
        let token_address = match self.get_token_address(symbol, chain_id) {
            Some(addr) if addr != "0x0000000000000000000000000000000000000000" => addr,
            _ => return vec![], // Skip if no address
        };

        // USDC as quote token
        let usdc_address = match chain_id {
            1 => "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
            56 => "0x8AC76a51cc950d9822D68b83fE1Ad97B32Cd580d",
            137 => "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174",
            42161 => "0xaf88d065e77c8cC2239327C5EDb3A432268e5831",
            10 => "0x7F5c764cBc14f9669B88837ca1490cCa17c31607",
            _ => return vec![],
        };

        let url = format!(
            "{}/v1/quote?chain_id={}&from_token={}&to_token={}&amount=1000000000000000000",
            self.base_url, chain_id, token_address, usdc_address
        );

        let mut pools = Vec::new();

        match self.client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(data) = resp.json::<MetaQuoteResponse>().await {
                    if let Some(result) = data.result {
                        // Extract price from dstAmount
                        let price = result.dst_amount
                            .as_ref()
                            .and_then(|s| s.parse::<f64>().ok())
                            .map(|v| v / 1e6) // USDC 6 decimals
                            .unwrap_or(0.0);

                        // Extract protocols/pools from route
                        if let Some(protocols) = result.protocols {
                            self.extract_pools_from_protocols(
                                symbol, chain_name, price, &protocols, &mut pools
                            );
                        }

                        // If no specific pools extracted, add aggregated result
                        if pools.is_empty() && price > 0.0 {
                            pools.push(PoolData::new(
                                symbol.to_string(),
                                chain_name.to_string(),
                                "meta-agg".to_string(),
                                format!("meta:{}:{}", chain_id, symbol),
                                format!("{}/USDC", symbol),
                                price,
                                0.0,
                                0.0,
                                "meta-aggregation".to_string(),
                            ));
                        }
                    }
                }
            }
            _ => {
                tracing::debug!("MetaAgg: Failed to fetch {} on chain {}", symbol, chain_id);
            }
        }

        pools
    }

    fn extract_pools_from_protocols(
        &self,
        symbol: &str,
        chain_name: &str,
        price: f64,
        protocols: &serde_json::Value,
        pools: &mut Vec<PoolData>,
    ) {
        // Protocols is usually an array of arrays of swaps
        if let Some(protocol_groups) = protocols.as_array() {
            for group in protocol_groups {
                if let Some(steps) = group.as_array() {
                    for step in steps {
                        if let Some(swaps) = step.as_array() {
                            for swap in swaps {
                                let dex_name = swap["name"].as_str()
                                    .unwrap_or("unknown");
                                let pool_address = swap["fromTokenAddress"].as_str()
                                    .or(swap["pool"].as_str())
                                    .unwrap_or("");
                                let part = swap["part"].as_f64().unwrap_or(0.0);

                                if part > 0.0 {
                                    pools.push(PoolData::new(
                                        symbol.to_string(),
                                        chain_name.to_string(),
                                        dex_name.to_string(),
                                        pool_address.to_string(),
                                        format!("{}/USDC ({}%)", symbol, part as i32),
                                        price,
                                        0.0,
                                        0.0,
                                        "meta-aggregation".to_string(),
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

#[async_trait]
impl PoolSource for MetaAggSource {
    fn name(&self) -> &'static str { "MetaAgg" }

    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        let mut all_pools = Vec::new();

        // Query multiple chains
        for chain_id in [1, 56, 137] { // Ethereum, BSC, Polygon
            let pools = self.fetch_chain(symbol, chain_id).await;
            all_pools.extend(pools);
        }

        Ok(all_pools)
    }
}
