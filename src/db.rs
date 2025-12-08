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
                last_error TEXT
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

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

    pub async fn update_feed_fetched(&self, feed_id: i64, error: Option<&str>) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            UPDATE feeds
            SET last_fetched = ?, last_error = ?
            WHERE id = ?
            "#,
        )
        .bind(&now)
        .bind(error)
        .bind(feed_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
