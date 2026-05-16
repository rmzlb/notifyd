use crate::{
    connectors::{
        email::create_email_connector, in_app::InAppConnector, sms::SmsConnector, Channel,
        Connector, SendRequest,
    },
    db::Job,
    templates, workflow_engine, AppState,
};
use anyhow::Result;
use chrono::{Duration, Utc};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::watch;
use tracing::{error, info, warn};

pub async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
    let interval = std::time::Duration::from_millis(state.config.worker.poll_interval_ms);
    info!(
        "Worker started, polling every {}ms",
        state.config.worker.poll_interval_ms
    );

    let mut cleanup_interval = tokio::time::interval(std::time::Duration::from_secs(3600));
    cleanup_interval.tick().await; // skip first immediate tick

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                info!("Worker received shutdown signal");
                break;
            }
            _ = tokio::time::sleep(interval) => {
                if let Err(e) = process_batch(&state).await {
                    error!("Worker batch error: {}", e);
                }
                if let Err(e) = workflow_engine::resume_paused_runs(&state).await {
                    error!("Workflow resume error: {}", e);
                }
            }
            _ = cleanup_interval.tick() => {
                if let Err(e) = cleanup_old_jobs(&state).await {
                    error!("Job cleanup error: {}", e);
                }
            }
        }
    }

    info!("Worker stopped");
}

