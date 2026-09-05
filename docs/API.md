# notifyd API Reference

**Base URL:** `http://localhost:3400` (dev) or your deployed domain.

All endpoints require `X-Api-Key: sk_<project>_xxx` header unless noted otherwise.

---

## Quick Start

Send your first notification in 30 seconds:

```bash
curl -X POST http://localhost:3400/v1/send \
  -H "X-Api-Key: sk_myapp_xxx" \
  -H "Content-Type: application/json" \
  -d '{
    "channels": ["email"],
    "subscriber_id": "user-1",
    "subject": "Hello!",
    "body": "Welcome to the app."
  }'
```

---

## Authentication

### API Key (backend-to-backend)

```
X-Api-Key: sk_myapp_xxxxxxxxxxxxxxxxxxxxx
```

Defined per project in `notifyd.toml`. Identifies the project and scopes all data.

### Subscriber JWT (frontend)

For inbox/SSE endpoints, generate a short-lived token:

```bash
curl -X POST http://localhost:3400/v1/auth/subscriber-token \
  -H "X-Api-Key: sk_myapp_xxx" \
  -d '{"subscriber_id": "user-1"}'
# → {"token": "eyJ..."}
```

Then use it:

```
Authorization: Bearer eyJ...
```

### Admin Key

Some endpoints (`/v1/metrics`, `/v1/admin/*`) require the admin API key defined in config.

---

## Response Format

All responses are JSON.

### Success

```json
{"success": true, "id": "uuid", ...}
```

### Error

```json
{"error": "Description of what went wrong"}
```

| Status | When |
|--------|------|
| 200 | Success |
| 201 | Created |
| 400 | Invalid request body |
| 401 | Missing or invalid API key |
| 403 | Forbidden (wrong subscriber, wrong project) |
| 404 | Resource not found |
| 429 | Rate limited |
| 500 | Internal server error |

---

## Endpoints

### Summary

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| **POST** | `/v1/send` | API Key | Send notification (immediate or scheduled) |
| **POST** | `/v1/batch` | API Key | Send to multiple subscribers |
| **GET** | `/v1/jobs/:id` | API Key | Get job status |
| **DELETE** | `/v1/jobs/:id` | API Key | Cancel pending/scheduled job |
| **POST** | `/v1/subscribers` | API Key | Create or update subscriber |
| **GET** | `/v1/subscribers` | API Key | List subscribers |
| **GET** | `/v1/subscribers/:id` | API Key | Get subscriber |
| **DELETE** | `/v1/subscribers/:id` | API Key | Delete subscriber |
| **GET** | `/v1/subscribers/:id/preferences` | API Key | Get notification preferences |
| **PUT** | `/v1/subscribers/:id/preferences` | API Key | Set notification preferences |
| **GET** | `/v1/inbox/:sub_id` | JWT or API Key | List in-app notifications |
| **PATCH** | `/v1/inbox/:sub_id/:msg_id` | JWT or API Key | Update notification (read/archive/todo) |
| **POST** | `/v1/inbox/:sub_id/read-all` | JWT or API Key | Mark all as read |
| **GET** | `/v1/inbox/:sub_id/unread-count` | JWT or API Key | Unread badge count |
| **GET** | `/v1/inbox/:sub_id/stream` | JWT (query) | SSE realtime stream |
| **POST** | `/v1/inbox/:sub_id/stream-ticket` | JWT or API Key | One-time SSE auth ticket |
| **POST** | `/v1/auth/subscriber-token` | API Key | Generate subscriber JWT |
| **POST** | `/v1/workflows` | API Key | Create/update workflow |
| **GET** | `/v1/workflows` | API Key | List workflows |
| **GET** | `/v1/workflows/:id` | API Key | Get workflow |
| **DELETE** | `/v1/workflows/:id` | API Key | Delete workflow |
| **POST** | `/v1/workflows/trigger` | API Key | Trigger workflow event |
| **GET** | `/v1/workflows/runs` | API Key | List workflow runs |
| **DELETE** | `/v1/workflows/runs/:id` | API Key | Cancel workflow run |
| **POST** | `/v1/templates` | API Key | Create/update template |
| **GET** | `/v1/templates` | API Key | List templates |
| **GET** | `/v1/templates/:id` | API Key | Get template |
| **DELETE** | `/v1/templates/:id` | API Key | Delete template |
| **POST** | `/v1/push-tokens` | API Key | Register push token |
| **GET** | `/v1/push-tokens/subscriber/:id` | API Key | List push tokens |
| **DELETE** | `/v1/push-tokens/:id` | API Key | Delete push token |
| **GET** | `/v1/health` | None | Health check |
| **GET** | `/v1/metrics` | Admin | Service metrics |
| **POST** | `/v1/admin/projects` | Admin | Create project |
| **GET** | `/v1/admin/projects` | Admin | List projects |
| **POST** | `/v1/admin/projects/:id/rotate-key` | Admin | Rotate API key |
| **POST** | `/v1/admin/projects/:id/revoke-secondary` | Admin | Revoke old key |
| **DELETE** | `/v1/admin/projects/:id` | Admin | Delete project |
| **GET** | `/v1/admin/audit` | Admin | Audit log |
| **POST** | `/v1/admin/webhooks` | Admin | Create webhook |
| **GET** | `/v1/admin/webhooks` | Admin | List webhooks |
| **DELETE** | `/v1/admin/webhooks/:id` | Admin | Delete webhook |

