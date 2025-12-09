use std::collections::HashMap;
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

        // Extract comments URLs from raw XML (feed_rs doesn't parse RSS <comments> element)
        let comments_map = Self::extract_comments_from_xml(&bytes);

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
            let discussion_link =
                Self::extract_discussion_link(feed, &entry, comments_map.get(&link), &link);

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

    /// Extract <comments> URLs from raw RSS XML since feed_rs doesn't parse them
    pub fn extract_comments_from_xml(xml_bytes: &[u8]) -> HashMap<String, String> {
        let mut comments_map = HashMap::new();
        let xml_str = match std::str::from_utf8(xml_bytes) {
            Ok(s) => s,
            Err(_) => return comments_map,
        };

        // Simple regex-free parsing: find <item> blocks and extract <link> and <comments>
        for item_block in xml_str.split("<item>").skip(1) {
            let item_end = item_block.find("</item>").unwrap_or(item_block.len());
            let item = &item_block[..item_end];

            // Extract <link>
            let link = Self::extract_xml_element(item, "link");
            // Extract <comments>
            let comments = Self::extract_xml_element(item, "comments");

            if let (Some(link), Some(comments)) = (link, comments) {
                comments_map.insert(link, comments);
            }
        }

        comments_map
    }

    pub fn extract_xml_element(xml: &str, tag: &str) -> Option<String> {
        let start_tag = format!("<{}>", tag);
        let end_tag = format!("</{}>", tag);

        let start = xml.find(&start_tag)? + start_tag.len();
        let end = xml[start..].find(&end_tag)? + start;

        Some(xml[start..end].trim().to_string())
    }

    pub fn extract_discussion_link(
        feed: &Feed,
        entry: &feed_rs::model::Entry,
        comments_from_xml: Option<&String>,
        main_link: &str,
    ) -> Option<String> {
        if !feed.has_discussion {
            return None;
        }

        // For Hacker News: the guid/id IS the discussion URL
        // Skip if the main link is already an HN URL (e.g., Ask HN posts)
        if feed.url.contains("news.ycombinator.com") {
            if main_link.contains("news.ycombinator.com/item?id=") {
                // Main link IS the discussion, no need for separate discussion link
                return None;
            }
            // Use entry.id (guid) as the discussion URL - it's always the HN item URL
            if entry.id.contains("news.ycombinator.com/item?id=") {
                return Some(entry.id.clone());
            }
        }

        // For Lobste.rs, the guid/id is the discussion URL
        if feed.url.contains("lobste.rs") {
            if entry.id.contains("lobste.rs/s/") {
                return Some(entry.id.clone());
            }
        }

        // For other feeds: check if we extracted a <comments> URL from raw XML
        if let Some(comments_url) = comments_from_xml {
            return Some(comments_url.clone());
        }

        // Look for a comments link in the links array
        for link in &entry.links {
            let rel = link.rel.as_deref().unwrap_or("").to_lowercase();
            if rel == "replies" || rel == "comments" {
                return Some(link.href.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use feed_rs::model::{Entry, Link};

    fn create_test_feed(name: &str, url: &str, has_discussion: bool) -> Feed {
        Feed {
            id: 1,
            name: name.to_string(),
            url: url.to_string(),
            has_discussion,
            last_fetched: None,
            last_error: None,
        }
    }

    fn create_test_entry(id: &str, links: Vec<(&str, Option<&str>)>) -> Entry {
        Entry {
            id: id.to_string(),
            links: links
                .into_iter()
                .map(|(href, rel)| Link {
                    href: href.to_string(),
                    rel: rel.map(|r| r.to_string()),
                    media_type: None,
                    href_lang: None,
                    title: None,
                    length: None,
                })
                .collect(),
            ..Default::default()
        }
    }

    // Tests for extract_xml_element
    mod extract_xml_element_tests {
        use super::*;

        #[test]
        fn test_extract_simple_element() {
            let xml = "<title>Hello World</title>";
            let result = Fetcher::extract_xml_element(xml, "title");
            assert_eq!(result, Some("Hello World".to_string()));
        }

        #[test]
        fn test_extract_element_with_whitespace() {
            let xml = "<link>  https://example.com  </link>";
            let result = Fetcher::extract_xml_element(xml, "link");
            assert_eq!(result, Some("https://example.com".to_string()));
        }

        #[test]
        fn test_extract_element_not_found() {
            let xml = "<title>Hello</title>";
            let result = Fetcher::extract_xml_element(xml, "link");
            assert_eq!(result, None);
        }

        #[test]
        fn test_extract_element_empty() {
            let xml = "<title></title>";
            let result = Fetcher::extract_xml_element(xml, "title");
            assert_eq!(result, Some("".to_string()));
        }

        #[test]
        fn test_extract_element_no_closing_tag() {
            let xml = "<title>Hello";
            let result = Fetcher::extract_xml_element(xml, "title");
            assert_eq!(result, None);
        }

        #[test]
        fn test_extract_element_with_surrounding_content() {
            let xml = "<item><link>https://example.com</link><title>Test</title></item>";
            let result = Fetcher::extract_xml_element(xml, "link");
            assert_eq!(result, Some("https://example.com".to_string()));
        }

        #[test]
        fn test_extract_first_element_when_multiple() {
            let xml = "<link>first</link><link>second</link>";
            let result = Fetcher::extract_xml_element(xml, "link");
            assert_eq!(result, Some("first".to_string()));
        }
    }

    // Tests for extract_comments_from_xml
    mod extract_comments_from_xml_tests {
        use super::*;

        #[test]
        fn test_extract_single_item_with_comments() {
            let xml = r#"
                <rss>
                    <channel>
                        <item>
                            <link>https://article.com</link>
                            <comments>https://forum.com/discuss/123</comments>
                        </item>
                    </channel>
                </rss>
            "#;

            let result = Fetcher::extract_comments_from_xml(xml.as_bytes());
            assert_eq!(result.len(), 1);
            assert_eq!(
                result.get("https://article.com"),
                Some(&"https://forum.com/discuss/123".to_string())
            );
        }

        #[test]
        fn test_extract_multiple_items_with_comments() {
            let xml = r#"
                <rss>
                    <channel>
                        <item>
                            <link>https://article1.com</link>
                            <comments>https://forum.com/1</comments>
                        </item>
                        <item>
                            <link>https://article2.com</link>
                            <comments>https://forum.com/2</comments>
                        </item>
                    </channel>
                </rss>
            "#;

            let result = Fetcher::extract_comments_from_xml(xml.as_bytes());
            assert_eq!(result.len(), 2);
            assert_eq!(
                result.get("https://article1.com"),
                Some(&"https://forum.com/1".to_string())
            );
            assert_eq!(
                result.get("https://article2.com"),
                Some(&"https://forum.com/2".to_string())
            );
        }

        #[test]
        fn test_extract_item_without_comments() {
            let xml = r#"
                <rss>
                    <channel>
                        <item>
                            <link>https://article.com</link>
                            <title>No comments here</title>
                        </item>
                    </channel>
                </rss>
            "#;

            let result = Fetcher::extract_comments_from_xml(xml.as_bytes());
            assert!(result.is_empty());
        }

        #[test]
        fn test_extract_mixed_items() {
            let xml = r#"
                <rss>
                    <channel>
                        <item>
                            <link>https://article1.com</link>
                            <comments>https://forum.com/1</comments>
                        </item>
                        <item>
                            <link>https://article2.com</link>
                        </item>
                        <item>
                            <link>https://article3.com</link>
                            <comments>https://forum.com/3</comments>
                        </item>
                    </channel>
                </rss>
            "#;

            let result = Fetcher::extract_comments_from_xml(xml.as_bytes());
            assert_eq!(result.len(), 2);
            assert!(result.contains_key("https://article1.com"));
            assert!(!result.contains_key("https://article2.com"));
            assert!(result.contains_key("https://article3.com"));
        }

        #[test]
        fn test_extract_empty_xml() {
            let xml = "";
            let result = Fetcher::extract_comments_from_xml(xml.as_bytes());
            assert!(result.is_empty());
        }

        #[test]
        fn test_extract_invalid_utf8() {
            let invalid_bytes = vec![0xFF, 0xFE, 0x00, 0x01];
            let result = Fetcher::extract_comments_from_xml(&invalid_bytes);
            assert!(result.is_empty());
        }

        #[test]
        fn test_extract_no_items() {
            let xml = r#"
                <rss>
                    <channel>
                        <title>Empty Feed</title>
                    </channel>
                </rss>
            "#;

            let result = Fetcher::extract_comments_from_xml(xml.as_bytes());
            assert!(result.is_empty());
        }
    }

    // Tests for extract_discussion_link
    mod extract_discussion_link_tests {
        use super::*;

        #[test]
        fn test_no_discussion_when_disabled() {
            let feed = create_test_feed("Blog", "https://blog.example.com", false);
            let entry = create_test_entry("123", vec![("https://article.com", None)]);

            let result = Fetcher::extract_discussion_link(&feed, &entry, None, "https://article.com");
            assert_eq!(result, None);
        }

        #[test]
        fn test_hn_discussion_link_from_entry_id() {
            let feed = create_test_feed(
                "Hacker News",
                "https://news.ycombinator.com/rss",
                true,
            );
            let entry = create_test_entry(
                "https://news.ycombinator.com/item?id=12345",
                vec![("https://article.example.com", None)],
            );

            let result =
                Fetcher::extract_discussion_link(&feed, &entry, None, "https://article.example.com");
            assert_eq!(
                result,
                Some("https://news.ycombinator.com/item?id=12345".to_string())
            );
        }

        #[test]
        fn test_hn_skip_when_main_link_is_discussion() {
            let feed = create_test_feed(
                "Hacker News",
                "https://news.ycombinator.com/rss",
                true,
            );
            // Ask HN posts where the main link IS the discussion
            let entry = create_test_entry(
                "https://news.ycombinator.com/item?id=12345",
                vec![("https://news.ycombinator.com/item?id=12345", None)],
            );

            let result = Fetcher::extract_discussion_link(
                &feed,
                &entry,
                None,
                "https://news.ycombinator.com/item?id=12345",
            );
            assert_eq!(result, None);
        }

        #[test]
        fn test_lobsters_discussion_link() {
            let feed = create_test_feed("Lobste.rs", "https://lobste.rs/rss", true);
            let entry = create_test_entry(
                "https://lobste.rs/s/abc123",
                vec![("https://article.example.com", None)],
            );

            let result =
                Fetcher::extract_discussion_link(&feed, &entry, None, "https://article.example.com");
            assert_eq!(result, Some("https://lobste.rs/s/abc123".to_string()));
        }

        #[test]
        fn test_discussion_link_from_xml_comments() {
            let feed = create_test_feed("Reddit", "https://reddit.com/.rss", true);
            let entry = create_test_entry("123", vec![("https://article.com", None)]);
            let comments_url = "https://reddit.com/r/programming/comments/abc".to_string();

            let result = Fetcher::extract_discussion_link(
                &feed,
                &entry,
                Some(&comments_url),
                "https://article.com",
            );
            assert_eq!(result, Some(comments_url));
        }

        #[test]
        fn test_discussion_link_from_replies_rel() {
            let feed = create_test_feed("Forum", "https://forum.example.com/feed", true);
            let entry = create_test_entry(
                "123",
                vec![
                    ("https://article.com", None),
                    ("https://forum.example.com/topic/123/replies", Some("replies")),
                ],
            );

            let result = Fetcher::extract_discussion_link(&feed, &entry, None, "https://article.com");
            assert_eq!(
                result,
                Some("https://forum.example.com/topic/123/replies".to_string())
            );
        }

        #[test]
        fn test_discussion_link_from_comments_rel() {
            let feed = create_test_feed("Blog", "https://blog.example.com/feed", true);
            let entry = create_test_entry(
                "123",
                vec![
                    ("https://blog.example.com/post/1", None),
                    ("https://blog.example.com/post/1/comments", Some("comments")),
                ],
            );

            let result = Fetcher::extract_discussion_link(
                &feed,
                &entry,
                None,
                "https://blog.example.com/post/1",
            );
            assert_eq!(
                result,
                Some("https://blog.example.com/post/1/comments".to_string())
            );
        }

        #[test]
        fn test_no_discussion_link_found() {
            let feed = create_test_feed("Blog", "https://blog.example.com/feed", true);
            let entry = create_test_entry("123", vec![("https://article.com", None)]);

            let result = Fetcher::extract_discussion_link(&feed, &entry, None, "https://article.com");
            assert_eq!(result, None);
        }

        #[test]
        fn test_xml_comments_takes_precedence_over_link_rel() {
            let feed = create_test_feed("Forum", "https://forum.example.com/feed", true);
            let entry = create_test_entry(
                "123",
                vec![
                    ("https://article.com", None),
                    ("https://forum.example.com/fallback", Some("replies")),
                ],
            );
            let comments_url = "https://forum.example.com/preferred".to_string();

            let result = Fetcher::extract_discussion_link(
                &feed,
                &entry,
                Some(&comments_url),
                "https://article.com",
            );
            assert_eq!(result, Some(comments_url));
        }

        #[test]
        fn test_case_insensitive_rel_matching() {
            let feed = create_test_feed("Blog", "https://blog.example.com/feed", true);
            let entry = create_test_entry(
                "123",
                vec![
                    ("https://blog.example.com/post/1", None),
                    ("https://blog.example.com/post/1/comments", Some("COMMENTS")),
                ],
            );

            let result = Fetcher::extract_discussion_link(
                &feed,
                &entry,
                None,
                "https://blog.example.com/post/1",
            );
            assert_eq!(
                result,
                Some("https://blog.example.com/post/1/comments".to_string())
            );
        }
    }
}