async fn process_batch(state: &Arc<AppState>) -> Result<()> {
    let now = Utc::now();
    let batch_size = state.config.worker.batch_size;

    // BUG FIX #1: Wrap in transaction for SELECT FOR UPDATE safety
    let mut tx = state.pool.begin().await?;

    let jobs: Vec<Job> = sqlx::query_as(
        r#"
        SELECT id, project_id, channel, subscriber_id, recipient, template_id, payload,
               status, scheduled_at, attempts, max_attempts, next_retry_at, idempotency_key,
               created_at, sent_at, error
        FROM jobs
        WHERE status IN ('pending', 'retry')
          AND scheduled_at <= $1
          AND (next_retry_at IS NULL OR next_retry_at <= $1)
        ORDER BY scheduled_at ASC
        LIMIT $2
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(now)
    .bind(batch_size)
    .fetch_all(&mut *tx)
    .await?;

    if jobs.is_empty() {
        tx.commit().await?;
        return Ok(());
    }

    info!("Processing {} jobs", jobs.len());

    for job in &jobs {
        sqlx::query("UPDATE jobs SET status='processing', attempts=attempts+1 WHERE id=$1")
            .bind(job.id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

    // Group email jobs together so we can send them via Resend's
    // `/emails/batch` endpoint (up to 100 per call) instead of N
    // individual API calls. Other channels (SMS, in-app, push) still
    // dispatch one-by-one but in parallel via for_each_concurrent.
    //
    // Why this matters at scale:
    // - Resend default rate limit is 5 req/s per team. With 50 jobs
    //   sequential @ ~50ms each, we burn ~5s wall clock and get throttled.
    //   With batch we make 1 call for 50 emails — same content, 50× less
    //   API surface, no rate limit pressure.
    // - The email connector trait has a default `send_batch` that just
    //   loops `send()`, so non-Resend connectors (AgentMail) still work
    //   correctly without being aware of batching.
    let (email_jobs, other_jobs): (Vec<_>, Vec<_>) =
        jobs.into_iter().partition(|j| j.channel == "email");

    // 1. Email batch path
    if !email_jobs.is_empty() {
        process_email_batch(state, email_jobs).await;
    }

    // 2. Other channels: parallel dispatch (up to 4 concurrent in-flight).
    //    SMS / in-app / push connectors typically have their own rate limits
    //    much higher than Resend's, but we cap at 4 to avoid overwhelming
    //    the DB pool when updating statuses.
    if !other_jobs.is_empty() {
        use futures::stream::{self, StreamExt};
        const PARALLEL_DISPATCH: usize = 4;
        stream::iter(other_jobs)
            .for_each_concurrent(PARALLEL_DISPATCH, |job| async move {
                let result = dispatch_job(state, &job).await;
                finalize_job_result(state, &job, result).await;
            })
            .await;
    }

    Ok(())
}

/// Process a batch of email jobs by routing them through the email
/// connector's `send_batch()` method. For Resend, this hits
/// `POST /emails/batch` (up to 100 emails per call); for connectors
/// without native batch the trait default loops `send()`.
///
/// Jobs that need a per-recipient subscriber preference check (opt-out
/// per channel/template) are filtered upstream — for emails we currently
/// only check at the project level (the global preference mechanism is
/// per-subscriber but channel-level only). If a project later adds
/// per-template opt-outs for email, this batch path needs to filter the
/// batch the same way `dispatch_job` does today.
async fn process_email_batch(state: &Arc<AppState>, jobs: Vec<Job>) {
    use crate::connectors::email::{create_email_connector, RESEND_BATCH_MAX};

    let config = match &state.config.connectors.email {
        Some(c) => c.clone(),
        None => {
            error!("Email connector not configured — failing {} jobs", jobs.len());
            for job in &jobs {
                finalize_job_result(state, job, Err(anyhow::anyhow!("Email connector not configured"))).await;
            }
            return;
        }
    };

    // Build the SendRequest for each job (template render etc.). If a job
    // is opted-out at the subscriber level, mark it sent (skipped) right
    // away — same semantic as `dispatch_job`'s skip path.
    let mut prepared: Vec<(Job, SendRequest)> = Vec::with_capacity(jobs.len());
    for job in jobs {
        // Subscriber preference check (per-channel opt-out)
        if let Some(sub_id) = &job.subscriber_id {
            if !workflow_engine::check_preference(
                state,
                &job.project_id,
                sub_id,
                &job.channel,
                job.template_id.as_deref(),
            )
            .await
            {
                info!(
                    "Job {} skipped (subscriber opted out of {} channel)",
                    job.id, job.channel
                );
                finalize_job_result(state, &job, Ok(())).await;
                continue;
            }
        }

        let req = match build_send_request(state, &job).await {
            Ok(r) => r,
            Err(e) => {
                finalize_job_result(state, &job, Err(e)).await;
                continue;
            }
        };
        prepared.push((job, req));
    }

    if prepared.is_empty() {
        return;
    }

    // Chunk by RESEND_BATCH_MAX (defensive — our default batch_size is 50,
    // but if the operator raises it >100 we still respect Resend's cap).
    let connector = create_email_connector(config);
    for chunk in prepared.chunks(RESEND_BATCH_MAX) {
        let reqs: Vec<SendRequest> = chunk.iter().map(|(_, r)| r.clone()).collect();
        let results = connector.send_batch(&reqs).await;

        // Match results back to jobs (same order guaranteed by send_batch contract).
        for ((job, _), result) in chunk.iter().zip(results.into_iter()) {
            finalize_job_result(state, job, result).await;
        }
    }
}

/// Update the job row to its terminal state (sent/failed/retry) and fire
/// webhooks on terminal states. Extracted so both the email batch path
/// and the parallel non-email path share the same status-update logic.
async fn finalize_job_result(state: &Arc<AppState>, job: &Job, result: Result<()>) {
    let new_status = match result {
        Ok(()) => {
            if let Err(e) = sqlx::query(
                "UPDATE jobs SET status='sent', sent_at=now(), error=NULL WHERE id=$1",
            )
            .bind(job.id)
            .execute(&state.pool)
            .await
            {
                error!("Failed to mark job {} as sent: {}", job.id, e);
                return;
            }
            info!("Job {} sent", job.id);
            "sent"
        }
        Err(e) => {
            let attempts = job.attempts + 1;
            let max = job.max_attempts;
            let err_msg = e.to_string();

            if attempts >= max {
                if let Err(db_e) = sqlx::query("UPDATE jobs SET status='failed', error=$2 WHERE id=$1")
                    .bind(job.id)
                    .bind(&err_msg)
                    .execute(&state.pool)
                    .await
                {
                    error!("Failed to mark job {} as failed: {}", job.id, db_e);
                    return;
                }
                error!(
                    "Job {} failed permanently after {} attempts: {}",
                    job.id, attempts, err_msg
                );
                "failed"
            } else {
                let delay_secs: i64 = match attempts {
                    1 => 30,
                    2 => 120,
                    _ => 600,
                };
                let next_retry = Utc::now() + Duration::seconds(delay_secs);
                if let Err(db_e) = sqlx::query(
                    "UPDATE jobs SET status='retry', error=$2, next_retry_at=$3 WHERE id=$1",
                )
                .bind(job.id)
                .bind(&err_msg)
                .bind(next_retry)
                .execute(&state.pool)
                .await
                {
                    error!("Failed to mark job {} as retry: {}", job.id, db_e);
                    return;
                }
                warn!(
                    "Job {} retry in {}s (attempt {}/{}): {}",
                    job.id, delay_secs, attempts, max, err_msg
                );
                "retry"
            }
        }
    };

    // Fire outbound webhooks on terminal states (decoupled from DB update).
    if new_status == "sent" || new_status == "failed" {
        let pool = state.pool.clone();
        let job_id = job.id;
        let channel = job.channel.clone();
        let subscriber_id = job.subscriber_id.clone();
        let project_id = job.project_id.clone();
        let status = new_status.to_string();
        tokio::spawn(async move {
            if let Err(e) = crate::webhooks::fire_webhooks(
                &pool,
                &project_id,
                &format!("job.{}", status),
                job_id,
                &channel,
                subscriber_id.as_deref(),
            )
            .await
            {
                warn!("Webhook fire error: {}", e);
            }
        });
    }
}

/// Build the `SendRequest` for a job (template render + metadata). Extracted
/// from the original `dispatch_job` so both the email batch path and the
/// per-job dispatch can share the prep work.
async fn build_send_request(state: &Arc<AppState>, job: &Job) -> Result<SendRequest> {
    let (subject, body, body_html) = if let Some(tmpl_id) = &job.template_id {
        let tmpl: Option<crate::db::Template> = sqlx::query_as(
            "SELECT id, project_id, channel, subject, body, body_html FROM templates WHERE project_id=$1 AND id=$2 AND channel=$3"
        )
        .bind(&job.project_id).bind(tmpl_id).bind(&job.channel)
        .fetch_optional(&state.pool).await?;

        if let Some(t) = tmpl {
            let vars = job
                .payload
                .get("vars")
                .cloned()
                .unwrap_or(job.payload.clone());
            (
                t.subject.map(|s| templates::render(&s, &vars)),
                templates::render(&t.body, &vars),
                t.body_html.map(|h| templates::render(&h, &vars)),
            )
        } else {
            inline_from_payload(&job.payload)
        }
    } else {
        inline_from_payload(&job.payload)
    };

    let mut metadata = job.payload.clone();
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert("project_id".into(), Value::String(job.project_id.clone()));
        if let Some(sid) = &job.subscriber_id {
            obj.insert("subscriber_id".into(), Value::String(sid.clone()));
        }
    }

    Ok(SendRequest {
        recipient: job.recipient.clone(),
        subject,
        body,
        body_html,
        metadata,
    })
}

/// Feature #8: Cleanup old jobs periodically
async fn cleanup_old_jobs(state: &Arc<AppState>) -> Result<()> {
    let sent_deleted = sqlx::query(
        "DELETE FROM jobs WHERE status IN ('sent', 'cancelled') AND created_at < now() - interval '7 days'"
    )
    .execute(&state.pool)
    .await?;

    let failed_deleted = sqlx::query(
        "DELETE FROM jobs WHERE status = 'failed' AND created_at < now() - interval '30 days'",
    )
    .execute(&state.pool)
    .await?;

    let total = sent_deleted.rows_affected() + failed_deleted.rows_affected();
    if total > 0 {
        info!(
            "Job cleanup: removed {} sent/cancelled, {} failed",
            sent_deleted.rows_affected(),
            failed_deleted.rows_affected()
        );
    }

    Ok(())
}

async fn dispatch_job(state: &Arc<AppState>, job: &Job) -> Result<()> {
    if let Some(sub_id) = &job.subscriber_id {
        if !workflow_engine::check_preference(
            state,
            &job.project_id,
            sub_id,
            &job.channel,
            job.template_id.as_deref(),
        )
        .await
        {
            info!(
                "Job {} skipped (subscriber opted out of {} channel)",
                job.id, job.channel
            );
            return Ok(());
        }
    }

    let (subject, body, body_html) = if let Some(tmpl_id) = &job.template_id {
        let tmpl: Option<crate::db::Template> = sqlx::query_as(
            "SELECT id, project_id, channel, subject, body, body_html FROM templates WHERE project_id=$1 AND id=$2 AND channel=$3"
        )
        .bind(&job.project_id).bind(tmpl_id).bind(&job.channel)
        .fetch_optional(&state.pool).await?;

        if let Some(t) = tmpl {
            let vars = job
                .payload
                .get("vars")
                .cloned()
                .unwrap_or(job.payload.clone());
            (
                t.subject.map(|s| templates::render(&s, &vars)),
                templates::render(&t.body, &vars),
                t.body_html.map(|h| templates::render(&h, &vars)),
            )
        } else {
            inline_from_payload(&job.payload)
        }
    } else {
        inline_from_payload(&job.payload)
    };

    let mut metadata = job.payload.clone();
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert("project_id".into(), Value::String(job.project_id.clone()));
        if let Some(sid) = &job.subscriber_id {
            obj.insert("subscriber_id".into(), Value::String(sid.clone()));
        }
    }

    let req = SendRequest {
        recipient: job.recipient.clone(),
        subject,
        body,
        body_html,
        metadata,
    };

    match Channel::from_str(&job.channel) {
        Some(Channel::Email) => {
            let config = state
                .config
                .connectors
                .email
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Email connector not configured"))?;
            create_email_connector(config.clone()).send(&req).await
        }
        Some(Channel::Sms) => {
            let config = state
                .config
                .connectors
                .sms
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("SMS connector not configured"))?;
            SmsConnector::new(config.clone()).send(&req).await
        }
        Some(Channel::InApp) => {
            InAppConnector::new(state.pool.clone(), state.broadcaster.clone())
                .send(&req)
                .await
        }
        Some(Channel::Push) => {
            if let Some(sub_id) = &job.subscriber_id {
                let tokens: Vec<(String,)> = sqlx::query_as(
                    "SELECT token FROM push_tokens WHERE project_id=$1 AND subscriber_id=$2",
                )
                .bind(&job.project_id)
                .bind(sub_id)
                .fetch_all(&state.pool)
                .await?;

                if tokens.is_empty() {
                    warn!(
                        "No push tokens for subscriber {} in project {}",
                        sub_id, job.project_id
                    );
                    return Ok(());
                }

                let server_key = std::env::var("FCM_SERVER_KEY").unwrap_or_default();
                if server_key.is_empty() {
                    return Err(anyhow::anyhow!("FCM_SERVER_KEY not configured"));
                }

                let connector = crate::connectors::push::PushConnector::new(
                    crate::connectors::push::FcmConfig { server_key },
                );

                for (token,) in tokens {
                    let mut push_req = req.clone();
                    push_req.recipient = token;
                    connector.send(&push_req).await?;
                }
                Ok(())
            } else {
                Err(anyhow::anyhow!("Push requires subscriber_id"))
            }
        }
        None => Err(anyhow::anyhow!("Unknown channel: {}", job.channel)),
    }
}

fn inline_from_payload(payload: &Value) -> (Option<String>, String, Option<String>) {
    (
        payload
            .get("subject")
            .and_then(|v| v.as_str())
            .map(String::from),
        payload
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        payload
            .get("body_html")
            .and_then(|v| v.as_str())
            .map(String::from),
    )
}
