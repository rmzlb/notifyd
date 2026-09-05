use crate::{
    connectors::{
        email::create_email_connector, in_app::InAppConnector, sms::SmsConnector, Channel,
        Connector, Delivery, ProviderError, ProviderErrorKind, SendRequest, SendResult,
    },
    db::{Job, JOB_COLUMNS},
    metrics, templates, workflow_engine, AppState,
};
use anyhow::Result;
use chrono::{Duration, Utc};
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::watch;
use tracing::{error, info, warn};

pub async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
    let interval = std::time::Duration::from_millis(state.config.worker.poll_interval_ms);
    info!(
        "Worker started, polling every {}ms, max {} attempts, email pacing {}/s",
        state.config.worker.poll_interval_ms,
        state.config.worker.max_attempts,
        state.config.worker.pacing.email_per_sec
    );

    let mut cleanup_interval = tokio::time::interval(std::time::Duration::from_secs(3600));
    cleanup_interval.tick().await; // skip first immediate tick
    let mut reaper_interval = tokio::time::interval(std::time::Duration::from_secs(60));
    reaper_interval.tick().await;

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
            _ = reaper_interval.tick() => {
                if let Err(e) = requeue_stuck_jobs(&state).await {
                    error!("Stuck job reaper error: {}", e);
                }
            }
        }
    }

    info!("Worker stopped");
}

/// Claim due jobs, most urgent first, skipping the lanes a provider asked
/// us to pause. The claim runs in one transaction with `FOR UPDATE SKIP
/// LOCKED`, so several workers never take the same job.
async fn process_batch(state: &Arc<AppState>) -> Result<()> {
    let now = Utc::now();
    let batch_size = state.config.worker.batch_size;
    let paused = state.pacer.paused_channels();

    let mut tx = state.pool.begin().await?;

    let jobs: Vec<Job> = sqlx::query_as(&format!(
        r#"
        SELECT {JOB_COLUMNS}
        FROM jobs
        WHERE status IN ('pending', 'retry')
          AND scheduled_at <= $1
          AND (next_retry_at IS NULL OR next_retry_at <= $1)
          AND NOT (channel = ANY($3))
        ORDER BY priority ASC, scheduled_at ASC
        LIMIT $2
        FOR UPDATE SKIP LOCKED
        "#
    ))
    .bind(now)
    .bind(batch_size)
    .bind(&paused)
    .fetch_all(&mut *tx)
    .await?;

    if jobs.is_empty() {
        tx.commit().await?;
        return Ok(());
    }

    info!("Processing {} jobs", jobs.len());

    for job in &jobs {
        sqlx::query(
            "UPDATE jobs SET status='processing', attempts=attempts+1, claimed_at=now() WHERE id=$1",
        )
            .bind(job.id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

    // Plain emails go through the connector's batch path (one provider call
    // for up to `batch_max` messages); emails with attachments and every
    // other channel dispatch one by one, a few in flight at a time.
    let (email_jobs, mut other_jobs): (Vec<_>, Vec<_>) =
        jobs.into_iter().partition(|j| j.channel == "email");
    let (email_batch_jobs, email_attachment_jobs): (Vec<_>, Vec<_>) = email_jobs
        .into_iter()
        .partition(|j| !job_has_attachments(j));
    other_jobs.extend(email_attachment_jobs);

    if !email_batch_jobs.is_empty() {
        process_email_batch(state, email_batch_jobs).await;
    }

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

/// Route plain email jobs through `send_batch()`: for Resend that is
/// `POST /emails/batch`; connectors without native batch loop `send()`.
async fn process_email_batch(state: &Arc<AppState>, jobs: Vec<Job>) {
    let config = match &state.config.connectors.email {
        Some(c) => c.clone(),
        None => {
            error!(
                "Email connector not configured — failing {} jobs",
                jobs.len()
            );
            for job in &jobs {
                finalize_job_result(
                    state,
                    job,
                    Err(ProviderError::permanent(
                        "none",
                        "email connector not configured",
                    )),
                )
                .await;
            }
            return;
        }
    };

    let mut prepared: Vec<(Job, SendRequest)> = Vec::with_capacity(jobs.len());
    for job in jobs {
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
                finalize_skipped(state, &job).await;
                continue;
            }
        }

        match build_send_request(state, &job).await {
            Ok(req) => prepared.push((job, req)),
            Err(e) => finalize_job_result(state, &job, Err(e)).await,
        }
    }

    if prepared.is_empty() {
        return;
    }

    let connector = create_email_connector(config);
    let chunk_size = connector.batch_max().max(1);
    for chunk in prepared.chunks(chunk_size) {
        // One provider request per chunk: one token.
        state.pacer.acquire("email").await;
        let reqs: Vec<SendRequest> = chunk.iter().map(|(_, r)| r.clone()).collect();
        let started = Instant::now();
        let mut results = connector.send_batch(&reqs).await;
        metrics::observe_latency(
            "email",
            connector.provider(),
            started.elapsed().as_secs_f64(),
        );

        // A provider rejects a whole batch (4xx) when a single item is bad,
        // e.g. one malformed address among a hundred. Sending the items one
        // by one lets the 99 good ones go and pins the failure on the bad one.
        if chunk.len() > 1 && all_permanent(&results) {
            warn!(
                "Email batch of {} rejected permanently — retrying items individually",
                chunk.len()
            );
            results = Vec::with_capacity(reqs.len());
            for req in &reqs {
                state.pacer.acquire("email").await;
                results.push(connector.send(req).await);
            }
        }

        for ((job, _), result) in chunk.iter().zip(results.into_iter()) {
            finalize_job_result(state, job, result).await;
        }
    }
}

fn all_permanent(results: &[SendResult]) -> bool {
    !results.is_empty()
        && results
            .iter()
            .all(|r| matches!(r, Err(e) if e.kind == ProviderErrorKind::Permanent))
}

/// Backoff for transient errors, by attempt number (1-based), with ±20 %
/// jitter so a burst of failures does not come back as one burst of retries.
pub fn retry_delay(attempt: i32) -> Duration {
    const SCHEDULE_SECS: [i64; 5] = [30, 120, 600, 1800, 7200];
    let index = (attempt.max(1) as usize - 1).min(SCHEDULE_SECS.len() - 1);
    let base = SCHEDULE_SECS[index];
    let jitter = jitter_fraction();
    let with_jitter = (base as f64 * (1.0 + jitter)).round() as i64;
    Duration::seconds(with_jitter.max(1))
}

/// Uniform in [-0.2, 0.2] from the clock: no RNG dependency needed for
/// spreading retries.
fn jitter_fraction() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (nanos % 401) as f64 / 1000.0 - 0.2
}

