//! Model Context Protocol server, Streamable HTTP transport, stateless,
//! **dual-era**: it speaks the current revision (2026-07-28: no
//! `initialize`, no session, `server/discover`, `_meta` on every request,
//! `Mcp-Method`/`Mcp-Name` headers, `resultType`, cache hints) and the
//! legacy revisions (2024-11-05 → 2025-11-25: `initialize` handshake) on
//! the same `POST /mcp`. A request is modern when it carries
//! `params._meta["io.modelcontextprotocol/protocolVersion"]`; otherwise it
//! is legacy. Clients that probe modern-first and fall back to `initialize`
//! (Claude Code v2) get a clean answer either way.
//!
//! Auth: `Authorization: Bearer <ADMIN_API_KEY>` (or `x-api-key`). `Origin`
//! is validated against `CORS_ORIGINS` (403 otherwise). One JSON-RPC message
//! per POST: batches were removed from the protocol in 2025-06-18 and are
//! rejected. Notifications → 202 with no body. `GET /mcp` → 405. Every tool
//! wraps a function of `ops`, so the MCP surface and the REST admin API
//! cannot drift apart; every `tools/call` is rate limited and audited.

use crate::{ops, AppState};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

/// Newest revision we implement; also the answer to a legacy `initialize`
/// that asks for something we do not know.
pub const CURRENT_VERSION: &str = "2026-07-28";
pub const LEGACY_DEFAULT_VERSION: &str = "2025-06-18";
pub const SUPPORTED_VERSIONS: &[&str] = &[
    "2026-07-28",
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];
const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";
/// `tools/list` and `server/discover` are identical for every admin key.
const LIST_TTL_MS: u64 = 300_000;
/// Tool calls per minute for the (single) admin principal.
const TOOL_CALLS_PER_MIN: u32 = 600;

const INSTRUCTIONS: &str = "notifyd is a self-hosted notification queue (email, SMS, WhatsApp, in-app, push). Start with `digest` to see the state of this instance; its findings say what to do. Use list_jobs/get_job to investigate, retry_job after fixing a cause, update_project to set a sender, send_test to prove a channel. Read-only tools carry readOnlyHint; retry_job and send_test enqueue real messages.";

fn server_info() -> Value {
    json!({ "name": "notifyd", "version": env!("CARGO_PKG_VERSION") })
}

fn capabilities() -> Value {
    json!({ "tools": { "listChanged": false } })
}

/// Annotations shared by every read-only tool: no side effect, our own
/// database, safe to call without confirmation.
fn read_only() -> Value {
    json!({ "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false })
}

/// Additive, repeatable mutation (same arguments → same end state).
fn idempotent_write() -> Value {
    json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false })
}

/// Mutation that changes what recipients receive or stops something.
fn destructive_write() -> Value {
    json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": true, "openWorldHint": false })
}

/// Enqueues a real message to a real recipient.
fn sends_message() -> Value {
    json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": true })
}

