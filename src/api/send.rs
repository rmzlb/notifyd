use crate::{middleware, AppState};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

// ─── Auth helper ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Project {
    pub id: String,
    #[allow(dead_code)]
    pub rate_limit: u32,
}

pub fn extract_api_key(headers: &HeaderMap) -> &str {
    headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
        })
        .unwrap_or("")
        .trim()
}

pub async fn extract_project(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Project, (StatusCode, Json<Value>)> {
    let api_key = extract_api_key(headers);

    if api_key.is_empty() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Missing API key (X-Api-Key header)"})),
        ));
    }

    // 1. Check TOML config (fast path, constant-time compare)
    for (id, proj) in &state.config.projects {
        if constant_time_eq(proj.api_key.as_bytes(), api_key.as_bytes()) {
            return Ok(Project {
                id: id.clone(),
                rate_limit: 600,
            });
        }
    }

    // 2. Check DB (hash the incoming key and compare against stored hashes)
    // BUG FIX #3: Never compare plaintext API keys in SQL
    let api_key_hash = crate::api::projects::hash_key(api_key);
    let row: Option<(String, Option<i32>)> = sqlx::query_as(
        "SELECT id, rate_limit_per_min FROM projects WHERE api_key_hash = $1 OR secondary_api_key_hash = $1"
    )
    .bind(&api_key_hash)
    .fetch_optional(&state.pool)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "DB error"}))))?;

    if let Some((id, rate_limit)) = row {
        // Rate limit check
        let limit = rate_limit.unwrap_or(600) as u32;
        if !state.rate_limiter.check(&id, limit).await {
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error": "Rate limit exceeded"})),
            ));
        }
        return Ok(Project {
            id,
            rate_limit: limit,
        });
    }

    Err((
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": "Invalid API key"})),
    ))
}

/// Constant-time byte comparison to prevent timing attacks on API key validation
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
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
    /// Optional MIME headers to attach to the outgoing email. Forwarded
    /// to the email connector via `metadata.email_headers`. Used for
    /// List-Unsubscribe / List-Unsubscribe-Post (RFC 8058) and similar.
    /// Format: `{ "Header-Name": "value", ... }`.
    pub email_headers: Option<Value>,
    /// Optional Resend tags for dashboard categorization & filtering.
    /// Format: `[ {"name":"category","value":"campaign"}, ... ]`.
    /// See https://resend.com/docs/api-reference/emails/send-email#body-parameters
    pub tags: Option<Value>,
    /// Optional email attachments. Forwarded to the email connector via
    /// `metadata.attachments`. Resend's single-send `/emails` endpoint
    /// supports these natively; the `/emails/batch` endpoint does NOT, so
    /// the worker routes any email carrying attachments through the
    /// single-send path. Format:
    /// `[ {"filename": "facture.pdf", "content": "<base64>", "content_type": "application/pdf"} ]`.
    pub attachments: Option<Value>,
    /// Carbon-copy recipients for email deliveries. These are part of the
    /// durable job payload so retries preserve the exact envelope.
    pub cc: Option<Vec<String>>,
    /// Address that receives replies to the email.
    pub reply_to: Option<String>,
    /// Queue priority: `"critical"` (10), `"high"` (30), `"normal"` (50,
    /// default), `"low"` (70), `"bulk"` (80) or a number 0–100. Lower goes
    /// first. Transactional traffic keeps the default; marketing uses `bulk`.
    pub priority: Option<Value>,
    /// Send window override: `{start, end, tz?, days?, applies_to?}` or
    /// `false` to bypass the project's window for this request.
    pub send_window: Option<Value>,
}

