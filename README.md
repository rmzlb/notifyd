<p align="center">
  <img src="docs/assets/notifyd-logo.svg" alt="notifyd" width="96" />
</p>

<h1 align="center">notifyd</h1>

<p align="center">
  <strong>The notification service your agent can send through <em>and</em> run.</strong><br>
  Email, SMS, WhatsApp, push, in-app inbox. One Rust binary. Postgres only. No dashboard: a digest endpoint and an MCP server instead.
</p>

<p align="center">
  <a href="https://github.com/rmzlb/notifyd/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="License"></a>
  <a href="https://github.com/rmzlb/notifyd/pkgs/container/notifyd"><img src="https://img.shields.io/badge/ghcr.io-rmzlb%2Fnotifyd-2496ED?style=flat-square&logo=docker&logoColor=white" alt="Container image"></a>
  <a href="https://crates.io/crates/notifyd"><img src="https://img.shields.io/crates/v/notifyd?style=flat-square&logo=rust" alt="crates.io"></a>
  <a href="https://github.com/rmzlb/notifyd/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/rmzlb/notifyd/ci.yml?branch=main&style=flat-square&label=ci" alt="CI"></a>
  <img src="https://img.shields.io/badge/image-42_MB-green?style=flat-square" alt="Image size">
  <img src="https://img.shields.io/badge/RSS-13_MB_idle-green?style=flat-square" alt="Memory">
  <a href="https://registry.modelcontextprotocol.io/v0/servers?search=notifyd"><img src="https://img.shields.io/badge/MCP_registry-io.github.rmzlb%2Fnotifyd-8A2BE2?style=flat-square" alt="MCP registry"></a>
  <a href="https://skills.sh/rmzlb/notifyd"><img src="https://skills.sh/b/rmzlb/notifyd" alt="Agent Skills"></a>
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> •
  <a href="#let-your-agent-run-it">Agent operations</a> •
  <a href="docs/API.md">API Reference</a> •
  <a href="docs/ARCHITECTURE.md">Architecture</a> •
  <a href="docs/BENCHMARKS.md">Benchmarks</a> •
  <a href="docs/llms.txt">llms.txt</a> •
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

---

## What it is

Every product needs to send email, texts and in-app notifications. The usual
options are a hosted SaaS billed per notification, or a self-hosted stack
with MongoDB, Redis and four containers behind a React dashboard.

notifyd is the third option: a **single 10 MB binary** with **PostgreSQL as
its only dependency**, that sends through the providers you already have
(Resend, Cloudflare Email Service, any SMTP, AgentMail, Telnyx, Twilio,
Web Push, FCM), with a real delivery engine (priorities, pacing, retries with
`Retry-After`, provider failover, send windows, one-click unsubscribe) and
**no admin UI at all**. Operating it is an API call or an MCP tool, so the
person on call can be an AI agent.

```
your app / your agent ──POST /v1/send──▶ notifyd ──▶ email · sms · whatsapp · push · in-app (SSE)
your agent            ──POST /mcp─────▶ notifyd ──▶ digest · jobs · retries · suppressions · settings
```

Three instances run in production today, one per company, operated this way.

---

## Send in one call

```bash
curl -X POST https://notifyd.example.com/v1/send \
  -H "X-Api-Key: sk_myapp_xxx" \
  -H "Content-Type: application/json" \
  -d '{
    "channels": ["email", "in_app"],
    "subscriber_id": "user-1",
    "subject": "Your report is ready",
    "body": "Hey {{first_name}}, the analysis you requested is complete.",
    "vars": {"first_name": "Alice"},
    "priority": "normal",
    "idempotency_key": "report-42-ready"
  }'
```

Flat REST, `curl` is the SDK. Retries are safe (`idempotency_key`), scheduling
is a field (`scheduled_at`), a marketing campaign is `POST /v1/batch` with
thousands of subscribers per call and it lands in the bulk lane so it never delays a
password reset. Follow any send with `GET /v1/jobs/:id`.

---

## Let your agent run it

