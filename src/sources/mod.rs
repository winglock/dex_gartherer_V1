pub mod gecko;
pub mod upbit;
pub mod dexguru;

use async_trait::async_trait;
use crate::models::PoolData;

#[async_trait]
pub trait PoolSource: Send + Sync {
    fn name(&self) -> &'static str;
    async fn fetch_pools(&self, symbol: &str) -> Result<Vec<PoolData>, SourceError>;
}

#[derive(Debug)]
pub enum SourceError {
    Network(String),
    Parse(String),
    RateLimit,
    #[allow(dead_code)]
    NotFound,
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::Network(e) => write!(f, "Network error: {}", e),
            SourceError::Parse(e) => write!(f, "Parse error: {}", e),
            SourceError::RateLimit => write!(f, "Rate limited"),
            SourceError::NotFound => write!(f, "Not found"),
        }
    }
}
