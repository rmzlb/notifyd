use super::{Channel, Connector, SendRequest};
use crate::config::EmailConfig;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::json;
use tracing::{error, info};

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
}

#[async_trait]
impl Connector for ResendConnector {
    fn channel(&self) -> Channel {
        Channel::Email
    }

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

        let body = json!({
            "to": [req.recipient],
            "subject": req.subject.as_deref().unwrap_or("Notification"),
            "text": req.body,
            "html": req.body_html.as_deref().unwrap_or(&req.body),
        });

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
