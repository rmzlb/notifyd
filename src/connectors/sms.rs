use super::{http_outcome, Channel, Connector, ProviderError, SendRequest, SendResult};
use crate::config::SmsConfig;
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::info;
use uuid::Uuid;

/// Metadata key holding the job id on an SMS request, set by the worker: the
/// Twilio and smsmode connectors put it in the status callback URL, and
/// smsmode also as the message's `refClient`.
pub const JOB_ID_METADATA: &str = "notifyd_job_id";

const SMSMODE_MESSAGES_URL: &str = "https://rest.smsmode.com/sms/v1/messages";

/// Longest sender smsmode takes: 11 letters or digits, a number without `+`.
const SMSMODE_SENDER_MAX: usize = 11;

pub struct SmsConnector {
    config: SmsConfig,
    client: reqwest::Client,
}

impl SmsConnector {
    pub fn new(config: SmsConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl Connector for SmsConnector {
    fn channel(&self) -> Channel {
        Channel::Sms
    }

    fn provider(&self) -> &'static str {
        match self.config.provider.as_str() {
            "twilio" => "twilio",
            "smsmode" => "smsmode",
            _ => "telnyx",
        }
    }

    async fn send(&self, req: &SendRequest) -> SendResult {
        match self.config.provider.as_str() {
            "twilio" => self.send_twilio(req).await,
            "smsmode" => self.send_smsmode(req).await,
            "telnyx" => self.send_telnyx(req).await,
            p => Err(ProviderError::permanent(
                "sms",
                format!("unknown SMS provider: {p}"),
            )),
        }
    }
}

impl SmsConfig {
    /// Why the provider would refuse `from` as the sender. The API checks it
    /// at enqueue, so the caller gets the refusal instead of a job that the
    /// worker fails later.
    pub fn sender_refusal(&self, from: &str) -> Option<&'static str> {
        match self.provider.as_str() {
            // Telnyx sends an alphanumeric sender only through the messaging
            // profile that carries it.
            "telnyx" if is_alphanumeric_sender(from) && self.messaging_profile_id.is_none() => {
                Some("an alphanumeric sender needs TELNYX_MESSAGING_PROFILE_ID")
            }
            "smsmode" if smsmode_sender(from).len() > SMSMODE_SENDER_MAX => {
                Some("smsmode takes a sender of at most 11 characters (a number without its +)")
            }
            _ => None,
        }
    }
}

impl SmsConnector {
    /// The message's own sender (`sms.from`, checked at enqueue), else the
    /// instance's `SMS_FROM`.
    fn sender<'a>(&'a self, req: &'a SendRequest) -> &'a str {
        req.metadata
            .get("sms")
            .and_then(|sms| sms.get("from"))
            .and_then(Value::as_str)
            .unwrap_or(&self.config.from)
    }

    async fn send_twilio(&self, req: &SendRequest) -> SendResult {
        let account_sid = self
            .config
            .account_sid
            .as_deref()
            .ok_or_else(|| ProviderError::permanent("twilio", "account_sid required"))?;
        let auth_token = self
            .config
            .auth_token
            .as_deref()
            .ok_or_else(|| ProviderError::permanent("twilio", "auth_token required"))?;

        let url = format!(
            "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
            account_sid
        );

        let status_callback =
            status_callback(crate::unsubscribe::public_url().as_deref(), &req.metadata);
        let params = twilio_form(
            self.sender(req),
            &req.recipient,
            &req.body,
            status_callback.as_deref(),
        );

        let response = self
            .client
            .post(&url)
            .basic_auth(account_sid, Some(auth_token))
            .form(&params)
            .send()
            .await
            .map_err(|e| ProviderError::transport("twilio", e))?;
        let delivery = http_outcome("twilio", response, |json| {
            json.get("sid").and_then(Value::as_str).map(String::from)
        })
        .await?;
        info!(
            "SMS sent via Twilio to {}",
            crate::pii::mask_phone(&req.recipient)
        );
        Ok(delivery)
    }

