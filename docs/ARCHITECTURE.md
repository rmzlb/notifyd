# Architecture

Deep dive into notifyd internals — how the queue works, how SSE broadcasts, and why we made these choices.

---

## High-Level Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                          notifyd                                 │
│                                                                  │
│  ┌──────────────┐    ┌────────────┐    ┌──────────────────────┐ │
│  │   REST API   │───→│   Queue    │───→│     Connectors       │ │
│  │  (Axum 0.7)  │    │ (Postgres) │    │                      │ │
│  └──────────────┘    └────────────┘    │  ┌──────────────┐    │ │
│         │                    │         │  │ Email (Resend)│    │ │
│         │                    │         │  └──────────────┘    │ │
│  ┌──────────────┐    ┌──────────────┐  │  ┌──────────────┐    │ │
│  │ SSE Broadcast│◄──→│   Worker    │  │  │ SMS (Twilio)  │    │ │
│  │(tokio channel)│   │ (tokio loop)│  │  └──────────────┘    │ │
│  └──────────────┘    └──────────────┘  │  ┌──────────────┐    │ │
│                                        │  │ Push (FCM)    │    │ │
│                                        │  └──────────────┘    │ │
│                                        │  ┌──────────────┐    │ │
│                                        │  │ In-App (DB)   │    │ │
│                                        │  └──────────────┘    │ │
│                                        └──────────────────────┘ │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                   PostgreSQL 16                           │   │
│  │  jobs │ subscribers │ inbox_messages │ templates │ ...    │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

---

## The Queue

### Why Postgres Instead of Redis?

Redis + BullMQ is the standard for job queues. But for a notification service doing ~1000 sends/minute (which covers 99% of self-hosted use cases), Postgres is more than fast enough — and you already have it.

**`SELECT FOR UPDATE SKIP LOCKED`** is the key. It's a Postgres feature that:
- Atomically claims rows for processing
- Skips rows already claimed by another worker
- Works like a distributed lock without a separate lock service

```sql
SELECT * FROM jobs
WHERE status IN ('pending', 'retry')
  AND scheduled_at <= now()
ORDER BY scheduled_at ASC
LIMIT 50
FOR UPDATE SKIP LOCKED
```

