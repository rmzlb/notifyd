//! Cloudflare Email Sending delivery events: the Cloudflare counterpart of
//! the Resend webhook (deliverability.rs).
//!
//! Cloudflare publishes outbound delivery events (delivered, deferred,
//! bounced, failed, rejected, complained) to a Cloudflare Queue through an
//! event subscription, one subscription per sending domain. notifyd pulls that
//! queue over the REST API — no Worker, no public route, no signing secret —
//! and applies the Resend rules: a delivery stamps the job, a hard bounce or a
//! complaint suppresses the address for the project, every event matched to a
//! job (and every unmatched bounce or complaint) is kept in provider_events
//! under the Resend event names, so digests and metrics count both providers.
//!
//! Job mapping: the send API answers with `result.message_id`
//! (connectors/cloudflare.rs), stored in `jobs.provider_message_id`; events
//! carry it in `payload.messageId`. Both are compared without angle brackets.
//!
//! Enabled by CLOUDFLARE_EMAIL_EVENTS_QUEUE_ID, with CLOUDFLARE_ACCOUNT_ID and
//! CLOUDFLARE_QUEUES_API_TOKEN (an account token limited to Queues Edit; the
//! queue needs an HTTP pull consumer). One queue per notifyd instance: two
//! instances pulling the same queue would each consume the other's events.

use crate::{
    deliverability::{self, Bounce, EventSource},
    AppState,
};
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tracing::{info, warn};
use uuid::Uuid;

/// Delivery events are minutes-scale facts; a 15 s poll keeps the queue
/// calls cheap (Queues bills three operations per message, not per poll).
const POLL_EVERY: Duration = Duration::from_secs(15);
const BATCH_SIZE: u32 = 100;
/// Time to record a batch before Cloudflare hands its messages out again.
const VISIBILITY_TIMEOUT_MS: u64 = 120_000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const EVENT_TYPE_PREFIX: &str = "cf.email.sending.message.";
const ACTOR: &str = "cloudflare-events";
/// A message put back after a failure waits 60 s per attempt, at most an
/// hour: a database outage must not burn the queue's retries in seconds.
const RETRY_STEP_SECS: u32 = 60;
const MAX_RETRY_DELAY_SECS: u32 = 3_600;

pub fn retry_delay_secs(attempts: u32) -> u32 {
    attempts
        .max(1)
        .saturating_mul(RETRY_STEP_SECS)
        .min(MAX_RETRY_DELAY_SECS)
}

/// An event can reach us before the worker has stored the message id on the
/// job (failover sends a batch item by item). An unmatched event is put back
/// once; from its second delivery on it is handled as unmatched.
pub fn last_chance(attempts: u32) -> bool {
    attempts >= 2
}

/// What notifyd does with an event.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Delivered,
    Bounced {
        permanent: bool,
        label: String,
        reason: String,
    },
    Complained,
    RecordOnly,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmailEvent {
    pub event_id: String,
    /// Normalized (no angle brackets), as stored on the job.
    pub message_id: String,
    pub recipient: Option<String>,
    /// Resend event name, the vocabulary of provider_events.
    pub event_type: &'static str,
    pub outcome: Outcome,
    pub payload: Value,
}

pub fn normalize_message_id(id: &str) -> String {
    id.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_string()
}

