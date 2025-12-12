use chrono::{DateTime, Utc};
use sqlx::{sqlite::SqlitePoolOptions, FromRow, SqlitePool};

use crate::config::FeedConfig;

#[derive(Debug, Clone, FromRow)]
pub struct Feed {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub has_discussion: bool,
    pub last_fetched: Option<String>,
    pub last_error: Option<String>,
    pub homepage_url: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct Item {
    pub id: i64,
    pub feed_id: i64,
    pub guid: String,
    pub title: String,
    pub link: String,
    pub discussion_link: Option<String>,
    pub published: Option<String>,
}

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn new(database_url: &str) -> anyhow::Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;

        Ok(Self { pool })
    }

    pub async fn initialize(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS feeds (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                url TEXT NOT NULL UNIQUE,
                has_discussion INTEGER DEFAULT 0,
                last_fetched TEXT,
                last_error TEXT,
                homepage_url TEXT
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Migration: add homepage_url column if it doesn't exist
        let _ = sqlx::query("ALTER TABLE feeds ADD COLUMN homepage_url TEXT")
            .execute(&self.pool)
            .await;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS items (
                id INTEGER PRIMARY KEY,
                feed_id INTEGER NOT NULL REFERENCES feeds(id),
                guid TEXT NOT NULL,
                title TEXT NOT NULL,
                link TEXT NOT NULL,
                discussion_link TEXT,
                published TEXT,
                UNIQUE(feed_id, guid)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_items_feed_published
            ON items(feed_id, published DESC)
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn sync_feeds(&self, configs: &[FeedConfig]) -> anyhow::Result<()> {
        for config in configs {
            sqlx::query(
                r#"
                INSERT INTO feeds (name, url, has_discussion)
                VALUES (?, ?, ?)
                ON CONFLICT(url) DO UPDATE SET
                    name = excluded.name,
                    has_discussion = excluded.has_discussion
                "#,
            )
            .bind(&config.name)
            .bind(&config.url)
            .bind(config.has_discussion)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    pub async fn get_all_feeds(&self) -> anyhow::Result<Vec<Feed>> {
        let feeds = sqlx::query_as::<_, Feed>("SELECT * FROM feeds ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        Ok(feeds)
    }

    pub async fn get_feed(&self, feed_id: i64) -> anyhow::Result<Option<Feed>> {
        let feed = sqlx::query_as::<_, Feed>("SELECT * FROM feeds WHERE id = ?")
            .bind(feed_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(feed)
    }

    pub async fn get_items_for_feed(
        &self,
        feed_id: i64,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Vec<Item>> {
        let items = sqlx::query_as::<_, Item>(
            r#"
            SELECT * FROM items
            WHERE feed_id = ?
            ORDER BY published DESC NULLS LAST, id DESC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(feed_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(items)
    }

    pub async fn get_item_count_for_feed(&self, feed_id: i64) -> anyhow::Result<i64> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM items WHERE feed_id = ?")
            .bind(feed_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(count.0)
    }

    pub async fn upsert_item(
        &self,
        feed_id: i64,
        guid: &str,
        title: &str,
        link: &str,
        discussion_link: Option<&str>,
        published: Option<DateTime<Utc>>,
    ) -> anyhow::Result<()> {
        let published_str = published.map(|p| p.to_rfc3339());

        sqlx::query(
            r#"
            INSERT INTO items (feed_id, guid, title, link, discussion_link, published)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(feed_id, guid) DO UPDATE SET
                title = excluded.title,
                link = excluded.link,
                discussion_link = excluded.discussion_link,
                published = excluded.published
            "#,
        )
        .bind(feed_id)
        .bind(guid)
        .bind(title)
        .bind(link)
        .bind(discussion_link)
        .bind(published_str)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn update_feed_fetched(
        &self,
        feed_id: i64,
        error: Option<&str>,
        homepage_url: Option<&str>,
    ) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            UPDATE feeds
            SET last_fetched = ?, last_error = ?, homepage_url = COALESCE(?, homepage_url)
            WHERE id = ?
            "#,
        )
        .bind(&now)
        .bind(error)
        .bind(homepage_url)
        .bind(feed_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FeedConfig;

    async fn create_test_db() -> Database {
        let db = Database::new("sqlite::memory:").await.unwrap();
        db.initialize().await.unwrap();
        db
    }

    fn create_feed_config(name: &str, url: &str, has_discussion: bool) -> FeedConfig {
        FeedConfig {
            name: name.to_string(),
            url: url.to_string(),
            has_discussion,
        }
    }

    // Database initialization tests
    mod initialization_tests {
        use super::*;

        #[tokio::test]
        async fn test_database_creation() {
            let db = Database::new("sqlite::memory:").await;
            assert!(db.is_ok());
        }

        #[tokio::test]
        async fn test_database_initialization() {
            let db = create_test_db().await;
            // If we get here without error, initialization succeeded
            let feeds = db.get_all_feeds().await.unwrap();
            assert!(feeds.is_empty());
        }

        #[tokio::test]
        async fn test_double_initialization_is_safe() {
            let db = create_test_db().await;
            // Initialize again - should not fail due to IF NOT EXISTS
            let result = db.initialize().await;
            assert!(result.is_ok());
        }
    }

    // Feed sync tests
    mod sync_feeds_tests {
        use super::*;

        #[tokio::test]
        async fn test_sync_single_feed() {
            let db = create_test_db().await;
            let configs = vec![create_feed_config(
                "Test Feed",
                "https://example.com/rss",
                false,
            )];

            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();
            assert_eq!(feeds.len(), 1);
            assert_eq!(feeds[0].name, "Test Feed");
            assert_eq!(feeds[0].url, "https://example.com/rss");
            assert!(!feeds[0].has_discussion);
        }

        #[tokio::test]
        async fn test_sync_multiple_feeds() {
            let db = create_test_db().await;
            let configs = vec![
                create_feed_config("Feed 1", "https://feed1.com/rss", true),
                create_feed_config("Feed 2", "https://feed2.com/rss", false),
                create_feed_config("Feed 3", "https://feed3.com/rss", true),
            ];

            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();
            assert_eq!(feeds.len(), 3);
        }

        #[tokio::test]
        async fn test_sync_updates_existing_feed() {
            let db = create_test_db().await;

            // Initial sync
            let configs = vec![create_feed_config(
                "Original Name",
                "https://example.com/rss",
                false,
            )];
            db.sync_feeds(&configs).await.unwrap();

            // Update with same URL but different name and has_discussion
            let configs = vec![create_feed_config(
                "Updated Name",
                "https://example.com/rss",
                true,
            )];
            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();
            assert_eq!(feeds.len(), 1);
            assert_eq!(feeds[0].name, "Updated Name");
            assert!(feeds[0].has_discussion);
        }

        #[tokio::test]
        async fn test_sync_empty_feeds() {
            let db = create_test_db().await;
            let configs: Vec<FeedConfig> = vec![];

            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();
            assert!(feeds.is_empty());
        }
    }

    // Get feed tests
    mod get_feed_tests {
        use super::*;

        #[tokio::test]
        async fn test_get_existing_feed() {
            let db = create_test_db().await;
            let configs = vec![create_feed_config("Test", "https://test.com/rss", false)];
            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();
            let feed = db.get_feed(feeds[0].id).await.unwrap();

            assert!(feed.is_some());
            assert_eq!(feed.unwrap().name, "Test");
        }

        #[tokio::test]
        async fn test_get_nonexistent_feed() {
            let db = create_test_db().await;

            let feed = db.get_feed(999).await.unwrap();
            assert!(feed.is_none());
        }
    }

    // Item upsert tests
    mod upsert_item_tests {
        use super::*;

        #[tokio::test]
        async fn test_upsert_new_item() {
            let db = create_test_db().await;
            let configs = vec![create_feed_config("Test", "https://test.com/rss", false)];
            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();
            let feed_id = feeds[0].id;

            db.upsert_item(
                feed_id,
                "guid-123",
                "Test Title",
                "https://article.com",
                Some("https://comments.com"),
                Some(Utc::now()),
            )
            .await
            .unwrap();

            let items = db.get_items_for_feed(feed_id, 10, 0).await.unwrap();
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].title, "Test Title");
            assert_eq!(items[0].link, "https://article.com");
            assert_eq!(
                items[0].discussion_link,
                Some("https://comments.com".to_string())
            );
        }

        #[tokio::test]
        async fn test_upsert_item_without_discussion_link() {
            let db = create_test_db().await;
            let configs = vec![create_feed_config("Test", "https://test.com/rss", false)];
            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();
            let feed_id = feeds[0].id;

            db.upsert_item(
                feed_id,
                "guid-123",
                "Test Title",
                "https://article.com",
                None,
                None,
            )
            .await
            .unwrap();

            let items = db.get_items_for_feed(feed_id, 10, 0).await.unwrap();
            assert_eq!(items.len(), 1);
            assert!(items[0].discussion_link.is_none());
            assert!(items[0].published.is_none());
        }

        #[tokio::test]
        async fn test_upsert_updates_existing_item() {
            let db = create_test_db().await;
            let configs = vec![create_feed_config("Test", "https://test.com/rss", false)];
            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();
            let feed_id = feeds[0].id;

            // Insert initial item
            db.upsert_item(
                feed_id,
                "guid-123",
                "Original Title",
                "https://original.com",
                None,
                None,
            )
            .await
            .unwrap();

            // Update same item (same guid)
            db.upsert_item(
                feed_id,
                "guid-123",
                "Updated Title",
                "https://updated.com",
                Some("https://comments.com"),
                Some(Utc::now()),
            )
            .await
            .unwrap();

            let items = db.get_items_for_feed(feed_id, 10, 0).await.unwrap();
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].title, "Updated Title");
            assert_eq!(items[0].link, "https://updated.com");
        }

        #[tokio::test]
        async fn test_upsert_multiple_items() {
            let db = create_test_db().await;
            let configs = vec![create_feed_config("Test", "https://test.com/rss", false)];
            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();
            let feed_id = feeds[0].id;

            for i in 1..=5 {
                db.upsert_item(
                    feed_id,
                    &format!("guid-{}", i),
                    &format!("Title {}", i),
                    &format!("https://article{}.com", i),
                    None,
                    None,
                )
                .await
                .unwrap();
            }

            let items = db.get_items_for_feed(feed_id, 10, 0).await.unwrap();
            assert_eq!(items.len(), 5);
        }

        #[tokio::test]
        async fn test_same_guid_different_feeds() {
            let db = create_test_db().await;
            let configs = vec![
                create_feed_config("Feed 1", "https://feed1.com/rss", false),
                create_feed_config("Feed 2", "https://feed2.com/rss", false),
            ];
            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();

            // Same GUID in different feeds should create separate items
            db.upsert_item(feeds[0].id, "guid-123", "Title 1", "https://a.com", None, None)
                .await
                .unwrap();
            db.upsert_item(feeds[1].id, "guid-123", "Title 2", "https://b.com", None, None)
                .await
                .unwrap();

            let items1 = db.get_items_for_feed(feeds[0].id, 10, 0).await.unwrap();
            let items2 = db.get_items_for_feed(feeds[1].id, 10, 0).await.unwrap();

            assert_eq!(items1.len(), 1);
            assert_eq!(items2.len(), 1);
            assert_eq!(items1[0].title, "Title 1");
            assert_eq!(items2[0].title, "Title 2");
        }
    }

    // Pagination tests
    mod pagination_tests {
        use super::*;

        async fn setup_feed_with_items(db: &Database, count: i64) -> i64 {
            let configs = vec![create_feed_config("Test", "https://test.com/rss", false)];
            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();
            let feed_id = feeds[0].id;

            for i in 1..=count {
                let published = Utc::now() - chrono::Duration::hours(count - i);
                db.upsert_item(
                    feed_id,
                    &format!("guid-{}", i),
                    &format!("Title {}", i),
                    &format!("https://article{}.com", i),
                    None,
                    Some(published),
                )
                .await
                .unwrap();
            }

            feed_id
        }

        #[tokio::test]
        async fn test_get_items_with_limit() {
            let db = create_test_db().await;
            let feed_id = setup_feed_with_items(&db, 20).await;

            let items = db.get_items_for_feed(feed_id, 5, 0).await.unwrap();
            assert_eq!(items.len(), 5);
        }

        #[tokio::test]
        async fn test_get_items_with_offset() {
            let db = create_test_db().await;
            let feed_id = setup_feed_with_items(&db, 20).await;

            let first_page = db.get_items_for_feed(feed_id, 5, 0).await.unwrap();
            let second_page = db.get_items_for_feed(feed_id, 5, 5).await.unwrap();

            // Pages should have different items
            assert_eq!(first_page.len(), 5);
            assert_eq!(second_page.len(), 5);
            assert_ne!(first_page[0].id, second_page[0].id);
        }

        #[tokio::test]
        async fn test_get_items_offset_beyond_count() {
            let db = create_test_db().await;
            let feed_id = setup_feed_with_items(&db, 10).await;

            let items = db.get_items_for_feed(feed_id, 10, 100).await.unwrap();
            assert!(items.is_empty());
        }

        #[tokio::test]
        async fn test_get_item_count() {
            let db = create_test_db().await;
            let feed_id = setup_feed_with_items(&db, 15).await;

            let count = db.get_item_count_for_feed(feed_id).await.unwrap();
            assert_eq!(count, 15);
        }

        #[tokio::test]
        async fn test_get_item_count_empty_feed() {
            let db = create_test_db().await;
            let configs = vec![create_feed_config("Test", "https://test.com/rss", false)];
            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();
            let count = db.get_item_count_for_feed(feeds[0].id).await.unwrap();
            assert_eq!(count, 0);
        }

        #[tokio::test]
        async fn test_items_ordered_by_published_desc() {
            let db = create_test_db().await;
            let feed_id = setup_feed_with_items(&db, 5).await;

            let items = db.get_items_for_feed(feed_id, 10, 0).await.unwrap();

            // Most recent should be first (Title 5 has the most recent timestamp)
            assert_eq!(items[0].title, "Title 5");
            assert_eq!(items[4].title, "Title 1");
        }
    }

    // Update feed fetched tests
    mod update_feed_fetched_tests {
        use super::*;

        #[tokio::test]
        async fn test_update_feed_fetched_success() {
            let db = create_test_db().await;
            let configs = vec![create_feed_config("Test", "https://test.com/rss", false)];
            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();
            let feed_id = feeds[0].id;

            assert!(feeds[0].last_fetched.is_none());

            db.update_feed_fetched(feed_id, None, None).await.unwrap();

            let feed = db.get_feed(feed_id).await.unwrap().unwrap();
            assert!(feed.last_fetched.is_some());
            assert!(feed.last_error.is_none());
        }

        #[tokio::test]
        async fn test_update_feed_fetched_with_error() {
            let db = create_test_db().await;
            let configs = vec![create_feed_config("Test", "https://test.com/rss", false)];
            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();
            let feed_id = feeds[0].id;

            db.update_feed_fetched(feed_id, Some("Connection timeout"), None)
                .await
                .unwrap();

            let feed = db.get_feed(feed_id).await.unwrap().unwrap();
            assert!(feed.last_fetched.is_some());
            assert_eq!(feed.last_error, Some("Connection timeout".to_string()));
        }

        #[tokio::test]
        async fn test_update_clears_previous_error() {
            let db = create_test_db().await;
            let configs = vec![create_feed_config("Test", "https://test.com/rss", false)];
            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();
            let feed_id = feeds[0].id;

            // First update with error
            db.update_feed_fetched(feed_id, Some("Error 1"), None)
                .await
                .unwrap();

            // Second update without error
            db.update_feed_fetched(feed_id, None, None).await.unwrap();

            let feed = db.get_feed(feed_id).await.unwrap().unwrap();
            assert!(feed.last_error.is_none());
        }

        #[tokio::test]
        async fn test_update_feed_fetched_with_homepage_url() {
            let db = create_test_db().await;
            let configs = vec![create_feed_config("Test", "https://test.com/rss", false)];
            db.sync_feeds(&configs).await.unwrap();

            let feeds = db.get_all_feeds().await.unwrap();
            let feed_id = feeds[0].id;

            db.update_feed_fetched(feed_id, None, Some("https://test.com"))
                .await
                .unwrap();

            let feed = db.get_feed(feed_id).await.unwrap().unwrap();
            assert_eq!(feed.homepage_url, Some("https://test.com".to_string()));
        }
    }
}
