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
}

impl ProviderError {
    pub fn permanent(provider: &'static str, message: impl Into<String>) -> Self {
        Self {
            provider,
            kind: ProviderErrorKind::Permanent,
            message: message.into(),
        }
    }

    pub fn transient(provider: &'static str, message: impl Into<String>) -> Self {
        Self {
            provider,
            kind: ProviderErrorKind::Transient,
            message: message.into(),
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
        }
    }

    pub fn suppressed(reason: impl Into<String>) -> Self {
        Self {
            provider: "suppression-list",
            kind: ProviderErrorKind::Suppressed,
            message: reason.into(),
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