/// Effective send window for a request: the request's own object wins,
/// `false` disables, otherwise the project's `settings.send_window`.
pub async fn effective_send_window(
    state: &Arc<AppState>,
    project_id: &str,
    requested: Option<&Value>,
) -> Result<Option<crate::send_window::SendWindow>, String> {
    match requested {
        Some(Value::Bool(false)) => return Ok(None),
        Some(v) if v.is_object() => {
            return crate::send_window::SendWindow::parse(v)
                .map(Some)
                .map_err(|e| e.to_string())
        }
        Some(Value::Null) | None => {}
        Some(_) => return Err("send_window must be an object or false".to_string()),
    }
    let settings: Option<Value> = sqlx::query_scalar("SELECT settings FROM projects WHERE id = $1")
        .bind(project_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| e.to_string())?;
    match settings.as_ref().and_then(|s| s.get("send_window")) {
        Some(v) if v.is_object() => crate::send_window::SendWindow::parse(v)
            .map(Some)
            .map_err(|e| format!("project send_window is invalid: {e}")),
        _ => Ok(None),
    }
}

/// Recipient timezone from the subscriber record, when the job names one.
pub async fn subscriber_timezone(
    state: &Arc<AppState>,
    project_id: &str,
    subscriber_id: Option<&str>,
) -> Option<String> {
    let id = subscriber_id?;
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT timezone FROM subscribers WHERE project_id = $1 AND id = $2",
    )
    .bind(project_id)
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .flatten()
}

/// Apply the window to a job's schedule. Marketing jobs always obey it;
/// other jobs only when the window says `applies_to: all`.
pub fn windowed_schedule(
    window: Option<&crate::send_window::SendWindow>,
    scheduled_at: DateTime<Utc>,
    marketing: bool,
    recipient_tz: Option<&str>,
) -> DateTime<Utc> {
    match window {
        Some(w) if marketing || w.applies_to_all => w.next_allowed(scheduled_at, recipient_tz),
        _ => scheduled_at,
    }
}

/// Resolve the `priority` request field to the 0–100 column value.
pub fn resolve_priority(value: Option<&Value>, default: i16) -> Result<i16, String> {
    match value {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Number(n)) => n
            .as_i64()
            .filter(|p| (0..=100).contains(p))
            .map(|p| p as i16)
            .ok_or_else(|| "priority must be an integer between 0 and 100".to_string()),
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "critical" => Ok(PRIORITY_CRITICAL),
            "high" => Ok(PRIORITY_HIGH),
            "normal" => Ok(PRIORITY_NORMAL),
            "low" => Ok(PRIORITY_LOW),
            "bulk" => Ok(PRIORITY_BULK),
            other => other
                .parse::<i64>()
                .ok()
                .filter(|p| (0..=100).contains(p))
                .map(|p| p as i16)
                .ok_or_else(|| {
                    "priority must be critical, high, normal, low, bulk or 0–100".to_string()
                }),
        },
        Some(_) => Err("priority must be a string or a number".to_string()),
    }
}

/// Marketing traffic identified by its provider tag defaults to `bulk`, so
/// clients that already tag campaigns get the priority lane for free.
pub fn default_priority_from_tags(tags: Option<&Value>) -> i16 {
    let is_marketing = tags
        .and_then(Value::as_array)
        .map(|items| {
            items.iter().any(|tag| {
                tag.get("name").and_then(Value::as_str) == Some("category")
                    && matches!(
                        tag.get("value").and_then(Value::as_str),
                        Some("campaign") | Some("marketing") | Some("newsletter") | Some("bulk")
                    )
            })
        })
        .unwrap_or(false);
    if is_marketing {
        PRIORITY_BULK
    } else {
        PRIORITY_NORMAL
    }
}

pub const PRIORITY_CRITICAL: i16 = 10;
pub const PRIORITY_HIGH: i16 = 30;
pub const PRIORITY_NORMAL: i16 = 50;
pub const PRIORITY_LOW: i16 = 70;
pub const PRIORITY_BULK: i16 = 80;

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
    pub icon: Option<String>,
    pub url: Option<String>,
    /// Same values as on `/v1/send`. A fan-out is marketing by default:
    /// `bulk` (80) unless the caller says otherwise.
    pub priority: Option<Value>,
    /// Dedupes the whole fan-out: the key is declined per subscriber and
    /// channel (`<key>-<subscriber>-<channel>`), so replaying the call
    /// creates no second job for anyone already queued or sent.
    pub idempotency_key: Option<String>,
    /// Send window override, see `/v1/send`.
    pub send_window: Option<Value>,
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