async fn finalize_skipped(state: &Arc<AppState>, job: &Job) {
    if let Err(e) = sqlx::query(
        "UPDATE jobs SET status='sent', sent_at=now(), error=NULL, provider='skipped' WHERE id=$1",
    )
    .bind(job.id)
    .execute(&state.pool)
    .await
    {
        error!("Failed to mark job {} as skipped: {}", job.id, e);
        return;
    }
    metrics::record_outcome(&job.channel, "skipped", "skipped");
    fire_terminal_webhooks(state, job, "sent");
}

/// Move the job to its next state from the provider's answer:
/// - accepted → `sent`, with provider and provider_message_id;
/// - rate limited → back to `retry` without consuming an attempt, and the
///   lane pauses for `Retry-After` (or the default pause);
/// - permanent or suppressed → `failed` now;
/// - transient → `retry` with backoff, `failed` after `max_attempts`.
async fn finalize_job_result(state: &Arc<AppState>, job: &Job, result: SendResult) {
    let new_status = match result {
        Ok(delivery) => {
            if let Err(e) = mark_sent(state, job, &delivery).await {
                error!("Failed to mark job {} as sent: {}", job.id, e);
                return;
            }
            metrics::record_outcome(&job.channel, delivery.provider, "sent");
            info!(
                "Job {} sent via {}{}",
                job.id,
                delivery.provider,
                delivery
                    .provider_message_id
                    .as_deref()
                    .map(|id| format!(" ({id})"))
                    .unwrap_or_default()
            );
            "sent"
        }
        Err(err) => {
            metrics::record_provider_error(&job.channel, err.provider, err.kind_label());
            let attempts = job.attempts + 1; // the claim already incremented the row
            let max = job.max_attempts.max(1);
            let err_msg = err.to_string();

            match &err.kind {
                ProviderErrorKind::RateLimited { retry_after } => {
                    let pause = state.pacer.pause(&job.channel, *retry_after);
                    metrics::record_lane_pause(&job.channel);
                    metrics::record_outcome(&job.channel, err.provider, "rate_limited");
                    let next_retry = Utc::now() + Duration::milliseconds(pause.as_millis() as i64);
                    if let Err(db_e) = sqlx::query(
                        "UPDATE jobs SET status='retry', attempts=GREATEST(attempts-1, 0), error=$2, next_retry_at=$3 WHERE id=$1",
                    )
                    .bind(job.id)
                    .bind(&err_msg)
                    .bind(next_retry)
                    .execute(&state.pool)
                    .await
                    {
                        error!("Failed to requeue rate-limited job {}: {}", job.id, db_e);
                        return;
                    }
                    warn!(
                        "Job {} rate limited by {} — lane {} paused {:?}",
                        job.id, err.provider, job.channel, pause
                    );
                    "retry"
                }
                ProviderErrorKind::Permanent | ProviderErrorKind::Suppressed => {
                    if let Err(db_e) = mark_failed(state, job, &err_msg).await {
                        error!("Failed to mark job {} as failed: {}", job.id, db_e);
                        return;
                    }
                    metrics::record_outcome(&job.channel, err.provider, "failed");
                    warn!(
                        "Job {} failed without retry ({}): {}",
                        job.id,
                        err.kind_label(),
                        err_msg
                    );
                    "failed"
                }
                ProviderErrorKind::Transient if attempts >= max => {
                    if let Err(db_e) = mark_failed(state, job, &err_msg).await {
                        error!("Failed to mark job {} as failed: {}", job.id, db_e);
                        return;
                    }
                    metrics::record_outcome(&job.channel, err.provider, "failed");
                    error!(
                        "Job {} failed permanently after {} attempts: {}",
                        job.id, attempts, err_msg
                    );
                    "failed"
                }
                ProviderErrorKind::Transient => {
                    let delay = retry_delay(attempts);
                    let next_retry = Utc::now() + delay;
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
                    metrics::record_outcome(&job.channel, err.provider, "retry");
                    warn!(
                        "Job {} retry in {}s (attempt {}/{}): {}",
                        job.id,
                        delay.num_seconds(),
                        attempts,
                        max,
                        err_msg
                    );
                    "retry"
                }
            }
        }
    };

    if new_status == "sent" || new_status == "failed" {
        fire_terminal_webhooks(state, job, new_status);
    }
}

