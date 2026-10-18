//! SMS delivery status from smsmode.
//!
//! An smsmode SMS job is `sent` as soon as smsmode accepts the API call; the
//! carrier's report comes back later. With `PUBLIC_URL` and
//! `SMSMODE_WEBHOOK_SECRET` set, the smsmode connector asks for delivery
//! reports on `{PUBLIC_URL}/webhooks/smsmode/status?job=<job id>`
//! (connectors/sms.rs). This route checks smsmode's signature and records the
//! outcome on the job (sms_status.rs): `DELIVERED` delivers it,
//! `UNDELIVERABLE` / `UNDELIVERED` bounce it with
//! `error = "smsmode <detail>"` (e.g. `smsmode SPAM`), every other status
//! (`SCHEDULED`, `ENROUTE`) changes nothing.
//!
//! smsmode signs the raw body with HMAC-SHA256 under the secret set in its
//! interface (Paramètres > Webhooks) and sends `X-Smsmode-256:
//! sha256=<hex>`. The route stays on while the secret is set, also after a
//! switch back to Twilio, for the messages smsmode still reports on. The
//! body also carries the recipient, the sender and the text: they are
//! covered by the signature and never logged, stored or echoed.

#[cfg(test)]
use crate::config::SmsConfig;
use crate::sms_status::Report;
use crate::AppState;
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use std::sync::Arc;
use tracing::{debug, error, warn};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// Route smsmode posts delivery reports to, below `PUBLIC_URL`.
pub const STATUS_PATH: &str = "/webhooks/smsmode/status";

/// A delivery report is about a kilobyte; anything bigger is refused (413)
/// before the signature is computed.
pub const BODY_LIMIT: usize = 64 * 1024;

/// smsmode refuses a `callbackUrlStatus` longer than this.
pub const CALLBACK_URL_MAX: usize = 255;

/// The `callbackUrlStatus` of one job's message, or `None` when it would
/// exceed what smsmode accepts: the message then leaves without one rather
/// than being refused.
pub fn callback_url(public_url: &str, job_id: Uuid) -> Option<String> {
    let url = format!(
        "{}{STATUS_PATH}?job={job_id}",
        public_url.trim_end_matches('/')
    );
    if url.len() > CALLBACK_URL_MAX {
        warn!(
            "smsmode delivery reports are off: the callback URL built from PUBLIC_URL is longer than {CALLBACK_URL_MAX} characters"
        );
        return None;
    }
    Some(url)
}

/// Constant-time check of `X-Smsmode-256: sha256=<hex HMAC-SHA256 of body>`.
fn verify(secret: &str, body: &[u8], header: Option<&str>) -> bool {
    let Some(signature) = header
        .and_then(|h| h.trim().strip_prefix("sha256="))
        .and_then(|hex_mac| hex::decode(hex_mac).ok())
    else {
        return false;
    };
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key size");
    mac.update(body);
    mac.verify_slice(&signature).is_ok()
}

