use crate::{api::send::extract_project, config::ConnectorsConfig, connectors::Channel, AppState};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde_json::{json, Map, Value};
use std::sync::Arc;

/// The channels whose delivery depends on a connector of this instance.
const REPORTED: [Channel; 5] = [
    Channel::Email,
    Channel::Sms,
    Channel::Whatsapp,
    Channel::Push,
    Channel::InApp,
];

/// GET /v1/channels — for each channel, `allowed` (in the project's channel
/// list) and `configured` (this instance can deliver it with its default
/// sender, so `/v1/send` will not refuse it). A client offers a channel only
/// when both are true.
pub async fn list_channels(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;
    let allowed = project_channels(&state, &project.id).await?;
    Ok(Json(channel_report(&state.config.connectors, &allowed)))
}

/// The project's channel list: `notifyd.toml` for a configured project, the
/// `projects` table otherwise.
async fn project_channels(
    state: &AppState,
    project_id: &str,
) -> Result<Vec<String>, (StatusCode, Json<Value>)> {
    if let Some(project) = state.config.projects.get(project_id) {
        return Ok(project.channels.clone());
    }
    let channels: Option<Option<Vec<String>>> =
        sqlx::query_scalar("SELECT channels FROM projects WHERE id = $1")
            .bind(project_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| {
                tracing::error!("DB error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "Internal server error"})),
                )
            })?;
    Ok(channels.flatten().unwrap_or_default())
}

fn channel_report(connectors: &ConnectorsConfig, allowed: &[String]) -> Value {
    let channels: Map<String, Value> = REPORTED
        .iter()
        .map(|channel| {
            let listed = allowed
                .iter()
                .any(|name| Channel::from_str(name) == Some(*channel));
            (
                channel.as_str().to_string(),
                json!({
                    "allowed": listed,
                    "configured": connectors.refusal(channel.as_str(), None).is_none(),
                }),
            )
        })
        .collect();
    json!({ "channels": channels })
}

#[cfg(test)]
mod tests {
    use super::channel_report;

    #[test]
    fn reports_listed_and_configured_separately() {
        let connectors = toml::from_str(
            r#"
            [email]
            provider = "log"
            from = "noreply@example.com"
            "#,
        )
        .expect("valid connectors table");
        let allowed = vec!["email".to_string(), "sms".to_string(), "inapp".to_string()];

        let report = channel_report(&connectors, &allowed);
        let channels = &report["channels"];

        assert_eq!(channels["email"]["allowed"], true);
        assert_eq!(channels["email"]["configured"], true);
        // Listed for the project, but this instance cannot send it.
        assert_eq!(channels["sms"]["allowed"], true);
        assert_eq!(channels["sms"]["configured"], false);
        // `inapp` is an accepted spelling of `in_app`.
        assert_eq!(channels["in_app"]["allowed"], true);
        assert_eq!(channels["push"]["allowed"], false);
        assert_eq!(channels["push"]["configured"], false);
    }
}
