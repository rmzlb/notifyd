//! Email deliverability: Resend webhook ingestion and the per-project
//! suppression list.
//!
//! "Sent" only means Resend accepted the API call. What the recipient's mail
//! server did with it comes back minutes later through webhooks — and without
//! this module those outcomes were invisible: a bounced address kept being
//! written to, and nobody could tell a delivered notification from a
//! black-holed one.
//!
//! Flow:
//!   - the worker tags every outgoing email with `notifyd_job_id` (worker.rs),
//!     Resend echoes tags back in webhook payloads → exact job mapping without
//!     touching the connector trait;
//!   - `POST /webhooks/resend` (svix-signed, no API key) records the event
//!     idempotently, stamps the job (`delivered_at` / `bounced_at`), and turns
//!     permanent bounces and complaints into suppressions;
//!   - the worker refuses to dispatch email to an actively suppressed address
//!     (`RecipientSuppressed`, terminal — retrying cannot succeed);
//!   - `GET /v1/suppressions` + `DELETE /v1/suppressions/:id` let a project
//!     inspect and release (never delete — history stays).

use crate::{middleware, AppState};
use anyhow::Result;
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// Tag the worker adds to every outgoing email so webhook events map back to
/// the exact job. Resend tag values only allow ASCII letters, digits,
/// underscores and dashes — a hyphenated UUID fits.
pub const JOB_ID_TAG: &str = "notifyd_job_id";

/// Reject events whose svix timestamp is further than this from our clock:
/// a replayed capture must not re-suppress an address years later.
const TIMESTAMP_TOLERANCE_SECS: i64 = 300;

// ─── Suppression check (used by the worker) ─────────────────────────────────

/// Terminal send error: the recipient is on the suppression list. The worker
/// downcasts to this to fail the job immediately — a retry cannot succeed
/// while the suppression is active, so burning attempts on it is noise.
#[derive(Debug, thiserror::Error)]
#[error("recipient suppressed: {0}")]
pub struct RecipientSuppressed(pub String);

/// Human-readable description of the active suppression for this address,
/// or None when sending is allowed.
pub async fn active_suppression(
    pool: &PgPool,
    project_id: &str,
    email: &str,
) -> Result<Option<String>> {
    let row: Option<(String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT reason, created_at FROM email_suppressions
         WHERE project_id = $1 AND lower(email) = lower($2) AND released_at IS NULL",
    )
    .bind(project_id)
    .bind(email)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(reason, since)| {
        format!(
            "{} on {} — release it via DELETE /v1/suppressions to send again",
            reason,
            since.format("%Y-%m-%d")
        )
    }))
}

// ─── Svix signature verification ────────────────────────────────────────────

/// Verify a svix-signed request (Resend webhooks are delivered by svix).
/// Scheme: HMAC-SHA256 over `{svix-id}.{svix-timestamp}.{raw body}`, keyed by
/// the base64-decoded secret (after the `whsec_` prefix), compared in constant
/// time against each space-separated `v1,<base64>` candidate in svix-signature.
fn verify_svix(secret: &str, headers: &HeaderMap, body: &[u8], now_unix: i64) -> bool {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());

    let (Some(msg_id), Some(timestamp), Some(signatures)) = (
        header("svix-id"),
        header("svix-timestamp"),
        header("svix-signature"),
    ) else {
        return false;
    };

    let Ok(ts) = timestamp.parse::<i64>() else {
        return false;
    };
    if (now_unix - ts).abs() > TIMESTAMP_TOLERANCE_SECS {
        return false;
    }

    let key = secret.strip_prefix("whsec_").unwrap_or(secret);
    let Ok(key) = base64::engine::general_purpose::STANDARD.decode(key) else {
        return false;
    };

    signatures
        .split_whitespace()
        .filter_map(|s| s.strip_prefix("v1,"))
        .filter_map(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
        .any(|candidate| {
            let mut mac = HmacSha256::new_from_slice(&key).expect("HMAC accepts any key size");
            mac.update(msg_id.as_bytes());
            mac.update(b".");
            mac.update(timestamp.as_bytes());
            mac.update(b".");
            mac.update(body);
            mac.verify_slice(&candidate).is_ok()
        })
}

