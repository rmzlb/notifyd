use crate::{
    api::{auth::validate_subscriber_token, send::extract_project},
    db::InboxMessage,
    AppState,
};
use axum::response::sse::{Event, KeepAlive};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Sse,
    Json,
};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

async fn auth_inbox(
    state: &AppState,
    headers: &HeaderMap,
    path_subscriber_id: &str,
) -> Result<(String, String), (StatusCode, Json<Value>)> {
    // Subscriber JWT (frontend)
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    if let Some(token) = bearer {
        if let Some(claims) = validate_subscriber_token(state, token) {
            if claims.sub == path_subscriber_id {
                return Ok((claims.project, claims.sub));
            }
            return Err((
                StatusCode::FORBIDDEN,
                Json(json!({"error": "Token subscriber mismatch"})),
            ));
        }
    }

    // API key fallback
    let project = extract_project(state, headers).await?;
    Ok((project.id, path_subscriber_id.to_string()))
}

#[derive(Deserialize)]
pub struct InboxQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub filter: Option<String>,
    pub q: Option<String>,
}

/// Query of the SSE stream. `EventSource` cannot send headers, so the URL
/// carries a one-time ticket and nothing else: a subscriber JWT or an API key
/// would stay in proxy logs, browser history and `Referer`. `token` is the
/// former name of `ticket`, read as a ticket only until every client sends
/// `ticket`.
#[derive(Deserialize)]
pub struct StreamQuery {
    pub ticket: Option<String>,
    pub token: Option<String>,
}

pub async fn list_notifications(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(subscriber_id): Path<String>,
    Query(query): Query<InboxQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (project_id, sub_id) = auth_inbox(&state, &headers, &subscriber_id).await?;

    let limit = query.limit.unwrap_or(20).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inbox_messages WHERE project_id=$1 AND subscriber_id=$2 AND archived_at IS NULL"
    )
    .bind(&project_id)
    .bind(&sub_id)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    let rows: Vec<InboxMessage> = sqlx::query_as(
        r#"
        SELECT id, project_id, subscriber_id, body, icon, url, data, read_at, archived_at, is_todo, created_at
        FROM inbox_messages
        WHERE project_id=$1 AND subscriber_id=$2 AND archived_at IS NULL
        ORDER BY created_at DESC
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(&project_id)
    .bind(&sub_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    let search = query.q.as_deref().unwrap_or("").to_lowercase();
    let filter = query.filter.as_deref().unwrap_or("all");

    let items: Vec<Value> = rows
        .iter()
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
        .map(|r| {
            json!({
                "id": r.id,
                "body": r.body,
                "icon": r.icon,
                "url": r.url,
                "data": r.data,
                "is_read": r.read_at.is_some(),
                "read_at": r.read_at,
                "is_todo": r.is_todo,
                "created_at": r.created_at,
            })
        })
        .collect();

    Ok(Json(json!({
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
}

pub async fn unread_count(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(subscriber_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (project_id, sub_id) = auth_inbox(&state, &headers, &subscriber_id).await?;

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inbox_messages WHERE project_id=$1 AND subscriber_id=$2 AND read_at IS NULL AND archived_at IS NULL"
    )
    .bind(&project_id)
    .bind(&sub_id)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

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
    let (project_id, sub_id) = auth_inbox(&state, &headers, &subscriber_id).await?;

    if let Some(read) = req.read {
        let sql = if read {
            "UPDATE inbox_messages SET read_at=now() WHERE id=$1 AND project_id=$2 AND subscriber_id=$3"
        } else {
            "UPDATE inbox_messages SET read_at=NULL WHERE id=$1 AND project_id=$2 AND subscriber_id=$3"
        };
        sqlx::query(sql)
            .bind(msg_id)
            .bind(&project_id)
            .bind(&sub_id)
            .execute(&state.pool)
            .await
            .ok();
    }

    if let Some(archived) = req.archived {
        let sql = if archived {
            "UPDATE inbox_messages SET archived_at=now() WHERE id=$1 AND project_id=$2 AND subscriber_id=$3"
        } else {
            "UPDATE inbox_messages SET archived_at=NULL WHERE id=$1 AND project_id=$2 AND subscriber_id=$3"
        };
        sqlx::query(sql)
            .bind(msg_id)
            .bind(&project_id)
            .bind(&sub_id)
            .execute(&state.pool)
            .await
            .ok();
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
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    let ev = json!({"type": "count_update", "unread_count": count});
    state
        .broadcaster
        .send(&project_id, &sub_id, ev.to_string())
        .await;

    Ok(Json(json!({"success": true})))
}

pub async fn read_all(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(subscriber_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (project_id, sub_id) = auth_inbox(&state, &headers, &subscriber_id).await?;

    let result = sqlx::query(
        "UPDATE inbox_messages SET read_at=now() WHERE project_id=$1 AND subscriber_id=$2 AND read_at IS NULL AND archived_at IS NULL"
    )
    .bind(&project_id)
    .bind(&sub_id)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

    let ev = json!({"type": "count_update", "unread_count": 0});
    state
        .broadcaster
        .send(&project_id, &sub_id, ev.to_string())
        .await;

    Ok(Json(
        json!({"success": true, "updated": result.rows_affected()}),
    ))
}

/// POST /v1/inbox/:subscriber_id/stream-ticket
/// Returns a one-time ticket for the SSE stream URL, the only credential
/// that URL accepts.
pub async fn stream_ticket(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(subscriber_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (project_id, sub_id) = auth_inbox(&state, &headers, &subscriber_id).await?;
    let ticket = state.broadcaster.issue_ticket(&project_id, &sub_id).await;
    Ok(Json(
        json!({"ticket": ticket, "expires_in_seconds": crate::sse::TICKET_TTL_SECS}),
    ))
}

/// Consume the ticket of a stream URL. A wrong, used or expired ticket is
/// 401 and is never tried as a JWT or an API key.
async fn auth_stream_ticket(
    state: &AppState,
    ticket: &str,
    path_subscriber_id: &str,
) -> Result<(String, String), (StatusCode, Json<Value>)> {
    match state.broadcaster.consume_ticket(ticket).await {
        Some((project_id, subscriber_id)) if subscriber_id == path_subscriber_id => {
            Ok((project_id, subscriber_id))
        }
        Some(_) => Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "Ticket subscriber mismatch"})),
        )),
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": "Invalid or expired stream ticket: get one from POST /v1/inbox/:subscriber_id/stream-ticket (a JWT goes in the Authorization header, never in the URL)"
            })),
        )),
    }
}

