//! Own open and click tracking for email, independent of the provider.
//!
//! When `PUBLIC_URL` is set, every outgoing HTML email gets a 1×1 pixel
//! (`/t/o/<token>.gif`) and its `http(s)` links are wrapped
//! (`/t/c/<token>` → 302 to the original). Tokens are HMACs signed with
//! `JWT_SECRET`, so the redirect cannot be pointed anywhere but the URL that
//! was in the email, and nobody can forge an open for a job they do not know.
//! Each hit is a `provider_events` row (`provider = 'notifyd'`), the same
//! table Resend webhooks feed, so the digest and per-template funnel count
//! them without knowing where they came from; the job gets `opened_at` /
//! `clicked_at` (first time). A click event stores the link's host only,
//! never the full URL (sign-in and reset links carry tokens).
//!
//! Scope: **marketing email only by default** (bulk priority or a campaign
//! tag), the way every serious sender does it. Transactional mail (receipts,
//! sign-in links) keeps its links untouched unless the project says
//! `{"applies_to": "all"}` or the request asks for it with `"track": true`
//! (or an object). Off switches: project `settings.tracking = false` (or
//! `{"opens": false, "clicks": false}`), request `"track": false`.
//! Opens are approximate by nature (image proxies and mail scanners load
//! pixels); clicks are reliable.

use crate::AppState;
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use base64::Engine;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Smallest valid transparent GIF (43 bytes).
const PIXEL: [u8; 43] = [
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xff, 0xff, 0xff, 0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3b,
];

/// What a project (or a request) allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tracking {
    pub opens: bool,
    pub clicks: bool,
    /// Also instrument transactional (non-marketing) email.
    pub all_email: bool,
}

impl Tracking {
    pub const ON: Tracking = Tracking {
        opens: true,
        clicks: true,
        all_email: false,
    };
    pub const OFF: Tracking = Tracking {
        opens: false,
        clicks: false,
        all_email: false,
    };

    /// `settings.tracking`: absent → on, `false` → off, object → per flag.
    pub fn from_settings(settings: Option<&Value>) -> Tracking {
        match settings.and_then(|s| s.get("tracking")) {
            None | Some(Value::Null) => Tracking::ON,
            Some(Value::Bool(b)) => {
                if *b {
                    Tracking::ON
                } else {
                    Tracking::OFF
                }
            }
            Some(obj) => Tracking {
                opens: obj.get("opens").and_then(Value::as_bool).unwrap_or(true),
                clicks: obj.get("clicks").and_then(Value::as_bool).unwrap_or(true),
                all_email: obj.get("applies_to").and_then(Value::as_str) == Some("all"),
            },
        }
    }

    /// Whether this email is in scope: marketing always, transactional only
    /// when the project extends tracking to all mail or the request asks.
    pub fn applies(&self, marketing: bool, payload: &Value) -> bool {
        if marketing || self.all_email {
            return true;
        }
        match payload.get("track") {
            Some(Value::Bool(true)) => true,
            Some(obj) if obj.is_object() => true,
            _ => false,
        }
    }

    /// A request can only turn tracking off, never on for a project that
    /// disabled it.
    pub fn with_request(self, payload: &Value) -> Tracking {
        match payload.get("track") {
            Some(Value::Bool(false)) => Tracking::OFF,
            Some(obj) if obj.is_object() => Tracking {
                opens: self.opens && obj.get("opens").and_then(Value::as_bool).unwrap_or(true),
                clicks: self.clicks && obj.get("clicks").and_then(Value::as_bool).unwrap_or(true),
                all_email: self.all_email,
            },
            _ => self,
        }
    }
}

fn token(secret: &str, payload: &str) -> String {
    format!(
        "{}.{}",
        B64.encode(payload.as_bytes()),
        crate::unsubscribe::sign(secret, payload)
    )
}

