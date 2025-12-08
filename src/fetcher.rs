use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use feed_rs::parser;
use reqwest::Client;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::db::{Database, Feed};

pub struct Fetcher {
    client: Client,
    db: Arc<Database>,
    refreshing: Arc<RwLock<bool>>,
}

impl Fetcher {
    pub fn new(db: Arc<Database>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("MoarNews/1.0 (RSS Aggregator)")
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            db,
            refreshing: Arc::new(RwLock::new(false)),
        }
    }

    pub async fn is_refreshing(&self) -> bool {
        *self.refreshing.read().await
    }

    pub async fn refresh_all_feeds(&self) -> anyhow::Result<()> {
        // Check if already refreshing
        {
            let mut refreshing = self.refreshing.write().await;
            if *refreshing {
                info!("Refresh already in progress, skipping");
                return Ok(());
            }
            *refreshing = true;
        }

        let result = self.do_refresh_all().await;

        // Clear refreshing flag
        {
            let mut refreshing = self.refreshing.write().await;
            *refreshing = false;
        }

        result
    }

    async fn do_refresh_all(&self) -> anyhow::Result<()> {
        let feeds = self.db.get_all_feeds().await?;
        info!("Refreshing {} feeds", feeds.len());

        for feed in feeds {
            if let Err(e) = self.refresh_feed(&feed).await {
                error!("Failed to refresh feed '{}': {}", feed.name, e);
                let _ = self
                    .db
                    .update_feed_fetched(feed.id, Some(&e.to_string()))
                    .await;
            } else {
                let _ = self.db.update_feed_fetched(feed.id, None).await;
            }
        }

        info!("Feed refresh complete");
        Ok(())
    }

    async fn refresh_feed(&self, feed: &Feed) -> anyhow::Result<()> {
        info!("Fetching feed: {} ({})", feed.name, feed.url);

        let response = self.client.get(&feed.url).send().await?;
        let bytes = response.bytes().await?;

        let parsed = parser::parse(&bytes[..])?;

        let mut count = 0;
        for entry in parsed.entries {
            let guid = entry.id.clone();

            let title = entry
                .title
                .as_ref()
                .map(|t| t.content.clone())
                .unwrap_or_else(|| "Untitled".to_string());

            // Get the main link - for HN/Lobste.rs, the actual article is typically the first link
            let link = entry
                .links
                .first()
                .map(|l| l.href.clone())
                .unwrap_or_default();

            if link.is_empty() {
                warn!("Skipping entry with no link: {}", title);
                continue;
            }

            // Get discussion link for HN/Lobste.rs
            let discussion_link = self.extract_discussion_link(feed, &entry);

            // Get published date
            let published: Option<DateTime<Utc>> = entry
                .published
                .or(entry.updated)
                .map(|dt| dt.into());

            self.db
                .upsert_item(
                    feed.id,
                    &guid,
                    &title,
                    &link,
                    discussion_link.as_deref(),
                    published,
                )
                .await?;

            count += 1;
        }

        info!("Added/updated {} items for feed '{}'", count, feed.name);
        Ok(())
    }

    fn extract_discussion_link(&self, feed: &Feed, entry: &feed_rs::model::Entry) -> Option<String> {
        if !feed.has_discussion {
            return None;
        }

        // Look for a comments link in the links array
        for link in &entry.links {
            if link.rel.as_deref() == Some("replies") || link.rel.as_deref() == Some("comments") {
                return Some(link.href.clone());
            }
        }

        // For Hacker News, the guid/id IS the discussion URL
        if feed.url.contains("news.ycombinator.com") {
            // HN RSS guid is like "https://news.ycombinator.com/item?id=12345"
            if entry.id.contains("item?id=") {
                return Some(entry.id.clone());
            }
        }

        // For Lobste.rs, the guid/id is the discussion URL
        if feed.url.contains("lobste.rs") {
            // Lobste.rs RSS has guid like "https://lobste.rs/s/xxxxx"
            if entry.id.contains("lobste.rs/s/") {
                return Some(entry.id.clone());
            }
        }

        None
    }
}

pub async fn start_background_refresh(fetcher: Arc<Fetcher>, interval_minutes: u64) {
    let interval = Duration::from_secs(interval_minutes * 60);

    // Do initial fetch
    info!("Starting initial feed fetch");
    if let Err(e) = fetcher.refresh_all_feeds().await {
        error!("Initial feed fetch failed: {}", e);
    }

    // Then schedule periodic refreshes
    loop {
        tokio::time::sleep(interval).await;
        info!("Starting scheduled feed refresh");
        if let Err(e) = fetcher.refresh_all_feeds().await {
            error!("Scheduled feed refresh failed: {}", e);
        }
    }
}
