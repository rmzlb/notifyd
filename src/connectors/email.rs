use super::{Channel, Connector, SendRequest};
use crate::config::EmailConfig;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{error, info, warn};

/// Maximum number of emails the Resend `/emails/batch` endpoint accepts
/// in a single API call (2026-05). Documented at
/// https://resend.com/docs/api-reference/emails/send-batch-emails
pub const RESEND_BATCH_MAX: usize = 100;

/// Create the right email connector based on provider config
pub fn create_email_connector(config: EmailConfig) -> Box<dyn Connector> {
    match config.provider.as_str() {
        "agentmail" => Box::new(AgentMailConnector::new(config)),
        _ => Box::new(ResendConnector::new(config)), // "resend" or default
    }
}

// ─── Resend ─────────────────────────────────────────────────────────

pub struct ResendConnector {
    config: EmailConfig,
    client: reqwest::Client,
}

impl ResendConnector {
    pub fn new(config: EmailConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// Resolve the canonical "from" string used on every send/batch call.
    fn from_address(&self) -> String {
        if let Some(name) = &self.config.from_name {
            format!("{} <{}>", name, self.config.from)
        } else {
            self.config.from.clone()
        }
    }

    /// Build a single email JSON object suitable for either `/emails`
    /// (single send) or one item of `/emails/batch`. The `from` field is
    /// always included so it works in both contexts.
    fn build_email_body(&self, req: &SendRequest) -> Value {
        let mut body = json!({
            "from": self.from_address(),
            "to": [req.recipient],
            "subject": req.subject.as_deref().unwrap_or("Notification"),
            "html": req.body_html.as_deref().unwrap_or(&req.body),
            "text": req.body,
        });

        // Forward custom email headers (e.g. List-Unsubscribe,
        // List-Unsubscribe-Post) — required for Gmail/Yahoo bulk sender
        // compliance (RFC 8058). Set by the caller via
        // `metadata.email_headers = { "Header-Name": "value", ... }`.
        if let Some(headers) = req.metadata.get("email_headers") {
            if headers.is_object() {
                if let Some(obj) = body.as_object_mut() {
                    obj.insert("headers".to_string(), headers.clone());
                }
            }
        }

        // Forward Resend tags (category=transactional/campaign/test, etc.)
        // for dashboard filtering. Format:
        //   [{"name":"category","value":"campaign"}, ...]
        if let Some(tags) = req.metadata.get("tags") {
            if tags.is_array() {
                if let Some(obj) = body.as_object_mut() {
                    obj.insert("tags".to_string(), tags.clone());
                }
            }
        }

        // Forward email attachments to Resend's `/emails` endpoint.
        // Resend expects: [{ "filename": "...", "content": "<base64>" }]
        // (content_type is optional and inferred from the filename).
        // We accept the inbound `content_type`/`mime` alias and normalise.
        // NOTE: only valid on single-send; the batch endpoint rejects
        // attachments, so the worker must never batch these (see worker.rs).
        if let Some(atts) = req.metadata.get("attachments").and_then(|v| v.as_array()) {
            let mapped: Vec<Value> = atts
                .iter()
                .filter_map(|a| {
                    let filename = a.get("filename").or_else(|| a.get("name"))?.as_str()?;
                    let content = a.get("content").or_else(|| a.get("file"))?.as_str()?;
                    let mut obj = serde_json::Map::new();
                    obj.insert("filename".into(), json!(filename));
                    obj.insert("content".into(), json!(content));
                    if let Some(ct) = a
                        .get("content_type")
                        .or_else(|| a.get("mime"))
                        .and_then(|v| v.as_str())
                    {
                        obj.insert("content_type".into(), json!(ct));
                    }
                    Some(Value::Object(obj))
                })
                .collect();
            if !mapped.is_empty() {
                if let Some(obj) = body.as_object_mut() {
                    obj.insert("attachments".to_string(), Value::Array(mapped));
                }
            }
        }

        body
    }
}

#[async_trait]
impl Connector for ResendConnector {
    fn channel(&self) -> Channel {
        Channel::Email
    }

    async fn send(&self, req: &SendRequest) -> Result<()> {
        let body = self.build_email_body(req);

        let res = self
            .client
            .post("https://api.resend.com/emails")
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await?;

        if res.status().is_success() {
            info!(
                "Email sent via Resend to {}",
                crate::pii::mask_email(&req.recipient)
            );
            Ok(())
        } else {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            error!("Resend error {}: {}", status, text);
            Err(anyhow!("Resend error {}: {}", status, text))
        }
    }

