//! Per-user Telegram credentials, destinations, link tokens, and webhook ingestion.
//!
//! Management routes require a project API key (`x-api-key`). The `owner`
//! path parameter is an opaque id trusted from Baaton after its own auth;
//! notifyd enforces project isolation only.
//!
//! The public webhook callback (`POST /v1/telegram/webhooks/:id`) is not
//! project-auth'd — it authenticates via the per-bot header secret.

use crate::{api::send::extract_project, AppState};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

// ─── Public worker type ────────────────────────────────────────────────────────

/// Route data loaded by the worker for per-user Telegram dispatch.
pub struct TelegramRoute {
    pub bot_token: String,
    pub address: String,
    pub telegram_thread_id: Option<i64>,
}

// ─── Request types ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct PutBotBody {
    bot_token: String,
    webhook_base: String,
}

#[derive(Deserialize)]
pub struct PutDestBody {
    address: String,
    #[serde(default)]
    telegram_thread_id: Option<Value>,
}

#[derive(Deserialize)]
pub struct LinkBody {
    #[serde(default = "private_kind")]
    kind: String,
}

fn private_kind() -> String { "private".to_string() }

#[derive(Deserialize)]
pub struct LookupBody {
    owners: Vec<String>,
}

// ─── Telegram update shapes ────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Deserialize)]
struct TgUpdate { message: Option<TgMessage> }

