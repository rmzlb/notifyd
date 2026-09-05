pub mod admin_ops;
pub mod auth;
pub mod health;
pub mod inbox;
pub mod jobs;
pub mod preferences;
pub mod projects;
pub mod push_tokens;
pub mod send;
pub mod subscribers;
pub mod templates;
pub mod webhooks;
pub mod workflows;

use crate::AppState;
use axum::Router;
use std::sync::Arc;

pub fn router(state: Arc<AppState>) -> Router {
    // Provider callbacks live outside /v1: they are not part of the client
    // API and authenticate with a svix signature, not an API key.
    let provider = Router::new()
        .route(
            "/webhooks/resend",
            axum::routing::post(crate::deliverability::resend_webhook),
        )
        .with_state(state.clone());

    // Model Context Protocol endpoint for agents (admin key).
    let mcp = Router::new()
        .route(
            "/mcp",
            axum::routing::post(crate::mcp::post).get(crate::mcp::get),
        )
        .with_state(state.clone());

    Router::new()
        .nest("/v1", api_routes(state))
        .merge(provider)
        .merge(mcp)
}

fn api_routes(state: Arc<AppState>) -> Router {
    use axum::routing::{delete, get, patch, post};

    Router::new()
        // Health + Metrics
        .route("/health", get(health::health))
        .route("/metrics", get(health::metrics))
        .route("/metrics/prometheus", get(health::metrics_prometheus))
        // Auth
        .route("/auth/subscriber-token", post(auth::subscriber_token))
        // Send
        .route("/send", post(send::send_notification))
        .route("/schedule", post(send::send_notification))
        .route("/batch", post(send::batch_notification))
        // Jobs
        .route("/jobs/:id", get(jobs::get_job).delete(jobs::cancel_job))
        // Subscribers (list + create)
        .route(
            "/subscribers",
            post(subscribers::upsert_subscriber).get(subscribers::list_subscribers),
        )
        .route(
            "/subscribers/:id",
            get(subscribers::get_subscriber).delete(subscribers::delete_subscriber),
        )
        // Preferences
        .route(
            "/subscribers/:id/preferences",
            get(preferences::get_preferences).put(preferences::set_preferences),
        )
        // Inbox
        .route("/inbox/:subscriber_id", get(inbox::list_notifications))
        .route(
            "/inbox/:subscriber_id/unread-count",
            get(inbox::unread_count),
        )
        .route("/inbox/:subscriber_id/read-all", post(inbox::read_all))
        .route(
            "/inbox/:subscriber_id/:msg_id",
            patch(inbox::update_notification),
        )
        .route(
            "/inbox/:subscriber_id/stream-ticket",
            post(inbox::stream_ticket),
        )
        .route("/inbox/:subscriber_id/stream", get(inbox::sse_stream))
        // Workflows
        .route(
            "/workflows",
            post(workflows::create_workflow).get(workflows::list_workflows),
        )
        .route("/workflows/trigger", post(workflows::trigger_workflow))
        .route("/workflows/runs", get(workflows::list_runs))
        .route("/workflows/runs/:id", delete(workflows::cancel_run))
        .route(
            "/workflows/:id",
            get(workflows::get_workflow).delete(workflows::delete_workflow),
        )
        // Push tokens — BUG FIX #4: split list-by-subscriber and delete-by-uuid
        .route("/push/vapid-public-key", get(push_tokens::vapid_public_key))
        .route("/push-tokens", post(push_tokens::register_token))
        .route(
            "/push-tokens/subscriber/:subscriber_id",
            get(push_tokens::list_tokens),
        )
        .route("/push-tokens/:id", delete(push_tokens::delete_token))
        // Templates
        .route(
            "/templates",
            post(templates::upsert_template).get(templates::list_templates),
        )
        .route(
            "/templates/:id",
            get(templates::get_template).delete(templates::delete_template),
        )
        // Deliverability (suppression list)
        .route(
            "/suppressions",
            get(crate::deliverability::list_suppressions).post(admin_ops::project_add_suppression),
        )
        .route(
            "/suppressions/:id",
            delete(crate::deliverability::release_suppression),
        )
        // Admin
        .route(
            "/admin/projects",
            post(projects::create_project).get(projects::list_projects),
        )
        .route("/admin/projects/:id/rotate-key", post(projects::rotate_key))
        .route(
            "/admin/projects/:id/revoke-secondary",
            post(projects::revoke_secondary),
        )
        .route(
            "/admin/projects/:id",
            delete(projects::delete_project).patch(admin_ops::patch_project),
        )
        .route("/admin/audit", get(projects::audit_log))
        // Operator surface (digest, jobs, suppressions, project patch)
        .route("/admin/digest", get(admin_ops::digest))
        .route("/admin/jobs", get(admin_ops::list_jobs))
        .route("/admin/jobs/:id/retry", post(admin_ops::admin_retry_job))
        .route("/admin/jobs/:id/cancel", post(admin_ops::admin_cancel_job))
        .route(
            "/admin/suppressions",
            get(admin_ops::admin_list_suppressions).post(admin_ops::admin_add_suppression),
        )
        .route(
            "/admin/suppressions/:id",
            delete(admin_ops::admin_release_suppression),
        )
        .route("/jobs/:id/retry", post(admin_ops::project_retry_job))
        // Webhooks
        .route(
            "/admin/webhooks",
            post(webhooks::create_webhook).get(webhooks::list_webhooks),
        )
        .route("/admin/webhooks/:id", delete(webhooks::delete_webhook))
        .with_state(state)
}
