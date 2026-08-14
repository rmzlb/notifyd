use crate::{api::send::extract_project, db::Job, AppState};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde_json::{json, Value};
use sqlx::Row;
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

    let delivery: (
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as("SELECT delivered_at, bounced_at FROM jobs WHERE id=$1 AND project_id=$2")
        .bind(id)
        .bind(&project.id)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!("DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Internal server error"})),
            )
        })?;

    let event_rows = sqlx::query(
        "SELECT provider, event_type, payload, received_at
         FROM provider_events
         WHERE job_id=$1
         ORDER BY received_at ASC, id ASC",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("DB error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Internal server error"})),
        )
    })?;

    let provider_events: Vec<Value> = event_rows
        .into_iter()
        .map(|row| {
            let payload: Value = row.get("payload");
            json!({
                "provider": row.get::<String, _>("provider"),
                "type": row.get::<String, _>("event_type"),
                "occurred_at": payload.get("created_at").cloned().unwrap_or_else(|| {
                    json!(row.get::<chrono::DateTime<chrono::Utc>, _>("received_at"))
                }),
                "provider_message_id": payload.pointer("/data/email_id").cloned(),
                "recipients": payload.pointer("/data/to").cloned().unwrap_or_else(|| json!([])),
                "error": payload.pointer("/data/bounce/message").cloned()
                    .or_else(|| payload.pointer("/data/error").cloned()),
            })
        })
        .collect();

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
        "delivered_at": delivery.0,
        "bounced_at": delivery.1,
        "provider_events": provider_events,
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