/// 503 while the route cannot tell smsmode from anyone (no
/// `SMSMODE_WEBHOOK_SECRET`), 403 on a bad or missing signature. The 503 is
/// logged at debug only: anyone can post here, and the digest already names
/// the missing setting.
fn authenticate(
    secret: Option<&str>,
    body: &[u8],
    signature: Option<&str>,
) -> Result<(), StatusCode> {
    let Some(secret) = secret else {
        debug!("smsmode status callback received but the route is off: it needs SMSMODE_WEBHOOK_SECRET");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };
    if !verify(secret, body, signature) {
        warn!("smsmode status callback rejected: invalid X-Smsmode-256");
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}

/// smsmode's `status.value` (and `status.detail`) in notifyd's terms.
/// RCS also reports `READ` when the recipient opens the message.
fn report(value: &str, detail: Option<&str>) -> Report {
    match value {
        "DELIVERED" | "READ" => Report::Delivered,
        "UNDELIVERABLE" | "UNDELIVERED" => Report::Failed(bounce_error(value, detail)),
        _ => Report::Pending,
    }
}

/// `smsmode <detail>` (smsmode's reason, e.g. `SPAM`, `INVALID_PHONE_NUMBER`),
/// or `smsmode <status>` when the report gives no well-formed reason.
fn bounce_error(value: &str, detail: Option<&str>) -> String {
    match detail.map(str::trim) {
        Some(detail)
            if !detail.is_empty()
                && detail.len() <= 64
                && detail
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') =>
        {
            format!("smsmode {detail}")
        }
        _ => format!("smsmode {value}"),
    }
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    job: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Callback {
    message_id: String,
    direction: Option<String>,
    status: Option<CallbackStatus>,
    ref_client: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CallbackStatus {
    value: String,
    detail: Option<String>,
}

/// The job a report is about: `?job=` of the callback URL, else the
/// `refClient` the connector set to the job id.
fn job_id(query: Option<&str>, ref_client: Option<&str>) -> Option<Uuid> {
    query
        .or(ref_client)
        .and_then(|id| Uuid::parse_str(id.trim()).ok())
}

/// `POST /webhooks/smsmode/status` — no API key: authenticated by smsmode's
/// signature instead. A report for an unknown job, or for another message
/// than the one the job holds, is acknowledged and changes nothing. An
/// incoming message (`direction: MO`) posted here is acknowledged and
/// ignored: replies go to the URL the channel sets for them.
pub async fn status_callback(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CallbackQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, StatusCode> {
    let signature = headers.get("x-smsmode-256").and_then(|v| v.to_str().ok());
    authenticate(
        state.config.connectors.smsmode_webhook_secret(),
        &body,
        signature,
    )?;

    let callback: Callback = serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    if callback.direction.as_deref() == Some("MO") {
        debug!("smsmode incoming message posted to the status route: ignored");
        return Ok(StatusCode::OK);
    }
    let Some(status) = callback.status else {
        return Err(StatusCode::BAD_REQUEST);
    };
    let Some(job_id) = job_id(query.job.as_deref(), callback.ref_client.as_deref()) else {
        debug!(
            "smsmode status callback for message {} without a valid job id: ignored",
            callback.message_id
        );
        return Ok(StatusCode::OK);
    };

    let report = report(&status.value, status.detail.as_deref());
    let changed = crate::sms_status::record(
        &state.pool,
        "smsmode",
        job_id,
        &callback.message_id,
        &report,
    )
    .await
    .map_err(|e| {
        error!("smsmode status callback for job {job_id} not recorded: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    crate::sms_status::announce(&state, job_id, changed);
    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "fictional-smsmode-secret";
    const JOB: &str = "7d9f0c2e-1b3a-4c5d-8e6f-0a1b2c3d4e5f";
    const BODY: &str = r#"{"messageId":"67c15045-1067-4588-ba3c-737cc5051438","direction":"MT","recipient":{"to":"33600000000"},"from":"36034","status":{"value":"DELIVERED","deliveryDate":"2026-10-17T22:30:08"},"refClient":"7d9f0c2e-1b3a-4c5d-8e6f-0a1b2c3d4e5f"}"#;
    /// Computed apart from this code (Python hmac, hashlib.sha256) over BODY.
    const SIGNATURE: &str =
        "sha256=e635721b6f055fbb11e5a01f8f6eb488a12b42d263817be2c07e791e0affe036";

    fn sms(secret: Option<&str>) -> SmsConfig {
        SmsConfig {
            provider: "smsmode".to_string(),
            account_sid: None,
            auth_token: None,
            api_key: Some("fictional-smsmode-key".to_string()),
            messaging_profile_id: None,
            webhook_secret: secret.map(str::to_string),
            from: "36034".to_string(),
        }
    }

    #[test]
    fn signature_matches_the_independent_computation() {
        assert!(verify(SECRET, BODY.as_bytes(), Some(SIGNATURE)));
        assert!(verify(
            SECRET,
            BODY.as_bytes(),
            Some(&SIGNATURE.to_uppercase().replace("SHA256=", "sha256="))
        ));
    }

    #[test]
    fn verify_rejects_anything_smsmode_did_not_sign() {
        let body = BODY.as_bytes();
        assert!(!verify("another-secret", body, Some(SIGNATURE)));
        assert!(!verify(SECRET, body, None));
        assert!(!verify(SECRET, body, Some("")));
        assert!(!verify(
            SECRET,
            body,
            Some(SIGNATURE.trim_start_matches("sha256="))
        ));
        assert!(!verify(SECRET, body, Some("sha256=not-hex")));
        assert!(!verify(
            SECRET,
            body,
            Some(&SIGNATURE[..SIGNATURE.len() - 2])
        ));
        let tampered = BODY.replace("DELIVERED", "UNDELIVERED");
        assert!(!verify(SECRET, tampered.as_bytes(), Some(SIGNATURE)));
    }

    #[test]
    fn the_route_is_off_without_the_secret() {
        let body = BODY.as_bytes();
        let unavailable = Err(StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(authenticate(None, body, Some(SIGNATURE)), unavailable);
        assert_eq!(
            authenticate(sms(None).webhook_secret.as_deref(), body, Some(SIGNATURE)),
            unavailable
        );
        assert_eq!(
            authenticate(sms(Some(SECRET)).webhook_secret.as_deref(), body, None),
            Err(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            authenticate(
                sms(Some(SECRET)).webhook_secret.as_deref(),
                body,
                Some(SIGNATURE)
            ),
            Ok(())
        );
        // Switched back to Twilio with the smsmode secret left in place.
        let twilio = SmsConfig {
            provider: "twilio".to_string(),
            ..sms(Some(SECRET))
        };
        assert_eq!(
            authenticate(twilio.webhook_secret.as_deref(), body, Some(SIGNATURE)),
            Ok(())
        );
    }

    #[test]
    fn statuses_map_to_delivered_bounced_or_nothing() {
        assert_eq!(report("DELIVERED", None), Report::Delivered);
        assert_eq!(report("READ", None), Report::Delivered);
        assert_eq!(
            report("UNDELIVERABLE", Some("SPAM")),
            Report::Failed("smsmode SPAM".to_string())
        );
        assert_eq!(
            report("UNDELIVERED", Some("PHONE_NOT_SUITABLE")),
            Report::Failed("smsmode PHONE_NOT_SUITABLE".to_string())
        );
        assert_eq!(
            report("UNDELIVERED", None),
            Report::Failed("smsmode UNDELIVERED".to_string())
        );
        assert_eq!(
            report("UNDELIVERABLE", Some("INSUFFICIENT_CREDIT")),
            Report::Failed("smsmode INSUFFICIENT_CREDIT".to_string())
        );
        for value in ["SCHEDULED", "ENROUTE", "delivered", ""] {
            assert_eq!(
                report(value, Some("DELIVERED_TO_NETWORK")),
                Report::Pending,
                "{value}"
            );
        }
    }

    #[test]
    fn bounce_error_keeps_only_a_well_formed_reason() {
        assert_eq!(
            bounce_error("UNDELIVERABLE", Some(" SPAM ")),
            "smsmode SPAM"
        );
        assert_eq!(bounce_error("UNDELIVERED", Some("")), "smsmode UNDELIVERED");
        assert_eq!(
            bounce_error("UNDELIVERABLE", Some("SPAM +33600000000")),
            "smsmode UNDELIVERABLE"
        );
        assert_eq!(
            bounce_error("UNDELIVERABLE", Some(&"A".repeat(65))),
            "smsmode UNDELIVERABLE"
        );
    }

    #[test]
    fn a_report_names_its_job_by_query_then_ref_client() {
        let job = Uuid::parse_str(JOB).unwrap();
        assert_eq!(job_id(Some(JOB), None), Some(job));
        assert_eq!(job_id(None, Some(JOB)), Some(job));
        assert_eq!(job_id(Some(JOB), Some("not-a-job")), Some(job));
        assert_eq!(job_id(None, Some("notifyd-test-2")), None);
        assert_eq!(job_id(Some("x"), None), None);
        assert_eq!(job_id(None, None), None);
    }

    #[test]
    fn the_callback_body_smsmode_posts_parses() {
        let callback: Callback = serde_json::from_str(BODY).unwrap();
        assert_eq!(callback.message_id, "67c15045-1067-4588-ba3c-737cc5051438");
        assert_eq!(callback.direction.as_deref(), Some("MT"));
        let status = callback.status.unwrap();
        assert_eq!(status.value, "DELIVERED");
        assert_eq!(status.detail, None);
        assert_eq!(callback.ref_client.as_deref(), Some(JOB));
    }

    #[test]
    fn callback_url_names_the_job_and_fits_smsmode_s_limit() {
        let job = Uuid::parse_str(JOB).unwrap();
        let expected = format!("https://notifyd.example.com{STATUS_PATH}?job={JOB}");
        assert_eq!(
            callback_url("https://notifyd.example.com/", job).as_deref(),
            Some(expected.as_str())
        );
        let long = format!("https://{}.example.com", "a".repeat(200));
        assert_eq!(callback_url(&long, job), None);
    }
}