/// Tool catalogue: name, description, JSON Schema of the arguments.
pub fn tools() -> Value {
    json!([
        {
            "name": "digest",
            "annotations": read_only(),
            "outputSchema": {"type": "object", "properties": {"findings": {"type": "array"}, "queue": {"type": "object"}, "outcomes": {"type": "array"}, "projects": {"type": "array"}}, "required": ["findings", "queue"]},
            "description": "Health and activity digest of this notifyd instance: findings ranked by severity (with the action to take), queue depth, outcomes per channel/provider, failure reasons, retries waiting, latency, deliverability, projects. Call this first when asked how notifications are doing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "window": {"type": "string", "enum": ["1h", "6h", "24h", "7d", "30d"], "description": "Lookback window (default 24h)."},
                    "format": {"type": "string", "enum": ["markdown", "json"], "description": "markdown (default) for reading, json for processing."}
                }
            }
        },
        {
            "name": "template_metrics",
            "annotations": read_only(),
            "outputSchema": {"type": "object", "properties": {"rows": {"type": "array"}}, "required": ["rows"]},
            "description": "Delivery funnel per email template over time: sent, failed, delivered, bounced, complained, opened, clicked per bucket. Template = template_id, else the `template` or `category` tag, else `untemplated`. Delivery events exist only where provider webhooks are ingested.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "window": {"type": "string", "enum": ["1h", "6h", "24h", "7d", "30d"]},
                    "bucket": {"type": "string", "enum": ["1h", "1d"]},
                    "project_id": {"type": "string"}
                }
            }
        },
        {
            "name": "list_jobs",
            "annotations": read_only(),
            "outputSchema": {"type": "object", "properties": {"jobs": {"type": "array"}, "count": {"type": "integer"}}, "required": ["jobs", "count"]},
            "description": "Search notification jobs across projects. Recipients are masked. Default: last 7 days, 50 most recent.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "status": {"type": "string", "enum": ["pending", "processing", "retry", "sent", "failed", "cancelled"]},
                    "channel": {"type": "string", "enum": ["email", "sms", "whatsapp", "in_app", "push"]},
                    "recipient": {"type": "string", "description": "Exact address or phone number."},
                    "since": {"type": "string", "format": "date-time"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500}
                }
            }
        },
        {
            "name": "get_job",
            "annotations": read_only(),
            "outputSchema": {"type": "object", "properties": {"id": {"type": "string"}, "status": {"type": "string"}, "provider": {"type": ["string", "null"]}, "provider_message_id": {"type": ["string", "null"]}}, "required": ["id", "status"]},
            "description": "One job with its attempts, provider, provider message id and delivery events.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}}, "required": ["id"]}
        },
        {
            "name": "retry_job",
            "annotations": sends_message(),
            "outputSchema": {"type": "object", "properties": {"id": {"type": "string"}, "project_id": {"type": "string"}, "channel": {"type": "string"}, "status": {"type": "string"}}, "required": ["id", "status"]},
            "description": "Re-queue a failed or cancelled job with a fresh attempt budget. Fix the cause first: a permanent error (bad address, unverified sender) will fail again.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}}, "required": ["id"]}
        },
        {
            "name": "cancel_job",
            "annotations": destructive_write(),
            "outputSchema": {"type": "object", "properties": {"id": {"type": "string"}, "project_id": {"type": "string"}, "channel": {"type": "string"}, "status": {"type": "string"}}, "required": ["id", "status"]},
            "description": "Cancel a job that is still pending or waiting for a retry.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}}, "required": ["id"]}
        },
        {
            "name": "list_projects",
            "annotations": read_only(),
            "outputSchema": {"type": "object", "properties": {"projects": {"type": "array"}}, "required": ["projects"]},
            "description": "Projects on this instance with channels, sender identity and inbound rate limit.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "update_project",
            "annotations": idempotent_write(),
            "outputSchema": {"type": "object", "properties": {"id": {"type": "string"}, "from_email": {"type": ["string", "null"]}, "from_name": {"type": ["string", "null"]}}, "required": ["id"]},
            "description": "Change a project's name, channels, sender (from_email/from_name; empty from_email clears it), inbound rate limit, or daily send window for bulk email (recipients' local time; null removes it). Keys are never touched.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "name": {"type": "string"},
                    "channels": {"type": "array", "items": {"type": "string", "enum": ["email", "sms", "whatsapp", "in_app", "push"]}},
                    "from_email": {"type": "string"},
                    "from_name": {"type": "string"},
                    "rate_limit_per_min": {"type": "integer", "minimum": 1},
                    "send_window": {
                        "description": "Bulk email waits for this daily window in the recipient's timezone (subscribers.timezone) or tz. Example {\"start\":\"09:00\",\"end\":\"20:00\",\"tz\":\"Europe/Paris\",\"days\":[1,2,3,4,5]}. null removes it.",
                        "type": ["object", "null"],
                        "properties": {
                            "start": {"type": "string", "pattern": "^[0-2][0-9]:[0-5][0-9]$"},
                            "end": {"type": "string", "pattern": "^[0-2][0-9]:[0-5][0-9]$"},
                            "tz": {"type": "string"},
                            "days": {"type": "array", "items": {"type": "integer", "minimum": 1, "maximum": 7}},
                            "applies_to": {"type": "string", "enum": ["marketing", "all"]}
                        }
                    }
                },
                "required": ["id"]
            }
        },
        {
            "name": "list_suppressions",
            "annotations": read_only(),
            "outputSchema": {"type": "object", "properties": {"suppressions": {"type": "array"}, "count": {"type": "integer"}}, "required": ["suppressions", "count"]},
            "description": "Active email suppressions (bounced, complained or manually blocked addresses), masked.",
            "inputSchema": {"type": "object", "properties": {"project_id": {"type": "string"}, "limit": {"type": "integer", "minimum": 1, "maximum": 500}}}
        },
        {
            "name": "add_suppression",
            "annotations": destructive_write(),
            "outputSchema": {"type": "object", "properties": {"id": {"type": "string"}, "project_id": {"type": "string"}, "reason": {"type": "string"}}, "required": ["id"]},
            "description": "Block an address for a project. scope=all (default): no email at all. scope=marketing: bulk/campaign email stops, transactional email (orders, security) still goes — this is what a commercial unsubscribe does.",
            "inputSchema": {"type": "object", "properties": {"project_id": {"type": "string"}, "email": {"type": "string"}, "detail": {"type": "string"}, "scope": {"type": "string", "enum": ["all", "marketing"]}}, "required": ["project_id", "email"]}
        },
        {
            "name": "release_suppression",
            "annotations": destructive_write(),
            "outputSchema": {"type": "object", "properties": {"id": {"type": "string"}, "released": {"type": "boolean"}}, "required": ["id", "released"]},
            "description": "Release a suppression so the address can receive email again.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}}, "required": ["id"]}
        },
        {
            "name": "send_test",
            "annotations": sends_message(),
            "outputSchema": {"type": "object", "properties": {"id": {"type": "string"}, "project_id": {"type": "string"}, "channel": {"type": "string"}, "status": {"type": "string"}}, "required": ["id", "status"]},
            "description": "Enqueue a high-priority test message tagged category=test on a channel of a project, to prove the pipeline end to end. Returns the job id to follow with get_job.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": {"type": "string"},
                    "channel": {"type": "string", "enum": ["email", "sms", "whatsapp", "in_app", "push"]},
                    "to": {"type": "string", "description": "Email, E.164 number or subscriber id (in_app/push)."},
                    "subject": {"type": "string"},
                    "body": {"type": "string"}
                },
                "required": ["project_id", "channel", "to", "body"]
            }
        }
    ])
}