    async fn send_smsmode(&self, req: &SendRequest) -> SendResult {
        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| ProviderError::permanent("smsmode", "api_key required"))?;

        let from = self.sender(req);
        if let Some(why) = self.config.sender_refusal(from) {
            return Err(ProviderError::permanent("smsmode", why));
        }

        let job = job_id(&req.metadata);
        let status_callback = smsmode_status_callback(
            crate::unsubscribe::public_url().as_deref(),
            self.config.webhook_secret.is_some(),
            job,
        );
        let body = smsmode_body(
            from,
            &req.recipient,
            &req.body,
            job,
            status_callback.as_deref(),
        );

        let response = self
            .client
            .post(SMSMODE_MESSAGES_URL)
            .header("X-Api-Key", api_key)
            .header("Accept", "application/json")
            .json(&body)
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
            "SMS sent via smsmode to {}",
            crate::pii::mask_phone(&req.recipient)
        );
        Ok(delivery)
    }

    async fn send_telnyx(&self, req: &SendRequest) -> SendResult {
        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| ProviderError::permanent("telnyx", "api_key required"))?;

        let from = self.sender(req);
        if let Some(why) = self.config.sender_refusal(from) {
            return Err(ProviderError::permanent("telnyx", why));
        }

        let mut body = serde_json::json!({
            "from": from,
            "to": req.recipient,
            "text": req.body,
            "type": "SMS",
        });

        // Optional messaging profile
        if let Some(profile_id) = &self.config.messaging_profile_id {
            body["messaging_profile_id"] = serde_json::Value::String(profile_id.clone());
        }

        let response = self
            .client
            .post("https://api.telnyx.com/v2/messages")
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::transport("telnyx", e))?;
        let delivery = http_outcome("telnyx", response, telnyx_message_id).await?;
        info!(
            "SMS sent via Telnyx to {}",
            crate::pii::mask_phone(&req.recipient)
        );
        Ok(delivery)
    }
}

/// A sender carriers accept: an E.164 number, or an alphanumeric sender ID of
/// 1 to 11 ASCII letters, digits or spaces with at least one letter. The
/// alphanumeric form is one-way: the recipient cannot answer it.
pub fn valid_sender(from: &str) -> bool {
    if let Some(digits) = from.strip_prefix('+') {
        return (7..=15).contains(&digits.len())
            && !digits.starts_with('0')
            && digits.bytes().all(|b| b.is_ascii_digit());
    }
    (1..=11).contains(&from.len())
        && from.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ')
        && is_alphanumeric_sender(from)
}

fn is_alphanumeric_sender(from: &str) -> bool {
    from.bytes().any(|b| b.is_ascii_alphabetic())
}

/// The job id the worker put on the request.
pub(super) fn job_id(metadata: &Value) -> Option<Uuid> {
    metadata
        .get(JOB_ID_METADATA)
        .and_then(Value::as_str)
        .and_then(|id| Uuid::parse_str(id).ok())
}

/// Where Twilio reports the message's delivery status: only when the
/// instance has a public URL and the request carries its job id.
fn status_callback(public_url: Option<&str>, metadata: &Value) -> Option<String> {
    Some(crate::twilio_status::callback_url(
        public_url?,
        job_id(metadata)?,
    ))
}

/// Where smsmode reports the message's delivery: only when the instance has
/// a public URL, can check smsmode's signature (`SMSMODE_WEBHOOK_SECRET`)
/// and the request carries its job id. Without the secret every report
/// would be refused and retried for a day, so none is asked for.
pub(super) fn smsmode_status_callback(
    public_url: Option<&str>,
    has_secret: bool,
    job: Option<Uuid>,
) -> Option<String> {
    if !has_secret {
        return None;
    }
    crate::smsmode_status::callback_url(public_url?, job?)
}

