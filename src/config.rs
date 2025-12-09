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

    /// Parse config from a TOML string (useful for testing)
    pub fn from_str(content: &str) -> anyhow::Result<Self> {
        let config: Config = toml::from_str(content)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_refresh_interval() {
        assert_eq!(default_refresh_interval(), 15);
    }

    #[test]
    fn test_load_valid_config() {
        let content = r#"
            refresh_interval = 30

            [[feeds]]
            name = "Test Feed"
            url = "https://example.com/feed.xml"
            has_discussion = true

            [[feeds]]
            name = "Another Feed"
            url = "https://example.org/rss"
        "#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(content.as_bytes()).unwrap();

        let config = Config::load(temp_file.path()).unwrap();

        assert_eq!(config.refresh_interval, 30);
        assert_eq!(config.feeds.len(), 2);
        assert_eq!(config.feeds[0].name, "Test Feed");
        assert_eq!(config.feeds[0].url, "https://example.com/feed.xml");
        assert!(config.feeds[0].has_discussion);
        assert_eq!(config.feeds[1].name, "Another Feed");
        assert!(!config.feeds[1].has_discussion);
    }

    #[test]
    fn test_load_config_with_default_refresh_interval() {
        let content = r#"
            [[feeds]]
            name = "Test Feed"
            url = "https://example.com/feed.xml"
        "#;

        let config = Config::from_str(content).unwrap();

        assert_eq!(config.refresh_interval, 15); // Default value
        assert_eq!(config.feeds.len(), 1);
    }

    #[test]
    fn test_load_config_missing_file() {
        let result = Config::load("/nonexistent/path/config.toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_load_config_invalid_toml() {
        let content = "this is not valid toml {{{";

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(content.as_bytes()).unwrap();

        let result = Config::load(temp_file.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_load_config_missing_required_fields() {
        let content = r#"
            [[feeds]]
            name = "Test Feed"
            # Missing url field
        "#;

        let result = Config::from_str(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_feed_config_has_discussion_default() {
        let content = r#"
            [[feeds]]
            name = "Test Feed"
            url = "https://example.com/feed.xml"
        "#;

        let config = Config::from_str(content).unwrap();
        assert!(!config.feeds[0].has_discussion); // Default is false
    }

    #[test]
    fn test_empty_feeds_list() {
        let content = "feeds = []";

        let config = Config::from_str(content).unwrap();
        assert!(config.feeds.is_empty());
    }

    #[test]
    fn test_multiple_feeds_with_mixed_settings() {
        let content = r#"
            refresh_interval = 5

            [[feeds]]
            name = "HN"
            url = "https://news.ycombinator.com/rss"
            has_discussion = true

            [[feeds]]
            name = "Blog"
            url = "https://blog.example.com/feed"
            has_discussion = false

            [[feeds]]
            name = "News"
            url = "https://news.example.com/rss"
        "#;

        let config = Config::from_str(content).unwrap();

        assert_eq!(config.refresh_interval, 5);
        assert_eq!(config.feeds.len(), 3);

        assert!(config.feeds[0].has_discussion);
        assert!(!config.feeds[1].has_discussion);
        assert!(!config.feeds[2].has_discussion); // Default
    }
}
