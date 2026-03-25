use std::{collections::HashMap, sync::Arc};
use tokio::sync::{broadcast, RwLock};
use tracing::debug;
use uuid::Uuid;

const BROADCAST_CAPACITY: usize = 64;

#[derive(Debug, Clone)]
pub struct SseMessage(pub String);

type ChannelKey = String; // "project_id:subscriber_id"

/// One-time ticket for SSE auth (avoids JWT in URL/logs)
#[derive(Debug, Clone)]
pub struct SseTicket {
    pub project_id: String,
    pub subscriber_id: String,
    pub created_at: std::time::Instant,
}

#[derive(Clone)]
pub struct SseBroadcaster {
    channels: Arc<RwLock<HashMap<ChannelKey, broadcast::Sender<SseMessage>>>>,
    tickets: Arc<RwLock<HashMap<String, SseTicket>>>,
}

impl SseBroadcaster {
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            tickets: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn key(project_id: &str, subscriber_id: &str) -> String {
        format!("{}:{}", project_id, subscriber_id)
    }

    /// Subscribe to events for a subscriber. Returns a receiver.
    pub async fn subscribe(&self, project_id: &str, subscriber_id: &str) -> broadcast::Receiver<SseMessage> {
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

    /// Send an event to a subscriber.
    pub async fn send(&self, project_id: &str, subscriber_id: &str, data: String) {
        let key = Self::key(project_id, subscriber_id);
        let channels = self.channels.read().await;
        if let Some(tx) = channels.get(&key) {
            let _ = tx.send(SseMessage(data));
        }
    }

    /// Issue a one-time SSE ticket (valid 60s, consumed on use)
    pub async fn issue_ticket(&self, project_id: &str, subscriber_id: &str) -> String {
        let ticket_id = Uuid::new_v4().to_string();
        let ticket = SseTicket {
            project_id: project_id.to_string(),
            subscriber_id: subscriber_id.to_string(),
            created_at: std::time::Instant::now(),
        };
        self.tickets.write().await.insert(ticket_id.clone(), ticket);
        ticket_id
    }

    /// Consume a one-time ticket. Returns (project_id, subscriber_id) if valid.
    pub async fn consume_ticket(&self, ticket_id: &str) -> Option<(String, String)> {
        let mut tickets = self.tickets.write().await;
        if let Some(ticket) = tickets.remove(ticket_id) {
            // Valid for 60 seconds
            if ticket.created_at.elapsed().as_secs() <= 60 {
                return Some((ticket.project_id, ticket.subscriber_id));
            }
        }
        None
    }

    /// Clean up channels with no active subscribers + expired tickets.
    pub async fn cleanup(&self) {
        let mut channels = self.channels.write().await;
        channels.retain(|_, tx| tx.receiver_count() > 0);

        let mut tickets = self.tickets.write().await;
        tickets.retain(|_, t| t.created_at.elapsed().as_secs() < 120);
    }
}

impl Default for SseBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}
