use axum::{extract::{State, Path}, Json, http::{StatusCode, HeaderMap}};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;
use crate::{AppState, db::PushToken, api::send::extract_project};

#[derive(Deserialize)]
pub struct RegisterToken {
    pub subscriber_id: String,
    pub token: String,
    pub platform: Option<String>,
    pub device_name: Option<String>,
}

/// POST /v1/push-tokens
pub async fn register_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<RegisterToken>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    sqlx::query(
        r#"
        INSERT INTO push_tokens (project_id, subscriber_id, token, platform, device_name)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (project_id, subscriber_id, token) DO UPDATE SET
            platform = EXCLUDED.platform,
            device_name = EXCLUDED.device_name,
            last_used_at = now()
        "#
    )
    .bind(&project.id)
    .bind(&req.subscriber_id)
    .bind(&req.token)
    .bind(req.platform.as_deref().unwrap_or("fcm"))
    .bind(req.device_name.as_deref())
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(json!({"success": true})))
}

/// GET /v1/push-tokens/:subscriber_id
pub async fn list_tokens(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(subscriber_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let tokens: Vec<PushToken> = sqlx::query_as(
        "SELECT id, project_id, subscriber_id, token, platform, device_name FROM push_tokens WHERE project_id=$1 AND subscriber_id=$2"
    )
    .bind(&project.id)
    .bind(&subscriber_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    let items: Vec<Value> = tokens.iter().map(|t| json!({
        "id": t.id,
        "token": t.token,
        "platform": t.platform,
        "device_name": t.device_name,
    })).collect();

    Ok(Json(json!({"tokens": items})))
}

/// DELETE /v1/push-tokens/:id
pub async fn delete_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    sqlx::query("DELETE FROM push_tokens WHERE id=$1 AND project_id=$2")
        .bind(id).bind(&project.id)
        .execute(&state.pool).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(json!({"success": true})))
}