This gives us:
- **At-most-once processing** per poll cycle
- **Horizontal scaling** — run multiple workers, they won't double-process
- **Scheduling** — `scheduled_at` is just a WHERE clause
- **Persistence** — jobs survive restarts (Redis can't guarantee this without AOF)

### Retry Strategy

Failed jobs get exponential backoff:

| Attempt | Delay | Total elapsed |
|---------|-------|---------------|
| 1 | 30 seconds | 30s |
| 2 | 2 minutes | 2m 30s |
| 3 | 10 minutes | 12m 30s |

After `max_attempts` (default 3), the job moves to `failed` status. Check via `GET /v1/jobs/:id`.

### Job Lifecycle

```
pending → processing → sent ✓
                     → retry → processing → sent ✓
                                           → retry → ... → failed ✗
```

Cancelled via `DELETE /v1/jobs/:id`:
```
pending → cancelled ✓
scheduled → cancelled ✓
processing → (cannot cancel, already being sent)
```

---

## SSE Broadcasting

### Why SSE Over WebSocket?

| | SSE | WebSocket |
|---|---|---|
| Direction | Server → Client | Bidirectional |
| Complexity | `EventSource` (5 lines) | ws library + reconnection logic |
| Proxy support | Works through all proxies | Some proxies break ws |
| Auto-reconnect | Built-in | Manual |
| Use case fit | Notifications (one-way) | Chat (two-way) |

Notifications are inherently one-way: server pushes to client. SSE is the simpler, more reliable choice.

### How It Works

```
                  ┌─────────────────────┐
                  │   SseBroadcaster    │
                  │                     │
  Worker ────────→│  HashMap<key, tx>   │
  (sends event)   │                     │
                  │  key = "project:sub"│
                  │  tx = broadcast::Tx │
                  └─────┬──────┬────────┘
                        │      │
                   ┌────▼┐  ┌──▼───┐
                   │ rx1 │  │ rx2  │  (multiple tabs/devices)
                   │(SSE)│  │(SSE) │
                   └─────┘  └──────┘
```

1. Client connects: `GET /v1/inbox/:sub_id/stream?token=xxx`
2. `SseBroadcaster` creates a `tokio::sync::broadcast` channel for this subscriber
3. When a notification is sent to this subscriber, the in-app connector calls `broadcaster.send()`
4. All connected clients receive the event instantly

**Cleanup**: channels with no receivers are cleaned up periodically (every 2 minutes).

### SSE Auth: One-Time Tickets

Putting a JWT in the URL (query param) is a security concern — it shows up in server logs, browser history, and referrer headers.

notifyd solves this with **one-time tickets**:

```
1. POST /v1/inbox/:sub_id/stream-ticket
   → {"ticket": "abc-123"}  (valid 60 seconds)

2. GET /v1/inbox/:sub_id/stream?ticket=abc-123
   → Ticket consumed, stream starts
   → Same ticket can't be reused
```

---

## Connectors

Each connector implements a simple trait:

```rust
#[async_trait]
pub trait Connector: Send + Sync {
    async fn send(&self, request: SendRequest) -> Result<()>;
}
```

`SendRequest` contains:
- `to`: recipient (email, phone number, subscriber ID)
- `subject`: optional (email)
- `body`: rendered body (template vars already substituted)
- `payload`: raw JSON for connector-specific fields

### Email: Resend

Simple HTTP POST to Resend API. Supports HTML (`body_html`) and plain text (`body`).

### SMS: Twilio / Telnyx

Swappable via config — change `provider = "twilio"` to `provider = "telnyx"` and it works. Same interface, different HTTP calls internally.

### Push: FCM

Firebase Cloud Messaging. Requires push token registration via `POST /v1/push-tokens`.

### In-App

No external service. Writes to `inbox_messages` table and broadcasts via `SseBroadcaster`.

---

## Workflow Engine

Lightweight event-driven workflows. Not Temporal — just what you need for notification sequences.

### Workflow Definition

```json
{
  "id": "welcome-series",
  "trigger_event": "user.signup",
  "steps": [
    {"type": "send", "channel": "email", "template": "welcome"},
    {"type": "delay", "duration": "24h"},
    {"type": "send", "channel": "email", "template": "getting_started"},
    {"type": "delay", "duration": "72h"},
    {"type": "condition", "check": "has_completed_onboarding", "if_false": [
      {"type": "send", "channel": "email", "template": "nudge"}
    ]}
  ]
}
```

### Execution

1. Event arrives (`POST /v1/workflows/trigger`)
2. Engine finds matching workflows
3. Creates a `workflow_run` record
4. Executes steps sequentially
5. On `delay` steps: saves state, marks run as `paused`
6. Worker resumes paused runs when delay expires

State is persisted in Postgres — survives restarts. No in-memory state to lose.

---

## Multi-Tenancy

notifyd is designed for running one instance across multiple projects (apps).

### Isolation

- Every table has `project_id` as part of the primary key or foreign key
- API keys are project-scoped
- Subscribers, templates, inbox — all isolated by project
- A subscriber `user-1` in project `square` is different from `user-1` in project `clozup`

### Key Rotation

Zero-downtime key rotation:

1. `POST /v1/admin/projects/:id/rotate-key` — generates new key, old key still valid (24h grace)
2. Update your app to use the new key
3. `POST /v1/admin/projects/:id/revoke-secondary` — revoke old key

---

## Security

### PII Masking

The `pii.rs` module masks sensitive data in logs:
- Email: `u***@example.com`
- Phone: `+336***678`
- Names: first 2 chars + `***`

### Audit Log

Every API mutation is logged to `audit_log`:
- Who (API key / project)
- What (action)
- When (timestamp)
- From where (IP, request ID)

### Rate Limiting

In-memory sliding window, per-project. Default 100 req/min. Configurable.

```rust
// Middleware checks before every request
if !rate_limiter.check(&project.id, project.rate_limit).await {
    return Err(StatusCode::TOO_MANY_REQUESTS);
}
```

---

## Performance

### Benchmarks (single instance, 1 vCPU, 1GB RAM)

| Metric | Value |
|--------|-------|
| Send throughput | ~500 notifications/second |
| Queue latency (enqueue→process) | <100ms |
| SSE connection overhead | ~2KB per connection |
| Memory footprint | ~15MB idle, ~50MB under load |
| Cold start | <2 seconds |

### Scaling

For most use cases (< 10K notifications/hour), a single instance is plenty.

If you need more:
- Run multiple instances pointing to the same Postgres (queue handles coordination)
- Scale Postgres reads with replicas
- For >100K/hour sustained, consider moving to Redis-backed queue (but you probably don't need this)
