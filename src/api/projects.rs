use axum::{extract::State, Json, http::{StatusCode, HeaderMap}};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use crate::AppState;

/// Admin auth: requires ADMIN_API_KEY env var
fn require_admin(headers: &HeaderMap) -> Result<(), (StatusCode, Json<Value>)> {
    let admin_key = std::env::var("ADMIN_API_KEY").unwrap_or_default();
    if admin_key.is_empty() {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "ADMIN_API_KEY not configured"}))));
    }

    let provided = headers.get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if provided != admin_key {
        return Err((StatusCode::UNAUTHORIZED, Json(json!({"error": "Admin access required"}))));
    }

    Ok(())
}

#[derive(Deserialize)]
pub struct CreateProject {
    pub id: String,
    pub name: String,
    pub channels: Option<Vec<String>>,
    pub rate_limit_per_min: Option<i32>,
    pub settings: Option<Value>,
}

/// POST /v1/admin/projects
pub async fn create_project(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateProject>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_admin(&headers)?;

    // Generate API key
    let api_key = format!("sk_{}_{}",
        req.id,
        hex::encode(&uuid::Uuid::new_v4().as_bytes()[..16])
    );
    let api_key_hash = hash_key(&api_key);

    let channels: Vec<String> = req.channels.unwrap_or_else(|| vec!["email".into(), "in_app".into()]);
    let channels_arr: Vec<&str> = channels.iter().map(|s| s.as_str()).collect();

    sqlx::query(
        r#"
        INSERT INTO projects (id, api_key, api_key_hash, name, channels, rate_limit_per_min, settings)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (id) DO UPDATE SET
            name = EXCLUDED.name,
            channels = EXCLUDED.channels,
            rate_limit_per_min = EXCLUDED.rate_limit_per_min,
            settings = EXCLUDED.settings,
            updated_at = now()
        "#
    )
    .bind(&req.id)
    .bind(&api_key)
    .bind(&api_key_hash)
    .bind(&req.name)
    .bind(&channels_arr)
    .bind(req.rate_limit_per_min.unwrap_or(600))
    .bind(req.settings.as_ref().unwrap_or(&json!({})))
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    Ok(Json(json!({
        "success": true,
        "project": {
            "id": req.id,
            "api_key": api_key,
        },
        "warning": "Store this API key securely. It won't be shown again."
    })))
}

/// GET /v1/admin/projects
pub async fn list_projects(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_admin(&headers)?;

    let rows: Vec<(String, String, Option<Vec<String>>, Option<i32>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT id, name, channels, rate_limit_per_min, created_at FROM projects ORDER BY created_at"
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    let projects: Vec<Value> = rows.iter().map(|(id, name, channels, rate_limit, created_at)| json!({
        "id": id,
        "name": name,
        "channels": channels,
        "rate_limit_per_min": rate_limit,
        "created_at": created_at,
    })).collect();

    Ok(Json(json!({"projects": projects})))
}

/// POST /v1/admin/projects/:id/rotate-key
pub async fn rotate_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_admin(&headers)?;

    // Generate new key, move current to secondary
    let new_key = format!("sk_{}_{}", id, hex::encode(&uuid::Uuid::new_v4().as_bytes()[..16]));
    let new_hash = hash_key(&new_key);

    // Move current key to secondary (grace period for migration)
    sqlx::query(
        "UPDATE projects SET secondary_api_key = api_key, secondary_api_key_hash = api_key_hash, api_key = $2, api_key_hash = $3, updated_at = now() WHERE id = $1"
    )
    .bind(&id)
    .bind(&new_key)
    .bind(&new_hash)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    Ok(Json(json!({
        "success": true,
        "new_api_key": new_key,
        "note": "Old key is still valid as secondary. Call /revoke-secondary to fully disable it.",
        "warning": "Store this API key securely."
    })))
}

/// POST /v1/admin/projects/:id/revoke-secondary
pub async fn revoke_secondary(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_admin(&headers)?;

    sqlx::query("UPDATE projects SET secondary_api_key = NULL, secondary_api_key_hash = NULL, updated_at = now() WHERE id = $1")
        .bind(&id).execute(&state.pool).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    Ok(Json(json!({"success": true, "message": "Secondary key revoked"})))
}

/// DELETE /v1/admin/projects/:id
pub async fn delete_project(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_admin(&headers)?;

    let result = sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(&id).execute(&state.pool).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, Json(json!({"error": "Project not found"}))));
    }

    Ok(Json(json!({"success": true})))
}

/// GET /v1/admin/audit?project_id=xxx&limit=100
#[derive(Deserialize)]
pub struct AuditQuery {
    pub project_id: Option<String>,
    pub action: Option<String>,
    pub limit: Option<i64>,
}

pub async fn audit_log(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<AuditQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_admin(&headers)?;

    let limit = q.limit.unwrap_or(100).min(500);

    let rows: Vec<(uuid::Uuid, String, String, String, Option<String>, Option<Value>, Option<String>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT id, project_id, actor, action, resource, metadata, ip, created_at FROM audit_log ORDER BY created_at DESC LIMIT $1"
    )
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    let entries: Vec<Value> = rows.iter().map(|(id, project_id, actor, action, resource, metadata, ip, created_at)| json!({
        "id": id,
        "project_id": project_id,
        "actor": actor,
        "action": action,
        "resource": resource,
        "metadata": metadata,
        "ip": ip,
        "created_at": created_at,
    })).collect();

    Ok(Json(json!({"entries": entries, "count": entries.len()})))
}

/// Hash an API key for storage (SHA-256, simple but sufficient for API keys)
fn hash_key(key: &str) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}
