mod config;
mod connectors;
mod db;
mod middleware;
mod sse;
mod templates;
mod worker;
mod workflow_engine;
mod api;

use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower_http::cors::{CorsLayer, Any};
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub config: config::Config,
    pub broadcaster: sse::SseBroadcaster,
    pub rate_limiter: middleware::RateLimiter,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "notifyd=info,tower_http=warn".to_string())
                .as_str(),
        )
        .init();

    let config = config::Config::from_env()?;

    let pool = PgPoolOptions::new()
        .max_connections(config.database.max_connections)
        .connect(&config.database.url)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;
    info!("Migrations applied");

    let broadcaster = sse::SseBroadcaster::new();
    let rate_limiter = middleware::RateLimiter::new();

    let state = Arc::new(AppState {
        pool: pool.clone(),
        config: config.clone(),
        broadcaster: broadcaster.clone(),
        rate_limiter: rate_limiter.clone(),
    });

    // Worker
    let worker_state = state.clone();
    tokio::spawn(async move { worker::run(worker_state).await; });

    // SSE cleanup
    let cleanup_broadcaster = broadcaster.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            cleanup_broadcaster.cleanup().await;
        }
    });

    // Rate limiter cleanup
    let rl = rate_limiter.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(120));
        loop {
            interval.tick().await;
            rl.cleanup().await;
        }
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = api::router(state)
        .layer(cors)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let port = config.server.port;
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    info!("notifyd listening on port {}", port);

    axum::serve(listener, app).await?;
    Ok(())
}