use crate::api::projects::Operator;

/// Tools a read-only operator may call: those annotated `readOnlyHint`.
fn tool_is_read_only(tool: &Value) -> bool {
    tool["annotations"]["readOnlyHint"]
        .as_bool()
        .unwrap_or(false)
}

/// The catalogue a given operator sees.
pub fn tools_for(operator: Operator) -> Value {
    match operator {
        Operator::Admin => tools(),
        Operator::ReadOnly => Value::Array(
            tools()
                .as_array()
                .map(|t| t.iter().filter(|t| tool_is_read_only(t)).cloned().collect())
                .unwrap_or_default(),
        ),
    }
}

fn tool_allowed(operator: Operator, name: &str) -> bool {
    tools_for(operator)
        .as_array()
        .map(|t| t.iter().any(|t| t["name"] == name))
        .unwrap_or(false)
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn text_result(text: String, structured: Option<Value>, is_error: bool) -> Value {
    let mut result = json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    });
    if let Some(s) = structured {
        result["structuredContent"] = s;
    }
    result
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn arg_uuid(args: &Value, key: &str) -> Result<Uuid, String> {
    arg_str(args, key)
        .ok_or_else(|| format!("{key} is required"))?
        .parse::<Uuid>()
        .map_err(|_| format!("{key} must be a UUID"))
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// Run one tool. Errors are returned as tool results with `isError: true`
/// (the protocol's way of telling the model what went wrong), not as
/// JSON-RPC errors, which are reserved for malformed requests.
pub async fn call_tool(state: &Arc<AppState>, name: &str, args: &Value) -> Value {
    let outcome: Result<(String, Option<Value>), String> = match name {
        "digest" => match ops::parse_window(arg_str(args, "window")) {
            Err(e) => Err(e.to_string()),
            Ok(window) => match ops::digest(state, window).await {
                Err(e) => Err(e.to_string()),
                Ok(digest) => {
                    let structured = json!(digest);
                    let text = if arg_str(args, "format") == Some("json") {
                        pretty(&structured)
                    } else {
                        ops::render_markdown(&digest)
                    };
                    Ok((text, Some(structured)))
                }
            },
        },
        "template_metrics" => match ops::parse_window(arg_str(args, "window")) {
            Err(e) => Err(e.to_string()),
            Ok(window) => ops::template_metrics(
                state,
                window,
                arg_str(args, "bucket").unwrap_or("1d"),
                arg_str(args, "project_id"),
            )
            .await
            .map(|rows| {
                let v = json!({ "rows": rows });
                (pretty(&v), Some(v))
            })
            .map_err(|e| e.to_string()),
        },
        "list_jobs" => {
            let filter: ops::JobFilter = match serde_json::from_value(args.clone()) {
                Ok(f) => f,
                Err(e) => return text_result(format!("invalid arguments: {e}"), None, true),
            };
            ops::list_jobs(state, &filter)
                .await
                .map(|jobs| {
                    let v = json!({ "jobs": jobs, "count": jobs.len() });
                    (pretty(&v), Some(v))
                })
                .map_err(|e| e.to_string())
        }
        "get_job" => match arg_uuid(args, "id") {
            Err(e) => Err(e),
            Ok(id) => ops::get_job(state, id)
                .await
                .map(|v| (pretty(&v), Some(v)))
                .map_err(|e| e.to_string()),
        },
        "retry_job" => match arg_uuid(args, "id") {
            Err(e) => Err(e),
            Ok(id) => ops::retry_job(state, id, None)
                .await
                .map(|v| (format!("Job {} re-queued.", id), Some(v)))
                .map_err(|e| e.to_string()),
        },
        "cancel_job" => match arg_uuid(args, "id") {
            Err(e) => Err(e),
            Ok(id) => ops::cancel_job(state, id, None)
                .await
                .map(|v| (format!("Job {} cancelled.", id), Some(v)))
                .map_err(|e| e.to_string()),
        },
        "list_projects" => ops::list_projects(state)
            .await
            .map(|p| {
                let v = json!({ "projects": p });
                (pretty(&v), Some(v))
            })
            .map_err(|e| e.to_string()),
        "update_project" => match arg_str(args, "id") {
            None => Err("id is required".to_string()),
            Some(id) => {
                let patch: ops::ProjectPatch = match serde_json::from_value(args.clone()) {
                    Ok(p) => p,
                    Err(e) => return text_result(format!("invalid arguments: {e}"), None, true),
                };
                ops::update_project(state, id, &patch)
                    .await
                    .map(|v| (format!("Project {id} updated.\n{}", pretty(&v)), Some(v)))
                    .map_err(|e| e.to_string())
            }
        },
        "list_suppressions" => ops::list_suppressions(
            state,
            arg_str(args, "project_id"),
            args.get("limit").and_then(Value::as_i64).unwrap_or(100),
        )
        .await
        .map(|s| {
            let v = json!({ "suppressions": s, "count": s.len() });
            (pretty(&v), Some(v))
        })
        .map_err(|e| e.to_string()),
        "add_suppression" => match (arg_str(args, "project_id"), arg_str(args, "email")) {
            (Some(project), Some(email)) => {
                match ops::SuppressionScope::parse(arg_str(args, "scope")) {
                    Err(e) => Err(e.to_string()),
                    Ok(scope) => ops::add_suppression(
                        state,
                        project,
                        email,
                        arg_str(args, "detail"),
                        "mcp",
                        scope,
                    )
                    .await
                    .map(|v| ("Address suppressed.".to_string(), Some(v)))
                    .map_err(|e| e.to_string()),
                }
            }
            _ => Err("project_id and email are required".to_string()),
        },
        "release_suppression" => match arg_uuid(args, "id") {
            Err(e) => Err(e),
            Ok(id) => ops::release_suppression(state, id, None, "mcp")
                .await
                .map(|v| ("Suppression released.".to_string(), Some(v)))
                .map_err(|e| e.to_string()),
        },
        "send_test" => match (
            arg_str(args, "project_id"),
            arg_str(args, "channel"),
            arg_str(args, "to"),
            arg_str(args, "body"),
        ) {
            (Some(project), Some(channel), Some(to), Some(body)) => {
                ops::enqueue_test(state, project, channel, to, arg_str(args, "subject"), body)
                    .await
                    .map(|v| {
                        (
                            format!(
                                "Test job {} queued on {channel}; follow it with get_job.",
                                v["id"].as_str().unwrap_or_default()
                            ),
                            Some(v),
                        )
                    })
                    .map_err(|e| e.to_string())
            }
            _ => Err("project_id, channel, to and body are required".to_string()),
        },
        other => Err(format!("unknown tool {other}")),
    };
    match outcome {
        Ok((text, structured)) => text_result(text, structured, false),
        Err(message) => text_result(message, None, true),
    }
}

/// Which revision a message speaks. Modern requests carry the protocol
/// version in `params._meta`; `server/discover` only exists in the modern
/// protocol. Everything else is legacy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Era {
    Modern,
    Legacy,
}

