//! Chat channels: Telegram, Slack and Discord. Three thin connectors over
//! one shape: a text built from `subject` + `body` (+ `url`), a recipient
//! that is a chat id, a channel id or a webhook URL, and the same lifecycle
//! as every other channel (preferences, topics, priorities, pacing, retries,
//! `dead_recipient` when the destination is gone for good).
//!
//! | channel    | recipient (`to` or subscriber `data.*`)                  | server config          |
//! |------------|-----------------------------------------------------------|------------------------|
//! | `telegram` | chat id (`data.telegram_chat_id`)                         | `TELEGRAM_BOT_TOKEN`   |
//! | `slack`    | incoming webhook URL, or channel id (`data.slack`)        | `SLACK_BOT_TOKEN` for channel ids |
//! | `discord`  | webhook URL (`data.discord_webhook`)                      | none                   |
//!
//! Formatting: Telegram gets plain text (no parse mode, so any content is
//! safe) with an inline "Open" button when the job has a `url`; Slack gets
//! mrkdwn (`*subject*`); Discord gets an embed (title, description, url) plus
//! a short `content`. `chat.text` in the request replaces the built text.

use super::{
    parse_retry_after, Channel, Connector, Delivery, ProviderError, SendRequest, SendResult,
};
use crate::config::ChatConfig;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::info;

pub const TELEGRAM_API: &str = "https://api.telegram.org";
pub const SLACK_API: &str = "https://slack.com/api";

/// Plain text every chat channel can show: subject line, blank line, body.
pub fn plain_text(req: &SendRequest) -> String {
    if let Some(t) = req
        .metadata
        .get("chat")
        .and_then(|c| c.get("text"))
        .and_then(Value::as_str)
    {
        return t.to_string();
    }
    match req.subject.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(subject) if !req.body.trim().is_empty() => format!("{subject}\n\n{}", req.body),
        Some(subject) => subject.to_string(),
        None => req.body.clone(),
    }
}

fn url_of(req: &SendRequest) -> Option<&str> {
    req.metadata
        .get("url")
        .and_then(Value::as_str)
        .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
}

pub struct ChatConnector {
    channel: Channel,
    config: ChatConfig,
    client: reqwest::Client,
    telegram_base: String,
    slack_base: String,
}

impl ChatConnector {
    pub fn new(channel: Channel, config: ChatConfig) -> Self {
        let telegram = config
            .telegram_api_base
            .clone()
            .map(|b| b.trim_end_matches('/').to_string())
            .unwrap_or_else(|| TELEGRAM_API.to_string());
        let slack = config
            .slack_api_base
            .clone()
            .map(|b| b.trim_end_matches('/').to_string())
            .unwrap_or_else(|| SLACK_API.to_string());
        Self::with_bases(channel, config, telegram, slack)
    }

    /// Base URLs override for tests.
    pub fn with_bases(
        channel: Channel,
        config: ChatConfig,
        telegram_base: String,
        slack_base: String,
    ) -> Self {
        Self {
            channel,
            config,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("http client"),
            telegram_base,
            slack_base,
        }
    }

