# notifyd — Notification Service

Self-hosted notification micro-service in Rust. Replaces Novu for all rmzlb projects.

## Goals

- **One service** for all projects (Square, Clozup, HelmAI, NLDos, GGBelote...)
- **4 channels**: email (Resend), SMS (Twilio), push (FCM, later), in-app (SSE + REST)
- **Scheduling**: `scheduled_at` on any notification
- **Queue with retry**: Postgres-backed, `SELECT FOR UPDATE SKIP LOCKED`
- **In-app inbox**: REST API + SSE realtime, replaces Novu's `@novu/react`
- **Simple**: no MongoDB, no Redis, just Postgres

## Architecture

```
┌──────────────┐
│  Project API  │  POST /v1/send, /v1/schedule, /v1/batch
└──────┬───────┘
       ▼
┌──────────────────┐
│   notifyd (axum) │
├──────────────────┤
│  REST API        │  → enqueue jobs
│  SSE endpoint    │  → /v1/inbox/{subscriber_id}/stream
│  Worker (tokio)  │  → poll queue, dispatch to connectors
└──────┬───────────┘
       ▼
┌──────────────────┐
│   PostgreSQL     │  jobs, subscribers, inbox_messages, templates
└──────────────────┘
```

## API Design

### Authentication

Every request needs:
- `X-Api-Key: sk_<project>_xxx` — identifies the project
- In-app/inbox endpoints also accept subscriber JWT for frontend use

### Core Endpoints

```
POST   /v1/send                  → immediate send (queued, processed <1s)
POST   /v1/schedule              → scheduled send
POST   /v1/batch                 → send to multiple recipients
DELETE /v1/jobs/{id}             → cancel a pending/scheduled job
GET    /v1/jobs/{id}             → job status

POST   /v1/subscribers           → create/update subscriber
GET    /v1/subscribers/{id}      → get subscriber
DELETE /v1/subscribers/{id}      → delete subscriber

GET    /v1/inbox/{sub_id}        → list in-app notifications (paginated)
PATCH  /v1/inbox/{sub_id}/{id}   → mark read/unread/archived/todo
POST   /v1/inbox/{sub_id}/read-all → mark all as read
GET    /v1/inbox/{sub_id}/unread-count → badge count
GET    /v1/inbox/{sub_id}/stream → SSE realtime stream

POST   /v1/auth/subscriber-token → generate subscriber JWT (HMAC signed)

GET    /v1/health                → healthcheck
```

### Send Payload

```json
{
  "channel": "email",
  "to": "subscriber_id_or_email_or_phone",
  "subscriber_id": "uuid-optional",
  "template": "appointment_reminder",
  "vars": {
    "name": "Jean",
    "date": "25 mars",
    "hour": "14h"
  },
  "scheduled_at": "2026-03-25T12:00:00Z",
  "idempotency_key": "appt-42-reminder-1"
}
```

For in-app channel:
```json
{
  "channel": "in_app",
  "subscriber_id": "user-uuid",
  "body": "Nouvelle demande d'achat PR-001",
  "icon": "package",
  "url": "/purchases?id=xxx",
  "vars": {}
}
```

Multi-channel (send to both email + in-app):
```json
{
  "channels": ["email", "in_app"],
  "subscriber_id": "user-uuid",
  "template": "purchase_approved",
  "vars": { "request_number": "PR-001", "approved_by": "Manager" }
}
```

### In-app Inbox Features

Matches what Novu provides in Square:
- **Realtime**: SSE stream pushes new notifications instantly
- **Read/Unread**: toggle per notification
- **Archive**: soft delete
- **Todo/Star**: flag for follow-up
- **Pagination**: cursor-based
- **Unread count**: for badge
- **Search**: by body text (query param)

### Subscriber JWT (frontend auth)

Projects call `POST /v1/auth/subscriber-token` with their API key + subscriber_id.
Returns a short-lived JWT that the frontend uses for inbox/SSE endpoints.
This replaces Novu's HMAC subscriber hash.

## Database Schema

### `projects`
```sql
CREATE TABLE projects (
    id          TEXT PRIMARY KEY,        -- "square", "clozup"
    api_key     TEXT UNIQUE NOT NULL,    -- "sk_square_xxx"
    name        TEXT NOT NULL,
    channels    TEXT[] DEFAULT '{}',     -- allowed channels
    created_at  TIMESTAMPTZ DEFAULT now()
);
```

### `subscribers`
```sql
CREATE TABLE subscribers (
    id          TEXT NOT NULL,           -- external user ID
    project_id  TEXT NOT NULL REFERENCES projects(id),
    email       TEXT,
    phone       TEXT,
    first_name  TEXT,
    last_name   TEXT,
    locale      TEXT DEFAULT 'fr',
    data        JSONB DEFAULT '{}',
    created_at  TIMESTAMPTZ DEFAULT now(),
    updated_at  TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (project_id, id)
);
```

### `templates`
```sql
CREATE TABLE templates (
    id          TEXT NOT NULL,           -- "appointment_reminder"
    project_id  TEXT NOT NULL REFERENCES projects(id),
    channel     TEXT NOT NULL,           -- "email", "sms", "in_app"
    subject     TEXT,                    -- email subject
    body        TEXT NOT NULL,           -- template body (supports {{var}})
    body_html   TEXT,                    -- optional HTML for email
    created_at  TIMESTAMPTZ DEFAULT now(),
    updated_at  TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (project_id, id, channel)
);
```

