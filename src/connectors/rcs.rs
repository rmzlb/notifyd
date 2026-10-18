//! RCS connector via smsmode (`POST https://rest.smsmode.com/rcs/v1/messages`).
//!
//! Application notifications map onto smsmode's five RCS body types without
//! asking callers to learn the RCS schema:
//!   - A plain message of at most 160 characters (UTF-8 and emojis included)
//!     with no buttons leaves as `BASIC` (smsmode's RCS Basic tier).
//!   - A longer message (up to 3 072 characters) or a message carrying action
//!     buttons (`url`, `rcs.replies`, `rcs.dial`, `rcs.suggestions`) leaves as
//!     `TEXT` with its `suggestions`.
//!   - `rcs.card` builds a rich `CARD` (using `subject` as `title` and `body`
//!     as `description` when the card does not set its own, plus `media_url`).
//!   - `rcs.carousel` builds a 2-to-11 card `CAROUSEL`.
//!   - `rcs.file_url` (or `rcs.file`) sends a `FILE` (PDF, image, video, audio).
//!   - `rcs.body` passes a raw smsmode `body` object through when a caller
//!     wants full control.
//!
//! Delivery and read reports share `/webhooks/smsmode/status` with SMS
//! (`smsmode_status.rs`).

use super::sms::{job_id, smsmode_refusal, smsmode_status_callback};
use super::{http_outcome, Channel, Connector, ProviderError, SendRequest, SendResult};
use crate::config::RcsConfig;
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use tracing::info;
use uuid::Uuid;

const SMSMODE_RCS_MESSAGES_URL: &str = "https://rest.smsmode.com/rcs/v1/messages";

/// Longest RCS agent identifier smsmode accepts (`from`).
pub const AGENT_MAX_CHARS: usize = 40;

/// Up to 160 characters without suggestions qualifies for the `BASIC` tier.
pub const BASIC_TEXT_MAX_CHARS: usize = 160;

/// Longest `TEXT` RCS body smsmode accepts.
pub const TEXT_MAX_CHARS: usize = 3072;

/// Longest card title smsmode accepts.
pub const CARD_TITLE_MAX_CHARS: usize = 200;

/// Longest card description smsmode accepts.
pub const CARD_DESCRIPTION_MAX_CHARS: usize = 2000;

/// Longest suggestion button label smsmode accepts.
pub const SUGGESTION_TEXT_MAX_CHARS: usize = 25;

/// Longest suggestion `postbackData` smsmode accepts.
pub const POSTBACK_MAX_CHARS: usize = 2048;

/// Longest URL smsmode accepts on an `OPEN_URL` suggestion or media file.
pub const URL_MAX_CHARS: usize = 255;

/// Maximum suggestions on a message (`TEXT`, `CARD`, `CAROUSEL`, `FILE`).
pub const MESSAGE_SUGGESTIONS_MAX: usize = 11;

/// Maximum suggestions inside a single card (`CardContent`).
pub const CARD_SUGGESTIONS_MAX: usize = 4;

pub struct RcsConnector {
    config: RcsConfig,
    client: reqwest::Client,
}

impl RcsConnector {
    pub fn new(config: RcsConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// Per-message RCS agent (`rcs.from`), else the instance's `RCS_FROM`.
    /// When neither is set, smsmode uses the channel's default RCS Agent.
    fn sender<'a>(&'a self, req: &'a SendRequest) -> Option<&'a str> {
        req.metadata
            .get("rcs")
            .and_then(|rcs| rcs.get("from"))
            .and_then(Value::as_str)
            .or(self.config.from.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
}

impl RcsConfig {
    /// Why smsmode would refuse `from` as the RCS agent identifier.
    pub fn sender_refusal(&self, from: Option<&str>) -> Option<&'static str> {
        let from = from.or(self.from.as_deref())?;
        if !valid_agent(from) {
            return Some(
                "smsmode RCS agent ('from') must be 1 to 40 characters without control characters",
            );
        }
        None
    }
}

/// A valid smsmode RCS agent identifier (`from`): 1 to 40 printable characters.
pub fn valid_agent(from: &str) -> bool {
    let trimmed = from.trim();
    let count = trimmed.chars().count();
    (1..=AGENT_MAX_CHARS).contains(&count) && !trimmed.chars().any(char::is_control)
}

#[async_trait]
impl Connector for RcsConnector {
    fn channel(&self) -> Channel {
        Channel::Rcs
    }

    fn provider(&self) -> &'static str {
        match self.config.provider.as_str() {
            "smsmode" => "smsmode",
            _ => "smsmode",
        }
    }

    async fn send(&self, req: &SendRequest) -> SendResult {
        match self.config.provider.as_str() {
            "smsmode" => self.send_smsmode(req).await,
            p => Err(ProviderError::permanent(
                "rcs",
                format!("unknown RCS provider: {p}"),
            )),
        }
    }
}

impl RcsConnector {
    async fn send_smsmode(&self, req: &SendRequest) -> SendResult {
        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| ProviderError::permanent("smsmode", "RCS api_key required"))?;

