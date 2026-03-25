use axum::{extract::State, Json, http::{StatusCode, HeaderMap}};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use chrono::{Utc, DateTime};
use uuid::Uuid;
use crate::AppState;

// ─── Auth helper ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Project {
    pub id: String,
}

pub fn extract_project(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Project, (StatusCode, Json<Value>)> {
    let api_key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            headers.get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
        })
        .unwrap_or("")
        .trim();

    if api_key.is_empty() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Missing API key (X-Api-Key header)"})),
        ));
    }

    // Check TOML projects (fast path)
    for (id, proj) in &state.config.projects {
        if proj.api_key == api_key {
            return Ok(Project { id: id.clone() });
        }
    }

    Err((StatusCode::UNAUTHORIZED, Json(json!({"error": "Invalid API key"}))))
}

// ─── Request types ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SendRequest {
    pub channel: Option<String>,
    pub channels: Option<Vec<String>>,
    pub to: Option<String>,
    pub subscriber_id: Option<String>,
    pub template: Option<String>,
    pub subject: Option<String>,
    pub body: Option<String>,
    pub body_html: Option<String>,
    pub vars: Option<Value>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub idempotency_key: Option<String>,
    pub icon: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BatchRequest {
    pub channel: Option<String>,
    pub channels: Option<Vec<String>>,
    pub subscribers: Vec<String>,
    pub template: Option<String>,
    pub subject: Option<String>,
    pub body: Option<String>,
    pub body_html: Option<String>,
    pub vars: Option<Value>,
    pub scheduled_at: Option<DateTime<Utc>>,
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

pub async fn send_notification(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<SendRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers)?;

    let channels: Vec<String> = req.channels.clone()
        .unwrap_or_else(|| {
            req.channel.as_deref().unwrap_or("in_app")
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        });

    let recipient = req.to.clone()
        .or_else(|| req.subscriber_id.clone())
        .ok_or_else(|| (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "Missing 'to' or 'subscriber_id'"})),
        ))?;

    let scheduled_at = req.scheduled_at.unwrap_or_else(Utc::now);

    let mut payload = json!({
        "subject": req.subject,
        "body": req.body,
        "body_html": req.body_html,
        "icon": req.icon.as_deref().unwrap_or("bell"),
        "url": req.url,
    });

    // Merge vars into payload
    if let Some(vars) = &req.vars {
        if let (Some(p), Some(v)) = (payload.as_object_mut(), vars.as_object()) {
            p.insert("vars".to_string(), vars.clone());
            for (k, val) in v {
                p.entry(k).or_insert(val.clone());
            }
        }
    }

    let mut job_ids: Vec<Uuid> = Vec::new();

    for channel in &channels {
        let idem_key = req.idempotency_key.as_ref()
            .map(|k| format!("{}-{}", k, channel));

        let job_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO jobs (project_id, channel, subscriber_id, recipient, template_id, payload, scheduled_at, idempotency_key)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (project_id, idempotency_key) DO UPDATE SET status=EXCLUDED.status
            RETURNING id
            "#,
        )
        .bind(&project.id)
        .bind(channel)
        .bind(req.subscriber_id.as_deref())
        .bind(&recipient)
        .bind(req.template.as_deref())
        .bind(&payload)
        .bind(scheduled_at)
        .bind(idem_key.as_deref())
        .fetch_one(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

        job_ids.push(job_id);
    }

    Ok(Json(json!({
        "success": true,
        "job_ids": job_ids,
        "scheduled_at": scheduled_at,
        "channels": channels,
    })))
}

pub async fn batch_notification(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<BatchRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers)?;

    let channels: Vec<String> = req.channels.clone()
        .unwrap_or_else(|| {
            req.channel.as_deref().unwrap_or("in_app")
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        });

    let scheduled_at = req.scheduled_at.unwrap_or_else(Utc::now);

    let payload = json!({
        "subject": req.subject,
        "body": req.body,
        "body_html": req.body_html,
        "vars": req.vars,
    });

    let mut total = 0usize;
    for subscriber_id in &req.subscribers {
        for channel in &channels {
            sqlx::query(
                "INSERT INTO jobs (project_id, channel, subscriber_id, recipient, template_id, payload, scheduled_at) VALUES ($1, $2, $3, $4, $5, $6, $7)"
            )
            .bind(&project.id)
            .bind(channel)
            .bind(subscriber_id)
            .bind(subscriber_id)
            .bind(req.template.as_deref())
            .bind(&payload)
            .bind(scheduled_at)
            .execute(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
            total += 1;
        }
    }

    Ok(Json(json!({
        "success": true,
        "jobs_created": total,
        "subscribers": req.subscribers.len(),
        "channels": channels,
    })))
}
