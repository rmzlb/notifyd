//! Universal SMTP submission connector (lettre). Works with any provider
//! that offers SMTP: Amazon SES, Postmark, Brevo, Mailgun, OVH, Cloudflare
//! (`smtp.mx.cloudflare.net:465`), a local relay… Configure with
//! `EMAIL_PROVIDER=smtp`, `SMTP_HOST`, `SMTP_PORT`, `SMTP_USERNAME`,
//! `SMTP_PASSWORD`, `SMTP_SECURITY` (`starttls` | `tls` | `none`).

use super::{
    email::{cc_recipients, normalized_attachments, reply_to},
    Channel, Connector, Delivery, ProviderError, SendRequest, SendResult,
};
use crate::config::{EmailConfig, SmtpConfig};
use async_trait::async_trait;
use base64::Engine;
use lettre::{
    message::{
        header::{ContentType, HeaderName, HeaderValue},
        Attachment, Mailbox, MultiPart,
    },
    transport::smtp::{
        authentication::Credentials,
        client::{Tls, TlsParameters},
    },
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};
use std::time::Duration;
use tracing::info;

const PROVIDER: &str = "smtp";

pub struct SmtpConnector {
    config: EmailConfig,
    smtp: SmtpConfig,
}

impl SmtpConnector {
    pub fn new(config: EmailConfig) -> Self {
        let smtp = config.smtp.clone().unwrap_or_else(|| SmtpConfig {
            host: "localhost".to_string(),
            port: 25,
            username: None,
            password: None,
            security: "none".to_string(),
        });
        Self { config, smtp }
    }

    fn transport(&self) -> Result<AsyncSmtpTransport<Tokio1Executor>, ProviderError> {
        let tls_params = || {
            TlsParameters::new(self.smtp.host.clone())
                .map_err(|e| ProviderError::permanent(PROVIDER, format!("TLS setup: {e}")))
        };
        let tls = match self.smtp.security.as_str() {
            "tls" => Tls::Wrapper(tls_params()?),
            "none" => Tls::None,
            _ => Tls::Required(tls_params()?),
        };
        let mut builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&self.smtp.host)
            .port(self.smtp.port)
            .tls(tls)
            .timeout(Some(Duration::from_secs(20)));
        if let (Some(user), Some(pass)) = (&self.smtp.username, &self.smtp.password) {
            builder = builder.credentials(Credentials::new(user.clone(), pass.clone()));
        }
        Ok(builder.build())
    }

    fn from_mailbox(&self, req: &SendRequest) -> Result<Mailbox, ProviderError> {
        let (email, name) = match &req.from_email {
            Some(project_email) => (project_email.as_str(), req.from_name.as_deref()),
            None => (self.config.from.as_str(), self.config.from_name.as_deref()),
        };
        let address = email.parse().map_err(|e| {
            ProviderError::permanent(PROVIDER, format!("invalid sender {email}: {e}"))
        })?;
        Ok(Mailbox::new(name.map(String::from), address))
    }

    /// Build the MIME message. Returns it with its `Message-ID`, which is the
    /// only identifier an SMTP relay gives us back for later correlation.
    pub fn build_message(&self, req: &SendRequest) -> Result<(Message, String), ProviderError> {
        let from = self.from_mailbox(req)?;
        let to: Mailbox = req.recipient.parse().map_err(|e| {
            ProviderError::permanent(
                PROVIDER,
                format!(
                    "invalid recipient {}: {e}",
                    crate::pii::mask_email(&req.recipient)
                ),
            )
        })?;
        let domain = from.email.domain().to_string();
        let message_id = format!("<{}@{}>", uuid::Uuid::new_v4(), domain);

        let mut builder = Message::builder()
            .from(from)
            .to(to)
            .message_id(Some(message_id.clone()))
            .subject(req.subject.as_deref().unwrap_or("Notification"));

        for cc in cc_recipients(&req.metadata) {
            let mailbox: Mailbox = cc
                .parse()
                .map_err(|e| ProviderError::permanent(PROVIDER, format!("invalid cc {cc}: {e}")))?;
            builder = builder.cc(mailbox);
        }
        if let Some(reply) = reply_to(&req.metadata) {
            let mailbox: Mailbox = reply.parse().map_err(|e| {
                ProviderError::permanent(PROVIDER, format!("invalid reply_to {reply}: {e}"))
            })?;
            builder = builder.reply_to(mailbox);
        }
        if let Some(headers) = req
            .metadata
            .get("email_headers")
            .and_then(|h| h.as_object())
        {
            for (name, value) in headers {
                let Some(value) = value.as_str() else {
                    continue;
                };
                let header_name = HeaderName::new_from_ascii(name.clone()).map_err(|e| {
                    ProviderError::permanent(PROVIDER, format!("invalid header {name}: {e}"))
                })?;
                builder = builder.raw_header(HeaderValue::new(header_name, value.to_string()));
            }
        }

        let text = req.body.clone();
        let html = req.body_html.clone().unwrap_or_else(|| req.body.clone());
        let mut mixed = MultiPart::mixed().multipart(MultiPart::alternative_plain_html(text, html));
        for att in normalized_attachments(&req.metadata) {
            let filename = att["filename"].as_str().unwrap_or("attachment").to_string();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(att["content"].as_str().unwrap_or_default())
                .map_err(|e| {
                    ProviderError::permanent(
                        PROVIDER,
                        format!("attachment {filename} is not base64: {e}"),
                    )
                })?;
            let content_type = att
                .get("content_type")
                .and_then(|v| v.as_str())
                .map(ContentType::parse)
                .transpose()
                .map_err(|e| {
                    ProviderError::permanent(PROVIDER, format!("attachment {filename}: {e}"))
                })?
                .unwrap_or(ContentType::parse("application/octet-stream").expect("valid mime"));
            mixed = mixed.singlepart(Attachment::new(filename).body(bytes, content_type));
        }

        let message = builder
            .multipart(mixed)
            .map_err(|e| ProviderError::permanent(PROVIDER, format!("message build: {e}")))?;
        Ok((message, message_id))
    }
}

