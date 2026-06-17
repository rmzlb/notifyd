use super::{Channel, Connector, SendRequest};
use crate::config::PushConfig;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use base64::Engine;
use serde_json::json;
use tracing::{error, info};
use web_push::{
    ContentEncoding, IsahcWebPushClient, SubscriptionInfo, VapidSignatureBuilder, WebPushClient,
    WebPushMessageBuilder,
};

pub struct PushConnector {
    config: PushConfig,
    client: reqwest::Client,
}

impl PushConnector {
    pub fn new(config: PushConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    pub fn vapid_public_key(config: &PushConfig) -> Result<String> {
        if let Some(public_key) = config
            .vapid_public_key
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return Ok(public_key.to_string());
        }

        let bytes = if let Some(pem) = config
            .vapid_private_key_pem
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            VapidSignatureBuilder::from_pem_no_sub(pem.as_bytes())?.get_public_key()
        } else if let Some(private_key) = config
            .vapid_private_key
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            VapidSignatureBuilder::from_base64_no_sub(private_key)?.get_public_key()
        } else {
            return Err(anyhow!("VAPID private or public key not configured"));
        };

        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
    }

    async fn send_fcm(&self, req: &SendRequest) -> Result<()> {
        let server_key = self
            .config
            .fcm_server_key
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow!("FCM server key not configured"))?;

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
            .header("Authorization", format!("key={}", server_key))
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

    async fn send_web_push(&self, req: &SendRequest) -> Result<()> {
        let web_push = req
            .metadata
            .get("web_push")
            .and_then(|v| v.as_object())
            .ok_or_else(|| anyhow!("Missing web_push metadata"))?;

        let endpoint = web_push
            .get("endpoint")
            .and_then(|v| v.as_str())
            .unwrap_or(req.recipient.as_str());
        let p256dh = web_push
            .get("p256dh")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing web_push.p256dh"))?;
        let auth = web_push
            .get("auth")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing web_push.auth"))?;

        let subscription_info = SubscriptionInfo::new(endpoint, p256dh, auth);
        let mut message_builder = WebPushMessageBuilder::new(&subscription_info);
        let payload = json!({
            "title": req.subject.as_deref().unwrap_or("Notification"),
            "body": req.body,
            "icon": req.metadata.get("icon").and_then(|v| v.as_str()).unwrap_or("/icons/icon-192.png"),
            "badge": req.metadata.get("badge").and_then(|v| v.as_str()).unwrap_or("/icons/icon-192.png"),
            "url": req.metadata.get("url").and_then(|v| v.as_str()).unwrap_or("/"),
            "data": req.metadata,
        });
        let payload_bytes = serde_json::to_vec(&payload)?;
        message_builder.set_payload(ContentEncoding::Aes128Gcm, &payload_bytes);

        let mut signature_builder = if let Some(pem) = self
            .config
            .vapid_private_key_pem
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            VapidSignatureBuilder::from_pem(pem.as_bytes(), &subscription_info)?
        } else if let Some(private_key) = self
            .config
            .vapid_private_key
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            VapidSignatureBuilder::from_base64(private_key, &subscription_info)?
        } else {
            return Err(anyhow!("VAPID private key not configured"));
        };

        if let Some(subject) = self
            .config
            .vapid_subject
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            signature_builder.add_claim("sub", subject);
        }

        message_builder.set_vapid_signature(signature_builder.build()?);
        let client = IsahcWebPushClient::new()?;
        client.send(message_builder.build()?).await?;

        info!(
            "Push sent via Web Push to {}",
            crate::pii::mask_recipient("push", endpoint)
        );
        Ok(())
    }
}

#[async_trait]
impl Connector for PushConnector {
    fn channel(&self) -> Channel {
        Channel::Push
    }

    async fn send(&self, req: &SendRequest) -> Result<()> {
        if req.metadata.get("web_push").is_some() || req.recipient.starts_with("https://") {
            self.send_web_push(req).await
        } else {
            self.send_fcm(req).await
        }
    }
}
