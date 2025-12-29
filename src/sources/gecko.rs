use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use crate::models::PoolData;
use super::{PoolSource, SourceError};

pub struct GeckoTerminal {
    client: Client,
}

#[derive(Debug, Deserialize)]
struct GeckoResponse {
    data: Vec<GeckoPool>,
}

#[derive(Debug, Deserialize)]
struct GeckoPool {
    #[allow(dead_code)]
    id: String,
    attributes: GeckoPoolAttributes,
    relationships: Option<GeckoRelationships>,
}

#[derive(Debug, Deserialize)]
struct GeckoPoolAttributes {
    name: String,
    address: String,
    base_token_price_usd: Option<String>,
    reserve_in_usd: Option<String>,
    volume_usd: Option<GeckoVolume>,
}

#[derive(Debug, Deserialize)]
struct GeckoVolume {
    h24: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeckoRelationships {
    dex: Option<GeckoDex>,
}

#[derive(Debug, Deserialize)]
struct GeckoDex {
    data: Option<GeckoDexData>,
}

#[derive(Debug, Deserialize)]
struct GeckoDexData {
    id: String,
}

impl GeckoTerminal {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap(),
        }
    }
}

#[async_trait]
impl PoolSource for GeckoTerminal {
    fn name(&self) -> &'static str {
        "GeckoTerminal"
    }

    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError> {
        let url = format!(
            "https://api.geckoterminal.com/api/v2/search/pools?query={}",
            symbol
        );

        let resp = self.client.get(&url)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;

        if resp.status() == 429 {
            return Err(SourceError::RateLimit);
        }

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let data: GeckoResponse = resp.json()
            .await
            .map_err(|e| SourceError::Parse(e.to_string()))?;

        let pools: Vec<PoolData> = data.data.into_iter()
            .filter_map(|p| {
                let price = p.attributes.base_token_price_usd
                    .as_ref()
                    .and_then(|s| s.parse::<f64>().ok())?;
                    
                let lp = p.attributes.reserve_in_usd
                    .as_ref()
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                    
                let volume = p.attributes.volume_usd
                    .as_ref()
                    .and_then(|v| v.h24.as_ref())
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);

                let dex_name = p.relationships
                    .and_then(|r| r.dex)
                    .and_then(|d| d.data)
                    .map(|d| d.id)
                    .unwrap_or_else(|| "unknown".to_string());

                Some(PoolData::new(
                    symbol.to_string(),
                    "multi".to_string(), // GeckoTerminal 검색은 멀티체인
                    dex_name,
                    p.attributes.address,
                    p.attributes.name,
                    price,
                    lp,
                    volume,
                    "geckoterminal".to_string(),
                ))
            })
            .collect();

        Ok(pools)
    }
}
