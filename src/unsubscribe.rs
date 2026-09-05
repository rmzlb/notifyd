//! Commercial unsubscribe, the way Gmail and Yahoo require it for bulk
//! senders (RFC 8058 one-click): every `bulk` email leaves with
//! `List-Unsubscribe` and `List-Unsubscribe-Post` headers pointing at a link
//! this instance hosts, `PUBLIC_URL/u/<token>`. The token is an HMAC over
//! (project, address, expiry) signed with `JWT_SECRET`, so no table and no
//! guessable id. A click adds a **marketing-scoped** suppression: campaigns
//! stop, order confirmations keep going.
//!
//! Callers that set their own `List-Unsubscribe` header keep it; notifyd only
//! fills the gap.

use crate::AppState;
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
};
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::sync::Arc;

type HmacSha256 = Hmac<Sha256>;

/// Unsubscribe links must keep working long after the campaign: 400 days.
const TOKEN_TTL_SECS: i64 = 400 * 24 * 3600;
const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

pub fn public_url() -> Option<String> {
    std::env::var("PUBLIC_URL")
        .ok()
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| u.starts_with("http"))
}

fn sign(secret: &str, payload: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(payload.as_bytes());
    B64.encode(mac.finalize().into_bytes())
}

/// `base64url(project|email|exp) . base64url(hmac)`
pub fn make_token(secret: &str, project_id: &str, email: &str) -> String {
    let exp = chrono::Utc::now().timestamp() + TOKEN_TTL_SECS;
    let payload = format!("{}|{}|{}", project_id, email.trim().to_lowercase(), exp);
    format!(
        "{}.{}",
        B64.encode(payload.as_bytes()),
        sign(secret, &payload)
    )
}

/// Returns (project_id, email) for a valid, unexpired token.
pub fn verify_token(secret: &str, token: &str) -> Option<(String, String)> {
    let (payload_b64, sig) = token.split_once('.')?;
    let payload = String::from_utf8(B64.decode(payload_b64).ok()?).ok()?;
    let expected = sign(secret, &payload);
    if !constant_time_eq(expected.as_bytes(), sig.as_bytes()) {
        return None;
    }
    let mut parts = payload.splitn(3, '|');
    let project = parts.next()?.to_string();
    let email = parts.next()?.to_string();
    let exp: i64 = parts.next()?.parse().ok()?;
    if exp < chrono::Utc::now().timestamp() {
        return None;
    }
    Some((project, email))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Headers to add to a bulk email that has none of its own.
pub fn headers_for(
    secret: &str,
    public_url: &str,
    project_id: &str,
    email: &str,
) -> serde_json::Value {
    let url = format!("{}/u/{}", public_url, make_token(secret, project_id, email));
    serde_json::json!({
        "List-Unsubscribe": format!("<{url}>"),
        "List-Unsubscribe-Post": "List-Unsubscribe=One-Click",
    })
}

async fn apply(
    state: &Arc<AppState>,
    token: &str,
    via: &str,
) -> Result<(String, String), StatusCode> {
    let (project_id, email) =
        verify_token(&state.config.server.jwt_secret, token).ok_or(StatusCode::NOT_FOUND)?;
    crate::ops::add_suppression(
        state,
        &project_id,
        &email,
        Some(&format!("unsubscribed via {via}")),
        "recipient",
        crate::ops::SuppressionScope::Marketing,
    )
    .await
    .map_err(|e| {
        tracing::error!("unsubscribe failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok((project_id, email))
}

/// POST /u/:token — one-click from the mail client (RFC 8058) or the button
/// on the confirmation page. Idempotent.
pub async fn post(State(state): State<Arc<AppState>>, Path(token): Path<String>) -> Response {
    match apply(&state, &token, "one-click").await {
        Ok((_, email)) => {
            tracing::info!(
                "Commercial unsubscribe recorded for {}",
                crate::pii::mask_email(&email)
            );
            (StatusCode::OK, Html(page("Vous êtes désinscrit(e).", "Vous ne recevrez plus nos emails commerciaux. Les emails liés à vos commandes continueront d'arriver."))).into_response()
        }
        Err(status) => (
            status,
            Html(page(
                "Lien invalide",
                "Ce lien de désinscription n'est plus valide.",
            )),
        )
            .into_response(),
    }
}

/// GET /u/:token — a person clicking the link: confirm with one button (a
/// GET must not change state, mail scanners follow links).
pub async fn get(State(state): State<Arc<AppState>>, Path(token): Path<String>) -> Response {
    if verify_token(&state.config.server.jwt_secret, &token).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Html(page(
                "Lien invalide",
                "Ce lien de désinscription n'est plus valide.",
            )),
        )
            .into_response();
    }
    let body = format!(
        r#"{}<form method="post" action=""><button type="submit">Me désinscrire des emails commerciaux</button></form><p class="s">Les emails liés à vos commandes ne sont pas concernés.</p></main></body></html>"#,
        page_head("Se désinscrire ?")
    );
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

fn page_head(title: &str) -> String {
    format!(
        r#"<!doctype html><html lang="fr"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="robots" content="noindex"><title>{title}</title><style>body{{font-family:system-ui,sans-serif;background:#faf9f7;color:#1a1a1a;margin:0;display:grid;place-items:center;min-height:100vh}}main{{max-width:28rem;padding:2rem;text-align:center}}h1{{font-size:1.4rem}}button{{font:inherit;padding:.8rem 1.4rem;border:2px solid #1a1a1a;background:#1a1a1a;color:#fff;cursor:pointer}}.s{{color:#666;font-size:.9rem}}</style></head><body><main><h1>{title}</h1>"#
    )
}

fn page(title: &str, text: &str) -> String {
    format!("{}<p>{text}</p></main></body></html>", page_head(title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_round_trips_and_normalises_email() {
        let t = make_token("s3cret", "philoe", " Jane@Example.com ");
        assert_eq!(
            verify_token("s3cret", &t),
            Some(("philoe".to_string(), "jane@example.com".to_string()))
        );
    }

    #[test]
    fn tampered_or_foreign_tokens_are_rejected() {
        let t = make_token("s3cret", "philoe", "jane@example.com");
        assert!(verify_token("other", &t).is_none());
        let (p, s) = t.split_once('.').unwrap();
        assert!(verify_token("s3cret", &format!("{p}x.{s}")).is_none());
        assert!(verify_token("s3cret", "garbage").is_none());
    }

    #[test]
    fn headers_follow_rfc_8058() {
        let h = headers_for(
            "s3cret",
            "https://n.example.com",
            "philoe",
            "jane@example.com",
        );
        let lu = h["List-Unsubscribe"].as_str().unwrap();
        assert!(lu.starts_with("<https://n.example.com/u/") && lu.ends_with('>'));
        assert_eq!(h["List-Unsubscribe-Post"], "List-Unsubscribe=One-Click");
    }
}
