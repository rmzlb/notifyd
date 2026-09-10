# Connectors

A connector turns a job into one provider request. Every connector returns
the same two things: a `Delivery` (provider name + provider message id) or a
`ProviderError` whose *kind* drives the worker (`RateLimited`, `Transient`,
`Permanent`, `Suppressed`). Adding a provider is one file implementing the
`Connector` trait in `src/connectors/` plus its environment variables below;
retries, pacing, priority, metrics and evidence come for free.

Configuration is per instance (one instance per company, see
`DEPLOYMENTS.md`): the instance's environment selects one provider per
channel. Names only here, never values.

## Email — `EMAIL_PROVIDER`

Common: `EMAIL_FROM` (default sender, verified at the provider),
`EMAIL_FROM_NAME`. A project can override both with `from_email` /
`from_name` (`POST /v1/admin/projects`).

| `EMAIL_PROVIDER` | Variables | Notes |
|---|---|---|
| `resend` (default when `RESEND_API_KEY` is set) | `RESEND_API_KEY` | Native batch (`/emails/batch`, 100 per call), tags, headers, attachments on single send. Rate limit 10 req/s per team: keep `EMAIL_RATE_PER_SEC` ≤ 8 per replica. |
| `cloudflare` | `CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_EMAIL_API_TOKEN` (token with the Email Sending permission) | Cloudflare Email Service REST API (beta, Workers Paid). Headers, reply-to, cc, attachments (5 MiB total). A recipient reported in `permanent_bounces` fails the job permanently. No provider message id. |
| `smtp` | `SMTP_HOST`, `SMTP_PORT` (587), `SMTP_USERNAME`, `SMTP_PASSWORD`, `SMTP_SECURITY` = `starttls` \| `tls` \| `none` | Any SMTP submission service: Amazon SES, Postmark, Brevo, Mailgun, OVH, Cloudflare (`smtp.mx.cloudflare.net:465`, `tls`), a relay. Multipart text+HTML, headers, attachments; the generated `Message-ID` is the provider message id. SMTP 4xx = transient, 5xx = permanent. |
| `agentmail` | `AGENTMAIL_API_KEY`, `EMAIL_FROM` = inbox address | Agent inbox provider. |
| `log` | — | Nothing is sent. One info log line per message, `provider="log"` in metrics. Development and previews only. |

### Failover — `EMAIL_FALLBACK_PROVIDER`

A second email provider, configured with its own variables exactly as if it
were primary (`EMAIL_FALLBACK_PROVIDER=smtp` + `SMTP_*`, or `cloudflare` +
`CLOUDFLARE_*`, or `resend` + `RESEND_API_KEY`). The sender domain must be
verified at both providers.

When the primary answers 429, 5xx or fails at the network level, the message
goes out through the fallback **in the same worker tick** and a breaker
opens: for `EMAIL_FAILOVER_COOLDOWN_SECS` (60) every email uses the fallback,
then the primary is tried again. Permanent errors (bad address, unverified
sender) never fail over: they would fail at any provider. Metrics:
`notifyd_email_failovers_total{from,to,outcome}`; the digest reports the
breaker state and suggests a fallback when none is configured.

## SMS — `SMS_PROVIDER`

Common: `SMS_FROM` (E.164 number or alphanumeric sender), pacing
`SMS_RATE_PER_SEC` (10).

| `SMS_PROVIDER` | Variables |
|---|---|
| `telnyx` | `TELNYX_API_KEY`, `TELNYX_MESSAGING_PROFILE_ID` (optional) |
| `twilio` | `TWILIO_ACCOUNT_SID`, `TWILIO_AUTH_TOKEN` |

## WhatsApp — Telnyx

`TELNYX_WHATSAPP_API_KEY` (falls back to `TELNYX_API_KEY`), `WHATSAPP_FROM`
(WhatsApp-enabled E.164 number), `TELNYX_MESSAGING_PROFILE_ID` (optional),
pacing `WHATSAPP_RATE_PER_SEC` (10). Free-form text is allowed within the
24-hour conversation window; outside it, pass a Meta-approved template in the
job payload: `{"whatsapp": {"template": {...}}}`.

## Web push and FCM

`VAPID_PRIVATE_KEY` or `VAPID_PRIVATE_KEY_PEM`, `VAPID_PUBLIC_KEY`,
`VAPID_SUBJECT` (`mailto:…`) for browsers; `FCM_SERVER_KEY` for FCM legacy.
A subscription the push service rejects permanently (404/410, bad request) is
deleted, so it stops failing every send. Pacing `PUSH_RATE_PER_SEC` (50).

## APNs (iOS, native)

Token-based authentication with the `.p8` key from the Apple Developer
portal, HTTP/2 to `api.push.apple.com`:

| Variable | Purpose |
|---|---|
| `APNS_KEY_ID` | Key id of the `.p8` (10 characters) |
| `APNS_TEAM_ID` | Your Apple team id |
| `APNS_PRIVATE_KEY` | Contents of the `.p8` file (`\n` accepted), or `APNS_PRIVATE_KEY_PATH` to a mounted file |
| `APNS_TOPIC` | The app's bundle identifier |
| `APNS_ENVIRONMENT` | `production` (default) or `sandbox` for development builds |

