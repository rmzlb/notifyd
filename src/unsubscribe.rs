//! Commercial unsubscribe, the way Gmail and Yahoo require it for bulk
//! senders (RFC 8058 one-click): every `bulk` email leaves with
//! `List-Unsubscribe` and `List-Unsubscribe-Post` headers pointing at a link
//! this instance hosts, `PUBLIC_URL/u/<token>`. The token is an HMAC over
//! (project, address, expiry[, topic]) signed with `JWT_SECRET`, so no table
//! and no guessable id. A click adds a **marketing-scoped** suppression:
//! campaigns stop, order confirmations keep going. When the email carried a
//! topic, the landing page offers the narrower choice first: stop that topic
//! only (a subscriber preference), or stop every commercial email.
//!
//! Callers that set their own `List-Unsubscribe` header keep it; notifyd only
//! fills the gap.

use crate::AppState;
use axum::{
    extract::{Path, Query, State},
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

/// What an unsubscribe token identifies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenClaims {
    pub project_id: String,
    pub email: String,
    /// Topic of the email the link was in, when it had one.
    pub topic: Option<String>,
}

/// `base64url(project|email|exp[|topic]) . base64url(hmac)`
pub fn make_token(secret: &str, project_id: &str, email: &str, topic: Option<&str>) -> String {
    let exp = chrono::Utc::now().timestamp() + TOKEN_TTL_SECS;
    let mut payload = format!("{}|{}|{}", project_id, email.trim().to_lowercase(), exp);
    if let Some(t) = topic.filter(|t| !t.is_empty()) {
        payload.push('|');
        payload.push_str(t);
    }
    format!(
        "{}.{}",
        B64.encode(payload.as_bytes()),
        sign(secret, &payload)
    )
}

/// Claims of a valid, unexpired token.
pub fn verify_token(secret: &str, token: &str) -> Option<TokenClaims> {
    let (payload_b64, sig) = token.split_once('.')?;
    let payload = String::from_utf8(B64.decode(payload_b64).ok()?).ok()?;
    let expected = sign(secret, &payload);
    if !constant_time_eq(expected.as_bytes(), sig.as_bytes()) {
        return None;
    }
    let mut parts = payload.splitn(4, '|');
    let project_id = parts.next()?.to_string();
    let email = parts.next()?.to_string();
    let exp: i64 = parts.next()?.parse().ok()?;
    if exp < chrono::Utc::now().timestamp() {
        return None;
    }
    let topic = parts.next().filter(|t| !t.is_empty()).map(str::to_string);
    Some(TokenClaims {
        project_id,
        email,
        topic,
    })
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
    topic: Option<&str>,
) -> serde_json::Value {
    let url = format!(
        "{}/u/{}",
        public_url,
        make_token(secret, project_id, email, topic)
    );
    serde_json::json!({
        "List-Unsubscribe": format!("<{url}>"),
        "List-Unsubscribe-Post": "List-Unsubscribe=One-Click",
    })
}

/// What a click decides: the topic only, or every commercial email.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Choice {
    Topic,
    Marketing,
}

#[derive(serde::Deserialize, Default)]
pub struct ChoiceQuery {
    /// `topic` to stop only the topic named in the token; anything else
    /// (including the RFC 8058 one-click POST, which sends no query) stops
    /// all commercial email.
    pub scope: Option<String>,
}

impl ChoiceQuery {
    fn choice(&self, claims: &TokenClaims) -> Choice {
        match (self.scope.as_deref(), claims.topic.as_deref()) {
            (Some("topic"), Some(_)) => Choice::Topic,
            _ => Choice::Marketing,
        }
    }
}

