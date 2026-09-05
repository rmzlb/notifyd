//! Model Context Protocol server, Streamable HTTP transport, stateless.
//!
//! `POST /mcp` with `Authorization: Bearer <ADMIN_API_KEY>` (or `x-api-key`).
//! Each request is one JSON-RPC 2.0 message; the response is a single JSON
//! document (no server-initiated stream, so `GET /mcp` answers 405 and
//! `Mcp-Session-Id` is never issued). Notifications are acknowledged with
//! 202 and no body. Methods: `initialize`, `ping`, `tools/list`,
//! `tools/call`. Every tool wraps a function of `ops`, so the MCP surface
//! and the REST admin API cannot drift apart.

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

pub const PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Tool catalogue: name, description, JSON Schema of the arguments.
pub fn tools() -> Value {
    json!([
        {
            "name": "digest",
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
            "name": "list_jobs",
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
            "description": "One job with its attempts, provider, provider message id and delivery events.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}}, "required": ["id"]}
        },
        {
            "name": "retry_job",
            "description": "Re-queue a failed or cancelled job with a fresh attempt budget. Fix the cause first: a permanent error (bad address, unverified sender) will fail again.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}}, "required": ["id"]}
        },
        {
            "name": "cancel_job",
            "description": "Cancel a job that is still pending or waiting for a retry.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}}, "required": ["id"]}
        },
        {
            "name": "list_projects",
            "description": "Projects on this instance with channels, sender identity and inbound rate limit.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "update_project",
            "description": "Change a project's name, channels, sender (from_email/from_name; empty from_email clears it) or inbound rate limit. Keys are never touched.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "name": {"type": "string"},
                    "channels": {"type": "array", "items": {"type": "string", "enum": ["email", "sms", "whatsapp", "in_app", "push"]}},
                    "from_email": {"type": "string"},
                    "from_name": {"type": "string"},
                    "rate_limit_per_min": {"type": "integer", "minimum": 1}
                },
                "required": ["id"]
            }
        },
        {
            "name": "list_suppressions",
            "description": "Active email suppressions (bounced, complained or manually blocked addresses), masked.",
            "inputSchema": {"type": "object", "properties": {"project_id": {"type": "string"}, "limit": {"type": "integer", "minimum": 1, "maximum": 500}}}
        },
        {
            "name": "add_suppression",
            "description": "Block an address for a project: no email will be sent to it until released.",
            "inputSchema": {"type": "object", "properties": {"project_id": {"type": "string"}, "email": {"type": "string"}, "detail": {"type": "string"}}, "required": ["project_id", "email"]}
        },
        {
            "name": "release_suppression",
            "description": "Release a suppression so the address can receive email again.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}}, "required": ["id"]}
        },
        {
            "name": "send_test",
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

fn authorized(headers: &HeaderMap) -> bool {
    crate::api::projects::require_admin(headers).is_ok()
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
                ops::add_suppression(state, project, email, arg_str(args, "detail"), "mcp")
                    .await
                    .map(|v| ("Address suppressed.".to_string(), Some(v)))
                    .map_err(|e| e.to_string())
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

/// Dispatch one JSON-RPC message. `None` for notifications (no response).
pub async fn handle_message(state: &Arc<AppState>, message: &Value) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    // Notifications and client responses carry no id: nothing to answer.
    let Some(id) = id else {
        return None;
    };
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(rpc_error(id, -32600, "jsonrpc must be \"2.0\""));
    }

    Some(match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            let version = if SUPPORTED_VERSIONS.contains(&requested) {
                requested
            } else {
                PROTOCOL_VERSION
            };
            rpc_result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "notifyd", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "notifyd is a self-hosted notification queue (email, SMS, WhatsApp, in-app, push). Start with `digest` to see the state of this instance; its findings say what to do. Use list_jobs/get_job to investigate, retry_job after fixing a cause, update_project to set a sender, send_test to prove a channel."
                }),
            )
        }
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(id, json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if name.is_empty() {
                rpc_error(id, -32602, "params.name is required")
            } else {
                rpc_result(id, call_tool(state, name, &args).await)
            }
        }
        "resources/list" => rpc_result(id, json!({ "resources": [] })),
        "prompts/list" => rpc_result(id, json!({ "prompts": [] })),
        _ => rpc_error(id, -32601, format!("method not found: {method}")),
    })
}

/// POST /mcp
pub async fn post(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !authorized(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "admin key required (Authorization: Bearer …)" })),
        )
            .into_response();
    }
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(rpc_error(Value::Null, -32700, format!("parse error: {e}"))),
            )
                .into_response()
        }
    };
    match parsed {
        Value::Array(messages) => {
            let mut responses = Vec::new();
            for message in &messages {
                if let Some(r) = handle_message(&state, message).await {
                    responses.push(r);
                }
            }
            if responses.is_empty() {
                StatusCode::ACCEPTED.into_response()
            } else {
                Json(Value::Array(responses)).into_response()
            }
        }
        message => match handle_message(&state, &message).await {
            Some(response) => Json(response).into_response(),
            None => StatusCode::ACCEPTED.into_response(),
        },
    }
}

/// GET /mcp — no server-initiated stream on this transport.
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
}