---

### POST /v1/send

Send a notification via one or more channels. Jobs are queued and processed asynchronously (<1s typical).

**Request:**

```json
{
  "channels": ["email", "in_app"],
  "subscriber_id": "user-uuid",
  "to": "user@example.com",
  "cc": ["orders@example.com"],
  "reply_to": "buyer@example.com",
  "subject": "Your order shipped",
  "body": "Hey {{first_name}}, order #{{order_id}} is on its way!",
  "vars": {
    "first_name": "Alice",
    "order_id": "ORD-42"
  },
  "scheduled_at": "2026-03-25T14:00:00Z",
  "idempotency_key": "order-42-shipped"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `channels` | `string[]` | ✅ (or `channel`) | `"email"`, `"sms"`, `"in_app"`, `"push"` |
| `channel` | `string` | ✅ (or `channels`) | Single channel shorthand |
| `subscriber_id` | `string` | ❌ | Links to subscriber record for template vars |
| `to` | `string` | ❌ | Override recipient (email/phone). Falls back to subscriber. |
| `subject` | `string` | ❌ | Email subject |
| `body` | `string` | ❌ | Message body (supports `{{var}}` substitution) |
| `template` | `string` | ❌ | Use a stored template instead of inline body |
| `vars` | `object` | ❌ | Template variables |
| `scheduled_at` | `ISO 8601` | ❌ | Schedule for future delivery (default: now) |
| `idempotency_key` | `string` | ❌ | Dedupes sends: reusing a key held by a live or succeeded job returns that job untouched (no re-send); a `failed`/`cancelled` job releases its key, so retrying after a failure creates a fresh job |
| `attachments` | `object[]` | ❌ | Email only. `[{ "filename", "content" (base64), "content_type"? }]`. Forces single-send (Resend batch rejects attachments). |
| `cc` | `string[]` | ❌ | Email only. Up to 10 carbon-copy recipients; duplicates are removed. |
| `reply_to` | `string` | ❌ | Email only. Address that receives replies. |
| `priority` | `string \| int` | ❌ | Queue lane: `critical` (10), `high` (30), `normal` (50, default), `low` (70), `bulk` (80) or `0–100`. Lower goes first. An email tagged `{"name":"category","value":"campaign"\|"marketing"\|"newsletter"}` defaults to `bulk`. |
| `tags` | `object[]` | ❌ | Email only. Provider tags `[{ "name", "value" }]`; also drives the default priority (see above). |
| `email_headers` | `object` | ❌ | Email only. Custom MIME headers such as `List-Unsubscribe`. |
| `send_window` | `object \| false` | ❌ | `{ "start": "09:00", "end": "20:00", "tz": "Europe/Paris", "days": [1..7], "applies_to": "marketing" \| "all" }`. Bulk email waits for the recipient's daytime (`subscribers.timezone`, else `tz`). Overrides the project's `settings.send_window`; `false` bypasses it. |

**Response:**

```json
{
  "success": true,
  "jobs": [
    {"id": "uuid", "channel": "email", "status": "pending"},
    {"id": "uuid", "channel": "in_app", "status": "pending"}
  ]
}
```

### GET /v1/jobs/:id

Returns both the transport state and the provider evidence. `status: "sent"`
means the provider accepted the API call; `delivered_at` and the per-recipient
`provider_events` are the proof of what happened afterwards.

```json
{
  "id": "uuid",
  "status": "sent",
  "priority": 50,
  "attempts": 1,
  "max_attempts": 5,
  "provider": "resend",
  "provider_message_id": "provider-email-id",
  "sent_at": "2026-08-14T08:16:10Z",
  "delivered_at": "2026-08-14T08:16:14Z",
  "bounced_at": null,
  "email_envelope": {
    "to": ["supplier@example.com"],
    "cc": ["orders@example.com"],
    "reply_to": "orders@example.com"
  },
  "provider_events": [
    {
      "provider": "resend",
      "type": "email.delivered",
      "occurred_at": "2026-08-14T08:16:14Z",
      "provider_message_id": "provider-email-id",
      "recipients": ["user@example.com"],
      "error": null
    }
  ]
}
```

`provider` and `provider_message_id` name the connector that accepted the
message and the provider's own identifier (Resend id, Telnyx message id, SMTP
`Message-ID`).

**Job lifecycle.** `pending` → `processing` → `sent` | `retry` | `failed`.
The worker classifies every provider answer:

| Provider answer | Job | Notes |
|---|---|---|
| accepted | `sent` | `provider_message_id` stored |
| `429`, `5xx`, network — with `EMAIL_FALLBACK_PROVIDER` set | sent through the fallback at once | the primary rests for `EMAIL_FAILOVER_COOLDOWN_SECS`; `provider` on the job names the connector that accepted |
| `429` | `retry`, attempt **not** consumed | the channel lane pauses for `Retry-After` (default `RATE_LIMIT_PAUSE_SECS`) |
| `5xx`, timeout, network | `retry` | backoff 30 s → 2 min → 10 min → 30 min → 2 h (±20 % jitter), `failed` after `max_attempts` (default 5) |
| other `4xx`, invalid recipient, unverified sender, integrity violation | `failed` at once | retrying the same request would give the same answer |
| recipient on the suppression list | `failed` at once | the provider is never contacted |

A job left in `processing` for more than 10 minutes (worker crash, OOM, hard
restart) is re-queued by the reaper with its attempt consumed, so nothing is
lost silently and nothing loops forever.

`email_envelope` is the exact recipient envelope persisted before provider
handoff. It lets clients distinguish a recipient still awaiting an event from
an address that was never included in the accepted send. It is `null` for
non-email jobs.

**curl example:**

```bash
curl -X POST http://localhost:3400/v1/send \
  -H "X-Api-Key: sk_myapp_xxx" \
  -H "Content-Type: application/json" \
  -d '{
    "channel": "email",
    "subscriber_id": "user-1",
    "subject": "Password reset",
    "body": "Click here to reset: {{reset_url}}",
    "vars": {"reset_url": "https://app.example.com/reset?token=abc"}
  }'
