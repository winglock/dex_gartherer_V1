use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use crate::models::PoolData;
use super::{PoolSource, SourceError};

/// dex-guru 스타일 멀티 어그리게이터
pub struct DexGuruSource {
    client: Client,
}

#[derive(Debug, Deserialize)]
struct OneInchQuote {
    #[serde(rename = "toAmount")]
    to_amount: String,
    #[serde(rename = "fromAmount")]
    from_amount: String,
}

impl DexGuruSource {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap(),
        }
    }

    async fn fetch_1inch(&self, chain_id: u32, token: &str) -> Result<Option<f64>, SourceError> {
        // 1inch API - USDC 기준 가격 조회
        let usdc = match chain_id {
            1 => "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", // ETH
            56 => "0x8AC76a51cc950d9822D68b83fE1Ad97B32Cd580d", // BSC
            137 => "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174", // Polygon
            _ => return Ok(None),
        };

        let url = format!(
            "https://api.1inch.dev/swap/v6.0/{}/quote?src={}&dst={}&amount=1000000000000000000",
            chain_id, token, usdc
        );

        let resp = self.client.get(&url)
            .header("Authorization", "Bearer YOUR_API_KEY") // 필요시 설정
            .send()
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            return Ok(None);
        }

        let quote: OneInchQuote = resp.json()
            .await
            .map_err(|e| SourceError::Parse(e.to_string()))?;

        let from_amount: f64 = quote.from_amount.parse().unwrap_or(1.0);
        let to_amount: f64 = quote.to_amount.parse().unwrap_or(0.0);
        let price = to_amount / from_amount * 1e12; // USDC 6 decimals

        Ok(Some(price))
    }

    async fn fetch_0x(&self, chain_id: u32, token: &str, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        // 0x API
        let chain = match chain_id {
            1 => "ethereum",
            56 => "bsc",
            137 => "polygon",
            _ => return Ok(vec![]),
        };

        let url = format!(
            "https://api.0x.org/swap/v1/price?sellToken={}&buyToken=USDC&sellAmount=1000000000000000000",
            token
        );

        let resp = self.client.get(&url)
            .header("0x-api-key", "YOUR_API_KEY")
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                if let Ok(data) = r.json::<serde_json::Value>().await {
                    if let Some(price) = data["price"].as_str().and_then(|s| s.parse::<f64>().ok()) {
                        return Ok(vec![PoolData::new(
                            symbol.to_string(),
                            chain.to_string(),
                            "0x".to_string(),
                            "aggregated".to_string(),
                            format!("{}/USDC", symbol),
                            price,
                            0.0, // 0x doesn't return LP
                            0.0,
                            "0x".to_string(),
                        )]);
                    }
                }
            }
            _ => {}
        }

        Ok(vec![])
    }
}

#[async_trait]
impl PoolSource for DexGuruSource {
    fn name(&self) -> &'static str {
        "DexGuru"
    }

    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        let mut pools = Vec::new();

        // 여러 체인에서 0x 가격 조회
        for (chain_id, chain_name) in [(1u32, "ethereum"), (56, "bsc"), (137, "polygon")] {
            if let Ok(chain_pools) = self.fetch_0x(chain_id, symbol, symbol).await {
                pools.extend(chain_pools);
            }
        }

        Ok(pools)
    }
}
