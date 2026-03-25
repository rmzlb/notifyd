use std::{collections::HashMap, sync::Arc};
use tokio::sync::{broadcast, RwLock};
use tracing::debug;

const BROADCAST_CAPACITY: usize = 64;

#[derive(Debug, Clone)]
pub struct SseMessage(pub String);

type ChannelKey = String; // "project_id:subscriber_id"

#[derive(Clone)]
pub struct SseBroadcaster {
    channels: Arc<RwLock<HashMap<ChannelKey, broadcast::Sender<SseMessage>>>>,
}

impl SseBroadcaster {
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
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

    /// Clean up channels with no active subscribers.
    pub async fn cleanup(&self) {
        let mut channels = self.channels.write().await;
        channels.retain(|_, tx| tx.receiver_count() > 0);
    }
}

impl Default for SseBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}
