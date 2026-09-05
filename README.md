<p align="center">
  <img src="docs/assets/notifyd-logo.svg" alt="notifyd" width="80" />
</p>

<h1 align="center">notifyd</h1>

<p align="center">
  <strong>Agent-first notification service. One Rust binary. Postgres only. No Redis, no Mongo, no nonsense.</strong>
</p>

<p align="center">
  <a href="https://github.com/rmzlb/notifyd/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="License"></a>
  <a href="https://github.com/rmzlb/notifyd/pkgs/container/notifyd"><img src="https://img.shields.io/badge/ghcr.io-rmzlb%2Fnotifyd-2496ED?style=flat-square&logo=docker&logoColor=white" alt="Container image"></a>
  <a href="https://github.com/rmzlb/notifyd/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/rmzlb/notifyd/ci.yml?branch=main&style=flat-square&label=ci" alt="CI"></a>
  <img src="https://img.shields.io/badge/rust-2021_edition-orange?style=flat-square&logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/image-42_MB-green?style=flat-square" alt="Image size">
  <img src="https://img.shields.io/badge/RSS-13_MB_idle-green?style=flat-square" alt="Memory">
  <a href="https://modelcontextprotocol.io"><img src="https://img.shields.io/badge/MCP-server-8A2BE2?style=flat-square" alt="MCP server"></a>
  <a href="https://skills.sh/rmzlb/notifyd"><img src="https://skills.sh/b/rmzlb/notifyd" alt="Agent Skills"></a>
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> •
  <a href="docs/API.md">API Reference</a> •
  <a href="docs/SETUP.md">Setup Guide</a> •
  <a href="docs/ARCHITECTURE.md">Architecture</a> •
  <a href="docs/BENCHMARKS.md">Benchmarks</a> •
  <a href="docs/llms.txt">LLM Docs</a> •
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

---

## The Problem

Your AI agent needs to send an email. Or a push notification. Or update an in-app inbox.

You look at Novu: MongoDB, Redis, 4 containers, a React SDK, 30 minutes of setup. Your agent doesn't care about any of that. It just wants to `POST /v1/send` and move on.

**notifyd** is what that looks like. A single Rust binary. One `POST` call. Your agent sends notifications and gets back to work.

```
Agent ──POST /v1/send──→ notifyd ──→ Email (Resend)
                                 ──→ SMS (Twilio/Telnyx)
                                 ──→ Push (FCM)
                                 ──→ In-App (SSE)
```

---

## Why Agents Love This

Most notification services were designed for humans clicking buttons in a dashboard. notifyd was designed for agents making API calls.

**Flat REST API** — no SDK needed, no WebSocket handshake, no complex auth flows. `curl` works. Your agent's HTTP client works.

**`docs/llms.txt`** — the entire API reference in plain text, optimized for LLM context windows. Point your agent at it and it can call any endpoint. ([View it](docs/llms.txt))

**Idempotency built-in** — agents retry. That's fine. Pass `idempotency_key` and notifyd deduplicates.

**One binary, one config file** — `docker compose up` and you have a notification service. No infra degree required.

```bash
# Your agent sends a notification. That's it.
curl -X POST http://localhost:3400/v1/send \
  -H "X-Api-Key: sk_myapp_xxx" \
  -H "Content-Type: application/json" \
  -d '{
    "channels": ["email", "in_app"],
    "subscriber_id": "user-1",
    "subject": "Your report is ready",
    "body": "Hey {{first_name}}, the analysis you requested is complete.",
    "vars": {"first_name": "Alice"},
    "idempotency_key": "report-42-ready"
  }'
```

### Connect Your Agent to the Docs

Feed `docs/llms.txt` to any LLM agent and it can operate the full API:

```
https://raw.githubusercontent.com/rmzlb/notifyd/main/docs/llms.txt
```

Or describe notifyd as a tool:

```json
{
  "name": "send_notification",
  "description": "Send email/SMS/push/in-app via notifyd",
  "endpoint": "POST /v1/send",
  "auth": "X-Api-Key header"
}
```

---

## vs. The Alternatives

| | **Novu** | **Knock / Courier / SuprSend** | **notifyd** |
|---|---|---|---|
| **Infra** | MongoDB + Redis + 4 containers | Hosted SaaS | Postgres only, one 42 MB image |
| **Setup** | 30+ min | Signup + dashboard | `docker compose up` (2 min) |
| **Language** | Node.js (multiple services) | N/A (hosted) | Rust (single binary) |
| **Memory** | not measured by us | N/A | 13 MB idle, 23 MB draining 100k jobs ([method](docs/BENCHMARKS.md)) |
| **Throughput** | — | quota-bound | 44k jobs/s enqueued, 3.5k jobs/s drained ([benchmarks](docs/BENCHMARKS.md)) |
| **Provider 429** | job fails | managed | lane paused, `Retry-After` honoured, failover provider |
| **Priorities / send windows** | ❌ | ✅ | ✅ critical → bulk lanes, per-subscriber timezone windows |
| **Ops for agents** | React dashboard | dashboard + API | `GET /v1/admin/digest`, MCP server, Agent Skills, Prometheus |
| **Realtime** | WebSocket | WebSocket | SSE (native `EventSource`, multi-replica via Postgres `NOTIFY`) |
| **Self-hosted** | ✅ (heavy) | ❌ | ✅ (one container per company) |
| **Queue** | Redis + BullMQ | Managed | Postgres `SKIP LOCKED`, stuck-job reaper |
| **Cost** | Free tier / paid | per notification | Free forever, MIT |

