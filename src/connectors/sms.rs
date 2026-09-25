use super::{http_outcome, Channel, Connector, ProviderError, SendRequest, SendResult};
use crate::config::SmsConfig;
use async_trait::async_trait;
use serde_json::Value;
use tracing::info;

/// Metadata key holding the job id on an SMS request, set by the worker: the
/// Twilio connector puts it in the status callback URL.
pub const JOB_ID_METADATA: &str = "notifyd_job_id";

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
            _ => "telnyx",
        }
    }

    async fn send(&self, req: &SendRequest) -> SendResult {
        match self.config.provider.as_str() {
            "twilio" => self.send_twilio(req).await,
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
        // Telnyx sends an alphanumeric sender only through the messaging
        // profile that carries it.
        (self.provider == "telnyx"
            && is_alphanumeric_sender(from)
            && self.messaging_profile_id.is_none())
        .then_some("an alphanumeric sender needs TELNYX_MESSAGING_PROFILE_ID")
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

/// Where Twilio reports the message's delivery status: only when the
/// instance has a public URL and the request carries its job id.
fn status_callback(public_url: Option<&str>, metadata: &Value) -> Option<String> {
    let job_id = metadata
        .get(JOB_ID_METADATA)
        .and_then(Value::as_str)
        .and_then(|id| uuid::Uuid::parse_str(id).ok())?;
    Some(crate::twilio_status::callback_url(public_url?, job_id))
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
}
