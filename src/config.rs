use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub arbitrage: ArbitrageConfig,
    pub filter: FilterConfig,
    pub server: ServerConfig,
    #[serde(default)]
    pub storage: StorageConfig,
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

#[derive(Debug, Deserialize, Clone)]
pub struct StorageConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
}

fn default_enabled() -> bool { true }
fn default_data_dir() -> String { "./data".to_string() }

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            data_dir: "./data".to_string(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string("config.toml")?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}
