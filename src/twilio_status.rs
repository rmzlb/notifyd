//! SMS delivery status from Twilio.
//!
//! A Twilio SMS job is `sent` as soon as Twilio accepts the API call; what
//! the carrier did with it comes back later. With `PUBLIC_URL` set, the
//! Twilio connector asks for status callbacks on
//! `{PUBLIC_URL}/webhooks/twilio/status?job=<job id>` (connectors/sms.rs).
//! This route checks Twilio's signature and records the outcome on the job:
//!   - `delivered` stamps `delivered_at`, the job stays `sent`, and
//!     `job.delivered` fires;
//!   - `undelivered` / `failed` turn the job `bounced`, with
//!     `error = "twilio <ErrorCode>"`, and `job.bounced` fires; a callback
//!     that beats the worker's write of the send finds the job still
//!     `processing`, and that write keeps the bounce (worker.rs `mark_sent`);
//!   - every other status changes nothing.
//!
//! The first terminal outcome wins: Twilio may post callbacks late, out of
//! order or twice, and none of them changes the job again or re-fires an
//! event. Twilio also posts `To`, `From` and sometimes the message text: they
//! are covered by the signature and never logged, stored or echoed.

use crate::config::SmsConfig;
use crate::AppState;
use axum::{
    extract::{OriginalUri, Query, State},
    http::{uri::PathAndQuery, HeaderMap, StatusCode},
    Form,
};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha1::Sha1;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{debug, error, warn};
use uuid::Uuid;

type HmacSha1 = Hmac<Sha1>;

/// Route Twilio posts SMS statuses to, below `PUBLIC_URL`.
pub const STATUS_PATH: &str = "/webhooks/twilio/status";

/// A status callback is a few hundred bytes; the body is parsed before the
/// signature can be checked, so anything bigger is refused (413) up front.
pub const BODY_LIMIT: usize = 64 * 1024;

/// The `StatusCallback` of one job's message: the URL Twilio calls and signs.
pub fn callback_url(public_url: &str, job_id: Uuid) -> String {
    format!(
        "{}{STATUS_PATH}?job={job_id}",
        public_url.trim_end_matches('/')
    )
}

/// Twilio's signature scheme: HMAC-SHA1 keyed by the auth token over the
/// full URL Twilio called, followed by every POST parameter sorted by name,
/// each written as name then value with no separator.
fn signing_mac(auth_token: &str, url: &str, params: &[(String, String)]) -> HmacSha1 {
    let mut sorted: Vec<&(String, String)> = params.iter().collect();
    sorted.sort();
    let mut mac =
        HmacSha1::new_from_slice(auth_token.as_bytes()).expect("HMAC accepts any key size");
    mac.update(url.as_bytes());
    for (name, value) in sorted {
        mac.update(name.as_bytes());
        mac.update(value.as_bytes());
    }
    mac
}

/// Constant-time check of the base64 `X-Twilio-Signature` header.
fn verify(auth_token: &str, url: &str, params: &[(String, String)], header: Option<&str>) -> bool {
    let Some(signature) = header.and_then(|h| {
        base64::engine::general_purpose::STANDARD
            .decode(h.trim())
            .ok()
    }) else {
        return false;
    };
    signing_mac(auth_token, url, params)
        .verify_slice(&signature)
        .is_ok()
}