async fn mark_sent(state: &Arc<AppState>, job: &Job, delivery: &Delivery) -> Result<()> {
    sqlx::query(
        "UPDATE jobs SET status='sent', sent_at=now(), error=NULL, provider=$2, provider_message_id=$3 WHERE id=$1",
    )
    .bind(job.id)
    .bind(delivery.provider)
    .bind(&delivery.provider_message_id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn mark_failed(state: &Arc<AppState>, job: &Job, error: &str) -> Result<()> {
    sqlx::query("UPDATE jobs SET status='failed', error=$2 WHERE id=$1")
        .bind(job.id)
        .bind(error)
        .execute(&state.pool)
        .await?;
    Ok(())
}

/// Outbound webhooks on terminal states, decoupled from the DB update.
fn fire_terminal_webhooks(state: &Arc<AppState>, job: &Job, status: &str) {
    let pool = state.pool.clone();
    let job_id = job.id;
    let channel = job.channel.clone();
    let subscriber_id = job.subscriber_id.clone();
    let project_id = job.project_id.clone();
    let status = status.to_string();
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

/// Resolve the per-project sender override for the email channel.
/// TOML-configured projects win (static config), then the projects table.
/// Any lookup failure falls back to the instance default — a bad from
/// must never block a send.
async fn resolve_project_from(
    state: &Arc<AppState>,
    project_id: &str,
) -> (Option<String>, Option<String>) {
    if let Some(p) = state.config.projects.get(project_id) {
        if p.from_email.is_some() {
            return (p.from_email.clone(), p.from_name.clone());
        }
    }
    match sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT from_email, from_name FROM projects WHERE id = $1",
    )
    .bind(project_id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => (None, None),
        Err(e) => {
            warn!(
                "from_email lookup failed for project {} ({}) — using instance default",
                project_id, e
            );
            (None, None)
        }
    }
}

/// Build the `SendRequest` for a job (template render + metadata). Shared by
/// the email batch path and the per-job dispatch.
async fn build_send_request(
    state: &Arc<AppState>,
    job: &Job,
) -> Result<SendRequest, ProviderError> {
    let (subject, body, body_html) = if let Some(tmpl_id) = &job.template_id {
        let tmpl: Option<crate::db::Template> = sqlx::query_as(
            "SELECT id, project_id, channel, subject, body, body_html FROM templates WHERE project_id=$1 AND id=$2 AND channel=$3"
        )
        .bind(&job.project_id).bind(tmpl_id).bind(&job.channel)
        .fetch_optional(&state.pool).await
        .map_err(|e| ProviderError::transient("database", e.to_string()))?;

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

    if job.channel == "email" {
        // Suppressed address (bounce/complaint): fail before the provider.
        if let Some(reason) =
            crate::deliverability::active_suppression(&state.pool, &job.project_id, &job.recipient)
                .await
                .map_err(|e| ProviderError::transient("database", e.to_string()))?
        {
            return Err(ProviderError::suppressed(reason));
        }

        // Tag the outgoing email with its job id: Resend echoes tags back in
        // webhook events, which is how a bounce finds its way to this job.
        if let Some(obj) = metadata.as_object_mut() {
            let tags = obj
                .entry("tags")
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Some(arr) = tags.as_array_mut() {
                arr.push(serde_json::json!({
                    "name": crate::deliverability::JOB_ID_TAG,
                    "value": job.id.to_string(),
                }));
            }
        }
    }

    let (from_email, from_name) = if job.channel == "email" {
        resolve_project_from(state, &job.project_id).await
    } else {
        (None, None)
    };

    Ok(SendRequest {
        recipient: job.recipient.clone(),
        subject,
        body,
        body_html,
        from_email,
        from_name,
        metadata,
    })
}

/// A job claimed more than `STUCK_AFTER` ago and still `processing` was in
/// the hands of a worker that died (crash, OOM, hard restart) before it could
/// record an outcome. Put it back in the queue; the attempt it consumed
/// stays consumed, so a job that crashes the worker every time still ends
/// `failed` instead of looping forever.
pub const STUCK_AFTER: std::time::Duration = std::time::Duration::from_secs(10 * 60);

async fn requeue_stuck_jobs(state: &Arc<AppState>) -> Result<()> {
    let requeued = sqlx::query(
        r#"
        UPDATE jobs
        SET status = CASE WHEN attempts >= max_attempts THEN 'failed' ELSE 'retry' END,
            next_retry_at = now(),
            error = COALESCE(error, '') || ' [requeued: worker lost the job while processing]'
        WHERE status = 'processing'
          AND claimed_at IS NOT NULL
          AND claimed_at < now() - make_interval(secs => $1)
        "#,
    )
    .bind(STUCK_AFTER.as_secs_f64())
    .execute(&state.pool)
    .await?;
    if requeued.rows_affected() > 0 {
        warn!(
            "Reaper: {} job(s) stuck in processing were re-queued",
            requeued.rows_affected()
        );
        metrics::record_outcome("all", "reaper", "requeued");
    }
    Ok(())
}

/// Cleanup old jobs periodically
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

async fn dispatch_job(state: &Arc<AppState>, job: &Job) -> SendResult {
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
            return Ok(Delivery::new("skipped", None));
        }
    }

    let req = build_send_request(state, job).await?;

    let connector: Box<dyn Connector> = match Channel::from_str(&job.channel) {
        Some(Channel::Email) => {
            let config = state.config.connectors.email.as_ref().ok_or_else(|| {
                ProviderError::permanent("none", "email connector not configured")
            })?;
            create_email_connector(config.clone())
        }
        Some(Channel::Sms) => {
            let config =
                state.config.connectors.sms.as_ref().ok_or_else(|| {
                    ProviderError::permanent("none", "SMS connector not configured")
                })?;
            Box::new(SmsConnector::new(config.clone()))
        }
        Some(Channel::Whatsapp) => {
            let config = state.config.connectors.whatsapp.as_ref().ok_or_else(|| {
                ProviderError::permanent("none", "WhatsApp connector not configured")
            })?;
            Box::new(crate::connectors::whatsapp::WhatsappConnector::new(
                config.clone(),
            ))
        }
        Some(Channel::InApp) => Box::new(InAppConnector::new(
            state.pool.clone(),
            state.broadcaster.clone(),
        )),
        Some(Channel::Push) => return dispatch_push(state, job, req).await,
        None => {
            return Err(ProviderError::permanent(
                "none",
                format!("unknown channel: {}", job.channel),
            ))
        }
    };

    let lane = connector.channel().as_str();
    state.pacer.acquire(lane).await;
    let started = Instant::now();
    let result = connector.send(&req).await;
    metrics::observe_latency(lane, connector.provider(), started.elapsed().as_secs_f64());
    result
}

/// Push fans out to every registered token of the subscriber. The job is
/// accepted when at least one token accepted it; a token that the push
/// service rejects permanently is dropped so it stops failing every send.
async fn dispatch_push(state: &Arc<AppState>, job: &Job, req: SendRequest) -> SendResult {
    let Some(sub_id) = &job.subscriber_id else {
        return Err(ProviderError::permanent(
            "web-push",
            "push requires subscriber_id",
        ));
    };
    let tokens: Vec<PushTokenRow> = sqlx::query_as(
        r#"
        SELECT id, token, platform, endpoint, p256dh, auth
        FROM push_tokens
        WHERE project_id=$1 AND subscriber_id=$2
        ORDER BY last_used_at DESC NULLS LAST, created_at DESC
        "#,
    )
    .bind(&job.project_id)
    .bind(sub_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| ProviderError::transient("database", e.to_string()))?;

    if tokens.is_empty() {
        warn!(
            "No push tokens for subscriber {} in project {}",
            sub_id, job.project_id
        );
        return Ok(Delivery::new("skipped", None));
    }

    let config = state
        .config
        .connectors
        .push
        .clone()
        .or_else(crate::config::PushConfig::from_env)
        .ok_or_else(|| ProviderError::permanent("none", "push connector not configured"))?;
    let connector = crate::connectors::push::PushConnector::new(config);

    let mut accepted: Option<Delivery> = None;
    let mut last_error: Option<ProviderError> = None;
    for token in tokens {
        let mut push_req = req.clone();
        if let (Some(endpoint), Some(p256dh), Some(auth)) =
            (&token.endpoint, &token.p256dh, &token.auth)
        {
            push_req.recipient = endpoint.clone();
            if let Some(obj) = push_req.metadata.as_object_mut() {
                obj.insert(
                    "web_push".into(),
                    serde_json::json!({
                        "endpoint": endpoint,
                        "p256dh": p256dh,
                        "auth": auth,
                        "platform": token.platform,
                        "token_id": token.id,
                    }),
                );
            }
        } else {
            push_req.recipient = token.token.clone();
        }
        state.pacer.acquire("push").await;
        let started = Instant::now();
        let result = connector.send(&push_req).await;
        metrics::observe_latency(
            "push",
            connector.provider(),
            started.elapsed().as_secs_f64(),
        );
        match result {
            Ok(delivery) => accepted = Some(delivery),
            Err(err) => {
                if err.kind == ProviderErrorKind::Permanent {
                    if let Err(db_e) = sqlx::query("DELETE FROM push_tokens WHERE id=$1")
                        .bind(token.id)
                        .execute(&state.pool)
                        .await
                    {
                        warn!("Could not drop dead push token {}: {}", token.id, db_e);
                    } else {
                        info!("Dropped dead push token {} ({})", token.id, err.message);
                    }
                }
                last_error = Some(err);
            }
        }
    }
    match (accepted, last_error) {
        (Some(delivery), _) => Ok(delivery),
        (None, Some(err)) => Err(err),
        (None, None) => Ok(Delivery::new("skipped", None)),
    }
}

#[derive(Debug, sqlx::FromRow)]
struct PushTokenRow {
    id: uuid::Uuid,
    token: String,
    platform: String,
    endpoint: Option<String>,
    p256dh: Option<String>,
    auth: Option<String>,
}

/// True when an email job carries a non-empty `attachments` array in its
/// payload. Such jobs must NOT go through Resend's `/emails/batch`
/// (which rejects attachments) — the worker routes them to single-send.
fn job_has_attachments(job: &Job) -> bool {
    job.payload
        .get("attachments")
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_schedule_grows_and_caps() {
        let seconds: Vec<i64> = (1..=7).map(|a| retry_delay(a).num_seconds()).collect();
        // Within ±20 % of 30 s, 2 min, 10 min, 30 min, 2 h, then capped at 2 h.
        let expected = [30, 120, 600, 1800, 7200, 7200, 7200];
        for (got, want) in seconds.iter().zip(expected.iter()) {
            let (lo, hi) = ((*want as f64 * 0.79) as i64, (*want as f64 * 1.21) as i64);
            assert!(*got >= lo && *got <= hi, "got {got} for expected ~{want}");
        }
    }

    #[test]
    fn whole_batch_permanent_rejection_is_detected() {
        let perm = || Err(ProviderError::permanent("resend", "bad"));
        assert!(all_permanent(&[perm(), perm()]));
        assert!(!all_permanent(&[
            perm(),
            Err(ProviderError::transient("resend", "503"))
        ]));
        assert!(!all_permanent(&[perm(), Ok(Delivery::new("resend", None))]));
        assert!(!all_permanent(&[]));
    }

    #[test]
    fn jitter_stays_within_bounds() {
        for _ in 0..50 {
            let j = jitter_fraction();
            assert!((-0.2..=0.2).contains(&j), "jitter {j}");
        }
    }
}
