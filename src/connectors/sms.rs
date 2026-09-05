use super::{http_outcome, Channel, Connector, ProviderError, SendRequest, SendResult};
use crate::config::SmsConfig;
use async_trait::async_trait;
use serde_json::Value;
use tracing::info;

pub struct SmsConnector {
    config: SmsConfig,
    client: reqwest::Client,
}

impl SmsConnector {
    pub fn new(config: SmsConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl Connector for SmsConnector {
    fn channel(&self) -> Channel {
        Channel::Sms
    }

    fn provider(&self) -> &'static str {
        match self.config.provider.as_str() {
            "twilio" => "twilio",
            _ => "telnyx",
        }
    }

    async fn send(&self, req: &SendRequest) -> SendResult {
        match self.config.provider.as_str() {
            "twilio" => self.send_twilio(req).await,
            "telnyx" => self.send_telnyx(req).await,
            p => Err(ProviderError::permanent(
                "sms",
                format!("unknown SMS provider: {p}"),
            )),
        }
    }
}

impl SmsConnector {
    async fn send_twilio(&self, req: &SendRequest) -> SendResult {
        let account_sid = self
            .config
            .account_sid
            .as_deref()
            .ok_or_else(|| ProviderError::permanent("twilio", "account_sid required"))?;
        let auth_token = self
            .config
            .auth_token
            .as_deref()
            .ok_or_else(|| ProviderError::permanent("twilio", "auth_token required"))?;

        let url = format!(
            "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
            account_sid
        );

        let params = [
            ("From", self.config.from.as_str()),
            ("To", req.recipient.as_str()),
            ("Body", req.body.as_str()),
        ];

        let response = self
            .client
            .post(&url)
            .basic_auth(account_sid, Some(auth_token))
            .form(&params)
            .send()
            .await
            .map_err(|e| ProviderError::transport("twilio", e))?;
        let delivery = http_outcome("twilio", response, |json| {
            json.get("sid").and_then(Value::as_str).map(String::from)
        })
        .await?;
        info!(
            "SMS sent via Twilio to {}",
            crate::pii::mask_phone(&req.recipient)
        );
        Ok(delivery)
    }

    async fn send_telnyx(&self, req: &SendRequest) -> SendResult {
        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| ProviderError::permanent("telnyx", "api_key required"))?;

        let mut body = serde_json::json!({
            "from": self.config.from,
            "to": req.recipient,
            "text": req.body,
            "type": "SMS",
        });

        // Optional messaging profile
        if let Some(profile_id) = &self.config.messaging_profile_id {
            body["messaging_profile_id"] = serde_json::Value::String(profile_id.clone());
        }

        let response = self
            .client
            .post("https://api.telnyx.com/v2/messages")
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::transport("telnyx", e))?;
        let delivery = http_outcome("telnyx", response, telnyx_message_id).await?;
        info!(
            "SMS sent via Telnyx to {}",
            crate::pii::mask_phone(&req.recipient)
        );
        Ok(delivery)
    }
}

/// Telnyx wraps the message as `{ "data": { "id": "…" } }`.
pub fn telnyx_message_id(json: &Value) -> Option<String> {
    json.get("data")
        .and_then(|d| d.get("id"))
        .and_then(Value::as_str)
        .map(String::from)
}
