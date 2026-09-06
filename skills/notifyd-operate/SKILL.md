---
name: notifyd-operate
description: Operate a running notifyd instance (self-hosted notification queue for email, SMS, WhatsApp, in-app, push) through its MCP tools or REST admin API. Use when asked how notifications are doing, why an email did not arrive, to retry or cancel a job, to block or release an address, to set a project's sender or send window, or to prove a channel with a test message.
license: MIT
metadata:
  author: rmzlb
  version: "1.0"
---

# Operating notifyd

notifyd has no dashboard. You are the operator: read the digest, act with the
tools, read the digest again.

## Connect

MCP endpoint: `POST <instance>/mcp`, `Authorization: Bearer <key>`.
Two keys exist: the admin key (everything) and an optional read-only key
(digest, listings, metrics). With the read-only key, mutating tools answer
`isError: true` and say so; do not retry them, report the limitation.

Claude Code: `.mcp.json` →
`{"mcpServers":{"notifyd":{"type":"http","url":"<instance>/mcp","headers":{"Authorization":"Bearer ${NOTIFYD_ADMIN_API_KEY}"}}}}`

## Procedure

1. `digest` (window `24h` by default; `7d` for a weekly review). Read the
   **findings** first: each has a severity and the action to take. "All
   quiet" means stop here unless the user asked for details.
2. For a failure finding: `list_jobs` with `status: "failed"` (add
   `project_id`, `channel`, `since` to narrow), then `get_job` on the sample
   id from the finding. Read `error`:
   - `HTTP 4xx` / `invalid recipient` / `unverified sender` → permanent, the
     caller must fix the address, the sender (`update_project` with
     `from_email`) or the template. Then `retry_job`.
   - `suppression-list: …` → the address bounced, complained or unsubscribed.
     Do not release a bounce or complaint unless the user confirms the address
     is valid again. A marketing-scope suppression is a commercial
     unsubscribe: never override it for marketing.
   - `HTTP 5xx` / `transport error` → transient; the worker already retried
     (5 attempts over ~2 h 40) or failed over to the fallback provider. Retry
     only if the provider is back.
3. For a queue finding ("oldest job waiting"): check `retries_waiting` and
   `paused_lanes` in the digest (one entry per channel). A paused channel means a 429 from the provider;
   it clears itself. A stuck queue with nothing paused means the worker is
   down: escalate to the operator, you cannot restart it.
4. For deliverability findings (bounce rate, complaints): `template_metrics`
   with `bucket: "1d"` shows which template drives them. Suggest list
   hygiene; do not send more.
5. To prove a channel after a fix: `send_test` to an address the user owns,
   then `get_job` until `status: "sent"` and `provider_message_id` is set.
6. Close with `digest` again and report: what was wrong, what you changed,
   what remains (with the finding text).

## Rules

- Recipients are masked in listings; never try to reconstruct them.
- `retry_job` and `send_test` send real messages. Say so before calling them
  when the user did not explicitly ask.
- Never release a suppression created by a bounce or a complaint without an
  explicit instruction that names the address.
- Numbers you report must come from the digest or metrics of this call, not
  from memory.
