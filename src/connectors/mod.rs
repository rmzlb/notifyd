pub mod apns;
pub mod chat;
pub mod cloudflare;
pub mod email;
pub mod in_app;
pub mod log;
pub mod push;
pub mod sms;
pub mod smtp;
pub mod whatsapp;

use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Email,
    Sms,
    Whatsapp,
    InApp,
    Push,
    Telegram,
    Slack,
    Discord,
}

impl Channel {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "email" => Some(Self::Email),
            "sms" => Some(Self::Sms),
            "whatsapp" => Some(Self::Whatsapp),
            "in_app" | "inapp" => Some(Self::InApp),
            "push" | "fcm" => Some(Self::Push),
            "telegram" => Some(Self::Telegram),
            "slack" => Some(Self::Slack),
            "discord" => Some(Self::Discord),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Sms => "sms",
            Self::Whatsapp => "whatsapp",
            Self::InApp => "in_app",
            Self::Push => "push",
            Self::Telegram => "telegram",
            Self::Slack => "slack",
            Self::Discord => "discord",
        }
    }

    /// Channels whose recipient is an address to look up on the subscriber
    /// (`email`, `phone`, `data.*`) when the request names no `to`.
    pub fn needs_address(&self) -> bool {
        !matches!(self, Self::InApp | Self::Push)
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

/// Longest display name a request may put on its email (`from_name`).
pub const SENDER_NAME_MAX_CHARS: usize = 100;

/// Per-message sender display name, trimmed; blank means none. Control
/// characters could inject headers and `"<>\` could make the name pose as
/// another address, so both are refused.
pub fn normalize_sender_name(raw: &str) -> Result<Option<String>, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Ok(None);
    }
    if name.chars().count() > SENDER_NAME_MAX_CHARS {
        return Err(format!(
            "from_name cannot exceed {SENDER_NAME_MAX_CHARS} characters"
        ));
    }
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '"' | '<' | '>' | '\\'))
    {
        return Err("from_name cannot contain control characters or \" < > \\".to_string());
    }
    Ok(Some(name.to_string()))
}

/// Address and display name an email connector sends from. A project
/// address keeps the name the worker resolved for it; without one the
/// instance address is used, under the per-message name when there is one.
pub fn sender<'a>(
    req: &'a SendRequest,
    instance_email: &'a str,
    instance_name: Option<&'a str>,
) -> (&'a str, Option<&'a str>) {
    match &req.from_email {
        Some(project_email) => (project_email.as_str(), req.from_name.as_deref()),
        None => (instance_email, req.from_name.as_deref().or(instance_name)),
    }
}

/// What a provider accepted. `provider_message_id` is the provider's own
/// identifier (Resend `id`, Telnyx message id, SMTP `Message-ID`): stored on
/// the job so webhook events and support tickets can be joined to it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Delivery {
    pub provider: &'static str,
    pub provider_message_id: Option<String>,
}

impl Delivery {
    pub fn new(provider: &'static str, provider_message_id: Option<String>) -> Self {
        Self {
            provider,
            provider_message_id,
        }
    }
}

