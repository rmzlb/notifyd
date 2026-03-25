use axum::{extract::{State, Path}, Json, http::{StatusCode, HeaderMap}};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use crate::{AppState, db::Subscriber, api::send::extract_project};

#[derive(Debug, Deserialize)]
pub struct UpsertSubscriber {
    pub id: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub locale: Option<String>,
    pub data: Option<Value>,
}

pub async fn upsert_subscriber(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<UpsertSubscriber>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    sqlx::query(
        r#"
        INSERT INTO subscribers (id, project_id, email, phone, first_name, last_name, locale, data)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (project_id, id) DO UPDATE SET
            email = COALESCE(EXCLUDED.email, subscribers.email),
            phone = COALESCE(EXCLUDED.phone, subscribers.phone),
            first_name = COALESCE(EXCLUDED.first_name, subscribers.first_name),
            last_name = COALESCE(EXCLUDED.last_name, subscribers.last_name),
            locale = COALESCE(EXCLUDED.locale, subscribers.locale),
            data = subscribers.data || EXCLUDED.data,
            updated_at = now()
        "#,
    )
    .bind(&req.id)
    .bind(&project.id)
    .bind(req.email.as_deref())
    .bind(req.phone.as_deref())
    .bind(req.first_name.as_deref())
    .bind(req.last_name.as_deref())
    .bind(req.locale.as_deref().unwrap_or("fr"))
    .bind(req.data.as_ref().unwrap_or(&json!({})))
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    Ok(Json(json!({"success": true, "id": req.id, "project_id": project.id})))
}

pub async fn get_subscriber(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let sub: Option<Subscriber> = sqlx::query_as(
        "SELECT id, project_id, email, phone, first_name, last_name, locale, data, created_at, updated_at FROM subscribers WHERE project_id=$1 AND id=$2"
    )
    .bind(&project.id)
    .bind(&id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    let sub = sub.ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "Subscriber not found"}))))?;

    Ok(Json(json!({
        "id": sub.id,
        "project_id": project.id,
        "email": sub.email,
        "phone": sub.phone,
        "first_name": sub.first_name,
        "last_name": sub.last_name,
        "locale": sub.locale,
        "data": sub.data,
        "created_at": sub.created_at,
    })))
}

pub async fn delete_subscriber(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let result = sqlx::query("DELETE FROM subscribers WHERE project_id=$1 AND id=$2")
        .bind(&project.id)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, Json(json!({"error": "Subscriber not found"}))));
    }

    Ok(Json(json!({"success": true})))
}