        let from = self.sender(req);
        if let Some(why) = self.config.sender_refusal(from) {
            return Err(ProviderError::permanent("smsmode", why));
        }

        let rcs_body =
            build_rcs_body(req).map_err(|why| ProviderError::permanent("smsmode", why))?;
        let job = job_id(&req.metadata);
        let status_callback = smsmode_status_callback(
            crate::unsubscribe::public_url().as_deref(),
            self.config.webhook_secret.is_some(),
            job,
        );
        let payload = smsmode_rcs_payload(
            from,
            &req.recipient,
            rcs_body,
            job,
            status_callback.as_deref(),
        );

        let response = self
            .client
            .post(SMSMODE_RCS_MESSAGES_URL)
            .header("X-Api-Key", api_key)
            .header("Accept", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| ProviderError::transport("smsmode", e))?;
        let mut refused = None;
        let delivery = http_outcome("smsmode", response, |json| {
            refused = smsmode_refusal(json);
            json.get("messageId")
                .and_then(Value::as_str)
                .map(String::from)
        })
        .await?;
        if let Some(why) = refused {
            return Err(ProviderError::permanent("smsmode", why));
        }
        info!(
            "RCS sent via smsmode to {}",
            crate::pii::mask_phone(&req.recipient)
        );
        Ok(delivery)
    }
}

/// The `POST /rcs/v1/messages` JSON envelope. `recipient.to` carries digits
/// without `+`; `from` is optional (channel default agent when omitted).
pub fn smsmode_rcs_payload(
    from: Option<&str>,
    to: &str,
    rcs_body: Value,
    job: Option<Uuid>,
    status_callback: Option<&str>,
) -> Value {
    let mut payload = json!({
        "recipient": { "to": to.trim().trim_start_matches('+') },
        "body": rcs_body,
    });
    if let Some(from) = from.map(str::trim).filter(|s| !s.is_empty()) {
        payload["from"] = json!(from);
    }
    if let Some(job) = job {
        payload["refClient"] = json!(job.to_string());
    }
    if let Some(url) = status_callback {
        payload["callbackUrlStatus"] = json!(url);
    }
    payload
}

/// Validates the `rcs` extras object on `/v1/send` or `/v1/batch` at enqueue.
pub fn validate_extras(rcs: &Value) -> Result<Value, String> {
    let Some(extras) = rcs.as_object() else {
        return Err("rcs must be an object".to_string());
    };
    let mut extras = extras.clone();
    match extras.get("from") {
        None | Some(Value::Null) => {}
        Some(Value::String(from)) => {
            let from = from.trim().to_string();
            if !valid_agent(&from) {
                return Err(format!(
                    "invalid rcs.from '{from}': must be 1 to {AGENT_MAX_CHARS} characters without control characters"
                ));
            }
            extras.insert("from".to_string(), json!(from));
        }
        Some(_) => return Err("rcs.from must be a string".to_string()),
    }
    let normalized = Value::Object(extras);
    // Validate the structural fields (`replies`, `dial`, `suggestions`, `card`,
    // `carousel`, `file_url`, `body`) with a placeholder text so any malformed
    // button or card is rejected with 422 before queueing.
    let probe = SendRequest {
        recipient: "+33600000000".to_string(),
        subject: Some("Subject".to_string()),
        body: "Body".to_string(),
        body_html: None,
        from_email: None,
        from_name: None,
        metadata: json!({ "rcs": normalized }),
    };
    build_rcs_body(&probe)?;
    Ok(normalized)
}