```

**TypeScript example:**

```typescript
const response = await fetch('https://notifyd.example.com/v1/send', {
  method: 'POST',
  headers: {
    'X-Api-Key': process.env.NOTIFYD_API_KEY,
    'Content-Type': 'application/json',
  },
  body: JSON.stringify({
    channels: ['email', 'in_app'],
    subscriber_id: userId,
    subject: 'New comment on your post',
    body: '{{commenter}} commented: "{{comment}}"',
    vars: { commenter: 'Bob', comment: 'Great post!' },
  }),
});
```

**Rust example:**

```rust
let client = reqwest::Client::new();
let res = client
    .post("http://localhost:3400/v1/send")
    .header("X-Api-Key", "sk_myapp_xxx")
    .json(&serde_json::json!({
        "channel": "email",
        "subscriber_id": "user-1",
        "subject": "Welcome!",
        "body": "Hello {{first_name}}!",
        "vars": {"first_name": "Alice"}
    }))
    .send()
    .await?;
```

---

### POST /v1/batch

Fan-out to many subscribers. Jobs default to priority `bulk` (80): a campaign
never delays transactional traffic. Pass `"priority"` to override.

`send_window` works as on `/v1/send`; each recipient is scheduled in its own
timezone. `idempotency_key` (optional) dedupes the whole fan-out: the key is declined per
subscriber and channel, so replaying the same call after a timeout creates no
second job for anyone already queued or sent. The response reports
`jobs_created` and `jobs_deduplicated`.

Send the same notification to multiple subscribers.

```bash
curl -X POST http://localhost:3400/v1/batch \
  -H "X-Api-Key: sk_myapp_xxx" \
  -H "Content-Type: application/json" \
  -d '{
    "channels": ["email", "in_app"],
    "subscribers": ["user-1", "user-2", "user-3"],
    "template": "weekly_digest",
    "vars": {"week": "March 24-30"},
    "icon": "calendar",
    "url": "/digest/2026-w13"
  }'