/// smsmode writes a number sender without its `+`.
fn smsmode_sender(from: &str) -> &str {
    match from.strip_prefix('+') {
        Some(digits) if digits.bytes().all(|b| b.is_ascii_digit()) => digits,
        _ => from,
    }
}

/// The `POST /sms/v1/messages` body. smsmode reads the text as GSM 7-bit
/// unless told otherwise and silently replaces what that alphabet lacks
/// (`’` becomes `'`, an emoji `?`), so a text outside it is sent as
/// `UNICODE`, as Twilio does on its own. No `stop`: notifyd's SMS are
/// transactional. The job id is the `refClient`, which smsmode also puts on
/// the replies to the message.
fn smsmode_body(
    from: &str,
    to: &str,
    text: &str,
    job: Option<Uuid>,
    status_callback: Option<&str>,
) -> Value {
    let mut body = json!({
        "recipient": { "to": to.trim().trim_start_matches('+') },
        "body": { "text": text },
        "from": smsmode_sender(from),
    });
    if !is_gsm7(text) {
        body["body"]["encoding"] = json!("UNICODE");
    }
    if let Some(job) = job {
        body["refClient"] = json!(job.to_string());
    }
    if let Some(url) = status_callback {
        body["callbackUrlStatus"] = json!(url);
    }
    body
}

/// GSM 03.38: the default alphabet and its extension table (sent as two
/// septets). A text made only of these goes out as GSM 7-bit.
fn is_gsm7(text: &str) -> bool {
    const DEFAULT: &str = "@£$¥èéùìòÇ\nØø\rÅåΔ_ΦΓΛΩΠΨΣΘΞÆæßÉ !\"#¤%&'()*+,-./0123456789:;<=>?\
                           ¡ABCDEFGHIJKLMNOPQRSTUVWXYZÄÖÑÜ§¿abcdefghijklmnopqrstuvwxyzäöñüà";
    const EXTENSION: &str = "\u{0c}^{}\\[~]|€";
    text.chars()
        .all(|c| DEFAULT.contains(c) || EXTENSION.contains(c))
}

/// A 201 whose message smsmode already declares undeliverable (no route, no
/// credit…): the job fails now rather than waiting for a report.
pub(super) fn smsmode_refusal(json: &Value) -> Option<String> {
    let status = json.get("status")?;
    let value = status.get("value").and_then(Value::as_str)?;
    if !matches!(value, "UNDELIVERABLE" | "UNDELIVERED") {
        return None;
    }
    Some(match status.get("detail").and_then(Value::as_str) {
        Some(detail) => format!("smsmode {value} {detail}"),
        None => format!("smsmode {value}"),
    })
}

/// The `Messages.json` form. Without a status callback it is the form
/// Twilio always received.
fn twilio_form<'a>(
    from: &'a str,
    to: &'a str,
    body: &'a str,
    status_callback: Option<&'a str>,
) -> Vec<(&'static str, &'a str)> {
    let mut params = vec![("From", from), ("To", to), ("Body", body)];
    if let Some(url) = status_callback {
        params.push(("StatusCallback", url));
    }
    params
}

