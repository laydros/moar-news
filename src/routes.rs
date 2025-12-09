use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use serde::Deserialize;

use crate::db::{Database, Feed, Item};
use crate::fetcher::Fetcher;

const ITEMS_PER_PAGE: i64 = 15;

pub struct AppState {
    pub db: Arc<Database>,
    pub fetcher: Arc<Fetcher>,
}

// Template structs
#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub feeds: Vec<FeedWithItems>,
}

pub struct FeedWithItems {
    pub feed: Feed,
    pub items: Vec<Item>,
    pub has_more: bool,
}

#[derive(Template)]
#[template(path = "feed_items.html")]
pub struct FeedItemsTemplate {
    pub feed: Feed,
    pub items: Vec<Item>,
    pub offset: i64,
    pub has_more: bool,
}

#[derive(Template)]
#[template(path = "refresh_button.html")]
pub struct RefreshButtonTemplate {
    pub refreshing: bool,
}

// Wrapper for HTML responses
struct HtmlTemplate<T>(T);

impl<T: Template> IntoResponse for HtmlTemplate<T> {
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

// Custom error type
pub struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Error: {}", self.0),
        )
            .into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(err: E) -> Self {
        AppError(err.into())
    }
}

// Route handlers
pub async fn index(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let feeds = state.db.get_all_feeds().await?;

    let mut feeds_with_items = Vec::new();
    for feed in feeds {
        let items = state
            .db
            .get_items_for_feed(feed.id, ITEMS_PER_PAGE, 0)
            .await?;
        let total = state.db.get_item_count_for_feed(feed.id).await?;
        let has_more = total > ITEMS_PER_PAGE;

        feeds_with_items.push(FeedWithItems {
            feed,
            items,
            has_more,
        });
    }

    Ok(HtmlTemplate(IndexTemplate {
        feeds: feeds_with_items,
    }))
}

#[derive(Deserialize)]
pub struct MoreQuery {
    #[serde(default)]
    pub offset: i64,
}

