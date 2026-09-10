//! Apple Push Notification service, natively: HTTP/2 to
//! `api.push.apple.com`, token-based authentication with the `.p8` key
//! (ES256 JWT, refreshed every 50 minutes as Apple asks), the `aps`
//! dictionary built from the job (title, body, badge, sound, thread id,
//! mutable-content, background pushes), `apns-collapse-id`, and the device
//! token lifecycle: `Unregistered` / `BadDeviceToken` mark the recipient dead
//! so the worker drops the token instead of failing every future send.
//!
//! Configuration (env): `APNS_KEY_ID`, `APNS_TEAM_ID`, `APNS_PRIVATE_KEY`
//! (contents of the .p8, `\n` allowed) or `APNS_PRIVATE_KEY_PATH`,
//! `APNS_TOPIC` (the app's bundle id), `APNS_ENVIRONMENT`
//! (`production`, default, or `sandbox`).
//!
//! Job → payload: `subject` is the title, `body` the body. Extras live under
//! `push` in the request: `{"badge": 3, "sound": "default", "thread_id":
//! "orders", "category": "ORDER", "collapse_id": "order-42", "mutable_content":
//! true, "background": true, "ttl_secs": 3600, "data": {...}}`. `url` is
//! forwarded as a custom key.

use super::{Channel, Connector, Delivery, ProviderError, SendRequest, SendResult};
use crate::config::ApnsConfig;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::info;

/// Apple: refresh at most every 20 minutes, at least every 60.
const TOKEN_LIFETIME: Duration = Duration::from_secs(50 * 60);

pub const PRODUCTION_URL: &str = "https://api.push.apple.com";
pub const SANDBOX_URL: &str = "https://api.sandbox.push.apple.com";

#[derive(Serialize)]
struct Claims {
    iss: String,
    iat: u64,
}

pub struct ApnsConnector {
    config: ApnsConfig,
    key: EncodingKey,
    base_url: String,
    client: reqwest::Client,
    token: Mutex<Option<(String, Instant)>>,
}

impl ApnsConnector {
    pub fn new(config: ApnsConfig) -> Result<Self, ProviderError> {
        Self::with_base_url(config, None)
    }

    /// `base_url` override for tests (plain HTTP, h2 prior knowledge).
    pub fn with_base_url(
        config: ApnsConfig,
        base_url: Option<String>,
    ) -> Result<Self, ProviderError> {
        let key = EncodingKey::from_ec_pem(config.private_key_pem.as_bytes()).map_err(|e| {
            ProviderError::permanent(
                "apns",
                format!("APNS_PRIVATE_KEY is not a valid EC PEM key: {e}"),
            )
        })?;
        let base_url = base_url.unwrap_or_else(|| {
            if config.sandbox {
                SANDBOX_URL.to_string()
            } else {
                PRODUCTION_URL.to_string()
            }
        });
        let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(20));
        if base_url.starts_with("http://") {
            builder = builder.http2_prior_knowledge();
        }
        let client = builder
            .build()
            .map_err(|e| ProviderError::permanent("apns", format!("http client: {e}")))?;
        Ok(Self {
            config,
            key,
            base_url,
            client,
            token: Mutex::new(None),
        })
    }

    /// Cached provider token, re-signed after 50 minutes or on demand.
    fn provider_token(&self, force: bool) -> Result<String, ProviderError> {
        let mut guard = self.token.lock().unwrap_or_else(|p| p.into_inner());
        if !force {
            if let Some((token, issued)) = guard.as_ref() {
                if issued.elapsed() < TOKEN_LIFETIME {
                    return Ok(token.clone());
                }
            }
        }
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.config.key_id.clone());
        let claims = Claims {
            iss: self.config.team_id.clone(),
            iat: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        let token = jsonwebtoken::encode(&header, &claims, &self.key).map_err(|e| {
            ProviderError::permanent("apns", format!("cannot sign provider token: {e}"))
        })?;
        *guard = Some((token.clone(), Instant::now()));
        Ok(token)
    }

    async fn post(
        &self,
        req: &SendRequest,
        token: &str,
    ) -> Result<reqwest::Response, ProviderError> {
        let (payload, headers) = build(req, &self.config.topic);
        let mut request = self
            .client
            .post(format!("{}/3/device/{}", self.base_url, req.recipient))
            .header("authorization", format!("bearer {token}"))
            .json(&payload);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        request
            .send()
            .await
            .map_err(|e| ProviderError::transport("apns", e))
    }
}

