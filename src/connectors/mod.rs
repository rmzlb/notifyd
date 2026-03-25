pub mod email;
pub mod sms;
pub mod in_app;
pub mod push;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Channel {
    Email,
    Sms,
    InApp,
    Push,
}

impl Channel {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "email" => Some(Self::Email),
            "sms" => Some(Self::Sms),
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
    pub metadata: Value,
}

#[async_trait]
#[allow(dead_code)]
pub trait Connector: Send + Sync {
    fn channel(&self) -> Channel;
    async fn send(&self, req: &SendRequest) -> Result<()>;
}