/// 503 while the route cannot tell Twilio from anyone (SMS provider other
/// than Twilio, no `TWILIO_AUTH_TOKEN`, no `PUBLIC_URL`), 403 on a bad or
/// missing signature. Twilio signs the URL it called: behind the proxy the
/// Host header is not that URL, so it is rebuilt from `PUBLIC_URL`. The 503
/// is logged at debug only: anyone can post here, and the digest already
/// names the missing setting.
fn authenticate(
    sms: Option<&SmsConfig>,
    public_url: Option<&str>,
    path_and_query: &str,
    params: &[(String, String)],
    signature: Option<&str>,
) -> Result<(), StatusCode> {
    let auth_token = sms
        .filter(|sms| sms.provider == "twilio")
        .and_then(|sms| sms.auth_token.as_deref());
    let (Some(auth_token), Some(public_url)) = (auth_token, public_url) else {
        debug!(
            "Twilio status callback received but the route is off: it needs SMS_PROVIDER=twilio, TWILIO_AUTH_TOKEN and PUBLIC_URL"
        );
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };
    let signed = signed_urls(public_url, path_and_query);
    if !signed
        .iter()
        .any(|url| verify(auth_token, url, params, signature))
    {
        warn!("Twilio status callback rejected: invalid X-Twilio-Signature");
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}

/// The URLs a genuine callback can be signed over: `PUBLIC_URL` then the path
/// and query, as written and with the port toggled. Twilio does not always
/// sign the port the way the URL was written, so its own SDKs accept both.
fn signed_urls(public_url: &str, path_and_query: &str) -> Vec<String> {
    let mut urls = vec![format!("{public_url}{path_and_query}")];
    if let Some(other) = toggle_port(public_url) {
        urls.push(format!("{other}{path_and_query}"));
    }
    urls
}

/// `https://host` gains `:443` (`http://host` gains `:80`); a URL written
/// with a port loses it. Same rule as Twilio's SDK validators.
fn toggle_port(public_url: &str) -> Option<String> {
    let (scheme, rest) = public_url.split_once("://")?;
    let default_port = match scheme {
        "https" => "443",
        "http" => "80",
        _ => return None,
    };
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let authority = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            host.to_string()
        }
        _ => format!("{authority}:{default_port}"),
    };
    Some(format!("{scheme}://{authority}{path}"))
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Delivered,
    Bounced(String),
}

/// What a callback changes, given the job's current `status` and whether it
/// is already `delivered`. A delivered job never bounces and a bounced one is
/// never delivered, so a late or repeated callback changes nothing. Twilio
/// holds the message once it answered the send, which the worker records a
/// moment later: `processing` counts as `sent`.
fn outcome(
    status: &str,
    delivered: bool,
    message_status: &str,
    error_code: Option<&str>,
) -> Option<Outcome> {
    match message_status {
        "delivered" if !delivered && status != "bounced" => Some(Outcome::Delivered),
        "undelivered" | "failed" if !delivered && matches!(status, "sent" | "processing") => {
            Some(Outcome::Bounced(bounce_error(message_status, error_code)))
        }
        _ => None,
    }
}

/// `twilio <ErrorCode>` (Twilio's numeric code, e.g. 30003), or
/// `twilio <status>` when the callback carries no numeric code.
fn bounce_error(message_status: &str, error_code: Option<&str>) -> String {
    match error_code.map(str::trim) {
        Some(code) if !code.is_empty() && code.bytes().all(|b| b.is_ascii_digit()) => {
            format!("twilio {code}")
        }
        _ => format!("twilio {message_status}"),
    }
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    job: Option<String>,
}