/// Builds the smsmode `body` object (`BASIC`, `TEXT`, `CARD`, `CAROUSEL` or
/// `FILE`) from a `SendRequest`.
pub fn build_rcs_body(req: &SendRequest) -> Result<Value, String> {
    let rcs = req.metadata.get("rcs").and_then(Value::as_object);

    // 1. Raw `rcs.body` passthrough for callers that construct the smsmode
    //    schema directly.
    if let Some(raw_body) = rcs.and_then(|r| r.get("body")).filter(|v| !v.is_null()) {
        return normalize_raw_body(raw_body);
    }

    let top_url = req
        .metadata
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|u| !u.is_empty());
    let suggestions = collect_suggestions(rcs, top_url, MESSAGE_SUGGESTIONS_MAX)?;

    // 2. Rich `CARD` (`rcs.card`).
    if let Some(card) = rcs.and_then(|r| r.get("card")).filter(|v| !v.is_null()) {
        let content = build_card_content(card, req.subject.as_deref(), Some(&req.body))?;
        let mut body = json!({
            "type": "CARD",
            "content": content,
        });
        if let Some(orientation) = card
            .get("orientation")
            .or_else(|| rcs.and_then(|r| r.get("orientation")))
            .and_then(Value::as_str)
        {
            let upper = orientation.trim().to_ascii_uppercase();
            if !matches!(upper.as_str(), "VERTICAL" | "HORIZONTAL") {
                return Err("rcs card orientation must be VERTICAL or HORIZONTAL".to_string());
            }
            body["orientation"] = json!(upper);
        }
        if let Some(alignment) = card
            .get("alignment")
            .or_else(|| rcs.and_then(|r| r.get("alignment")))
            .and_then(Value::as_str)
        {
            let upper = alignment.trim().to_ascii_uppercase();
            if !matches!(upper.as_str(), "LEFT" | "RIGHT") {
                return Err("rcs card alignment must be LEFT or RIGHT".to_string());
            }
            body["alignment"] = json!(upper);
        }
        if !suggestions.is_empty() {
            body["suggestions"] = Value::Array(suggestions);
        }
        return Ok(body);
    }

    // 3. `CAROUSEL` (`rcs.carousel`: 2 to 11 cards).
    if let Some(carousel) = rcs.and_then(|r| r.get("carousel")).filter(|v| !v.is_null()) {
        let Some(items) = carousel.as_array() else {
            return Err("rcs.carousel must be an array of 2 to 11 cards".to_string());
        };
        if !(2..=11).contains(&items.len()) {
            return Err(format!(
                "rcs.carousel requires 2 to 11 cards (got {})",
                items.len()
            ));
        }
        let mut contents = Vec::with_capacity(items.len());
        for item in items {
            contents.push(build_card_content(item, None, None)?);
        }
        let mut body = json!({
            "type": "CAROUSEL",
            "contents": contents,
        });
        if let Some(width) = rcs
            .and_then(|r| r.get("card_width").or_else(|| r.get("cardWidth")))
            .and_then(Value::as_str)
        {
            let upper = width.trim().to_ascii_uppercase();
            if !matches!(upper.as_str(), "SMALL" | "MEDIUM") {
                return Err("rcs.card_width must be SMALL or MEDIUM".to_string());
            }
            body["cardWidth"] = json!(upper);
        }
        if !suggestions.is_empty() {
            body["suggestions"] = Value::Array(suggestions);
        }
        return Ok(body);
    }

    // 4. `FILE` (`rcs.file_url` or `rcs.file`).
    if let Some(file_val) = rcs
        .and_then(|r| {
            r.get("file_url")
                .or_else(|| r.get("fileUrl"))
                .or_else(|| r.get("file"))
        })
        .filter(|v| !v.is_null())
    {
        let (file_url, thumbnail_url) = match file_val {
            Value::String(url) => (
                url.as_str(),
                rcs.and_then(|r| r.get("thumbnail_url").or_else(|| r.get("thumbnailUrl")))
                    .and_then(Value::as_str),
            ),
            Value::Object(obj) => (
                obj.get("file_url")
                    .or_else(|| obj.get("fileUrl"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| "rcs.file requires file_url".to_string())?,
                obj.get("thumbnail_url")
                    .or_else(|| obj.get("thumbnailUrl"))
                    .and_then(Value::as_str),
            ),
            _ => return Err("rcs.file_url must be a URL string or an object".to_string()),
        };
        let file_url = validate_http_url(file_url, "rcs.file_url")?;
        let mut body = json!({
            "type": "FILE",
            "fileUrl": file_url,
        });
        if let Some(thumb) = thumbnail_url {
            body["thumbnailUrl"] = json!(validate_http_url(thumb, "rcs.thumbnail_url")?);
        }
        if !suggestions.is_empty() {
            body["suggestions"] = Value::Array(suggestions);
        }
        return Ok(body);
    }

    // 5. `BASIC` or `TEXT`.
    let text = plain_text(req);
    let count = text.chars().count();
    if count == 0 {
        return Err("RCS message text cannot be empty".to_string());
    }
    if count > TEXT_MAX_CHARS {
        return Err(format!(
            "RCS message text cannot exceed {TEXT_MAX_CHARS} characters (got {count})"
        ));
    }

    let forced_type = rcs
        .and_then(|r| r.get("type"))
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_uppercase());
    if let Some(ref t) = forced_type {
        if !matches!(t.as_str(), "BASIC" | "TEXT") {
            return Err(format!(
                "rcs.type '{t}' requires its matching payload (card, carousel or file_url)"
            ));
        }
        if t == "BASIC" && (!suggestions.is_empty() || count > BASIC_TEXT_MAX_CHARS) {
            return Err(format!(
                "RCS BASIC messages cannot exceed {BASIC_TEXT_MAX_CHARS} characters or carry suggestions"
            ));
        }
    }

    let use_basic = forced_type.as_deref() != Some("TEXT")
        && suggestions.is_empty()
        && count <= BASIC_TEXT_MAX_CHARS;
    if use_basic {
        Ok(json!({
            "type": "BASIC",
            "text": text,
        }))
    } else {
        let mut body = json!({
            "type": "TEXT",
            "text": text,
        });
        if !suggestions.is_empty() {
            body["suggestions"] = Value::Array(suggestions);
        }
        Ok(body)
    }
}