pub fn detect_era(message: &Value) -> Era {
    let has_meta_version = message
        .pointer(&format!(
            "/params/_meta/{}",
            META_PROTOCOL_VERSION.replace('/', "~1")
        ))
        .is_some();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    if has_meta_version || method == "server/discover" {
        Era::Modern
    } else {
        Era::Legacy
    }
}

/// HTTP status + JSON-RPC error for a rejected request.
#[derive(Debug)]
pub struct Rejected(pub StatusCode, pub Value);

/// Validate a modern request against its transport headers (SEP-2243) and
/// `_meta` (SEP-2575). Returns the negotiated protocol version.
pub fn validate_modern(headers: &HeaderMap, message: &Value) -> Result<String, Rejected> {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let meta = message
        .pointer("/params/_meta")
        .cloned()
        .unwrap_or(Value::Null);

    let version = meta
        .get(META_PROTOCOL_VERSION)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if version.is_empty() {
        return Err(Rejected(
            StatusCode::BAD_REQUEST,
            rpc_error(
                id,
                -32602,
                format!("params._meta.{META_PROTOCOL_VERSION} is required"),
            ),
        ));
    }
    if !SUPPORTED_VERSIONS.contains(&version.as_str()) {
        return Err(Rejected(
            StatusCode::BAD_REQUEST,
            json!({
                "jsonrpc": "2.0", "id": id,
                "error": {
                    "code": -32022,
                    "message": format!("unsupported protocol version {version}"),
                    "data": { "supported": SUPPORTED_VERSIONS, "requested": version }
                }
            }),
        ));
    }
    if meta.get(META_CLIENT_CAPABILITIES).is_none() {
        return Err(Rejected(
            StatusCode::BAD_REQUEST,
            rpc_error(
                id,
                -32602,
                format!("params._meta.{META_CLIENT_CAPABILITIES} is required"),
            ),
        ));
    }
    if let Some(header_version) = header_str(headers, "mcp-protocol-version") {
        if header_version != version {
            return Err(Rejected(
                StatusCode::BAD_REQUEST,
                rpc_error(
                    id,
                    -32020,
                    "MCP-Protocol-Version header does not match params._meta",
                ),
            ));
        }
    }
    match header_str(headers, "mcp-method") {
        Some(h) if h == method => {}
        Some(_) => {
            return Err(Rejected(
                StatusCode::BAD_REQUEST,
                rpc_error(
                    id,
                    -32020,
                    "Mcp-Method header does not match the request method",
                ),
            ))
        }
        None => {
            return Err(Rejected(
                StatusCode::BAD_REQUEST,
                rpc_error(id, -32020, "Mcp-Method header is required"),
            ))
        }
    }
    if method == "tools/call" {
        let name = message
            .pointer("/params/name")
            .and_then(Value::as_str)
            .unwrap_or("");
        match header_str(headers, "mcp-name") {
            Some(h) if h == name => {}
            _ => {
                return Err(Rejected(
                    StatusCode::BAD_REQUEST,
                    rpc_error(
                        id,
                        -32020,
                        "Mcp-Name header must equal params.name on tools/call",
                    ),
                ))
            }
        }
    }
    Ok(version)
}

