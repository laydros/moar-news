use std::{fs, path::Path};

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval: u64,
    pub feeds: Vec<FeedConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FeedConfig {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub has_discussion: bool,
}

fn default_refresh_interval() -> u64 {
    15
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}