Most notification tools were designed for a human clicking through a
dashboard. notifyd exposes **the operator's job as tools**, with the detail a
human operator would need:

**1. One call says what needs attention.** `GET /v1/admin/digest` ranks
findings and tells you the action for each one, in JSON or Markdown:

```markdown
# notifyd digest — last 1d
Instance: commit e14e6f3, up 3d, email resend (+ smtp fallback), sms telnyx

## Findings
- **warning** — Primary email provider `resend` is resting for 47s after refusing messages; `smtp` is delivering.
  _Nothing lost. Check the primary provider's status page; if it repeats, lower EMAIL_RATE_PER_SEC or move the primary role to the other provider._
- **warning** — Bounce rate 5.3 % over the window (14 bounced / 263 delivered).
  _Above 5 % providers throttle or suspend the sender. Clean the recipient list; suppressions are applied automatically._
- **warning** — 3 job(s) failed in the last 1d (0.1 % of terminal jobs). Top cause: 422 unverified sender domain.
  _Inspect with list_jobs(status=failed); permanent errors need a fix on the caller side, then retry_job._

## Queue          pending 0, retry 2, processing 0
## Outcomes       email/resend 4 812 sent, 3 failed · in_app 1 203 sent
## Latency        email p50 0.6s, p95 2.1s (scheduled → accepted by provider)
## Deliverability delivered 4 790, bounced 14, complained 0, unsubscribed 9
```

**2. The same operations as MCP tools.** `POST /mcp` is a Streamable HTTP
MCP server (current spec revision, legacy `initialize` kept). Add it to Claude
Code, Claude Desktop, Cursor or your own agent:

```json
{ "mcpServers": { "notifyd": {
  "type": "http", "url": "https://notifyd.example.com/mcp",
  "headers": { "Authorization": "Bearer ${NOTIFYD_ADMIN_API_KEY}" } } } }
```

| Tool | What the agent can do |
|---|---|
| `digest` | Ranked findings with actions, queue, outcomes, latency, deliverability, per project |
| `list_jobs`, `get_job` | Filter by project, channel, status, recipient, time; see provider, attempts, last error |
| `retry_job`, `cancel_job` | Act on a stuck or wrong send |
| `list_projects`, `update_project` | Sender identity (`from_email`, `from_name`), channels, inbound rate limit, bulk `send_window` in the recipients' timezone |
| `list_suppressions`, `add_suppression`, `release_suppression` | Suppression list with `all` or `marketing` scope |
| `template_metrics` | Sent, failed, bounced, opened per template |
| `send_test` | Prove the pipeline end to end on any channel |

Every tool carries `readOnlyHint` / `destructiveHint` annotations and an
`outputSchema`. A **read-only operator key** (`READONLY_API_KEY`) exposes only
the read tools, for an agent that reports but must not act. Every MCP call is
audited.

**3. Everything an agent needs to integrate is in the repo.** `docs/llms.txt`
is the whole API in plain text for a context window; three **Agent Skills**
ship in [`skills/`](skills/) (`notifyd-operate`, `notifyd-integrate`,
`notifyd-deploy`):

```bash
npx skills add rmzlb/notifyd
```

Published on the official MCP registry as `mcp-name: io.github.rmzlb/notifyd`
([`server.json`](server.json)). Full operator guide: [docs/AGENT.md](docs/AGENT.md).

---

## Features

**Channels**
- **Email** — Resend, Cloudflare Email Service, AgentMail, any SMTP (`lettre`), attachments, per-project sender identity
- **SMS** — Telnyx or Twilio, swap with one variable
- **WhatsApp** — Telnyx
- **Push** — Web Push (VAPID) or FCM
- **In-app inbox** — REST + realtime SSE (`EventSource`), read / archive / star, unread badge, multi-replica through Postgres `NOTIFY`