/// Text of a `BASIC` or `TEXT` RCS message: `rcs.text` when set, else
/// `subject` + blank line + `body` (like `chat::plain_text`).
pub fn plain_text(req: &SendRequest) -> String {
    if let Some(t) = req
        .metadata
        .get("rcs")
        .and_then(|r| r.get("text"))
        .and_then(Value::as_str)
    {
        return t.to_string();
    }
    match req
        .subject
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(subject) if !req.body.trim().is_empty() => format!("{subject}\n\n{}", req.body),
        Some(subject) => subject.to_string(),
        None => req.body.clone(),
    }
}

/// Builds a `CardContent` object from a card specification, falling back to
/// `default_title` (`req.subject`) and `default_description` (`req.body`) on a
/// single card.
fn build_card_content(
    card: &Value,
    default_title: Option<&str>,
    default_description: Option<&str>,
) -> Result<Value, String> {
    let Some(obj) = card.as_object() else {
        return Err("rcs card must be an object".to_string());
    };
    let mut content = Map::new();

    let title = obj
        .get("title")
        .and_then(Value::as_str)
        .or(default_title)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(title) = title {
        if title.chars().count() > CARD_TITLE_MAX_CHARS {
            return Err(format!(
                "rcs card title cannot exceed {CARD_TITLE_MAX_CHARS} characters"
            ));
        }
        content.insert("title".to_string(), json!(title));
    }

    let description = obj
        .get("description")
        .and_then(Value::as_str)
        .or(default_description)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(description) = description {
        if description.chars().count() > CARD_DESCRIPTION_MAX_CHARS {
            return Err(format!(
                "rcs card description cannot exceed {CARD_DESCRIPTION_MAX_CHARS} characters"
            ));
        }
        content.insert("description".to_string(), json!(description));
    }

    if let Some(media) = build_card_media(obj)? {
        content.insert("media".to_string(), media);
    }

    if !content.contains_key("title") && !content.contains_key("media") {
        return Err("an RCS card must contain a title (or subject) or media".to_string());
    }

    let card_url = obj
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|u| !u.is_empty());
    let suggestions = collect_suggestions(Some(obj), card_url, CARD_SUGGESTIONS_MAX)?;
    if !suggestions.is_empty() {
        content.insert("suggestions".to_string(), Value::Array(suggestions));
    }

    Ok(Value::Object(content))
}

fn build_card_media(card: &Map<String, Value>) -> Result<Option<Value>, String> {
    if let Some(media_obj) = card.get("media").and_then(Value::as_object) {
        let raw_url = media_obj
            .get("fileUrl")
            .or_else(|| media_obj.get("file_url"))
            .and_then(Value::as_str)
            .ok_or_else(|| "rcs card media requires fileUrl".to_string())?;
        let file_url = validate_http_url(raw_url, "rcs card media.fileUrl")?;
        let mut media = json!({ "fileUrl": file_url });
        if let Some(thumb) = media_obj
            .get("thumbnailUrl")
            .or_else(|| media_obj.get("thumbnail_url"))
            .and_then(Value::as_str)
        {
            media["thumbnailUrl"] = json!(validate_http_url(thumb, "rcs card media.thumbnailUrl")?);
        }
        if let Some(height) = media_obj.get("height").and_then(Value::as_str) {
            media["height"] = json!(validate_media_height(height)?);
        }
        return Ok(Some(media));
    }

    let Some(raw_url) = card
        .get("media_url")
        .or_else(|| card.get("mediaUrl"))
        .or_else(|| card.get("file_url"))
        .or_else(|| card.get("fileUrl"))
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let file_url = validate_http_url(raw_url, "rcs card media_url")?;
    let mut media = json!({ "fileUrl": file_url });
    if let Some(thumb) = card
        .get("thumbnail_url")
        .or_else(|| card.get("thumbnailUrl"))
        .and_then(Value::as_str)
    {
        media["thumbnailUrl"] = json!(validate_http_url(thumb, "rcs card thumbnail_url")?);
    }
    if let Some(height) = card.get("height").and_then(Value::as_str) {
        media["height"] = json!(validate_media_height(height)?);
    }
    Ok(Some(media))
}

fn validate_media_height(height: &str) -> Result<String, String> {
    let upper = height.trim().to_ascii_uppercase();
    if !matches!(upper.as_str(), "SHORT" | "MEDIUM" | "TALL") {
        return Err("rcs card media height must be SHORT, MEDIUM or TALL".to_string());
    }
    Ok(upper)
}

