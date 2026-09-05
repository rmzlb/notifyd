use super::{http_outcome, Channel, Connector, ProviderError, SendRequest, SendResult};
use crate::config::WhatsappConfig;
use async_trait::async_trait;
use tracing::info;

/// WhatsApp connector via Telnyx (`POST /v2/messages/whatsapp`).
///
/// Note the 24-hour conversation window: free-form `text` messages are only allowed
/// within 24h of the recipient's last inbound message. Outside that window a Meta-approved
/// `template` is required. The job metadata may carry `whatsapp.template` to send a template;
/// otherwise we send a free-form text and let Telnyx enforce the window.
pub struct WhatsappConnector {
    config: WhatsappConfig,
    client: reqwest::Client,
}

impl WhatsappConnector {
    pub fn new(config: WhatsappConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    fn build_message(&self, req: &SendRequest) -> serde_json::Value {
        // If the caller supplied a template object in metadata, use it (required outside 24h).
        if let Some(template) = req.metadata.get("whatsapp").and_then(|w| w.get("template")) {
            return serde_json::json!({
                "type": "template",
                "template": template,
            });
        }
        serde_json::json!({
            "type": "text",
            "text": { "body": req.body },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn connector() -> WhatsappConnector {
        WhatsappConnector::new(WhatsappConfig {
            provider: "telnyx".to_string(),
            api_key: Some("KEY".to_string()),
            messaging_profile_id: None,
            from: "+33600000000".to_string(),
        })
    }

    fn req(metadata: serde_json::Value) -> SendRequest {
        SendRequest {
            recipient: "+33611111111".to_string(),
            subject: None,
            body: "hello".to_string(),
            body_html: None,
            from_email: None,
            from_name: None,
            metadata,
        }
    }

    #[test]
    fn builds_text_message_by_default() {
        let msg = connector().build_message(&req(json!({})));
        assert_eq!(msg["type"], "text");
        assert_eq!(msg["text"]["body"], "hello");
    }

    #[test]
    fn builds_template_message_when_provided() {
        let metadata = json!({
            "whatsapp": { "template": { "name": "welcome", "language": { "code": "en_US" } } }
        });
        let msg = connector().build_message(&req(metadata));
        assert_eq!(msg["type"], "template");
        assert_eq!(msg["template"]["name"], "welcome");
    }
}

#[async_trait]
impl Connector for WhatsappConnector {
    fn channel(&self) -> Channel {
        Channel::Whatsapp
    }

    fn provider(&self) -> &'static str {
        "telnyx"
    }

    async fn send(&self, req: &SendRequest) -> SendResult {
        match self.config.provider.as_str() {
            "telnyx" => {}
            p => {
                return Err(ProviderError::permanent(
                    "whatsapp",
                    format!("unknown WhatsApp provider: {p}"),
                ))
            }
        }

        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| ProviderError::permanent("telnyx", "WhatsApp api_key required"))?;

        let mut body = serde_json::json!({
            "from": self.config.from,
            "to": req.recipient,
            "whatsapp_message": self.build_message(req),
        });
        if let Some(profile_id) = &self.config.messaging_profile_id {
            body["messaging_profile_id"] = serde_json::Value::String(profile_id.clone());
        }

        let response = self
            .client
            .post("https://api.telnyx.com/v2/messages/whatsapp")
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::transport("telnyx", e))?;
        let delivery = http_outcome("telnyx", response, super::sms::telnyx_message_id).await?;
        info!(
            "WhatsApp sent via Telnyx to {}",
            crate::pii::mask_phone(&req.recipient)
        );
        Ok(delivery)
    }
}
