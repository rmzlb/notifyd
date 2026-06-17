use crate::{api::send::extract_project, db::PushToken, AppState};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct RegisterToken {
    pub subscriber_id: String,
    pub token: Option<String>,
    pub platform: Option<String>,
    pub device_name: Option<String>,
    pub endpoint: Option<String>,
    pub keys: Option<WebPushKeys>,
    #[serde(alias = "expirationTime")]
    pub expiration_time: Option<DateTime<Utc>>,
    pub user_agent: Option<String>,
}

#[derive(Deserialize)]
pub struct WebPushKeys {
    pub p256dh: String,
    pub auth: String,
}

#[derive(Deserialize)]
pub struct VapidPublicKeyQuery {
    pub project: Option<String>,
}

/// POST /v1/push-tokens
pub async fn register_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<RegisterToken>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;
    let token = req
        .token
        .clone()
        .or_else(|| req.endpoint.clone())
        .ok_or_else(|| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": "Missing push token or Web Push endpoint"})),
            )
        })?;
    let platform =
        req.platform.as_deref().unwrap_or_else(
            || {
                if req.endpoint.is_some() {
                    "web"
                } else {
                    "fcm"
                }
            },
        );

    sqlx::query(
        r#"
        INSERT INTO push_tokens (
            project_id, subscriber_id, token, platform, device_name,
            endpoint, p256dh, auth, expiration_time, user_agent
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        ON CONFLICT (project_id, subscriber_id, token) DO UPDATE SET
            platform = EXCLUDED.platform,
            device_name = EXCLUDED.device_name,
            endpoint = EXCLUDED.endpoint,
            p256dh = EXCLUDED.p256dh,
            auth = EXCLUDED.auth,
            expiration_time = EXCLUDED.expiration_time,
            user_agent = EXCLUDED.user_agent,
            last_used_at = now()
        "#,
    )
    .bind(&project.id)
    .bind(&req.subscriber_id)
    .bind(&token)
    .bind(platform)
    .bind(req.device_name.as_deref())
    .bind(req.endpoint.as_deref())
    .bind(req.keys.as_ref().map(|k| k.p256dh.as_str()))
    .bind(req.keys.as_ref().map(|k| k.auth.as_str()))
    .bind(req.expiration_time)
    .bind(req.user_agent.as_deref())
    .execute(&state.pool)
    .await
    .map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, {
            tracing::error!("DB error: {}", e);
            Json(json!({"error": "Internal server error"}))
        })
    })?;

    Ok(Json(json!({"success": true})))
}

/// GET /v1/push-tokens/subscriber/:subscriber_id
pub async fn list_tokens(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(subscriber_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let tokens: Vec<PushToken> = sqlx::query_as(
        r#"
        SELECT id, project_id, subscriber_id, token, platform, device_name,
               endpoint, p256dh, auth, expiration_time, user_agent
        FROM push_tokens
        WHERE project_id=$1 AND subscriber_id=$2
        ORDER BY last_used_at DESC NULLS LAST, created_at DESC
        "#,
    )
    .bind(&project.id)
    .bind(&subscriber_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, {
            tracing::error!("DB error: {}", e);
            Json(json!({"error": "Internal server error"}))
        })
    })?;

    let items: Vec<Value> = tokens
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "token": t.token,
                "platform": t.platform,
                "device_name": t.device_name,
                "endpoint": t.endpoint,
                "expiration_time": t.expiration_time,
                "user_agent": t.user_agent,
            })
        })
        .collect();

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
        .bind(id)
        .bind(&project.id)
        .execute(&state.pool)
        .await
        .map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, {
                tracing::error!("DB error: {}", e);
                Json(json!({"error": "Internal server error"}))
            })
        })?;

    Ok(Json(json!({"success": true})))
}

/// GET /v1/push/vapid-public-key
pub async fn vapid_public_key(
    State(state): State<Arc<AppState>>,
    Query(query): Query<VapidPublicKeyQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if let Some(project_id) = query.project.as_deref() {
        if !state.config.projects.contains_key(project_id) {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id=$1)")
                    .bind(project_id)
                    .fetch_one(&state.pool)
                    .await
                    .unwrap_or(false);

            if !exists {
                return Err((
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": "Project not found"})),
                ));
            }
        }
    }

    let config = state.config.connectors.push.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "Push connector not configured"})),
        )
    })?;

    let public_key =
        crate::connectors::push::PushConnector::vapid_public_key(config).map_err(|e| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    Ok(Json(json!({ "public_key": public_key })))
}