/// Collects and validates suggestions from ergonomic fields (`replies`, `url` +
/// `button`, `dial`) and explicit `suggestions`, up to `max_items`.
fn collect_suggestions(
    scope: Option<&Map<String, Value>>,
    url: Option<&str>,
    max_items: usize,
) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();

    // 1. Quick-reply buttons: `replies: ["Confirmer", "Annuler"]` or
    //    `replies: [{"text": "Confirmer", "postback_data": "yes"}]`.
    if let Some(replies) = scope
        .and_then(|s| s.get("replies"))
        .filter(|v| !v.is_null())
    {
        let Some(items) = replies.as_array() else {
            return Err("rcs.replies must be an array".to_string());
        };
        for item in items {
            match item {
                Value::String(label) => {
                    let text = validate_suggestion_text(label)?;
                    out.push(json!({
                        "type": "REPLY",
                        "text": text,
                        "postbackData": text,
                    }));
                }
                Value::Object(obj) => {
                    let label = obj
                        .get("text")
                        .or_else(|| obj.get("label"))
                        .and_then(Value::as_str)
                        .ok_or_else(|| "each item in rcs.replies must have 'text'".to_string())?;
                    let text = validate_suggestion_text(label)?;
                    let postback = obj
                        .get("postbackData")
                        .or_else(|| obj.get("postback_data"))
                        .or_else(|| obj.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or(&text);
                    let postback = validate_postback_data(postback)?;
                    out.push(json!({
                        "type": "REPLY",
                        "text": text,
                        "postbackData": postback,
                    }));
                }
                _ => {
                    return Err("each item in rcs.replies must be a string or an object".to_string())
                }
            }
        }
    }

    // 2. Top-level or card-level `url` becomes an `OPEN_URL` button, unless the
    //    caller already put that URL in `suggestions`.
    if let Some(raw_url) = url {
        let already_in_suggestions = scope
            .and_then(|s| s.get("suggestions"))
            .and_then(Value::as_array)
            .is_some_and(|arr| {
                arr.iter().any(|s| {
                    s.get("url")
                        .and_then(Value::as_str)
                        .is_some_and(|u| u.trim() == raw_url)
                })
            });
        if !already_in_suggestions
            && (raw_url.starts_with("http://") || raw_url.starts_with("https://"))
        {
            let valid_url = validate_http_url(raw_url, "url")?;
            let label = scope
                .and_then(|s| s.get("button"))
                .and_then(Value::as_str)
                .unwrap_or("Ouvrir");
            let text = validate_suggestion_text(label)?;
            let mut suggestion = json!({
                "type": "OPEN_URL",
                "text": text,
                "url": valid_url,
                "postbackData": valid_url,
            });
            if let Some(webview) = scope
                .and_then(|s| s.get("webview_size").or_else(|| s.get("webviewSize")))
                .and_then(Value::as_str)
            {
                suggestion["webviewSize"] = json!(validate_webview_size(webview)?);
            }
            out.push(suggestion);
        }
    }

    // 3. `dial`: `"+33140000000"` or `{"text": "Appeler", "phone": "+33140000000"}`.
    if let Some(dial) = scope.and_then(|s| s.get("dial")).filter(|v| !v.is_null()) {
        let (label, phone, postback) = match dial {
            Value::String(phone) => ("Appeler", phone.as_str(), None),
            Value::Object(obj) => (
                obj.get("text")
                    .or_else(|| obj.get("label"))
                    .and_then(Value::as_str)
                    .unwrap_or("Appeler"),
                obj.get("phone")
                    .or_else(|| obj.get("phoneNumber"))
                    .or_else(|| obj.get("phone_number"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| "rcs.dial requires 'phone'".to_string())?,
                obj.get("postbackData")
                    .or_else(|| obj.get("postback_data"))
                    .and_then(Value::as_str),
            ),
            _ => return Err("rcs.dial must be a phone number string or an object".to_string()),
        };
        let text = validate_suggestion_text(label)?;
        let phone = normalize_dial_phone(phone)?;
        let postback = validate_postback_data(postback.unwrap_or(&phone))?;
        out.push(json!({
            "type": "DIAL_PHONE",
            "text": text,
            "phoneNumber": phone,
            "postbackData": postback,
        }));
    }

    // 4. Explicit smsmode `suggestions` array.
    if let Some(raw_list) = scope
        .and_then(|s| s.get("suggestions"))
        .filter(|v| !v.is_null())
    {
        let Some(items) = raw_list.as_array() else {
            return Err("rcs.suggestions must be an array".to_string());
        };
        for item in items {
            out.push(normalize_suggestion(item)?);
        }
    }

    if out.len() > max_items {
        return Err(format!(
            "RCS allows at most {max_items} suggestions here (got {})",
            out.len()
        ));
    }

    Ok(out)
}

fn normalize_suggestion(raw: &Value) -> Result<Value, String> {
    let Some(obj) = raw.as_object() else {
        return Err("each RCS suggestion must be an object".to_string());
    };
    let kind = obj
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "RCS suggestion requires 'type'".to_string())?
        .trim()
        .to_ascii_uppercase();
    let label = obj
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("RCS {kind} suggestion requires 'text'"))?;
    let text = validate_suggestion_text(label)?;
    let postback_raw = obj
        .get("postbackData")
        .or_else(|| obj.get("postback_data"))
        .and_then(Value::as_str)
        .unwrap_or(&text);
    let postback = validate_postback_data(postback_raw)?;

    match kind.as_str() {
        "REPLY" => Ok(json!({
            "type": "REPLY",
            "text": text,
            "postbackData": postback,
        })),
        "OPEN_URL" => {
            let raw_url = obj
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| "RCS OPEN_URL suggestion requires 'url'".to_string())?;
            let url = validate_http_url(raw_url, "OPEN_URL suggestion url")?;
            let mut out = json!({
                "type": "OPEN_URL",
                "text": text,
                "url": url,
                "postbackData": postback,
            });
            if let Some(webview) = obj
                .get("webviewSize")
                .or_else(|| obj.get("webview_size"))
                .and_then(Value::as_str)
            {
                out["webviewSize"] = json!(validate_webview_size(webview)?);
            }
            Ok(out)
        }
        "DIAL_PHONE" => {
            let raw_phone = obj
                .get("phoneNumber")
                .or_else(|| obj.get("phone_number"))
                .or_else(|| obj.get("phone"))
                .and_then(Value::as_str)
                .ok_or_else(|| "RCS DIAL_PHONE suggestion requires 'phoneNumber'".to_string())?;
            let phone = normalize_dial_phone(raw_phone)?;
            Ok(json!({
                "type": "DIAL_PHONE",
                "text": text,
                "phoneNumber": phone,
                "postbackData": postback,
            }))
        }
        "REQUEST_LOCATION" => Ok(json!({
            "type": "REQUEST_LOCATION",
            "text": text,
            "postbackData": postback,
        })),
        "SHOW_LOCATION" => {
            let lat = obj
                .get("latitude")
                .and_then(Value::as_f64)
                .filter(|v| (-90.0..=90.0).contains(v))
                .ok_or_else(|| {
                    "RCS SHOW_LOCATION suggestion requires latitude between -90 and 90".to_string()
                })?;
            let lon = obj
                .get("longitude")
                .and_then(Value::as_f64)
                .filter(|v| (-180.0..=180.0).contains(v))
                .ok_or_else(|| {
                    "RCS SHOW_LOCATION suggestion requires longitude between -180 and 180"
                        .to_string()
                })?;
            let mut out = json!({
                "type": "SHOW_LOCATION",
                "text": text,
                "postbackData": postback,
                "latitude": lat,
                "longitude": lon,
            });
            if let Some(loc_label) = obj
                .get("label")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if loc_label.chars().count() > 100 {
                    return Err("RCS SHOW_LOCATION label cannot exceed 100 characters".to_string());
                }
                out["label"] = json!(loc_label);
            }
            Ok(out)
        }
        "CREATE_CALENDAR_EVENT" => {
            let title = obj
                .get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "RCS CREATE_CALENDAR_EVENT suggestion requires 'title'".to_string())?;
            if title.chars().count() > 100 {
                return Err(
                    "RCS CREATE_CALENDAR_EVENT title cannot exceed 100 characters".to_string(),
                );
            }
            let start = obj
                .get("startTime")
                .or_else(|| obj.get("start_time"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    "RCS CREATE_CALENDAR_EVENT suggestion requires 'startTime'".to_string()
                })?;
            let end = obj
                .get("endTime")
                .or_else(|| obj.get("end_time"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    "RCS CREATE_CALENDAR_EVENT suggestion requires 'endTime'".to_string()
                })?;
            let mut out = json!({
                "type": "CREATE_CALENDAR_EVENT",
                "text": text,
                "postbackData": postback,
                "title": title,
                "startTime": start,
                "endTime": end,
            });
            if let Some(desc) = obj
                .get("description")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if desc.chars().count() > 500 {
                    return Err(
                        "RCS CREATE_CALENDAR_EVENT description cannot exceed 500 characters"
                            .to_string(),
                    );
                }
                out["description"] = json!(desc);
            }
            Ok(out)
        }
        other => Err(format!(
            "unknown RCS suggestion type '{other}' (expected REPLY, OPEN_URL, DIAL_PHONE, SHOW_LOCATION, REQUEST_LOCATION or CREATE_CALENDAR_EVENT)"
        )),
    }
}

