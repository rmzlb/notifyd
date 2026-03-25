use super::{Channel, Connector, SendRequest};
use crate::config::EmailConfig;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::json;
use tracing::{info, error};

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
}

#[async_trait]
impl Connector for ResendConnector {
    fn channel(&self) -> Channel { Channel::Email }

    async fn send(&self, req: &SendRequest) -> Result<()> {
        let from = if let Some(name) = &self.config.from_name {
            format!("{} <{}>", name, self.config.from)
        } else {
            self.config.from.clone()
        };

        let body = json!({
            "from": from,
            "to": [req.recipient],
            "subject": req.subject.as_deref().unwrap_or("Notification"),
            "html": req.body_html.as_deref().unwrap_or(&req.body),
            "text": req.body,
        });

        let res = self.client
            .post("https://api.resend.com/emails")
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await?;

        if res.status().is_success() {
            info!("Email sent via Resend to {}", req.recipient);
            Ok(())
        } else {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            error!("Resend error {}: {}", status, text);
            Err(anyhow!("Resend error {}: {}", status, text))
        }
    }
}
