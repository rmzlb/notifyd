use axum::{
    extract::{State, Path, Query},
    Json,
    http::{StatusCode, HeaderMap},
    response::Sse,
};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{sync::Arc, convert::Infallible, time::Duration};
use tokio_stream::wrappers::BroadcastStream;
use crate::{AppState, db::InboxMessage, api::{send::extract_project, auth::validate_subscriber_token}};
use axum::response::sse::{Event, KeepAlive};
use uuid::Uuid;

fn auth_inbox(
    state: &AppState,
    headers: &HeaderMap,
    path_subscriber_id: &str,
) -> Result<(String, String), (StatusCode, Json<Value>)> {
    // Subscriber JWT (frontend)
    let bearer = headers.get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    if let Some(token) = bearer {
        if let Some(claims) = validate_subscriber_token(state, token) {
            if claims.sub == path_subscriber_id {
                return Ok((claims.project, claims.sub));
            }
            return Err((StatusCode::FORBIDDEN, Json(json!({"error": "Token subscriber mismatch"}))));
        }
    }

    // API key fallback
    let project = extract_project(state, headers)?;
    Ok((project.id, path_subscriber_id.to_string()))
}

#[derive(Deserialize)]
pub struct InboxQuery {
    pub limit: Option<i64>,
    pub filter: Option<String>,
    pub q: Option<String>,
    pub token: Option<String>, // SSE auth via query param
}

pub async fn list_notifications(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(subscriber_id): Path<String>,
    Query(query): Query<InboxQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (project_id, sub_id) = auth_inbox(&state, &headers, &subscriber_id)?;

    let limit = query.limit.unwrap_or(20).min(100);

    let rows: Vec<InboxMessage> = sqlx::query_as(
        r#"
        SELECT id, project_id, subscriber_id, body, icon, url, data, read_at, archived_at, is_todo, created_at
        FROM inbox_messages
        WHERE project_id=$1 AND subscriber_id=$2 AND archived_at IS NULL
        ORDER BY created_at DESC
        LIMIT $3
        "#,
    )
    .bind(&project_id)
    .bind(&sub_id)
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    // Apply in-memory filter (simple, avoids dynamic SQL)
    let search = query.q.as_deref().unwrap_or("").to_lowercase();
    let filter = query.filter.as_deref().unwrap_or("all");

    let notifications: Vec<Value> = rows.iter()
        .filter(|r| {
            if !search.is_empty() && !r.body.to_lowercase().contains(&search) {
                return false;
            }
            match filter {
                "unread" => r.read_at.is_none(),
                "todo" => r.is_todo,
                _ => true,
            }
        })
        .map(|r| json!({
            "id": r.id,
            "body": r.body,
            "icon": r.icon,
            "url": r.url,
            "data": r.data,
            "is_read": r.read_at.is_some(),
            "read_at": r.read_at,
            "is_todo": r.is_todo,
            "created_at": r.created_at,
        }))
        .collect();

    Ok(Json(json!({
        "notifications": notifications,
        "count": notifications.len(),
        "has_more": rows.len() == limit as usize,
    })))
}

pub async fn unread_count(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(subscriber_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (project_id, sub_id) = auth_inbox(&state, &headers, &subscriber_id)?;

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inbox_messages WHERE project_id=$1 AND subscriber_id=$2 AND read_at IS NULL AND archived_at IS NULL"
    )
    .bind(&project_id)
    .bind(&sub_id)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(json!({"unread_count": count})))
}

#[derive(Deserialize)]
pub struct UpdateNotification {
    pub read: Option<bool>,
    pub archived: Option<bool>,
    pub is_todo: Option<bool>,
}

pub async fn update_notification(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((subscriber_id, msg_id)): Path<(String, Uuid)>,
    Json(req): Json<UpdateNotification>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (project_id, sub_id) = auth_inbox(&state, &headers, &subscriber_id)?;

    if let Some(read) = req.read {
        let sql = if read {
            "UPDATE inbox_messages SET read_at=now() WHERE id=$1 AND project_id=$2 AND subscriber_id=$3"
        } else {
            "UPDATE inbox_messages SET read_at=NULL WHERE id=$1 AND project_id=$2 AND subscriber_id=$3"
        };
        sqlx::query(sql).bind(msg_id).bind(&project_id).bind(&sub_id)
            .execute(&state.pool).await.ok();
    }

    if let Some(archived) = req.archived {
        let sql = if archived {
            "UPDATE inbox_messages SET archived_at=now() WHERE id=$1 AND project_id=$2 AND subscriber_id=$3"
        } else {
            "UPDATE inbox_messages SET archived_at=NULL WHERE id=$1 AND project_id=$2 AND subscriber_id=$3"
        };
        sqlx::query(sql).bind(msg_id).bind(&project_id).bind(&sub_id)
            .execute(&state.pool).await.ok();
    }

    if let Some(todo) = req.is_todo {
        sqlx::query("UPDATE inbox_messages SET is_todo=$4 WHERE id=$1 AND project_id=$2 AND subscriber_id=$3")
            .bind(msg_id).bind(&project_id).bind(&sub_id).bind(todo)
            .execute(&state.pool).await.ok();
    }

    // Broadcast count
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inbox_messages WHERE project_id=$1 AND subscriber_id=$2 AND read_at IS NULL AND archived_at IS NULL"
    )
    .bind(&project_id)
    .bind(&sub_id)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    let ev = json!({"type": "count_update", "unread_count": count});
    state.broadcaster.send(&project_id, &sub_id, ev.to_string()).await;

    Ok(Json(json!({"success": true})))
}

pub async fn read_all(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(subscriber_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (project_id, sub_id) = auth_inbox(&state, &headers, &subscriber_id)?;

    let result = sqlx::query(
        "UPDATE inbox_messages SET read_at=now() WHERE project_id=$1 AND subscriber_id=$2 AND read_at IS NULL AND archived_at IS NULL"
    )
    .bind(&project_id)
    .bind(&sub_id)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    let ev = json!({"type": "count_update", "unread_count": 0});
    state.broadcaster.send(&project_id, &sub_id, ev.to_string()).await;

    Ok(Json(json!({"success": true, "updated": result.rows_affected()})))
}

pub async fn sse_stream(
    State(state): State<Arc<AppState>>,
    Path(subscriber_id): Path<String>,
    Query(query): Query<InboxQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, (StatusCode, Json<Value>)> {
    // Accept token via query param for EventSource (can't set headers)
    let headers = if let Some(token) = &query.token {
        let mut h = headers.clone();
        if let Ok(val) = axum::http::HeaderValue::from_str(&format!("Bearer {}", token)) {
            h.insert("authorization", val);
        }
        h
    } else {
        headers
    };

    let (project_id, sub_id) = auth_inbox(&state, &headers, &subscriber_id)?;

    let rx = state.broadcaster.subscribe(&project_id, &sub_id).await;

    let stream = BroadcastStream::new(rx)
        .filter_map(|msg| async move {
            match msg {
                Ok(m) => Some(Ok(Event::default().data(m.0))),
                Err(_) => None,
            }
        });

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("ping"),
    ))
}