fn normalize_raw_body(raw: &Value) -> Result<Value, String> {
    let Some(obj) = raw.as_object() else {
        return Err("rcs.body must be an object".to_string());
    };
    let mut out = obj.clone();
    let kind = out
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            "rcs.body requires 'type' (BASIC, TEXT, CARD, CAROUSEL or FILE)".to_string()
        })?
        .trim()
        .to_ascii_uppercase();
    if !matches!(
        kind.as_str(),
        "BASIC" | "TEXT" | "CARD" | "CAROUSEL" | "FILE"
    ) {
        return Err(format!(
            "rcs.body.type must be BASIC, TEXT, CARD, CAROUSEL or FILE (got '{kind}')"
        ));
    }
    out.insert("type".to_string(), json!(kind));
    if let Some(suggestions) = out.get("suggestions").and_then(Value::as_array) {
        let mut normalized = Vec::with_capacity(suggestions.len());
        for s in suggestions {
            normalized.push(normalize_suggestion(s)?);
        }
        if normalized.len() > MESSAGE_SUGGESTIONS_MAX {
            return Err(format!(
                "rcs.body.suggestions cannot exceed {MESSAGE_SUGGESTIONS_MAX} items"
            ));
        }
        out.insert("suggestions".to_string(), Value::Array(normalized));
    }
    Ok(Value::Object(out))
}