```

`icon` and `url` are optional and are forwarded to in-app notifications for batch sends too.

---

### GET /v1/inbox/:subscriber_id

List in-app notifications for a subscriber.

**Query Parameters:**

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `limit` | `int` | 20 | Max items to return |
| `cursor` | `string` | — | Pagination cursor (ISO timestamp) |
| `unread_only` | `bool` | false | Filter to unread only |

**Response:**

```json
{
  "notifications": [
    {
      "id": "uuid",
      "body": "New comment on your post",
      "icon": "message",
      "url": "/posts/42#comments",
      "read_at": null,
      "is_todo": false,
      "created_at": "2026-03-25T10:30:00Z"
    }
  ],
  "has_more": true,
  "next_cursor": "2026-03-25T10:29:00Z"
}
```

---

### GET /v1/inbox/:subscriber_id/stream

Realtime SSE stream for in-app notifications.

**Auth:** Pass subscriber JWT as query parameter `?token=eyJ...` or via `Authorization: Bearer` header.

**Events:**

```
event: message
data: {"type":"new_notification","notification":{"id":"uuid","body":"...","icon":"bell","created_at":"..."}}

event: message
data: {"type":"count_update","unread_count":5}

event: message
data: {"type":"read","notification_id":"uuid"}

event: message
data: {"type":"archived","notification_id":"uuid"}
```

**JavaScript example:**

```javascript
const events = new EventSource(
  `https://notifyd.example.com/v1/inbox/${userId}/stream?token=${jwt}`
);

events.onmessage = (e) => {
  const data = JSON.parse(e.data);

  switch (data.type) {
    case 'new_notification':
      showToast(data.notification);
      break;
    case 'count_update':
      updateBadge(data.unread_count);
      break;
  }
};
```

> **Tip:** Use `POST /v1/inbox/:sub_id/stream-ticket` to get a one-time ticket instead of passing the JWT in the URL.

---

### POST /v1/workflows/trigger

Trigger all workflows matching an event.

```bash
curl -X POST http://localhost:3400/v1/workflows/trigger \
  -H "X-Api-Key: sk_myapp_xxx" \
  -H "Content-Type: application/json" \
  -d '{
    "event": "order.completed",
    "subscriber_id": "user-42",
    "payload": {
      "order_id": "ORD-42",
      "total": 99.00
    }
  }'
```

---

### GET /v1/health

No authentication required.

```bash
curl http://localhost:3400/v1/health
```

```json
{
  "status": "ok",
  "db": "ok",
  "version": "0.2.0"
}
```

---

### GET /v1/metrics

Requires admin API key (`x-api-key` or `Authorization: Bearer`).

```json
{
  "jobs_pending": 12,
  "jobs_processing": 3,
  "jobs_sent_24h": 1547,
  "jobs_failed_24h": 2,
  "subscribers_total": 8420,
  "inbox_messages_total": 34210,
  "active_workflow_runs": 5,
  "uptime_seconds": 86400
}
```

### GET /v1/metrics/prometheus

Same admin key, Prometheus text exposition (point a Prometheus / Grafana
Agent scrape at it with `bearer_token`). Series:

| Metric | Labels | Meaning |
|---|---|---|
| `notifyd_jobs_outcome_total` | `channel`, `provider`, `outcome` | `sent`, `retry`, `failed`, `rate_limited`, `skipped` |
| `notifyd_provider_errors_total` | `channel`, `provider`, `kind` | `rate_limited`, `transient`, `permanent`, `suppressed` |
| `notifyd_lane_pauses_total` | `channel` | lane pauses after a 429 |
| `notifyd_send_latency_seconds` | `channel`, `provider` | provider call latency histogram |
| `notifyd_jobs_queue_depth` | `status` | `pending`, `retry`, `processing` at scrape time |
| `notifyd_oldest_pending_age_seconds` | `band` | oldest waiting job per priority band (`urgent` <50, `normal`, `bulk` ≥80) |

---

### Subscriber Preferences

Users can opt out of specific channels or workflows.

**Get preferences:**

```bash
curl http://localhost:3400/v1/subscribers/user-1/preferences \
  -H "X-Api-Key: sk_myapp_xxx"
