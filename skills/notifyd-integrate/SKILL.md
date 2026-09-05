---
name: notifyd-integrate
description: Send notifications from an application through notifyd's REST API (POST /v1/send, /v1/batch, /v1/schedule), choose channels, priority, idempotency keys, tags, send windows, attachments, and follow delivery with GET /v1/jobs/:id. Use when writing code that emails, texts or notifies users through a self-hosted notifyd instance, or when debugging why a send call was rejected.
license: MIT
metadata:
  author: rmzlb
  version: "1.0"
---

# Integrating an application with notifyd

Auth: project API key in `x-api-key`. Base URL: the instance (`/v1/...`).
Full reference: `docs/API.md`; machine-readable summary: `docs/llms.txt`.

## Sending

`POST /v1/send` with one channel (`channel`) or several (`channels`):

```json
{
  "channel": "email",
  "to": "jane@example.com",
  "subscriber_id": "user_42",
  "subject": "Your order #1042 shipped",
  "body": "Plain text version",
  "body_html": "<p>HTML version</p>",
  "idempotency_key": "order-1042-shipped",
  "priority": "normal",
  "tags": [{"name": "category", "value": "transactional"}, {"name": "template", "value": "order-shipped"}],
  "reply_to": "support@example.com",
  "email_headers": {"X-Entity-Ref-ID": "1042"}
}
```

Rules that keep production clean:

- **Always** set `idempotency_key` for anything triggered by an event (order,
  payment, signup): `<event>-<entity-id>`. A retried call returns the same
  job instead of sending twice.
- **Priority**: `critical` for security codes and payment confirmations,
  `normal` (default) for transactional, `bulk` for campaigns. Bulk never
  delays transactional. A `category` tag of `campaign`, `marketing` or
  `newsletter` makes the job bulk automatically.
- **Tags**: `category` (transactional | campaign | …) and `template`
  (your template name) — they power `template_metrics` and the digest.
- **Bulk email** gets `List-Unsubscribe` one-click headers automatically;
  do not add your own unless you host the unsubscribe page yourself.
- **Send window**: bulk waits for the recipient's daytime when the project
  has `settings.send_window` (or pass `send_window` in the request). Set the
  recipient's timezone with `POST /v1/subscribers {"id", "timezone"}`.
- Attachments: `"attachments": [{"filename", "content" (base64), "content_type"}]`,
  single-send only.

`POST /v1/batch` fans one message out to many `subscribers` (in-app or email
via subscriber records); pass `idempotency_key` to make the whole fan-out
replay-safe. `POST /v1/schedule` for a future `scheduled_at`.

## Following a job

The send response returns job ids. `GET /v1/jobs/:id` gives `status`
(`pending` → `processing` → `sent` | `retry` | `failed`), `provider`,
`provider_message_id`, `delivered_at`, `error`. `sent` means the provider
accepted the message; delivery events arrive later through webhooks.

`failed` is final: notifyd already retried transient errors (5 attempts) or
the error was permanent (bad address, unverified sender, suppressed
recipient). Fix the cause, then `POST /v1/jobs/:id/retry`.

## What not to do

- Do not loop on `GET /v1/jobs/:id` faster than every 2 s.
- Do not send marketing to an address that appears in
  `GET /v1/suppressions`: it is blocked server-side anyway, and each attempt
  is a failed job in the digest.
- Do not put the API key in client-side code; the in-app inbox uses
  subscriber JWTs (`POST /v1/auth/subscriber-token`) and one-time stream
  tickets instead.
