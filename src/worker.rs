use crate::{
    AppState,
    connectors::{Channel, Connector, SendRequest, email::ResendConnector, sms::SmsConnector, in_app::InAppConnector},
    db::Job,
    templates,
};
use anyhow::Result;
use chrono::{Utc, Duration};
use serde_json::Value;
use std::sync::Arc;
use tracing::{info, warn, error};

pub async fn run(state: Arc<AppState>) {
    let interval = std::time::Duration::from_millis(state.config.worker.poll_interval_ms);
    info!("Worker started, polling every {}ms", state.config.worker.poll_interval_ms);

    loop {
        if let Err(e) = process_batch(&state).await {
            error!("Worker batch error: {}", e);
        }
        tokio::time::sleep(interval).await;
    }
}

async fn process_batch(state: &Arc<AppState>) -> Result<()> {
    let now = Utc::now();
    let batch_size = state.config.worker.batch_size;

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
    .fetch_all(&state.pool)
    .await?;

    if !jobs.is_empty() {
        info!("Processing {} jobs", jobs.len());
    }

    for job in jobs {
        // Mark as processing
        sqlx::query("UPDATE jobs SET status='processing', attempts=attempts+1 WHERE id=$1")
            .bind(job.id)
            .execute(&state.pool)
            .await?;

        let result = dispatch_job(state, &job).await;

        match result {
            Ok(()) => {
                sqlx::query("UPDATE jobs SET status='sent', sent_at=now(), error=NULL WHERE id=$1")
                    .bind(job.id)
                    .execute(&state.pool)
                    .await?;
                info!("Job {} sent", job.id);
            }
            Err(e) => {
                let attempts = job.attempts + 1;
                let max = job.max_attempts;
                let err_msg = e.to_string();

                if attempts >= max {
                    sqlx::query("UPDATE jobs SET status='failed', error=$2 WHERE id=$1")
                        .bind(job.id).bind(&err_msg)
                        .execute(&state.pool).await?;
                    error!("Job {} failed permanently after {} attempts: {}", job.id, attempts, err_msg);
                } else {
                    let delay_secs: i64 = match attempts {
                        1 => 30,
                        2 => 120,
                        _ => 600,
                    };
                    let next_retry = Utc::now() + Duration::seconds(delay_secs);
                    sqlx::query("UPDATE jobs SET status='retry', error=$2, next_retry_at=$3 WHERE id=$1")
                        .bind(job.id).bind(&err_msg).bind(next_retry)
                        .execute(&state.pool).await?;
                    warn!("Job {} retry in {}s (attempt {}/{}): {}", job.id, delay_secs, attempts, max, err_msg);
                }
            }
        }
    }

    Ok(())
}

async fn dispatch_job(state: &Arc<AppState>, job: &Job) -> Result<()> {
    // Resolve template if specified
    let (subject, body, body_html) = if let Some(tmpl_id) = &job.template_id {
        let tmpl: Option<crate::db::Template> = sqlx::query_as(
            "SELECT id, project_id, channel, subject, body, body_html FROM templates WHERE project_id=$1 AND id=$2 AND channel=$3"
        )
        .bind(&job.project_id).bind(tmpl_id).bind(&job.channel)
        .fetch_optional(&state.pool).await?;

        if let Some(t) = tmpl {
            let vars = job.payload.get("vars").cloned().unwrap_or(job.payload.clone());
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
            let config = state.config.connectors.email.as_ref()
                .ok_or_else(|| anyhow::anyhow!("Email connector not configured"))?;
            ResendConnector::new(config.clone()).send(&req).await
        }
        Some(Channel::Sms) => {
            let config = state.config.connectors.sms.as_ref()
                .ok_or_else(|| anyhow::anyhow!("SMS connector not configured"))?;
            SmsConnector::new(config.clone()).send(&req).await
        }
        Some(Channel::InApp) => {
            InAppConnector::new(state.pool.clone(), state.broadcaster.clone()).send(&req).await
        }
        None => Err(anyhow::anyhow!("Unknown channel: {}", job.channel)),
    }
}

fn inline_from_payload(payload: &Value) -> (Option<String>, String, Option<String>) {
    (
        payload.get("subject").and_then(|v| v.as_str()).map(String::from),
        payload.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        payload.get("body_html").and_then(|v| v.as_str()).map(String::from),
    )
}