/// Why a send did not happen. The kind drives the worker, not the message:
/// - `RateLimited`: the provider asked us to slow down. The job is
///   re-queued without consuming an attempt and the lane pauses.
/// - `Transient`: network error, 5xx, timeout. Retry with backoff.
/// - `Permanent`: our request is wrong (4xx other than 429, invalid
///   recipient, unverified sender, misconfiguration). Fail now; retrying
///   the same request would give the same answer.
/// - `Suppressed`: recipient is on the suppression list. Fail without
///   contacting the provider.
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderErrorKind {
    RateLimited { retry_after: Option<Duration> },
    Transient,
    Permanent,
    Suppressed,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{provider}: {message}")]
pub struct ProviderError {
    pub provider: &'static str,
    pub kind: ProviderErrorKind,
    pub message: String,
    /// The recipient itself is gone (unregistered device token, dead push
    /// endpoint): the worker drops it. A permanent error without this flag
    /// is a payload or configuration problem and keeps the recipient.
    pub dead_recipient: bool,
}

impl ProviderError {
    pub fn permanent(provider: &'static str, message: impl Into<String>) -> Self {
        Self {
            provider,
            kind: ProviderErrorKind::Permanent,
            message: message.into(),
            dead_recipient: false,
        }
    }

    pub fn transient(provider: &'static str, message: impl Into<String>) -> Self {
        Self {
            provider,
            kind: ProviderErrorKind::Transient,
            message: message.into(),
            dead_recipient: false,
        }
    }

    pub fn rate_limited(
        provider: &'static str,
        retry_after: Option<Duration>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            kind: ProviderErrorKind::RateLimited { retry_after },
            message: message.into(),
            dead_recipient: false,
        }
    }

    /// Permanent error that also says the recipient no longer exists.
    pub fn dead_recipient(provider: &'static str, message: impl Into<String>) -> Self {
        Self {
            provider,
            kind: ProviderErrorKind::Permanent,
            message: message.into(),
            dead_recipient: true,
        }
    }

    pub fn suppressed(reason: impl Into<String>) -> Self {
        Self {
            provider: "suppression-list",
            kind: ProviderErrorKind::Suppressed,
            message: reason.into(),
            dead_recipient: false,
        }
    }

    /// Network-level failure while talking to the provider: always transient.
    pub fn transport(provider: &'static str, error: reqwest::Error) -> Self {
        Self::transient(provider, format!("transport error: {error}"))
    }

    /// Database failure while a connector writes (in-app inbox): an
    /// integrity violation (SQLSTATE class 23, e.g. unknown subscriber) is
    /// permanent, anything else (connection, timeout) is transient.
    pub fn database(provider: &'static str, error: sqlx::Error) -> Self {
        let integrity = error
            .as_database_error()
            .and_then(|db| db.code())
            .map(|code| code.starts_with("23"))
            .unwrap_or(false);
        if integrity {
            Self::permanent(provider, error.to_string())
        } else {
            Self::transient(provider, error.to_string())
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            ProviderErrorKind::RateLimited { .. } => "rate_limited",
            ProviderErrorKind::Transient => "transient",
            ProviderErrorKind::Permanent => "permanent",
            ProviderErrorKind::Suppressed => "suppressed",
        }
    }
}

pub type SendResult = Result<Delivery, ProviderError>;

/// Classify an HTTP status into a provider error kind. `retry_after` is the
/// parsed `Retry-After` header when the provider sent one.
pub fn classify_status(
    provider: &'static str,
    status: reqwest::StatusCode,
    retry_after: Option<Duration>,
    body: &str,
) -> ProviderError {
    let message = format!("HTTP {}: {}", status.as_u16(), truncate(body, 500));
    if status.as_u16() == 429 {
        ProviderError::rate_limited(provider, retry_after, message)
    } else if status.is_server_error() || status.as_u16() == 408 {
        ProviderError::transient(provider, message)
    } else {
        ProviderError::permanent(provider, message)
    }
}

/// `Retry-After` as seconds (RFC 9110 delta-seconds). HTTP-date forms are
/// rare on APIs and ignored: the lane then uses its default pause.
pub fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

/// Turn a provider response into a `SendResult`, reading the message id
/// with `message_id` on success. Shared by every HTTP connector so the
/// classification rules live in one place.
pub async fn http_outcome<F>(
    provider: &'static str,
    response: reqwest::Response,
    message_id: F,
) -> SendResult
where
    F: FnOnce(&Value) -> Option<String>,
{
    let status = response.status();
    let retry_after = parse_retry_after(response.headers());
    let text = response.text().await.unwrap_or_default();
    if status.is_success() {
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        return Ok(Delivery::new(provider, message_id(&parsed)));
    }
    Err(classify_status(provider, status, retry_after, &text))
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let cut: String = text.chars().take(max).collect();
        format!("{cut}…")
    }
}

#[async_trait]
pub trait Connector: Send + Sync {
    fn channel(&self) -> Channel;

    /// Stable provider name used in metrics and on the job row.
    fn provider(&self) -> &'static str;

    async fn send(&self, req: &SendRequest) -> SendResult;

    /// Default batch implementation: sequential `send()` calls.
    /// Connectors that have a native bulk endpoint (e.g. Resend's
    /// `/emails/batch`) override this to coalesce N requests into a
    /// single API call. Returns one result per request, in input order.
    ///
    /// IMPORTANT: do NOT pass more than the connector's batch limit (see
    /// `email::RESEND_BATCH_MAX`). The worker is responsible for chunking
    /// before calling this.
    async fn send_batch(&self, reqs: &[SendRequest]) -> Vec<SendResult> {
        let mut out = Vec::with_capacity(reqs.len());
        for req in reqs {
            out.push(self.send(req).await);
        }
        out
    }

