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
    /// Outbound pacing per channel, in provider requests per second. A batch
    /// call counts as one request. Zero disables pacing for that channel.
    #[serde(default)]
    pub pacing: PacingConfig,
}

fn default_poll_interval() -> u64 {
    500
}
fn default_batch_size() -> i64 {
    50
}
/// Five attempts with the worker's backoff (30 s, 2 min, 10 min, 30 min)
/// covers a provider incident of about 45 minutes without losing a job.
fn default_max_attempts() -> i32 {
    5
}

#[derive(Debug, Clone, Deserialize)]
pub struct PacingConfig {
    /// Resend allows 10 requests/s per team; 8 leaves room for the
    /// application's own direct calls (webhook provisioning, audiences).
    #[serde(default = "default_email_per_sec")]
    pub email_per_sec: f64,
    #[serde(default = "default_sms_per_sec")]
    pub sms_per_sec: f64,
    #[serde(default = "default_sms_per_sec")]
    pub whatsapp_per_sec: f64,
    #[serde(default = "default_push_per_sec")]
    pub push_per_sec: f64,
    /// Lane pause after a 429 without `Retry-After`, in seconds.
    #[serde(default = "default_rate_limit_pause_secs")]
    pub rate_limit_pause_secs: u64,
}

impl Default for PacingConfig {
    fn default() -> Self {
        Self {
            email_per_sec: default_email_per_sec(),
            sms_per_sec: default_sms_per_sec(),
            whatsapp_per_sec: default_sms_per_sec(),
            push_per_sec: default_push_per_sec(),
            rate_limit_pause_secs: default_rate_limit_pause_secs(),
        }
    }
}

