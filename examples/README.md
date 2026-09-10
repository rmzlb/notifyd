# Examples

Every file here runs against a notifyd started with `docker compose up -d`
(see the [Quick Start](../README.md#quick-start)). Set two variables first:

```bash
export NOTIFYD_URL=http://localhost:3400
export NOTIFYD_API_KEY=sk_myapp_…        # from POST /v1/admin/projects
```

| File | What it shows | Needs |
|---|---|---|
| [`send.sh`](send.sh) | One call, email + in-app, then poll the job | `curl`, `jq` |
| [`send.ts`](send.ts) | Same with the TypeScript client | Node 18+, `npm i notifyd-sdk` |
| [`send.py`](send.py) | Same with the Python client | Python 3.9+, `pip install notifyd-sdk` |
| [`campaign.py`](campaign.py) | 10 000 recipients in one `batch` call, bulk lane, recipients' daytime only, unsubscribe handled | Python |
| [`welcome-series.sh`](welcome-series.sh) | A 3-step workflow: welcome, wait a day, nudge unless the user upgraded | `curl` |
| [`inbox.html`](inbox.html) | A browser inbox with live updates over `EventSource` | a browser |
| [`mcp.json`](mcp.json) | Let Claude Code / Cursor operate the instance | an MCP client |

Nothing here needs a real email provider: start notifyd with
`EMAIL_PROVIDER=log` and emails are printed to the server log instead of sent.
