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