fn default_email_per_sec() -> f64 {
    8.0
}
fn default_sms_per_sec() -> f64 {
    10.0
}
fn default_push_per_sec() -> f64 {
    50.0
}
fn default_rate_limit_pause_secs() -> u64 {
    2
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectorsConfig {
    pub email: Option<EmailConfig>,
    pub sms: Option<SmsConfig>,
    #[serde(default)]
    pub whatsapp: Option<WhatsappConfig>,
    pub push: Option<PushConfig>,
}

/// Email provider. `provider` selects the connector:
/// - `resend`     : `api_key` = Resend key
/// - `agentmail`  : `api_key` = AgentMail token, `from` = inbox address
/// - `cloudflare` : `api_key` = Cloudflare API token with Email Sending
///                  permission, `account_id` = Cloudflare account
/// - `smtp`       : `smtp` block (any SMTP submission service)
/// - `log`        : nothing is sent; every message is logged as accepted.
///                  For development and previews only.
#[derive(Debug, Clone, Deserialize)]
pub struct EmailConfig {
    pub provider: String,
    #[serde(default)]
    pub api_key: String,
    pub from: String,
    pub from_name: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub smtp: Option<SmtpConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    #[serde(default = "default_smtp_port")]
    pub port: u16,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    /// `starttls` (port 587, default), `tls` (implicit TLS, port 465) or
    /// `none` (plain text, local relays only).
    #[serde(default = "default_smtp_security")]
    pub security: String,
}

fn default_smtp_port() -> u16 {
    587
}
fn default_smtp_security() -> String {
    "starttls".to_string()
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

impl SmsConfig {
    /// `SMS_PROVIDER=telnyx` with `TELNYX_API_KEY` (+ optional
    /// `TELNYX_MESSAGING_PROFILE_ID`), or `SMS_PROVIDER=twilio` with
    /// `TWILIO_ACCOUNT_SID` and `TWILIO_AUTH_TOKEN`. Both need `SMS_FROM`.
    pub fn from_env() -> Option<Self> {
        let provider = env_non_empty("SMS_PROVIDER")?;
        let from = env_non_empty("SMS_FROM")?;
        match provider.as_str() {
            "telnyx" => Some(Self {
                provider,
                account_sid: None,
                auth_token: None,
                api_key: Some(env_non_empty("TELNYX_API_KEY")?),
                messaging_profile_id: env_non_empty("TELNYX_MESSAGING_PROFILE_ID"),
                from,
            }),
            "twilio" => Some(Self {
                provider,
                account_sid: Some(env_non_empty("TWILIO_ACCOUNT_SID")?),
                auth_token: Some(env_non_empty("TWILIO_AUTH_TOKEN")?),
                api_key: None,
                messaging_profile_id: None,
                from,
            }),
            _ => None,
        }
    }
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
        let api_key =
            env_non_empty("TELNYX_WHATSAPP_API_KEY").or_else(|| env_non_empty("TELNYX_API_KEY"))?;
        let from = env_non_empty("WHATSAPP_FROM")?;
        Some(Self {
            provider: "telnyx".to_string(),
            api_key: Some(api_key),
            messaging_profile_id: env_non_empty("TELNYX_MESSAGING_PROFILE_ID"),
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

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_parse<T: std::str::FromStr>(name: &str, default: T) -> T {
    env_non_empty(name)
        .and_then(|value| value.parse::<T>().ok())
        .unwrap_or(default)
}

impl EmailConfig {
    /// `EMAIL_PROVIDER` selects the connector. When unset, `RESEND_API_KEY`
    /// alone still means Resend, so existing deployments keep working.
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let from =
            env_non_empty("EMAIL_FROM").unwrap_or_else(|| "notifications@example.com".to_string());
        let from_name = env_non_empty("EMAIL_FROM_NAME");
        let provider = match env_non_empty("EMAIL_PROVIDER") {
            Some(p) => p,
            None => {
                if env_non_empty("RESEND_API_KEY").is_some() {
                    "resend".to_string()
                } else {
                    return Ok(None);
                }
            }
        };
        let config = match provider.as_str() {
            "resend" => Self {
                provider,
                api_key: required("RESEND_API_KEY", "EMAIL_PROVIDER=resend")?,
                from,
                from_name,
                account_id: None,
                smtp: None,
            },
            "agentmail" => Self {
                provider,
                api_key: required("AGENTMAIL_API_KEY", "EMAIL_PROVIDER=agentmail")?,
                from,
                from_name,
                account_id: None,
                smtp: None,
            },
            "cloudflare" => Self {
                provider,
                api_key: required("CLOUDFLARE_EMAIL_API_TOKEN", "EMAIL_PROVIDER=cloudflare")?,
                from,
                from_name,
                account_id: Some(required(
                    "CLOUDFLARE_ACCOUNT_ID",
                    "EMAIL_PROVIDER=cloudflare",
                )?),
                smtp: None,
            },
            "smtp" => Self {
                provider,
                api_key: String::new(),
                from,
                from_name,
                account_id: None,
                smtp: Some(SmtpConfig {
                    host: required("SMTP_HOST", "EMAIL_PROVIDER=smtp")?,
                    port: env_parse("SMTP_PORT", default_smtp_port()),
                    username: env_non_empty("SMTP_USERNAME"),
                    password: env_non_empty("SMTP_PASSWORD"),
                    security: env_non_empty("SMTP_SECURITY").unwrap_or_else(default_smtp_security),
                }),
            },
            "log" => Self {
                provider,
                api_key: String::new(),
                from,
                from_name,
                account_id: None,
                smtp: None,
            },
            other => anyhow::bail!(
                "EMAIL_PROVIDER={other} is not supported (resend, agentmail, cloudflare, smtp, log)"
            ),
        };
        Ok(Some(config))
    }
}

fn required(name: &str, context: &str) -> anyhow::Result<String> {
    env_non_empty(name).ok_or_else(|| anyhow::anyhow!("{name} is required with {context}"))
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
        if config.connectors.sms.is_none() {
            config.connectors.sms = SmsConfig::from_env();
        }
        Ok(config)
    }

    pub fn from_env() -> anyhow::Result<Self> {
        // Try file first, then env vars
        let path = std::env::var("NOTIFYD_CONFIG").unwrap_or_else(|_| "notifyd.toml".to_string());
        if std::path::Path::new(&path).exists() {
            return Self::from_file(&path);
        }
        let config = Config {
            server: ServerConfig {
                port: env_parse("PORT", 3400),
                jwt_secret: std::env::var("JWT_SECRET")
                    .unwrap_or_else(|_| "change-me-in-production".to_string()),
            },
            database: DatabaseConfig {
                url: std::env::var("DATABASE_URL").expect("DATABASE_URL required"),
                max_connections: env_parse("DATABASE_MAX_CONNECTIONS", default_max_connections()),
            },
            worker: WorkerConfig {
                poll_interval_ms: env_parse("WORKER_POLL_INTERVAL_MS", default_poll_interval()),
                batch_size: env_parse("WORKER_BATCH_SIZE", default_batch_size()),
                max_attempts: env_parse("WORKER_MAX_ATTEMPTS", default_max_attempts()),
                pacing: PacingConfig {
                    email_per_sec: env_parse("EMAIL_RATE_PER_SEC", default_email_per_sec()),
                    sms_per_sec: env_parse("SMS_RATE_PER_SEC", default_sms_per_sec()),
                    whatsapp_per_sec: env_parse("WHATSAPP_RATE_PER_SEC", default_sms_per_sec()),
                    push_per_sec: env_parse("PUSH_RATE_PER_SEC", default_push_per_sec()),
                    rate_limit_pause_secs: env_parse(
                        "RATE_LIMIT_PAUSE_SECS",
                        default_rate_limit_pause_secs(),
                    ),
                },
            },
            connectors: ConnectorsConfig {
                email: EmailConfig::from_env()?,
                sms: SmsConfig::from_env(),
                whatsapp: WhatsappConfig::from_env(),
                push: PushConfig::from_env(),
            },
            projects: HashMap::new(),
        };
        Ok(config)
    }
}