// ─── Webhook endpoint ───────────────────────────────────────────────────────

/// `POST /webhooks/resend` — unauthenticated route, authenticated by the svix
/// signature instead. Always answers 200 once the event is safely recorded
/// (or known-duplicate): svix retries every non-2xx, and a retried event we
/// already stored would only make noise.
pub async fn resend_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, StatusCode> {
    let Some(secret) = state.resend_webhook_secret.as_deref() else {
        // Fail closed: without a secret we cannot tell Resend from anyone.
        error!("Resend webhook received but RESEND_WEBHOOK_SECRET is not set");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };

    if !verify_svix(secret, &headers, &body, chrono::Utc::now().timestamp()) {
        warn!("Resend webhook rejected: invalid svix signature");
        return Err(StatusCode::UNAUTHORIZED);
    }

    let svix_id = headers
        .get("svix-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let payload: Value = serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let event_type = payload
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    // Job mapping: the tag we planted at send time comes back in data.tags —
    // sent as [{name, value}], returned as a {name: value} object.
    let job_id = payload
        .pointer(&format!("/data/tags/{}", JOB_ID_TAG))
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok());

    // The Resend account is shared by several apps; only notifyd-tagged
    // deliveries concern us. Untagged bounces/complaints stay worth keeping
    // (rare, and they explain reputation problems) — untagged deliveries are
    // pure noise, skip them before touching the database.
    if event_type == "email.delivered" && job_id.is_none() {
        return Ok(StatusCode::OK);
    }

    // Idempotence gate: the svix id is the primary key. 0 rows = retry of an
    // event we already processed — acknowledge and stop.
    let inserted = sqlx::query(
        "INSERT INTO provider_events (id, provider, event_type, job_id, payload)
         VALUES ($1, 'resend', $2, $3, $4)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(&svix_id)
    .bind(&event_type)
    .bind(job_id)
    .bind(&payload)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        error!("provider_events insert failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    if inserted.rows_affected() == 0 {
        return Ok(StatusCode::OK);
    }

    match event_type.as_str() {
        "email.delivered" => {
            if let Some(id) = job_id {
                let _ = sqlx::query(
                    "UPDATE jobs SET delivered_at = COALESCE(delivered_at, now()) WHERE id = $1",
                )
                .bind(id)
                .execute(&state.pool)
                .await;
            }
        }
        "email.bounced" => handle_bounce(&state, job_id, &payload, &svix_id).await,
        "email.complained" => handle_complaint(&state, job_id, &payload, &svix_id).await,
        other => {
            // Recorded above for audit; nothing to act on (yet).
            info!("Resend event {} recorded without action", other);
        }
    }

    Ok(StatusCode::OK)
}

/// Look up the job a webhook event points at. Events for emails sent before
/// tagging existed (or sent outside notifyd) have no job — record-only.
async fn job_context(
    pool: &PgPool,
    job_id: Option<Uuid>,
) -> Option<(Uuid, String, String, Option<String>)> {
    let id = job_id?;
    sqlx::query_as::<_, (Uuid, String, String, Option<String>)>(
        "SELECT id, project_id, recipient, subscriber_id FROM jobs WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

async fn handle_bounce(
    state: &Arc<AppState>,
    job_id: Option<Uuid>,
    payload: &Value,
    svix_id: &str,
) {
    let bounce_type = payload
        .pointer("/data/bounce/type")
        .and_then(|v| v.as_str())
        .unwrap_or("Permanent");
    let message = payload
        .pointer("/data/bounce/message")
        .and_then(|v| v.as_str())
        .unwrap_or("bounced");

    let Some((id, project_id, recipient, subscriber_id)) = job_context(&state.pool, job_id).await
    else {
        info!("Bounce event without a matching job — recorded only");
        return;
    };

    let error_text = format!("bounced ({}): {}", bounce_type, message);
    let _ = sqlx::query(
        "UPDATE jobs SET status = 'bounced', bounced_at = now(), error = $2
         WHERE id = $1 AND status = 'sent'",
    )
    .bind(id)
    .bind(&error_text)
    .execute(&state.pool)
    .await;

    // Transient bounces (full mailbox, greylisting) resolve on their own —
    // only a permanent rejection proves the address is dead.
    if bounce_type.eq_ignore_ascii_case("permanent") {
        let impacted = impacted_recipients(payload, &recipient);
        for address in impacted {
            suppress(
                state,
                &project_id,
                &address,
                "bounce",
                message,
                Some(id),
                svix_id,
            )
            .await;
        }
    }

    warn!(
        "Job {} bounced ({}) for {}",
        id,
        bounce_type,
        crate::pii::mask_email(&recipient)
    );
    fire_job_event(state, &project_id, "job.bounced", id, subscriber_id);
}

async fn handle_complaint(
    state: &Arc<AppState>,
    job_id: Option<Uuid>,
    payload: &Value,
    svix_id: &str,
) {
    let Some((id, project_id, recipient, subscriber_id)) = job_context(&state.pool, job_id).await
    else {
        info!("Complaint event without a matching job — recorded only");
        return;
    };

    // The mail WAS delivered — the recipient marked it as spam. The job stays
    // 'sent'; what must change is that we never write to them again: spam
    // complaints are the one signal mailbox providers weigh most against a
    // sender domain.
    let detail = payload
        .pointer("/data/subject")
        .and_then(|v| v.as_str())
        .map(|s| format!("marked as spam (subject: {})", s))
        .unwrap_or_else(|| "marked as spam".to_string());
    let impacted = impacted_recipients(payload, &recipient);
    for address in impacted {
        suppress(
            state,
            &project_id,
            &address,
            "complaint",
            &detail,
            Some(id),
            svix_id,
        )
        .await;
    }

    warn!(
        "Job {} complained ({})",
        id,
        crate::pii::mask_email(&recipient)
    );
    fire_job_event(state, &project_id, "job.complained", id, subscriber_id);
}

fn impacted_recipients(payload: &Value, fallback: &str) -> Vec<String> {
    let mut recipients: Vec<String> = payload
        .pointer("/data/to")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .map(str::to_string)
        .collect();
    recipients.sort_by_key(|address| address.to_ascii_lowercase());
    recipients.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    if recipients.is_empty() {
        recipients.push(fallback.to_string());
    }
    recipients
}

/// Insert an active suppression; a concurrent duplicate loses silently
/// (partial unique index on active rows).
async fn suppress(
    state: &Arc<AppState>,
    project_id: &str,
    email: &str,
    reason: &str,
    detail: &str,
    source_job_id: Option<Uuid>,
    provider_event_id: &str,
) {
    let res = sqlx::query(
        "INSERT INTO email_suppressions
             (project_id, email, reason, detail, source_job_id, provider_event_id)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (project_id, lower(email)) WHERE released_at IS NULL DO NOTHING",
    )
    .bind(project_id)
    .bind(email)
    .bind(reason)
    .bind(detail)
    .bind(source_job_id)
    .bind(provider_event_id)
    .execute(&state.pool)
    .await;

    match res {
        Ok(r) if r.rows_affected() > 0 => {
            middleware::audit(
                &state.pool,
                project_id,
                "resend-webhook",
                &format!("suppression.created ({})", reason),
                Some(&crate::pii::mask_email(email)),
                None,
            )
            .await;
        }
        Ok(_) => {} // already suppressed
        Err(e) => error!("suppression insert failed: {}", e),
    }
}

/// Notify the project's outbound webhooks (fire-and-forget, same pattern as
/// the worker's terminal-state notifications).
fn fire_job_event(
    state: &Arc<AppState>,
    project_id: &str,
    event: &str,
    job_id: Uuid,
    subscriber_id: Option<String>,
) {
    let pool = state.pool.clone();
    let project_id = project_id.to_string();
    let event = event.to_string();
    tokio::spawn(async move {
        if let Err(e) = crate::webhooks::fire_webhooks(
            &pool,
            &project_id,
            &event,
            job_id,
            "email",
            subscriber_id.as_deref(),
        )
        .await
        {
            warn!("Webhook fire error: {}", e);
        }
    });
}

// ─── Project-facing suppression API ─────────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct ListQuery {
    /// Include released (historical) rows; default = active only.
    #[serde(default)]
    pub include_released: bool,
}

#[derive(sqlx::FromRow)]
struct SuppressionRow {
    id: Uuid,
    email: String,
    reason: String,
    detail: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    released_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn list_suppressions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = crate::api::send::extract_project(&state, &headers).await?;

    let rows: Vec<SuppressionRow> = sqlx::query_as(
        "SELECT id, email, reason, detail, created_at, released_at
         FROM email_suppressions
         WHERE project_id = $1 AND (released_at IS NULL OR $2)
         ORDER BY created_at DESC
         LIMIT 200",
    )
    .bind(&project.id)
    .bind(q.include_released)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        error!("suppressions list failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Internal server error"})),
        )
    })?;

    let data: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.id,
                "email": r.email,
                "reason": r.reason,
                "detail": r.detail,
                "created_at": r.created_at,
                "released_at": r.released_at,
            })
        })
        .collect();

    Ok(Json(json!({ "data": data })))
}