pub async fn send_notification(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<SendRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    // Audit
    tokio::spawn({
        let pool = state.pool.clone();
        let pid = project.id.clone();
        async move {
            middleware::audit(&pool, &pid, "api_key", "send", None, None).await;
        }
    });

    let channels: Vec<String> = req.channels.clone().unwrap_or_else(|| {
        req.channel
            .as_deref()
            .unwrap_or("in_app")
            .split(',')
            .map(|s| s.trim().to_string())
            .collect()
    });

    let recipient = req
        .to
        .clone()
        .or_else(|| req.subscriber_id.clone())
        .ok_or_else(|| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": "Missing 'to' or 'subscriber_id'"})),
            )
        })?;

    let (cc, reply_to) = validate_email_envelope(
        &channels,
        &recipient,
        req.cc.as_deref(),
        req.reply_to.as_deref(),
    )
    .map_err(|error| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": error })),
        )
    })?;

    let priority = resolve_priority(
        req.priority.as_ref(),
        default_priority_from_tags(req.tags.as_ref()),
    )
    .map_err(|error| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": error })),
        )
    })?;
    let window = effective_send_window(&state, &project.id, req.send_window.as_ref())
        .await
        .map_err(|error| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": error })),
            )
        })?;
    let recipient_tz = subscriber_timezone(&state, &project.id, req.subscriber_id.as_deref()).await;
    let marketing =
        priority >= PRIORITY_BULK || default_priority_from_tags(req.tags.as_ref()) == PRIORITY_BULK;
    let scheduled_at = windowed_schedule(
        window.as_ref(),
        req.scheduled_at.unwrap_or_else(Utc::now),
        marketing,
        recipient_tz.as_deref(),
    );

    let mut payload = json!({
        "subject": req.subject,
        "body": req.body,
        "body_html": req.body_html,
        "icon": req.icon.as_deref().unwrap_or("bell"),
        "url": req.url,
    });

    // Forward optional MIME headers (List-Unsubscribe etc.) into the job
    // payload so the email connector picks them up via `metadata.email_headers`.
    if let Some(headers) = &req.email_headers {
        if let Some(p) = payload.as_object_mut() {
            p.insert("email_headers".to_string(), headers.clone());
        }
    }

    // Forward Resend tags (categorize emails: transactional / campaign / test).
    if let Some(tags) = &req.tags {
        if let Some(p) = payload.as_object_mut() {
            p.insert("tags".to_string(), tags.clone());
        }
    }

    // Forward email attachments (base64). The connector adds them to the
    // Resend single-send body; the worker keeps attachment-bearing emails
    // out of the batch path (Resend batch doesn't support attachments).
    if let Some(attachments) = &req.attachments {
        if let Some(p) = payload.as_object_mut() {
            p.insert("attachments".to_string(), attachments.clone());
        }
    }

    if !cc.is_empty() {
        if let Some(p) = payload.as_object_mut() {
            p.insert("cc".to_string(), json!(cc));
        }
    }

    if let Some(reply_to) = reply_to {
        if let Some(p) = payload.as_object_mut() {
            p.insert("reply_to".to_string(), json!(reply_to));
        }
    }

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
        let idem_key = req
            .idempotency_key
            .as_ref()
            .map(|k| format!("{}-{}", k, channel));

        // Stripe-like idempotency: a key held by a live or succeeded job
        // returns THAT job untouched — re-POSTing must never re-arm a sent
        // notification (the old DO UPDATE reset it to 'pending' and the
        // email went out twice). A failed/cancelled job releases its key
        // (partial unique index, migration 014), so the retry inserts a
        // fresh row and the history keeps both.
        let inserted: Option<Uuid> = sqlx::query_scalar(
            r#"
            INSERT INTO jobs (project_id, channel, subscriber_id, recipient, template_id, payload, scheduled_at, idempotency_key, priority, max_attempts)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (project_id, idempotency_key)
                WHERE idempotency_key IS NOT NULL
                  AND status NOT IN ('failed', 'cancelled')
                DO NOTHING
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
        .bind(priority)
        .bind(state.config.worker.max_attempts)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;

        let job_id: Uuid = match inserted {
            Some(id) => id,
            None => sqlx::query_scalar(
                "SELECT id FROM jobs
                 WHERE project_id = $1 AND idempotency_key = $2
                   AND status NOT IN ('failed', 'cancelled')
                 ORDER BY created_at DESC
                 LIMIT 1",
            )
            .bind(&project.id)
            .bind(idem_key.as_deref())
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, {
                    tracing::error!("DB error: {}", e);
                    Json(json!({"error": "Internal server error"}))
                })
            })?
            .ok_or_else(|| {
                // Only reachable if the holder flipped to failed between the
                // two statements — the caller can simply retry.
                (
                    StatusCode::CONFLICT,
                    Json(json!({"error": "Idempotency key contention, retry the request"})),
                )
            })?,
        };

        job_ids.push(job_id);
    }

    Ok(Json(json!({
        "success": true,
        "job_ids": job_ids,
        "scheduled_at": scheduled_at,
        "channels": channels,
    })))
}

