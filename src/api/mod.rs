pub mod send;
pub mod jobs;
pub mod subscribers;
pub mod inbox;
pub mod auth;
pub mod health;

use axum::{Router, middleware};
use std::sync::Arc;
use crate::AppState;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .nest("/v1", api_routes(state.clone()))
}

fn api_routes(state: Arc<AppState>) -> Router {
    use axum::routing::{get, post, patch, delete};

    Router::new()
        // Health (no auth)
        .route("/health", get(health::health))
        // Auth
        .route("/auth/subscriber-token", post(auth::subscriber_token))
        // Send
        .route("/send", post(send::send_notification))
        .route("/schedule", post(send::send_notification))  // alias
        .route("/batch", post(send::batch_notification))
        // Jobs
        .route("/jobs/:id", get(jobs::get_job))
        .route("/jobs/:id", delete(jobs::cancel_job))
        // Subscribers
        .route("/subscribers", post(subscribers::upsert_subscriber))
        .route("/subscribers/:id", get(subscribers::get_subscriber))
        .route("/subscribers/:id", delete(subscribers::delete_subscriber))
        // Inbox (API-key or subscriber-JWT auth)
        .route("/inbox/:subscriber_id", get(inbox::list_notifications))
        .route("/inbox/:subscriber_id/unread-count", get(inbox::unread_count))
        .route("/inbox/:subscriber_id/read-all", post(inbox::read_all))
        .route("/inbox/:subscriber_id/:msg_id", patch(inbox::update_notification))
        // SSE
        .route("/inbox/:subscriber_id/stream", get(inbox::sse_stream))
        .with_state(state)
}
