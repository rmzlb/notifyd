use super::{Channel, Connector, SendRequest};
use anyhow::Result;
use async_trait::async_trait;
use sqlx::PgPool;
use tracing::info;
use crate::sse::SseBroadcaster;
use serde_json::json;
use uuid::Uuid;
use chrono::{DateTime, Utc};

pub struct InAppConnector {
    pub pool: PgPool,
    pub broadcaster: SseBroadcaster,
}

impl InAppConnector {
    pub fn new(pool: PgPool, broadcaster: SseBroadcaster) -> Self {
        Self { pool, broadcaster }
    }
}

#[derive(sqlx::FromRow)]
struct InsertedMessage {
    id: Uuid,
    created_at: Option<DateTime<Utc>>,
}

#[async_trait]
impl Connector for InAppConnector {
    fn channel(&self) -> Channel { Channel::InApp }

    async fn send(&self, req: &SendRequest) -> Result<()> {
        let project_id = req.metadata["project_id"].as_str().unwrap_or("");
        let subscriber_id = req.metadata["subscriber_id"].as_str()
            .unwrap_or(req.recipient.as_str());
        let icon = req.metadata["icon"].as_str().unwrap_or("bell");
        let url = req.metadata["url"].as_str();

        let row: InsertedMessage = sqlx::query_as(
            "INSERT INTO inbox_messages (project_id, subscriber_id, body, icon, url, data) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id, created_at"
        )
        .bind(project_id)
        .bind(subscriber_id)
        .bind(&req.body)
        .bind(icon)
        .bind(url)
        .bind(&req.metadata)
        .fetch_one(&self.pool)
        .await?;

        let event = json!({
            "type": "new_notification",
            "notification": {
                "id": row.id,
                "body": req.body,
                "icon": icon,
                "url": url,
                "created_at": row.created_at,
                "is_read": false,
                "is_todo": false,
            }
        });

        self.broadcaster.send(project_id, subscriber_id, event.to_string()).await;

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM inbox_messages WHERE project_id=$1 AND subscriber_id=$2 AND read_at IS NULL AND archived_at IS NULL"
        )
        .bind(project_id)
        .bind(subscriber_id)
        .fetch_one(&self.pool)
        .await?;

        let count_event = json!({"type": "count_update", "unread_count": count});
        self.broadcaster.send(project_id, subscriber_id, count_event.to_string()).await;

        info!("In-app sent to {}:{}", project_id, subscriber_id);
        Ok(())
    }
}
