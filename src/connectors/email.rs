use super::{http_outcome, Channel, Connector, ProviderError, SendRequest, SendResult};
use crate::config::EmailConfig;
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{info, warn};

/// Maximum number of emails the Resend `/emails/batch` endpoint accepts
/// in a single API call (2026-05). Documented at
/// https://resend.com/docs/api-reference/emails/send-batch-emails
pub const RESEND_BATCH_MAX: usize = 100;

/// Create the email connector selected by `config.provider`.
pub fn create_email_connector(config: EmailConfig) -> Box<dyn Connector> {
    match config.provider.as_str() {
        "agentmail" => Box::new(AgentMailConnector::new(config)),
        "cloudflare" => Box::new(super::cloudflare::CloudflareEmailConnector::new(config)),
        "smtp" => Box::new(super::smtp::SmtpConnector::new(config)),
        "log" => Box::new(super::log::LogConnector::new(Channel::Email)),
        _ => Box::new(ResendConnector::new(config)), // "resend" or default
    }
}

/// `Name <address>` or the bare address. Per-project override wins over the
/// instance default; the two never mix (a project address without a name
/// sends bare rather than with the instance's name).
pub fn from_address(config: &EmailConfig, req: &SendRequest) -> String {
    let (email, name) = match &req.from_email {
        Some(project_email) => (project_email.as_str(), req.from_name.as_deref()),
        None => (config.from.as_str(), config.from_name.as_deref()),
    };
    match name {
        Some(n) => format!("{} <{}>", n, email),
        None => email.to_string(),
    }
}

/// Attachments as `[{filename, content(base64), content_type?}]`, accepting
/// the `name` / `file` / `mime` aliases callers have used.
pub fn normalized_attachments(metadata: &Value) -> Vec<Value> {
    metadata
        .get("attachments")
        .and_then(|v| v.as_array())
        .map(|atts| {
            atts.iter()
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
                .collect()
        })
        .unwrap_or_default()
}