fn looks_like_email(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.len() <= 254
        && !trimmed.contains(char::is_whitespace)
        && trimmed
            .split_once('@')
            .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'))
}

fn validate_email_envelope(
    channels: &[String],
    recipient: &str,
    cc: Option<&[String]>,
    reply_to: Option<&str>,
) -> Result<(Vec<String>, Option<String>), String> {
    const MAX_CC_RECIPIENTS: usize = 10;

    let has_email = channels.iter().any(|channel| channel == "email");
    if !has_email && (cc.is_some() || reply_to.is_some()) {
        return Err("cc and reply_to can only be used with the email channel".to_string());
    }

    let mut seen = HashSet::new();
    seen.insert(recipient.trim().to_ascii_lowercase());
    let mut normalized_cc = Vec::new();
    for address in cc.unwrap_or_default() {
        let address = address.trim();
        if !looks_like_email(address) {
            return Err(format!("Invalid CC email address: {address}"));
        }
        if seen.insert(address.to_ascii_lowercase()) {
            normalized_cc.push(address.to_string());
        }
    }
    if normalized_cc.len() > MAX_CC_RECIPIENTS {
        return Err(format!(
            "An email cannot contain more than {MAX_CC_RECIPIENTS} CC recipients"
        ));
    }

    let normalized_reply_to = reply_to
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .map(|address| {
            if looks_like_email(address) {
                Ok(address.to_string())
            } else {
                Err(format!("Invalid reply_to email address: {address}"))
            }
        })
        .transpose()?;

    Ok((normalized_cc, normalized_reply_to))
}

#[cfg(test)]
mod priority_tests {
    use super::*;

    #[test]
    fn named_and_numeric_priorities() {
        assert_eq!(resolve_priority(Some(&json!("critical")), 50).unwrap(), 10);
        assert_eq!(resolve_priority(Some(&json!("BULK")), 50).unwrap(), 80);
        assert_eq!(resolve_priority(Some(&json!(42)), 50).unwrap(), 42);
        assert_eq!(resolve_priority(Some(&json!("7")), 50).unwrap(), 7);
        assert_eq!(resolve_priority(None, 80).unwrap(), 80);
        assert!(resolve_priority(Some(&json!(101)), 50).is_err());
        assert!(resolve_priority(Some(&json!("urgentissime")), 50).is_err());
        assert!(resolve_priority(Some(&json!(true)), 50).is_err());
    }

    #[test]
    fn campaign_tag_defaults_to_bulk() {
        let tags = json!([{"name": "category", "value": "campaign"}]);
        assert_eq!(default_priority_from_tags(Some(&tags)), PRIORITY_BULK);
        let tags = json!([{"name": "category", "value": "transactional"}]);
        assert_eq!(default_priority_from_tags(Some(&tags)), PRIORITY_NORMAL);
        assert_eq!(default_priority_from_tags(None), PRIORITY_NORMAL);
    }
}

#[cfg(test)]
mod tests {
    use super::validate_email_envelope;

