use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub worker: WorkerConfig,
    pub connectors: ConnectorsConfig,
    #[serde(default)]
    pub projects: HashMap<String, ProjectConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub port: u16,
    pub jwt_secret: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
}

fn default_max_connections() -> u32 {
    10
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    #[serde(default = "default_poll_interval")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_batch_size")]
    pub batch_size: i64,
    #[serde(default = "default_max_attempts")]
    pub max_attempts: i32,
}

fn default_poll_interval() -> u64 {
    500
}
fn default_batch_size() -> i64 {
    50
}
fn default_max_attempts() -> i32 {
    3
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectorsConfig {
    pub email: Option<EmailConfig>,
    pub sms: Option<SmsConfig>,
    #[serde(default)]
    pub whatsapp: Option<WhatsappConfig>,
    pub push: Option<PushConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmailConfig {
    pub provider: String, // "resend"
    pub api_key: String,
    pub from: String,
    pub from_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SmsConfig {
    pub provider: String, // "twilio" | "telnyx"
    // Twilio
    pub account_sid: Option<String>,
    pub auth_token: Option<String>,
    // Telnyx
    pub api_key: Option<String>,
    pub messaging_profile_id: Option<String>,
    // Common
    pub from: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WhatsappConfig {
    pub provider: String, // "telnyx"
    pub api_key: Option<String>,
    pub messaging_profile_id: Option<String>,
    pub from: String, // WhatsApp-enabled E.164 number
}

impl WhatsappConfig {
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("TELNYX_WHATSAPP_API_KEY")
            .ok()
            .or_else(|| std::env::var("TELNYX_API_KEY").ok())?;
        let from = std::env::var("WHATSAPP_FROM").ok()?;
        if api_key.trim().is_empty() || from.trim().is_empty() {
            return None;
        }
        Some(Self {
            provider: "telnyx".to_string(),
            api_key: Some(api_key),
            messaging_profile_id: std::env::var("TELNYX_MESSAGING_PROFILE_ID").ok(),
            from,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PushConfig {
    pub fcm_server_key: Option<String>,
    /// URL-safe base64-encoded raw VAPID private key, without padding.
    pub vapid_private_key: Option<String>,
    /// PEM-encoded VAPID private key. Env values may use literal "\n".
    pub vapid_private_key_pem: Option<String>,
    /// Optional precomputed URL-safe base64 public key. If omitted, notifyd
    /// derives it from the private key when the public-key endpoint is called.
    pub vapid_public_key: Option<String>,
    /// VAPID subject, usually "mailto:ops@example.com" or an HTTPS contact URL.
    pub vapid_subject: Option<String>,
}

impl PushConfig {
    pub fn from_env() -> Option<Self> {
        let fcm_server_key = std::env::var("FCM_SERVER_KEY").ok();
        let vapid_private_key = std::env::var("VAPID_PRIVATE_KEY").ok();
        let vapid_private_key_pem = std::env::var("VAPID_PRIVATE_KEY_PEM")
            .ok()
            .map(|v| v.replace("\\n", "\n"));
        let vapid_public_key = std::env::var("VAPID_PUBLIC_KEY").ok();
        let vapid_subject = std::env::var("VAPID_SUBJECT").ok();

        if fcm_server_key.is_none()
            && vapid_private_key.is_none()
            && vapid_private_key_pem.is_none()
            && vapid_public_key.is_none()
        {
            return None;
        }

        Some(Self {
            fcm_server_key,
            vapid_private_key,
            vapid_private_key_pem,
            vapid_public_key,
            vapid_subject,
        })
    }

    pub fn has_web_push_private_key(&self) -> bool {
        self.vapid_private_key
            .as_deref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
            || self
                .vapid_private_key_pem
                .as_deref()
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectConfig {
    pub api_key: String,
    pub channels: Vec<String>,
    // Per-project sender identity (email channel). None = instance default.
    #[serde(default)]
    pub from_email: Option<String>,
    #[serde(default)]
    pub from_name: Option<String>,
}

impl Config {
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&content)?;
        if config.connectors.push.is_none() {
            config.connectors.push = PushConfig::from_env();
        }
        if config.connectors.whatsapp.is_none() {
            config.connectors.whatsapp = WhatsappConfig::from_env();
        }
        Ok(config)
    }

    pub fn from_env() -> anyhow::Result<Self> {
        // Try file first, then env vars
        let path = std::env::var("NOTIFYD_CONFIG").unwrap_or_else(|_| "notifyd.toml".to_string());
        if std::path::Path::new(&path).exists() {
            return Self::from_file(&path);
        }
        // Minimal env-based config
        let config = Config {
            server: ServerConfig {
                port: std::env::var("PORT")
                    .unwrap_or_else(|_| "3400".to_string())
                    .parse()
                    .unwrap_or(3400),
                jwt_secret: std::env::var("JWT_SECRET")
                    .unwrap_or_else(|_| "change-me-in-production".to_string()),
            },
            database: DatabaseConfig {
                url: std::env::var("DATABASE_URL").expect("DATABASE_URL required"),
                max_connections: 10,
            },
            worker: WorkerConfig {
                poll_interval_ms: 500,
                batch_size: 50,
                max_attempts: 3,
            },
            connectors: ConnectorsConfig {
                email: std::env::var("RESEND_API_KEY")
                    .ok()
                    .map(|api_key| EmailConfig {
                        provider: "resend".to_string(),
                        api_key,
                        from: std::env::var("EMAIL_FROM")
                            .unwrap_or_else(|_| "notifications@example.com".to_string()),
                        from_name: std::env::var("EMAIL_FROM_NAME").ok(),
                    }),
                sms: None,
                whatsapp: WhatsappConfig::from_env(),
                push: PushConfig::from_env(),
            },
            projects: HashMap::new(),
        };
        Ok(config)
    }
}
