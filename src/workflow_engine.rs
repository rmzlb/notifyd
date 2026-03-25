use crate::{AppState, db::{Workflow, WorkflowRun, WorkflowStep}};
use anyhow::Result;
use chrono::{Utc, Duration};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{info, warn, error};
use uuid::Uuid;

/// Check if a subscriber has opted out of a channel/workflow
pub async fn check_preference(
    state: &AppState,
    project_id: &str,
    subscriber_id: &str,
    channel: &str,
    workflow_id: Option<&str>,
) -> bool {
    // Check specific workflow preference first
    if let Some(wf_id) = workflow_id {
        let specific: Option<(bool,)> = sqlx::query_as(
            "SELECT enabled FROM subscriber_preferences WHERE project_id=$1 AND subscriber_id=$2 AND channel=$3 AND workflow_id=$4"
        )
        .bind(project_id).bind(subscriber_id).bind(channel).bind(wf_id)
        .fetch_optional(&state.pool).await.ok().flatten();

        if let Some((enabled,)) = specific {
            return enabled;
        }
    }

    // Check wildcard channel preference
    let wildcard: Option<(bool,)> = sqlx::query_as(
        "SELECT enabled FROM subscriber_preferences WHERE project_id=$1 AND subscriber_id=$2 AND channel=$3 AND workflow_id='*'"
    )
    .bind(project_id).bind(subscriber_id).bind(channel)
    .fetch_optional(&state.pool).await.ok().flatten();

    if let Some((enabled,)) = wildcard {
        return enabled;
    }

    // Check global opt-out
    let global: Option<(bool,)> = sqlx::query_as(
        "SELECT enabled FROM subscriber_preferences WHERE project_id=$1 AND subscriber_id=$2 AND channel='*' AND workflow_id='*'"
    )
    .bind(project_id).bind(subscriber_id)
    .fetch_optional(&state.pool).await.ok().flatten();

    if let Some((enabled,)) = global {
        return enabled;
    }

    // Default: enabled
    true
}

/// Trigger all workflows matching an event
pub async fn trigger_event(
    state: &Arc<AppState>,
    project_id: &str,
    event: &str,
    subscriber_id: &str,
    payload: &Value,
) -> Result<Vec<Uuid>> {
    let workflows: Vec<Workflow> = sqlx::query_as(
        "SELECT id, project_id, name, description, trigger_event, steps, enabled, created_at, updated_at FROM workflows WHERE project_id=$1 AND trigger_event=$2 AND enabled=true"
    )
    .bind(project_id).bind(event)
    .fetch_all(&state.pool).await?;

    let mut run_ids = Vec::new();

    for wf in workflows {
        let run_id = start_workflow_run(state, &wf, subscriber_id, payload).await?;
        run_ids.push(run_id);
    }

    Ok(run_ids)
}

async fn start_workflow_run(
    state: &Arc<AppState>,
    workflow: &Workflow,
    subscriber_id: &str,
    payload: &Value,
) -> Result<Uuid> {
    let run_id: Uuid = sqlx::query_scalar(
        "INSERT INTO workflow_runs (project_id, workflow_id, subscriber_id, trigger_payload, current_step, status) VALUES ($1, $2, $3, $4, 0, 'running') RETURNING id"
    )
    .bind(&workflow.project_id)
    .bind(&workflow.id)
    .bind(subscriber_id)
    .bind(payload)
    .fetch_one(&state.pool)
    .await?;

    info!("Workflow run {} started for {}:{} event={}", run_id, workflow.project_id, subscriber_id, workflow.trigger_event);

    // Execute first step immediately
    advance_workflow(state, run_id).await?;

    Ok(run_id)
}

