mod api;
mod config;
mod connectors;
mod db;
mod middleware;
mod pii;
mod sse;
mod templates;
mod webhooks;
mod worker;
mod workflow_engine;

use axum::{
    http::{HeaderValue, Request},
    middleware::Next,
    response::Response,
};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::watch;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub config: config::Config,
    pub broadcaster: sse::SseBroadcaster,
    pub rate_limiter: middleware::RateLimiter,
    pub started_at: Instant,
}

// Implement manually since Instant doesn't impl Debug
impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState").finish()
    }
}

/// Feature #12: Request ID middleware
async fn request_id_middleware(mut req: Request<axum::body::Body>, next: Next) -> Response {
    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // Make available for tracing
    let span = tracing::Span::current();
    span.record("request_id", &request_id.as_str());

    req.extensions_mut().insert(request_id.clone());

    let mut response = next.run(req).await;

    if let Ok(val) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", val);
    }

    response
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
        started_at: Instant::now(),
    });

    // Feature #13: Graceful shutdown signal
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Worker
    let worker_state = state.clone();
    let worker_shutdown = shutdown_rx.clone();
    let worker_handle = tokio::spawn(async move {
        worker::run(worker_state, worker_shutdown).await;
    });

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

    let cors = {
        let origins_str = std::env::var("CORS_ORIGINS")
            .unwrap_or_else(|_| "http://localhost:3000,http://localhost:5173".to_string());
        let origins: Vec<_> = origins_str
            .split(',')
            .filter_map(|s| s.trim().parse::<axum::http::HeaderValue>().ok())
            .collect();
        if origins.is_empty() {
            CorsLayer::new().allow_origin(Any)
        } else {
            CorsLayer::new().allow_origin(origins)
        }
    }
    .allow_methods(Any)
    .allow_headers(Any);

    let app = api::router(state)
        .layer(axum::middleware::from_fn(request_id_middleware))
        .layer(axum::extract::DefaultBodyLimit::max(1_048_576))
        .layer(cors)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let port = config.server.port;
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    info!("notifyd listening on port {}", port);

    // Feature #13: Graceful shutdown
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("failed to install SIGTERM handler");

            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    info!("SIGINT received, shutting down gracefully...");
                }
                _ = sigterm.recv() => {
                    info!("SIGTERM received, shutting down gracefully...");
                }
            }

            // Signal worker to stop
            let _ = shutdown_tx.send(true);

            // Wait for worker with timeout
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), worker_handle).await;

            // Close DB pool
            pool.close().await;

            info!("Shutdown complete");
        })
        .await?;

    Ok(())
}
