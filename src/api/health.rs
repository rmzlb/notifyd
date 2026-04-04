use crate::api::projects::require_admin;
use crate::AppState;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    let db_ok = sqlx::query("SELECT 1").fetch_one(&state.pool).await.is_ok();
    Json(json!({
        "status": if db_ok { "ok" } else { "degraded" },
        "db": if db_ok { "ok" } else { "error" },
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// GET /v1/metrics — requires admin API key
pub async fn metrics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_admin(&headers)?;

    let jobs_pending: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status = 'pending'")
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0);

    let jobs_processing: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status = 'processing'")
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0);

    let jobs_sent_24h: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM jobs WHERE status = 'sent' AND sent_at > now() - interval '24 hours'",
    )
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0);

    let jobs_failed_24h: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM jobs WHERE status = 'failed' AND created_at > now() - interval '24 hours'"
    ).fetch_one(&state.pool).await.unwrap_or(0);

    let subscribers_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM subscribers")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);

    let inbox_messages_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM inbox_messages")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);

    let active_workflow_runs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM workflow_runs WHERE status IN ('running', 'paused')",
    )
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0);

    let uptime_seconds = state.started_at.elapsed().as_secs();

    Ok(Json(json!({
        "jobs_pending": jobs_pending,
        "jobs_processing": jobs_processing,
        "jobs_sent_24h": jobs_sent_24h,
        "jobs_failed_24h": jobs_failed_24h,
        "subscribers_total": subscribers_total,
        "inbox_messages_total": inbox_messages_total,
        "active_workflow_runs": active_workflow_runs,
        "uptime_seconds": uptime_seconds,
    })))
}