/// Header value, decoding the `=?base64?…?=` sentinel the transport uses for
/// values that are not plain ASCII (SEP-2243).
fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(name)?.to_str().ok()?.trim();
    if let Some(inner) = raw
        .strip_prefix("=?base64?")
        .and_then(|r| r.strip_suffix("?="))
    {
        use base64::Engine;
        return base64::engine::general_purpose::STANDARD
            .decode(inner)
            .ok()
            .and_then(|b| String::from_utf8(b).ok());
    }
    Some(raw.to_string())
}

fn with_modern_envelope(mut result: Value) -> Value {
    if let Some(obj) = result.as_object_mut() {
        obj.entry("resultType").or_insert(json!("complete"));
        obj.insert("_meta".into(), json!({ META_SERVER_INFO: server_info() }));
    }
    result
}

fn cacheable(mut result: Value) -> Value {
    if let Some(obj) = result.as_object_mut() {
        obj.insert("ttlMs".into(), json!(LIST_TTL_MS));
        obj.insert("cacheScope".into(), json!("public"));
    }
    result
}

/// Dispatch one JSON-RPC message. `None` for notifications (no response).
/// Modern requests must already have passed `validate_modern`.
pub async fn handle_message(
    state: &Arc<AppState>,
    message: &Value,
    era: Era,
    operator: Operator,
) -> Option<Value> {
    let id = message.get("id").cloned()?; // notifications and client responses: nothing to answer
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(rpc_error(id, -32600, "jsonrpc must be \"2.0\""));
    }

    let envelope = |result: Value| match era {
        Era::Modern => with_modern_envelope(result),
        Era::Legacy => result,
    };

    Some(match (method, era) {
        ("server/discover", Era::Modern) => rpc_result(
            id,
            envelope(cacheable(json!({
                "supportedVersions": SUPPORTED_VERSIONS,
                "capabilities": capabilities(),
                "instructions": INSTRUCTIONS,
            }))),
        ),
        ("initialize", Era::Legacy) => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(LEGACY_DEFAULT_VERSION);
            // A legacy handshake never negotiates the stateless revision.
            let version = if SUPPORTED_VERSIONS.contains(&requested) && requested != CURRENT_VERSION
            {
                requested
            } else {
                LEGACY_DEFAULT_VERSION
            };
            rpc_result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": capabilities(),
                    "serverInfo": server_info(),
                    "instructions": INSTRUCTIONS,
                }),
            )
        }
        ("ping", Era::Legacy) => rpc_result(id, json!({})),
        ("tools/list", _) => {
            let mut result = cacheable(json!({ "tools": tools_for(operator) }));
            if operator == Operator::ReadOnly {
                // The list depends on the key: not shareable across principals.
                result["cacheScope"] = json!("private");
            }
            rpc_result(id, envelope(result))
        }
        ("tools/call", _) => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if name.is_empty() {
                rpc_error(id, -32602, "params.name is required")
            } else {
                let started = std::time::Instant::now();
                let result = if tool_allowed(operator, name) {
                    call_tool(state, name, &args).await
                } else if operator == Operator::ReadOnly
                    && tools()
                        .as_array()
                        .map(|t| t.iter().any(|t| t["name"] == name))
                        .unwrap_or(false)
                {
                    text_result(
                        format!("`{name}` changes state; this key is read-only. Ask an operator with the admin key."),
                        None,
                        true,
                    )
                } else {
                    call_tool(state, name, &args).await
                };
                audit_tool_call(state, operator, name, &args, &result, started.elapsed());
                rpc_result(id, envelope(result))
            }
        }
        ("resources/list", _) => rpc_result(id, envelope(cacheable(json!({ "resources": [] })))),
        ("prompts/list", _) => rpc_result(id, envelope(cacheable(json!({ "prompts": [] })))),
        _ => rpc_error(id, -32601, format!("method not found: {method}")),
    })
}