---

## Features

- **📧 Email** — Resend, Cloudflare Email Service, AgentMail or any SMTP, with an optional failover provider
- **📱 SMS** — Twilio or Telnyx (swap in config, zero code change)
- **🔔 Push** — FCM (Firebase Cloud Messaging)
- **💬 In-app inbox** — REST + realtime SSE stream
- **⏰ Scheduling** — `scheduled_at` on any notification
- **🔄 Retry** — backoff 30s → 2m → 10m → 30m → 2h with jitter, provider `Retry-After` honoured
- **🚥 Priorities** — critical / normal / bulk lanes, a 429 pauses only the lane that hit it
- **🕰️ Send windows** — per-project quiet hours in each subscriber's timezone
- **📭 Unsubscribe** — `List-Unsubscribe` one-click on every marketing email, suppression scopes
- **🔑 Idempotency** — safe agent retries
- **📋 Templates** — `{{variable}}` substitution, stored per project
- **🏢 Multi-project** — one instance, many projects, isolated by API key
- **⚡ Workflows** — event-triggered multi-step sequences
- **👤 Preferences** — per-subscriber opt-in/opt-out
- **🔍 Audit log** — every mutation logged
- **🚦 Rate limiting** — per-project sliding window
- **📊 Metrics** — `/v1/metrics`, `/v1/metrics/prometheus`, per-template metrics
- **🧭 Digest + MCP** — `GET /v1/admin/digest` and `POST /mcp` so an agent operates the instance
- **🪝 Webhooks** — delivery events to your endpoints

---

## Quick Start

### Docker (recommended)

Prebuilt image (linux/amd64 + linux/arm64, 42 MB): `ghcr.io/rmzlb/notifyd`.

```bash
git clone https://github.com/rmzlb/notifyd.git && cd notifyd
cp notifyd.toml.example notifyd.toml
# Edit notifyd.toml — add your Resend API key at minimum

docker compose up -d
# → notifyd running on http://localhost:3400
```

### From source

```bash
# Rust 1.75+, PostgreSQL 16+
git clone https://github.com/rmzlb/notifyd.git && cd notifyd
cp notifyd.toml.example notifyd.toml
cargo run
```

### Verify

```bash
curl http://localhost:3400/v1/health
# → {"status":"ok","db":"ok","version":"0.2.0"}
```

→ Full setup: [docs/SETUP.md](docs/SETUP.md)

---

## API at a Glance

Every endpoint uses `X-Api-Key: sk_<project>_xxx`. Inbox endpoints also accept subscriber JWT.

| Method | Endpoint | What it does |
|--------|----------|--------------|
| `POST` | `/v1/send` | Send notification (email, SMS, push, in-app) |
| `POST` | `/v1/batch` | Send to multiple subscribers |
| `GET` | `/v1/inbox/:id` | List in-app notifications |
| `GET` | `/v1/inbox/:id/stream` | SSE realtime stream |
| `POST` | `/v1/workflows/trigger` | Trigger event-based workflow |
| `GET` | `/v1/health` | Health check |
| `GET` | `/v1/metrics` | Service metrics |

→ Full reference: [docs/API.md](docs/API.md) — or feed [docs/llms.txt](docs/llms.txt) to your agent.

---

## TypeScript SDK

A small official SDK now ships in this repo for backend + frontend apps.

```bash
pnpm add notifyd-sdk@github:rmzlb/notifyd
```

```typescript
import { createNotifydClient } from 'notifyd-sdk';

const notifyd = createNotifydClient({
  url: process.env.NOTIFYD_URL!,
  apiKey: process.env.NOTIFYD_API_KEY!,
});

await notifyd.send({
  channels: ['email', 'in_app'],
  subscriberId: 'user-123',
  subject: 'Your report is ready',
  body: 'Hey {{first_name}}, the analysis is complete.',
  vars: { first_name: 'Alice' },
});

const token = await notifyd.createSubscriberToken({
  subscriberId: 'user-123',
  ttlHours: 8,
});
```

It wraps the REST API with typed helpers for send, subscribers, inbox, unread count, mark read, and SSE stream setup.

---

## In-App Inbox

Complete notification inbox with realtime SSE. No WebSocket library, no Redis pub/sub — just native `EventSource`.

```typescript
// Connect to realtime stream
const events = new EventSource(
  `https://notifyd.example.com/v1/inbox/${userId}/stream?token=${jwt}`
);

