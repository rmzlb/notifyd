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

    // Now dispatch each job (outside transaction to avoid long-held locks)
    for job in jobs {
        let result = dispatch_job(state, &job).await;

        let new_status;
        match result {
            Ok(()) => {
                sqlx::query("UPDATE jobs SET status='sent', sent_at=now(), error=NULL WHERE id=$1")
                    .bind(job.id)
                    .execute(&state.pool)
                    .await?;
                new_status = "sent";
                info!("Job {} sent", job.id);
            }
            Err(e) => {
                let attempts = job.attempts + 1;
                let max = job.max_attempts;
                let err_msg = e.to_string();

                if attempts >= max {
                    sqlx::query("UPDATE jobs SET status='failed', error=$2 WHERE id=$1")
                        .bind(job.id)
                        .bind(&err_msg)
                        .execute(&state.pool)
                        .await?;
                    new_status = "failed";
                    error!(
                        "Job {} failed permanently after {} attempts: {}",
                        job.id, attempts, err_msg
                    );
                } else {
                    let delay_secs: i64 = match attempts {
                        1 => 30,
                        2 => 120,
                        _ => 600,
                    };
                    let next_retry = Utc::now() + Duration::seconds(delay_secs);
                    sqlx::query(
                        "UPDATE jobs SET status='retry', error=$2, next_retry_at=$3 WHERE id=$1",
                    )
                    .bind(job.id)
                    .bind(&err_msg)
                    .bind(next_retry)
                    .execute(&state.pool)
                    .await?;
                    new_status = "retry";
                    warn!(
                        "Job {} retry in {}s (attempt {}/{}): {}",
                        job.id, delay_secs, attempts, max, err_msg
                    );
                }
            }
        }

        // Feature #10: Fire webhooks on terminal states
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

    Ok(())
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