/// `aps` payload and APNs headers for a job.
pub fn build(req: &SendRequest, topic: &str) -> (Value, Vec<(&'static str, String)>) {
    let extras = req.metadata.get("push").cloned().unwrap_or(Value::Null);
    let background = extras
        .get("background")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut aps = serde_json::Map::new();
    if background {
        aps.insert("content-available".into(), json!(1));
    } else {
        aps.insert(
            "alert".into(),
            json!({
                "title": req.subject.as_deref().unwrap_or("Notification"),
                "body": req.body,
            }),
        );
        let sound = extras
            .get("sound")
            .and_then(Value::as_str)
            .unwrap_or("default");
        if sound != "none" {
            aps.insert("sound".into(), json!(sound));
        }
    }
    if let Some(badge) = extras.get("badge").and_then(Value::as_u64) {
        aps.insert("badge".into(), json!(badge));
    }
    if let Some(thread) = extras.get("thread_id").and_then(Value::as_str) {
        aps.insert("thread-id".into(), json!(thread));
    }
    if let Some(category) = extras.get("category").and_then(Value::as_str) {
        aps.insert("category".into(), json!(category));
    }
    if extras
        .get("mutable_content")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        aps.insert("mutable-content".into(), json!(1));
    }
    let mut payload = json!({ "aps": Value::Object(aps) });
    if let Some(obj) = payload.as_object_mut() {
        if let Some(url) = req.metadata.get("url").and_then(Value::as_str) {
            obj.insert("url".into(), json!(url));
        }
        if let Some(data) = extras.get("data") {
            obj.insert("data".into(), data.clone());
        }
    }

    let mut headers: Vec<(&'static str, String)> = vec![
        ("apns-topic", topic.to_string()),
        (
            "apns-push-type",
            if background { "background" } else { "alert" }.to_string(),
        ),
        (
            "apns-priority",
            if background { "5" } else { "10" }.to_string(),
        ),
    ];
    match extras.get("ttl_secs").and_then(Value::as_u64) {
        Some(ttl) => {
            let exp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                + ttl;
            headers.push(("apns-expiration", exp.to_string()));
        }
        None => headers.push(("apns-expiration", "0".to_string())),
    }
    if let Some(collapse) = extras.get("collapse_id").and_then(Value::as_str) {
        headers.push(("apns-collapse-id", collapse.chars().take(64).collect()));
    }
    (payload, headers)
}

/// Apple's `reason` → what the worker should do.
pub fn classify(status: u16, reason: &str, retry_after: Option<Duration>) -> ProviderError {
    match reason {
        "Unregistered" | "BadDeviceToken" | "DeviceTokenNotForTopic" | "ExpiredToken" => {
            ProviderError::dead_recipient("apns", format!("{status} {reason}"))
        }
        "TooManyRequests" | "TooManyProviderTokenUpdates" => {
            ProviderError::rate_limited("apns", retry_after, format!("{status} {reason}"))
        }
        "InternalServerError"
        | "ServiceUnavailable"
        | "Shutdown"
        | "IdleTimeout"
        | "ExpiredProviderToken"
        | "InvalidProviderToken" => ProviderError::transient("apns", format!("{status} {reason}")),
        _ if status >= 500 => ProviderError::transient("apns", format!("{status} {reason}")),
        _ => ProviderError::permanent("apns", format!("{status} {reason}")),
    }
}

#[async_trait::async_trait]
impl Connector for ApnsConnector {
    fn channel(&self) -> Channel {
        Channel::Push
    }