fn validate_suggestion_text(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let count = trimmed.chars().count();
    if count == 0 || count > SUGGESTION_TEXT_MAX_CHARS {
        return Err(format!(
            "RCS button text must be 1 to {SUGGESTION_TEXT_MAX_CHARS} characters"
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_postback_data(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let count = trimmed.chars().count();
    if count == 0 || count > POSTBACK_MAX_CHARS {
        return Err(format!(
            "RCS postbackData must be 1 to {POSTBACK_MAX_CHARS} characters"
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_http_url(raw: &str, field: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(format!("{field} must start with http:// or https://"));
    }
    if trimmed.len() > URL_MAX_CHARS {
        return Err(format!("{field} cannot exceed {URL_MAX_CHARS} characters"));
    }
    Ok(trimmed.to_string())
}

fn validate_webview_size(raw: &str) -> Result<String, String> {
    let upper = raw.trim().to_ascii_uppercase();
    if !matches!(upper.as_str(), "NONE" | "HALF" | "TALL" | "FULL") {
        return Err("RCS webview_size must be NONE, HALF, TALL or FULL".to_string());
    }
    Ok(upper)
}

/// smsmode examples pass digits without `+` for `DIAL_PHONE.phoneNumber`
/// (`"3349106463"`). Accepts E.164 with or without `+` and strips `+`.
fn normalize_dial_phone(raw: &str) -> Result<String, String> {
    let digits = raw.trim().trim_start_matches('+');
    if !(2..=15).contains(&digits.len())
        || digits.starts_with('0')
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(format!(
            "invalid RCS dial phone '{raw}': expected international E.164 digits (e.g. +33140000000)"
        ));
    }
    Ok(digits.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::sms::JOB_ID_METADATA;
    use serde_json::json;

    const JOB: &str = "7d9f0c2e-1b3a-4c5d-8e6f-0a1b2c3d4e5f";

    fn req(subject: Option<&str>, body: &str, metadata: Value) -> SendRequest {
        SendRequest {
            recipient: "+33639980000".to_string(),
            subject: subject.map(String::from),
            body: body.to_string(),
            body_html: None,
            from_email: None,
            from_name: None,
            metadata,
        }
    }

    #[test]
    fn short_plain_message_uses_basic_tier_even_with_french_accents_and_emoji() {
        let r = req(
            None,
            "Bonjour Marie, votre RDV demain à 14h au Centre Foch est confirmé : C’est noté 👍",
            json!({}),
        );
        let built = build_rcs_body(&r).unwrap();
        assert_eq!(
            built,
            json!({
                "type": "BASIC",
                "text": "Bonjour Marie, votre RDV demain à 14h au Centre Foch est confirmé : C’est noté 👍",
            })
        );
    }

    #[test]
    fn message_over_160_chars_or_with_buttons_upgrades_to_text() {
        let long_text = "a".repeat(161);
        let built = build_rcs_body(&req(None, &long_text, json!({}))).unwrap();
        assert_eq!(built["type"], "TEXT");
        assert_eq!(built["text"], long_text);
        assert!(built.get("suggestions").is_none());

        // Short text, but carries a top-level URL and quick replies -> TEXT with suggestions.
        let interactive = req(
            Some("RDV demain à 14h"),
            "Confirmez-vous votre venue ?",
            json!({
                "url": "https://sqare.fr/rdv/42",
                "rcs": {
                    "button": "Voir le RDV",
                    "replies": ["Confirmer", {"text": "Annuler", "postback_data": "cancel_42"}],
                    "dial": "+33140000000"
                }
            }),
        );
        let built = build_rcs_body(&interactive).unwrap();
        assert_eq!(
            built,
            json!({
                "type": "TEXT",
                "text": "RDV demain à 14h\n\nConfirmez-vous votre venue ?",
                "suggestions": [
                    {
                        "type": "REPLY",
                        "text": "Confirmer",
                        "postbackData": "Confirmer"
                    },
                    {
                        "type": "REPLY",
                        "text": "Annuler",
                        "postbackData": "cancel_42"
                    },
                    {
                        "type": "OPEN_URL",
                        "text": "Voir le RDV",
                        "url": "https://sqare.fr/rdv/42",
                        "postbackData": "https://sqare.fr/rdv/42"
                    },
                    {
                        "type": "DIAL_PHONE",
                        "text": "Appeler",
                        "phoneNumber": "33140000000",
                        "postbackData": "33140000000"
                    }
                ]
            })
        );
    }

    #[test]
    fn card_uses_subject_and_body_as_title_and_description_defaults() {
        let r = req(
            Some("Centre Dentaire Foch"),
            "Votre devis est disponible dans votre espace patient.",
            json!({
                "url": "https://sqare.fr/devis/99",
                "rcs": {
                    "card": {
                        "media_url": "https://sqare.fr/img/clinique.png",
                        "height": "TALL",
                        "orientation": "VERTICAL",
                        "replies": ["Accepter"]
                    },
                    "button": "Ouvrir le devis"
                }
            }),
        );
        let built = build_rcs_body(&r).unwrap();
        assert_eq!(
            built,
            json!({
                "type": "CARD",
                "orientation": "VERTICAL",
                "content": {
                    "title": "Centre Dentaire Foch",
                    "description": "Votre devis est disponible dans votre espace patient.",
                    "media": {
                        "fileUrl": "https://sqare.fr/img/clinique.png",
                        "height": "TALL"
                    },
                    "suggestions": [
                        {
                            "type": "REPLY",
                            "text": "Accepter",
                            "postbackData": "Accepter"
                        }
                    ]
                },
                "suggestions": [
                    {
                        "type": "OPEN_URL",
                        "text": "Ouvrir le devis",
                        "url": "https://sqare.fr/devis/99",
                        "postbackData": "https://sqare.fr/devis/99"
                    }
                ]
            })
        );
    }

    #[test]
    fn carousel_and_file_messages_follow_smsmode_schema() {
        let carousel = req(
            None,
            "",
            json!({
                "rcs": {
                    "card_width": "SMALL",
                    "carousel": [
                        {
                            "title": "Carte 1",
                            "media_url": "https://sqare.fr/1.png",
                            "url": "https://sqare.fr/1",
                            "button": "Voir 1"
                        },
                        {
                            "title": "Carte 2",
                            "media_url": "https://sqare.fr/2.png"
                        }
                    ]
                }
            }),
        );
        let built = build_rcs_body(&carousel).unwrap();
        assert_eq!(built["type"], "CAROUSEL");
        assert_eq!(built["cardWidth"], "SMALL");
        assert_eq!(built["contents"].as_array().map(Vec::len), Some(2));
        assert_eq!(
            built["contents"][0]["suggestions"][0]["url"],
            "https://sqare.fr/1"
        );

        let file = req(
            None,
            "",
            json!({
                "rcs": {
                    "file_url": "https://sqare.fr/doc/devis.pdf",
                    "replies": ["Bien reçu"]
                }
            }),
        );
        let built_file = build_rcs_body(&file).unwrap();
        assert_eq!(
            built_file,
            json!({
                "type": "FILE",
                "fileUrl": "https://sqare.fr/doc/devis.pdf",
                "suggestions": [
                    {
                        "type": "REPLY",
                        "text": "Bien reçu",
                        "postbackData": "Bien reçu"
                    }
                ]
            })
        );
    }

    #[test]
    fn smsmode_rcs_envelope_strips_plus_and_includes_optional_agent_and_callback() {
        let job = Uuid::parse_str(JOB).unwrap();
        let callback = format!("https://notifyd.example.com/webhooks/smsmode/status?job={JOB}");
        let body = json!({"type": "BASIC", "text": "Bonjour"});
        assert_eq!(
            smsmode_rcs_payload(
                Some("Centre Foch"),
                "+33786976911",
                body.clone(),
                Some(job),
                Some(&callback),
            ),
            json!({
                "recipient": { "to": "33786976911" },
                "body": { "type": "BASIC", "text": "Bonjour" },
                "from": "Centre Foch",
                "refClient": JOB,
                "callbackUrlStatus": callback,
            })
        );
        // Without `from`, smsmode uses the channel's default RCS Agent.
        assert_eq!(
            smsmode_rcs_payload(None, "+33786976911", body, None, None),
            json!({
                "recipient": { "to": "33786976911" },
                "body": { "type": "BASIC", "text": "Bonjour" },
            })
        );
    }

    #[test]
    fn invalid_rcs_extras_are_rejected_at_enqueue() {
        assert!(validate_extras(&json!({"from": "  Centre Foch  "})).is_ok());
        assert!(validate_extras(&json!({"from": ""})).is_err());
        assert!(validate_extras(&json!({"from": "a".repeat(41)})).is_err());
        assert!(validate_extras(&json!({"replies": ["Ce bouton dépasse 25 caractères"]})).is_err());
        assert!(validate_extras(&json!({"carousel": [{"title": "Une seule carte"}]})).is_err());
        assert!(validate_extras(&json!({"file_url": "ftp://example.com/file.pdf"})).is_err());
        let _ = JOB_ID_METADATA;
    }
}