fn verify(secret: &str, token: &str) -> Option<String> {
    let (payload_b64, sig) = token.split_once('.')?;
    let payload = String::from_utf8(B64.decode(payload_b64).ok()?).ok()?;
    let expected = crate::unsubscribe::sign(secret, &payload);
    if expected.len() != sig.len()
        || expected
            .bytes()
            .zip(sig.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            != 0
    {
        return None;
    }
    Some(payload)
}

pub fn open_url(secret: &str, public_url: &str, job_id: Uuid) -> String {
    format!(
        "{}/t/o/{}.gif",
        public_url,
        token(secret, &format!("o|{job_id}"))
    )
}

pub fn click_url(secret: &str, public_url: &str, job_id: Uuid, target: &str) -> String {
    format!(
        "{}/t/c/{}",
        public_url,
        token(secret, &format!("c|{job_id}|{target}"))
    )
}

fn decode_entities(href: &str) -> String {
    href.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn encode_entities(url: &str) -> String {
    url.replace('&', "&amp;").replace('"', "&quot;")
}

/// Links that must keep their original target.
fn is_trackable(href: &str, public_url: &str) -> bool {
    let lower = href.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://"))
        && !lower.starts_with(&format!("{}/u/", public_url.to_ascii_lowercase()))
        && !lower.starts_with(&format!("{}/t/", public_url.to_ascii_lowercase()))
        && !href.contains("{{")
}

/// Rewrite `href` attributes to tracked links and append the pixel.
pub fn instrument(
    html: &str,
    secret: &str,
    public_url: &str,
    job_id: Uuid,
    tracking: Tracking,
) -> String {
    let mut out = if tracking.clicks {
        rewrite_links(html, |href| {
            let target = decode_entities(href);
            if is_trackable(&target, public_url) {
                Some(encode_entities(&click_url(
                    secret, public_url, job_id, &target,
                )))
            } else {
                None
            }
        })
    } else {
        html.to_string()
    };
    if tracking.opens {
        let pixel = format!(
            r#"<img src="{}" width="1" height="1" alt="" style="display:none;width:1px;height:1px;border:0">"#,
            open_url(secret, public_url, job_id)
        );
        let lower = out.to_ascii_lowercase();
        match lower.rfind("</body>") {
            Some(i) => out.insert_str(i, &pixel),
            None => out.push_str(&pixel),
        }
    }
    out
}

/// Replace every `href="…"` / `href='…'` value for which `f` returns a
/// replacement. A small scanner, so the email needs no HTML parser.
fn rewrite_links(html: &str, f: impl Fn(&str) -> Option<String>) -> String {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len() + 256);
    let mut i = 0;
    while i < bytes.len() {
        let rest = &html[i..];
        let Some(pos) = find_href(rest) else {
            out.push_str(rest);
            break;
        };
        let attr_start = i + pos;
        // Copy everything up to and including `href=` (any spacing).
        let mut j = attr_start + 4;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'=' {
            out.push_str(&html[i..j]);
            i = j;
            continue;
        }
        j += 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || (bytes[j] != b'"' && bytes[j] != b'\'') {
            out.push_str(&html[i..j]);
            i = j;
            continue;
        }
        let quote = bytes[j];
        let value_start = j + 1;
        let Some(len) = html[value_start..].find(quote as char) else {
            out.push_str(rest);
            break;
        };
        let value = &html[value_start..value_start + len];
        out.push_str(&html[i..value_start]);
        match f(value) {
            Some(replacement) => out.push_str(&replacement),
            None => out.push_str(value),
        }
        i = value_start + len;
    }
    out
}

/// Position of the next `href` attribute name (case-insensitive, preceded by
/// whitespace) in `s`.
fn find_href(s: &str) -> Option<usize> {
    let lower = s.to_ascii_lowercase();
    let mut from = 0;
    while let Some(p) = lower[from..].find("href") {
        let at = from + p;
        let preceded_by_space = at > 0 && lower.as_bytes()[at - 1].is_ascii_whitespace();
        if preceded_by_space {
            return Some(at);
        }
        from = at + 4;
    }
    None
}

async fn record(state: &Arc<AppState>, job_id: Uuid, event: &str, payload: Value) {
    let column = if event == "email.opened" {
        "opened_at"
    } else {
        "clicked_at"
    };
    let id = format!("notifyd:{}:{}", &event[6..], Uuid::new_v4());
    if let Err(e) = sqlx::query(
        "INSERT INTO provider_events (id, provider, event_type, job_id, payload) VALUES ($1, 'notifyd', $2, $3, $4)",
    )
    .bind(&id)
    .bind(event)
    .bind(job_id)
    .bind(&payload)
    .execute(&state.pool)
    .await
    {
        tracing::warn!("tracking event insert failed: {}", e);
        return;
    }
    let sql = format!(
        "UPDATE jobs SET {column} = COALESCE({column}, now()) WHERE id = $1 AND channel = 'email'"
    );
    if let Err(e) = sqlx::query(&sql).bind(job_id).execute(&state.pool).await {
        tracing::warn!("tracking job update failed: {}", e);
    }
}

fn pixel_response() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/gif"),
            (header::CACHE_CONTROL, "no-store, private"),
        ],
        PIXEL.to_vec(),
    )
        .into_response()
}

