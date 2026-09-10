"""Contract tests against a fake notifyd (httpx MockTransport): no server needed.

They pin the wire format (paths, methods, snake_case bodies, auth headers) that the
Rust API expects, so a refactor of the client cannot silently change it.
"""

import json

import httpx
import pytest

from notifyd import AsyncNotifyd, Notifyd, NotifydError


def fake_server(recorded):
    def handler(request: httpx.Request) -> httpx.Response:
        recorded.append(request)
        path = request.url.path
        if path == "/v1/send":
            return httpx.Response(200, json={"success": True, "job_ids": ["job-1"], "scheduled_at": "2026-09-10T09:00:00Z", "channels": ["email"]})
        if path == "/v1/batch":
            return httpx.Response(200, json={"success": True, "jobs_created": 2, "jobs_deduplicated": 0, "subscribers": 2, "channels": ["email"]})
        if path == "/v1/jobs/job-1" and request.method == "GET":
            return httpx.Response(200, json={"id": "job-1", "status": "sent", "channel": "email", "attempts": 1, "provider_events": []})
        if path == "/v1/jobs/missing":
            return httpx.Response(404, json={"error": "Job not found"})
        if path == "/v1/send-limited":
            return httpx.Response(429, json={"error": "rate limited"}, headers={"Retry-After": "12"})
        if path == "/v1/inbox/user-1/unread-count":
            return httpx.Response(200, json={"count": 3})
        return httpx.Response(200, json={"success": True})

    return handler


@pytest.fixture
def client():
    recorded = []
    nd = Notifyd("http://notifyd.test/", api_key="nd_test", transport=httpx.MockTransport(fake_server(recorded)))
    yield nd, recorded
    nd.close()


def body(request: httpx.Request):
    return json.loads(request.content)


def test_send_builds_snake_case_body_and_api_key_header(client):
    nd, recorded = client
    result = nd.send(channels=["email", "in_app"], to="alice@example.com", subscriber_id="user-1", subject="Hi", body="Hello {{first_name}}", vars={"first_name": "Alice"}, priority="high", idempotency_key="k1")
    assert result["job_ids"] == ["job-1"]
    req = recorded[0]
    assert req.method == "POST" and req.url.path == "/v1/send"
    assert req.headers["x-api-key"] == "nd_test"
    assert body(req) == {
        "channels": ["email", "in_app"],
        "to": "alice@example.com",
        "subscriber_id": "user-1",
        "subject": "Hi",
        "body": "Hello {{first_name}}",
        "vars": {"first_name": "Alice"},
        "priority": "high",
        "idempotency_key": "k1",
    }


def test_send_requires_a_channel(client):
    nd, _ = client
    with pytest.raises(ValueError):
        nd.send(to="alice@example.com", body="x")


def test_batch(client):
    nd, recorded = client
    result = nd.batch(subscribers=["u1", "u2"], channel="email", template="digest", send_window=False)
    assert result["jobs_created"] == 2
    assert body(recorded[0]) == {"subscribers": ["u1", "u2"], "channel": "email", "template": "digest", "send_window": False}


def test_job_lifecycle_paths(client):
    nd, recorded = client
    assert nd.get_job("job-1")["status"] == "sent"
    nd.cancel_job("job-1")
    nd.retry_job("job-1")
    assert [(r.method, r.url.path) for r in recorded] == [
        ("GET", "/v1/jobs/job-1"),
        ("DELETE", "/v1/jobs/job-1"),
        ("POST", "/v1/jobs/job-1/retry"),
    ]


def test_errors_carry_status_message_and_retry_after(client):
    nd, _ = client
    with pytest.raises(NotifydError) as exc:
        nd.get_job("missing")
    assert exc.value.status == 404 and exc.value.message == "Job not found" and not exc.value.is_retryable
    with pytest.raises(NotifydError) as exc:
        nd._request("POST", "/v1/send-limited")
    assert exc.value.is_rate_limited and exc.value.retry_after == 12.0


def test_subscribers_preferences_templates_workflows(client):
    nd, recorded = client
    nd.upsert_subscriber("user-1", email="a@example.com", first_name="Alice", timezone="Europe/Paris")
    nd.set_preferences("user-1", [{"channel": "sms", "workflow_id": "*", "enabled": False}])
    nd.upsert_template("welcome", channel="email", subject="Welcome {{first_name}}", body="Hi")
    nd.upsert_workflow(
        "welcome-series",
        name="Welcome series",
        trigger_event="user.signup",
        steps=[{"type": "send", "channel": "email", "template": "welcome"}, {"type": "delay", "duration_secs": 86400}],
    )
    nd.trigger_workflow("user.signup", "user-1", {"plan": "pro"})
    nd.list_workflow_runs(status="running", limit=10)
    paths = [(r.method, r.url.path) for r in recorded]
    assert paths == [
        ("POST", "/v1/subscribers"),
        ("PUT", "/v1/subscribers/user-1/preferences"),
        ("POST", "/v1/templates"),
        ("POST", "/v1/workflows"),
        ("POST", "/v1/workflows/trigger"),
        ("GET", "/v1/workflows/runs"),
    ]
    assert body(recorded[0]) == {"id": "user-1", "email": "a@example.com", "first_name": "Alice", "timezone": "Europe/Paris"}
    assert body(recorded[3])["trigger_event"] == "user.signup" and body(recorded[3])["steps"][1] == {"type": "delay", "duration_secs": 86400}
    assert body(recorded[4]) == {"event": "user.signup", "subscriber_id": "user-1", "payload": {"plan": "pro"}}
    assert dict(recorded[5].url.params) == {"status": "running", "limit": "10"}


def test_inbox_uses_subscriber_token_when_given(client):
    nd, recorded = client
    assert nd.unread_count("user-1", subscriber_token="tok")["count"] == 3
    assert recorded[0].headers["authorization"] == "Bearer tok"
    assert "x-api-key" not in recorded[0].headers


def test_suppress_defaults_to_scope_all(client):
    nd, recorded = client
    nd.suppress("bounce@example.com", detail="manual opt-out")
    assert body(recorded[0]) == {"email": "bounce@example.com", "scope": "all", "detail": "manual opt-out"}


@pytest.mark.asyncio
async def test_async_client_mirrors_sync_surface():
    recorded = []
    async with AsyncNotifyd("http://notifyd.test", api_key="nd_test", transport=httpx.MockTransport(fake_server(recorded))) as nd:
        result = await nd.send(channel="email", to="a@example.com", subject="Hi", body="Hello")
        assert result["job_ids"] == ["job-1"]
        assert (await nd.get_job("job-1"))["status"] == "sent"
        with pytest.raises(NotifydError):
            await nd.get_job("missing")
    assert AsyncNotifyd.send.__doc__ == Notifyd.send.__doc__
    assert {n for n in dir(Notifyd) if not n.startswith("_") and n != "close"} <= set(dir(AsyncNotifyd))