/// SMTP reply codes carry their own semantics: 4xx is transient (try again
/// later), 5xx is permanent, connection problems are transient.
pub fn classify_smtp_error(error: &lettre::transport::smtp::Error) -> ProviderError {
    let message = error.to_string();
    if error.is_permanent() {
        ProviderError::permanent(PROVIDER, message)
    } else if error.is_transient() || error.is_timeout() || error.is_transport_shutdown() {
        ProviderError::transient(PROVIDER, message)
    } else if error.is_client() || error.is_tls() {
        // Misconfiguration on our side (bad credentials, TLS handshake).
        ProviderError::permanent(PROVIDER, message)
    } else {
        ProviderError::transient(PROVIDER, message)
    }
}

#[async_trait]
impl Connector for SmtpConnector {
    fn channel(&self) -> Channel {
        Channel::Email
    }

    fn provider(&self) -> &'static str {
        PROVIDER
    }

    async fn send(&self, req: &SendRequest) -> SendResult {
        let (message, message_id) = self.build_message(req)?;
        let transport = self.transport()?;
        transport
            .send(message)
            .await
            .map_err(|e| classify_smtp_error(&e))?;
        info!(
            "Email sent via SMTP {} to {}",
            self.smtp.host,
            crate::pii::mask_email(&req.recipient)
        );
        Ok(Delivery::new(PROVIDER, Some(message_id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn connector() -> SmtpConnector {
        SmtpConnector::new(EmailConfig {
            provider: "smtp".to_string(),
            api_key: String::new(),
            from: "orders@philoeparis.fr".to_string(),
            from_name: Some("Philoé".to_string()),
            account_id: None,
            smtp: Some(SmtpConfig {
                host: "smtp.example.com".to_string(),
                port: 587,
                username: Some("user".to_string()),
                password: Some("pass".to_string()),
                security: "starttls".to_string(),
            }),
        })
    }

    #[test]
    fn builds_multipart_message_with_headers_and_attachment() {
        let req = SendRequest {
            recipient: "jane@example.com".to_string(),
            subject: Some("Facture".to_string()),
            body: "Bonjour".to_string(),
            body_html: Some("<p>Bonjour</p>".to_string()),
            from_email: None,
            from_name: None,
            metadata: json!({
                "reply_to": "support@philoeparis.fr",
                "cc": ["compta@example.com"],
                "email_headers": {"List-Unsubscribe": "<https://x/u>"},
                "attachments": [{"filename": "f.txt", "content": "QUJD", "content_type": "text/plain"}]
            }),
        };
        let (message, message_id) = connector().build_message(&req).unwrap();
        let raw = String::from_utf8(message.formatted()).unwrap();
        assert!(message_id.ends_with("@philoeparis.fr>"));
        assert!(raw.contains(&message_id));
        // Non-ASCII display names are RFC 2047 encoded; the address stays readable.
        assert!(raw.contains("<orders@philoeparis.fr>"));
        assert!(raw.contains("To: jane@example.com"));
        assert!(raw.contains("Reply-To: support@philoeparis.fr"));
        assert!(raw.contains("Cc: compta@example.com"));
        assert!(raw.contains("List-Unsubscribe: <https://x/u>"));
        assert!(raw.contains("multipart/alternative"));
        assert!(raw.contains("f.txt"));
    }

    #[test]
    fn invalid_recipient_is_permanent() {
        let req = SendRequest {
            recipient: "not-an-address".to_string(),
            subject: None,
            body: "x".to_string(),
            body_html: None,
            from_email: None,
            from_name: None,
            metadata: json!({}),
        };
        let err = connector().build_message(&req).unwrap_err();
        assert_eq!(err.kind, super::super::ProviderErrorKind::Permanent);
    }
}