    // ── Telegram ────────────────────────────────────────────────────────────
    async fn send_telegram(&self, req: &SendRequest) -> SendResult {
        let token = self
            .config
            .telegram_bot_token
            .as_deref()
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| {
                ProviderError::permanent("telegram", "TELEGRAM_BOT_TOKEN not configured")
            })?;
        let mut body = json!({
            "chat_id": req.recipient,
            "text": plain_text(req),
            "disable_web_page_preview": true,
        });
        if let Some(url) = url_of(req) {
            let label = req
                .metadata
                .get("chat")
                .and_then(|c| c.get("button"))
                .and_then(Value::as_str)
                .unwrap_or("Open");
            body["reply_markup"] = json!({ "inline_keyboard": [[{ "text": label, "url": url }]] });
        }
        let response = self
            .client
            .post(format!("{}/bot{}/sendMessage", self.telegram_base, token))
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::transport("telegram", e))?;
        let status = response.status().as_u16();
        let retry_after = parse_retry_after(response.headers());
        let text = response.text().await.unwrap_or_default();
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if (200..300).contains(&status) && parsed.get("ok").and_then(Value::as_bool) == Some(true) {
            let id = parsed.pointer("/result/message_id").map(|v| v.to_string());
            info!(
                "Chat sent via Telegram to chat {}",
                crate::pii::mask_recipient("telegram", &req.recipient)
            );
            return Ok(Delivery::new("telegram", id));
        }
        let description = parsed
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let retry_after = parsed
            .pointer("/parameters/retry_after")
            .and_then(Value::as_u64)
            .map(Duration::from_secs)
            .or(retry_after);
        Err(classify_telegram(status, &description, retry_after))
    }

    // ── Slack ────────────────────────────────────────────────────────────────
    async fn send_slack(&self, req: &SendRequest) -> SendResult {
        let text = slack_mrkdwn(req);
        if req.recipient.starts_with("https://hooks.slack.com/") {
            let response = self
                .client
                .post(&req.recipient)
                .json(&json!({ "text": text }))
                .send()
                .await
                .map_err(|e| ProviderError::transport("slack", e))?;
            let status = response.status().as_u16();
            let retry_after = parse_retry_after(response.headers());
            let body = response.text().await.unwrap_or_default();
            if (200..300).contains(&status) {
                info!("Chat sent via Slack webhook");
                return Ok(Delivery::new("slack", None));
            }
            // Webhook errors are plain text: "no_service", "channel_is_archived", "invalid_token"…
            return Err(classify_slack(status, body.trim(), retry_after));
        }
        let token = self
            .config
            .slack_bot_token
            .as_deref()
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| {
                ProviderError::permanent("slack", "SLACK_BOT_TOKEN not configured (needed for channel ids; webhook URLs need none)")
            })?;
        let response = self
            .client
            .post(format!("{}/chat.postMessage", self.slack_base))
            .bearer_auth(token)
            .json(&json!({ "channel": req.recipient, "text": text, "unfurl_links": false }))
            .send()
            .await
            .map_err(|e| ProviderError::transport("slack", e))?;
        let status = response.status().as_u16();
        let retry_after = parse_retry_after(response.headers());
        let body = response.text().await.unwrap_or_default();
        let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        if (200..300).contains(&status) && parsed.get("ok").and_then(Value::as_bool) == Some(true) {
            let id = parsed.get("ts").and_then(Value::as_str).map(String::from);
            info!("Chat sent via Slack to {}", req.recipient);
            return Ok(Delivery::new("slack", id));
        }
        let error = parsed
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or(body.trim());
        Err(classify_slack(status, error, retry_after))
    }

    // ── Discord ──────────────────────────────────────────────────────────────
    async fn send_discord(&self, req: &SendRequest) -> SendResult {
        if !req
            .recipient
            .starts_with("https://discord.com/api/webhooks/")
            && !req
                .recipient
                .starts_with("https://discordapp.com/api/webhooks/")
        {
            return Err(ProviderError::dead_recipient(
                "discord",
                "recipient must be a Discord webhook URL",
            ));
        }
        let mut embed = json!({ "description": req.body.chars().take(4096).collect::<String>() });
        if let Some(subject) = req.subject.as_deref().filter(|s| !s.trim().is_empty()) {
            embed["title"] = json!(subject.chars().take(256).collect::<String>());
        }
        if let Some(url) = url_of(req) {
            embed["url"] = json!(url);
        }
        let body = match req
            .metadata
            .get("chat")
            .and_then(|c| c.get("text"))
            .and_then(Value::as_str)
        {
            Some(text) => json!({ "content": text.chars().take(2000).collect::<String>() }),
            None => json!({ "embeds": [embed] }),
        };
        let response = self
            .client
            .post(format!("{}?wait=true", req.recipient))
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::transport("discord", e))?;
        let status = response.status().as_u16();
        let retry_after = parse_retry_after(response.headers());
        let text = response.text().await.unwrap_or_default();
        if (200..300).contains(&status) {
            let id = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("id").and_then(Value::as_str).map(String::from));
            info!("Chat sent via Discord webhook");
            return Ok(Delivery::new("discord", id));
        }
        let retry_after = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("retry_after").and_then(Value::as_f64))
            .map(|s| Duration::from_secs_f64(s.max(1.0)))
            .or(retry_after);
        Err(classify_discord(status, text.trim(), retry_after))
    }
}

/// `*subject*` then the body; the URL on its own line so Slack links it.
pub fn slack_mrkdwn(req: &SendRequest) -> String {
    if let Some(t) = req
        .metadata
        .get("chat")
        .and_then(|c| c.get("text"))
        .and_then(Value::as_str)
    {
        return t.to_string();
    }
    let mut out = String::new();
    if let Some(subject) = req.subject.as_deref().filter(|s| !s.trim().is_empty()) {
        out.push_str(&format!("*{}*\n", subject.replace('*', "")));
    }
    out.push_str(&req.body);
    if let Some(url) = url_of(req) {
        out.push_str(&format!("\n<{url}|Open>"));
    }
    out
}