async fn apply(
    state: &Arc<AppState>,
    token: &str,
    choice: &ChoiceQuery,
    via: &str,
) -> Result<(TokenClaims, Choice), StatusCode> {
    let claims =
        verify_token(&state.config.server.jwt_secret, token).ok_or(StatusCode::NOT_FOUND)?;
    let mut decided = choice.choice(&claims);
    if decided == Choice::Topic {
        let topic = claims.topic.as_deref().unwrap_or_default();
        // A topic opt-out is a subscriber preference: it needs a subscriber
        // record behind the address. Without one, fall back to the broad
        // suppression rather than silently doing nothing.
        let subscriber: Option<String> = sqlx::query_scalar(
            "SELECT id FROM subscribers WHERE project_id = $1 AND lower(email) = $2 ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(&claims.project_id)
        .bind(&claims.email)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!("unsubscribe lookup failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        match subscriber {
            Some(sub) => {
                sqlx::query(
                    r#"
                    INSERT INTO subscriber_preferences (project_id, subscriber_id, channel, workflow_id, enabled)
                    VALUES ($1, $2, 'email', $3, false)
                    ON CONFLICT (project_id, subscriber_id, channel, workflow_id)
                    DO UPDATE SET enabled = false, updated_at = now()
                    "#,
                )
                .bind(&claims.project_id)
                .bind(&sub)
                .bind(topic)
                .execute(&state.pool)
                .await
                .map_err(|e| {
                    tracing::error!("topic unsubscribe failed: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
                crate::middleware::audit(
                    &state.pool,
                    &claims.project_id,
                    "recipient",
                    &format!("unsubscribe_topic:{topic}"),
                    Some(&sub),
                    None,
                )
                .await;
            }
            None => decided = Choice::Marketing,
        }
    }
    if decided == Choice::Marketing {
        crate::ops::add_suppression(
            state,
            &claims.project_id,
            &claims.email,
            Some(&format!("unsubscribed via {via}")),
            "recipient",
            crate::ops::SuppressionScope::Marketing,
        )
        .await
        .map_err(|e| {
            tracing::error!("unsubscribe failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }
    Ok((claims, decided))
}

/// POST /u/:token — one-click from the mail client (RFC 8058) or a button on
/// the confirmation page (`?scope=topic` for the narrow choice). Idempotent.
pub async fn post(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
    Query(choice): Query<ChoiceQuery>,
) -> Response {
    match apply(&state, &token, &choice, "one-click").await {
        Ok((claims, Choice::Topic)) => {
            let topic = claims.topic.as_deref().unwrap_or_default();
            tracing::info!(
                "Topic unsubscribe recorded for {} ({})",
                crate::pii::mask_email(&claims.email),
                topic
            );
            let topic = html_escape(topic);
            (
                StatusCode::OK,
                Html(page(
                    "C'est noté.",
                    &format!("Vous ne recevrez plus les emails « {topic} ». Les autres emails continueront d'arriver."),
                )),
            )
                .into_response()
        }
        Ok((claims, Choice::Marketing)) => {
            tracing::info!(
                "Commercial unsubscribe recorded for {}",
                crate::pii::mask_email(&claims.email)
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

/// GET /u/:token — a person clicking the link: confirm with a button (a GET
/// must not change state, mail scanners follow links). With a topic, the
/// narrow choice comes first.
pub async fn get(State(state): State<Arc<AppState>>, Path(token): Path<String>) -> Response {
    let Some(claims) = verify_token(&state.config.server.jwt_secret, &token) else {
        return (
            StatusCode::NOT_FOUND,
            Html(page(
                "Lien invalide",
                "Ce lien de désinscription n'est plus valide.",
            )),
        )
            .into_response();
    };
    let topic_form = match claims.topic.as_deref() {
        Some(t) => {
            let t = html_escape(t);
            format!(
                r#"<form method="post" action="?scope=topic"><button type="submit">Ne plus recevoir « {t} »</button></form><p class="s">Seuls les emails « {t} » s'arrêtent.</p>"#
            )
        }
        None => String::new(),
    };
    let body = format!(
        r#"{}{}<form method="post" action=""><button type="submit" class="{}">Me désinscrire de tous les emails commerciaux</button></form><p class="s">Les emails liés à vos commandes ne sont pas concernés.</p></main></body></html>"#,
        page_head("Se désinscrire ?"),
        topic_form,
        if claims.topic.is_some() { "alt" } else { "" }
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

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn page_head(title: &str) -> String {
    format!(
        r#"<!doctype html><html lang="fr"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="robots" content="noindex"><title>{title}</title><style>body{{font-family:system-ui,sans-serif;background:#faf9f7;color:#1a1a1a;margin:0;display:grid;place-items:center;min-height:100vh}}main{{max-width:28rem;padding:2rem;text-align:center}}h1{{font-size:1.4rem}}button{{font:inherit;padding:.8rem 1.4rem;border:2px solid #1a1a1a;background:#1a1a1a;color:#fff;cursor:pointer;margin-top:.6rem}}button.alt{{background:#fff;color:#1a1a1a}}.s{{color:#666;font-size:.9rem}}</style></head><body><main><h1>{title}</h1>"#
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
        let t = make_token("s3cret", "philoe", " Jane@Example.com ", None);
        assert_eq!(
            verify_token("s3cret", &t),
            Some(TokenClaims {
                project_id: "philoe".to_string(),
                email: "jane@example.com".to_string(),
                topic: None,
            })
        );
    }

    #[test]
    fn token_carries_the_topic_and_the_choice_follows_it() {
        let t = make_token("s3cret", "philoe", "jane@example.com", Some("tips"));
        let claims = verify_token("s3cret", &t).unwrap();
        assert_eq!(claims.topic.as_deref(), Some("tips"));
        let narrow = ChoiceQuery {
            scope: Some("topic".into()),
        };
        assert_eq!(narrow.choice(&claims), Choice::Topic);
        assert_eq!(ChoiceQuery::default().choice(&claims), Choice::Marketing);
        let no_topic =
            verify_token("s3cret", &make_token("s3cret", "philoe", "j@e.com", None)).unwrap();
        assert_eq!(
            narrow.choice(&no_topic),
            Choice::Marketing,
            "without a topic in the token only the broad choice exists"
        );
    }

    #[test]
    fn tampered_or_foreign_tokens_are_rejected() {
        let t = make_token("s3cret", "philoe", "jane@example.com", None);
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
            Some("tips"),
        );
        let lu = h["List-Unsubscribe"].as_str().unwrap();
        assert!(lu.starts_with("<https://n.example.com/u/") && lu.ends_with('>'));
        assert_eq!(h["List-Unsubscribe-Post"], "List-Unsubscribe=One-Click");
    }
}