    fn provider(&self) -> &'static str {
        "apns"
    }

    async fn send(&self, req: &SendRequest) -> SendResult {
        let mut token = self.provider_token(false)?;
        let mut response = self.post(req, &token).await?;
        // A rejected provider token is re-signed once, right away.
        if response.status().as_u16() == 403 {
            let body = response.text().await.unwrap_or_default();
            if body.contains("ProviderToken") {
                token = self.provider_token(true)?;
                response = self.post(req, &token).await?;
            } else {
                let reason = reason_of(&body);
                return Err(classify(403, &reason, None));
            }
        }
        let status = response.status();
        if status.is_success() {
            let id = response
                .headers()
                .get("apns-id")
                .and_then(|v| v.to_str().ok())
                .map(String::from);
            info!(
                "Push sent via APNs to {}",
                crate::pii::mask_recipient("push", &req.recipient)
            );
            return Ok(Delivery::new("apns", id));
        }
        let retry_after = super::parse_retry_after(response.headers());
        let body = response.text().await.unwrap_or_default();
        Err(classify(status.as_u16(), &reason_of(&body), retry_after))
    }
}

fn reason_of(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("reason").and_then(Value::as_str).map(String::from))
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::ProviderErrorKind;

    /// Throw-away P-256 key generated for these tests only.
    const TEST_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgBSBA3f1UzAy+hv5Z
