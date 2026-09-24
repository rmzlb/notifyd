use super::{http_outcome, Channel, Connector, ProviderError, SendRequest, SendResult};
use crate::config::SmsConfig;
use async_trait::async_trait;
use serde_json::Value;
use tracing::info;

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

        let params = [
            ("From", self.sender(req)),
            ("To", req.recipient.as_str()),
            ("Body", req.body.as_str()),
        ];

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
        // Telnyx sends an alphanumeric sender only through the messaging
        // profile that carries it.
        if is_alphanumeric_sender(from) && self.config.messaging_profile_id.is_none() {
            return Err(ProviderError::permanent(
                "telnyx",
                "an alphanumeric sender needs TELNYX_MESSAGING_PROFILE_ID",
            ));
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
}