events.onmessage = (e) => {
  const data = JSON.parse(e.data);
  if (data.type === 'new_notification') showToast(data.notification);
  if (data.type === 'count_update') updateBadge(data.unread_count);
};
```

Features: read/unread, archive, todo/star, pagination, unread count badge, realtime push.

---

## Workflow Engine

Multi-step notification sequences triggered by events:

```bash
curl -X POST http://localhost:3400/v1/workflows \
  -H "X-Api-Key: sk_myapp_xxx" \
  -d '{
    "id": "welcome-series",
    "trigger_event": "user.signup",
    "steps": [
      {"type": "send", "channel": "email", "template": "welcome"},
      {"type": "delay", "duration": "24h"},
      {"type": "send", "channel": "email", "template": "getting_started"},
      {"type": "delay", "duration": "72h"},
      {"type": "condition", "check": "completed_onboarding", "if_false": [
        {"type": "send", "channel": "email", "template": "nudge"}
      ]}
    ]
  }'
```

State persisted in Postgres — survives restarts. No in-memory state to lose.

---

## Built for agents, not dashboards

No admin UI. `GET /v1/admin/digest` tells you, in one call, what deserves
attention and what to do about it; `POST /mcp` exposes the same operations as
MCP tools so Claude Code, Claude Desktop or Cursor can run the instance:

```json
{ "mcpServers": { "notifyd": { "type": "http", "url": "https://notifyd.example.com/mcp",
  "headers": { "Authorization": "Bearer ${NOTIFYD_ADMIN_API_KEY}" } } } }
```

→ [docs/AGENT.md](docs/AGENT.md)

Agent Skills for Claude Code, Cursor and friends live in [`skills/`](skills/):
`notifyd-operate`, `notifyd-integrate`, `notifyd-deploy`.

```bash
npx skills add rmzlb/notifyd
```

MCP registry name: `mcp-name: io.github.rmzlb/notifyd` (see [`server.json`](server.json)).

## Configuration

Single TOML file. Minimal setup:

```toml
[server]
port = 3400
jwt_secret = "your-secret-here"

[database]
url = "postgres://notifyd:pass@localhost:5432/notifyd"

[connectors.email]
provider = "resend"
api_key = "re_xxx"
from = "notifications@yourdomain.com"

[projects.myapp]
api_key = "sk_myapp_xxx"
channels = ["email", "in_app"]
```

→ Full config: [notifyd.toml.example](notifyd.toml.example). Providers and
their environment variables (Resend, Cloudflare Email Service, any SMTP,
AgentMail, Telnyx, Twilio, web push): [docs/CONNECTORS.md](docs/CONNECTORS.md).

---

## Project Structure

```
notifyd/
├── src/
│   ├── main.rs              # Server bootstrap, graceful shutdown
│   ├── config.rs             # TOML config
│   ├── db.rs                 # sqlx models
│   ├── worker.rs             # Background job processor
│   ├── workflow_engine.rs    # Event-driven workflows
│   ├── sse.rs                # SSE broadcaster (tokio channels)
│   ├── templates.rs          # {{var}} engine
│   ├── pii.rs                # PII masking for logs
│   ├── middleware.rs          # Rate limiter + audit
│   ├── api/                  # 13 route modules
│   └── connectors/           # Email, SMS, Push, In-App
├── migrations/               # SQL migrations (auto-run)
├── Dockerfile                # Multi-stage + cargo-chef
├── docker-compose.yml        # notifyd + Postgres
├── notifyd.toml.example      # Config reference
└── docs/                     # API ref, setup, architecture, llms.txt
```

~12,000 lines of Rust, no `unsafe`. 10.8 MB binary, 42 MB image, 13 MB RSS at idle.

---

## Documentation

| | |
|---|---|
| 📦 **[Setup Guide](docs/SETUP.md)** | Local dev, Docker, production deploy |
| 🔌 **[API Reference](docs/API.md)** | Every endpoint with curl/TS/Rust examples |
| 🏗️ **[Architecture](docs/ARCHITECTURE.md)** | Queue design, SSE internals, connectors |
| 📈 **[Benchmarks](docs/BENCHMARKS.md)** | Footprint, throughput, how to reproduce |
| 📣 **[Visibility](docs/VISIBILITY.md)** | Registries, lists and launch channels, in order |
| 🔌 **[Connectors](docs/CONNECTORS.md)** | Providers, environment variables, adding one |
| 🤝 **[Agent operations](docs/AGENT.md)** | Digest, MCP tools, how an agent runs an instance |
| 🚀 **[Deployments](docs/DEPLOYMENTS.md)** | One instance per company, runbook, inventory |
| 🤖 **[LLM Docs](docs/llms.txt)** | Full API in plain text — feed to your agent |

---

## Contributing

notifyd is built in **Grenoble, in the French Alps** 🏔️ — but open to contributors from everywhere.

1. Read the [Contributing Guide](CONTRIBUTING.md)
2. Check [open issues](https://github.com/rmzlb/notifyd/issues) — `good first issue` is a great start
3. Big features → open an issue first

```bash
git clone https://github.com/YOUR_USERNAME/notifyd.git
cd notifyd && cp notifyd.toml.example notifyd.toml
cargo test && cargo run
```

---

## License

[MIT](LICENSE) — use it however you want.

---

<p align="center">
  Built with 🦀 in Grenoble, France 🏔️
</p>
