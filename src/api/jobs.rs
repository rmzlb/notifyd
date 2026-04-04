use crate::{api::send::extract_project, db::Job, AppState};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

pub async fn get_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let job: Option<Job> = sqlx::query_as(
        "SELECT id, project_id, channel, subscriber_id, recipient, template_id, payload, status, scheduled_at, attempts, max_attempts, next_retry_at, idempotency_key, created_at, sent_at, error FROM jobs WHERE id=$1 AND project_id=$2"
    )
    .bind(id)
    .bind(&project.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    let job = job.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Job not found"})),
        )
    })?;

    Ok(Json(json!({
        "id": job.id,
        "channel": job.channel,
        "subscriber_id": job.subscriber_id,
        "recipient": job.recipient,
        "template_id": job.template_id,
        "status": job.status,
        "scheduled_at": job.scheduled_at,
        "attempts": job.attempts,
        "max_attempts": job.max_attempts,
        "created_at": job.created_at,
        "sent_at": job.sent_at,
        "error": job.error,
    })))
}

pub async fn cancel_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let result = sqlx::query(
        "UPDATE jobs SET status='cancelled' WHERE id=$1 AND project_id=$2 AND status IN ('pending', 'retry')"
    )
    .bind(id)
    .bind(&project.id)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    if result.rows_affected() == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Job not found or not cancellable"})),
        ));
    }

    Ok(Json(json!({"success": true, "id": id})))
}
