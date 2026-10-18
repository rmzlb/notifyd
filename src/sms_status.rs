//! What an SMS provider reports about a message it accepted, recorded on the
//! job. Shared by the Twilio (twilio_status.rs) and smsmode
//! (smsmode_status.rs) callbacks, which authenticate the provider and turn
//! its status into a [`Report`]:
//!   - `Delivered` stamps `delivered_at`, the job stays `sent`, and
//!     `job.delivered` fires;
//!   - `Failed` turns the job `bounced` with the provider's error, and
//!     `job.bounced` fires; a callback that beats the worker's write of the
//!     send finds the job still `processing`, and that write keeps the bounce
//!     (worker.rs `mark_sent`);
//!   - `Pending` changes nothing.
//!
//! The first terminal outcome wins: providers may post callbacks late, out of
//! order or twice, and none of them changes the job again or re-fires an
//! event.

use crate::AppState;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{debug, warn};
use uuid::Uuid;

/// A provider's status, in notifyd's terms.
#[derive(Debug, PartialEq)]
pub enum Report {
    Delivered,
    /// The error to keep on the bounced job, e.g. `twilio 30003`.
    Failed(String),
    /// Queued, sending, accepted by the carrier…: not final yet.
    Pending,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Outcome {
    Delivered,
    Bounced(String),
}

/// What a report changes, given the job's current `status` and whether it is
/// already `delivered`. A delivered job never bounces and a bounced one is
/// never delivered, so a late or repeated callback changes nothing. The
/// provider holds the message once it answered the send, which the worker
/// records a moment later: `processing` counts as `sent`.
pub(crate) fn outcome(status: &str, delivered: bool, report: &Report) -> Option<Outcome> {
    match report {
        Report::Delivered if !delivered && status != "bounced" => Some(Outcome::Delivered),
        Report::Failed(error) if !delivered && matches!(status, "sent" | "processing") => {
            Some(Outcome::Bounced(error.clone()))
        }
        _ => None,
    }
}

/// A job a callback changed, and the event that announces it.
#[derive(Debug, PartialEq)]
pub(crate) struct Changed {
    pub(crate) event: &'static str,
    pub(crate) channel: String,
    pub(crate) project_id: String,
    pub(crate) subscriber_id: Option<String>,
}

/// Records `report` on the SMS or RCS job `job_id` sent through `provider`,
/// when the job holds `message_id` (or no message yet). A callback for an
/// unknown job, another provider's job or another message changes nothing.
pub(crate) async fn record(
    pool: &PgPool,
    provider: &'static str,
    job_id: Uuid,
    message_id: &str,
    report: &Report,
) -> Result<Option<Changed>, sqlx::Error> {
    // `provider` stays NULL until the worker records the send, which a
    // callback can beat by a few milliseconds.
    let job: Option<(String, bool, Option<String>)> = sqlx::query_as(
        "SELECT status, delivered_at IS NOT NULL, provider_message_id FROM jobs
         WHERE id = $1 AND channel IN ('sms', 'rcs') AND (provider = $2 OR provider IS NULL)",
    )
    .bind(job_id)
    .bind(provider)
    .fetch_optional(pool)
    .await?;
    let Some((status, delivered, held_id)) = job else {
        debug!("{provider} status callback for unknown SMS/RCS job {job_id}: ignored");
        return Ok(None);
    };

    match held_id.as_deref() {
        Some(held) if held != message_id => {
            warn!(
                "{provider} status callback for job {job_id} names message {message_id}, the job holds {held}: ignored"
            );
            return Ok(None);
        }
        Some(_) => {}
        None => {
            sqlx::query(
                "UPDATE jobs SET provider_message_id = $2 WHERE id = $1 AND provider_message_id IS NULL",
            )
            .bind(job_id)
            .bind(message_id)
            .execute(pool)
            .await?;
        }
    }

    match outcome(&status, delivered, report) {
        Some(outcome) => apply(pool, job_id, message_id, outcome).await,
        None => Ok(None),
    }
}

type UpdatedRow = (String, String, Option<String>);

/// Each UPDATE re-checks the state the outcome was decided on, so two
/// callbacks racing each other still change the job once and fire one event.
async fn apply(
    pool: &PgPool,
    job_id: Uuid,
    message_id: &str,
    outcome: Outcome,
) -> Result<Option<Changed>, sqlx::Error> {
    let (event, changed): (&str, Option<UpdatedRow>) = match outcome {
        Outcome::Delivered => (
            "job.delivered",
            sqlx::query_as(
                "UPDATE jobs SET delivered_at = now()
                 WHERE id = $1 AND provider_message_id = $2
                   AND delivered_at IS NULL AND status <> 'bounced'
                 RETURNING channel, project_id, subscriber_id",
            )
            .bind(job_id)
            .bind(message_id)
            .fetch_optional(pool)
            .await?,
        ),
        Outcome::Bounced(error) => {
            let changed = sqlx::query_as(
                "UPDATE jobs SET status = 'bounced', bounced_at = now(), error = $3
                 WHERE id = $1 AND provider_message_id = $2
                   AND status IN ('sent', 'processing') AND delivered_at IS NULL
                 RETURNING channel, project_id, subscriber_id",
            )
            .bind(job_id)
            .bind(message_id)
            .bind(&error)
            .fetch_optional(pool)
            .await?;
            if changed.is_some() {
                warn!("Job {job_id} bounced ({error})");
            }
            ("job.bounced", changed)
        }
    };
    Ok(changed.map(|(channel, project_id, subscriber_id)| Changed {
        event,
        channel,
        project_id,
        subscriber_id,
    }))
}

/// Fires the project's webhooks for a job a callback changed.
pub(crate) fn announce(state: &Arc<AppState>, job_id: Uuid, changed: Option<Changed>) {
    if let Some(Changed {
        event,
        channel,
        project_id,
        subscriber_id,
    }) = changed
    {
        crate::deliverability::fire_job_event(
            state,
            &project_id,
            event,
            job_id,
            &channel,
            subscriber_id,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A report only ever changes a job of the provider that sent it, on a
    /// real Postgres. Like the other DB tests it needs `DATABASE_URL` set to
    /// a `*_test` database, and removes the rows it writes.
    #[tokio::test]
    async fn db_a_report_changes_only_its_own_provider_s_job() {
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
        let project = format!("test-sms-status-{}", Uuid::new_v4());
        sqlx::query("INSERT INTO projects (id, name) VALUES ($1, $1)")
            .bind(&project)
            .execute(&pool)
            .await
            .expect("project");
        let sent_job = |provider: &'static str, message_id: &'static str| {
            let pool = pool.clone();
            let project = project.clone();
            async move {
                sqlx::query_scalar::<_, Uuid>(
                    "INSERT INTO jobs (project_id, channel, recipient, status, provider, provider_message_id, sent_at)
                     VALUES ($1, 'sms', '+33600000000', 'sent', $2, $3, now()) RETURNING id",
                )
                .bind(&project)
                .bind(provider)
                .bind(message_id)
                .fetch_one(&pool)
                .await
                .expect("job")
            }
        };
        let state = |id: Uuid| {
            let pool = pool.clone();
            async move {
                sqlx::query_as::<_, (String, Option<String>, bool)>(
                    "SELECT status, error, delivered_at IS NOT NULL FROM jobs WHERE id = $1",
                )
                .bind(id)
                .fetch_one(&pool)
                .await
                .expect("job state")
            }
        };

        // An smsmode report naming a Twilio job's message changes nothing.
        let twilio_job = sent_job("twilio", "SM1").await;
        let failed = Report::Failed("smsmode SPAM".to_string());
        let changed = record(&pool, "smsmode", twilio_job, "SM1", &failed).await;
        assert_eq!(changed.unwrap(), None);
        assert_eq!(state(twilio_job).await, ("sent".to_string(), None, false));

        // On its own job it bounces it once.
        let smsmode_job = sent_job("smsmode", "m-1").await;
        let changed = record(&pool, "smsmode", smsmode_job, "m-1", &failed).await;
        assert_eq!(changed.unwrap().map(|c| c.event), Some("job.bounced"));
        let late = record(&pool, "smsmode", smsmode_job, "m-1", &Report::Delivered).await;
        assert_eq!(late.unwrap(), None);
        assert_eq!(
            state(smsmode_job).await,
            (
                "bounced".to_string(),
                Some("smsmode SPAM".to_string()),
                false
            )
        );

        // A pending report and another message's report leave a job as is.
        let other = sent_job("smsmode", "m-2").await;
        assert_eq!(
            record(&pool, "smsmode", other, "m-2", &Report::Pending)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            record(&pool, "smsmode", other, "m-9", &Report::Delivered)
                .await
                .unwrap(),
            None
        );
        let delivered = record(&pool, "smsmode", other, "m-2", &Report::Delivered).await;
        assert_eq!(delivered.unwrap().map(|c| c.event), Some("job.delivered"));
        assert_eq!(state(other).await, ("sent".to_string(), None, true));

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
