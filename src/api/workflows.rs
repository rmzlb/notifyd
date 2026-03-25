use axum::{extract::{State, Path, Query}, Json, http::{StatusCode, HeaderMap}};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;
use crate::{AppState, db::{Workflow, WorkflowRun}, api::send::extract_project, workflow_engine};

#[derive(Deserialize)]
pub struct CreateWorkflow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub trigger_event: String,
    pub steps: Value,
    pub enabled: Option<bool>,
}

/// POST /v1/workflows
pub async fn create_workflow(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateWorkflow>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    sqlx::query(
        r#"
        INSERT INTO workflows (id, project_id, name, description, trigger_event, steps, enabled)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (project_id, id) DO UPDATE SET
            name = EXCLUDED.name,
            description = EXCLUDED.description,
            trigger_event = EXCLUDED.trigger_event,
            steps = EXCLUDED.steps,
            enabled = EXCLUDED.enabled,
            updated_at = now()
        "#
    )
    .bind(&req.id)
    .bind(&project.id)
    .bind(&req.name)
    .bind(req.description.as_deref())
    .bind(&req.trigger_event)
    .bind(&req.steps)
    .bind(req.enabled.unwrap_or(true))
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    Ok(Json(json!({"success": true, "id": req.id, "project_id": project.id})))
}

/// GET /v1/workflows
pub async fn list_workflows(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let workflows: Vec<Workflow> = sqlx::query_as(
        "SELECT id, project_id, name, description, trigger_event, steps, enabled, created_at, updated_at FROM workflows WHERE project_id=$1 ORDER BY created_at DESC"
    )
    .bind(&project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    let items: Vec<Value> = workflows.iter().map(|w| json!({
        "id": w.id,
        "name": w.name,
        "description": w.description,
        "trigger_event": w.trigger_event,
        "steps": w.steps,
        "enabled": w.enabled,
        "created_at": w.created_at,
    })).collect();

    Ok(Json(json!({"workflows": items, "count": items.len()})))
}

/// GET /v1/workflows/:id
pub async fn get_workflow(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let wf: Option<Workflow> = sqlx::query_as(
        "SELECT id, project_id, name, description, trigger_event, steps, enabled, created_at, updated_at FROM workflows WHERE project_id=$1 AND id=$2"
    )
    .bind(&project.id)
    .bind(&id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    let wf = wf.ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "Workflow not found"}))))?;

    Ok(Json(json!({
        "id": wf.id,
        "name": wf.name,
        "description": wf.description,
        "trigger_event": wf.trigger_event,
        "steps": wf.steps,
        "enabled": wf.enabled,
        "created_at": wf.created_at,
    })))
}

/// DELETE /v1/workflows/:id
pub async fn delete_workflow(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let result = sqlx::query("DELETE FROM workflows WHERE project_id=$1 AND id=$2")
        .bind(&project.id).bind(&id)
        .execute(&state.pool).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, Json(json!({"error": "Workflow not found"}))));
    }

    Ok(Json(json!({"success": true})))
}

/// POST /v1/workflows/trigger
#[derive(Deserialize)]
pub struct TriggerRequest {
    pub event: String,
    pub subscriber_id: String,
    pub payload: Option<Value>,
}

pub async fn trigger_workflow(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<TriggerRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let payload = req.payload.unwrap_or(json!({}));
    let run_ids = workflow_engine::trigger_event(&state, &project.id, &req.event, &req.subscriber_id, &payload)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    Ok(Json(json!({
        "success": true,
        "workflow_runs": run_ids,
        "event": req.event,
    })))
}

/// GET /v1/workflows/runs?status=running
#[derive(Deserialize)]
pub struct RunsQuery {
    pub status: Option<String>,
    pub workflow_id: Option<String>,
    pub limit: Option<i64>,
}

pub async fn list_runs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<RunsQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;
    let limit = q.limit.unwrap_or(50).min(200);

    let runs: Vec<WorkflowRun> = sqlx::query_as(
        "SELECT id, project_id, workflow_id, subscriber_id, trigger_payload, current_step, status, step_state, resume_at, created_at, updated_at FROM workflow_runs WHERE project_id=$1 ORDER BY created_at DESC LIMIT $2"
    )
    .bind(&project.id)
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    let items: Vec<Value> = runs.iter()
        .filter(|r| {
            q.status.as_ref().map_or(true, |s| r.status == *s)
                && q.workflow_id.as_ref().map_or(true, |w| r.workflow_id == *w)
        })
        .map(|r| json!({
            "id": r.id,
            "workflow_id": r.workflow_id,
            "subscriber_id": r.subscriber_id,
            "current_step": r.current_step,
            "status": r.status,
            "resume_at": r.resume_at,
            "created_at": r.created_at,
        }))
        .collect();

    Ok(Json(json!({"runs": items, "count": items.len()})))
}

/// DELETE /v1/workflows/runs/:id — cancel a run
pub async fn cancel_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let result = sqlx::query(
        "UPDATE workflow_runs SET status='cancelled', updated_at=now() WHERE id=$1 AND project_id=$2 AND status IN ('running', 'paused')"
    )
    .bind(id).bind(&project.id)
    .execute(&state.pool).await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, Json(json!({"error": "Run not found or not active"}))));
    }

    Ok(Json(json!({"success": true, "id": id})))
}
