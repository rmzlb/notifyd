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

fn default_max_connections() -> u32 { 10 }

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    #[serde(default = "default_poll_interval")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_batch_size")]
    pub batch_size: i64,
    #[serde(default = "default_max_attempts")]
    pub max_attempts: i32,
}

fn default_poll_interval() -> u64 { 500 }
fn default_batch_size() -> i64 { 50 }
fn default_max_attempts() -> i32 { 3 }

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectorsConfig {
    pub email: Option<EmailConfig>,
    pub sms: Option<SmsConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmailConfig {
    pub provider: String,  // "resend"
    pub api_key: String,
    pub from: String,
    pub from_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SmsConfig {
    pub provider: String,  // "twilio" | "telnyx"
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
pub struct ProjectConfig {
    pub api_key: String,
    pub channels: Vec<String>,
}

impl Config {
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
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
                port: std::env::var("PORT").unwrap_or_else(|_| "3400".to_string()).parse().unwrap_or(3400),
                jwt_secret: std::env::var("JWT_SECRET").unwrap_or_else(|_| "change-me-in-production".to_string()),
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
                email: std::env::var("RESEND_API_KEY").ok().map(|api_key| EmailConfig {
                    provider: "resend".to_string(),
                    api_key,
                    from: std::env::var("EMAIL_FROM").unwrap_or_else(|_| "notifications@example.com".to_string()),
                    from_name: std::env::var("EMAIL_FROM_NAME").ok(),
                }),
                sms: None,
            },
            projects: HashMap::new(),
        };
        Ok(config)
    }
}
