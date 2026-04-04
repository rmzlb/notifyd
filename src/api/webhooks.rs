use crate::api::projects::require_admin;
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct CreateWebhook {
    pub project_id: String,
    pub url: String,
    pub events: Vec<String>,
    pub secret: Option<String>,
    pub enabled: Option<bool>,
}

/// POST /v1/admin/webhooks
pub async fn create_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateWebhook>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_admin(&headers)?;

    let secret = req
        .secret
        .unwrap_or_else(|| hex::encode(Uuid::new_v4().as_bytes()));
    let events: Vec<&str> = req.events.iter().map(|s| s.as_str()).collect();

    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO webhooks (project_id, url, events, secret, enabled) VALUES ($1, $2, $3, $4, $5) RETURNING id"
    )
    .bind(&req.project_id)
    .bind(&req.url)
    .bind(&events)
    .bind(&secret)
    .bind(req.enabled.unwrap_or(true))
    .fetch_one(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("DB error: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Internal server error"})))
    })?;

    Ok(Json(json!({
        "success": true,
        "id": id,
        "secret": secret,
        "warning": "Store this secret securely for signature verification."
    })))
}

/// GET /v1/admin/webhooks
pub async fn list_webhooks(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_admin(&headers)?;

    let rows: Vec<(Uuid, String, String, Vec<String>, bool, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT id, project_id, url, events, enabled, created_at FROM webhooks ORDER BY created_at DESC"
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("DB error: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Internal server error"})))
    })?;

    let items: Vec<Value> = rows
        .iter()
        .map(|(id, project_id, url, events, enabled, created_at)| {
            json!({
                "id": id,
                "project_id": project_id,
                "url": url,
                "events": events,
                "enabled": enabled,
                "created_at": created_at,
            })
        })
        .collect();

    Ok(Json(json!({"webhooks": items, "count": items.len()})))
}

/// DELETE /v1/admin/webhooks/:id
pub async fn delete_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_admin(&headers)?;

    let result = sqlx::query("DELETE FROM webhooks WHERE id=$1")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!("DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Internal server error"})),
            )
        })?;

    if result.rows_affected() == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Webhook not found"})),
        ));
    }

    Ok(Json(json!({"success": true})))
}