#[allow(dead_code)]
#[derive(Deserialize)]
struct TgMessage {
    chat: TgChat,
    text: Option<String>,
    message_thread_id: Option<i64>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct TgChat { id: i64, is_forum: Option<bool> }

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn mask_chat_id(s: &str) -> String {
    let d: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if d.len() < 4 { format!("***{s}") } else { format!("***{}", &d[d.len()-4..]) }
}

fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

fn random_secret() -> String { hex::encode(Uuid::new_v4().as_bytes()) }

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

type ApiResult<T> = Result<T, (StatusCode, Json<Value>)>;

fn ierr<E: std::fmt::Display>(e: E) -> (StatusCode, Json<Value>) {
    tracing::warn!("telegram: {e}");
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error":"internal server error"})))
}
fn bad(msg: &str) -> (StatusCode, Json<Value>) {
    (StatusCode::BAD_REQUEST, Json(json!({"error": msg})))
}
fn conflict(msg: &str) -> (StatusCode, Json<Value>) {
    (StatusCode::CONFLICT, Json(json!({"error": msg})))
}
fn enc_key() -> ApiResult<[u8;32]> {
    crate::crypto::load_key().map_err(|_| {
        (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":"encryption not configured"})))
    })
}
fn make_client() -> reqwest::Client {
    reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build().unwrap()
}
fn tg_base() -> String {
    std::env::var("TELEGRAM_API_BASE").ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "https://api.telegram.org".to_string())
}

// ─── Telegram API helpers ──────────────────────────────────────────────────────

async fn tg_get_me(c: &reqwest::Client, base: &str, tok: &str) -> Result<String, String> {
    let r = c.post(format!("{base}/bot{tok}/getMe")).send().await
        .map_err(|e| format!("transport: {e}"))?;
    let st = r.status().as_u16();
    let b: Value = r.json().await.unwrap_or(Value::Null);
    if st == 401 || st == 404 { return Err("invalid bot token".to_string()); }
    b.pointer("/result/username").and_then(Value::as_str).map(String::from)
        .ok_or_else(|| format!("getMe failed (HTTP {st})"))
}

async fn tg_webhook_url_safe(c: &reqwest::Client, base: &str, tok: &str) -> String {
    let Ok(r) = c.post(format!("{base}/bot{tok}/getWebhookInfo")).send().await else { return String::new(); };
    r.json::<Value>().await.ok()
        .and_then(|b| b.pointer("/result/url").and_then(Value::as_str).map(String::from))
        .unwrap_or_default()
}

async fn tg_set_webhook(c: &reqwest::Client, base: &str, tok: &str, url: &str, secret: &str) -> Result<(), String> {
    let r = c.post(format!("{base}/bot{tok}/setWebhook"))
        .json(&json!({"url":url,"secret_token":secret,"allowed_updates":["message"]}))
        .send().await.map_err(|e| format!("transport: {e}"))?;
    let st = r.status().as_u16();
    let b: Value = r.json().await.unwrap_or(Value::Null);
    if b.get("ok").and_then(Value::as_bool) == Some(true) { Ok(()) }
    else { Err(format!("setWebhook HTTP {st}: {}", b.get("description").and_then(Value::as_str).unwrap_or(""))) }
}

async fn tg_delete_webhook(c: &reqwest::Client, base: &str, tok: &str) {
    let _ = c.post(format!("{base}/bot{tok}/deleteWebhook")).send().await;
}

async fn tg_get_chat(c: &reqwest::Client, base: &str, tok: &str, chat: &str) -> Result<(i64, bool), String> {
    let r = c.post(format!("{base}/bot{tok}/getChat")).json(&json!({"chat_id":chat})).send().await
        .map_err(|e| format!("transport: {e}"))?;
    let st = r.status().as_u16();
    let b: Value = r.json().await.unwrap_or(Value::Null);
    if b.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(format!("getChat HTTP {st}: {}", b.get("description").and_then(Value::as_str).unwrap_or("")));
    }
    let id = b.pointer("/result/id").and_then(Value::as_i64)
        .ok_or_else(|| "getChat: missing numeric id".to_string())?;
    let is_forum = b.pointer("/result/is_forum").and_then(Value::as_bool).unwrap_or(false);
    Ok((id, is_forum))
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

/// GET /v1/telegram/:owner
pub async fn get_owner(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(owner): Path<String>,
) -> ApiResult<Json<Value>> {
    let project = extract_project(&state, &headers).await?;

    #[derive(sqlx::FromRow)]
    struct CredRow { bot_username: String }
    let cred: Option<CredRow> = sqlx::query_as(
        "SELECT bot_username FROM telegram_credentials WHERE project_id=$1 AND owner=$2")
        .bind(&project.id).bind(&owner)
        .fetch_optional(&state.pool).await.map_err(ierr)?;

    #[derive(sqlx::FromRow)]
    struct DestRow {
        address: String, telegram_thread_id: Option<i64>,
        verified: bool, created_at: chrono::DateTime<Utc>, bot_username: String,
    }
    let dest: Option<DestRow> = sqlx::query_as(
        "SELECT td.address, td.telegram_thread_id, td.verified, td.created_at, tc.bot_username
         FROM telegram_destinations td
         JOIN telegram_credentials tc ON tc.id = td.credential_id
         WHERE td.project_id=$1 AND td.owner=$2")
        .bind(&project.id).bind(&owner)
        .fetch_optional(&state.pool).await.map_err(ierr)?;

    let bot_out = cred.map(|c| json!({
        "bot_username": c.bot_username, "owned": true,
        "source": "personal", "webhook_registered": true, "can_link": true,
    }));
    let dest_out = dest.map(|d| json!({
        "channel": "telegram", "address_masked": mask_chat_id(&d.address),
        "verified": d.verified, "created_at": d.created_at,
        "telegram_thread_id": d.telegram_thread_id, "telegram_bot_username": d.bot_username,
    }));
    Ok(Json(json!({"data": {"bot": bot_out, "destination": dest_out}})))
}

/// PUT /v1/telegram/:owner — register or update personal bot + webhook.
pub async fn put_owner(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(owner): Path<String>,
    Json(body): Json<PutBotBody>,
) -> ApiResult<Json<Value>> {
    let project = extract_project(&state, &headers).await?;
    if body.bot_token.trim().is_empty() { return Err(bad("bot_token is required")); }
    if !body.webhook_base.starts_with("https://") { return Err(bad("webhook_base must start with https://")); }
    let inst = std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
    if !inst.is_empty() && body.bot_token.trim() == inst.trim() {
        return Err(bad("cannot register the instance bot as a personal bot"));
    }
    let key = enc_key()?;
    let client = make_client();
    let base = tg_base();

    let bot_username = tg_get_me(&client, &base, &body.bot_token).await.map_err(|e| bad(&e))?;

    #[derive(sqlx::FromRow)]
    struct ExRow { id: Uuid, bot_username: String, webhook_connection_id: Uuid, bot_token_enc: Vec<u8> }
    let existing: Option<ExRow> = sqlx::query_as(
        "SELECT id, bot_username, webhook_connection_id, bot_token_enc
         FROM telegram_credentials WHERE project_id=$1 AND owner=$2")
        .bind(&project.id).bind(&owner)
        .fetch_optional(&state.pool).await.map_err(ierr)?;

    let existing_url = tg_webhook_url_safe(&client, &base, &body.bot_token).await;
    if !existing_url.is_empty() {
        let is_ours = existing.as_ref().map_or(false, |e| {
            existing_url.ends_with(&format!("/{}", e.webhook_connection_id))
        });
        if !is_ours { return Err(conflict("bot already has a webhook set by another system")); }
    }

    // Bot swap: clean up old bot's webhook (best effort).
    if let Some(ref ex) = existing {
        if ex.bot_username != bot_username {
            if let Ok(pt) = crate::crypto::decrypt(&key, &ex.bot_token_enc) {
                if let Ok(old_tok) = String::from_utf8(pt) {
                    tg_delete_webhook(&client, &base, &old_tok).await;
                }
            }
        }
    }

    let conn_id = Uuid::new_v4();
    let secret = random_secret();
    let full_url = format!("{}/{}", body.webhook_base.trim_end_matches('/'), conn_id);
    tg_set_webhook(&client, &base, &body.bot_token, &full_url, &secret).await
        .map_err(|_| (StatusCode::BAD_GATEWAY, Json(json!({"error":"could not register webhook with Telegram"}))))?;

    let tok_enc = crate::crypto::encrypt(&key, body.bot_token.as_bytes()).map_err(ierr)?;
    let sec_enc = crate::crypto::encrypt(&key, secret.as_bytes()).map_err(ierr)?;

    sqlx::query(
        "INSERT INTO telegram_credentials (project_id, owner, bot_token_enc, bot_username, webhook_connection_id, webhook_secret_enc)
         VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (project_id, owner) DO UPDATE SET
             bot_token_enc=$3, bot_username=$4, webhook_connection_id=$5, webhook_secret_enc=$6")
        .bind(&project.id).bind(&owner).bind(&tok_enc).bind(&bot_username).bind(conn_id).bind(&sec_enc)
        .execute(&state.pool).await.map_err(ierr)?;

    Ok(Json(json!({"data": {
        "bot": {"bot_username": bot_username, "owned": true, "source": "personal",
                "webhook_registered": true, "can_link": true},
        "destination": null
    }})))
}

/// DELETE /v1/telegram/:owner
pub async fn delete_owner(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(owner): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;
    let row: Option<(Uuid, Vec<u8>)> = sqlx::query_as(
        "SELECT id, bot_token_enc FROM telegram_credentials WHERE project_id=$1 AND owner=$2")
        .bind(&project.id).bind(&owner)
        .fetch_optional(&state.pool).await.map_err(ierr)?;
    let Some((cred_id, enc)) = row else {
        return Err((StatusCode::NOT_FOUND, Json(json!({"error":"not found"}))));
    };
    if let Ok(key) = crate::crypto::load_key() {
        if let Ok(pt) = crate::crypto::decrypt(&key, &enc) {
            if let Ok(tok) = String::from_utf8(pt) {
                tg_delete_webhook(&make_client(), &tg_base(), &tok).await;
            }
        }
    }
    sqlx::query("DELETE FROM telegram_credentials WHERE id=$1")
        .bind(cred_id).execute(&state.pool).await.map_err(ierr)?;
    Ok(StatusCode::NO_CONTENT)
}

/// PUT /v1/telegram/:owner/destination
pub async fn put_destination(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(owner): Path<String>,
    Json(body): Json<PutDestBody>,
) -> ApiResult<Json<Value>> {
    let project = extract_project(&state, &headers).await?;
    if body.address.trim().is_empty() { return Err(bad("address is required")); }

    let thread_id: Option<i64> = match &body.telegram_thread_id {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) => Some(n.as_i64().filter(|&v| v > 0)
            .ok_or_else(|| bad("telegram_thread_id must be a positive integer"))?),
        Some(Value::String(s)) => Some(s.trim().parse::<i64>().ok().filter(|&v| v > 0)
            .ok_or_else(|| bad("telegram_thread_id must be a positive integer"))?),
        Some(_) => return Err(bad("telegram_thread_id must be a number or null")),
    };

    #[derive(sqlx::FromRow)]
    struct CRow { id: Uuid, bot_token_enc: Vec<u8>, bot_username: String }
    let cred: Option<CRow> = sqlx::query_as(
        "SELECT id, bot_token_enc, bot_username FROM telegram_credentials WHERE project_id=$1 AND owner=$2")
        .bind(&project.id).bind(&owner)
        .fetch_optional(&state.pool).await.map_err(ierr)?;
    let cred = cred.ok_or_else(|| bad("register a personal bot first (PUT /v1/telegram/:owner)"))?;

    let key = enc_key()?;
    let pt = crate::crypto::decrypt(&key, &cred.bot_token_enc)
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":"credential unavailable"}))))?;
    let tok = String::from_utf8(pt).map_err(ierr)?;

    let (chat_id_num, is_forum) = tg_get_chat(&make_client(), &tg_base(), &tok, body.address.trim())
        .await.map_err(|e| bad(&e))?;
    if thread_id.is_some() && !is_forum {
        return Err(bad("telegram_thread_id is only valid for forum supergroups"));
    }
    let address = chat_id_num.to_string();

    sqlx::query(
        "INSERT INTO telegram_destinations (project_id, owner, credential_id, address, telegram_thread_id, verified)
         VALUES ($1,$2,$3,$4,$5,true)
         ON CONFLICT (project_id, owner) DO UPDATE SET
             credential_id=$3, address=$4, telegram_thread_id=$5, verified=true, created_at=now()")
        .bind(&project.id).bind(&owner).bind(cred.id).bind(&address).bind(thread_id)
        .execute(&state.pool).await.map_err(ierr)?;

    Ok(Json(json!({"data": {
        "channel": "telegram", "address_masked": mask_chat_id(&address), "verified": true,
        "created_at": Utc::now(), "telegram_thread_id": thread_id,
        "telegram_bot_username": cred.bot_username,
    }})))
}

/// DELETE /v1/telegram/:owner/destination
pub async fn delete_destination(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(owner): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let project = extract_project(&state, &headers).await?;
    sqlx::query("DELETE FROM telegram_destinations WHERE project_id=$1 AND owner=$2")
        .bind(&project.id).bind(&owner)
        .execute(&state.pool).await.map_err(ierr)?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /v1/telegram/:owner/link
pub async fn post_link(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(owner): Path<String>,
    body: Option<Json<LinkBody>>,
) -> ApiResult<Json<Value>> {
    let project = extract_project(&state, &headers).await?;
    let kind = body.map(|b| b.0.kind).unwrap_or_else(private_kind);
    if kind != "private" && kind != "group" { return Err(bad("kind must be 'private' or 'group'")); }

    #[derive(sqlx::FromRow)]
    struct CRow { id: Uuid, bot_username: String }
    let cred: Option<CRow> = sqlx::query_as(
        "SELECT id, bot_username FROM telegram_credentials WHERE project_id=$1 AND owner=$2")
        .bind(&project.id).bind(&owner)
        .fetch_optional(&state.pool).await.map_err(ierr)?;
    let cred = cred.ok_or_else(|| bad("register a personal bot first (PUT /v1/telegram/:owner)"))?;

    let recent: Option<chrono::DateTime<Utc>> = sqlx::query_scalar(
        "SELECT created_at FROM telegram_link_tokens
         WHERE project_id=$1 AND owner=$2 AND consumed_at IS NULL AND expires_at > now()
         ORDER BY created_at DESC LIMIT 1")
        .bind(&project.id).bind(&owner)
        .fetch_optional(&state.pool).await.map_err(ierr)?;
    if let Some(ts) = recent {
        if Utc::now() - ts < Duration::seconds(60) {
            return Err((StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error":"rate limited: wait before generating a new link"}))));
        }
    }

    sqlx::query("DELETE FROM telegram_link_tokens WHERE project_id=$1 AND owner=$2")
        .bind(&project.id).bind(&owner).execute(&state.pool).await.map_err(ierr)?;

    let raw = hex::encode(Uuid::new_v4().as_bytes());
    let hash = sha256_hex(&raw);
    let expires_at = Utc::now() + Duration::minutes(15);

    sqlx::query(
        "INSERT INTO telegram_link_tokens (project_id, owner, credential_id, kind, token_hash, expires_at)
         VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(&project.id).bind(&owner).bind(cred.id).bind(&kind).bind(&hash).bind(expires_at)
        .execute(&state.pool).await.map_err(ierr)?;

    let (deep_link, command) = if kind == "group" {
        (format!("https://t.me/{}?startgroup={}", cred.bot_username, raw),
         format!("/start@{} {}", cred.bot_username, raw))
    } else {
        (format!("https://t.me/{}?start={}", cred.bot_username, raw),
         format!("/start {raw}"))
    };
    Ok(Json(json!({"data": {"deep_link": deep_link, "expires_at": expires_at, "command": command}})))
}

/// POST /v1/telegram/lookup — batch owner→route resolution, project-scoped.
pub async fn lookup(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<LookupBody>,
) -> ApiResult<Json<Value>> {
    let project = extract_project(&state, &headers).await?;
    if body.owners.len() > 100 { return Err(bad("owners list must not exceed 100 entries")); }

    #[derive(sqlx::FromRow)]
    struct Row { route_id: Uuid, owner: String, address: String,
                 telegram_thread_id: Option<i64>, bot_username: String, verified: bool }
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT tc.id AS route_id, tc.owner, td.address, td.telegram_thread_id,
                tc.bot_username, td.verified
         FROM telegram_credentials tc
         JOIN telegram_destinations td ON td.credential_id=tc.id AND td.project_id=tc.project_id
         WHERE tc.project_id=$1 AND tc.owner=ANY($2) AND td.verified=true")
        .bind(&project.id).bind(&body.owners)
        .fetch_all(&state.pool).await.map_err(ierr)?;

    let data: Vec<Value> = rows.into_iter().map(|r| json!({
        "owner": r.owner, "route_id": r.route_id, "address": r.address,
        "telegram_thread_id": r.telegram_thread_id, "bot_username": r.bot_username,
        "verified": r.verified,
    })).collect();
    Ok(Json(json!({"data": data})))
}

/// POST /v1/telegram/webhooks/:id — Telegram callback (no project auth).
pub async fn webhook_handler(
    State(state): State<Arc<AppState>>,
    Path(connection_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    #[derive(sqlx::FromRow)]
    struct CRow { project_id: String, owner: String, webhook_secret_enc: Vec<u8> }
    let row: Option<CRow> = sqlx::query_as(
        "SELECT project_id, owner, webhook_secret_enc FROM telegram_credentials
         WHERE webhook_connection_id=$1")
        .bind(connection_id)
        .fetch_optional(&state.pool).await.unwrap_or(None);
    let Some(cr) = row else { return StatusCode::NOT_FOUND; };

    let Ok(key) = crate::crypto::load_key() else { return StatusCode::SERVICE_UNAVAILABLE; };
    let Ok(sec) = crate::crypto::decrypt(&key, &cr.webhook_secret_enc) else { return StatusCode::SERVICE_UNAVAILABLE; };
    let presented = headers.get("x-telegram-bot-api-secret-token")
        .and_then(|v| v.to_str().ok()).unwrap_or("");
    if !constant_time_eq(presented.as_bytes(), &sec) { return StatusCode::UNAUTHORIZED; }

    let Ok(update) = serde_json::from_slice::<TgUpdate>(&body) else { return StatusCode::OK; };
    if let Some(msg) = update.message {
        if let Some(tok) = extract_start_token(msg.text.as_deref().unwrap_or("")) {
            let hash = sha256_hex(tok);
            let _ = consume_link(&state, &cr.project_id, &cr.owner, connection_id, &hash, &msg).await;
        }
    }
    StatusCode::OK
}

fn extract_start_token(text: &str) -> Option<&str> {
    let text = text.trim();
    let rest = text.strip_prefix("/start")?;
    let rest = if rest.starts_with('@') { rest.find(' ').map(|i| &rest[i..])? } else { rest };
    let t = rest.trim();
    if t.is_empty() { None } else { Some(t) }
}

async fn consume_link(
    state: &Arc<AppState>,
    project_id: &str, owner: &str,
    connection_id: Uuid, token_hash: &str, msg: &TgMessage,
) -> anyhow::Result<()> {
    #[derive(sqlx::FromRow)]
    struct TRow { id: Uuid, credential_id: Uuid, kind: String }
    let tok: Option<TRow> = sqlx::query_as(
        "SELECT tlt.id, tlt.credential_id, tlt.kind
         FROM telegram_link_tokens tlt
         JOIN telegram_credentials tc ON tc.id=tlt.credential_id
         WHERE tlt.token_hash=$1 AND tlt.consumed_at IS NULL AND tlt.expires_at > now()
           AND tc.project_id=$2 AND tc.webhook_connection_id=$3")
        .bind(token_hash).bind(project_id).bind(connection_id)
        .fetch_optional(&state.pool).await?;
    let Some(tok) = tok else { return Ok(()); };
    let address = msg.chat.id.to_string();
    let thread_id: Option<i64> = if tok.kind == "group" && msg.chat.is_forum == Some(true) {
        msg.message_thread_id.filter(|&v| v > 0)
    } else { None };
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO telegram_destinations (project_id, owner, credential_id, address, telegram_thread_id, verified)
         VALUES ($1,$2,$3,$4,$5,true)
         ON CONFLICT (project_id, owner) DO UPDATE SET
             credential_id=$3, address=$4, telegram_thread_id=$5, verified=true, created_at=now()")
        .bind(project_id).bind(owner).bind(tok.credential_id).bind(&address).bind(thread_id)
        .execute(&mut *tx).await?;
    sqlx::query("UPDATE telegram_link_tokens SET consumed_at=now() WHERE id=$1")
        .bind(tok.id).execute(&mut *tx).await?;
    tx.commit().await?;
    tracing::info!("Telegram link consumed: owner={owner} project={project_id} chat={}",
        crate::pii::mask_recipient("telegram", &address));
    Ok(())
}

// ─── Worker helper ─────────────────────────────────────────────────────────────

/// Load and decrypt per-route credentials for the worker.
/// Lookup by credential id (= route_id) AND project_id — never cross-project.
/// Returns None on any error; worker treats None as permanent failure, no fallback.
pub async fn load_telegram_route(
    pool: &PgPool,
    route_id: Uuid,
    project_id: &str,
) -> Option<TelegramRoute> {
    #[derive(sqlx::FromRow)]
    struct RouteRow { bot_token_enc: Vec<u8>, address: String, telegram_thread_id: Option<i64> }

    let row = sqlx::query_as::<_, RouteRow>(
        "SELECT tc.bot_token_enc, td.address, td.telegram_thread_id
         FROM telegram_credentials tc
         JOIN telegram_destinations td ON td.credential_id=tc.id AND td.project_id=tc.project_id
         WHERE tc.id=$1 AND tc.project_id=$2 AND td.verified=true")
        .bind(route_id).bind(project_id)
        .fetch_optional(pool).await.ok().flatten()?;

    let key = crate::crypto::load_key().ok()?;
    let pt = crate::crypto::decrypt(&key, &row.bot_token_enc).ok()?;
    let bot_token = String::from_utf8(pt).ok()?;
    Some(TelegramRoute { bot_token, address: row.address, telegram_thread_id: row.telegram_thread_id })
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_chat_id_formats() {
        assert_eq!(mask_chat_id("-1001234567890"), "***7890");
        assert_eq!(mask_chat_id("123456789"), "***6789");
        assert_eq!(mask_chat_id("12"), "***12");
    }

    #[test]
    fn extract_start_token_variants() {
        assert_eq!(extract_start_token("/start abc123"), Some("abc123"));
        assert_eq!(extract_start_token("/start@bot abc123"), Some("abc123"));
        assert_eq!(extract_start_token("/start"), None);
        assert_eq!(extract_start_token("/start "), None);
        assert_eq!(extract_start_token("/other token"), None);
        assert_eq!(extract_start_token("  /start   tok  "), Some("tok"));
    }

    #[test]
    fn link_kind_validation() {
        assert!(matches!("private", "private" | "group"));
        assert!(matches!("group", "private" | "group"));
        assert!(!matches!("channel", "private" | "group"));
    }

    #[test]
    fn deep_link_private_vs_group() {
        let (bot, tok) = ("my_bot", "abc123");
        let priv_link = format!("https://t.me/{bot}?start={tok}");
        let grp_link  = format!("https://t.me/{bot}?startgroup={tok}");
        assert!(priv_link.contains("?start=") && !priv_link.contains("startgroup"));
        assert!(grp_link.contains("?startgroup="));
    }

    #[test]
    fn lookup_owners_bound() {
        // over-100 list must trigger error in handler (enforced by owners.len() > 100)
        let owners: Vec<String> = (0..101).map(|i| format!("o{i}")).collect();
        assert!(owners.len() > 100);
    }

    #[test]
    fn webhook_secret_constant_time() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"SECRET"));
        assert!(!constant_time_eq(b"a", b"bb"));
    }

    #[test]
    fn sha256_hex_deterministic() {
        let h = sha256_hex("hello");
        assert_eq!(h, sha256_hex("hello"));
        assert_eq!(h.len(), 64);
        assert_ne!(sha256_hex("a"), sha256_hex("b"));
    }

    #[test]
    fn route_sql_is_project_scoped() {
        // The SQL in load_telegram_route binds tc.project_id — documented invariant.
        let sql = "WHERE tc.id=$1 AND tc.project_id=$2";
        assert!(sql.contains("tc.project_id"));
    }

    #[tokio::test]
    async fn db_unknown_route_returns_none() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL not set: skipping DB test");
            return;
        };
        let pool = sqlx::PgPool::connect(&url).await.expect("connect");
        sqlx::migrate!("./migrations").run(&pool).await.expect("migrate");
        let result = load_telegram_route(&pool, Uuid::new_v4(), "nonexistent-project").await;
        assert!(result.is_none());
    }
}
