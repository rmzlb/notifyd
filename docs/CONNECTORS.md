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

## In-app inbox

No configuration: messages are stored in Postgres and pushed to connected
browsers over SSE (`GET /v1/inbox/:subscriber_id/stream`). Live events fan
out through Postgres `NOTIFY`, so any number of replicas may serve the
stream. An unknown subscriber is a permanent error.

## Worker knobs

| Variable | Default | Meaning |
|---|---|---|
| `WORKER_MAX_ATTEMPTS` | 5 | Attempts before a transiently failing job is marked `failed` (30 s, 2 min, 10 min, 30 min, 2 h with ±20 % jitter). |
| `WORKER_BATCH_SIZE` | 50 | Jobs claimed per poll. |
| `WORKER_POLL_INTERVAL_MS` | 500 | Poll interval. |
| `EMAIL_RATE_PER_SEC`, `SMS_RATE_PER_SEC`, `WHATSAPP_RATE_PER_SEC`, `PUSH_RATE_PER_SEC` | 8, 10, 10, 50 | Provider requests per second per replica (a batch call counts once). `0` disables pacing for that channel. |
| `RATE_LIMIT_PAUSE_SECS` | 2 | Lane pause after a 429 without `Retry-After`. |

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
