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
}

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
}

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
