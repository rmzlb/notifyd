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
`SMS_RATE_PER_SEC` (10). A message can carry its own sender in
`{"sms": {"from": "…"}}` on `/v1/send` and `/v1/batch`, for a project that
sends under several identities. An alphanumeric sender (1 to 11 letters,
digits or spaces, at least one letter) is one-way; with Telnyx it needs
`TELNYX_MESSAGING_PROFILE_ID`, the profile that carries it, and the API
refuses it at enqueue (`422 channel_not_configured`) while that is missing.

| `SMS_PROVIDER` | Variables |
|---|---|
| `telnyx` | `TELNYX_API_KEY`, `TELNYX_MESSAGING_PROFILE_ID` (optional) |
| `twilio` | `TWILIO_ACCOUNT_SID`, `TWILIO_AUTH_TOKEN` |
| `smsmode` | `SMSMODE_API_KEY`, `SMSMODE_WEBHOOK_SECRET` (delivery status) |

With smsmode, the API key belongs to one smsmode channel and sends through it:
take it from a `TRANSACTIONAL` channel, whose messages carry no `STOP` and no
sending-hours limit (a channel's traffic type cannot change after it is
created). A sender is at most 11 characters, an E.164 number written without
its `+`; a longer one is refused at enqueue. Nobody can answer an
alphanumeric sender: for replies, send from a number smsmode rents you or from
the channel's short code (`36034`), which smsmode shows as one of its shared
short codes (38047, 38164…, not always the same one).
smsmode reads a text as GSM 7-bit and silently replaces what that alphabet
lacks (`’` becomes `'`, an emoji `?`), so notifyd sends any text outside
GSM 03.38 as `UNICODE` (70 characters per part instead of 160), as Twilio
does on its own. The job id is the message's `refClient`: smsmode copies it
onto the replies to that message, so a project receiving them (channel
webhook « Réception de SMS (MO) ») can tell which notifyd job was answered.

### Delivery status (Twilio)

`sent` on an SMS job only means the provider accepted the API call. With
`SMS_PROVIDER=twilio` and `PUBLIC_URL` set, every message asks Twilio for
status callbacks on `PUBLIC_URL/webhooks/twilio/status?job=<job id>`, and the
job records what the carrier did:

| Twilio status | Job | Webhook event |
|---|---|---|
| `delivered` | `delivered_at` stamped, status unchanged (`sent`) | `job.delivered` |
| `undelivered`, `failed` | the job turns `bounced`, from `sent` or from `processing` when the callback beats the worker's write of the send: `bounced_at` stamped, `error` = `twilio <ErrorCode>` (e.g. `twilio 30003`), or `twilio <status>` when Twilio gives no code | `job.bounced` |
| any other (`queued`, `sending`, Twilio's own `sent`…) | unchanged | none |

The first terminal status wins: a late, out-of-order or repeated callback
changes nothing and fires no event again. Twilio signs each callback with
`TWILIO_AUTH_TOKEN` over the URL it called; notifyd rebuilds that URL from
`PUBLIC_URL`, never from the `Host` header, so `PUBLIC_URL` must be the exact
base Twilio reaches, path prefix included, or every callback is refused
(`403`) and the jobs stay `sent`. Only the port may differ, as with Twilio's
SDK validators: notifyd also tries the URL with the default port added (`:443`,
or `:80` on `http`) or, when `PUBLIC_URL` names a port, without it. `To`,
`From` and the message text in a callback are never logged or stored. A
callback body over 64 KiB is refused (`413`).

Without `PUBLIC_URL`, the request to Twilio carries no `StatusCallback`, SMS
jobs stay `sent`, the callback route answers `503` and the digest says what is
missing. The route checks callbacks whenever `TWILIO_AUTH_TOKEN` is set, also
under `SMS_PROVIDER=smsmode`. Telnyx statuses are not ingested: Telnyx jobs
stay `sent`.

### Delivery status (smsmode)

With `SMS_PROVIDER=smsmode`, `PUBLIC_URL` and `SMSMODE_WEBHOOK_SECRET` set,
every message asks smsmode for its delivery reports on
`PUBLIC_URL/webhooks/smsmode/status?job=<job id>` (`callbackUrlStatus`, at
most 255 characters: a longer one is left out with a warning), and the job
records them with the same rules as Twilio's:

| smsmode `status.value` | Job | Webhook event |
|---|---|---|
| `DELIVERED` | `delivered_at` stamped, status unchanged (`sent`) | `job.delivered` |
| `UNDELIVERABLE`, `UNDELIVERED` | the job turns `bounced`: `error` = `smsmode <status.detail>` (e.g. `smsmode SPAM`, `smsmode INSUFFICIENT_CREDIT`), or `smsmode <status.value>` without a detail | `job.bounced` |
| `SCHEDULED`, `ENROUTE` | unchanged | none |

A send smsmode answers `201` with a message already `UNDELIVERABLE` or
`UNDELIVERED` fails the job at once (`smsmode UNDELIVERABLE
ROUTE_NOT_AVAILABLE`). smsmode signs each report with HMAC-SHA256 of the raw
body under the secret set in its interface (Paramètres > Webhooks, signature
cryptographique) and sends `X-Smsmode-256: sha256=<hex>`; notifyd compares it
in constant time and refuses anything else (`403`). Set the same value in
smsmode and in `SMSMODE_WEBHOOK_SECRET`. Without the secret notifyd asks for
no report (smsmode would retry each refused one for a day), smsmode jobs stay
`sent`, the route answers `503` and the digest says so. The route checks
reports whenever the secret is set, also after a switch back to Twilio. A body
over 64 KiB is refused (`413`); the recipient, sender and text it carries are
never logged or stored.

### Switching provider

`SMS_PROVIDER` picks the provider every new SMS leaves through; change it and
redeploy. Keep the previous provider's variables for a day or two: Twilio
(`TWILIO_AUTH_TOKEN`) and smsmode (`SMSMODE_WEBHOOK_SECRET`) still report on
the messages they took before the switch, and notifyd records those reports on
their jobs. A report only ever changes a job its own provider sent. Senders
are the provider's own: a Twilio number cannot send through smsmode, so move
`SMS_FROM` and the projects' `sms.from` with the switch.

## RCS — `RCS_PROVIDER` (smsmode)

`RCS_PROVIDER=smsmode` with `SMSMODE_RCS_API_KEY` (or `SMSMODE_API_KEY` when
the RCS agent channel shares the key), optional `RCS_FROM` (1 to 11 chars),
`SMSMODE_WEBHOOK_SECRET` (same HMAC-SHA256 secret as SMS delivery reports),
pacing `RCS_RATE_PER_SEC` (10).

`channel: "rcs"` sends via `POST https://rest.smsmode.com/rcs/v1/messages` with
application-friendly defaults:

- **Automatic `BASIC` vs `TEXT` tier selection:** plain messages `<= 160`
  characters with no buttons are sent as `body.type = "BASIC"` (full UTF-8 and
  emojis included, billed at the Basic RCS rate); longer messages (`<= 1000`
  characters) or messages with suggestion buttons upgrade automatically to
  `body.type = "TEXT"`.
- **Action buttons without boilerplate:**
  - Top-level `"url"` adds an `OPEN_URL` suggestion button (label defaults to
    `"Open"`, or `"rcs": {"button": "Confirmer"}`).
  - `"rcs": {"replies": ["Oui", "Non"]}` adds quick-reply (`REPLY`) buttons
    (`<= 25` chars each).
  - `"rcs": {"dial": "+33140000000"}` or `{"dial": {"text": "Appeler le centre", "phone": "+33140000000"}}`
    adds a click-to-call (`DIAL_PHONE`) button.
  - Every suggestion gets its required `postbackData` filled automatically when
    omitted.
- **Rich cards and carousels:**
  - `"rcs": {"card": {"file_url": "https://…/map.png", "replies": ["Confirmer", "Annuler"]}}`
    builds a `STANDALONE_RICH_CARD` using the notification's `subject` as
    `title` (`<= 200` chars), `body` as `description` (`<= 2000` chars), and
    `url` as an `OPEN_URL` button.
  - `"rcs": {"carousel": {"width": "MEDIUM", "cards": [...]}}` builds a
    `CAROUSEL_RICH_CARD` (`2..=10` cards).
  - `"rcs": {"file_url": "https://…/devis.pdf"}` sends a `FILE` message, and
    `"rcs": {"body": {...}}` passes a raw smsmode RCS `body` object through.
- **Delivery and read receipts:** RCS reports share `/webhooks/smsmode/status`:
  `DELIVERED` and `READ` stamp `delivered_at` (`job.delivered`), while
  `UNDELIVERABLE` and `UNDELIVERED` (including `PHONE_NOT_SUITABLE` /
  `NETWORK_NOT_SUITABLE` when a handset does not support RCS) mark the job
  `bounced` (`job.bounced`).

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

### Telegram forum topics

A Telegram group with topics enabled needs the topic id, or the message lands
in **General**. Telegram does not reject a message that omits it, so the
failure is silent and looks like "the notification never arrived" to whoever
watches the topic:

```bash
curl -X POST http://localhost:3400/v1/send \
  -H "x-api-key: sk_…" -H 'content-type: application/json' \
  -d '{
    "channel": "telegram",
    "to": "-1003803857627",
    "subject": "SQX-304 created",
    "body": "New issue in Square.",
    "chat": { "telegram_thread_id": 2 }
  }'
```

The id is a positive integer (a decimal string is accepted); anything else is
ignored rather than coerced, because a wrong topic is as invisible as none.
The topic id is the `message_thread_id` of any message in that topic.

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
bulk email, and the SMS status callbacks `/webhooks/twilio/status` and
`/webhooks/smsmode/status`. Without it, bulk email leaves without
`List-Unsubscribe` headers, Twilio and smsmode SMS get no delivery status, and
the digest says so.

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
