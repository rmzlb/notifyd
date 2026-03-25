pub mod email;
pub mod sms;
pub mod in_app;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Channel {
    Email,
    Sms,
    InApp,
}

impl Channel {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "email" => Some(Self::Email),
            "sms" => Some(Self::Sms),
            "in_app" | "inapp" => Some(Self::InApp),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Sms => "sms",
            Self::InApp => "in_app",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SendRequest {
    pub recipient: String,   // email / phone / subscriber_id
    pub subject: Option<String>,
    pub body: String,
    pub body_html: Option<String>,
    pub metadata: Value,
}

#[async_trait]
pub trait Connector: Send + Sync {
    fn channel(&self) -> Channel;
    async fn send(&self, req: &SendRequest) -> Result<()>;
}
