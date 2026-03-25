use axum::{extract::{State, Path, Query}, Json, http::{StatusCode, HeaderMap}};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use crate::{AppState, db::Template, api::send::extract_project};

#[derive(Deserialize)]
pub struct UpsertTemplate {
    pub id: String,
    pub channel: String,
    pub subject: Option<String>,
    pub body: String,
    pub body_html: Option<String>,
}

/// POST /v1/templates — create or upsert a template
pub async fn upsert_template(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<UpsertTemplate>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    sqlx::query(
        r#"
        INSERT INTO templates (id, project_id, channel, subject, body, body_html)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (project_id, id, channel) DO UPDATE SET
            subject = EXCLUDED.subject,
            body = EXCLUDED.body,
            body_html = EXCLUDED.body_html,
            updated_at = now()
        "#,
    )
    .bind(&req.id)
    .bind(&project.id)
    .bind(&req.channel)
    .bind(req.subject.as_deref())
    .bind(&req.body)
    .bind(req.body_html.as_deref())
    .execute(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("DB error: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Internal server error"})))
    })?;

    Ok(Json(json!({"success": true, "id": req.id, "channel": req.channel})))
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[allow(dead_code)]
    pub channel: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// GET /v1/templates — list templates for a project
pub async fn list_templates(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0).max(0);

    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM templates WHERE project_id=$1"
    )
    .bind(&project.id)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("DB error: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Internal server error"})))
    })?;

    let templates: Vec<Template> = sqlx::query_as(
        "SELECT id, project_id, channel, subject, body, body_html FROM templates WHERE project_id=$1 ORDER BY id, channel LIMIT $2 OFFSET $3"
    )
    .bind(&project.id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("DB error: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Internal server error"})))
    })?;

    let items: Vec<Value> = templates.iter().map(|t| json!({
        "id": t.id,
        "channel": t.channel,
        "subject": t.subject,
        "body": t.body,
        "body_html": t.body_html,
    })).collect();

    Ok(Json(json!({
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
}

/// GET /v1/templates/:id — get one template (returns all channel variants)
pub async fn get_template(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let templates: Vec<Template> = sqlx::query_as(
        "SELECT id, project_id, channel, subject, body, body_html FROM templates WHERE project_id=$1 AND id=$2"
    )
    .bind(&project.id)
    .bind(&id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("DB error: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Internal server error"})))
    })?;

    if templates.is_empty() {
        return Err((StatusCode::NOT_FOUND, Json(json!({"error": "Template not found"}))));
    }

    let items: Vec<Value> = templates.iter().map(|t| json!({
        "id": t.id,
        "channel": t.channel,
        "subject": t.subject,
        "body": t.body,
        "body_html": t.body_html,
    })).collect();

    Ok(Json(json!({"template_id": id, "variants": items})))
}

/// DELETE /v1/templates/:id — delete all channel variants of a template
pub async fn delete_template(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let result = sqlx::query("DELETE FROM templates WHERE project_id=$1 AND id=$2")
        .bind(&project.id)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!("DB error: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Internal server error"})))
        })?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, Json(json!({"error": "Template not found"}))));
    }

    Ok(Json(json!({"success": true, "deleted": result.rows_affected()})))
}
