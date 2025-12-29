use reqwest::Client;
use serde::Deserialize;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures::{StreamExt, SinkExt};
use std::sync::Arc;
use dashmap::DashMap;

#[derive(Debug, Clone)]
pub struct CexPrice {
    pub symbol: String,
    pub price_krw: f64,
    pub price_usd: f64,
    pub timestamp: i64,
}

pub struct UpbitClient {
    prices: Arc<DashMap<String, CexPrice>>,
    krw_usd_rate: f64,
}

#[derive(Debug, Deserialize)]
struct UpbitTicker {
    code: String,
    trade_price: f64,
    timestamp: i64,
}

impl UpbitClient {
    pub fn new() -> Self {
        Self {
            prices: Arc::new(DashMap::new()),
            krw_usd_rate: 1400.0, // 기본 환율
        }
    }

    pub fn get_price(&self, symbol: &str) -> Option<CexPrice> {
        self.prices.get(symbol).map(|p| p.clone())
    }

    pub fn get_all_prices(&self) -> Vec<CexPrice> {
        self.prices.iter().map(|p| p.value().clone()).collect()
    }

    pub async fn start_websocket(&self, symbols: Vec<String>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = "wss://api.upbit.com/websocket/v1";
        let (ws_stream, _) = connect_async(url).await?;
        let (mut write, mut read) = ws_stream.split();

        // 구독 메시지
        let codes: Vec<String> = symbols.iter()
            .map(|s| format!("KRW-{}", s.to_uppercase()))
            .collect();
        
        let subscribe = serde_json::json!([
            {"ticket": "dex-gatherer"},
            {"type": "ticker", "codes": codes}
        ]);

        write.send(Message::Text(subscribe.to_string())).await?;

        let prices = self.prices.clone();
        let rate = self.krw_usd_rate;

        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                if let Ok(Message::Binary(data)) = msg {
                    if let Ok(ticker) = serde_json::from_slice::<UpbitTicker>(&data) {
                        let symbol = ticker.code.replace("KRW-", "");
                        let price = CexPrice {
                            symbol: symbol.clone(),
                            price_krw: ticker.trade_price,
                            price_usd: ticker.trade_price / rate,
                            timestamp: ticker.timestamp,
                        };
                        prices.insert(symbol, price);
                    }
                }
            }
        });

        Ok(())
    }

    pub async fn fetch_krw_coins(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let client = Client::new();
        let resp = client.get("https://api.upbit.com/v1/market/all")
            .send()
            .await?;

        #[derive(Deserialize)]
        struct Market {
            market: String,
        }

        let markets: Vec<Market> = resp.json().await?;
        let krw_coins: Vec<String> = markets.into_iter()
            .filter(|m| m.market.starts_with("KRW-"))
            .map(|m| m.market.replace("KRW-", ""))
            .collect();

        Ok(krw_coins)
    }
}
