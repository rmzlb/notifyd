pub mod send;
pub mod jobs;
pub mod subscribers;
pub mod inbox;
pub mod auth;
pub mod health;
pub mod preferences;
pub mod workflows;
pub mod push_tokens;
pub mod projects;

use axum::Router;
use std::sync::Arc;
use crate::AppState;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .nest("/v1", api_routes(state))
}

fn api_routes(state: Arc<AppState>) -> Router {
    use axum::routing::{get, post, put, patch, delete};

    Router::new()
        // Health
        .route("/health", get(health::health))
        // Auth
        .route("/auth/subscriber-token", post(auth::subscriber_token))
        // Send
        .route("/send", post(send::send_notification))
        .route("/schedule", post(send::send_notification))
        .route("/batch", post(send::batch_notification))
        // Jobs
        .route("/jobs/:id", get(jobs::get_job).delete(jobs::cancel_job))
        // Subscribers
        .route("/subscribers", post(subscribers::upsert_subscriber))
        .route("/subscribers/:id", get(subscribers::get_subscriber).delete(subscribers::delete_subscriber))
        // Preferences
        .route("/subscribers/:id/preferences", get(preferences::get_preferences).put(preferences::set_preferences))
        // Inbox
        .route("/inbox/:subscriber_id", get(inbox::list_notifications))
        .route("/inbox/:subscriber_id/unread-count", get(inbox::unread_count))
        .route("/inbox/:subscriber_id/read-all", post(inbox::read_all))
        .route("/inbox/:subscriber_id/:msg_id", patch(inbox::update_notification))
        .route("/inbox/:subscriber_id/stream-ticket", post(inbox::stream_ticket))
        .route("/inbox/:subscriber_id/stream", get(inbox::sse_stream))
        // Workflows
        .route("/workflows", post(workflows::create_workflow).get(workflows::list_workflows))
        .route("/workflows/trigger", post(workflows::trigger_workflow))
        .route("/workflows/runs", get(workflows::list_runs))
        .route("/workflows/runs/:id", delete(workflows::cancel_run))
        .route("/workflows/:id", get(workflows::get_workflow).delete(workflows::delete_workflow))
        // Push tokens
        .route("/push-tokens", post(push_tokens::register_token))
        .route("/push-tokens/:subscriber_id", get(push_tokens::list_tokens))
        .route("/push-tokens/:id", delete(push_tokens::delete_token))
        // Admin
        .route("/admin/projects", post(projects::create_project).get(projects::list_projects))
        .route("/admin/projects/:id/rotate-key", post(projects::rotate_key))
        .route("/admin/projects/:id/revoke-secondary", post(projects::revoke_secondary))
        .route("/admin/projects/:id", delete(projects::delete_project))
        .route("/admin/audit", get(projects::audit_log))
        .with_state(state)
}
