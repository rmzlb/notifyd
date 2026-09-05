use anyhow::Result;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;
use sqlx::PgPool;
use tracing::{info, warn};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// One HTTP client for every webhook delivery: building a client per call
/// (TLS setup included) was the dominant cost of a batch of 500 jobs.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: once_cell::sync::Lazy<reqwest::Client> = once_cell::sync::Lazy::new(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("reqwest client")
    });
    &CLIENT
}

/// Projects among `projects` that have at least one enabled webhook. Lets the
/// worker skip the per-job fan-out entirely when nobody listens.
pub async fn projects_with_webhooks(
    pool: &sqlx::PgPool,
    projects: &[String],
) -> std::collections::HashSet<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT project_id FROM webhooks WHERE enabled = true AND project_id = ANY($1)",
    )
    .bind(projects)
    .fetch_all(pool)
    .await
    .map(|v| v.into_iter().collect())
    .unwrap_or_else(|e| {
        tracing::warn!(
            "webhook lookup failed ({}), assuming every project listens",
            e
        );
        projects.iter().cloned().collect()
    })
}

/// Fire webhooks for a project event. Called via tokio::spawn (fire-and-forget).
pub async fn fire_webhooks(
    pool: &PgPool,
    project_id: &str,
    event: &str,
    job_id: Uuid,
    channel: &str,
    subscriber_id: Option<&str>,
) -> Result<()> {
    let webhooks: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, url, secret FROM webhooks WHERE project_id=$1 AND enabled=true AND $2 = ANY(events)"
    )
    .bind(project_id)
    .bind(event)
    .fetch_all(pool)
    .await?;

    if webhooks.is_empty() {
        return Ok(());
    }

    let payload = json!({
        "event": event,
        "job_id": job_id,
        "channel": channel,
        "subscriber_id": subscriber_id,
        "timestamp": Utc::now().to_rfc3339(),
    });
    let body = serde_json::to_string(&payload)?;

    let client = http_client();

    for (wh_id, url, secret) in webhooks {
        let signature = sign_payload(&secret, &body);
        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("X-Notifyd-Signature", &signature)
            .body(body.clone())
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                info!("Webhook {} fired to {} for {}", wh_id, url, event);
            }
            Ok(r) => {
                warn!("Webhook {} to {} returned {}", wh_id, url, r.status());
            }
            Err(e) => {
                warn!("Webhook {} to {} failed: {}", wh_id, url, e);
            }
        }
    }

    Ok(())
}

fn sign_payload(secret: &str, body: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(body.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}
