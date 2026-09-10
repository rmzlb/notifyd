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

## What comes next

| # | Piece | Why | Size |
|---|---|---|---|
| 1 | **Native APNs connector** — HTTP/2 with `.p8` token auth, device token lifecycle (`410 Unregistered` deletes the token), rich payload (title, body, badge, sound, thread id, `mutable-content`, collapse id), silent push | Mobile teams evaluate a notification server on iOS push first. FCM can relay to APNs but adds a Google account and hides delivery errors | 3 d |
| 2 | **Topics** — a `topic` on `send` / `batch`, subscriber preferences per topic and channel, enforced at enqueue, topic-level opt-out on the unsubscribe landing | "Subscribe to release notes but not to tips" is the preference model users actually understand; today preferences are per channel and per workflow only | 1.5 d |
| 3 | **Segments** — `POST /v1/batch` with a filter on subscriber `locale`, `timezone` and `data` (JSON path, `eq/neq/in/exists`), plus a count preview endpoint | A campaign to "plan = pro, country = FR" without the caller paginating subscribers | 2 d |
| 4 | **Own open and click tracking** — pixel and redirect served by notifyd for every email provider (today opens and clicks come from Resend webhooks only), `opened_at` / `clicked_at` on the job, per-template funnel completed | Delivery receipts must not depend on which provider is configured | 1.5 d |
| 5 | **`notifyd` CLI subcommands** — `notifyd digest`, `notifyd jobs --status failed`, `notifyd retry <id>`, `notifyd send-test`, reusing `ops.rs`; same binary, no extra install | The operator surface stays API + MCP + terminal, no dashboard to host | 2 d |
| 6 | **Swift package** — device token registration, inbox, unread badge, `EventSource` stream | Pairs with #1; without it, an iOS team writes the same 200 lines every time | 2 d |
| 7 | **Kotlin package** — same surface for Android | Pairs with FCM/APNs | 2 d |

Items 1 to 5: about ten working days. Items 6 and 7 follow once APNs is
shipped.

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
