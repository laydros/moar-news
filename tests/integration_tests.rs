//! Integration tests for the moar-news RSS aggregator
//!
//! These tests verify the full workflow from configuration loading
//! through database operations and feed processing.

use std::io::Write;
use tempfile::NamedTempFile;

// Re-export modules for integration testing
// Note: For integration tests, we need to import from the crate

mod common {
    use tempfile::TempDir;

    /// Create a temporary directory for test databases
    pub fn create_temp_dir() -> TempDir {
        tempfile::tempdir().expect("Failed to create temp directory")
    }

    /// Create a test database path
    pub fn create_db_path(temp_dir: &TempDir) -> String {
        let db_path = temp_dir.path().join("test.db");
        format!("sqlite:{}?mode=rwc", db_path.display())
    }
}

#[cfg(test)]
mod config_integration_tests {
    use super::*;
    use moar_news::config::Config;

    #[test]
    fn test_load_actual_feeds_config() {
        // Test loading the actual feeds.toml from the project
        let config = Config::load("feeds.toml");
        assert!(config.is_ok(), "Failed to load feeds.toml: {:?}", config.err());

        let config = config.unwrap();
        assert!(!config.feeds.is_empty(), "feeds.toml should have at least one feed");
        assert!(config.refresh_interval > 0, "refresh_interval should be positive");
    }

    #[test]
    fn test_config_round_trip() {
        let toml_content = r#"
            refresh_interval = 30

            [[feeds]]
            name = "Hacker News"
            url = "https://news.ycombinator.com/rss"
            has_discussion = true

            [[feeds]]
            name = "Lobste.rs"
            url = "https://lobste.rs/rss"
            has_discussion = true

            [[feeds]]
            name = "Tech Blog"
            url = "https://blog.example.com/feed.xml"
        "#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(toml_content.as_bytes()).unwrap();

        let config = Config::load(temp_file.path()).unwrap();

        assert_eq!(config.refresh_interval, 30);
        assert_eq!(config.feeds.len(), 3);

        // Verify HN config
        assert_eq!(config.feeds[0].name, "Hacker News");
        assert!(config.feeds[0].has_discussion);

        // Verify Lobste.rs config
        assert_eq!(config.feeds[1].name, "Lobste.rs");
        assert!(config.feeds[1].has_discussion);

        // Verify blog config (default has_discussion)
        assert_eq!(config.feeds[2].name, "Tech Blog");
        assert!(!config.feeds[2].has_discussion);
    }
}

#[cfg(test)]
mod database_integration_tests {
    use super::common::*;
    use chrono::Utc;
    use moar_news::config::FeedConfig;
    use moar_news::db::Database;

    #[tokio::test]
    async fn test_full_database_workflow() {
        let temp_dir = create_temp_dir();
        let db_url = create_db_path(&temp_dir);

        // Create and initialize database
        let db = Database::new(&db_url).await.unwrap();
        db.initialize().await.unwrap();

        // Sync feeds
        let configs = vec![
            FeedConfig {
                name: "Test Feed".to_string(),
                url: "https://test.com/rss".to_string(),
                has_discussion: true,
            },
        ];
        db.sync_feeds(&configs).await.unwrap();

        // Verify feed was created
        let feeds = db.get_all_feeds().await.unwrap();
        assert_eq!(feeds.len(), 1);
        let feed = &feeds[0];
        assert_eq!(feed.name, "Test Feed");

        // Add items
        for i in 1..=25 {
            let published = Utc::now() - chrono::Duration::hours(25 - i);
            db.upsert_item(
                feed.id,
                &format!("guid-{}", i),
                &format!("Article {}", i),
                &format!("https://article{}.example.com", i),
                Some(&format!("https://discuss{}.example.com", i)),
                Some(published),
            )
            .await
            .unwrap();
        }

        // Verify item count
        let count = db.get_item_count_for_feed(feed.id).await.unwrap();
        assert_eq!(count, 25);

        // Test pagination - first page
        let page1 = db.get_items_for_feed(feed.id, 10, 0).await.unwrap();
        assert_eq!(page1.len(), 10);
        assert_eq!(page1[0].title, "Article 25"); // Most recent first

        // Test pagination - second page
        let page2 = db.get_items_for_feed(feed.id, 10, 10).await.unwrap();
        assert_eq!(page2.len(), 10);
        assert_ne!(page1[0].guid, page2[0].guid);

        // Test pagination - last page
        let page3 = db.get_items_for_feed(feed.id, 10, 20).await.unwrap();
        assert_eq!(page3.len(), 5); // Only 5 remaining

        // Test update feed fetched
        db.update_feed_fetched(feed.id, None, None).await.unwrap();
        let updated_feed = db.get_feed(feed.id).await.unwrap().unwrap();
        assert!(updated_feed.last_fetched.is_some());
        assert!(updated_feed.last_error.is_none());
    }

