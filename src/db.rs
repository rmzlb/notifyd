use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Job {
    pub id: Uuid,
    pub project_id: String,
    pub channel: String,
    pub subscriber_id: Option<String>,
    pub recipient: String,
    pub template_id: Option<String>,
    pub payload: serde_json::Value,
    pub status: String,
    pub scheduled_at: DateTime<Utc>,
    pub attempts: i32,
    pub max_attempts: i32,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub idempotency_key: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub sent_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    /// 0 = most urgent … 100 = bulk. Transactional defaults to 50, `/v1/batch`
    /// fan-outs to 80, so a campaign never delays an order confirmation.
    pub priority: i16,
    /// Provider that accepted the message and its own message id.
    pub provider: Option<String>,
    pub provider_message_id: Option<String>,
    /// When the worker claimed the job (see the stuck-job reaper).
    pub claimed_at: Option<DateTime<Utc>>,
    /// Subscriber-facing stream this message belongs to ("tips", "billing"…);
    /// preferences can opt out of it per channel. See migration 021.
    pub topic: Option<String>,
}

/// Column list shared by every `SELECT … FROM jobs` that loads a [`Job`].
pub const JOB_COLUMNS: &str = "id, project_id, channel, subscriber_id, recipient, template_id, payload, status, scheduled_at, attempts, max_attempts, next_retry_at, idempotency_key, created_at, sent_at, error, priority, provider, provider_message_id, claimed_at, topic";

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct InboxMessage {
    pub id: Uuid,
    pub project_id: String,
    pub subscriber_id: String,
    pub body: String,
    pub icon: Option<String>,
    pub url: Option<String>,
    pub data: Option<serde_json::Value>,
    pub read_at: Option<DateTime<Utc>>,
    pub archived_at: Option<DateTime<Utc>>,
    pub is_todo: bool,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Subscriber {
    pub id: String,
    pub project_id: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub locale: Option<String>,
    pub data: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Template {
    pub id: String,
    pub project_id: String,
    pub channel: String,
    pub subject: Option<String>,
    pub body: String,
    pub body_html: Option<String>,
    /// Default topic for messages sent with this template.
    pub topic: Option<String>,
}

/// Column list shared by every `SELECT … FROM templates` that loads a [`Template`].
pub const TEMPLATE_COLUMNS: &str = "id, project_id, channel, subject, body, body_html, topic";

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SubscriberPreference {
    pub project_id: String,
    pub subscriber_id: String,
    pub channel: String,
    pub workflow_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Workflow {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub description: Option<String>,
    pub trigger_event: String,
    pub steps: serde_json::Value,
    pub enabled: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WorkflowRun {
    pub id: Uuid,
    pub project_id: String,
    pub workflow_id: String,
    pub subscriber_id: String,
    pub trigger_payload: serde_json::Value,
    pub current_step: i32,
    pub status: String,
    pub step_state: serde_json::Value,
    pub resume_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PushToken {
    pub id: Uuid,
    pub project_id: String,
    pub subscriber_id: String,
    pub token: String,
    pub platform: String,
    pub device_name: Option<String>,
    pub endpoint: Option<String>,
    pub p256dh: Option<String>,
    pub auth: Option<String>,
    pub expiration_time: Option<DateTime<Utc>>,
    pub user_agent: Option<String>,
}

/// Workflow step types (serialized as JSON in the steps array)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WorkflowStep {
    #[serde(rename = "send")]
    Send {
        channel: String,
        template: Option<String>,
        subject: Option<String>,
        body: Option<String>,
        body_html: Option<String>,
    },
    #[serde(rename = "delay")]
    Delay { duration_secs: i64 },
    #[serde(rename = "condition")]
    Condition {
        field: String,    // e.g. "inbox.is_read"
        operator: String, // "eq", "neq", "gt", "lt"
        value: serde_json::Value,
        on_true: Option<usize>, // step index to jump to
        on_false: Option<usize>,
    },
    #[serde(rename = "digest")]
    Digest {
        duration_secs: i64, // collect events for this period
        channel: String,
        template: Option<String>,
        subject: Option<String>,
        body: Option<String>,
    },
}

/// Schema smoke test: every `FromRow` struct must be loadable with the
/// column list the code actually uses. Runs when `DATABASE_URL` points at a
/// Postgres (CI does), applies the migrations, and executes each `SELECT`
/// against an empty table, which is enough for sqlx to check the columns.
/// Caught in the wild: a struct gaining a field while one SELECT kept the old
/// column list ("no column found for name: topic").
#[cfg(test)]
mod schema_smoke {
    use super::*;

    #[tokio::test]
    async fn from_row_structs_match_their_selects() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL not set: schema smoke test skipped");
            return;
        };
        let pool = sqlx::PgPool::connect(&url).await.expect("connect");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");

        sqlx::query_as::<_, Job>(&format!("SELECT {JOB_COLUMNS} FROM jobs LIMIT 1"))
            .fetch_optional(&pool)
            .await
            .expect("Job columns");
        sqlx::query_as::<_, Template>(&format!("SELECT {TEMPLATE_COLUMNS} FROM templates LIMIT 1"))
            .fetch_optional(&pool)
            .await
            .expect("Template columns");
        sqlx::query_as::<_, Subscriber>(
            "SELECT id, project_id, email, phone, first_name, last_name, locale, data, created_at, updated_at FROM subscribers LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .expect("Subscriber columns");
        sqlx::query_as::<_, SubscriberPreference>(
            "SELECT project_id, subscriber_id, channel, workflow_id, enabled FROM subscriber_preferences LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .expect("SubscriberPreference columns");
        sqlx::query_as::<_, Workflow>(
            "SELECT id, project_id, name, description, trigger_event, steps, enabled, created_at, updated_at FROM workflows LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .expect("Workflow columns");
        sqlx::query_as::<_, WorkflowRun>(
            "SELECT id, project_id, workflow_id, subscriber_id, trigger_payload, current_step, status, step_state, resume_at, created_at, updated_at FROM workflow_runs LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .expect("WorkflowRun columns");
    }
}
