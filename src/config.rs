use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    /// Refresh interval in minutes
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval: u64,
    pub feeds: Vec<FeedConfig>,
}

fn default_refresh_interval() -> u64 {
    15
}

#[derive(Debug, Deserialize, Clone)]
pub struct FeedConfig {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub has_discussion: bool,
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}
