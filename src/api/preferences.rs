use crate::{api::send::extract_project, db::SubscriberPreference, AppState};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Deserialize)]
pub struct SetPreference {
    /// "email", "sms", "push", "in_app", "whatsapp" or "*" for every channel.
    pub channel: String,
    /// Scope: a topic id (`topic`), a workflow id (`workflow_id`), or nothing
    /// for the whole channel. `topic` wins when both are given.
    pub topic: Option<String>,
    pub workflow_id: Option<String>,
    pub enabled: bool,
}

#[derive(Deserialize)]
pub struct BulkPreferences {
    pub preferences: Vec<SetPreference>,
}

/// GET /v1/subscribers/:id/preferences
pub async fn get_preferences(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(subscriber_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let prefs: Vec<SubscriberPreference> = sqlx::query_as(
        "SELECT project_id, subscriber_id, channel, workflow_id, enabled FROM subscriber_preferences WHERE project_id=$1 AND subscriber_id=$2 ORDER BY channel, workflow_id"
    )
    .bind(&project.id)
    .bind(&subscriber_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    let items: Vec<Value> = prefs
        .iter()
        .map(|p| crate::topics::row_json(&p.channel, &p.workflow_id, p.enabled))
        .collect();

    Ok(Json(json!({"preferences": items})))
}

/// PUT /v1/subscribers/:id/preferences
pub async fn set_preferences(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(subscriber_id): Path<String>,
    Json(req): Json<BulkPreferences>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let mut scopes = Vec::with_capacity(req.preferences.len());
    for pref in &req.preferences {
        let scope = crate::topics::scope_from(pref.topic.as_deref(), pref.workflow_id.as_deref())
            .map_err(|error| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": error })),
            )
        })?;
        scopes.push(scope);
    }

    for (pref, workflow_id) in req.preferences.iter().zip(scopes.iter()) {
        sqlx::query(
            r#"
            INSERT INTO subscriber_preferences (project_id, subscriber_id, channel, workflow_id, enabled)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (project_id, subscriber_id, channel, workflow_id)
            DO UPDATE SET enabled = EXCLUDED.enabled, updated_at = now()
            "#
        )
        .bind(&project.id)
        .bind(&subscriber_id)
        .bind(&pref.channel)
        .bind(workflow_id)
        .bind(pref.enabled)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;
    }

    Ok(Json(
        json!({"success": true, "updated": req.preferences.len()}),
    ))
}