/// GET /t/o/:token.gif — always answers with the pixel, records the open
/// when the token is genuine.
pub async fn open(State(state): State<Arc<AppState>>, Path(token): Path<String>) -> Response {
    let token = token.strip_suffix(".gif").unwrap_or(&token);
    if let Some(payload) = verify(&state.config.server.jwt_secret, token) {
        if let Some(job) = payload
            .strip_prefix("o|")
            .and_then(|id| Uuid::parse_str(id).ok())
        {
            record(&state, job, "email.opened", serde_json::json!({})).await;
        }
    }
    pixel_response()
}

/// GET /t/c/:token — redirects to the URL that was in the email.
pub async fn click(State(state): State<Arc<AppState>>, Path(token): Path<String>) -> Response {
    let Some(payload) = verify(&state.config.server.jwt_secret, &token) else {
        return (StatusCode::NOT_FOUND, "unknown link").into_response();
    };
    let Some(rest) = payload.strip_prefix("c|") else {
        return (StatusCode::NOT_FOUND, "unknown link").into_response();
    };
    let Some((job, url)) = rest.split_once('|') else {
        return (StatusCode::NOT_FOUND, "unknown link").into_response();
    };
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return (StatusCode::NOT_FOUND, "unknown link").into_response();
    }
    if let Ok(job) = Uuid::parse_str(job) {
        // Only the host is kept: a clicked URL may carry a one-time sign-in
        // or reset token, which has no business in an events table.
        let host = url
            .split_once("://")
            .map(|(_, rest)| rest)
            .and_then(|rest| rest.split(['/', '?', '#']).next())
            .unwrap_or("")
            .to_string();
        record(
            &state,
            job,
            "email.clicked",
            serde_json::json!({ "host": host }),
        )
        .await;
    }
    // 302 rather than 307: the conventional answer for tracked links, and the
    // one every mail client and link scanner has always followed.
    (
        StatusCode::FOUND,
        [
            (header::LOCATION, url),
            (header::CACHE_CONTROL, "no-store, private"),
        ],
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "s3cret";
    const PUBLIC: &str = "https://n.example.com";

    #[test]
    fn settings_and_request_switches() {
        assert_eq!(Tracking::from_settings(None), Tracking::ON);
        assert_eq!(
            Tracking::from_settings(Some(&serde_json::json!({"tracking": false}))),
            Tracking::OFF
        );
        let partial =
            Tracking::from_settings(Some(&serde_json::json!({"tracking": {"opens": false}})));
        assert_eq!(
            partial,
            Tracking {
                opens: false,
                clicks: true,
                all_email: false
            }
        );
        let all = Tracking::from_settings(Some(
            &serde_json::json!({"tracking": {"applies_to": "all"}}),
        ));
        assert!(all.all_email && all.opens && all.clicks);
        assert_eq!(
            Tracking::ON.with_request(&serde_json::json!({"track": false})),
            Tracking::OFF
        );
        assert_eq!(
            Tracking::OFF.with_request(&serde_json::json!({"track": true})),
            Tracking::OFF,
            "a request cannot re-enable what the project disabled"
        );
    }

    #[test]
    fn scope_is_marketing_unless_extended() {
        let t = Tracking::ON;
        assert!(t.applies(true, &serde_json::json!({})), "marketing: always");
        assert!(
            !t.applies(false, &serde_json::json!({})),
            "transactional: not by default"
        );
        assert!(
            t.applies(false, &serde_json::json!({"track": true})),
            "request opts in"
        );
        assert!(t.applies(false, &serde_json::json!({"track": {"clicks": true}})));
        assert!(!t.applies(false, &serde_json::json!({"track": false})));
        let all = Tracking::from_settings(Some(
            &serde_json::json!({"tracking": {"applies_to": "all"}}),
        ));
        assert!(
            all.applies(false, &serde_json::json!({})),
            "project extends to all mail"
        );
    }

    #[test]
    fn tokens_verify_and_reject_tampering() {
        let job = Uuid::new_v4();
        let url = open_url(SECRET, PUBLIC, job);
        let tok = url
            .rsplit('/')
            .next()
            .unwrap()
            .strip_suffix(".gif")
            .unwrap();
        assert_eq!(verify(SECRET, tok).unwrap(), format!("o|{job}"));
        assert!(verify("other", tok).is_none());
        let click = click_url(SECRET, PUBLIC, job, "https://shop.example.com/a?b=1&c=2");
        let tok = click.rsplit('/').next().unwrap();
        assert_eq!(
            verify(SECRET, tok).unwrap(),
            format!("c|{job}|https://shop.example.com/a?b=1&c=2")
        );
    }

    #[test]
    fn instrument_rewrites_links_and_appends_pixel() {
        let job = Uuid::new_v4();
        let html = r#"<html><body><p>Hi</p><a href="https://shop.example.com/a?b=1&amp;c=2">Buy</a> <a HREF='http://x.test/y'>y</a> <a href="mailto:hi@example.com">mail</a> <a href="https://n.example.com/u/abc.def">unsubscribe</a><a href="{{link}}">tmpl</a></body></html>"#;
        let out = instrument(html, SECRET, PUBLIC, job, Tracking::ON);
        assert_eq!(
            out.matches("https://n.example.com/t/c/").count(),
            2,
            "two http links wrapped"
        );
        assert!(out.contains(r#"href="mailto:hi@example.com""#));
        assert!(out.contains(r#"href="https://n.example.com/u/abc.def""#));
        assert!(out.contains(r#"href="{{link}}""#));
        assert!(out.contains("/t/o/"));
        assert!(out.ends_with("</body></html>"), "pixel goes before </body>");
        // The wrapped link decodes back to the original URL with a real ampersand.
        let start = out.find("/t/c/").unwrap() + 5;
        let end = out[start..].find('"').unwrap();
        let payload = verify(SECRET, &out[start..start + end]).unwrap();
        assert!(payload.ends_with("|https://shop.example.com/a?b=1&c=2"));
    }

    #[test]
    fn instrument_respects_switches_and_odd_markup() {
        let job = Uuid::new_v4();
        let html = r#"<a href = "https://a.test">a</a><img data-href="x"><span>no body tag</span>"#;
        let clicks_only = instrument(
            html,
            SECRET,
            PUBLIC,
            job,
            Tracking {
                opens: false,
                clicks: true,
                all_email: false,
            },
        );
        assert!(clicks_only.contains("/t/c/") && !clicks_only.contains("/t/o/"));
        assert!(
            clicks_only.contains(r#"data-href="x""#),
            "data-href is not a link"
        );
        let opens_only = instrument(
            html,
            SECRET,
            PUBLIC,
            job,
            Tracking {
                opens: true,
                clicks: false,
                all_email: false,
            },
        );
        assert!(
            opens_only.contains(r#"href = "https://a.test""#)
                && opens_only.ends_with("border:0\">")
        );
        assert_eq!(instrument(html, SECRET, PUBLIC, job, Tracking::OFF), html);
    }
}