pub fn cc_recipients(metadata: &Value) -> Vec<String> {
    metadata
        .get("cc")
        .and_then(Value::as_array)
        .map(|cc| {
            cc.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|address| !address.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

pub fn reply_to(metadata: &Value) -> Option<String> {
    metadata
        .get("reply_to")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from)
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

    /// Build a single email JSON object suitable for either `/emails`
    /// (single send) or one item of `/emails/batch`. The `from` field is
    /// always included so it works in both contexts.
    pub fn build_email_body(&self, req: &SendRequest) -> Value {
        let mut body = json!({
            "from": from_address(&self.config, req),
            "to": [req.recipient],
            "subject": req.subject.as_deref().unwrap_or("Notification"),
            "html": req.body_html.as_deref().unwrap_or(&req.body),
            "text": req.body,
        });
        let obj = body.as_object_mut().expect("json object");

        // Custom MIME headers (List-Unsubscribe, List-Unsubscribe-Post…):
        // required for Gmail/Yahoo bulk sender compliance (RFC 8058).
        if let Some(headers) = req.metadata.get("email_headers").filter(|h| h.is_object()) {
            obj.insert("headers".to_string(), headers.clone());
        }
        // Resend tags (category=transactional/campaign/test, job id…).
        if let Some(tags) = req.metadata.get("tags").filter(|t| t.is_array()) {
            obj.insert("tags".to_string(), tags.clone());
        }
        let cc = cc_recipients(&req.metadata);
        if !cc.is_empty() {
            obj.insert("cc".to_string(), json!(cc));
        }
        if let Some(reply_to) = reply_to(&req.metadata) {
            obj.insert("reply_to".to_string(), json!(reply_to));
        }
        // Only valid on single-send; the batch endpoint rejects attachments,
        // so the worker never batches these (see worker.rs).
        let attachments = normalized_attachments(&req.metadata);
        if !attachments.is_empty() {
            obj.insert("attachments".to_string(), Value::Array(attachments));
        }
        body
    }
}

#[async_trait]
impl Connector for ResendConnector {
    fn channel(&self) -> Channel {
        Channel::Email
    }

    fn provider(&self) -> &'static str {
        "resend"
    }

    fn batch_max(&self) -> usize {
        RESEND_BATCH_MAX
    }

    async fn send(&self, req: &SendRequest) -> SendResult {
        let body = self.build_email_body(req);
        let response = self
            .client
            .post("https://api.resend.com/emails")
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::transport("resend", e))?;
        let delivery = http_outcome("resend", response, |json| {
            json.get("id").and_then(Value::as_str).map(String::from)
        })
        .await?;
        info!(
            "Email sent via Resend to {}",
            crate::pii::mask_email(&req.recipient)
        );
        Ok(delivery)
    }

    /// Coalesce up to `RESEND_BATCH_MAX` requests into a single API call.
    /// Returns the per-job results in the SAME order as the input slice.
    ///
    /// Resend `/emails/batch` rules (2026-05): max 100 emails per call, each
    /// item carries its own from/to/subject/html/text/headers/tags,
    /// `attachments` and `scheduled_at` are not supported, the response is
    /// `{ "data": [{ "id": "..." }, ...] }` in input order, and an API error
    /// fails every item the same way.
    async fn send_batch(&self, reqs: &[SendRequest]) -> Vec<SendResult> {
        if reqs.is_empty() {
            return Vec::new();
        }
        if reqs.len() == 1 {
            return vec![self.send(&reqs[0]).await];
        }
        if reqs.len() > RESEND_BATCH_MAX {
            // Never truncate silently: that would drop emails. Fail every
            // item so the worker retries them in smaller chunks.
            warn!(
                "send_batch called with {} > {} items — caller should chunk",
                reqs.len(),
                RESEND_BATCH_MAX
            );
            return reqs
                .iter()
                .map(|_| {
                    Err(ProviderError::transient(
                        "resend",
                        format!("batch size {} exceeds max {}", reqs.len(), RESEND_BATCH_MAX),
                    ))
                })
                .collect();
        }

        let bodies: Vec<Value> = reqs.iter().map(|r| self.build_email_body(r)).collect();
        let response = match self
            .client
            .post("https://api.resend.com/emails/batch")
            .bearer_auth(&self.config.api_key)
            .json(&Value::Array(bodies))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let err = ProviderError::transport("resend", e);
                return reqs.iter().map(|_| Err(err.clone())).collect();
            }
        };

        let status = response.status();
        let retry_after = super::parse_retry_after(response.headers());
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            let err = super::classify_status("resend", status, retry_after, &text);
            return reqs.iter().map(|_| Err(err.clone())).collect();
        }

        info!("Email batch sent via Resend ({} recipients)", reqs.len());
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let ids: Vec<Option<String>> = parsed
            .get("data")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| item.get("id").and_then(Value::as_str).map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        reqs.iter()
            .enumerate()
            .map(|(i, _)| {
                Ok(super::Delivery::new(
                    "resend",
                    ids.get(i).cloned().flatten(),
                ))
            })
            .collect()
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

    fn provider(&self) -> &'static str {
        "agentmail"
    }

    async fn send(&self, req: &SendRequest) -> SendResult {
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
        let obj = body.as_object_mut().expect("json object");
        if let Some(headers) = req.metadata.get("email_headers").filter(|h| h.is_object()) {
            obj.insert("headers".to_string(), headers.clone());
        }
        let cc = cc_recipients(&req.metadata);
        if !cc.is_empty() {
            obj.insert("cc".to_string(), json!(cc));
        }
        if let Some(reply_to) = reply_to(&req.metadata) {
            obj.insert("reply_to".to_string(), json!(reply_to));
        }

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::transport("agentmail", e))?;
        let delivery = http_outcome("agentmail", response, |json| {
            json.get("message_id")
                .or_else(|| json.get("id"))
                .and_then(Value::as_str)
                .map(String::from)
        })
        .await?;
        info!(
            "Email sent via AgentMail to {}",
            crate::pii::mask_email(&req.recipient)
        );
        Ok(delivery)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> EmailConfig {
        EmailConfig {
            provider: "resend".to_string(),
            api_key: "test".to_string(),
            from: "sender@example.com".to_string(),
            from_name: Some("Sender".to_string()),
            account_id: None,
            smtp: None,
        }
    }

    fn request(metadata: Value) -> SendRequest {
        SendRequest {
            recipient: "supplier@example.com".to_string(),
            subject: Some("Purchase order".to_string()),
            body: "Body".to_string(),
            body_html: Some("<p>Body</p>".to_string()),
            from_email: None,
            from_name: None,
            metadata,
        }
    }

    #[test]
    fn resend_payload_preserves_cc_and_reply_to() {
        let connector = ResendConnector::new(config());
        let body = connector.build_email_body(&request(json!({
            "cc": ["buyer@example.com", "orders@example.com"],
            "reply_to": "reply@example.com"
        })));
        assert_eq!(
            body.get("cc"),
            Some(&json!(["buyer@example.com", "orders@example.com"]))
        );
        assert_eq!(body.get("reply_to"), Some(&json!("reply@example.com")));
        assert_eq!(
            body.get("from"),
            Some(&json!("Sender <sender@example.com>"))
        );
    }

    #[test]
    fn project_sender_overrides_instance_default_without_mixing_names() {
        let mut req = request(json!({}));
        req.from_email = Some("hello@philoeparis.fr".to_string());
        assert_eq!(from_address(&config(), &req), "hello@philoeparis.fr");
        req.from_name = Some("Philoé".to_string());
        assert_eq!(
            from_address(&config(), &req),
            "Philoé <hello@philoeparis.fr>"
        );
    }

    #[test]
    fn attachments_accept_aliases() {
        let atts = normalized_attachments(&json!({
            "attachments": [
                {"name": "a.pdf", "file": "QUJD", "mime": "application/pdf"},
                {"filename": "b.txt", "content": "QUJD"},
                {"filename": "broken"}
            ]
        }));
        assert_eq!(atts.len(), 2);
        assert_eq!(atts[0]["filename"], "a.pdf");
        assert_eq!(atts[0]["content_type"], "application/pdf");
        assert!(atts[1].get("content_type").is_none());
    }
}
