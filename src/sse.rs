//! In-app inbox live updates over Server-Sent Events.
//!
//! Every event is published with Postgres `NOTIFY` on the `notifyd_sse`
//! channel and delivered to the browsers connected to *each* replica by that
//! replica's listener. Nothing about a connection lives in one process only,
//! so the service scales horizontally. One-time stream tickets live in the
//! `sse_tickets` table for the same reason.
//!
//! `NOTIFY` payloads are capped by Postgres (8000 bytes). Larger events are
//! delivered to the local replica only and logged; the inbox REST endpoints
//! remain the source of truth, live updates are an acceleration.

use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgListener, PgPool};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, error, warn};
use uuid::Uuid;

const BROADCAST_CAPACITY: usize = 64;
pub const NOTIFY_CHANNEL: &str = "notifyd_sse";
const NOTIFY_MAX_BYTES: usize = 7_900;
const TICKET_TTL_SECS: i64 = 60;

#[derive(Debug, Clone)]
pub struct SseMessage(pub String);

type ChannelKey = String; // "project_id:subscriber_id"

/// Escaped JSON inside `d` cannot be borrowed on the way back, hence `Cow`.
#[derive(Debug, Serialize, Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    k: std::borrow::Cow<'a, str>,
    #[serde(borrow)]
    d: std::borrow::Cow<'a, str>,
}

#[derive(Clone)]
pub struct SseBroadcaster {
    channels: Arc<RwLock<HashMap<ChannelKey, broadcast::Sender<SseMessage>>>>,
    pool: PgPool,
}

impl SseBroadcaster {
    pub fn new(pool: PgPool) -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            pool,
        }
    }

    fn key(project_id: &str, subscriber_id: &str) -> String {
        format!("{}:{}", project_id, subscriber_id)
    }

    pub async fn subscribe(
        &self,
        project_id: &str,
        subscriber_id: &str,
    ) -> broadcast::Receiver<SseMessage> {
        let key = Self::key(project_id, subscriber_id);
        let mut channels = self.channels.write().await;

        if let Some(tx) = channels.get(&key) {
            tx.subscribe()
        } else {
            let (tx, rx) = broadcast::channel(BROADCAST_CAPACITY);
            channels.insert(key.clone(), tx);
            debug!("SSE channel created for {}", key);
            rx
        }
    }

    /// Publish an event to a subscriber on every replica.
    pub async fn send(&self, project_id: &str, subscriber_id: &str, data: String) {
        let key = Self::key(project_id, subscriber_id);
        let envelope = serde_json::to_string(&Envelope {
            k: key.as_str().into(),
            d: data.as_str().into(),
        })
        .unwrap_or_default();
        if envelope.len() > NOTIFY_MAX_BYTES {
            warn!(
                "SSE event for {} is {} bytes, above the NOTIFY limit: delivered locally only",
                key,
                envelope.len()
            );
            self.deliver_local(&key, data).await;
            return;
        }
        if let Err(e) = sqlx::query("SELECT pg_notify($1, $2)")
            .bind(NOTIFY_CHANNEL)
            .bind(&envelope)
            .execute(&self.pool)
            .await
        {
            warn!(
                "pg_notify failed ({}): delivering SSE event locally only",
                e
            );
            self.deliver_local(&key, data).await;
        }
    }

    /// Deliver to the browsers connected to this process.
    pub async fn deliver_local(&self, key: &str, data: String) {
        let channels = self.channels.read().await;
        if let Some(tx) = channels.get(key) {
            let _ = tx.send(SseMessage(data));
        }
    }

    /// Issue a one-time SSE ticket (valid 60 s, consumed on use, shared by
    /// all replicas through the database).
    pub async fn issue_ticket(&self, project_id: &str, subscriber_id: &str) -> String {
        let ticket_id = Uuid::new_v4().to_string();
        if let Err(e) = sqlx::query(
            "INSERT INTO sse_tickets (id, project_id, subscriber_id) VALUES ($1, $2, $3)",
        )
        .bind(&ticket_id)
        .bind(project_id)
        .bind(subscriber_id)
        .execute(&self.pool)
        .await
        {
            error!("Could not store SSE ticket: {}", e);
        }
        ticket_id
    }

    /// Consume a one-time ticket. Returns (project_id, subscriber_id) if valid.
    pub async fn consume_ticket(&self, ticket_id: &str) -> Option<(String, String)> {
        let row: Option<(String, String)> = sqlx::query_as(
            "DELETE FROM sse_tickets
             WHERE id = $1 AND created_at > now() - make_interval(secs => $2)
             RETURNING project_id, subscriber_id",
        )
        .bind(ticket_id)
        .bind(TICKET_TTL_SECS as f64)
        .fetch_optional(&self.pool)
        .await
        .unwrap_or_else(|e| {
            error!("SSE ticket lookup failed: {}", e);
            None
        });
        row
    }

    /// Drop channels nobody listens to and tickets past their lifetime.
    pub async fn cleanup(&self) {
        let mut channels = self.channels.write().await;
        channels.retain(|_, tx| tx.receiver_count() > 0);
        drop(channels);
        if let Err(e) =
            sqlx::query("DELETE FROM sse_tickets WHERE created_at < now() - interval '2 minutes'")
                .execute(&self.pool)
                .await
        {
            warn!("SSE ticket cleanup failed: {}", e);
        }
    }

    /// Listen to `NOTIFY notifyd_sse` and hand every event to the local
    /// browsers. `PgListener::recv` reconnects on its own; this loop only
    /// gives up when the shutdown signal flips.
    pub async fn run_listener(self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut listener = match PgListener::connect_with(&self.pool).await {
            Ok(l) => l,
            Err(e) => {
                error!(
                    "SSE listener could not connect: {} — live updates limited to local events",
                    e
                );
                return;
            }
        };
        if let Err(e) = listener.listen(NOTIFY_CHANNEL).await {
            error!("SSE LISTEN failed: {}", e);
            return;
        }
        debug!("SSE listener attached to {}", NOTIFY_CHANNEL);
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                received = listener.recv() => match received {
                    Ok(notification) => {
                        match serde_json::from_str::<Envelope>(notification.payload()) {
                            Ok(envelope) => self.deliver_local(&envelope.k, envelope.d.into_owned()).await,
                            Err(e) => warn!("Ignoring malformed SSE notification: {}", e),
                        }
                    }
                    Err(e) => {
                        warn!("SSE listener error: {} — reconnecting", e);
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_round_trips() {
        let json = serde_json::to_string(&Envelope {
            k: "philoe:admin:1".into(),
            d: "{\"type\":\"x\"}".into(),
        })
        .unwrap();
        let back: Envelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back.k, "philoe:admin:1");
        assert_eq!(back.d, "{\"type\":\"x\"}");
    }

    #[test]
    fn key_joins_project_and_subscriber() {
        assert_eq!(SseBroadcaster::key("p", "s"), "p:s");
    }
}