The four first variables are all-or-nothing; a partial set stops the process
at start-up with the missing names. Register device tokens with
`POST /v1/push-tokens {"subscriber_id", "token", "platform": "apns"}`; a
subscriber can hold APNs, FCM and Web Push tokens at once, every send fans out
to all of them. The provider JWT is re-signed every 50 minutes.

Payload: `subject` → title, `body` → body, `url` forwarded as a custom key.
Extras under `push` in the send request:
`{"badge": 3, "sound": "default" | "none" | "<file>", "thread_id": "orders",
"category": "ORDER", "collapse_id": "order-42", "mutable_content": true,
"background": true, "ttl_secs": 3600, "data": {...}}`. `background` sends a
silent `content-available` push at priority 5.

Apple's answers: `Unregistered`, `BadDeviceToken`, `DeviceTokenNotForTopic`
and `ExpiredToken` delete the device token; `TooManyRequests` pauses the
push lane for `Retry-After`; 5xx and `ExpiredProviderToken` retry (the JWT is
re-signed at once on a 403); anything else (`BadTopic`, `PayloadTooLarge`…)
fails the job and keeps the token, since it is a configuration or payload
problem on our side.

## Telegram, Slack, Discord

Three chat channels with the same lifecycle as the others (preferences,
topics, priorities, pacing, retries).

| Channel | Recipient (`to`, or on the subscriber) | Server variable |
|---|---|---|
| `telegram` | chat id · `data.telegram_chat_id` | `TELEGRAM_BOT_TOKEN`; `TELEGRAM_API_BASE` for a self-hosted Bot API server |
| `slack` | incoming webhook URL, or a channel id · `data.slack` | `SLACK_BOT_TOKEN` (channel ids only) |
| `discord` | webhook URL · `data.discord_webhook` | none |

The message is `subject`, a blank line, `body`; `url` becomes an inline
"Open" button on Telegram, a link on Slack, the embed link on Discord.
`"chat": {"text": "…"}` in the request replaces the built text. A bot the
user blocked, a chat that no longer exists, an archived channel or a deleted
webhook fail the job for good and log why; a 429 pauses the lane for the
time the service asks. Pacing: `TELEGRAM_RATE_PER_SEC` (20),
`SLACK_RATE_PER_SEC` (1), `DISCORD_RATE_PER_SEC` (0.5).

### The digest in your chat

`DIGEST_NOTIFY=telegram:<chat id>` (or `slack:<webhook or channel id>`,
`discord:<webhook url>`) sends the operator digest to that destination every
`DIGEST_NOTIFY_EVERY` (default `1d`) when a finding reaches
`DIGEST_NOTIFY_LEVEL` (default `warning`; `always` sends every time). The
same message on demand: `notifyd digest --to telegram:<chat id>` or
`POST /v1/admin/digest/notify`.

## In-app inbox

No configuration: messages are stored in Postgres and pushed to connected
browsers over SSE (`GET /v1/inbox/:subscriber_id/stream`). Live events fan
out through Postgres `NOTIFY`, so any number of replicas may serve the
stream. An unknown subscriber is a permanent error.

## Public URL

`PUBLIC_URL` (e.g. `https://api-os.philoeparis.com/notifyd`) is the base of the
links this instance hosts: the one-click unsubscribe endpoint `/u/<token>` on
bulk email. Without it, bulk email leaves without `List-Unsubscribe` headers
and the digest says so.

## Operator keys

`ADMIN_API_KEY` (required) opens `/v1/admin/*` and the MCP endpoint with
every tool. `READONLY_API_KEY` (optional) opens the `GET` endpoints, the
metrics and the read-only MCP tools only.

## Worker knobs

| Variable | Default | Meaning |
|---|---|---|
| `WORKER_MAX_ATTEMPTS` | 5 | Attempts before a transiently failing job is marked `failed` (30 s, 2 min, 10 min, 30 min, 2 h with ±20 % jitter). |
| `WORKER_BATCH_SIZE` | 50 | Jobs claimed per poll. |
| `WORKER_POLL_INTERVAL_MS` | 500 | Poll interval. |
| `EMAIL_RATE_PER_SEC`, `SMS_RATE_PER_SEC`, `WHATSAPP_RATE_PER_SEC`, `PUSH_RATE_PER_SEC` | 8, 10, 10, 50 | Provider requests per second per replica (a batch call counts once). `0` disables pacing for that channel. |
| `RATE_LIMIT_PAUSE_SECS` | 2 | Lane pause after a 429 without `Retry-After`. |
| `EMAIL_FALLBACK_PROVIDER`, `EMAIL_FAILOVER_COOLDOWN_SECS` | none, 60 | Second email provider and how long the primary rests after tripping the breaker. |

## Adding a provider

1. Create `src/connectors/<provider>.rs` implementing `Connector`: `channel()`,
   `provider()`, `send()`, and `send_batch()` / `batch_max()` when the provider
   has a bulk endpoint.
2. Build the outcome with `http_outcome(provider, response, message_id)` for
   HTTP APIs: it classifies `429` (with `Retry-After`), `5xx`/`408`
   (transient) and other `4xx` (permanent) in one place.
3. Register it in the factory (`create_email_connector` for email) and in
   `EmailConfig::from_env` / `SmsConfig::from_env`, then document its
   variables in this file and pass them through `docker-compose.yml`.
4. Unit-test the request body you build (see `cloudflare.rs`, `smtp.rs`).