/// `POST /webhooks/twilio/status` — no API key: authenticated by Twilio's
/// signature instead. A callback for an unknown job, or for another message
/// than the one the job holds, is acknowledged and changes nothing.
pub async fn status_callback(
    State(state): State<Arc<AppState>>,
    OriginalUri(uri): OriginalUri,
    Query(query): Query<CallbackQuery>,
    headers: HeaderMap,
    Form(params): Form<Vec<(String, String)>>,
) -> Result<StatusCode, StatusCode> {
    let signature = headers
        .get("x-twilio-signature")
        .and_then(|v| v.to_str().ok());
    let path_and_query = uri
        .path_and_query()
        .map_or(uri.path(), PathAndQuery::as_str);
    authenticate(
        state.config.connectors.sms.as_ref(),
        crate::unsubscribe::public_url().as_deref(),
        path_and_query,
        &params,
        signature,
    )?;

    let param = |name: &str| {
        params
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    };
    let (Some(message_sid), Some(message_status)) = (param("MessageSid"), param("MessageStatus"))
    else {
        return Err(StatusCode::BAD_REQUEST);
    };
    let Some(job_id) = query.job.as_deref().and_then(|id| Uuid::parse_str(id).ok()) else {
        debug!("Twilio status callback for message {message_sid} without a valid job id: ignored");
        return Ok(StatusCode::OK);
    };

    let changed = record(
        &state.pool,
        job_id,
        message_sid,
        message_status,
        param("ErrorCode"),
    )
    .await
    .map_err(|e| {
        error!("Twilio status callback for job {job_id} not recorded: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    if let Some(Changed {
        event,
        project_id,
        subscriber_id,
    }) = changed
    {
        crate::deliverability::fire_job_event(
            &state,
            &project_id,
            event,
            job_id,
            "sms",
            subscriber_id,
        );
    }
    Ok(StatusCode::OK)
}

/// A job a callback changed, and the event that announces it.
#[derive(Debug, PartialEq)]
struct Changed {
    event: &'static str,
    project_id: String,
    subscriber_id: Option<String>,
}

async fn record(
    pool: &PgPool,
    job_id: Uuid,
    message_sid: &str,
    message_status: &str,
    error_code: Option<&str>,
) -> Result<Option<Changed>, sqlx::Error> {
    // `provider` stays NULL until the worker records the send, which a
    // callback can beat by a few milliseconds.
    let job: Option<(String, bool, Option<String>)> = sqlx::query_as(
        "SELECT status, delivered_at IS NOT NULL, provider_message_id FROM jobs
         WHERE id = $1 AND channel = 'sms' AND (provider = 'twilio' OR provider IS NULL)",
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await?;
    let Some((status, delivered, held_sid)) = job else {
        debug!("Twilio status callback for unknown SMS job {job_id}: ignored");
        return Ok(None);
    };

    match held_sid.as_deref() {
        Some(held) if held != message_sid => {
            warn!(
                "Twilio status callback for job {job_id} names message {message_sid}, the job holds {held}: ignored"
            );
            return Ok(None);
        }
        Some(_) => {}
        None => {
            sqlx::query(
                "UPDATE jobs SET provider_message_id = $2 WHERE id = $1 AND provider_message_id IS NULL",
            )
            .bind(job_id)
            .bind(message_sid)
            .execute(pool)
            .await?;
        }
    }

    match outcome(&status, delivered, message_status, error_code) {
        Some(outcome) => apply(pool, job_id, message_sid, outcome).await,
        None => Ok(None),
    }
}

/// Each UPDATE re-checks the state the outcome was decided on, so two
/// callbacks racing each other still change the job once and fire one event.
async fn apply(
    pool: &PgPool,
    job_id: Uuid,
    message_sid: &str,
    outcome: Outcome,
) -> Result<Option<Changed>, sqlx::Error> {
    let (event, changed): (&str, Option<(String, Option<String>)>) = match outcome {
        Outcome::Delivered => (
            "job.delivered",
            sqlx::query_as(
                "UPDATE jobs SET delivered_at = now()
                 WHERE id = $1 AND provider_message_id = $2
                   AND delivered_at IS NULL AND status <> 'bounced'
                 RETURNING project_id, subscriber_id",
            )
            .bind(job_id)
            .bind(message_sid)
            .fetch_optional(pool)
            .await?,
        ),
        Outcome::Bounced(error) => {
            let changed = sqlx::query_as(
                "UPDATE jobs SET status = 'bounced', bounced_at = now(), error = $3
                 WHERE id = $1 AND provider_message_id = $2
                   AND status IN ('sent', 'processing') AND delivered_at IS NULL
                 RETURNING project_id, subscriber_id",
            )
            .bind(job_id)
            .bind(message_sid)
            .bind(&error)
            .fetch_optional(pool)
            .await?;
            if changed.is_some() {
                warn!("Job {job_id} bounced ({error})");
            }
            ("job.bounced", changed)
        }
    };
    Ok(changed.map(|(project_id, subscriber_id)| Changed {
        event,
        project_id,
        subscriber_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "fictional-twilio-auth-token";
    const PUBLIC_URL: &str = "https://notifyd.example.com";
    const JOB: &str = "7d9f0c2e-1b3a-4c5d-8e6f-0a1b2c3d4e5f";
    /// Computed apart from this code (Python hmac, hashlib.sha1, base64) over
    /// `callback_url(PUBLIC_URL, JOB)` and `params()`, sorted by name.
    const SIGNATURE: &str = "hYYWoNWB4POmmEx+BxrVHmtuq4c=";

    fn job() -> Uuid {
        Uuid::parse_str(JOB).unwrap()
    }

    fn path() -> String {
        format!("{STATUS_PATH}?job={JOB}")
    }

    fn params() -> Vec<(String, String)> {
        [
            ("MessageStatus", "delivered"),
            ("To", "+15005550006"),
            ("From", "+15005550006"),
            ("AccountSid", "AC00000000000000000000000000000000"),
            ("ApiVersion", "2010-04-01"),
            ("MessageSid", "SM00000000000000000000000000000001"),
            ("SmsSid", "SM00000000000000000000000000000001"),
            ("SmsStatus", "delivered"),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
    }

    fn signature(auth_token: &str, url: &str, params: &[(String, String)]) -> String {
        base64::engine::general_purpose::STANDARD
            .encode(signing_mac(auth_token, url, params).finalize().into_bytes())
    }

    fn sms(provider: &str, auth_token: Option<&str>) -> SmsConfig {
        SmsConfig {
            provider: provider.to_string(),
            account_sid: Some("AC00000000000000000000000000000000".to_string()),
            auth_token: auth_token.map(str::to_string),
            api_key: None,
            messaging_profile_id: None,
            from: "+15005550006".to_string(),
        }
    }

    #[test]
    fn callback_url_names_the_job_below_public_url() {
        let expected = format!("{PUBLIC_URL}{}", path());
        assert_eq!(callback_url(PUBLIC_URL, job()), expected);
        assert_eq!(callback_url(&format!("{PUBLIC_URL}/"), job()), expected);
    }

    #[test]
    fn signature_matches_the_independent_computation() {
        let url = callback_url(PUBLIC_URL, job());
        assert_eq!(signature(TOKEN, &url, &params()), SIGNATURE);
        let mut reordered = params();
        reordered.reverse();
        assert_eq!(signature(TOKEN, &url, &reordered), SIGNATURE);
    }

    #[test]
    fn verify_rejects_anything_twilio_did_not_sign() {
        let url = callback_url(PUBLIC_URL, job());
        assert!(verify(TOKEN, &url, &params(), Some(SIGNATURE)));
        assert!(!verify("another-token", &url, &params(), Some(SIGNATURE)));
        assert!(!verify(TOKEN, &url, &params(), None));
        assert!(!verify(TOKEN, &url, &params(), Some("")));
        assert!(!verify(TOKEN, &url, &params(), Some("not base64!")));
        let mut tampered = params();
        tampered[0].1 = "undelivered".to_string();
        assert!(!verify(TOKEN, &url, &tampered, Some(SIGNATURE)));
        let other_job = callback_url(PUBLIC_URL, Uuid::nil());
        assert!(!verify(TOKEN, &other_job, &params(), Some(SIGNATURE)));
    }

    #[tokio::test]
    async fn a_body_posted_like_twilio_decodes_to_the_signed_parameters() {
        use axum::extract::FromRequest;
        let body = "MessageStatus=delivered&To=%2B15005550006&From=%2B15005550006\
                    &AccountSid=AC00000000000000000000000000000000&ApiVersion=2010-04-01\
                    &MessageSid=SM00000000000000000000000000000001\
                    &SmsSid=SM00000000000000000000000000000001&SmsStatus=delivered";
        let request = axum::http::Request::post(path())
            .header(
                "content-type",
                "application/x-www-form-urlencoded; charset=utf-8",
            )
            .body(axum::body::Body::from(body))
            .unwrap();
        let Form(posted) = Form::<Vec<(String, String)>>::from_request(request, &())
            .await
            .unwrap();
        assert_eq!(posted, params());
        let url = callback_url(PUBLIC_URL, job());
        assert!(verify(TOKEN, &url, &posted, Some(SIGNATURE)));
    }

    #[test]
    fn authenticate_rebuilds_the_url_from_public_url() {
        let twilio = sms("twilio", Some(TOKEN));
        assert_eq!(
            authenticate(
                Some(&twilio),
                Some(PUBLIC_URL),
                &path(),
                &params(),
                Some(SIGNATURE)
            ),
            Ok(())
        );
        // Signed for the address the container sees behind the proxy.
        let internal = signature(TOKEN, &format!("http://notifyd:3400{}", path()), &params());
        assert_eq!(
            authenticate(
                Some(&twilio),
                Some(PUBLIC_URL),
                &path(),
                &params(),
                Some(&internal)
            ),
            Err(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            authenticate(Some(&twilio), Some(PUBLIC_URL), &path(), &params(), None),
            Err(StatusCode::FORBIDDEN)
        );
    }

    #[test]
    fn a_signature_with_or_without_the_default_port_is_accepted() {
        let twilio = sms("twilio", Some(TOKEN));
        let check = |public_url: &str, signature: &str| {
            authenticate(
                Some(&twilio),
                Some(public_url),
                &path(),
                &params(),
                Some(signature),
            )
        };
        let with_port = signature(
            TOKEN,
            &format!("https://notifyd.example.com:443{}", path()),
            &params(),
        );
        assert_eq!(check(PUBLIC_URL, &with_port), Ok(()));
        // PUBLIC_URL written with its port, Twilio signing without it.
        assert_eq!(check("https://notifyd.example.com:443", SIGNATURE), Ok(()));
        let other_port = signature(
            TOKEN,
            &format!("https://notifyd.example.com:8443{}", path()),
            &params(),
        );
        assert_eq!(check(PUBLIC_URL, &other_port), Err(StatusCode::FORBIDDEN));
    }

    #[test]
    fn toggle_port_adds_the_default_port_or_removes_the_written_one() {
        let toggled = |url: &str| toggle_port(url);
        assert_eq!(
            toggled("https://n.example.com").as_deref(),
            Some("https://n.example.com:443")
        );
        assert_eq!(
            toggled("https://n.example.com:443").as_deref(),
            Some("https://n.example.com")
        );
        assert_eq!(
            toggled("http://api.example.com/notifyd").as_deref(),
            Some("http://api.example.com:80/notifyd")
        );
        assert_eq!(
            toggled("https://api.example.com:8443/notifyd").as_deref(),
            Some("https://api.example.com/notifyd")
        );
        assert_eq!(toggled("ftp://n.example.com"), None);
        assert_eq!(toggled("notifyd.example.com"), None);
    }

    #[test]
    fn the_route_is_off_without_twilio_its_token_or_public_url() {
        let off = |sms: Option<&SmsConfig>, public_url: Option<&str>| {
            authenticate(sms, public_url, &path(), &params(), Some(SIGNATURE))
        };
        let unavailable = Err(StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(off(None, Some(PUBLIC_URL)), unavailable);
        assert_eq!(
            off(Some(&sms("telnyx", Some(TOKEN))), Some(PUBLIC_URL)),
            unavailable
        );
        assert_eq!(
            off(Some(&sms("twilio", None)), Some(PUBLIC_URL)),
            unavailable
        );
        assert_eq!(off(Some(&sms("twilio", Some(TOKEN))), None), unavailable);
    }

    #[test]
    fn delivered_stamps_a_job_once_unless_it_bounced() {
        assert_eq!(
            outcome("sent", false, "delivered", None),
            Some(Outcome::Delivered)
        );
        // The callback beat the worker's write of the send.
        assert_eq!(
            outcome("processing", false, "delivered", None),
            Some(Outcome::Delivered)
        );
        assert_eq!(outcome("sent", true, "delivered", None), None);
        assert_eq!(outcome("bounced", false, "delivered", None), None);
    }

    #[test]
    fn undelivered_or_failed_bounce_a_sent_or_processing_job_once() {
        assert_eq!(
            outcome("sent", false, "undelivered", Some("30003")),
            Some(Outcome::Bounced("twilio 30003".to_string()))
        );
        assert_eq!(
            outcome("sent", false, "failed", Some("30008")),
            Some(Outcome::Bounced("twilio 30008".to_string()))
        );
        assert_eq!(
            outcome("bounced", false, "undelivered", Some("30003")),
            None
        );
        assert_eq!(outcome("sent", true, "undelivered", Some("30003")), None);
        // The callback beat the worker's write of the send.
        assert_eq!(
            outcome("processing", false, "failed", Some("30008")),
            Some(Outcome::Bounced("twilio 30008".to_string()))
        );
        for status in ["pending", "retry", "failed", "cancelled"] {
            assert_eq!(
                outcome(status, false, "undelivered", Some("30003")),
                None,
                "{status}"
            );
        }
    }

    #[test]
    fn other_statuses_change_nothing() {
        for message_status in [
            "queued",
            "sending",
            "sent",
            "accepted",
            "scheduled",
            "canceled",
            "read",
            "receiving",
            "received",
            "partially_delivered",
            "",
        ] {
            assert_eq!(
                outcome("sent", false, message_status, None),
                None,
                "{message_status}"
            );
        }
    }

    #[test]
    fn the_first_terminal_outcome_wins_and_fires_once() {
        // The job as the guarded UPDATEs leave it, and the events fired.
        fn replay(callbacks: &[&str]) -> (String, bool, Vec<&'static str>) {
            let (mut status, mut delivered, mut events) = ("sent".to_string(), false, Vec::new());
            for message_status in callbacks {
                match outcome(&status, delivered, message_status, Some("30003")) {
                    Some(Outcome::Delivered) => {
                        delivered = true;
                        events.push("job.delivered");
                    }
                    Some(Outcome::Bounced(_)) => {
                        status = "bounced".to_string();
                        events.push("job.bounced");
                    }
                    None => {}
                }
            }
            (status, delivered, events)
        }
        assert_eq!(
            replay(&["sent", "delivered", "delivered", "undelivered"]),
            ("sent".to_string(), true, vec!["job.delivered"])
        );
        assert_eq!(
            replay(&["undelivered", "failed", "delivered", "undelivered"]),
            ("bounced".to_string(), false, vec!["job.bounced"])
        );
    }

    #[test]
    fn bounce_error_keeps_only_a_numeric_code() {
        assert_eq!(bounce_error("undelivered", Some("30003")), "twilio 30003");
        assert_eq!(bounce_error("undelivered", Some(" 30005 ")), "twilio 30005");
        assert_eq!(bounce_error("failed", None), "twilio failed");
        assert_eq!(bounce_error("undelivered", Some("")), "twilio undelivered");
        assert_eq!(
            bounce_error("undelivered", Some("30003 +15005550006")),
            "twilio undelivered"
        );
    }

    async fn processing_sms_job(pool: &PgPool, project: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO jobs (project_id, channel, recipient, status)
             VALUES ($1, 'sms', '+15005550006', 'processing') RETURNING id",
        )
        .bind(project)
        .fetch_one(pool)
        .await
        .expect("job")
    }

    /// status, provider_message_id, error, bounced, delivered
    async fn job_state(
        pool: &PgPool,
        id: Uuid,
    ) -> (String, Option<String>, Option<String>, bool, bool) {
        sqlx::query_as(
            "SELECT status, provider_message_id, error, bounced_at IS NOT NULL, delivered_at IS NOT NULL
             FROM jobs WHERE id = $1",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("job state")
    }

    /// Callbacks and the worker's write of the send, in the order the race
    /// gives them, on a real Postgres. Like the schema smoke test it needs
    /// `DATABASE_URL` (CI sets one); it writes rows, so only into a database
    /// named `*_test`, and removes them.
    #[tokio::test]
    async fn db_the_worker_write_keeps_a_bounce_that_beat_it() {
        let Some(url) = std::env::var("DATABASE_URL").ok().filter(|url| {
            let without_query = url.split('?').next().unwrap_or_default();
            without_query
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .ends_with("_test")
        }) else {
            eprintln!("DATABASE_URL not set to a *_test database: skipping DB test");
            return;
        };
        let pool = PgPool::connect(&url).await.expect("connect");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrate");
        let project = format!("test-twilio-status-{}", Uuid::new_v4());
        sqlx::query("INSERT INTO projects (id, name) VALUES ($1, $1)")
            .bind(&project)
            .execute(&pool)
            .await
            .expect("project");
        const SID: &str = "SM00000000000000000000000000000001";
        const EARLIER: &str = "SM00000000000000000000000000000002";
        let twilio = |sid: &str| crate::connectors::Delivery::new("twilio", Some(sid.to_string()));
        let event = |changed: Option<Changed>| changed.map(|c| c.event);

        // Twilio's failure lands before the worker records the send.
        let job = processing_sms_job(&pool, &project).await;
        let changed = record(&pool, job, SID, "failed", Some("30008")).await;
        assert_eq!(event(changed.unwrap()), Some("job.bounced"));
        crate::worker::mark_sent(&pool, job, &twilio(SID))
            .await
            .unwrap();
        assert_eq!(
            job_state(&pool, job).await,
            (
                "bounced".to_string(),
                Some(SID.to_string()),
                Some("twilio 30008".to_string()),
                true,
                false
            )
        );
        let late = record(&pool, job, SID, "delivered", None).await;
        assert_eq!(late.unwrap(), None);

        // The bounce of an earlier attempt's message gives way to the send.
        let job = processing_sms_job(&pool, &project).await;
        let changed = record(&pool, job, EARLIER, "undelivered", Some("30003")).await;
        assert_eq!(event(changed.unwrap()), Some("job.bounced"));
        crate::worker::mark_sent(&pool, job, &twilio(SID))
            .await
            .unwrap();
        assert_eq!(
            job_state(&pool, job).await,
            (
                "sent".to_string(),
                Some(SID.to_string()),
                None,
                false,
                false
            )
        );
        let earlier = record(&pool, job, EARLIER, "delivered", None).await;
        assert_eq!(earlier.unwrap(), None);
        let changed = record(&pool, job, SID, "delivered", None).await;
        assert_eq!(event(changed.unwrap()), Some("job.delivered"));
        let late = record(&pool, job, SID, "undelivered", Some("30003")).await;
        assert_eq!(late.unwrap(), None);
        assert_eq!(
            job_state(&pool, job).await,
            ("sent".to_string(), Some(SID.to_string()), None, false, true)
        );

        sqlx::query("DELETE FROM jobs WHERE project_id = $1")
            .bind(&project)
            .execute(&pool)
            .await
            .expect("remove jobs");
        sqlx::query("DELETE FROM projects WHERE id = $1")
            .bind(&project)
            .execute(&pool)
            .await
            .expect("remove project");
    }
}