pub async fn feed_more(
    State(state): State<Arc<AppState>>,
    Path(feed_id): Path<i64>,
    Query(query): Query<MoreQuery>,
) -> Result<impl IntoResponse, AppError> {
    let feed = state
        .db
        .get_feed(feed_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Feed not found"))?;

    let offset = query.offset;
    let items = state
        .db
        .get_items_for_feed(feed_id, ITEMS_PER_PAGE, offset)
        .await?;
    let total = state.db.get_item_count_for_feed(feed_id).await?;
    let has_more = offset + ITEMS_PER_PAGE < total;

    Ok(HtmlTemplate(FeedItemsTemplate {
        feed,
        items,
        offset: offset + ITEMS_PER_PAGE,
        has_more,
    }))
}

pub async fn refresh(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    // Spawn the refresh task
    let fetcher = state.fetcher.clone();
    tokio::spawn(async move {
        let _ = fetcher.refresh_all_feeds().await;
    });

    // Return refreshing state immediately
    Ok(HtmlTemplate(RefreshButtonTemplate { refreshing: true }))
}

pub async fn refresh_status(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let refreshing = state.fetcher.is_refreshing().await;
    Ok(HtmlTemplate(RefreshButtonTemplate { refreshing }))
}

pub async fn health() -> impl IntoResponse {
    Html("OK")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FeedConfig;
    use crate::db::Database;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::{get, post},
        Router,
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn create_test_app() -> (Router, Arc<Database>) {
        let db = Database::new("sqlite::memory:").await.unwrap();
        db.initialize().await.unwrap();
        let db = Arc::new(db);

        let fetcher = Arc::new(Fetcher::new(db.clone()));
        let state = Arc::new(AppState {
            db: db.clone(),
            fetcher,
        });

        let app = Router::new()
            .route("/", get(index))
            .route("/feed/:id/more", get(feed_more))
            .route("/refresh", post(refresh))
            .route("/refresh/status", get(refresh_status))
            .route("/health", get(health))
            .with_state(state);

        (app, db)
    }

    async fn setup_test_data(db: &Database) {
        let configs = vec![
            FeedConfig {
                name: "Test Feed 1".to_string(),
                url: "https://feed1.com/rss".to_string(),
                has_discussion: true,
            },
            FeedConfig {
                name: "Test Feed 2".to_string(),
                url: "https://feed2.com/rss".to_string(),
                has_discussion: false,
            },
        ];
        db.sync_feeds(&configs).await.unwrap();

        // Add items to first feed
        let feeds = db.get_all_feeds().await.unwrap();
        for i in 1..=20 {
            let published = chrono::Utc::now() - chrono::Duration::hours(20 - i);
            db.upsert_item(
                feeds[0].id,
                &format!("guid-{}", i),
                &format!("Article {}", i),
                &format!("https://article{}.com", i),
                None,
                Some(published),
            )
            .await
            .unwrap();
        }
    }

    mod health_tests {
        use super::*;

        #[tokio::test]
        async fn test_health_endpoint() {
            let (app, _db) = create_test_app().await;

            let response = app
                .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);

            let body = response.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(&body[..], b"OK");
        }
    }

    mod index_tests {
        use super::*;

        #[tokio::test]
        async fn test_index_empty_feeds() {
            let (app, _db) = create_test_app().await;

            let response = app
                .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);
        }

        #[tokio::test]
        async fn test_index_with_feeds() {
            let (app, db) = create_test_app().await;
            setup_test_data(&db).await;

            let response = app
                .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);

            let body = response.into_body().collect().await.unwrap().to_bytes();
            let body_str = String::from_utf8(body.to_vec()).unwrap();

            // Check that feed names appear in the response
            assert!(body_str.contains("Test Feed 1"));
            assert!(body_str.contains("Test Feed 2"));
        }

        #[tokio::test]
        async fn test_index_shows_items() {
            let (app, db) = create_test_app().await;
            setup_test_data(&db).await;

            let response = app
                .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
                .await
                .unwrap();

            let body = response.into_body().collect().await.unwrap().to_bytes();
            let body_str = String::from_utf8(body.to_vec()).unwrap();

            // Check that some article titles appear
            assert!(body_str.contains("Article"));
        }
    }

    mod feed_more_tests {
        use super::*;

        #[tokio::test]
        async fn test_feed_more_returns_items() {
            let (app, db) = create_test_app().await;
            setup_test_data(&db).await;

            let feeds = db.get_all_feeds().await.unwrap();
            let feed_id = feeds[0].id;

            let response = app
                .oneshot(
                    Request::builder()
                        .uri(format!("/feed/{}/more?offset=0", feed_id))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);

            let body = response.into_body().collect().await.unwrap().to_bytes();
            let body_str = String::from_utf8(body.to_vec()).unwrap();
            assert!(body_str.contains("Article"));
        }

        #[tokio::test]
        async fn test_feed_more_with_offset() {
            let (app, db) = create_test_app().await;
            setup_test_data(&db).await;

            let feeds = db.get_all_feeds().await.unwrap();
            let feed_id = feeds[0].id;

            let response = app
                .oneshot(
                    Request::builder()
                        .uri(format!("/feed/{}/more?offset=15", feed_id))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);
        }

        #[tokio::test]
        async fn test_feed_more_nonexistent_feed() {
            let (app, _db) = create_test_app().await;

            let response = app
                .oneshot(
                    Request::builder()
                        .uri("/feed/999/more?offset=0")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    mod refresh_tests {
        use super::*;

        #[tokio::test]
        async fn test_refresh_endpoint() {
            let (app, _db) = create_test_app().await;

            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/refresh")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);

            let body = response.into_body().collect().await.unwrap().to_bytes();
            let body_str = String::from_utf8(body.to_vec()).unwrap();

            // Should indicate refreshing state
            assert!(body_str.contains("Refreshing") || body_str.contains("refresh"));
        }

        #[tokio::test]
        async fn test_refresh_status_endpoint() {
            let (app, _db) = create_test_app().await;

            let response = app
                .oneshot(
                    Request::builder()
                        .uri("/refresh/status")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);
        }
    }

    mod pagination_tests {
        use super::*;

        #[tokio::test]
        async fn test_has_more_flag_when_items_exceed_page_size() {
            let (app, db) = create_test_app().await;
            setup_test_data(&db).await; // Creates 20 items, ITEMS_PER_PAGE is 15

            let response = app
                .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
                .await
                .unwrap();

            let body = response.into_body().collect().await.unwrap().to_bytes();
            let body_str = String::from_utf8(body.to_vec()).unwrap();

            // Should contain "Show More" button for feed with 20 items
            assert!(body_str.contains("Show More") || body_str.contains("hx-get"));
        }
    }

    mod more_query_tests {
        use super::*;

        #[test]
        fn test_more_query_default_offset() {
            // This tests the MoreQuery struct's default behavior
            let query: MoreQuery = serde_urlencoded::from_str("").unwrap();
            assert_eq!(query.offset, 0);
        }

        #[test]
        fn test_more_query_with_offset() {
            let query: MoreQuery = serde_urlencoded::from_str("offset=10").unwrap();
            assert_eq!(query.offset, 10);
        }
    }
}