    #[tokio::test]
    async fn test_database_persistence() {
        let temp_dir = create_temp_dir();
        let db_url = create_db_path(&temp_dir);

        // Create database and add data
        {
            let db = Database::new(&db_url).await.unwrap();
            db.initialize().await.unwrap();

            let configs = vec![FeedConfig {
                name: "Persistent Feed".to_string(),
                url: "https://persistent.com/rss".to_string(),
                has_discussion: false,
            }];
            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();
            db.upsert_item(
                feeds[0].id,
                "persistent-guid",
                "Persistent Article",
                "https://persistent.com/article",
                None,
                None,
            )
            .await
            .unwrap();
        }

        // Reopen database and verify data persists
        {
            let db = Database::new(&db_url).await.unwrap();
            // Don't reinitialize - just use existing data

            let feeds = db.get_all_feeds().await.unwrap();
            assert_eq!(feeds.len(), 1);
            assert_eq!(feeds[0].name, "Persistent Feed");

            let items = db.get_items_for_feed(feeds[0].id, 10, 0).await.unwrap();
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].title, "Persistent Article");
        }
    }

    #[tokio::test]
    async fn test_concurrent_item_updates() {
        let temp_dir = create_temp_dir();
        let db_url = create_db_path(&temp_dir);

        let db = Database::new(&db_url).await.unwrap();
        db.initialize().await.unwrap();

        let configs = vec![FeedConfig {
            name: "Concurrent Feed".to_string(),
            url: "https://concurrent.com/rss".to_string(),
            has_discussion: false,
        }];
        db.sync_feeds(&configs).await.unwrap();
        let feeds = db.get_all_feeds().await.unwrap();
        let feed_id = feeds[0].id;

        // Simulate concurrent updates (upsert same items multiple times)
        for _ in 0..3 {
            for i in 1..=10 {
                db.upsert_item(
                    feed_id,
                    &format!("guid-{}", i),
                    &format!("Article {} - Updated", i),
                    &format!("https://article{}.com", i),
                    None,
                    None,
                )
                .await
                .unwrap();
            }
        }

        // Should still only have 10 items (upsert, not insert)
        let count = db.get_item_count_for_feed(feed_id).await.unwrap();
        assert_eq!(count, 10);

        // All should have "Updated" in title
        let items = db.get_items_for_feed(feed_id, 10, 0).await.unwrap();
        for item in items {
            assert!(item.title.contains("Updated"));
        }
    }
}

#[cfg(test)]
mod fetcher_integration_tests {
    use moar_news::db::Feed;
    use moar_news::fetcher::Fetcher;

    fn create_test_feed(url: &str, has_discussion: bool) -> Feed {
        Feed {
            id: 1,
            name: "Test".to_string(),
            url: url.to_string(),
            has_discussion,
            last_fetched: None,
            last_error: None,
            homepage_url: None,
        }
    }

    #[test]
    fn test_xml_parsing_real_rss_format() {
        let rss_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
            <rss version="2.0">
                <channel>
                    <title>Tech News</title>
                    <link>https://technews.example.com</link>
                    <description>Latest tech news</description>
                    <item>
                        <title>Breaking: New Technology Announced</title>
                        <link>https://technews.example.com/article/1</link>
                        <guid>https://technews.example.com/article/1</guid>
                        <comments>https://technews.example.com/article/1/comments</comments>
                        <pubDate>Mon, 09 Dec 2024 12:00:00 GMT</pubDate>
                    </item>
                    <item>
                        <title>Review: Latest Gadget</title>
                        <link>https://technews.example.com/article/2</link>
                        <guid>https://technews.example.com/article/2</guid>
                        <comments>https://technews.example.com/article/2/comments</comments>
                        <pubDate>Mon, 09 Dec 2024 10:00:00 GMT</pubDate>
                    </item>
                </channel>
            </rss>
        "#;

        let comments = Fetcher::extract_comments_from_xml(rss_xml.as_bytes());

        assert_eq!(comments.len(), 2);
        assert_eq!(
            comments.get("https://technews.example.com/article/1"),
            Some(&"https://technews.example.com/article/1/comments".to_string())
        );
        assert_eq!(
            comments.get("https://technews.example.com/article/2"),
            Some(&"https://technews.example.com/article/2/comments".to_string())
        );
    }

    fn create_link(href: &str, rel: Option<&str>) -> feed_rs::model::Link {
        feed_rs::model::Link {
            href: href.to_string(),
            rel: rel.map(|r| r.to_string()),
            media_type: None,
            href_lang: None,
            title: None,
            length: None,
        }
    }