/// Advance a workflow run to the next step. Called by worker for paused runs.
pub async fn advance_workflow(state: &Arc<AppState>, run_id: Uuid) -> Result<()> {
    let run: Option<WorkflowRun> = sqlx::query_as(
        "SELECT id, project_id, workflow_id, subscriber_id, trigger_payload, current_step, status, step_state, resume_at, created_at, updated_at FROM workflow_runs WHERE id=$1"
    )
    .bind(run_id)
    .fetch_optional(&state.pool).await?;

    let run = match run {
        Some(r) if r.status == "running" || r.status == "paused" => r,
        _ => return Ok(()),
    };

    let workflow: Option<Workflow> = sqlx::query_as(
        "SELECT id, project_id, name, description, trigger_event, steps, enabled, created_at, updated_at FROM workflows WHERE project_id=$1 AND id=$2"
    )
    .bind(&run.project_id).bind(&run.workflow_id)
    .fetch_optional(&state.pool).await?;

    let workflow = match workflow {
        Some(w) => w,
        None => {
            sqlx::query("UPDATE workflow_runs SET status='failed', updated_at=now() WHERE id=$1")
                .bind(run_id).execute(&state.pool).await?;
            return Ok(());
        }
    };

    let steps: Vec<WorkflowStep> = serde_json::from_value(workflow.steps.clone())
        .unwrap_or_default();

    let mut current = run.current_step as usize;

    while current < steps.len() {
        let step = &steps[current];

        match step {
            WorkflowStep::Send { channel, template, subject, body, body_html } => {
                // Check preference before sending
                if !check_preference(state, &run.project_id, &run.subscriber_id, channel, Some(&run.workflow_id)).await {
                    info!("Workflow {} step {} skipped (preference opt-out)", run_id, current);
                    current += 1;
                    continue;
                }

                // Resolve recipient from subscriber
                let subscriber: Option<crate::db::Subscriber> = sqlx::query_as(
                    "SELECT id, project_id, email, phone, first_name, last_name, locale, data, created_at, updated_at FROM subscribers WHERE project_id=$1 AND id=$2"
                )
                .bind(&run.project_id).bind(&run.subscriber_id)
                .fetch_optional(&state.pool).await?;

                let recipient = match subscriber {
                    Some(sub) => match channel.as_str() {
                        "email" => sub.email.unwrap_or_default(),
                        "sms" => sub.phone.unwrap_or_default(),
                        "in_app" => sub.id.clone(),
                        _ => sub.id.clone(),
                    },
                    None => {
                        warn!("Subscriber {} not found for workflow {}", run.subscriber_id, run_id);
                        current += 1;
                        continue;
                    }
                };

                if recipient.is_empty() {
                    warn!("No {} recipient for subscriber {} in workflow {}", channel, run.subscriber_id, run_id);
                    current += 1;
                    continue;
                }

                // Merge trigger payload with step data
                let mut payload = run.trigger_payload.clone();
                if let Some(obj) = payload.as_object_mut() {
                    if let Some(s) = subject { obj.insert("subject".into(), json!(s)); }
                    if let Some(b) = body { obj.insert("body".into(), json!(b)); }
                    if let Some(bh) = body_html { obj.insert("body_html".into(), json!(bh)); }
                }

                // Enqueue job
                sqlx::query(
                    "INSERT INTO jobs (project_id, channel, subscriber_id, recipient, template_id, payload, scheduled_at) VALUES ($1, $2, $3, $4, $5, $6, now())"
                )
                .bind(&run.project_id)
                .bind(channel)
                .bind(&run.subscriber_id)
                .bind(&recipient)
                .bind(template.as_deref())
                .bind(&payload)
                .execute(&state.pool).await?;

                info!("Workflow {} step {}: {} job enqueued for {}", run_id, current, channel, recipient);
                current += 1;
            }

            WorkflowStep::Delay { duration_secs } => {
                let resume_at = Utc::now() + Duration::seconds(*duration_secs);
                sqlx::query("UPDATE workflow_runs SET status='paused', current_step=$2, resume_at=$3, updated_at=now() WHERE id=$1")
                    .bind(run_id)
                    .bind((current + 1) as i32)
                    .bind(resume_at)
                    .execute(&state.pool).await?;
                info!("Workflow {} paused at step {}, resume at {}", run_id, current, resume_at);
                return Ok(());
            }

            WorkflowStep::Condition { field, operator, value, on_true, on_false } => {
                let condition_met = evaluate_condition(state, &run, field, operator, value).await;
                let next = if condition_met {
                    on_true.unwrap_or(current + 1)
                } else {
                    on_false.unwrap_or(current + 1)
                };
                info!("Workflow {} step {} condition: {} {} {} = {} → step {}", 
                    run_id, current, field, operator, value, condition_met, next);
                current = next;
            }

            WorkflowStep::Digest { duration_secs, channel, template, subject, body } => {
                // First time: start collecting. Subsequent: buffer event, wait.
                let resume_at = Utc::now() + Duration::seconds(*duration_secs);

                // Buffer the current trigger payload
                sqlx::query("INSERT INTO digest_buffer (run_id, payload) VALUES ($1, $2)")
                    .bind(run_id).bind(&run.trigger_payload)
                    .execute(&state.pool).await?;

                // Pause and wait for digest window
                let mut state_json = run.step_state.clone();
                if let Some(obj) = state_json.as_object_mut() {
                    obj.insert("digest_channel".into(), json!(channel));
                    obj.insert("digest_template".into(), json!(template));
                    obj.insert("digest_subject".into(), json!(subject));
                    obj.insert("digest_body".into(), json!(body));
                }

                sqlx::query("UPDATE workflow_runs SET status='paused', current_step=$2, resume_at=$3, step_state=$4, updated_at=now() WHERE id=$1")
                    .bind(run_id)
                    .bind((current + 1) as i32)
                    .bind(resume_at)
                    .bind(&state_json)
                    .execute(&state.pool).await?;

                info!("Workflow {} digest step {}: collecting for {}s", run_id, current, duration_secs);
                return Ok(());
            }
        }
    }

    // All steps completed
    sqlx::query("UPDATE workflow_runs SET status='completed', current_step=$2, updated_at=now() WHERE id=$1")
        .bind(run_id)
        .bind(current as i32)
        .execute(&state.pool).await?;

    info!("Workflow run {} completed", run_id);
    Ok(())
}

