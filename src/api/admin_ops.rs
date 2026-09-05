//! REST façade over `ops`: the digest and the operator actions an agent or a
//! script drives. Admin key for `/v1/admin/*`; project key for the
//! project-scoped variants (`/v1/jobs/:id/retry`, `/v1/suppressions`).

use crate::api::projects::require_admin;
use crate::api::send::extract_project;
use crate::{ops, AppState};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

type ApiError = (StatusCode, Json<Value>);

fn bad_request(message: impl std::fmt::Display) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": message.to_string() })),
    )
}

fn internal(error: anyhow::Error) -> ApiError {
    tracing::error!("ops error: {}", error);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": "Internal server error" })),
    )
}

#[derive(Debug, Deserialize)]
pub struct DigestQuery {
    pub window: Option<String>,
    /// `json` (default) or `markdown`
    pub format: Option<String>,
}

/// GET /v1/admin/digest?window=24h&format=markdown
pub async fn digest(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<DigestQuery>,
) -> Result<Response, ApiError> {
    require_admin(&headers)?;
    let window = ops::parse_window(q.window.as_deref()).map_err(bad_request)?;
    let digest = ops::digest(&state, window).await.map_err(internal)?;
    if q.format.as_deref() == Some("markdown") {
        return Ok((
            [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
            ops::render_markdown(&digest),
        )
            .into_response());
    }
    Ok(Json(json!(digest)).into_response())
}

/// GET /v1/admin/jobs?project_id=&status=&channel=&recipient=&since=&limit=
pub async fn list_jobs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(filter): Query<ops::JobFilter>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers)?;
    let jobs = ops::list_jobs(&state, &filter).await.map_err(internal)?;
    Ok(Json(json!({ "jobs": jobs, "count": jobs.len() })))
}

/// POST /v1/admin/jobs/:id/retry
pub async fn admin_retry_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers)?;
    let job = ops::retry_job(&state, id, None).await.map_err(|e| {
        (
            StatusCode::CONFLICT,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    Ok(Json(json!({ "success": true, "job": job })))
}

/// POST /v1/admin/jobs/:id/cancel
pub async fn admin_cancel_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers)?;
    let job = ops::cancel_job(&state, id, None).await.map_err(|e| {
        (
            StatusCode::CONFLICT,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    Ok(Json(json!({ "success": true, "job": job })))
}

/// POST /v1/jobs/:id/retry — project key, own jobs only.
pub async fn project_retry_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let project = extract_project(&state, &headers).await?;
    let job = ops::retry_job(&state, id, Some(&project.id))
        .await
        .map_err(|e| {
            (
                StatusCode::CONFLICT,
                Json(json!({ "error": e.to_string() })),
            )
        })?;
    Ok(Json(json!({ "success": true, "job": job })))
}

/// PATCH /v1/admin/projects/:id
pub async fn patch_project(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(patch): Json<ops::ProjectPatch>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers)?;
    let project = ops::update_project(&state, &id, &patch)
        .await
        .map_err(|e| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": e.to_string() })),
            )
        })?;
    Ok(Json(json!({ "success": true, "project": project })))
}

#[derive(Debug, Deserialize)]
pub struct SuppressionListQuery {
    pub project_id: Option<String>,
    pub limit: Option<i64>,
}

/// GET /v1/admin/suppressions?project_id=&limit=
pub async fn admin_list_suppressions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<SuppressionListQuery>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers)?;
    let items = ops::list_suppressions(&state, q.project_id.as_deref(), q.limit.unwrap_or(100))
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "suppressions": items, "count": items.len() })))
}

#[derive(Debug, Deserialize)]
pub struct AdminSuppressionBody {
    pub project_id: String,
    pub email: String,
    pub detail: Option<String>,
    /// `all` (default) or `marketing`.
    pub scope: Option<String>,
}

/// POST /v1/admin/suppressions
pub async fn admin_add_suppression(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<AdminSuppressionBody>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers)?;
    let scope = ops::SuppressionScope::parse(body.scope.as_deref()).map_err(bad_request)?;
    let item = ops::add_suppression(
        &state,
        &body.project_id,
        &body.email,
        body.detail.as_deref(),
        "admin",
        scope,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    Ok(Json(json!({ "success": true, "suppression": item })))
}

/// DELETE /v1/admin/suppressions/:id
pub async fn admin_release_suppression(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&headers)?;
    let result = ops::release_suppression(&state, id, None, "admin")
        .await
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": e.to_string() })),
            )
        })?;
    Ok(Json(result))
}

#[derive(Debug, Deserialize)]
pub struct ProjectSuppressionBody {
    pub email: String,
    pub detail: Option<String>,
    /// `all` (default) or `marketing`.
    pub scope: Option<String>,
}

/// POST /v1/suppressions — project key.
pub async fn project_add_suppression(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ProjectSuppressionBody>,
) -> Result<Json<Value>, ApiError> {
    let project = extract_project(&state, &headers).await?;
    let scope = ops::SuppressionScope::parse(body.scope.as_deref()).map_err(bad_request)?;
    let item = ops::add_suppression(
        &state,
        &project.id,
        &body.email,
        body.detail.as_deref(),
        &format!("project:{}", project.id),
        scope,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    Ok(Json(json!({ "success": true, "suppression": item })))
}
