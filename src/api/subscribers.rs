use crate::{api::send::extract_project, db::Subscriber, AppState};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct UpsertSubscriber {
    pub id: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub locale: Option<String>,
    pub data: Option<Value>,
    /// IANA timezone (e.g. Europe/Paris) used by send windows.
    pub timezone: Option<String>,
}

pub async fn upsert_subscriber(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<UpsertSubscriber>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    sqlx::query(
        r#"
        INSERT INTO subscribers (id, project_id, email, phone, first_name, last_name, locale, data, timezone)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (project_id, id) DO UPDATE SET
            email = COALESCE(EXCLUDED.email, subscribers.email),
            phone = COALESCE(EXCLUDED.phone, subscribers.phone),
            first_name = COALESCE(EXCLUDED.first_name, subscribers.first_name),
            last_name = COALESCE(EXCLUDED.last_name, subscribers.last_name),
            locale = COALESCE(EXCLUDED.locale, subscribers.locale),
            data = subscribers.data || EXCLUDED.data,
            timezone = COALESCE(EXCLUDED.timezone, subscribers.timezone),
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
    .bind(req.timezone.as_deref().map(str::trim).filter(|t| !t.is_empty()))
    .execute(&state.pool)
    .await
    .map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, {
            tracing::error!("DB error: {}", e);
            Json(json!({"error": "Internal server error"}))
        })
    })?;

    Ok(Json(
        json!({"success": true, "id": req.id, "project_id": project.id}),
    ))
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub q: Option<String>,
}

/// GET /v1/subscribers?limit=50&offset=0&q=search
pub async fn list_subscribers(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;
    let limit = query.limit.unwrap_or(50).min(200);
    let offset = query.offset.unwrap_or(0).max(0);
    let search = query.q.unwrap_or_default();

    let total: i64 = if search.is_empty() {
        sqlx::query_scalar("SELECT COUNT(*) FROM subscribers WHERE project_id=$1")
            .bind(&project.id)
            .fetch_one(&state.pool).await
    } else {
        let pattern = format!("%{}%", search);
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM subscribers WHERE project_id=$1 AND (id ILIKE $2 OR email ILIKE $2 OR first_name ILIKE $2 OR last_name ILIKE $2)"
        )
        .bind(&project.id).bind(&pattern)
        .fetch_one(&state.pool).await
    }.map_err(|e| {
        tracing::error!("DB error: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Internal server error"})))
    })?;

    let subscribers: Vec<Subscriber> = if search.is_empty() {
        sqlx::query_as(
            "SELECT id, project_id, email, phone, first_name, last_name, locale, data, created_at, updated_at FROM subscribers WHERE project_id=$1 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
        )
        .bind(&project.id).bind(limit).bind(offset)
        .fetch_all(&state.pool).await
    } else {
        let pattern = format!("%{}%", search);
        sqlx::query_as(
            "SELECT id, project_id, email, phone, first_name, last_name, locale, data, created_at, updated_at FROM subscribers WHERE project_id=$1 AND (id ILIKE $2 OR email ILIKE $2 OR first_name ILIKE $2 OR last_name ILIKE $2) ORDER BY created_at DESC LIMIT $3 OFFSET $4"
        )
        .bind(&project.id).bind(&pattern).bind(limit).bind(offset)
        .fetch_all(&state.pool).await
    }.map_err(|e| {
        tracing::error!("DB error: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Internal server error"})))
    })?;

    let items: Vec<Value> = subscribers
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "email": s.email,
                "phone": s.phone,
                "first_name": s.first_name,
                "last_name": s.last_name,
                "locale": s.locale,
                "data": s.data,
                "created_at": s.created_at,
            })
        })
        .collect();

    Ok(Json(json!({
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
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

    let sub = sub.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Subscriber not found"})),
        )
    })?;

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
        .map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, {
                tracing::error!("DB error: {}", e);
                Json(json!({"error": "Internal server error"}))
            })
        })?;

    if result.rows_affected() == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Subscriber not found"})),
        ));
    }

    Ok(Json(json!({"success": true})))
}
