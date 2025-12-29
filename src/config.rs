use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub arbitrage: ArbitrageConfig,
    pub filter: FilterConfig,
    pub server: ServerConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ArbitrageConfig {
    pub threshold: f64,
    pub update_interval: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FilterConfig {
    pub min_lp: f64,
    pub min_volume: f64,
    pub min_tx_count: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string("config.toml")?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}