pub fn classify_telegram(
    status: u16,
    description: &str,
    retry_after: Option<Duration>,
) -> ProviderError {
    let msg = format!("HTTP {status}: {description}");
    let d = description.to_ascii_lowercase();
    if status == 429 {
        return ProviderError::rate_limited("telegram", retry_after, msg);
    }
    if d.contains("chat not found")
        || d.contains("bot was blocked by the user")
        || d.contains("user is deactivated")
        || d.contains("bot was kicked")
        || d.contains("bot is not a member")
        || d.contains("chat_id is empty")
        || d.contains("have no rights to send")
    {
        return ProviderError::dead_recipient("telegram", msg);
    }
    if status >= 500 {
        return ProviderError::transient("telegram", msg);
    }
    ProviderError::permanent("telegram", msg)
}

pub fn classify_slack(status: u16, error: &str, retry_after: Option<Duration>) -> ProviderError {
    let msg = format!("HTTP {status}: {error}");
    match error {
        _ if status == 429 || error == "ratelimited" => {
            ProviderError::rate_limited("slack", retry_after, msg)
        }
        "channel_not_found"
        | "channel_is_archived"
        | "is_archived"
        | "not_in_channel"
        | "user_not_found"
        | "no_service"
        | "no_service_id"
        | "no_team"
        | "team_disabled"
        | "channel_disabled" => ProviderError::dead_recipient("slack", msg),
        _ if status == 404 || status == 410 => ProviderError::dead_recipient("slack", msg),
        _ if status >= 500 || error == "service_unavailable" || error == "internal_error" => {
            ProviderError::transient("slack", msg)
        }
        _ => ProviderError::permanent("slack", msg),
    }
}

pub fn classify_discord(status: u16, body: &str, retry_after: Option<Duration>) -> ProviderError {
    let msg = format!("HTTP {status}: {body}");
    match status {
        429 => ProviderError::rate_limited("discord", retry_after, msg),
        404 | 410 => ProviderError::dead_recipient("discord", msg),
        401 | 403 => ProviderError::dead_recipient("discord", msg),
        s if s >= 500 => ProviderError::transient("discord", msg),
        _ => ProviderError::permanent("discord", msg),
    }
}

#[async_trait]
impl Connector for ChatConnector {
    fn channel(&self) -> Channel {
        self.channel
    }

