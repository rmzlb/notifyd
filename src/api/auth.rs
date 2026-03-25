use axum::{extract::State, Json, http::StatusCode};
use jsonwebtoken::{encode, decode, Header, EncodingKey, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use chrono::{Utc, Duration};
use crate::{AppState, api::send::extract_project};

#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriberClaims {
    pub sub: String,
    pub project: String,
    pub exp: usize,
    pub iat: usize,
}

#[derive(Deserialize)]
pub struct TokenRequest {
    pub subscriber_id: String,
}

pub async fn subscriber_token(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<TokenRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;

    let now = Utc::now();
    let exp = now + Duration::hours(24);

    let claims = SubscriberClaims {
        sub: req.subscriber_id.clone(),
        project: project.id.clone(),
        exp: exp.timestamp() as usize,
        iat: now.timestamp() as usize,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.config.server.jwt_secret.as_bytes()),
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(json!({
        "token": token,
        "subscriber_id": req.subscriber_id,
        "project_id": project.id,
        "expires_at": exp.to_rfc3339(),
    })))
}

pub fn validate_subscriber_token(state: &AppState, token: &str) -> Option<SubscriberClaims> {
    decode::<SubscriberClaims>(
        token,
        &DecodingKey::from_secret(state.config.server.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .ok()
    .map(|d| d.claims)
}
