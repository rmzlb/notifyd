# Roadmap (September 2026)

What notifyd does not do yet, in the order we intend to build it, with the
size of each piece. Estimates are working days for one engineer who knows
the codebase; they include tests and documentation. Nothing here is a
promise of a date.

## Where notifyd already stands

Five channels from one call (email, SMS, WhatsApp, Web Push and FCM, in-app),
PostgreSQL as the only dependency, priority lanes, per-channel pacing with
exact `Retry-After` semantics, provider failover, send windows in the
recipient's time zone, RFC 8058 one-click unsubscribe, multi-project with key
rotation, digest + MCP server + Agent Skills for operations, reproducible
benchmarks, MIT licence, TypeScript and Python clients, three production
instances.

## Shipped since this roadmap was written (10 September 2026)

| Piece | Where |
|---|---|
| Native APNs connector (`.p8` token auth, HTTP/2, full `aps`, dead tokens dropped) | `src/connectors/apns.rs`, `APNS_*` in docs/CONNECTORS.md |
| Topics (per-topic, per-channel preferences, enforced at enqueue, topic-level unsubscribe page) | `src/topics.rs`, `topic` on send/batch/templates |
| Segments (`batch` to a filter, `POST /v1/segments/preview`) | `src/segments.rs` |
| Own open and click tracking for every email provider | `src/tracking.rs`, `/t/o`, `/t/c` |
| `notifyd` CLI subcommands (digest, jobs, job, retry, cancel, send-test) | `src/cli.rs` |

## What comes next

| # | Piece | Why | Size |
|---|---|---|---|
| 1 | **Swift package** — device token registration, inbox, unread badge, `EventSource` stream | Pairs with APNs; without it, an iOS team writes the same 200 lines every time | 2 d |
| 2 | **Kotlin package** — same surface for Android | Pairs with FCM | 2 d |
| 3 | **Per-topic outcomes in the digest** — sent, opened, unsubscribed per topic | Topics exist; the operator view should show them | 0.5 d |

## Deliberately not planned

- **A web dashboard.** The digest endpoint, the MCP tools and the CLI cover
  the operator's job. A dashboard is a second application to build, secure
  and host; the projects that need charts point Grafana at
  `/v1/metrics/prometheus`.
- **A hosted, multi-tenant SaaS.** One instance per company is the model:
  `docker compose up`, your providers, your data.
- **A/B testing, inbound email parsing, visual template editors.** Out of
  scope for a delivery engine; templates are text with `{{variables}}`.

## How to influence this

Open an issue with the use case, not the feature. The order above changes
when a production user needs something sooner.