**Delivery engine**
- **Priorities** — `critical`, `normal`, `bulk` lanes; `/v1/batch` and campaign tags land in `bulk`
- **Pacing** — token bucket per channel (`EMAIL_RATE_PER_SEC`), a provider 429 pauses only the lane that hit it and honours `Retry-After`
- **Retries** — 30 s → 2 m → 10 m → 30 m → 2 h with jitter, 4xx fail fast, rejected batches fall back item by item
- **Failover** — second email provider with a circuit breaker (`EMAIL_FALLBACK_PROVIDER`)
- **Send windows** — quiet hours per project, evaluated in each subscriber's timezone
- **Scheduling, idempotency, stuck-job reaper, batch idempotency**

**Governance**
- **Unsubscribe** — RFC 8058 `List-Unsubscribe` one-click on every marketing email, suppression scopes `all` / `marketing`
- **Preferences** — per-subscriber opt-in / opt-out
- **Multi-project** — one instance, many projects, isolated by API key, key rotation with a grace period
- **PII masking** in logs, audit log of every mutation, per-project rate limit

**Operations**
- **Digest**, **MCP server**, **Agent Skills**, **`llms.txt`**
- **Metrics** — `/v1/metrics`, `/v1/metrics/prometheus`, per-template metrics
- **Webhooks** — delivery events to your endpoints
- **Workflows** — event-triggered multi-step sequences, state in Postgres
- **Templates** — `{{variable}}` substitution, stored per project

---

## Quick Start

### Docker (recommended)

The image reads its configuration from environment variables; no config file
to mount.

```bash
git clone https://github.com/rmzlb/notifyd.git && cd notifyd
cat > .env <<'EOF'
JWT_SECRET=change-me-32-random-chars-minimum
ADMIN_API_KEY=change-me-32-random-chars-minimum
RESEND_API_KEY=re_xxx
EMAIL_FROM=notifications@yourdomain.com
EOF
docker compose up -d          # notifyd + Postgres 16, http://localhost:3400
```

Create a project and get its API key:

```bash
curl -s -X POST http://localhost:3400/v1/admin/projects \
  -H "X-Api-Key: $ADMIN_API_KEY" -H "Content-Type: application/json" \
  -d '{"id":"myapp","name":"My app","channels":["email","in_app"],"from_email":"hello@yourdomain.com"}'
# → {"project": {"id": "myapp", "api_key": "sk_myapp_…", …}}
```

Prebuilt image, linux/amd64 and linux/arm64: `ghcr.io/rmzlb/notifyd`.

### From crates.io or source

```bash
cargo install notifyd                      # or: git clone … && cargo run
DATABASE_URL=postgres://notifyd:pass@localhost:5432/notifyd \
JWT_SECRET=… ADMIN_API_KEY=… RESEND_API_KEY=… EMAIL_FROM=… notifyd
```

No provider yet? `EMAIL_PROVIDER=log` prints emails instead of sending them.

### Verify

```bash
curl http://localhost:3400/v1/health
# → {"status":"ok","db":"ok","version":"0.2.0","commit":"…","uptime_seconds":12}
```

→ Full setup, TOML alternative, production notes: [docs/SETUP.md](docs/SETUP.md)

---

## API at a glance

Project endpoints take `X-Api-Key: sk_<project>_…`; operator endpoints take
the admin (or read-only) key, as `X-Api-Key` or `Authorization: Bearer`.
Inbox endpoints also accept a subscriber JWT.

| Method | Endpoint | What it does |
|--------|----------|--------------|
| `POST` | `/v1/send` | Send on one or several channels |
| `POST` | `/v1/batch` | Send to many subscribers (bulk lane, idempotent) |
| `GET` | `/v1/jobs/:id` | Status, provider, attempts, last error |
| `GET` | `/v1/inbox/:id` · `/stream` | In-app inbox, SSE realtime stream |
| `POST` | `/v1/workflows/trigger` | Trigger an event-based workflow |
| `GET` | `/v1/admin/digest` | What needs attention, with actions |
| `GET` | `/v1/admin/jobs` · `POST …/:id/retry` | Operator view and actions |
| `PATCH` | `/v1/admin/projects/:id` | Sender, channels, rate limit, send window |
| `POST` | `/mcp` | MCP server (Streamable HTTP) |
| `GET` | `/v1/metrics/prometheus` | Prometheus exposition |
| `GET` | `/u/:token` | One-click unsubscribe landing |