/// Release a suppression (idempotent on active rows). The row is kept:
/// releasing is an audited decision, not an erasure — if the address bounces
/// again, a fresh suppression is created next to the released one.
pub async fn release_suppression(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = crate::api::send::extract_project(&state, &headers).await?;

    let row: Option<(String,)> = sqlx::query_as(
        "UPDATE email_suppressions
         SET released_at = now(), released_by = 'api'
         WHERE id = $1 AND project_id = $2 AND released_at IS NULL
         RETURNING email",
    )
    .bind(id)
    .bind(&project.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| {
        error!("suppression release failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Internal server error"})),
        )
    })?;

    let Some((email,)) = row else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Suppression not found or already released"})),
        ));
    };

    middleware::audit(
        &state.pool,
        &project.id,
        "api",
        "suppression.released",
        Some(&crate::pii::mask_email(&email)),
        None,
    )
    .await;

    Ok(Json(json!({"success": true, "id": id})))
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";

    fn sign(msg_id: &str, timestamp: i64, body: &[u8]) -> String {
        let key = base64::engine::general_purpose::STANDARD
            .decode(SECRET.strip_prefix("whsec_").unwrap())
            .unwrap();
        let mut mac = HmacSha256::new_from_slice(&key).unwrap();
        mac.update(format!("{}.{}.", msg_id, timestamp).as_bytes());
        mac.update(body);
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
    }

    fn headers(msg_id: &str, timestamp: i64, signature: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("svix-id", msg_id.parse().unwrap());
        h.insert("svix-timestamp", timestamp.to_string().parse().unwrap());
        h.insert("svix-signature", signature.parse().unwrap());
        h
    }

    #[test]
    fn accepts_a_valid_signature() {
        let body = br#"{"type":"email.bounced"}"#;
        let now = 1_700_000_000;
        let sig = format!("v1,{}", sign("msg_1", now, body));
        assert!(verify_svix(SECRET, &headers("msg_1", now, &sig), body, now));
    }

    #[test]
    fn accepts_the_valid_candidate_among_several() {
        let body = br#"{"type":"email.bounced"}"#;
        let now = 1_700_000_000;
        let sig = format!("v1,Zm9v v1,{}", sign("msg_1", now, body));
        assert!(verify_svix(SECRET, &headers("msg_1", now, &sig), body, now));
    }

    #[test]
    fn rejects_a_tampered_body() {
        let now = 1_700_000_000;
        let sig = format!("v1,{}", sign("msg_1", now, br#"{"a":1}"#));
        assert!(!verify_svix(
            SECRET,
            &headers("msg_1", now, &sig),
            br#"{"a":2}"#,
            now
        ));
    }

    #[test]
    fn rejects_a_stale_timestamp() {
        let body = br#"{}"#;
        let then = 1_700_000_000;
        let sig = format!("v1,{}", sign("msg_1", then, body));
        assert!(!verify_svix(
            SECRET,
            &headers("msg_1", then, &sig),
            body,
            then + TIMESTAMP_TOLERANCE_SECS + 1
        ));
    }

    #[test]
    fn rejects_missing_headers() {
        assert!(!verify_svix(
            SECRET,
            &HeaderMap::new(),
            b"{}",
            1_700_000_000
        ));
    }
}
