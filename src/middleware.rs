use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::sync::RwLock;

/// In-memory sliding window rate limiter per project
#[derive(Clone)]
pub struct RateLimiter {
    windows: Arc<RwLock<HashMap<String, WindowCounter>>>,
}

struct WindowCounter {
    count: u32,
    window_start: Instant,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            windows: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Check and increment. Returns true if under limit.
    pub async fn check(&self, project_id: &str, limit_per_min: u32) -> bool {
        let mut windows = self.windows.write().await;
        let now = Instant::now();

        let entry = windows
            .entry(project_id.to_string())
            .or_insert(WindowCounter {
                count: 0,
                window_start: now,
            });

        // Reset window if expired (1 minute)
        if now.duration_since(entry.window_start).as_secs() >= 60 {
            entry.count = 0;
            entry.window_start = now;
        }

        if entry.count >= limit_per_min {
            false
        } else {
            entry.count += 1;
            true
        }
    }

    /// Cleanup old entries periodically
    pub async fn cleanup(&self) {
        let mut windows = self.windows.write().await;
        let now = Instant::now();
        windows.retain(|_, v| now.duration_since(v.window_start).as_secs() < 120);
    }
}

/// Log an audit entry (fire-and-forget, don't block the request)
pub async fn audit(
    pool: &sqlx::PgPool,
    project_id: &str,
    actor: &str,
    action: &str,
    resource: Option<&str>,
    ip: Option<&str>,
) {
    if let Err(e) = sqlx::query(
        "INSERT INTO audit_log (project_id, actor, action, resource, ip) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(project_id)
    .bind(actor)
    .bind(action)
    .bind(resource)
    .bind(ip)
    .execute(pool)
    .await {
        tracing::warn!("Audit log write failed: {}", e);
    }
}
