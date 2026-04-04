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
    pub channel: String,
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
        .map(|p| {
            json!({
                "channel": p.channel,
                "workflow_id": p.workflow_id,
                "enabled": p.enabled,
            })
        })
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

    for pref in &req.preferences {
        let workflow_id = pref.workflow_id.as_deref().unwrap_or("*");
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
