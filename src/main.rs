mod config;
mod db;
mod fetcher;
mod routes;

use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};
use tower_http::services::ServeDir;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::Config;
use crate::db::Database;
use crate::fetcher::{start_background_refresh, Fetcher};
use crate::routes::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "moar_news=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let config = Config::load("feeds.toml")?;
    info!("Loaded {} feeds from configuration", config.feeds.len());

    // Initialize database
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:moar_news.db?mode=rwc".to_string());
    let db = Database::new(&database_url).await?;
    db.initialize().await?;
    db.sync_feeds(&config.feeds).await?;
    info!("Database initialized");

    let db = Arc::new(db);

    // Create fetcher
    let fetcher = Arc::new(Fetcher::new(db.clone()));

    // Start background refresh task
    let bg_fetcher = fetcher.clone();
    let refresh_interval = config.refresh_interval;
    tokio::spawn(async move {
        start_background_refresh(bg_fetcher, refresh_interval).await;
    });

    // Create app state
    let state = Arc::new(AppState {
        db: db.clone(),
        fetcher: fetcher.clone(),
    });

    // Build router
    let app = Router::new()
        .route("/", get(routes::index))
        .route("/feed/:id/more", get(routes::feed_more))
        .route("/refresh", post(routes::refresh))
        .route("/refresh/status", get(routes::refresh_status))
        .route("/health", get(routes::health))
        .nest_service("/static", ServeDir::new("static"))
        .with_state(state);

    // Start server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    info!("Server starting on http://localhost:3000");

    axum::serve(listener, app).await?;

    Ok(())
}