    #[test]
    fn test_hacker_news_format_discussion_detection() {
        use feed_rs::model::Entry;

        let hn_feed = create_test_feed("https://news.ycombinator.com/rss", true);

        // HN entry format: guid is the discussion URL
        let hn_entry = Entry {
            id: "https://news.ycombinator.com/item?id=42345678".to_string(),
            links: vec![create_link("https://external-article.com/cool-article", None)],
            ..Default::default()
        };

        let discussion = Fetcher::extract_discussion_link(
            &hn_feed,
            &hn_entry,
            None,
            "https://external-article.com/cool-article",
        );

        assert_eq!(
            discussion,
            Some("https://news.ycombinator.com/item?id=42345678".to_string())
        );
    }

    #[test]
    fn test_lobsters_format_discussion_detection() {
        use feed_rs::model::Entry;

        let lobsters_feed = create_test_feed("https://lobste.rs/rss", true);

        let lobsters_entry = Entry {
            id: "https://lobste.rs/s/abc123".to_string(),
            links: vec![create_link("https://blog.example.com/post", None)],
            ..Default::default()
        };

        let discussion = Fetcher::extract_discussion_link(
            &lobsters_feed,
            &lobsters_entry,
            None,
            "https://blog.example.com/post",
        );

        assert_eq!(
            discussion,
            Some("https://lobste.rs/s/abc123".to_string())
        );
    }

    #[test]
    fn test_generic_feed_comments_from_xml() {
        use feed_rs::model::Entry;

        let generic_feed = create_test_feed("https://blog.example.com/feed", true);

        let entry = Entry {
            id: "post-123".to_string(),
            links: vec![create_link("https://blog.example.com/posts/123", None)],
            ..Default::default()
        };

        let comments_url = "https://blog.example.com/posts/123/comments".to_string();
        let discussion = Fetcher::extract_discussion_link(
            &generic_feed,
            &entry,
            Some(&comments_url),
            "https://blog.example.com/posts/123",
        );

        assert_eq!(discussion, Some(comments_url));
    }
}

#[cfg(test)]
mod end_to_end_tests {
    use super::common::*;
    use moar_news::config::FeedConfig;
    use moar_news::db::Database;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_config_to_database_workflow() {
        let temp_dir = create_temp_dir();
        let db_url = create_db_path(&temp_dir);

        // Simulate the config → database sync workflow from main.rs
        let configs = vec![
            FeedConfig {
                name: "Hacker News".to_string(),
                url: "https://news.ycombinator.com/rss".to_string(),
                has_discussion: true,
            },
            FeedConfig {
                name: "Lobste.rs".to_string(),
                url: "https://lobste.rs/rss".to_string(),
                has_discussion: true,
            },
            FeedConfig {
                name: "Ars Technica".to_string(),
                url: "https://feeds.arstechnica.com/arstechnica/technology-lab".to_string(),
                has_discussion: false,
            },
        ];

        let db = Database::new(&db_url).await.unwrap();
        db.initialize().await.unwrap();
        db.sync_feeds(&configs).await.unwrap();

        let db = Arc::new(db);

        // Verify all feeds were synced
        let feeds = db.get_all_feeds().await.unwrap();
        assert_eq!(feeds.len(), 3);

        // Verify feed properties
        let hn = feeds.iter().find(|f| f.name == "Hacker News").unwrap();
        assert!(hn.has_discussion);
        assert!(hn.url.contains("news.ycombinator.com"));

        let lobsters = feeds.iter().find(|f| f.name == "Lobste.rs").unwrap();
        assert!(lobsters.has_discussion);

        let ars = feeds.iter().find(|f| f.name == "Ars Technica").unwrap();
        assert!(!ars.has_discussion);
    }

    #[tokio::test]
    async fn test_feed_update_workflow() {
        let temp_dir = create_temp_dir();
        let db_url = create_db_path(&temp_dir);

        let db = Database::new(&db_url).await.unwrap();
        db.initialize().await.unwrap();

        // Initial config
        let initial_configs = vec![FeedConfig {
            name: "Original Name".to_string(),
            url: "https://feed.example.com/rss".to_string(),
            has_discussion: false,
        }];
        db.sync_feeds(&initial_configs).await.unwrap();

        // Verify initial state
        let feeds = db.get_all_feeds().await.unwrap();
        assert_eq!(feeds[0].name, "Original Name");
        assert!(!feeds[0].has_discussion);

        // Update config (same URL, different properties)
        let updated_configs = vec![FeedConfig {
            name: "Updated Name".to_string(),
            url: "https://feed.example.com/rss".to_string(),
            has_discussion: true,
        }];
        db.sync_feeds(&updated_configs).await.unwrap();

        // Verify update
        let feeds = db.get_all_feeds().await.unwrap();
        assert_eq!(feeds.len(), 1); // Still only one feed
        assert_eq!(feeds[0].name, "Updated Name");
        assert!(feeds[0].has_discussion);
    }
}