/// Telnyx wraps the message as `{ "data": { "id": "…" } }`.
pub fn telnyx_message_id(json: &Value) -> Option<String> {
    json.get("data")
        .and_then(|d| d.get("id"))
        .and_then(Value::as_str)
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn connector() -> SmsConnector {
        SmsConnector::new(SmsConfig {
            provider: "telnyx".to_string(),
            account_sid: None,
            auth_token: None,
            api_key: Some("KEY".to_string()),
            messaging_profile_id: None,
            webhook_secret: None,
            from: "+33600000000".to_string(),
        })
    }

    fn req(metadata: Value) -> SendRequest {
        SendRequest {
            recipient: "+33639980000".to_string(),
            subject: None,
            body: "hello".to_string(),
            body_html: None,
            from_email: None,
            from_name: None,
            metadata,
        }
    }

    #[test]
    fn senders_carriers_accept() {
        for ok in [
            "CDMF",
            "CDMF Paris",
            "Helmai2",
            "A",
            "+33612345678",
            "+1415555",
        ] {
            assert!(valid_sender(ok), "{ok} should pass");
        }
        for bad in [
            "",
            "12345",
            "CDMF-Paris",
            "Cabinet dentaire",
            "Clinique é",
            "+0612345678",
            "+33 6 12 34",
            "+123456",
            "+1234567890123456",
        ] {
            assert!(!valid_sender(bad), "{bad} should fail");
        }
    }

    #[test]
    fn message_sender_overrides_the_instance_default() {
        let c = connector();
        assert_eq!(c.sender(&req(json!({"sms": {"from": "CDMF"}}))), "CDMF");
        assert_eq!(c.sender(&req(json!({}))), "+33600000000");
    }

    #[tokio::test]
    async fn alphanumeric_sender_without_profile_fails_before_calling_telnyx() {
        let err = connector()
            .send(&req(json!({"sms": {"from": "CDMF"}})))
            .await
            .expect_err("no messaging profile");
        assert_eq!(err.kind, crate::connectors::ProviderErrorKind::Permanent);
        assert!(err.message.contains("TELNYX_MESSAGING_PROFILE_ID"));
    }

    const JOB: &str = "7d9f0c2e-1b3a-4c5d-8e6f-0a1b2c3d4e5f";

    /// The bytes reqwest sends for `.form(params)`.
    fn form_body<T: serde::Serialize + ?Sized>(params: &T) -> String {
        let request = reqwest::Client::new()
            .post("https://api.twilio.com/2010-04-01/Accounts/AC0/Messages.json")
            .form(params)
            .build()
            .unwrap();
        String::from_utf8(request.body().unwrap().as_bytes().unwrap().to_vec()).unwrap()
    }

    #[test]
    fn twilio_form_without_status_callback_is_unchanged() {
        let form = twilio_form("+15005550006", "+15005550006", "hello", None);
        // The array the connector posted before status callbacks.
        let before = [
            ("From", "+15005550006"),
            ("To", "+15005550006"),
            ("Body", "hello"),
        ];
        assert_eq!(form_body(&form), form_body(&before));
        assert_eq!(
            form_body(&form),
            "From=%2B15005550006&To=%2B15005550006&Body=hello"
        );
    }

    #[test]
    fn twilio_form_asks_for_the_job_status_callback() {
        let callback = status_callback(
            Some("https://notifyd.example.com/"),
            &json!({ JOB_ID_METADATA: JOB }),
        )
        .expect("public URL and job id");
        assert_eq!(
            callback,
            format!("https://notifyd.example.com/webhooks/twilio/status?job={JOB}")
        );
        let form = twilio_form("+15005550006", "+15005550006", "hello", Some(&callback));
        assert_eq!(
            form_body(&form),
            format!(
                "From=%2B15005550006&To=%2B15005550006&Body=hello\
                 &StatusCallback=https%3A%2F%2Fnotifyd.example.com%2Fwebhooks%2Ftwilio%2Fstatus%3Fjob%3D{JOB}"
            )
        );
    }

    #[test]
    fn no_status_callback_without_public_url_or_job_id() {
        assert_eq!(
            status_callback(None, &json!({ JOB_ID_METADATA: JOB })),
            None
        );
        assert_eq!(
            status_callback(Some("https://notifyd.example.com"), &json!({})),
            None
        );
        assert_eq!(
            status_callback(
                Some("https://notifyd.example.com"),
                &json!({ JOB_ID_METADATA: "not-a-job-id" })
            ),
            None
        );
    }

    #[test]
    fn gsm7_covers_french_text_but_not_every_accent_or_emoji() {
        for gsm in [
            "Rendez-vous mardi à 10h, répondez OUI ou NON.",
            "Où ? Ça déjà réglé : 30 € [code] ~ | ^ {} \\ @ £ ¥ § ¿ ¡ ß ñ ü ö ä Ä Ö Ñ Ü É Ç Ø Å Æ è ù ì ò",
            "Ligne 1\nLigne 2\r",
            "",
        ] {
            assert!(is_gsm7(gsm), "{gsm:?} is GSM 7-bit");
        }
        for unicode in [
            "Très bien 👍",
            "À bientôt",
            "Fête",
            "garçon",
            "l’apostrophe",
            "« guillemets »",
            "œuvre",
            "…",
            "tab\t",
        ] {
            assert!(!is_gsm7(unicode), "{unicode:?} needs UNICODE");
        }
    }

    #[test]
    fn smsmode_body_follows_the_api() {
        let job = Uuid::parse_str(JOB).unwrap();
        let callback = format!("https://notifyd.example.com/webhooks/smsmode/status?job={JOB}");
        assert_eq!(
            smsmode_body(
                "+33939030466",
                "+33786976911",
                "Bonjour, à mardi.",
                Some(job),
                Some(&callback)
            ),
            json!({
                "recipient": { "to": "33786976911" },
                "body": { "text": "Bonjour, à mardi." },
                "from": "33939030466",
                "refClient": JOB,
                "callbackUrlStatus": callback,
            })
        );
        assert_eq!(
            smsmode_body("CDMF", "+33786976911", "Fête 👍", None, None),
            json!({
                "recipient": { "to": "33786976911" },
                "body": { "text": "Fête 👍", "encoding": "UNICODE" },
                "from": "CDMF",
            })
        );
    }

    #[test]
    fn smsmode_asks_for_reports_only_when_it_can_check_them() {
        let job = Some(Uuid::parse_str(JOB).unwrap());
        let url = Some("https://notifyd.example.com");
        assert_eq!(
            smsmode_status_callback(url, true, job),
            Some(format!(
                "https://notifyd.example.com/webhooks/smsmode/status?job={JOB}"
            ))
        );
        assert_eq!(smsmode_status_callback(url, false, job), None);
        assert_eq!(smsmode_status_callback(None, true, job), None);
        assert_eq!(smsmode_status_callback(url, true, None), None);
    }

    #[test]
    fn smsmode_refuses_senders_over_11_characters() {
        let smsmode = SmsConfig {
            provider: "smsmode".to_string(),
            ..connector().config
        };
        assert_eq!(smsmode.sender_refusal("+33939030466"), None);
        assert_eq!(smsmode.sender_refusal("CDMF"), None);
        assert_eq!(smsmode.sender_refusal("36034"), None);
        assert!(smsmode.sender_refusal("+447700900123").is_some());
        assert_eq!(smsmode_sender("+33939030466"), "33939030466");
        assert_eq!(smsmode_sender("CDMF"), "CDMF");
    }

    #[test]
    fn a_201_smsmode_already_declares_undeliverable_fails_the_job() {
        let accepted = json!({"messageId": "m1", "status": {"value": "ENROUTE"}});
        assert_eq!(smsmode_refusal(&accepted), None);
        let delivered = json!({"messageId": "m1", "status": {"value": "DELIVERED"}});
        assert_eq!(smsmode_refusal(&delivered), None);
        let no_route = json!({"messageId": "m1", "status": {"value": "UNDELIVERABLE", "detail": "ROUTE_NOT_AVAILABLE"}});
        assert_eq!(
            smsmode_refusal(&no_route).as_deref(),
            Some("smsmode UNDELIVERABLE ROUTE_NOT_AVAILABLE")
        );
        let bare = json!({"messageId": "m1", "status": {"value": "UNDELIVERED"}});
        assert_eq!(
            smsmode_refusal(&bare).as_deref(),
            Some("smsmode UNDELIVERED")
        );
        assert_eq!(smsmode_refusal(&json!({})), None);
    }
}
