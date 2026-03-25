# notifyd

Self-hosted notification micro-service in Rust. Replaces Novu. Handles email, SMS, and in-app notifications with scheduling, retry, and realtime SSE inbox.

## Features

- **Email** via Resend
- **SMS** via Twilio or Telnyx (switchable in config)
- **In-app** inbox with realtime SSE — replaces `@novu/react`
- **Scheduling** — `scheduled_at` on any notification
- **Queue** — Postgres-backed, `SELECT FOR UPDATE SKIP LOCKED`, no Redis
- **Retry** — exponential backoff (30s → 2min → 10min), max 3 attempts
- **Idempotency** — safe to retry from client
- **Multi-project** — one service, one API key per project
- **Templates** — `{{variable}}` substitution, stored in DB per project
- **Subscriber JWT** — frontend auth for inbox without exposing project API key

## Quick Start

```bash
cp notifyd.toml.example notifyd.toml
# Edit notifyd.toml with your keys

docker compose up -d
```

## API Reference

All endpoints require `X-Api-Key: sk_<project>_xxx` header.

### Send

```bash
# Immediate send (email + in-app)
curl -X POST http://localhost:3400/v1/send \
  -H "X-Api-Key: sk_square_xxx" \
  -H "Content-Type: application/json" \
  -d '{
    "channels": ["email", "in_app"],
    "subscriber_id": "user-uuid",
    "to": "user@example.com",
    "subject": "Votre demande a été approuvée",
    "body": "Bonjour {{name}}, votre demande PR-{{number}} est approuvée.",
    "vars": {"name": "Jean", "number": "001"}
  }'

# Scheduled SMS
curl -X POST http://localhost:3400/v1/schedule \
  -H "X-Api-Key: sk_square_xxx" \
  -H "Content-Type: application/json" \
  -d '{
    "channel": "sms",
    "to": "+33612345678",
    "body": "Rappel RDV {{date}} à {{hour}}",
    "vars": {"date": "25 mars", "hour": "14h"},
    "scheduled_at": "2026-03-25T12:00:00Z",
    "idempotency_key": "appt-42-reminder"
  }'

# Batch (multiple subscribers)
curl -X POST http://localhost:3400/v1/batch \
  -H "X-Api-Key: sk_square_xxx" \
  -H "Content-Type: application/json" \
  -d '{
    "channels": ["email", "in_app"],
    "subscribers": ["user-1", "user-2", "user-3"],
    "template": "purchase_approved",
    "vars": {"request_number": "PR-001"}
  }'
```

### In-App Inbox (frontend)

```bash
# 1. Backend: get subscriber token
curl -X POST http://localhost:3400/v1/auth/subscriber-token \
  -H "X-Api-Key: sk_square_xxx" \
  -d '{"subscriber_id": "user-uuid"}'
# → {"token": "eyJ..."}

# 2. Frontend: use token for inbox
curl http://localhost:3400/v1/inbox/user-uuid \
  -H "Authorization: Bearer eyJ..."

# 3. Realtime SSE
const es = new EventSource('/v1/inbox/user-uuid/stream?token=eyJ...')
es.onmessage = (e) => {
  const event = JSON.parse(e.data)
  // event.type: "new_notification" | "count_update"
}

# Mark read
curl -X PATCH http://localhost:3400/v1/inbox/user-uuid/msg-id \
  -H "Authorization: Bearer eyJ..." \
  -d '{"read": true}'

# Toggle todo
curl -X PATCH http://localhost:3400/v1/inbox/user-uuid/msg-id \
  -H "Authorization: Bearer eyJ..." \
  -d '{"is_todo": true}'

# Unread badge count
curl http://localhost:3400/v1/inbox/user-uuid/unread-count \
  -H "Authorization: Bearer eyJ..."
```

### Subscribers

```bash
# Create/update
curl -X POST http://localhost:3400/v1/subscribers \
  -H "X-Api-Key: sk_square_xxx" \
  -d '{"id": "user-uuid", "email": "user@example.com", "first_name": "Jean", "phone": "+33612345678"}'

# Get
curl http://localhost:3400/v1/subscribers/user-uuid \
  -H "X-Api-Key: sk_square_xxx"
```

### Jobs

```bash
# Check job status
curl http://localhost:3400/v1/jobs/job-uuid \
  -H "X-Api-Key: sk_square_xxx"

# Cancel scheduled job
curl -X DELETE http://localhost:3400/v1/jobs/job-uuid \
  -H "X-Api-Key: sk_square_xxx"
```

## SMS Providers

### Twilio

```toml
[connectors.sms]
provider = "twilio"
account_sid = "ACxxx"
auth_token = "xxx"
from = "+33600000000"
```

### Telnyx

```toml
[connectors.sms]
provider = "telnyx"
api_key = "KEY01xxx"
messaging_profile_id = "optional-uuid"
from = "+33600000000"
```

Switch between providers by changing `provider = "twilio"` to `provider = "telnyx"` — no code change needed.

## Migrating from Novu (Square)

1. Deploy notifyd, create Square project in `notifyd.toml`
2. Replace subscriber sync: `triggerNovu()` → `fetch('POST /v1/send')`
3. Backend: call `POST /v1/auth/subscriber-token` to get frontend JWT
4. Frontend: replace `NovuProvider` + `@novu/react` with `EventSource` + fetch calls
5. Kill Novu infra (MongoDB, Redis, 4 containers) ✓

## Environment Variables (alternative to TOML)

```env
DATABASE_URL=postgres://notifyd:xxx@localhost/notifyd
PORT=3400
JWT_SECRET=your-secret
RESEND_API_KEY=re_xxx
EMAIL_FROM=notifications@example.com
EMAIL_FROM_NAME=My App
```
