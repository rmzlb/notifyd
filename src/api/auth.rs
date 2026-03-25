use axum::{extract::State, Json, http::StatusCode};
use jsonwebtoken::{encode, decode, Header, EncodingKey, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use chrono::{Utc, Duration};
use crate::{AppState, api::send::extract_project};

#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriberClaims {
    pub sub: String,       // subscriber_id
    pub project: String,   // project_id
    pub aud: String,       // "notifyd:inbox"
    pub exp: usize,
    pub iat: usize,
}

#[derive(Deserialize)]
pub struct TokenRequest {
    pub subscriber_id: String,
    pub ttl_hours: Option<i64>,  // default 2, max 24
}

pub async fn subscriber_token(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<TokenRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let now = Utc::now();
    let ttl = req.ttl_hours.unwrap_or(2).min(24).max(1);
    let exp = now + Duration::hours(ttl);

    let claims = SubscriberClaims {
        sub: req.subscriber_id.clone(),
        project: project.id.clone(),
        aud: "notifyd:inbox".to_string(),
        exp: exp.timestamp() as usize,
        iat: now.timestamp() as usize,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.config.server.jwt_secret.as_bytes()),
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Token generation failed"}))))?;

    Ok(Json(json!({
        "token": token,
        "subscriber_id": req.subscriber_id,
        "project_id": project.id,
        "expires_at": exp.to_rfc3339(),
        "ttl_hours": ttl,
    })))
}

pub fn validate_subscriber_token(state: &AppState, token: &str) -> Option<SubscriberClaims> {
    let mut validation = Validation::default();
    validation.set_audience(&["notifyd:inbox"]);

    decode::<SubscriberClaims>(
        token,
        &DecodingKey::from_secret(state.config.server.jwt_secret.as_bytes()),
        &validation,
    )
    .ok()
    .map(|d| d.claims)
}