    #[test]
    fn email_envelope_is_normalized_without_duplicate_primary_recipient() {
        let channels = vec!["email".to_string()];
        let cc = vec![
            " Buyer@example.com ".to_string(),
            "supplier@example.com".to_string(),
            "buyer@example.com".to_string(),
        ];

        let (cc, reply_to) = validate_email_envelope(
            &channels,
            "SUPPLIER@example.com",
            Some(&cc),
            Some(" orders@example.com "),
        )
        .expect("valid envelope");

        assert_eq!(cc, vec!["Buyer@example.com"]);
        assert_eq!(reply_to.as_deref(), Some("orders@example.com"));
    }

    #[test]
    fn email_only_fields_fail_closed_on_invalid_input() {
        assert!(validate_email_envelope(
            &["in_app".to_string()],
            "subscriber-1",
            Some(&["buyer@example.com".to_string()]),
            None,
        )
        .is_err());
        assert!(validate_email_envelope(
            &["email".to_string()],
            "supplier@example.com",
            Some(&["invalid".to_string()]),
            None,
        )
        .is_err());
    }
}

pub async fn batch_notification(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<BatchRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    tokio::spawn({
        let pool = state.pool.clone();
        let pid = project.id.clone();
        async move {
            middleware::audit(&pool, &pid, "api_key", "batch", None, None).await;
        }
    });

    let channels: Vec<String> = req.channels.clone().unwrap_or_else(|| {
        req.channel
            .as_deref()
            .unwrap_or("in_app")
            .split(',')
            .map(|s| s.trim().to_string())
            .collect()
    });

    let requested_at = req.scheduled_at.unwrap_or_else(Utc::now);
    let priority = resolve_priority(req.priority.as_ref(), PRIORITY_BULK).map_err(|error| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": error })),
        )
    })?;
    let window = effective_send_window(&state, &project.id, req.send_window.as_ref())
        .await
        .map_err(|error| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": error })),
            )
        })?;
    let marketing = priority >= PRIORITY_BULK;

    let payload = json!({
        "subject": req.subject,
        "body": req.body,
        "body_html": req.body_html,
        "vars": req.vars,
        "icon": req.icon.as_deref().unwrap_or("bell"),
        "url": req.url,
    });

    let mut total = 0usize;
    let mut deduplicated = 0usize;
    for subscriber_id in &req.subscribers {
        // Each recipient gets its own daytime.
        let recipient_tz = if window.is_some() {
            subscriber_timezone(&state, &project.id, Some(subscriber_id)).await
        } else {
            None
        };
        let scheduled_at = windowed_schedule(
            window.as_ref(),
            requested_at,
            marketing,
            recipient_tz.as_deref(),
        );
        for channel in &channels {
            let idem_key = req
                .idempotency_key
                .as_ref()
                .map(|k| format!("{}-{}-{}", k, subscriber_id, channel));
            // Same rule as /v1/send: a key held by a live or succeeded job
            // returns it untouched (partial unique index, migration 014).
            let inserted = sqlx::query(
                r#"
                INSERT INTO jobs (project_id, channel, subscriber_id, recipient, template_id, payload, scheduled_at, priority, max_attempts, idempotency_key)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                ON CONFLICT (project_id, idempotency_key)
                    WHERE idempotency_key IS NOT NULL
                      AND status NOT IN ('failed', 'cancelled')
                    DO NOTHING
                "#,
            )
            .bind(&project.id)
            .bind(channel)
            .bind(subscriber_id)
            .bind(subscriber_id)
            .bind(req.template.as_deref())
            .bind(&payload)
            .bind(scheduled_at)
            .bind(priority)
            .bind(state.config.worker.max_attempts)
            .bind(idem_key.as_deref())
            .execute(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, { tracing::error!("DB error: {}", e); Json(json!({"error": "Internal server error"})) }))?;
            if inserted.rows_affected() == 1 {
                total += 1;
            } else {
                deduplicated += 1;
            }
        }
    }

    Ok(Json(json!({
        "success": true,
        "jobs_created": total,
        "jobs_deduplicated": deduplicated,
        "subscribers": req.subscribers.len(),
        "channels": channels,
    })))
}