    fn provider(&self) -> &'static str {
        match self.channel {
            Channel::Telegram => "telegram",
            Channel::Slack => "slack",
            Channel::Discord => "discord",
            _ => "chat",
        }
    }

    async fn send(&self, req: &SendRequest) -> SendResult {
        match self.channel {
            Channel::Telegram => self.send_telegram(req).await,
            Channel::Slack => self.send_slack(req).await,
            Channel::Discord => self.send_discord(req).await,
            other => Err(ProviderError::permanent(
                "chat",
                format!("{} is not a chat channel", other.as_str()),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::ProviderErrorKind;

    fn req(recipient: &str, url: Option<&str>) -> SendRequest {
        SendRequest {
            recipient: recipient.into(),
            subject: Some("Order shipped".into()),
            body: "Parcel FR-2041 is on its way.".into(),
            body_html: None,
            from_email: None,
            from_name: None,
            metadata: match url {
                Some(u) => json!({ "url": u }),
                None => json!({}),
            },
        }
    }

    fn cfg() -> ChatConfig {
        ChatConfig {
            telegram_bot_token: Some("123:ABC".into()),
            slack_bot_token: Some("xoxb-test".into()),
            telegram_api_base: None,
            slack_api_base: None,
        }
    }

    #[test]
    fn texts() {
        let r = req("42", Some("https://shop.example.com/o/1"));
        assert_eq!(
            plain_text(&r),
            "Order shipped\n\nParcel FR-2041 is on its way."
        );
        assert_eq!(
            slack_mrkdwn(&r),
            "*Order shipped*\nParcel FR-2041 is on its way.\n<https://shop.example.com/o/1|Open>"
        );
        let mut custom = req("42", None);
        custom.metadata = json!({ "chat": { "text": "exact text" } });
        assert_eq!(plain_text(&custom), "exact text");
        assert_eq!(slack_mrkdwn(&custom), "exact text");
    }

    #[test]
    fn classifications() {
        assert!(
            classify_telegram(403, "Forbidden: bot was blocked by the user", None).dead_recipient
        );
        assert!(classify_telegram(400, "Bad Request: chat not found", None).dead_recipient);
        assert!(matches!(
            classify_telegram(429, "Too Many Requests", Some(Duration::from_secs(3))).kind,
            ProviderErrorKind::RateLimited { .. }
        ));
        assert_eq!(
            classify_telegram(400, "Bad Request: message is too long", None).kind,
            ProviderErrorKind::Permanent
        );
        assert!(!classify_telegram(400, "Bad Request: message is too long", None).dead_recipient);
        assert!(classify_slack(200, "channel_not_found", None).dead_recipient);
        assert!(classify_slack(404, "no_service", None).dead_recipient);
        assert_eq!(
            classify_slack(200, "invalid_auth", None).kind,
            ProviderErrorKind::Permanent
        );
        assert!(matches!(
            classify_slack(429, "ratelimited", None).kind,
            ProviderErrorKind::RateLimited { .. }
        ));
        assert!(classify_discord(404, r#"{"message":"Unknown Webhook"}"#, None).dead_recipient);
        assert_eq!(
            classify_discord(502, "", None).kind,
            ProviderErrorKind::Transient
        );
    }

    /// Fake Telegram and Slack servers: request shapes, ids, dead recipients.
    #[tokio::test]
    async fn talks_to_telegram_and_slack_shaped_servers() {
        use axum::{extract::Path, http::HeaderMap, routing::post, Json, Router};
        use std::sync::{Arc, Mutex};
        let seen: Arc<Mutex<Vec<(String, HeaderMap, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let s1 = seen.clone();
        let s2 = seen.clone();
        let s3 = seen.clone();
        let app = Router::new()
            .route(
                "/bot:token/sendMessage",
                post(move |Path(token): Path<String>, headers: HeaderMap, Json(body): Json<Value>| {
                    let seen = s1.clone();
                    async move {
                        seen.lock().unwrap().push((format!("telegram:{token}"), headers, body.clone()));
                        if body["chat_id"] == "gone" {
                            (axum::http::StatusCode::FORBIDDEN, Json(json!({"ok": false, "error_code": 403, "description": "Forbidden: bot was blocked by the user"})))
                        } else {
                            (axum::http::StatusCode::OK, Json(json!({"ok": true, "result": {"message_id": 777}})))
                        }
                    }
                }),
            )
            .route(
                "/chat.postMessage",
                post(move |headers: HeaderMap, Json(body): Json<Value>| {
                    let seen = s2.clone();
                    async move {
                        seen.lock().unwrap().push(("slack:api".into(), headers, body.clone()));
                        if body["channel"] == "CARCHIVED" {
                            Json(json!({"ok": false, "error": "channel_is_archived"}))
                        } else {
                            Json(json!({"ok": true, "ts": "1700000000.000100"}))
                        }
                    }
                }),
            )
            .route(
                "/hooks/T000/B000/xyz",
                post(move |headers: HeaderMap, Json(body): Json<Value>| {
                    let seen = s3.clone();
                    async move {
                        seen.lock().unwrap().push(("slack:webhook".into(), headers, body));
                        "ok"
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base = format!("http://{addr}");

        let telegram =
            ChatConnector::with_bases(Channel::Telegram, cfg(), base.clone(), base.clone());
        let ok = telegram
            .send(&req("42", Some("https://shop.example.com/o/1")))
            .await
            .unwrap();
        assert_eq!(ok.provider, "telegram");
        assert_eq!(ok.provider_message_id.as_deref(), Some("777"));
        let dead = telegram.send(&req("gone", None)).await.unwrap_err();
        assert!(dead.dead_recipient, "{dead:?}");

        let slack = ChatConnector::with_bases(Channel::Slack, cfg(), base.clone(), base.clone());
        let ok = slack.send(&req("C123", None)).await.unwrap();
        assert_eq!(ok.provider_message_id.as_deref(), Some("1700000000.000100"));
        let archived = slack.send(&req("CARCHIVED", None)).await.unwrap_err();
        assert!(archived.dead_recipient);

        let seen = seen.lock().unwrap();
        let (route, _, body) = &seen[0];
        assert_eq!(route, "telegram:123:ABC");
        assert_eq!(body["chat_id"], "42");
        assert_eq!(
            body["text"],
            "Order shipped\n\nParcel FR-2041 is on its way."
        );
        assert_eq!(
            body["reply_markup"]["inline_keyboard"][0][0]["url"],
            "https://shop.example.com/o/1"
        );
        let (_, headers, body) = &seen[2];
        assert_eq!(headers["authorization"], "Bearer xoxb-test");
        assert_eq!(body["channel"], "C123");
        assert!(body["text"]
            .as_str()
            .unwrap()
            .starts_with("*Order shipped*"));
    }
}
