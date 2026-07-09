pub mod email;
pub mod in_app;
pub mod push;
pub mod sms;
pub mod whatsapp;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Channel {
    Email,
    Sms,
    Whatsapp,
    InApp,
    Push,
}

impl Channel {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "email" => Some(Self::Email),
            "sms" => Some(Self::Sms),
            "whatsapp" => Some(Self::Whatsapp),
            "in_app" | "inapp" => Some(Self::InApp),
            "push" | "fcm" => Some(Self::Push),
            _ => None,
        }
    }
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Sms => "sms",
            Self::Whatsapp => "whatsapp",
            Self::InApp => "in_app",
            Self::Push => "push",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SendRequest {
    pub recipient: String,
    pub subject: Option<String>,
    pub body: String,
    pub body_html: Option<String>,
    // Per-project sender override (email channel only). None = connector default.
    pub from_email: Option<String>,
    pub from_name: Option<String>,
    pub metadata: Value,
}

#[async_trait]
#[allow(dead_code)]
pub trait Connector: Send + Sync {
    fn channel(&self) -> Channel;
    async fn send(&self, req: &SendRequest) -> Result<()>;

    /// Default batch implementation: sequential `send()` calls.
    /// Connectors that have a native bulk endpoint (e.g. Resend's
    /// `/emails/batch`) override this to coalesce N requests into a
    /// single API call. Returns one result per request, in input order.
    ///
    /// IMPORTANT: do NOT pass more than the connector's batch limit (see
    /// `email::RESEND_BATCH_MAX`). The worker is responsible for chunking
    /// before calling this.
    async fn send_batch(&self, reqs: &[SendRequest]) -> Vec<Result<()>> {
        let mut out = Vec::with_capacity(reqs.len());
        for req in reqs {
            out.push(self.send(req).await);
        }
        out
    }
}
