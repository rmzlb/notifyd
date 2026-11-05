use anyhow::Result;
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::json;
use sha2::Sha256;
use sqlx::PgPool;
use std::net::IpAddr;
use tracing::{info, warn};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            let is_cgnat_tailscale = octets[0] == 100 && (octets[1] & 0b1100_0000) == 64;
            let is_benchmark = octets[0] == 198 && (octets[1] == 18 || octets[1] == 19);
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || is_cgnat_tailscale
                || is_benchmark
        }
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_forbidden_ip(IpAddr::V4(mapped));
            }
            let seg0 = v6.segments()[0];
            let is_unique_local = (seg0 & 0xfe00) == 0xfc00;
            let is_link_local = (seg0 & 0xffc0) == 0xfe80;
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || is_unique_local
                || is_link_local
        }
    }
}

/// Validate an outbound target URL (Web Push endpoint or webhook receiver) against SSRF.
/// Rejects loopback, RFC1918 private, link-local (`169.254.0.0/16`), and Tailscale (`100.64.0.0/10`)
/// hosts and resolved IPs.
pub async fn validate_outbound_url(raw_url: &str, require_https: bool) -> Result<reqwest::Url, String> {
    let parsed = reqwest::Url::parse(raw_url).map_err(|e| format!("Invalid URL: {e}"))?;
    match parsed.scheme() {
        "https" => {}
        "http" if !require_https => {}
        _ if require_https => return Err("URL must use https://".to_string()),
        _ => return Err("URL must use http:// or https://".to_string()),
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| "URL missing host".to_string())?;
    let host_lower = host.to_ascii_lowercase();
    if host_lower == "localhost"
        || host_lower.ends_with(".localhost")
        || host_lower.ends_with(".local")
        || host_lower.ends_with(".internal")
        || host_lower.ends_with(".ts.net")
    {
        return Err("Target host is not allowed".to_string());
    }

    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        if is_forbidden_ip(ip) {
            return Err("Target IP address is not allowed".to_string());
        }
        return Ok(parsed);
    }

    let port = parsed.port_or_known_default().unwrap_or(443);
    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("Failed to resolve target host: {e}"))?;

    let mut resolved_any = false;
    for addr in addrs {
        resolved_any = true;
        if is_forbidden_ip(addr.ip()) {
            return Err("Target host resolves to a forbidden IP address".to_string());
        }
    }
    if !resolved_any {
        return Err("Target host did not resolve to any IP".to_string());
    }

    Ok(parsed)
}

/// One HTTP client for every webhook delivery: building a client per call
/// (TLS setup included) was the dominant cost of a batch of 500 jobs.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: once_cell::sync::Lazy<reqwest::Client> = once_cell::sync::Lazy::new(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
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
        let validated_url = match validate_outbound_url(&url, false).await {
            Ok(u) => u,
            Err(reason) => {
                warn!("Webhook {} blocked by SSRF policy: {}", wh_id, reason);
                continue;
            }
        };
        let signature = sign_payload(&secret, &body);
        let resp = client
            .post(validated_url)
            .header("Content-Type", "application/json")
            .header("X-Notifyd-Signature", &signature)
            .body(body.clone())
            .send()
            .await;

        // The id, never the URL: a receiver URL often embeds its own secret.
        match resp {
            Ok(r) if r.status().is_success() => {
                info!("Webhook {} fired for {}", wh_id, event);
            }
            Ok(r) => {
                warn!("Webhook {} returned {}", wh_id, r.status());
            }
            Err(e) => {
                warn!("Webhook {} failed: {}", wh_id, e.without_url());
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
