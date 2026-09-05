//! Cloudflare Email Service (outbound), REST API.
//! `POST https://api.cloudflare.com/client/v4/accounts/{account_id}/email/sending/send`
//! Bearer token with the Email Sending permission. Documented at
//! https://developers.cloudflare.com/email-service/api/send-emails/rest-api/
//! (2026-06): body `to`, `from` (string or `{address, name}`), `cc`, `bcc`,
//! `reply_to`, `subject`, `html`, `text`, `headers`, `attachments`
//! (`{content(base64), filename, type, disposition}`); response
//! `result.delivered / queued / permanent_bounces`; 429 code 10004
//! `email.sending.error.throttled`. Limits: 50 recipients, 5 MiB per message.

use super::{
    email::{cc_recipients, normalized_attachments, reply_to},
    Channel, Connector, Delivery, ProviderError, SendRequest, SendResult,
};
use crate::config::EmailConfig;
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::info;

pub struct CloudflareEmailConnector {
    config: EmailConfig,
    client: reqwest::Client,
}

impl CloudflareEmailConnector {
    pub fn new(config: EmailConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    fn endpoint(&self) -> String {
        format!(
            "https://api.cloudflare.com/client/v4/accounts/{}/email/sending/send",
            self.config.account_id.as_deref().unwrap_or_default()
        )
    }

    /// Named sender as Cloudflare's `{address, name}` object, bare address
    /// otherwise. Same override rule as every email connector.
    fn from_value(&self, req: &SendRequest) -> Value {
        let (email, name) = match &req.from_email {
            Some(project_email) => (project_email.as_str(), req.from_name.as_deref()),
            None => (self.config.from.as_str(), self.config.from_name.as_deref()),
        };
        match name {
            Some(n) => json!({ "address": email, "name": n }),
            None => json!(email),
        }
    }

    pub fn build_body(&self, req: &SendRequest) -> Value {
        let mut body = json!({
            "to": req.recipient,
            "from": self.from_value(req),
            "subject": req.subject.as_deref().unwrap_or("Notification"),
            "html": req.body_html.as_deref().unwrap_or(&req.body),
            "text": req.body,
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
        let attachments: Vec<Value> = normalized_attachments(&req.metadata)
            .into_iter()
            .map(|a| {
                let mut item = json!({
                    "content": a["content"],
                    "filename": a["filename"],
                    "disposition": "attachment",
                });
                if let Some(ct) = a.get("content_type") {
                    item["type"] = ct.clone();
                }
                item
            })
            .collect();
        if !attachments.is_empty() {
            obj.insert("attachments".to_string(), Value::Array(attachments));
        }
        body
    }
}

/// Cloudflare answers 200 even when the recipient bounced at once: the
/// address then sits in `result.permanent_bounces`. That is a permanent
/// error for this job, not a success.
pub fn outcome_from_result(recipient: &str, result: &Value) -> Result<(), ProviderError> {
    let bounced = result
        .get("permanent_bounces")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .any(|address| address.eq_ignore_ascii_case(recipient))
        })
        .unwrap_or(false);
    if bounced {
        return Err(ProviderError::permanent(
            "cloudflare",
            format!(
                "recipient {} bounced permanently",
                crate::pii::mask_email(recipient)
            ),
        ));
    }
    Ok(())
}

#[async_trait]
impl Connector for CloudflareEmailConnector {
    fn channel(&self) -> Channel {
        Channel::Email
    }

    fn provider(&self) -> &'static str {
        "cloudflare"
    }

    async fn send(&self, req: &SendRequest) -> SendResult {
        let body = self.build_body(req);
        let response = self
            .client
            .post(self.endpoint())
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::transport("cloudflare", e))?;

        let status = response.status();
        let retry_after = super::parse_retry_after(response.headers());
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(super::classify_status(
                "cloudflare",
                status,
                retry_after,
                &text,
            ));
        }
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if parsed.get("success").and_then(Value::as_bool) == Some(false) {
            let message = parsed
                .get("errors")
                .and_then(Value::as_array)
                .and_then(|errors| errors.first())
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("request rejected")
                .to_string();
            return Err(ProviderError::permanent("cloudflare", message));
        }
        outcome_from_result(&req.recipient, parsed.get("result").unwrap_or(&Value::Null))?;
        info!(
            "Email sent via Cloudflare Email Service to {}",
            crate::pii::mask_email(&req.recipient)
        );
        // The REST API reports recipient status, not a message id.
        Ok(Delivery::new("cloudflare", None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connector() -> CloudflareEmailConnector {
        CloudflareEmailConnector::new(EmailConfig {
            provider: "cloudflare".to_string(),
            api_key: "token".to_string(),
            from: "orders@philoeparis.fr".to_string(),
            from_name: Some("Philoé".to_string()),
            account_id: Some("acct123".to_string()),
            smtp: None,
        })
    }

    fn request(metadata: Value) -> SendRequest {
        SendRequest {
            recipient: "jane@example.com".to_string(),
            subject: Some("Your invoice".to_string()),
            body: "Invoice attached".to_string(),
            body_html: Some("<h1>Invoice</h1>".to_string()),
            from_email: None,
            from_name: None,
            metadata,
        }
    }

    #[test]
    fn body_follows_cloudflare_schema() {
        let body = connector().build_body(&request(json!({
            "reply_to": "support@philoeparis.fr",
            "cc": ["manager@example.com"],
            "email_headers": {"List-Unsubscribe": "<https://x/u>"},
            "attachments": [{"filename": "f.pdf", "content": "QUJD", "content_type": "application/pdf"}]
        })));
        assert_eq!(body["to"], "jane@example.com");
        assert_eq!(
            body["from"],
            json!({"address": "orders@philoeparis.fr", "name": "Philoé"})
        );
        assert_eq!(body["reply_to"], "support@philoeparis.fr");
        assert_eq!(body["cc"], json!(["manager@example.com"]));
        assert_eq!(body["headers"]["List-Unsubscribe"], "<https://x/u>");
        assert_eq!(body["attachments"][0]["type"], "application/pdf");
        assert_eq!(body["attachments"][0]["disposition"], "attachment");
        assert_eq!(
            connector().endpoint(),
            "https://api.cloudflare.com/client/v4/accounts/acct123/email/sending/send"
        );
    }

    #[test]
    fn permanent_bounce_in_result_is_a_permanent_error() {
        let result =
            json!({"delivered": [], "permanent_bounces": ["JANE@example.com"], "queued": []});
        let err = outcome_from_result("jane@example.com", &result).unwrap_err();
        assert_eq!(err.kind, super::super::ProviderErrorKind::Permanent);
        assert!(outcome_from_result("other@example.com", &result).is_ok());
    }
}