ZsVup7Jjve9sJNA5PJnUUgQkjmmhRANCAAQFaKNnvcJu6GQlOyymgGWTNlXhbMeI
qE7wkgWlbfA2Q5MegIe3BD98KwxLDfAAITCBleTv5NpB45V0CxTBsVCV
-----END PRIVATE KEY-----";

    fn config() -> ApnsConfig {
        ApnsConfig {
            key_id: "ABC123DEFG".into(),
            team_id: "TEAM123456".into(),
            private_key_pem: TEST_KEY.into(),
            topic: "com.example.app".into(),
            sandbox: false,
        }
    }

    fn request(extras: Value) -> SendRequest {
        SendRequest {
            recipient: "0123456789abcdef".into(),
            subject: Some("Order shipped".into()),
            body: "Parcel FR-2041 is on its way".into(),
            body_html: None,
            from_email: None,
            from_name: None,
            metadata: json!({ "url": "https://shop.example.com/orders/42", "push": extras }),
        }
    }

    #[test]
    fn alert_payload_and_headers() {
        let (payload, headers) = build(
            &request(
                json!({"badge": 3, "thread_id": "orders", "collapse_id": "order-42", "mutable_content": true, "data": {"order": 42}}),
            ),
            "com.example.app",
        );
        assert_eq!(payload["aps"]["alert"]["title"], "Order shipped");
        assert_eq!(
            payload["aps"]["alert"]["body"],
            "Parcel FR-2041 is on its way"
        );
        assert_eq!(payload["aps"]["badge"], 3);
        assert_eq!(payload["aps"]["sound"], "default");
        assert_eq!(payload["aps"]["thread-id"], "orders");
        assert_eq!(payload["aps"]["mutable-content"], 1);
        assert_eq!(payload["url"], "https://shop.example.com/orders/42");
        assert_eq!(payload["data"]["order"], 42);
        let h: std::collections::HashMap<_, _> = headers.into_iter().collect();
        assert_eq!(h["apns-topic"], "com.example.app");
        assert_eq!(h["apns-push-type"], "alert");
        assert_eq!(h["apns-priority"], "10");
        assert_eq!(h["apns-collapse-id"], "order-42");
        assert_eq!(h["apns-expiration"], "0");
    }

    #[test]
    fn background_push_has_no_alert_and_low_priority() {
        let (payload, headers) = build(
            &request(json!({"background": true, "ttl_secs": 60})),
            "com.example.app",
        );
        assert_eq!(payload["aps"]["content-available"], 1);
        assert!(payload["aps"].get("alert").is_none());
        assert!(payload["aps"].get("sound").is_none());
        let h: std::collections::HashMap<_, _> = headers.into_iter().collect();
        assert_eq!(h["apns-push-type"], "background");
        assert_eq!(h["apns-priority"], "5");
        assert!(h["apns-expiration"].parse::<u64>().unwrap() > 1_700_000_000);
    }

    #[test]
    fn provider_token_is_an_es256_jwt_with_kid_and_iss() {
        let connector = ApnsConnector::new(config()).unwrap();
        let token = connector.provider_token(false).unwrap();
        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert_eq!(header.alg, Algorithm::ES256);
        assert_eq!(header.kid.as_deref(), Some("ABC123DEFG"));
        let claims: Value = {
            use base64::Engine;
            let part = token.split('.').nth(1).unwrap();
            serde_json::from_slice(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(part)
                    .unwrap(),
            )
            .unwrap()
        };
        assert_eq!(claims["iss"], "TEAM123456");
        assert!(claims["iat"].as_u64().unwrap() > 1_700_000_000);
        assert_eq!(
            connector.provider_token(false).unwrap(),
            token,
            "cached until it ages"
        );
        assert_ne!(connector.provider_token(true).unwrap().len(), 0);
    }

    #[test]
    fn reasons_map_to_dead_transient_or_permanent() {
        let dead = classify(410, "Unregistered", None);
        assert!(dead.dead_recipient && dead.kind == ProviderErrorKind::Permanent);
        assert!(classify(400, "BadDeviceToken", None).dead_recipient);
        assert!(
            !classify(400, "PayloadTooLarge", None).dead_recipient,
            "our bug, keep the token"
        );
        assert_eq!(
            classify(400, "BadTopic", None).kind,
            ProviderErrorKind::Permanent
        );
        assert_eq!(
            classify(503, "ServiceUnavailable", None).kind,
            ProviderErrorKind::Transient
        );
        assert!(matches!(
            classify(429, "TooManyRequests", Some(Duration::from_secs(5))).kind,
            ProviderErrorKind::RateLimited { .. }
        ));
        assert_eq!(
            classify(500, "Whatever", None).kind,
            ProviderErrorKind::Transient
        );
        assert!(classify(400, "MissingTopic", None)
            .message
            .contains("400 MissingTopic"));
    }

    /// A fake APNs over h2 prior knowledge: the connector's request shape,
    /// the success path (apns-id) and the dead-token path, end to end.
    #[tokio::test]
    async fn talks_http2_to_apple_shaped_server() {
        use axum::{extract::Path, http::HeaderMap, routing::post, Json, Router};
        use std::sync::Arc;
        let seen: Arc<Mutex<Vec<(String, HeaderMap, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_in_handler = seen.clone();
        let app = Router::new().route(
            "/3/device/:token",
            post(
                move |Path(token): Path<String>, headers: HeaderMap, Json(body): Json<Value>| {
                    let seen = seen_in_handler.clone();
                    async move {
                        seen.lock().unwrap().push((token.clone(), headers, body));
                        if token == "dead" {
                            (
                                axum::http::StatusCode::GONE,
                                [("apns-id", "aaaa")],
                                r#"{"reason":"Unregistered","timestamp":1700000000000}"#
                                    .to_string(),
                            )
                        } else {
                            (
                                axum::http::StatusCode::OK,
                                [("apns-id", "8f3e-1234-id")],
                                String::new(),
                            )
                        }
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let connector =
            ApnsConnector::with_base_url(config(), Some(format!("http://{addr}"))).unwrap();
        let ok = connector.send(&request(json!({"badge": 1}))).await.unwrap();
        assert_eq!(ok.provider, "apns");
        assert_eq!(ok.provider_message_id.as_deref(), Some("8f3e-1234-id"));

        let mut dead_req = request(Value::Null);
        dead_req.recipient = "dead".into();
        let err = connector.send(&dead_req).await.unwrap_err();
        assert!(
            err.dead_recipient,
            "410 Unregistered marks the token dead: {err:?}"
        );

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        let (token, headers, body) = &seen[0];
        assert_eq!(token, "0123456789abcdef");
        assert!(headers["authorization"]
            .to_str()
            .unwrap()
            .starts_with("bearer eyJ"));
        assert_eq!(headers["apns-topic"], "com.example.app");
        assert_eq!(headers["apns-push-type"], "alert");
        assert_eq!(body["aps"]["badge"], 1);
    }
}