```

**Set preferences:**

```bash
curl -X PUT http://localhost:3400/v1/subscribers/user-1/preferences \
  -H "X-Api-Key: sk_myapp_xxx" \
  -d '{
    "preferences": [
      {"channel": "email", "workflow_id": "marketing", "enabled": false},
      {"channel": "sms", "workflow_id": "*", "enabled": false}
    ]
  }'
```

Hierarchy: workflow-specific > channel-wide > global.

---

### Admin: Operator surface

Everything an operator (or an agent, see `docs/AGENT.md`) needs. Admin key;
the read-only key (`READONLY_API_KEY`) may call the `GET` endpoints of this
table, `/v1/metrics*` and `GET /v1/admin/projects`.

| Endpoint | Purpose |
|---|---|
| `GET /v1/admin/digest?window=24h&format=json\|markdown` | Findings ranked by severity with the action to take, queue, outcomes, failure reasons, retries waiting, latency, deliverability, projects. `window`: 1h, 6h, 24h, 7d, 30d. |
| `GET /v1/admin/metrics/templates?window=7d&bucket=1d&project_id=` | Delivery funnel per template and time bucket: sent, failed, delivered, bounced, complained, opened, clicked. Template = `template_id`, else `template` tag, else `category` tag, else `untemplated`. |
| `GET /v1/admin/jobs?project_id=&status=&channel=&recipient=&since=&limit=` | Search jobs across projects (recipients masked, default last 7 days, 50 rows, max 500). |
| `POST /v1/admin/jobs/:id/retry` | Re-queue a `failed`/`cancelled` job with a fresh attempt budget. |
| `POST /v1/admin/jobs/:id/cancel` | Cancel a `pending`/`retry` job. |
| `PATCH /v1/admin/projects/:id` | `{ name?, channels?, from_email?, from_name?, rate_limit_per_min?, send_window? }` — keys untouched; empty `from_email` clears it; `send_window` (object or `null`) is the project's daily window for bulk email. |
| `GET /v1/admin/suppressions?project_id=&limit=` | Active suppressions, masked. |
| `POST /v1/admin/suppressions` | `{ project_id, email, detail? }` — manual block. |
| `DELETE /v1/admin/suppressions/:id` | Release a suppression. |

Project keys have the scoped equivalents `POST /v1/jobs/:id/retry` and
`POST /v1/suppressions` (`{ email, detail? }`).

### MCP endpoint

`POST /mcp` — Model Context Protocol, Streamable HTTP, stateless, dual-era
(2026-07-28 with `server/discover` and `_meta`, plus the legacy `initialize`
handshake); admin key as `Authorization: Bearer`. `Origin`, when present,
must be in `CORS_ORIGINS` (403). One JSON-RPC message per request (batches
→ 400). Tools with annotations and `outputSchema`: `digest`, `list_jobs`,
`get_job`, `retry_job`, `cancel_job`, `list_projects`, `update_project`,
`list_suppressions`, `add_suppression`, `release_suppression`, `send_test`.
See `docs/AGENT.md`.

### Admin: Projects

**Create a project:**

```bash
curl -X POST http://localhost:3400/v1/admin/projects \
  -H "X-Api-Key: admin_xxx" \
  -d '{"id": "newapp", "name": "New App", "channels": ["email", "in_app"]}'
```

**Rotate API key (zero-downtime):**

```bash
# 1. Rotate — old key still works for 24h
curl -X POST http://localhost:3400/v1/admin/projects/newapp/rotate-key \
  -H "X-Api-Key: admin_xxx"
# → {"new_key": "sk_newapp_yyy", "old_key_expires_at": "..."}

# 2. Update your app with new key

# 3. Revoke old key
curl -X POST http://localhost:3400/v1/admin/projects/newapp/revoke-secondary \
  -H "X-Api-Key: admin_xxx"
```

---

### Admin: Webhooks

Receive POST callbacks when notifications are delivered, failed, or clicked.

```bash
curl -X POST http://localhost:3400/v1/admin/webhooks \
  -H "X-Api-Key: admin_xxx" \
  -d '{
    "url": "https://myapp.com/webhooks/notifyd",
    "events": ["notification.sent", "notification.failed"],
    "secret": "whsec_xxx"
  }'
