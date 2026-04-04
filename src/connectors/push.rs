use super::{Channel, Connector, SendRequest};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::json;
use tracing::{error, info};

pub struct FcmConfig {
    pub server_key: String,
}

pub struct PushConnector {
    config: FcmConfig,
    client: reqwest::Client,
}

impl PushConnector {
    pub fn new(config: FcmConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl Connector for PushConnector {
    fn channel(&self) -> Channel {
        Channel::Push
    }

    async fn send(&self, req: &SendRequest) -> Result<()> {
        // req.recipient is the FCM device token
        let body = json!({
            "to": req.recipient,
            "notification": {
                "title": req.subject.as_deref().unwrap_or("Notification"),
                "body": req.body,
            },
            "data": req.metadata,
        });

        let res = self
            .client
            .post("https://fcm.googleapis.com/fcm/send")
            .header("Authorization", format!("key={}", self.config.server_key))
            .json(&body)
            .send()
            .await?;

        if res.status().is_success() {
            info!(
                "Push sent via FCM to {}",
                crate::pii::mask_recipient("push", &req.recipient)
            );
            Ok(())
        } else {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            error!("FCM error {}: {}", status, text);
            Err(anyhow!("FCM error {}: {}", status, text))
        }
    }
}