/// Pull consumers receive `json` messages base64-encoded and `text`
/// messages as is. Event subscriptions publish JSON objects: a body that is
/// already one is taken as is, whatever `CF-Content-Type` says.
pub fn decode_body(body: &str, _content_type: Option<&str>) -> Result<Value, String> {
    if let Ok(value @ Value::Object(_)) = serde_json::from_str::<Value>(body) {
        return Ok(value);
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .map_err(|_| "message body is neither JSON nor base64 JSON".to_string())?;
    serde_json::from_slice(&bytes).map_err(|_| "message body is not JSON".to_string())
}

pub fn parse_event(event: Value) -> Result<EmailEvent, String> {
    let kind = event
        .get("type")
        .and_then(Value::as_str)
        .and_then(|t| t.strip_prefix(EVENT_TYPE_PREFIX))
        .ok_or_else(|| "not an Email Sending event".to_string())?
        .to_string();
    let text = |pointer: &str| {
        event
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let event_id = text("/payload/eventId").ok_or("event without payload.eventId")?;
    let message_id = text("/payload/messageId")
        .map(|id| normalize_message_id(&id))
        .filter(|id| !id.is_empty())
        .ok_or("event without payload.messageId")?;
    let recipient = text("/payload/recipient");

    let (event_type, outcome) = match kind.as_str() {
        "delivered" => ("email.delivered", Outcome::Delivered),
        "bounced" => {
            // Cloudflare also sends `bounced` when temporary retries run out
            // (type "soft"): only "hard" proves the address is dead.
            let label = text("/payload/bounce/type").unwrap_or_else(|| "unknown".to_string());
            let reason = text("/payload/bounce/reason")
                .or_else(|| text("/payload/delivery/smtpResponse"))
                .unwrap_or_else(|| "bounced".to_string());
            let outcome = Outcome::Bounced {
                permanent: label.eq_ignore_ascii_case("hard"),
                label,
                reason,
            };
            ("email.bounced", outcome)
        }
        "complained" => ("email.complained", Outcome::Complained),
        "deferred" => ("email.delivery_delayed", Outcome::RecordOnly),
        "rejected" if text("/payload/rejection/reason").as_deref() == Some("suppressed") => {
            ("email.suppressed", Outcome::RecordOnly)
        }
        "rejected" | "failed" => ("email.failed", Outcome::RecordOnly),
        other => return Err(format!("unknown Email Sending event '{other}'")),
    };

    Ok(EmailEvent {
        event_id,
        message_id,
        recipient,
        event_type,
        outcome,
        payload: event,
    })
}

#[derive(Debug, PartialEq)]
pub enum Applied {
    Done,
    /// No job carries this message id yet; nothing was written.
    Unmatched,
}

/// Records one event and applies it. An error means the database failed:
/// the message goes back to the queue. Without `last_chance`, an event that
/// matches no job is left untouched so a later delivery can still match it.
pub async fn apply(
    state: &Arc<AppState>,
    event: &EmailEvent,
    last_chance: bool,
) -> Result<Applied> {
    let job_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM jobs WHERE provider = 'cloudflare' AND provider_message_id = $1 LIMIT 1",
    )
    .bind(&event.message_id)
    .fetch_optional(&state.pool)
    .await?;

    if job_id.is_none() && !last_chance {
        return Ok(Applied::Unmatched);
    }
    // Same rule as Resend: unmatched deliveries are noise (another app on
    // the domain), unmatched bounces and complaints are worth keeping.
    if job_id.is_none() && matches!(event.outcome, Outcome::Delivered | Outcome::RecordOnly) {
        return Ok(Applied::Done);
    }

    let provider_event_id = format!("cloudflare:{}", event.event_id);
    let inserted = sqlx::query(
        "INSERT INTO provider_events (id, provider, event_type, job_id, payload)
         VALUES ($1, 'cloudflare', $2, $3, $4)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(&provider_event_id)
    .bind(event.event_type)
    .bind(job_id)
    .bind(&event.payload)
    .execute(&state.pool)
    .await?;
    if inserted.rows_affected() == 0 {
        return Ok(Applied::Done);
    }

    let source = EventSource {
        provider_event_id: &provider_event_id,
        actor: ACTOR,
    };
    let recipients: Vec<String> = event.recipient.iter().cloned().collect();
    match &event.outcome {
        Outcome::Delivered => deliverability::mark_delivered(&state.pool, job_id).await,
        Outcome::Bounced {
            permanent,
            label,
            reason,
        } => {
            let bounce = Bounce {
                permanent: *permanent,
                label,
                message: reason,
            };
            deliverability::apply_bounce(state, job_id, &bounce, recipients, &source).await
        }
        Outcome::Complained => {
            let detail =
                deliverability::complaint_detail(event.payload.pointer("/payload/subject"));
            deliverability::apply_complaint(state, job_id, &detail, recipients, &source).await
        }
        Outcome::RecordOnly => info!(
            "Cloudflare event {} recorded without action",
            event.event_type
        ),
    }
    Ok(Applied::Done)
}

// ─── Queue consumer ─────────────────────────────────────────────────────────

struct QueueClient {
    http: reqwest::Client,
    base: String,
    token: String,
}

#[derive(Deserialize)]
struct PullResponse {
    #[serde(default)]
    result: Option<PullResult>,
}

#[derive(Deserialize)]
struct PullResult {
    #[serde(default)]
    messages: Vec<QueueMessage>,
}

#[derive(Deserialize)]
struct QueueMessage {
    #[serde(default)]
    body: String,
    lease_id: String,
    #[serde(default)]
    attempts: u32,
    #[serde(default)]
    metadata: Value,
}

/// A message to put back, and how long the queue keeps it before redelivery.
struct Retry {
    lease_id: String,
    delay_secs: u32,
}

impl QueueClient {
    async fn post(&self, action: &str, body: Value) -> Result<Value> {
        let response = self
            .http
            .post(format!("{}/{action}", self.base))
            .bearer_auth(&self.token)
            .timeout(REQUEST_TIMEOUT)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("queue {action}"))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!("queue {action}: HTTP {status}"));
        }
        serde_json::from_str(&text).with_context(|| format!("queue {action}: response"))
    }

    async fn pull(&self) -> Result<Vec<QueueMessage>> {
        let value = self
            .post(
                "pull",
                json!({ "batch_size": BATCH_SIZE, "visibility_timeout_ms": VISIBILITY_TIMEOUT_MS }),
            )
            .await?;
        let parsed: PullResponse = serde_json::from_value(value).context("queue pull: shape")?;
        Ok(parsed.result.map(|r| r.messages).unwrap_or_default())
    }

    async fn ack(&self, acks: Vec<String>, retries: Vec<Retry>) -> Result<()> {
        let acks: Vec<Value> = acks
            .into_iter()
            .map(|id| json!({ "lease_id": id }))
            .collect();
        let retries: Vec<Value> = retries
            .into_iter()
            .map(|r| json!({ "lease_id": r.lease_id, "delay_seconds": r.delay_secs }))
            .collect();
        self.post("ack", json!({ "acks": acks, "retries": retries }))
            .await
            .map(|_| ())
    }
}