→ Every endpoint with examples: [docs/API.md](docs/API.md), or feed
[docs/llms.txt](docs/llms.txt) to your agent.

---

## vs. the alternatives

| | **Novu** | **Knock / Courier / SuprSend** | **notifyd** |
|---|---|---|---|
| **Infra** | MongoDB + Redis + 4 containers | Hosted SaaS | Postgres only, one 42 MB image |
| **Setup** | 30+ min | Signup + dashboard | `docker compose up` (2 min) |
| **Language** | Node.js (multiple services) | N/A (hosted) | Rust (single binary) |
| **Memory** | not measured by us | N/A | 13 MB idle, 23 MB draining 100k jobs ([method](docs/BENCHMARKS.md)) |
| **Throughput** | — | quota-bound | 44k jobs/s enqueued, 3.5k jobs/s drained ([benchmarks](docs/BENCHMARKS.md)) |
| **Provider 429** | job fails | managed | lane paused, `Retry-After` honoured, failover provider |
| **Priorities / send windows** | ❌ | ✅ | ✅ critical → bulk lanes, per-subscriber timezone windows |
| **Ops surface** | React dashboard | dashboard + API | digest endpoint, MCP server, Agent Skills, Prometheus |
| **Realtime** | WebSocket | WebSocket | SSE, native `EventSource`, multi-replica |
| **Self-hosted** | ✅ (heavy) | ❌ | ✅ one container per company |
| **Cost** | Free tier / paid | per notification | Free forever, MIT |

We only publish numbers we measured on notifyd itself; the rest of the table
describes shape, not performance. Method, hardware and bias disclaimer in
[docs/BENCHMARKS.md](docs/BENCHMARKS.md).

---

## In-app inbox and TypeScript SDK

```typescript
import { createNotifydClient } from 'notifyd-sdk';   // pnpm add notifyd-sdk@github:rmzlb/notifyd

const notifyd = createNotifydClient({ url: process.env.NOTIFYD_URL!, apiKey: process.env.NOTIFYD_API_KEY! });
await notifyd.send({ channels: ['email', 'in_app'], subscriberId: 'user-123',
  subject: 'Your report is ready', body: 'Hey {{first_name}}, the analysis is complete.', vars: { first_name: 'Alice' } });

// Browser: subscriber token from your backend, then a plain EventSource
const events = new EventSource(`${url}/v1/inbox/${userId}/stream?token=${jwt}`);
events.onmessage = (e) => { const d = JSON.parse(e.data);
  if (d.type === 'new_notification') showToast(d.notification);
  if (d.type === 'count_update') updateBadge(d.unread_count); };
```

---

## Workflows

```bash
curl -X POST http://localhost:3400/v1/workflows -H "X-Api-Key: sk_myapp_xxx" -d '{
  "id": "welcome-series", "trigger_event": "user.signup",
  "steps": [
    {"type": "send", "channel": "email", "template": "welcome"},
    {"type": "delay", "duration": "24h"},
    {"type": "condition", "check": "completed_onboarding",
     "if_false": [{"type": "send", "channel": "email", "template": "nudge"}]}
  ]}'
```

State lives in Postgres and survives restarts.

---

## Configuration

Environment variables are the primary interface (that is what the image and
the compose file use); a `notifyd.toml` is accepted for local development.
Required: `DATABASE_URL`, `JWT_SECRET`, `ADMIN_API_KEY`. Then one provider:

| Variable | Purpose |
|---|---|
| `EMAIL_PROVIDER` | `resend` (default when `RESEND_API_KEY` is set), `cloudflare`, `smtp`, `agentmail`, `log` |
| `EMAIL_FROM`, `EMAIL_FROM_NAME` | Instance default sender; projects can override |
| `EMAIL_FALLBACK_PROVIDER` | Second provider on 429 / 5xx |
| `EMAIL_RATE_PER_SEC` | Outbound pacing per replica |
| `SMS_PROVIDER`, `SMS_FROM` | `telnyx` or `twilio` with their credentials |
| `PUBLIC_URL` | Base URL for one-click unsubscribe links |
| `READONLY_API_KEY` | Optional read-only operator key |

