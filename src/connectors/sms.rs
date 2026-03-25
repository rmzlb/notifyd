use super::{Channel, Connector, SendRequest};
use crate::config::SmsConfig;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tracing::{info, error};

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
    fn channel(&self) -> Channel { Channel::Sms }

    async fn send(&self, req: &SendRequest) -> Result<()> {
        match self.config.provider.as_str() {
            "twilio" => self.send_twilio(req).await,
            "telnyx" => self.send_telnyx(req).await,
            p => Err(anyhow!("Unknown SMS provider: {}", p)),
        }
    }
}

impl SmsConnector {
    async fn send_twilio(&self, req: &SendRequest) -> Result<()> {
        let account_sid = self.config.account_sid.as_deref()
            .ok_or_else(|| anyhow!("Twilio account_sid required"))?;
        let auth_token = self.config.auth_token.as_deref()
            .ok_or_else(|| anyhow!("Twilio auth_token required"))?;

        let url = format!(
            "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
            account_sid
        );

        let params = [
            ("From", self.config.from.as_str()),
            ("To", req.recipient.as_str()),
            ("Body", req.body.as_str()),
        ];

        let res = self.client
            .post(&url)
            .basic_auth(account_sid, Some(auth_token))
            .form(&params)
            .send()
            .await?;

        if res.status().is_success() {
            info!("SMS sent via Twilio to {}", crate::pii::mask_phone(&req.recipient));
            Ok(())
        } else {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            error!("Twilio error {}: {}", status, text);
            Err(anyhow!("Twilio error {}: {}", status, text))
        }
    }

    async fn send_telnyx(&self, req: &SendRequest) -> Result<()> {
        let api_key = self.config.api_key.as_deref()
            .ok_or_else(|| anyhow!("Telnyx api_key required"))?;

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

        let res = self.client
            .post("https://api.telnyx.com/v2/messages")
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await?;

        if res.status().is_success() {
            info!("SMS sent via Telnyx to {}", crate::pii::mask_phone(&req.recipient));
            Ok(())
        } else {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            error!("Telnyx error {}: {}", status, text);
            Err(anyhow!("Telnyx error {}: {}", status, text))
        }
    }
}