    /// Coalesce up to `RESEND_BATCH_MAX` requests into a single API call.
    /// Falls back to `send()` for a single request (no point batching 1).
    /// Returns the per-job results in the SAME order as the input slice
    /// so the worker can match them to job ids.
    ///
    /// Resend `/emails/batch` rules (2026-05):
    /// - Max 100 emails per call
    /// - Each item has its own from/to/subject/html/text/headers/tags
    /// - `attachments` and `scheduled_at` are NOT supported
    /// - Idempotency-Key header is per-call (not per-email) — we skip it
    ///   here because per-recipient idempotency is enforced upstream by
    ///   the worker's `idempotency_key` payload field
    /// - Response shape: { "data": [{ "id": "..." }, ...] } same order
    /// - On API error: ALL items in the batch fail the same way
    async fn send_batch(&self, reqs: &[SendRequest]) -> Vec<Result<()>> {
        if reqs.is_empty() {
            return Vec::new();
        }
        if reqs.len() == 1 {
            return vec![self.send(&reqs[0]).await];
        }
        if reqs.len() > RESEND_BATCH_MAX {
            // Defensive: caller should chunk before calling. Don't silently
            // truncate — that would drop emails. Bubble up an error per item
            // so the worker retries them.
            warn!(
                "send_batch called with {} > {} items — caller should chunk",
                reqs.len(),
                RESEND_BATCH_MAX
            );
            return reqs
                .iter()
                .map(|_| {
                    Err(anyhow!(
                        "Batch size {} exceeds Resend max {}",
                        reqs.len(),
                        RESEND_BATCH_MAX
                    ))
                })
                .collect();
        }

        let bodies: Vec<Value> = reqs.iter().map(|r| self.build_email_body(r)).collect();
        let payload = Value::Array(bodies);

        let res = self
            .client
            .post("https://api.resend.com/emails/batch")
            .bearer_auth(&self.config.api_key)
            .json(&payload)
            .send()
            .await;

        let res = match res {
            Ok(r) => r,
            Err(e) => {
                error!("Resend batch transport error: {}", e);
                // All items fail the same network error — workers will retry.
                return reqs
                    .iter()
                    .map(|_| Err(anyhow!("transport error: {}", e)))
                    .collect();
            }
        };

        let status = res.status();
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            error!("Resend batch error {}: {}", status, text);
            // All items fail with the same provider error — workers retry.
            return reqs
                .iter()
                .map(|_| Err(anyhow!("Resend batch {} : {}", status, text)))
                .collect();
        }

        info!("Email batch sent via Resend ({} recipients)", reqs.len());

        // Per-item success: Resend returns one row per accepted email,
        // keyed by index. We treat all as Ok(()) — partial failures inside
        // a 200-response batch are not currently surfaced by the Resend API
        // beyond what the webhook events stream tells us later.
        reqs.iter().map(|_| Ok(())).collect()
    }
}

// ─── AgentMail ──────────────────────────────────────────────────────
// AgentMail API: POST https://api.agentmail.to/v0/inboxes/{inbox}/messages/send
// Auth: Bearer token
// Body: { "to": ["email"], "subject": "...", "text": "...", "html": "..." }

pub struct AgentMailConnector {
    config: EmailConfig,
    client: reqwest::Client,
}

impl AgentMailConnector {
    pub fn new(config: EmailConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl Connector for AgentMailConnector {
    fn channel(&self) -> Channel {
        Channel::Email
    }

    async fn send(&self, req: &SendRequest) -> Result<()> {
        // config.from = inbox address (e.g., "craie@agentmail.to")
        // config.api_key = AgentMail API bearer token
        let inbox = &self.config.from;
        let url = format!(
            "https://api.agentmail.to/v0/inboxes/{}/messages/send",
            inbox
        );

        let mut body = json!({
            "to": [req.recipient],
            "subject": req.subject.as_deref().unwrap_or("Notification"),
            "text": req.body,
            "html": req.body_html.as_deref().unwrap_or(&req.body),
        });

        // Forward custom headers (List-Unsubscribe, etc.) — same convention
        // as the Resend path. AgentMail doc:
        // https://docs.agentmail.to/api-reference/inboxes/send-message
        if let Some(headers) = req.metadata.get("email_headers") {
            if headers.is_object() {
                if let Some(obj) = body.as_object_mut() {
                    obj.insert("headers".to_string(), headers.clone());
                }
            }
        }

        let res = self
            .client
            .post(&url)
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await?;

        if res.status().is_success() {
            info!(
                "Email sent via AgentMail to {}",
                crate::pii::mask_email(&req.recipient)
            );
            Ok(())
        } else {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            error!("AgentMail error {}: {}", status, text);
            Err(anyhow!("AgentMail error {}: {}", status, text))
        }
    }
}