→ Every variable, per provider: [docs/CONNECTORS.md](docs/CONNECTORS.md) and
[docker-compose.yml](docker-compose.yml). TOML reference:
[notifyd.toml.example](notifyd.toml.example).

---

## Architecture

```
                 ┌──────────────────────── notifyd (one binary) ───────────────────────┐
 HTTP /v1, /mcp ─▶ axum API ─▶ jobs table ─▶ worker: claim (SKIP LOCKED, by priority) │
                 │                              ├─ pacer per channel, lane pause on 429  │
                 │                              ├─ connectors (email/sms/whatsapp/push/in-app)
                 │                              ├─ failover breaker, retries, reaper      │
                 │                              └─ webhooks, metrics, audit               │
                 │  SSE hub ◀── Postgres NOTIFY ── (any replica)                          │
                 └───────────────────────────────┬──────────────────────────────────────┘
                                                 ▼
                                           PostgreSQL 16
```

```
src/
├── api/              # routes: send, batch, jobs, inbox, subscribers, templates, workflows, webhooks, admin ops, health
├── connectors/       # email (resend, cloudflare, smtp, agentmail, log), sms, whatsapp, push, in_app
├── worker.rs         # claim by priority, batch context, finalize, retries, failover
├── pacing.rs         # token buckets and lane pauses
├── failover.rs       # provider circuit breaker
├── ops.rs            # digest, findings, operator actions
├── mcp.rs            # MCP server (tools, annotations, audit)
├── send_window.rs    # quiet hours in the subscriber's timezone
├── unsubscribe.rs    # List-Unsubscribe tokens and landing
├── sse.rs            # inbox stream, Postgres NOTIFY fan-out
├── workflow_engine.rs, templates.rs, webhooks.rs, deliverability.rs, metrics.rs, pii.rs, middleware.rs
migrations/           # SQL, applied at start-up
skills/               # Agent Skills: operate, integrate, deploy
server.json           # MCP registry entry
```

~12 000 lines of Rust, no `unsafe`. 10.8 MB binary, 42 MB image, 13 MB RSS
idle, 23 MB while draining 100 000 jobs. → [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

---

## Status

notifyd is a 0.x used in production by its authors. What it does not do yet:
no dashboard (by design), no A/B testing, no APNs, no inbound email parsing, no multi-tenant billing. Breaking changes are announced
in release notes; the queue schema is migrated automatically.

---

## Documentation

| | |
|---|---|
| 📦 **[Setup](docs/SETUP.md)** | Local dev, Docker, production |
| 🔌 **[API reference](docs/API.md)** | Every endpoint with curl / TypeScript / Rust examples |
| 🤝 **[Agent operations](docs/AGENT.md)** | Digest, MCP tools, read-only key, how an agent runs an instance |
| 🔌 **[Connectors](docs/CONNECTORS.md)** | Providers, environment variables, adding one |
| 🏗️ **[Architecture](docs/ARCHITECTURE.md)** | Queue, lanes, SSE, connectors |
| 📈 **[Benchmarks](docs/BENCHMARKS.md)** | Footprint, throughput, how to reproduce |
| 🚀 **[Deployments](docs/DEPLOYMENTS.md)** | One instance per company, runbook |
| 📣 **[Visibility](docs/VISIBILITY.md)** | Registries and launch channels |
| 🤖 **[llms.txt](docs/llms.txt)** | The API in plain text for agents |

---

## Contributing

Issues and pull requests are welcome; `good first issue` is the place to
start, and big features begin with an issue. Read [CONTRIBUTING.md](CONTRIBUTING.md).

```bash
git clone https://github.com/YOUR_USERNAME/notifyd.git && cd notifyd
cargo test && EMAIL_PROVIDER=log DATABASE_URL=… JWT_SECRET=dev ADMIN_API_KEY=dev cargo run
```

## License

[MIT](LICENSE).

<p align="center">Built with 🦀 in Grenoble, France 🏔️</p>