pub async fn sse_stream(
    State(state): State<Arc<AppState>>,
    Path(subscriber_id): Path<String>,
    Query(query): Query<StreamQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, (StatusCode, Json<Value>)>
{
    let (project_id, sub_id) = match query.ticket.or(query.token) {
        Some(ticket) => auth_stream_ticket(&state, &ticket, &subscriber_id).await?,
        None => auth_inbox(&state, &headers, &subscriber_id).await?,
    };

    let rx = state.broadcaster.subscribe(&project_id, &sub_id).await;

    let stream = BroadcastStream::new(rx).filter_map(|msg| async move {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::auth::SubscriberClaims;
    use axum::{body::Body, http::Request};
    use sqlx::PgPool;
    use tower::ServiceExt;

    const JWT_SECRET: &str = "inbox-stream-test-secret";
    const API_KEY: &str = "inbox-stream-test-key";

    /// Like the other DB tests: only a database named `*_test`, rows removed
    /// by `cleanup`.
    async fn test_state() -> Option<(Arc<AppState>, String)> {
        let url = std::env::var("DATABASE_URL").ok().filter(|url| {
            let without_query = url.split('?').next().unwrap_or_default();
            without_query
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .ends_with("_test")
        });
        let Some(url) = url else {
            eprintln!("DATABASE_URL not set to a *_test database: skipping DB test");
            return None;
        };
        let pool = PgPool::connect(&url).await.expect("connect");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrate");
        let project = format!("test-inbox-stream-{}", Uuid::new_v4());
        let config: crate::config::Config = toml::from_str(&format!(
            r#"
            [server]
            port = 0
            jwt_secret = "{JWT_SECRET}"
            [database]
            url = "{url}"
            [worker]
            [connectors]
            [projects."{project}"]
            api_key = "{API_KEY}"
            channels = ["in_app"]
            "#
        ))
        .expect("test config");
        let state = AppState {
            pool: pool.clone(),
            broadcaster: crate::sse::SseBroadcaster::new(pool),
            rate_limiter: crate::middleware::RateLimiter::new(),
            resend_webhook_secret: None,
            started_at: std::time::Instant::now(),
            pacer: Arc::new(crate::pacing::Pacer::new(config.worker.pacing.clone())),
            email_breaker: Arc::new(crate::failover::Breaker::new()),
            config,
        };
        Some((Arc::new(state), project))
    }

    async fn cleanup(state: &AppState, project: &str) {
        sqlx::query("DELETE FROM sse_tickets WHERE project_id = $1")
            .bind(project)
            .execute(&state.pool)
            .await
            .expect("remove tickets");
    }

    fn subscriber_jwt(project: &str, subscriber: &str) -> String {
        let now = chrono::Utc::now().timestamp() as usize;
        let claims = SubscriberClaims {
            sub: subscriber.to_string(),
            project: project.to_string(),
            aud: "notifyd:inbox".to_string(),
            exp: now + 3600,
            iat: now,
        };
        jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(JWT_SECRET.as_bytes()),
        )
        .expect("jwt")
    }

    async fn stream_ticket(state: &Arc<AppState>, subscriber: &str) -> String {
        let request = Request::post(format!("/v1/inbox/{subscriber}/stream-ticket"))
            .header("x-api-key", API_KEY)
            .body(Body::empty())
            .unwrap();
        let response = crate::api::router(state.clone())
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        body["ticket"].as_str().expect("ticket").to_string()
    }

    async fn open_stream(state: &Arc<AppState>, uri: &str, bearer: Option<&str>) -> StatusCode {
        let mut request = Request::get(uri);
        if let Some(bearer) = bearer {
            request = request.header("authorization", format!("Bearer {bearer}"));
        }
        crate::api::router(state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn db_a_long_lived_credential_in_the_stream_url_is_refused() {
        let Some((state, project)) = test_state().await else {
            return;
        };
        let jwt = subscriber_jwt(&project, "user-1");
        for uri in [
            format!("/v1/inbox/user-1/stream?token={jwt}"),
            format!("/v1/inbox/user-1/stream?ticket={jwt}"),
            format!("/v1/inbox/user-1/stream?token={API_KEY}"),
            format!("/v1/inbox/user-1/stream?ticket={API_KEY}"),
        ] {
            assert_eq!(
                open_stream(&state, &uri, None).await,
                StatusCode::UNAUTHORIZED,
                "{uri}"
            );
        }
        cleanup(&state, &project).await;
    }

    #[tokio::test]
    async fn db_a_ticket_opens_the_stream_once() {
        let Some((state, project)) = test_state().await else {
            return;
        };
        let ticket = stream_ticket(&state, "user-1").await;
        let uri = format!("/v1/inbox/user-1/stream?ticket={ticket}");
        assert_eq!(open_stream(&state, &uri, None).await, StatusCode::OK);
        assert_eq!(
            open_stream(&state, &uri, None).await,
            StatusCode::UNAUTHORIZED
        );

        // Former parameter name, still read as a ticket until clients move.
        let ticket = stream_ticket(&state, "user-1").await;
        let uri = format!("/v1/inbox/user-1/stream?token={ticket}");
        assert_eq!(open_stream(&state, &uri, None).await, StatusCode::OK);
        assert_eq!(
            open_stream(&state, &uri, None).await,
            StatusCode::UNAUTHORIZED
        );
        cleanup(&state, &project).await;
    }

    #[tokio::test]
    async fn db_an_expired_ticket_is_refused() {
        let Some((state, project)) = test_state().await else {
            return;
        };
        for (age_secs, expected) in [(29, StatusCode::OK), (31, StatusCode::UNAUTHORIZED)] {
            let ticket = stream_ticket(&state, "user-1").await;
            sqlx::query(
                "UPDATE sse_tickets SET created_at = now() - make_interval(secs => $2) WHERE id = $1",
            )
            .bind(&ticket)
            .bind(age_secs as f64)
            .execute(&state.pool)
            .await
            .expect("age ticket");
            let uri = format!("/v1/inbox/user-1/stream?ticket={ticket}");
            assert_eq!(
                open_stream(&state, &uri, None).await,
                expected,
                "ticket {age_secs} s old"
            );
        }
        cleanup(&state, &project).await;
    }

    #[tokio::test]
    async fn db_a_ticket_for_another_subscriber_is_forbidden() {
        let Some((state, project)) = test_state().await else {
            return;
        };
        let ticket = stream_ticket(&state, "user-1").await;
        let uri = format!("/v1/inbox/user-2/stream?ticket={ticket}");
        assert_eq!(open_stream(&state, &uri, None).await, StatusCode::FORBIDDEN);
        cleanup(&state, &project).await;
    }

    #[tokio::test]
    async fn db_a_jwt_in_the_authorization_header_opens_the_stream() {
        let Some((state, project)) = test_state().await else {
            return;
        };
        let jwt = subscriber_jwt(&project, "user-1");
        assert_eq!(
            open_stream(&state, "/v1/inbox/user-1/stream", Some(&jwt)).await,
            StatusCode::OK
        );
        assert_eq!(
            open_stream(&state, "/v1/inbox/user-2/stream", Some(&jwt)).await,
            StatusCode::FORBIDDEN
        );
        cleanup(&state, &project).await;
    }
}
