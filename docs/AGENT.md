# Operating notifyd with an agent

notifyd has no admin UI on purpose. Everything an operator needs is an API
call, and the same calls are exposed as **MCP tools** so an AI agent
(Claude Code, Claude Desktop, Cursor, any MCP client) can run the service:
read a digest, investigate, fix, prove.

## Connect an MCP client

Endpoint: `POST https://<your-notifyd>/mcp`, Streamable HTTP, stateless.
Authentication: the instance `ADMIN_API_KEY` as a bearer token.

The server speaks the current MCP revision (**2026-07-28**: `server/discover`,
`_meta` on every request, `Mcp-Method`/`Mcp-Name` headers, `resultType`,
cache hints) and the legacy `initialize` handshake (2024-11-05 → 2025-11-25)
on the same endpoint, so both new and old clients work. Every tool carries
annotations (`readOnlyHint`, `destructiveHint`, `idempotentHint`,
`openWorldHint`) and an `outputSchema`; clients use the read-only hint to
skip confirmations on `digest`, `list_jobs`, `get_job`, `list_projects`,
`list_suppressions`. `retry_job` and `send_test` are flagged as sending real
messages. Tool calls are rate limited (600/min) and written to the audit log
(tool name, argument keys, outcome, latency; never argument values).

Claude Code (`.mcp.json` in the project, or `~/.claude.json`):

```json
{
  "mcpServers": {
    "notifyd-philoe": {
      "type": "http",
      "url": "https://api-os.philoeparis.com/notifyd/mcp",
      "headers": { "Authorization": "Bearer ${NOTIFYD_ADMIN_API_KEY}" }
    }
  }
}
```

One entry per company instance (`notifyd-craie`, `notifyd-sqare`…). The key
is read from the environment, never written in the file.

## Tools

| Tool | Use it when |
|---|---|
| `digest` | "How are notifications doing?" Findings first, ranked `critical` → `warning` → `info`, each with the action to take. Then queue, outcomes per channel/provider, failure reasons, retries waiting, latency p50/p95, deliverability, projects. `window` 1h…30d, `format` markdown or json. |
| `list_jobs` | Investigate: by status, channel, project, recipient, since. Recipients are masked. |
| `get_job` | One job: attempts, provider, provider message id, delivery events, error. |
| `retry_job` | After fixing a cause. Re-queues a `failed`/`cancelled` job with a fresh attempt budget. |
| `cancel_job` | Stop a pending or retrying job. |
| `list_projects`, `update_project` | Sender identity (`from_email`, `from_name`), channels, inbound rate limit. Keys are never touched. |
| `list_suppressions`, `add_suppression`, `release_suppression` | The do-not-send list: bounces, complaints and commercial unsubscribes land there automatically; block (`scope` all or marketing) or release an address by hand. |
| `send_test` | Prove a channel end to end: enqueues a high-priority `category=test` message and returns the job id. |

The same operations exist as REST endpoints under `/v1/admin/*` (see
`docs/API.md`): `GET /v1/admin/digest?window=24h&format=markdown`,
`GET /v1/admin/jobs`, `POST /v1/admin/jobs/:id/retry`,
`PATCH /v1/admin/projects/:id`, `GET|POST /v1/admin/suppressions`,
`DELETE /v1/admin/suppressions/:id`. Project keys get `POST /v1/jobs/:id/retry`
and `POST /v1/suppressions` for their own scope.

## How the digest decides what to flag

| Finding | Threshold | Why |
|---|---|---|
| No email provider / provider `log` | always | nothing leaves the instance |
| Lane paused | any | a provider answered 429 |
| Oldest waiting job | > 5 min (urgent, normal), > 60 min (bulk) | the worker is stuck or paced out |
| Failed jobs | ≥ 2 % warning, ≥ 10 % critical, of terminal jobs in the window | permanent errors need a fix at the caller |
| Bounce rate | ≥ 2 % warning, ≥ 5 % critical | above 5 % providers throttle or suspend senders |
| Complaints | any | content or frequency problem |
| Project without `from_email` | always | emails leave with the instance default identity |

When none applies the digest says so ("All quiet: N sent, 0 failed").

## A typical session

1. `digest` → read the findings.
2. For a failure group: `list_jobs(status="failed")`, then `get_job` on the
   sample id. The `error` says whether it is permanent (fix the address, the
   sender, the template) or was transient (already retried by the worker).
3. Fix the cause (`update_project` for a sender, `release_suppression` for a
   wrongly blocked address), then `retry_job`.
4. `send_test` on the channel to confirm, `get_job` until `sent`.
5. `digest` again: the finding should be gone.

## Scope and safety

- Admin key = full control of one instance (one company). Give an agent the
  key of the instance it operates, nothing else.
- Tool errors come back as results with `isError: true` and a sentence the
  model can act on; JSON-RPC errors are reserved for malformed requests.
- Recipients are masked in every listing. `get_job` shows the masked
  recipient too; the raw address is only in the database.
- `send_test` enqueues a real message: use an address you own.