    /// Largest batch the provider accepts in one call; 1 = no native batch.
    fn batch_max(&self) -> usize {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn classifies_429_as_rate_limited_with_retry_after() {
        let err = classify_status(
            "resend",
            StatusCode::TOO_MANY_REQUESTS,
            Some(Duration::from_secs(3)),
            "slow down",
        );
        assert_eq!(
            err.kind,
            ProviderErrorKind::RateLimited {
                retry_after: Some(Duration::from_secs(3))
            }
        );
        assert_eq!(err.kind_label(), "rate_limited");
    }

    #[test]
    fn classifies_5xx_and_408_as_transient() {
        for code in [500u16, 502, 503, 504, 408] {
            let err = classify_status("resend", StatusCode::from_u16(code).unwrap(), None, "");
            assert_eq!(err.kind, ProviderErrorKind::Transient, "status {code}");
        }
    }

    #[test]
    fn classifies_other_4xx_as_permanent() {
        for code in [400u16, 401, 403, 404, 422] {
            let err = classify_status(
                "resend",
                StatusCode::from_u16(code).unwrap(),
                None,
                "{\"message\":\"invalid\"}",
            );
            assert_eq!(err.kind, ProviderErrorKind::Permanent, "status {code}");
        }
    }

    #[test]
    fn parses_delta_seconds_retry_after_only() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "7".parse().unwrap());
        assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(7)));
        headers.insert(
            reqwest::header::RETRY_AFTER,
            "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(parse_retry_after(&headers), None);
    }

    #[test]
    fn truncates_long_provider_bodies() {
        let long = "x".repeat(2000);
        let err = classify_status("resend", StatusCode::BAD_REQUEST, None, &long);
        assert!(err.message.len() < 600);
        assert!(err.message.ends_with('…'));
    }
}

#[cfg(test)]
mod sender_tests {
    use super::*;
    use serde_json::json;

    fn request(from_email: Option<&str>, from_name: Option<&str>) -> SendRequest {
        SendRequest {
            recipient: "patient@example.com".to_string(),
            subject: None,
            body: String::new(),
            body_html: None,
            from_email: from_email.map(String::from),
            from_name: from_name.map(String::from),
            metadata: json!({}),
        }
    }

    #[test]
    fn sender_name_is_trimmed_and_blank_means_none() {
        assert_eq!(
            normalize_sender_name("  Cendre dentaire M. Foch ").unwrap(),
            Some("Cendre dentaire M. Foch".to_string())
        );
        assert_eq!(normalize_sender_name("   ").unwrap(), None);
    }

    #[test]
    fn sender_name_refuses_header_injection_and_address_lookalikes() {
        for raw in [
            "Foch\r\nBcc: victim@example.com",
            "Foch\nX",
            "Foch\u{0}",
            "Foch <evil@example.com>",
            "\"Foch\"",
            "Foch\\",
        ] {
            assert!(normalize_sender_name(raw).is_err(), "accepted {raw:?}");
        }
    }

    #[test]
    fn sender_name_length_counts_characters_not_bytes() {
        assert!(normalize_sender_name(&"é".repeat(SENDER_NAME_MAX_CHARS)).is_ok());
        assert!(normalize_sender_name(&"a".repeat(SENDER_NAME_MAX_CHARS + 1)).is_err());
    }

    #[test]
    fn project_address_keeps_its_resolved_name() {
        let req = request(Some("noreply@sqarex.com"), Some("Centre Foch"));
        assert_eq!(
            sender(&req, "instance@example.com", Some("Instance")),
            ("noreply@sqarex.com", Some("Centre Foch"))
        );
        let bare = request(Some("noreply@sqarex.com"), None);
        assert_eq!(
            sender(&bare, "instance@example.com", Some("Instance")),
            ("noreply@sqarex.com", None)
        );
    }

    #[test]
    fn instance_address_takes_the_per_message_name_or_its_own() {
        let named = request(None, Some("Centre Foch"));
        assert_eq!(
            sender(&named, "instance@example.com", Some("Instance")),
            ("instance@example.com", Some("Centre Foch"))
        );
        let plain = request(None, None);
        assert_eq!(
            sender(&plain, "instance@example.com", Some("Instance")),
            ("instance@example.com", Some("Instance"))
        );
    }
}