```

Webhook payloads are signed with HMAC-SHA256. Verify with the `X-Notifyd-Signature` header.

---

## Rate Limits

Default: 100 requests/minute per project. Configurable per project.

When rate limited, you'll receive:

```
HTTP 429 Too Many Requests
{"error": "Rate limit exceeded"}
```

Retry after 60 seconds or contact the admin to increase limits.

---

## SDKs

### Official

- `notifyd-sdk` — TypeScript client shipped from this repo

Install from GitHub:

```bash
pnpm add notifyd-sdk@github:rmzlb/notifyd
```

Usage:

```typescript
import { createNotifydClient } from 'notifyd-sdk';

const notifyd = createNotifydClient({
  url: process.env.NOTIFYD_URL!,
  apiKey: process.env.NOTIFYD_API_KEY!,
});

await notifyd.send({
  channel: 'email',
  subscriberId: 'user-1',
  subject: 'Welcome!',
  body: 'Hello {{first_name}}!',
  vars: { first_name: 'Alice' },
});

const inbox = createNotifydClient({
  url: process.env.NEXT_PUBLIC_NOTIFYD_URL!,
  subscriberToken: token,
});

const messages = await inbox.getInbox('user-1', { limit: 20 });
```

The SDK wraps send, subscribers, subscriber JWT creation, inbox reads, unread count, mark read, mark all read, and SSE stream setup.

### Roll Your Own

The API is still simple REST + SSE. Any HTTP client works. See the examples above in curl, TypeScript, and Rust.

## Email Deliverability

"Sent" only means the provider accepted the API call. Resend webhooks close the
loop: notifyd records what actually happened (delivered, bounced, complained)
and stops writing to addresses that bounced or complained.

### Webhook ingestion

`POST /webhooks/resend` (note: **not** under `/v1`) receives Resend events,
authenticated by their svix signature — no API key. Configure it once:

1. Create a webhook in Resend pointing at
   `https://<your-notifyd>/webhooks/resend` with the events
   `email.delivered`, `email.bounced`, `email.complained`.
2. Put its signing secret in the `RESEND_WEBHOOK_SECRET` env var and restart.
   Without it the endpoint answers `503` and nothing is ingested (fail closed).

Every accepted event is stored in `provider_events` (idempotent on the svix
message id). Effects per event:

| Event | Effect |
|---|---|
| `email.delivered` | stamps `jobs.delivered_at` |
| `email.bounced` (Permanent) | job status → `bounced`, error = bounce message, **suppression created** |
| `email.bounced` (Transient) | recorded only — soft bounces resolve on their own |
| `email.complained` | job stays `sent` (it WAS delivered), **suppression created** |

Events map back to jobs through the `notifyd_job_id` tag that the worker adds
to every outgoing email. Projects subscribed to outbound webhooks also receive
`job.bounced` / `job.complained`.

### Commercial unsubscribe (bulk email)

Every email that is `bulk` (priority ≥ 80, or tagged `category=campaign|marketing|newsletter`)
leaves with `List-Unsubscribe: <PUBLIC_URL/u/<token>>` and
`List-Unsubscribe-Post: List-Unsubscribe=One-Click` (RFC 8058), unless the
caller set its own `List-Unsubscribe` header. Requires `PUBLIC_URL` on the
instance. The token is an HMAC (`JWT_SECRET`) over project, address and a
400-day expiry: nothing to store, nothing to guess.

- `GET /u/:token` shows a confirmation page with one button (a GET never
  changes state: mail scanners follow links).
- `POST /u/:token` records the unsubscribe: a suppression with
  `reason = unsubscribe`, `scope = marketing`. Bulk email to the address
  stops; transactional email (orders, security codes) still goes.

Suppressions carry a `scope`: `all` (bounce, complaint, manual block: nothing
is sent) or `marketing`. `POST /v1/suppressions` and
`POST /v1/admin/suppressions` accept `"scope": "all" | "marketing"`.

### Suppression list

An **active suppression** (project + address) makes the worker fail every email
job to that address immediately — `status: failed`, error
`recipient suppressed: …` — without calling the provider. Releasing it is an
audited decision, never a deletion.

**List suppressions**

```bash
curl https://notifyd.example.com/v1/suppressions \
  -H "X-Api-Key: sk_your_project_key"
# → { "data": [ { "id", "email", "reason", "detail", "created_at", "released_at" } ] }
# ?include_released=true also returns historical (released) rows
```

**Release a suppression** (allow sending to the address again)

```bash
curl -X DELETE https://notifyd.example.com/v1/suppressions/<id> \
  -H "X-Api-Key: sk_your_project_key"
# → { "success": true, "id": "..." }
```

If the address bounces again after a release, a fresh suppression is created
next to the released one — the history tells the whole story.
