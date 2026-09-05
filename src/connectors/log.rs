//! Development connector: accepts every message and logs it instead of
//! contacting a provider. Selected with `EMAIL_PROVIDER=log`. It is loud on
//! purpose (one info line per message, `provider="log"` in metrics) so a
//! misconfigured production instance cannot pass for a working one.

use super::{Channel, Connector, Delivery, SendRequest, SendResult};
use async_trait::async_trait;
use tracing::info;

pub struct LogConnector {
    channel: Channel,
}

impl LogConnector {
    pub fn new(channel: Channel) -> Self {
        Self { channel }
    }
}

#[async_trait]
impl Connector for LogConnector {
    fn channel(&self) -> Channel {
        self.channel.clone()
    }

    fn provider(&self) -> &'static str {
        "log"
    }

    async fn send(&self, req: &SendRequest) -> SendResult {
        let id = uuid::Uuid::new_v4().to_string();
        info!(
            provider = "log",
            channel = self.channel.as_str(),
            recipient = %crate::pii::mask_email(&req.recipient),
            subject = req.subject.as_deref().unwrap_or(""),
            message_id = %id,
            "message accepted by the log connector (nothing was sent)"
        );
        Ok(Delivery::new("log", Some(id)))
    }
}
