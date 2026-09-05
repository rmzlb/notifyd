# Benchmarks

Numbers you can reproduce, measured on 2026-09-05 against commit `679c211`
plus the batch-insert / batch-context work shipped right after it. All
figures come from a single notifyd process talking to one PostgreSQL 16
container on the same host. Nothing was tuned in Postgres.

## Test bench

| | |
|---|---|
| Host | 8 vCPU (Arm Neoverse-V2), 30 GB RAM, Ubuntu, Docker |
| Postgres | 16, default `postgresql.conf`, one container, same host |
| notifyd | release build, `DATABASE_MAX_CONNECTIONS=20`, `WORKER_BATCH_SIZE=500`, `WORKER_POLL_INTERVAL_MS=100`, `EMAIL_RATE_PER_SEC=0` (pacer off) |
| Provider | `EMAIL_PROVIDER=log` with a batch size of 100, i.e. the exact code path Resend takes (claim → build → batch → finalize → webhooks) minus the network |
| Load | one project with an unlimited rate limit, 100 000 `email` jobs enqueued with `scheduled_at` in the future, released in a single `UPDATE`, drained by one worker |

The `log` provider is a no-op, so the drain numbers measure **notifyd and
Postgres**, not Resend. On a real provider the ceiling is the provider's
quota (Resend: 100 recipients per batch call, 2 requests/s by default) and
the pacer will hold the line there; see `docs/CONNECTORS.md`.

## Footprint

| Metric | Value |
|---|---|
| Release binary (stripped, musl) | 10.8 MB |
| Docker image (`alpine:3.22` runtime) | 42 MB |
| RSS at idle (worker polling, SSE hub up) | 13 MB |
| RSS peak while draining 100 000 jobs | 23 MB |
| RSS peak while accepting 50 000 jobs through `/v1/batch` | 29 MB |
| Postgres `jobs` table + indexes for 100 000 sent jobs | 84 MB (≈ 0.85 kB per job, body included) |
| Rust source | ~12 000 lines, 14 migrations, no `unsafe` |

The process does not grow with the queue: jobs live in Postgres, the worker
holds at most one claimed batch in memory.

## Throughput

| Path | Result |
|---|---|
| `POST /v1/send` (HTTP round-trip incl. auth, rate limit, insert), 8 concurrent clients | 1 867 req/s, p50 3.9 ms, p99 < 15 ms |
| `POST /v1/batch`, 5 000 subscribers per call (one `INSERT … SELECT FROM unnest`) | 44 500 jobs/s enqueued |
| Worker drain, batching provider (100 per call) | **3 537 jobs/s** sustained over 100 000 jobs |
| Worker drain, non-batching provider (1 per call, e.g. SMTP, Twilio) | 640 jobs/s |
| `GET /v1/admin/digest` over 100 000 jobs | 70 ms |

Before the set-based insert, `/v1/batch` ran at 719 jobs/s (one `INSERT` per
subscriber) and the drain at 516 jobs/s (per-job sender/suppression/preference
lookups and one webhook task per job). The commit that follows `679c211`
replaced those with one statement per batch: senders, suppressions and
preferences are fetched once per claimed batch, jobs are claimed and
finalised with `ANY($1)` / `unnest`, and webhook fan-out is skipped
entirely for projects without a webhook.

## What this means in practice

- A **transactional workload** (order confirmations, password resets, a few
  thousand emails a day) runs comfortably on the smallest VM you can rent,
  next to the rest of your stack, with Postgres you already have.
- A **marketing send of 100 000 emails** is accepted in about 2 s and is
  fully queued in Postgres before the API answers. The drain is then bound by
  your provider's quota, not by notifyd: with Resend's default 2 requests/s
  × 100 recipients, 100 000 emails leave in roughly 8–9 minutes, priority
  lanes keep transactional mail ahead of the bulk, and a 429 pauses the
  bulk lane only.
- One instance per company (see `docs/DEPLOYMENTS.md`) costs one 42 MB
  container and one Postgres database. Three companies = three containers,
  well under 100 MB of RAM in total.

## How the numbers compare

Are we biased? Probably, yes: this page is written by the people who wrote
notifyd. So it only publishes numbers we measured ourselves, on notifyd. We
have not run Novu, Knock, Courier or SuprSend through this protocol, and the
hosted products cannot be measured this way at all. What can be compared
without a benchmark is the shape of each system:

| | notifyd | Novu (self-hosted) | Knock / Courier / SuprSend |
|---|---|---|---|
| Runtime | 1 container, Postgres | API, worker, WebSocket and web containers, MongoDB, Redis | hosted SaaS |
| Queue durability | Postgres row per job, `SKIP LOCKED` | Redis / BullMQ | managed |
| Provider 429 | pauses the lane, retries with `Retry-After` | job fails (one attempt) | managed |
| Ops surface | REST digest, MCP tools, Prometheus | React dashboard | dashboard + API |

If you run Novu (or anything else) through the protocol below on the same
class of hardware, open an issue with the command lines and the numbers and
we will link it here, whatever they say.

## Reproduce

```bash
# 1. Scratch database and a release binary
createdb notifyd_bench
cargo build --release

# 2. Run notifyd with a no-op provider and the pacer off
DATABASE_URL=postgres://.../notifyd_bench JWT_SECRET=... ADMIN_API_KEY=bench \
EMAIL_PROVIDER=log EMAIL_FROM=bench@example.invalid \
WORKER_BATCH_SIZE=500 WORKER_POLL_INTERVAL_MS=100 EMAIL_RATE_PER_SEC=0 \
DATABASE_MAX_CONNECTIONS=20 RUST_LOG=notifyd=warn ./target/release/notifyd

# 3. Project without rate limit, 20 × 5 000 jobs scheduled in the future
curl -s -X POST -H "x-api-key: bench" -H 'content-type: application/json' \
  -d '{"id":"bench","name":"Bench","channels":["email"],"rate_limit_per_min":100000000}' \
  http://localhost:3400/v1/admin/projects
# then POST /v1/batch with {"channel":"email","subscribers":[...5000 ids...],
#   "subject":"bench","body":"x","scheduled_at":"2030-01-01T00:00:00Z"} × 20

# 4. Release everything at once and time the drain
psql notifyd_bench -c "UPDATE jobs SET scheduled_at = now() WHERE status = 'pending'"
# poll: SELECT count(*) FROM jobs WHERE status IN ('pending','processing','retry')
# RSS:  ps -o rss= -p <pid>
```