/// Evaluate a condition step
async fn evaluate_condition(
    state: &AppState,
    run: &WorkflowRun,
    field: &str,
    operator: &str,
    expected: &Value,
) -> bool {
    match field {
        // Check if the last in-app notification was read
        "inbox.is_read" => {
            let is_read: Option<bool> = sqlx::query_scalar(
                "SELECT (read_at IS NOT NULL) FROM inbox_messages WHERE project_id=$1 AND subscriber_id=$2 ORDER BY created_at DESC LIMIT 1"
            )
            .bind(&run.project_id)
            .bind(&run.subscriber_id)
            .fetch_optional(&state.pool).await.ok().flatten();

            let actual = is_read.unwrap_or(false);
            match operator {
                "eq" => json!(actual) == *expected,
                "neq" => json!(actual) != *expected,
                _ => false,
            }
        }
        // Check trigger payload field
        _ if field.starts_with("payload.") => {
            let key = field.strip_prefix("payload.").unwrap_or("");
            let actual = run.trigger_payload.get(key).unwrap_or(&Value::Null);
            match operator {
                "eq" => actual == expected,
                "neq" => actual != expected,
                "gt" => actual.as_f64().unwrap_or(0.0) > expected.as_f64().unwrap_or(0.0),
                "lt" => actual.as_f64().unwrap_or(0.0) < expected.as_f64().unwrap_or(0.0),
                _ => false,
            }
        }
        _ => {
            warn!("Unknown condition field: {}", field);
            false
        }
    }
}

/// Called by the worker to resume paused workflow runs
pub async fn resume_paused_runs(state: &Arc<AppState>) -> Result<()> {
    let now = Utc::now();

    let runs: Vec<WorkflowRun> = sqlx::query_as(
        "SELECT id, project_id, workflow_id, subscriber_id, trigger_payload, current_step, status, step_state, resume_at, created_at, updated_at FROM workflow_runs WHERE status='paused' AND resume_at <= $1 LIMIT 50"
    )
    .bind(now)
    .fetch_all(&state.pool).await?;

    for run in runs {
        // Set back to running
        sqlx::query("UPDATE workflow_runs SET status='running', resume_at=NULL, updated_at=now() WHERE id=$1")
            .bind(run.id).execute(&state.pool).await?;

        // Check if this was a digest step — flush the buffer
        if run.step_state.get("digest_channel").is_some() {
            flush_digest(state, &run).await?;
        }

        // Continue execution
        if let Err(e) = advance_workflow(state, run.id).await {
            error!("Failed to advance workflow {}: {}", run.id, e);
            sqlx::query("UPDATE workflow_runs SET status='failed', updated_at=now() WHERE id=$1")
                .bind(run.id).execute(&state.pool).await?;
        }
    }

    Ok(())
}

/// Flush digest buffer and send aggregated notification
async fn flush_digest(state: &Arc<AppState>, run: &WorkflowRun) -> Result<()> {
    let items: Vec<(serde_json::Value,)> = sqlx::query_as(
        "SELECT payload FROM digest_buffer WHERE run_id=$1 ORDER BY created_at"
    )
    .bind(run.id)
    .fetch_all(&state.pool).await?;

    if items.is_empty() {
        return Ok(());
    }

    let channel = run.step_state["digest_channel"].as_str().unwrap_or("email");
    let template = run.step_state["digest_template"].as_str();
    let subject = run.step_state["digest_subject"].as_str();
    let body = run.step_state["digest_body"].as_str();

    // Resolve recipient
    let subscriber: Option<crate::db::Subscriber> = sqlx::query_as(
        "SELECT id, project_id, email, phone, first_name, last_name, locale, data, created_at, updated_at FROM subscribers WHERE project_id=$1 AND id=$2"
    )
    .bind(&run.project_id).bind(&run.subscriber_id)
    .fetch_optional(&state.pool).await?;

    let recipient = match subscriber {
        Some(sub) => match channel {
            "email" => sub.email.unwrap_or_default(),
            "sms" => sub.phone.unwrap_or_default(),
            _ => sub.id.clone(),
        },
        None => return Ok(()),
    };

    let digest_items: Vec<Value> = items.into_iter().map(|(p,)| p).collect();
    let payload = json!({
        "subject": subject,
        "body": body,
        "items": digest_items,
        "item_count": digest_items.len(),
        "vars": {
            "item_count": digest_items.len(),
            "items": digest_items,
        }
    });

    sqlx::query(
        "INSERT INTO jobs (project_id, channel, subscriber_id, recipient, template_id, payload, scheduled_at) VALUES ($1, $2, $3, $4, $5, $6, now())"
    )
    .bind(&run.project_id)
    .bind(channel)
    .bind(&run.subscriber_id)
    .bind(&recipient)
    .bind(template)
    .bind(&payload)
    .execute(&state.pool).await?;

    // Clear buffer
    sqlx::query("DELETE FROM digest_buffer WHERE run_id=$1")
        .bind(run.id)
        .execute(&state.pool).await?;

    info!("Digest flushed for workflow run {}: {} items via {}", run.id, digest_items.len(), channel);
    Ok(())
}