/// Every tool call lands in the audit log: name, argument keys (never the
/// values, which may hold addresses), outcome and latency.
fn audit_tool_call(
    state: &Arc<AppState>,
    operator: Operator,
    name: &str,
    args: &Value,
    result: &Value,
    took: std::time::Duration,
) {
    let pool = state.pool.clone();
    let action = format!("mcp.{name}");
    let actor = match operator {
        Operator::Admin => "mcp",
        Operator::ReadOnly => "mcp:readonly",
    };
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let arg_keys: Vec<String> = args
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let detail = json!({ "args": arg_keys, "is_error": is_error, "ms": took.as_millis() as u64 })
        .to_string();
    tokio::spawn(async move {
        crate::middleware::audit(&pool, "admin", actor, &action, Some(&detail), None).await;
    });
}

/// `Origin` present → must be one of `CORS_ORIGINS`, else 403 (DNS
/// rebinding defence, MUST in the transport spec). No header (CLI, SDK,
/// server-side agent) is fine.
fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) else {
        return true;
    };
    std::env::var("CORS_ORIGINS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|allowed| allowed.eq_ignore_ascii_case(origin.trim_end_matches('/')))
}

/// POST /mcp
pub async fn post(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !origin_allowed(&headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(rpc_error(Value::Null, -32600, "Origin not allowed")),
        )
            .into_response();
    }
    let operator =
        match crate::api::projects::operator(&headers) {
            Ok(op) => op,
            Err(_) => return (
                StatusCode::UNAUTHORIZED,
                Json(
                    json!({ "error": "admin or read-only key required (Authorization: Bearer …)" }),
                ),
            )
                .into_response(),
        };
    if !state
        .rate_limiter
        .check("mcp:admin", TOOL_CALLS_PER_MIN)
        .await
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(rpc_error(
                Value::Null,
                -32000,
                "rate limit exceeded, retry in a minute",
            )),
        )
            .into_response();
    }
    let message: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(rpc_error(Value::Null, -32700, format!("parse error: {e}"))),
            )
                .into_response()
        }
    };
    if message.is_array() {
        // Batching left the protocol in 2025-06-18.
        return (
            StatusCode::BAD_REQUEST,
            Json(rpc_error(
                Value::Null,
                -32600,
                "one JSON-RPC message per request; batches are not supported",
            )),
        )
            .into_response();
    }
    if !message.is_object() {
        return (
            StatusCode::BAD_REQUEST,
            Json(rpc_error(Value::Null, -32600, "expected a JSON-RPC object")),
        )
            .into_response();
    }

    let era = detect_era(&message);
    if era == Era::Modern {
        if let Err(Rejected(status, error)) = validate_modern(&headers, &message) {
            return (status, Json(error)).into_response();
        }
        // Modern: unknown methods are 404 (+ -32601 in the body).
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let known = matches!(
            method,
            "server/discover" | "tools/list" | "tools/call" | "resources/list" | "prompts/list"
        );
        if !known && message.get("id").is_some() {
            return (
                StatusCode::NOT_FOUND,
                Json(rpc_error(
                    message["id"].clone(),
                    -32601,
                    format!("method not found: {method}"),
                )),
            )
                .into_response();
        }
    }

    match handle_message(&state, &message, era, operator).await {
        Some(response) => {
            let mut res = Json(response).into_response();
            if era == Era::Modern {
                res.headers_mut().insert(
                    "mcp-protocol-version",
                    message
                        .pointer(&format!(
                            "/params/_meta/{}",
                            META_PROTOCOL_VERSION.replace('/', "~1")
                        ))
                        .and_then(Value::as_str)
                        .unwrap_or(CURRENT_VERSION)
                        .parse()
                        .expect("header value"),
                );
            }
            res
        }
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// GET /mcp — no server-initiated stream on this transport (modern clients
/// use `subscriptions/listen`, which this server does not offer).
pub async fn get() -> Response {
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_schema_and_a_description() {
        let list = tools();
        let items = list.as_array().unwrap();
        assert!(items.len() >= 10);
        for tool in items {
            assert!(tool["name"].as_str().unwrap().len() > 2);
            assert!(tool["description"].as_str().unwrap().len() > 20);
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn uuid_arguments_are_validated() {
        assert!(arg_uuid(&json!({"id": "nope"}), "id").is_err());
        assert!(arg_uuid(&json!({}), "id").is_err());
        assert!(arg_uuid(&json!({"id": "8d92aa5c-b400-492c-8a5e-1ac7eebc3847"}), "id").is_ok());
    }

    #[test]
    fn tool_errors_are_results_not_rpc_errors() {
        let r = text_result("boom".into(), None, true);
        assert_eq!(r["isError"], true);
        assert_eq!(r["content"][0]["text"], "boom");
    }

    #[test]
    fn every_tool_declares_annotations_and_output_schema() {
        for tool in tools().as_array().unwrap() {
            let a = &tool["annotations"];
            assert!(a["readOnlyHint"].is_boolean(), "{}", tool["name"]);
            assert!(a["destructiveHint"].is_boolean(), "{}", tool["name"]);
            assert!(a["idempotentHint"].is_boolean(), "{}", tool["name"]);
            assert!(a["openWorldHint"].is_boolean(), "{}", tool["name"]);
            assert_eq!(tool["outputSchema"]["type"], "object", "{}", tool["name"]);
        }
        let list = tools();
        let names: Vec<&str> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(names.len(), unique.len(), "tool names must be unique");
    }

    #[test]
    fn read_only_operator_sees_only_read_only_tools() {
        let all = tools().as_array().unwrap().len();
        let ro = tools_for(Operator::ReadOnly);
        let ro_names: Vec<&str> = ro
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(ro_names.len() < all);
        for n in [
            "digest",
            "list_jobs",
            "get_job",
            "list_projects",
            "list_suppressions",
            "template_metrics",
        ] {
            assert!(ro_names.contains(&n), "{n} should be read-only");
        }
        for n in [
            "retry_job",
            "cancel_job",
            "update_project",
            "add_suppression",
            "release_suppression",
            "send_test",
        ] {
            assert!(!ro_names.contains(&n), "{n} must not be read-only");
            assert!(!tool_allowed(Operator::ReadOnly, n));
            assert!(tool_allowed(Operator::Admin, n));
        }
    }

    #[test]
    fn era_detection() {
        assert_eq!(
            detect_era(
                &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})
            ),
            Era::Legacy
        );
        assert_eq!(
            detect_era(
                &json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}})
            ),
            Era::Modern
        );
        assert_eq!(
            detect_era(&json!({"jsonrpc":"2.0","id":1,"method":"server/discover","params":{}})),
            Era::Modern
        );
    }

    fn modern_headers(method: &str, name: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("mcp-method", method.parse().unwrap());
        h.insert("mcp-protocol-version", "2026-07-28".parse().unwrap());
        if let Some(n) = name {
            h.insert("mcp-name", n.parse().unwrap());
        }
        h
    }

    fn modern_message(method: &str, params: Value) -> Value {
        let mut p = params;
        p["_meta"] = json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {},
        });
        json!({"jsonrpc":"2.0","id":7,"method":method,"params":p})
    }

    #[test]
    fn modern_validation_accepts_well_formed_requests() {
        let m = modern_message("tools/call", json!({"name":"digest","arguments":{}}));
        assert_eq!(
            validate_modern(&modern_headers("tools/call", Some("digest")), &m).unwrap(),
            "2026-07-28"
        );
    }

    #[test]
    fn modern_validation_rejects_header_mismatch_and_bad_versions() {
        let m = modern_message("tools/call", json!({"name":"digest","arguments":{}}));
        let Rejected(status, err) =
            validate_modern(&modern_headers("tools/list", Some("digest")), &m).unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(err["error"]["code"], -32020);
        let Rejected(_, err) =
            validate_modern(&modern_headers("tools/call", Some("other")), &m).unwrap_err();
        assert_eq!(err["error"]["code"], -32020);
        let mut old = m.clone();
        old["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("1999-01-01");
        let mut h = modern_headers("tools/call", Some("digest"));
        h.insert("mcp-protocol-version", "1999-01-01".parse().unwrap());
        let Rejected(_, err) = validate_modern(&h, &old).unwrap_err();
        assert_eq!(err["error"]["code"], -32022);
        assert!(err["error"]["data"]["supported"].as_array().unwrap().len() >= 3);
        let mut no_caps = m.clone();
        no_caps["params"]["_meta"]
            .as_object_mut()
            .unwrap()
            .remove("io.modelcontextprotocol/clientCapabilities");
        let Rejected(_, err) =
            validate_modern(&modern_headers("tools/call", Some("digest")), &no_caps).unwrap_err();
        assert_eq!(err["error"]["code"], -32602);
    }

    #[test]
    fn base64_header_sentinel_is_decoded() {
        let mut h = HeaderMap::new();
        h.insert("mcp-name", "=?base64?ZGlnZXN0?=".parse().unwrap());
        assert_eq!(header_str(&h, "mcp-name").as_deref(), Some("digest"));
    }

    #[test]
    fn modern_envelope_adds_result_type_and_server_info() {
        let r = with_modern_envelope(cacheable(json!({"tools": []})));
        assert_eq!(r["resultType"], "complete");
        assert_eq!(
            r["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "notifyd"
        );
        assert_eq!(r["ttlMs"], 300000);
        assert_eq!(r["cacheScope"], "public");
    }
}
