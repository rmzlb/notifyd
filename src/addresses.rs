//! Where a message goes when the request names a subscriber and no `to`:
//! the address is read from the subscriber record at enqueue, per channel.
//!
//! | channel              | address                                   |
//! |----------------------|-------------------------------------------|
//! | email                | `subscribers.email`                       |
//! | sms, whatsapp        | `subscribers.phone`                       |
//! | telegram             | `data.telegram_chat_id`                   |
//! | slack                | `data.slack` (webhook URL or channel id)  |
//! | discord              | `data.discord_webhook`                    |
//! | in_app, push         | the subscriber id itself                  |
//!
//! A subscriber without the address for a channel gets no job on that
//! channel; the API answer says so (`skipped` / `jobs_without_address`).

use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct SubscriberAddresses {
    pub id: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub timezone: Option<String>,
    pub data: Option<Value>,
}

impl SubscriberAddresses {
    /// Address for `channel`, or `None` when the subscriber has none.
    pub fn for_channel(&self, channel: &str) -> Option<String> {
        let non_empty = |v: &Option<String>| {
            v.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let data_str = |key: &str| {
            self.data
                .as_ref()
                .and_then(|d| d.get(key))
                .and_then(|v| match v {
                    Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
        };
        match channel {
            "email" => non_empty(&self.email),
            "sms" | "whatsapp" => non_empty(&self.phone),
            "telegram" => data_str("telegram_chat_id"),
            "slack" => data_str("slack")
                .or_else(|| data_str("slack_webhook"))
                .or_else(|| data_str("slack_channel")),
            "discord" => data_str("discord_webhook"),
            "in_app" | "inapp" | "push" | "fcm" => Some(self.id.clone()),
            _ => None,
        }
    }
}

/// What the caller should set to make the channel reachable.
pub fn missing_reason(channel: &str) -> String {
    let field = match channel {
        "email" => "email",
        "sms" | "whatsapp" => "phone",
        "telegram" => "data.telegram_chat_id",
        "slack" => "data.slack (webhook URL or channel id)",
        "discord" => "data.discord_webhook",
        _ => "an address",
    };
    format!("subscriber has no {field} for {channel}")
}

/// Addresses of several subscribers in one query, keyed by id.
pub async fn load(
    pool: &sqlx::PgPool,
    project_id: &str,
    ids: &[String],
) -> Result<HashMap<String, SubscriberAddresses>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<SubscriberAddresses> = sqlx::query_as(
        "SELECT id, email, phone, timezone, data FROM subscribers WHERE project_id = $1 AND id = ANY($2)",
    )
    .bind(project_id)
    .bind(ids)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| (r.id.clone(), r)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sub() -> SubscriberAddresses {
        SubscriberAddresses {
            id: "u1".into(),
            email: Some(" a@b.c ".into()),
            phone: None,
            timezone: None,
            data: Some(
                json!({"telegram_chat_id": 123456, "slack": "https://hooks.slack.com/services/T/B/x", "discord_webhook": ""}),
            ),
        }
    }

    #[test]
    fn picks_the_address_per_channel() {
        let s = sub();
        assert_eq!(s.for_channel("email").as_deref(), Some("a@b.c"));
        assert_eq!(s.for_channel("sms"), None, "no phone");
        assert_eq!(
            s.for_channel("telegram").as_deref(),
            Some("123456"),
            "numeric chat ids are fine"
        );
        assert_eq!(
            s.for_channel("slack").as_deref(),
            Some("https://hooks.slack.com/services/T/B/x")
        );
        assert_eq!(
            s.for_channel("discord"),
            None,
            "empty string counts as missing"
        );
        assert_eq!(s.for_channel("in_app").as_deref(), Some("u1"));
        assert_eq!(s.for_channel("push").as_deref(), Some("u1"));
        assert_eq!(s.for_channel("pigeon"), None);
        assert!(missing_reason("telegram").contains("data.telegram_chat_id"));
    }
}
