# notifyd — Python client

Python client for [notifyd](https://github.com/rmzlb/notifyd), the self-hosted notification
server: email, SMS, WhatsApp, push and in-app from one API, with queue, retries, provider
failover and send windows handled server-side.

```bash
pip install notifyd-sdk
```

```python
from notifyd import Notifyd

nd = Notifyd("http://localhost:3400", api_key="nd_...")

# One call, several channels. Delivery is queued and retried by the server.
result = nd.send(
    channels=["email", "in_app"],
    subscriber_id="user-42",
    subject="Your order shipped",
    body="Hi {{first_name}}, parcel {{parcel}} is on its way.",
    vars={"first_name": "Alice", "parcel": "FR-2041"},
)
print(result["job_ids"])

# Ask what happened to it.
print(nd.get_job(result["job_ids"][0])["status"])   # pending -> processing -> sent | retry | failed
```

Async, same methods:

```python
from notifyd import AsyncNotifyd

async with AsyncNotifyd("http://localhost:3400", api_key="nd_...") as nd:
    await nd.send(channel="sms", to="+33612345678", body="Code: 482913", priority="critical")
```

## What you can do

| Area | Methods |
| --- | --- |
| Send | `send`, `batch` (a list of ids or a `segment` filter, deduplicated by `idempotency_key`), `preview_segment` |
| Jobs | `get_job`, `cancel_job`, `retry_job` |
| Subscribers | `upsert_subscriber`, `get_subscriber`, `list_subscribers`, `delete_subscriber` |
| Preferences | `get_preferences`, `set_preferences` (per channel and per topic or workflow; `"*"` = all) |
| Templates | `upsert_template`, `get_template`, `list_templates`, `delete_template` |
| Workflows | `upsert_workflow`, `trigger_workflow`, `list_workflow_runs`, `cancel_workflow_run`, … |
| Suppressions | `suppress`, `list_suppressions`, `release_suppression` |
| In-app inbox | `create_subscriber_token`, `get_inbox`, `unread_count`, `update_inbox_message`, `mark_all_read`, `create_stream_ticket` |

Every method returns the server's JSON as a `dict`. Any non-2xx answer raises `NotifydError`
with `.status`, `.message`, `.details` and `.retry_after` (seconds, when the server sent one).

```python
from notifyd import NotifydError

try:
    nd.send(channel="email", to="nobody", body="x")
except NotifydError as e:
    if e.is_retryable:        # 429 or 5xx
        ...
    print(e.status, e.message)
```

## Topics

```python
nd.upsert_template("weekly-tips", channel="email", subject="Tip of the week", body="…", topic="tips")
nd.set_preferences("user-42", [{"channel": "email", "topic": "tips", "enabled": False}])

result = nd.send(channels=["email", "in_app"], subscriber_id="user-42", template="weekly-tips")
result["channels"]  # ["in_app"]: the email was not even queued
result["skipped"]   # [{"channel": "email", "reason": "subscriber opted out of topic 'tips' on email"}]
```

## Workflows in one screen

```python
nd.upsert_template("welcome", channel="email", subject="Welcome {{first_name}}", body="Glad you're here.")
nd.upsert_template("nudge", channel="email", subject="Need a hand?", body="Reply to this email.")

nd.upsert_workflow(
    "welcome-series",
    name="Welcome series",
    trigger_event="user.signup",
    steps=[
        {"type": "send", "channel": "email", "template": "welcome"},
        {"type": "delay", "duration_secs": 86400},
        {"type": "condition", "field": "payload.plan", "operator": "eq", "value": "pro", "on_true": 4},
        {"type": "send", "channel": "email", "template": "nudge"},
    ],
)

nd.trigger_workflow("user.signup", subscriber_id="user-42", payload={"plan": "pro"})
```

Step types: `send`, `delay` (`duration_secs`), `condition` (`field` is `inbox.is_read` or `payload.<key>`,
`operator` in `eq|neq|gt|lt`, `value`, jump with `on_true` / `on_false` step indexes) and `digest`
(collect events for `duration_secs`, then send one message).

The full HTTP contract is in [`docs/llms.txt`](https://github.com/rmzlb/notifyd/blob/main/docs/llms.txt).

## Development

```bash
uv venv && uv pip install -e . pytest pytest-asyncio
.venv/bin/python -m pytest
```

Tests run against a fake server (`httpx.MockTransport`) and pin the wire format the Rust API expects.