/// Starts the consumer when CLOUDFLARE_EMAIL_EVENTS_QUEUE_ID is set. A
/// partial configuration is a start-up error, not a silent no-op.
pub fn spawn(state: Arc<AppState>) -> Result<()> {
    let env = |name: &str| {
        std::env::var(name)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let Some(queue_id) = env("CLOUDFLARE_EMAIL_EVENTS_QUEUE_ID") else {
        return Ok(());
    };
    let account_id = env("CLOUDFLARE_ACCOUNT_ID").ok_or_else(|| {
        anyhow!("CLOUDFLARE_EMAIL_EVENTS_QUEUE_ID is set but CLOUDFLARE_ACCOUNT_ID is not")
    })?;
    let token = env("CLOUDFLARE_QUEUES_API_TOKEN").ok_or_else(|| {
        anyhow!("CLOUDFLARE_EMAIL_EVENTS_QUEUE_ID is set but CLOUDFLARE_QUEUES_API_TOKEN is not")
    })?;
    let client = QueueClient {
        http: reqwest::Client::new(),
        base: format!(
            "https://api.cloudflare.com/client/v4/accounts/{account_id}/queues/{queue_id}/messages"
        ),
        token,
    };
    info!(
        "Cloudflare email events: pulling the queue every {}s",
        POLL_EVERY.as_secs()
    );
    tokio::spawn(async move {
        loop {
            match drain_once(&state, &client).await {
                // A full batch handled without failure means more is
                // waiting: pull again at once.
                Ok(drained) if drained.count == BATCH_SIZE as usize && drained.retried == 0 => {
                    continue
                }
                Ok(_) => {}
                Err(e) => warn!("Cloudflare email events: {:#}", e),
            }
            tokio::time::sleep(POLL_EVERY).await;
        }
    });
    Ok(())
}

struct Drained {
    count: usize,
    retried: usize,
}

/// One pull, applied and acknowledged.
async fn drain_once(state: &Arc<AppState>, client: &QueueClient) -> Result<Drained> {
    let messages = client.pull().await?;
    let count = messages.len();
    let (mut acks, mut retries) = (Vec::new(), Vec::new());
    let mut db_failures = 0;
    for message in messages {
        let content_type = message
            .metadata
            .get("CF-Content-Type")
            .and_then(Value::as_str);
        let event = decode_body(&message.body, content_type).and_then(parse_event);
        match event {
            Ok(event) => match apply(state, &event, last_chance(message.attempts)).await {
                Ok(Applied::Done) => acks.push(message.lease_id),
                Ok(Applied::Unmatched) => retries.push(Retry {
                    lease_id: message.lease_id,
                    delay_secs: RETRY_STEP_SECS,
                }),
                Err(e) => {
                    db_failures += 1;
                    if db_failures == 1 {
                        warn!("Cloudflare event {} not recorded: {:#}", event.event_id, e);
                    }
                    retries.push(Retry {
                        lease_id: message.lease_id,
                        delay_secs: retry_delay_secs(message.attempts),
                    });
                }
            },
            // Retrying cannot fix a message we cannot read.
            Err(reason) => {
                warn!("Cloudflare queue message dropped: {}", reason);
                acks.push(message.lease_id);
            }
        }
    }
    if db_failures > 1 {
        warn!(
            "Cloudflare events: {} events of this batch not recorded",
            db_failures
        );
    }
    let retried = retries.len();
    if !acks.is_empty() || !retries.is_empty() {
        client.ack(acks, retries).await?;
    }
    Ok(Drained { count, retried })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use serde_json::json;

    fn bounced(kind: &str) -> Value {
        json!({
            "type": "cf.email.sending.message.bounced",
            "source": {"type": "email.sending", "domain": "send.example.com"},
            "payload": {
                "eventId": "0190d0c4-7ea1-7af2-8b88-c1d2e3f4a5b6",
                "messageId": "<0101018f7d0c4d9a-msg-bounced@send.example.com>",
                "recipient": "user@example.net",
                "subject": "Your receipt",
                "terminal": true,
                "delivery": {"status": "bounced", "smtpResponse": "550 5.1.1 User unknown"},
                "bounce": {"type": kind, "classification": "permanent_failure", "reason": "550 5.1.1 User unknown"}
            },
            "metadata": {"eventSchemaVersion": 1}
        })
    }

    #[test]
    fn json_messages_arrive_base64_encoded() {
        let event = bounced("hard");
        let body = base64::engine::general_purpose::STANDARD.encode(event.to_string());
        assert_eq!(decode_body(&body, Some("json")).unwrap(), event);
    }

    #[test]
    fn plain_json_is_accepted_whatever_the_content_type() {
        let event = bounced("hard");
        assert_eq!(
            decode_body(&event.to_string(), Some("json")).unwrap(),
            event
        );
    }

    #[test]
    fn retries_back_off_and_unmatched_events_wait_once() {
        assert_eq!(retry_delay_secs(1), 60);
        assert_eq!(retry_delay_secs(3), 180);
        assert_eq!(retry_delay_secs(500), 3600);
        assert!(!last_chance(1));
        assert!(last_chance(2));
    }

    #[test]
    fn text_messages_arrive_as_plain_json() {
        let event = bounced("hard");
        assert_eq!(
            decode_body(&event.to_string(), Some("text")).unwrap(),
            event
        );
        assert_eq!(decode_body(&event.to_string(), None).unwrap(), event);
    }

    #[test]
    fn an_undecodable_body_is_an_error() {
        assert!(decode_body("not json, not base64 json", Some("json")).is_err());
        assert!(decode_body("", None).is_err());
    }

    #[test]
    fn hard_bounce_is_permanent_and_soft_bounce_is_not() {
        let hard = parse_event(bounced("hard")).unwrap();
        assert_eq!(hard.event_type, "email.bounced");
        assert_eq!(hard.event_id, "0190d0c4-7ea1-7af2-8b88-c1d2e3f4a5b6");
        assert_eq!(
            hard.message_id,
            "0101018f7d0c4d9a-msg-bounced@send.example.com"
        );
        assert_eq!(hard.recipient.as_deref(), Some("user@example.net"));
        assert_eq!(
            hard.outcome,
            Outcome::Bounced {
                permanent: true,
                label: "hard".to_string(),
                reason: "550 5.1.1 User unknown".to_string()
            }
        );
        let soft = parse_event(bounced("soft")).unwrap();
        assert!(matches!(
            soft.outcome,
            Outcome::Bounced {
                permanent: false,
                ..
            }
        ));
    }

    #[test]
    fn a_bounce_without_type_does_not_suppress() {
        let mut event = bounced("hard");
        event["payload"]["bounce"]
            .as_object_mut()
            .unwrap()
            .remove("type");
        let parsed = parse_event(event).unwrap();
        assert!(matches!(
            parsed.outcome,
            Outcome::Bounced {
                permanent: false,
                ..
            }
        ));
    }

    #[test]
    fn event_types_map_to_the_resend_vocabulary() {
        let of = |kind: &str, extra: Value| {
            let mut event = bounced("hard");
            event["type"] = json!(format!("cf.email.sending.message.{kind}"));
            event["payload"].as_object_mut().unwrap().remove("bounce");
            for (k, v) in extra.as_object().unwrap() {
                event["payload"][k] = v.clone();
            }
            parse_event(event).unwrap()
        };
        let delivered = of("delivered", json!({}));
        assert_eq!(delivered.event_type, "email.delivered");
        assert_eq!(delivered.outcome, Outcome::Delivered);

        let complained = of("complained", json!({"complaint": {"type": "abuse"}}));
        assert_eq!(complained.event_type, "email.complained");
        assert_eq!(complained.outcome, Outcome::Complained);

        assert_eq!(
            of("deferred", json!({})).event_type,
            "email.delivery_delayed"
        );
        assert_eq!(of("failed", json!({})).event_type, "email.failed");
        let suppressed = of("rejected", json!({"rejection": {"reason": "suppressed"}}));
        assert_eq!(suppressed.event_type, "email.suppressed");
        let spam = of("rejected", json!({"rejection": {"reason": "spam"}}));
        assert_eq!(spam.event_type, "email.failed");
        assert_eq!(spam.outcome, Outcome::RecordOnly);
    }

    #[test]
    fn events_without_ids_or_from_other_sources_are_refused() {
        let mut no_event_id = bounced("hard");
        no_event_id["payload"]
            .as_object_mut()
            .unwrap()
            .remove("eventId");
        assert!(parse_event(no_event_id).is_err());

        let mut no_message_id = bounced("hard");
        no_message_id["payload"]
            .as_object_mut()
            .unwrap()
            .remove("messageId");
        assert!(parse_event(no_message_id).is_err());

        let mut other_source = bounced("hard");
        other_source["type"] = json!("cf.workers.build.completed");
        assert!(parse_event(other_source).is_err());
    }

    /// The whole path on a real Postgres: event -> job, suppression,
    /// provider_events, idempotence. Like the other DB tests it only writes
    /// into a database named `*_test`, and removes its rows.
    #[tokio::test]
    async fn db_events_reach_the_job_and_the_suppression_list() {
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
        let pool = sqlx::PgPool::connect(&url).await.expect("connect");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrate");
        let project = format!("test-cloudflare-events-{}", Uuid::new_v4());
        let config: crate::config::Config = toml::from_str(&format!(
            r#"
            [server]
            port = 0
            jwt_secret = "cloudflare-events-test"
            [database]
            url = "{url}"
            [worker]
            [connectors]
            "#
        ))
        .expect("test config");
        let state = Arc::new(AppState {
            pool: pool.clone(),
            broadcaster: crate::sse::SseBroadcaster::new(pool.clone()),
            rate_limiter: crate::middleware::RateLimiter::new(),
            resend_webhook_secret: None,
            started_at: std::time::Instant::now(),
            pacer: Arc::new(crate::pacing::Pacer::new(config.worker.pacing.clone())),
            email_breaker: Arc::new(crate::failover::Breaker::new()),
            config,
        });
        sqlx::query("INSERT INTO projects (id, name) VALUES ($1, $1)")
            .bind(&project)
            .execute(&pool)
            .await
            .expect("project");
        let tag = Uuid::new_v4().simple().to_string();
        let sent_job = |recipient: &'static str, message_id: String| {
            let pool = pool.clone();
            let project = project.clone();
            async move {
                sqlx::query_scalar::<_, Uuid>(
                    "INSERT INTO jobs (project_id, channel, recipient, status, sent_at, provider, provider_message_id)
                     VALUES ($1, 'email', $2, 'sent', now(), 'cloudflare', $3) RETURNING id",
                )
                .bind(&project)
                .bind(recipient)
                .bind(message_id)
                .fetch_one(&pool)
                .await
                .expect("job")
            }
        };
        let event = |kind: &str, message_id: &str, recipient: &str, extra: Value| {
            let mut event = bounced("hard");
            event["type"] = json!(format!("cf.email.sending.message.{kind}"));
            event["payload"]["eventId"] = json!(format!("{tag}-{kind}-{message_id}"));
            event["payload"]["messageId"] = json!(format!("<{message_id}>"));
            event["payload"]["recipient"] = json!(recipient);
            event["payload"].as_object_mut().unwrap().remove("bounce");
            for (k, v) in extra.as_object().unwrap() {
                event["payload"][k] = v.clone();
            }
            parse_event(event).unwrap()
        };
        let suppressions = |email: &'static str| {
            let pool = pool.clone();
            let project = project.clone();
            async move {
                sqlx::query_scalar::<_, String>(
                    "SELECT reason FROM email_suppressions
                     WHERE project_id = $1 AND email = $2 AND released_at IS NULL",
                )
                .bind(&project)
                .bind(email)
                .fetch_all(&pool)
                .await
                .expect("suppressions")
            }
        };

        // Hard bounce: job bounced, address suppressed; the same event twice
        // changes nothing more.
        let bounced_id = format!("{tag}-a@send.example.com");
        let job = sent_job("dead@example.net", bounced_id.clone()).await;
        let hard = event(
            "bounced",
            &bounced_id,
            "dead@example.net",
            json!({"bounce": {"type": "hard", "reason": "550 5.1.1 User unknown"}}),
        );
        apply(&state, &hard, true).await.unwrap();
        apply(&state, &hard, true).await.unwrap();
        let (status, error): (String, Option<String>) =
            sqlx::query_as("SELECT status, error FROM jobs WHERE id = $1")
                .bind(job)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "bounced");
        assert_eq!(
            error.as_deref(),
            Some("bounced (hard): 550 5.1.1 User unknown")
        );
        assert_eq!(suppressions("dead@example.net").await, vec!["bounce"]);
        let recorded: Vec<(String, Option<Uuid>)> =
            sqlx::query_as("SELECT event_type, job_id FROM provider_events WHERE id = $1")
                .bind(format!("cloudflare:{}", hard.event_id))
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(recorded, vec![("email.bounced".to_string(), Some(job))]);

        // Delivered then complained: delivered_at stamped, address suppressed,
        // job stays sent.
        let delivered_id = format!("{tag}-b@send.example.com");
        let job = sent_job("reader@example.net", delivered_id.clone()).await;
        apply(
            &state,
            &event("delivered", &delivered_id, "reader@example.net", json!({})),
            true,
        )
        .await
        .unwrap();
        apply(
            &state,
            &event(
                "complained",
                &delivered_id,
                "reader@example.net",
                json!({"complaint": {"type": "abuse"}}),
            ),
            true,
        )
        .await
        .unwrap();
        let (status, delivered): (String, bool) =
            sqlx::query_as("SELECT status, delivered_at IS NOT NULL FROM jobs WHERE id = $1")
                .bind(job)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!((status.as_str(), delivered), ("sent", true));
        assert_eq!(suppressions("reader@example.net").await, vec!["complaint"]);

        // A delivery for a message notifyd did not send leaves no trace.
        let stranger = event(
            "delivered",
            &format!("{tag}-c@send.example.com"),
            "x@example.net",
            json!({}),
        );
        let kept = |event_id: String| {
            let pool = pool.clone();
            async move {
                sqlx::query_as::<_, (Option<Uuid>,)>(
                    "SELECT job_id FROM provider_events WHERE id = $1",
                )
                .bind(format!("cloudflare:{event_id}"))
                .fetch_all(&pool)
                .await
                .unwrap()
            }
        };
        assert_eq!(
            apply(&state, &stranger, false).await.unwrap(),
            Applied::Unmatched
        );
        assert_eq!(apply(&state, &stranger, true).await.unwrap(), Applied::Done);
        assert!(kept(stranger.event_id.clone()).await.is_empty());

        // An unmatched bounce waits for its second delivery (the job may not
        // carry the message id yet), then is kept without a job.
        let orphan = event(
            "bounced",
            &format!("{tag}-d@send.example.com"),
            "y@example.net",
            json!({"bounce": {"type": "hard", "reason": "550"}}),
        );
        assert_eq!(
            apply(&state, &orphan, false).await.unwrap(),
            Applied::Unmatched
        );
        assert!(kept(orphan.event_id.clone()).await.is_empty());
        assert_eq!(apply(&state, &orphan, true).await.unwrap(), Applied::Done);
        assert_eq!(kept(orphan.event_id.clone()).await, vec![(None,)]);

        sqlx::query("DELETE FROM provider_events WHERE id LIKE $1")
            .bind(format!("cloudflare:{tag}-%"))
            .execute(&pool)
            .await
            .expect("remove events");
        sqlx::query("DELETE FROM audit_log WHERE project_id = $1")
            .bind(&project)
            .execute(&pool)
            .await
            .expect("remove audit rows");
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

    #[test]
    fn message_ids_compare_without_angle_brackets() {
        assert_eq!(
            normalize_message_id(" <abc@send.example.com> "),
            "abc@send.example.com"
        );
        assert_eq!(
            normalize_message_id("abc@send.example.com"),
            "abc@send.example.com"
        );
    }
}