### `jobs`
```sql
CREATE TABLE jobs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id      TEXT NOT NULL REFERENCES projects(id),
    channel         TEXT NOT NULL,
    subscriber_id   TEXT,
    recipient       TEXT NOT NULL,       -- email/phone/subscriber_id
    template_id     TEXT,
    payload         JSONB NOT NULL DEFAULT '{}',
    status          TEXT DEFAULT 'pending',  -- pending/processing/sent/failed/cancelled
    scheduled_at    TIMESTAMPTZ DEFAULT now(),
    attempts        INT DEFAULT 0,
    max_attempts    INT DEFAULT 3,
    next_retry_at   TIMESTAMPTZ,
    idempotency_key TEXT,
    created_at      TIMESTAMPTZ DEFAULT now(),
    sent_at         TIMESTAMPTZ,
    error           TEXT
);

-- The idempotency key only reserves its slot while the job is live or
-- succeeded — a failed/cancelled job releases it (retry = new row):
CREATE UNIQUE INDEX jobs_active_idempotency_key
    ON jobs (project_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL
      AND status NOT IN ('failed', 'cancelled');

CREATE INDEX idx_jobs_queue ON jobs (scheduled_at)
    WHERE status IN ('pending', 'retry');
```

### `inbox_messages`
```sql
CREATE TABLE inbox_messages (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id      TEXT NOT NULL,
    subscriber_id   TEXT NOT NULL,
    body            TEXT NOT NULL,
    icon            TEXT DEFAULT 'bell',
    url             TEXT,
    data            JSONB DEFAULT '{}',
    read_at         TIMESTAMPTZ,
    archived_at     TIMESTAMPTZ,
    is_todo         BOOLEAN DEFAULT false,
    created_at      TIMESTAMPTZ DEFAULT now(),
    FOREIGN KEY (project_id, subscriber_id) REFERENCES subscribers(project_id, id)
);

CREATE INDEX idx_inbox_subscriber ON inbox_messages (project_id, subscriber_id, created_at DESC)
    WHERE archived_at IS NULL;
CREATE INDEX idx_inbox_unread ON inbox_messages (project_id, subscriber_id)
    WHERE read_at IS NULL AND archived_at IS NULL;
```

## Connectors

### Email (Resend)
```toml
[connectors.email]
provider = "resend"
api_key = "re_xxx"
from = "notifications@ctrlnz.com"
```

### SMS (Twilio)
```toml
[connectors.sms]
provider = "twilio"
account_sid = "ACxxx"
auth_token = "xxx"
from = "+33xxxxxxxxx"
```

### In-App
No external service. Writes to `inbox_messages` + broadcasts via SSE.

### Push (later)
FCM connector, not in v1.

## Worker

- Polls `jobs` table every 500ms with `SELECT FOR UPDATE SKIP LOCKED`
- Batch size: 50 jobs per poll
- Dispatches to appropriate connector
- On failure: exponential backoff (30s, 2min, 10min), max 3 attempts
- For in_app channel: insert into `inbox_messages` + notify SSE subscribers

## SSE (Server-Sent Events)

- Endpoint: `GET /v1/inbox/{subscriber_id}/stream`
- Auth: subscriber JWT in query param or Authorization header
- Events: `new_notification`, `read`, `unread`, `archived`, `count_update`
- Heartbeat: ping every 30s to keep connection alive
- Uses tokio broadcast channels internally (no Redis pubsub needed)

## Config

```toml
[server]
port = 3400
jwt_secret = "xxx"

[database]
url = "postgres://notifyd:xxx@localhost/notifyd"
max_connections = 10

[worker]
poll_interval_ms = 500
batch_size = 50
max_attempts = 3

[connectors.email]
provider = "resend"
api_key = "re_xxx"
from = "notifications@ctrlnz.com"

[connectors.sms]
provider = "twilio"
account_sid = "ACxxx"
auth_token = "xxx"
from = "+33xxxxxxxxx"

[projects.square]
api_key = "sk_square_xxx"
channels = ["email", "sms", "in_app"]

[projects.clozup]
api_key = "sk_clozup_xxx"
channels = ["email", "in_app"]
```

## Repo Structure

```
notifyd/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── config.rs
│   ├── db.rs
│   ├── api/
│   │   ├── mod.rs
│   │   ├── send.rs
│   │   ├── jobs.rs
│   │   ├── subscribers.rs
│   │   ├── inbox.rs
│   │   ├── auth.rs
│   │   └── health.rs
│   ├── worker.rs
│   ├── connectors/
│   │   ├── mod.rs
│   │   ├── email.rs      (Resend)
│   │   ├── sms.rs        (Twilio)
│   │   └── in_app.rs     (DB + SSE broadcast)
│   ├── sse.rs
│   └── templates.rs
├── migrations/
│   └── 001_init.sql
├── notifyd.toml.example
├── Dockerfile
├── docker-compose.yml
└── README.md
```

## Client SDK (later)

TypeScript package `@notifyd/react` that replaces `@novu/react`:
- `<NotifydProvider>` — connects SSE
- `useNotifications()` — list, read, archive, todo
- `<NotificationBell>` — drop-in badge
- `<NotificationInbox>` — full inbox component

For v1: projects use raw `fetch()` + `EventSource` — the API is simple enough.

## Migration from Novu (Square)

1. Deploy notifyd
2. Create Square project + subscribers via API
3. Migrate templates from `lib/novu/workflows/` to notifyd templates
4. Replace `triggerNovu()` calls with `fetch('POST /v1/send')`
5. Replace `NovuProvider` + `@novu/react` with EventSource + custom inbox
6. Kill Novu infra (MongoDB, Redis, 4 containers)

## Non-Goals (v1)

- No dashboard UI (query DB or add later)
- No preference center (projects handle opt-in/out)
- No digest/batching (add in v2 if needed)
- No push/FCM (add connector later)
- No webhook delivery (add if needed)
