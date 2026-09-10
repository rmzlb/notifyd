"""Sync and async clients. Both expose the same methods; the async one awaits them.

Every method returns the server's JSON as a plain dict (notifyd already speaks
snake_case) and raises `NotifydError` on any non-2xx status.
"""

from __future__ import annotations

import functools
from typing import Any, Dict, Iterable, Mapping, Optional, Sequence, Union

import httpx

from .errors import NotifydError

Json = Dict[str, Any]
Channel = str  # "email" | "sms" | "whatsapp" | "in_app" | "push" | "telegram" | "slack" | "discord"
Priority = Union[str, int]  # "critical" | "high" | "normal" | "low" | "bulk" or 0-100

DEFAULT_TIMEOUT = 10.0


def _compact(payload: Mapping[str, Any]) -> Json:
    """Drop `None` values so optional fields are simply absent on the wire."""
    return {k: v for k, v in payload.items() if v is not None}


def _send_body(
    *,
    channel: Optional[Channel],
    channels: Optional[Sequence[Channel]],
    to: Optional[str],
    subscriber_id: Optional[str],
    template: Optional[str],
    subject: Optional[str],
    body: Optional[str],
    body_html: Optional[str],
    vars: Optional[Mapping[str, Any]],
    scheduled_at: Optional[str],
    idempotency_key: Optional[str],
    priority: Optional[Priority],
    tags: Optional[Sequence[Mapping[str, str]]],
    email_headers: Optional[Mapping[str, str]],
    attachments: Optional[Sequence[Mapping[str, str]]],
    cc: Optional[Sequence[str]],
    reply_to: Optional[str],
    send_window: Optional[Union[Mapping[str, Any], bool]],
    icon: Optional[str],
    url: Optional[str],
    topic: Optional[str],
    track: Optional[Union[Mapping[str, bool], bool]],
    push: Optional[Mapping[str, Any]],
    chat: Optional[Mapping[str, str]],
) -> Json:
    if channel is None and not channels:
        raise ValueError("send() needs `channel` or `channels`")
    return _compact(
        {
            "channel": channel,
            "channels": list(channels) if channels else None,
            "to": to,
            "subscriber_id": subscriber_id,
            "template": template,
            "subject": subject,
            "body": body,
            "body_html": body_html,
            "vars": dict(vars) if vars else None,
            "scheduled_at": scheduled_at,
            "idempotency_key": idempotency_key,
            "priority": priority,
            "tags": list(tags) if tags else None,
            "email_headers": dict(email_headers) if email_headers else None,
            "attachments": list(attachments) if attachments else None,
            "cc": list(cc) if cc else None,
            "reply_to": reply_to,
            "send_window": send_window,
            "icon": icon,
            "url": url,
            "topic": topic,
            "track": track,
            "push": dict(push) if push else None,
            "chat": dict(chat) if chat else None,
        }
    )


def _raise_for_status(response: httpx.Response) -> Json:
    if response.status_code // 100 == 2:
        if not response.content:
            return {}
        try:
            return response.json()
        except ValueError:
            return {"raw": response.text}
    try:
        details: Any = response.json()
    except ValueError:
        details = response.text
    message = details.get("error") if isinstance(details, dict) and details.get("error") else response.reason_phrase or "request failed"
    retry_after = response.headers.get("Retry-After")
    raise NotifydError(
        response.status_code,
        str(message),
        details,
        float(retry_after) if retry_after and retry_after.replace(".", "", 1).isdigit() else None,
    )


class _Base:
    """Request-building shared by the sync and async clients."""

    def __init__(self, base_url: str, api_key: Optional[str] = None, *, timeout: float = DEFAULT_TIMEOUT):
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.timeout = timeout

    def _headers(self, *, subscriber_token: Optional[str] = None) -> Dict[str, str]:
        headers = {"Content-Type": "application/json"}
        if subscriber_token:
            headers["Authorization"] = f"Bearer {subscriber_token}"
        elif self.api_key:
            headers["X-Api-Key"] = self.api_key
        return headers

    def _url(self, path: str) -> str:
        return f"{self.base_url}{path}"


class Notifyd(_Base):
    """Synchronous client.

        nd = Notifyd("http://localhost:3400", api_key="nd_...")
        job = nd.send(channel="email", to="alice@example.com", subject="Hi", body="Hello")
    """

    def __init__(self, base_url: str, api_key: Optional[str] = None, *, timeout: float = DEFAULT_TIMEOUT, transport: Optional[httpx.BaseTransport] = None):
        super().__init__(base_url, api_key, timeout=timeout)
        self._http = httpx.Client(timeout=timeout, transport=transport)

    def close(self) -> None:
        self._http.close()

    def __enter__(self) -> "Notifyd":
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()

    def _request(self, method: str, path: str, *, json: Optional[Json] = None, params: Optional[Mapping[str, Any]] = None, subscriber_token: Optional[str] = None) -> Json:
        response = self._http.request(method, self._url(path), json=json, params=_compact(params or {}), headers=self._headers(subscriber_token=subscriber_token))
        return _raise_for_status(response)

    # ── Health ────────────────────────────────────────────────────────────
    def health(self) -> Json:
        return self._request("GET", "/v1/health")

    # ── Send ──────────────────────────────────────────────────────────────
    def send(
        self,
        *,
        channel: Optional[Channel] = None,
        channels: Optional[Sequence[Channel]] = None,
        to: Optional[str] = None,
        subscriber_id: Optional[str] = None,
        template: Optional[str] = None,
        subject: Optional[str] = None,
        body: Optional[str] = None,
        body_html: Optional[str] = None,
        vars: Optional[Mapping[str, Any]] = None,
        scheduled_at: Optional[str] = None,
        idempotency_key: Optional[str] = None,
        priority: Optional[Priority] = None,
        tags: Optional[Sequence[Mapping[str, str]]] = None,
        email_headers: Optional[Mapping[str, str]] = None,
        attachments: Optional[Sequence[Mapping[str, str]]] = None,
        cc: Optional[Sequence[str]] = None,
        reply_to: Optional[str] = None,
        send_window: Optional[Union[Mapping[str, Any], bool]] = None,
        icon: Optional[str] = None,
        url: Optional[str] = None,
        topic: Optional[str] = None,
        track: Optional[Union[Mapping[str, bool], bool]] = None,
        push: Optional[Mapping[str, Any]] = None,
        chat: Optional[Mapping[str, str]] = None,
    ) -> Json:
        """Queue one notification on one or several channels.

        Without `to`, each channel takes its address from the subscriber (email, phone, `data.telegram_chat_id`,
        `data.slack`, `data.discord_webhook`); a channel with no address is listed in `skipped`. `chat` holds
        `{"text": "exact text", "button": "Open"}` for telegram/slack/discord.

        `push` carries APNs/FCM extras: `{"badge": 3, "sound": "default", "thread_id": "orders",
        "collapse_id": "order-42", "mutable_content": True, "background": False, "ttl_secs": 3600, "data": {...}}`.

        `topic` names the stream ("tips", "billing"); it defaults to the template's topic and subscribers can
        opt out of it per channel. Returns `{"success": true, "job_ids": [...], "scheduled_at": ..., "channels": [...],
        "topic": ..., "skipped": [{"channel", "reason"}]}`; channels the subscriber opted out of create no job.
        Delivery is asynchronous: poll `get_job(job_id)` or subscribe to webhooks.
        """
        payload = _send_body(
            channel=channel, channels=channels, to=to, subscriber_id=subscriber_id, template=template, subject=subject,
            body=body, body_html=body_html, vars=vars, scheduled_at=scheduled_at, idempotency_key=idempotency_key,
            priority=priority, tags=tags, email_headers=email_headers, attachments=attachments, cc=cc, reply_to=reply_to,
            send_window=send_window, icon=icon, url=url, topic=topic, track=track, push=push, chat=chat,
        )
        return self._request("POST", "/v1/send", json=payload)

    def batch(
        self,
        *,
        subscribers: Optional[Sequence[str]] = None,
        segment: Optional[Mapping[str, Any]] = None,
        channel: Optional[Channel] = None,
        channels: Optional[Sequence[Channel]] = None,
        template: Optional[str] = None,
        subject: Optional[str] = None,
        body: Optional[str] = None,
        body_html: Optional[str] = None,
        vars: Optional[Mapping[str, Any]] = None,
        scheduled_at: Optional[str] = None,
        idempotency_key: Optional[str] = None,
        priority: Optional[Priority] = None,
        send_window: Optional[Union[Mapping[str, Any], bool]] = None,
        icon: Optional[str] = None,
        url: Optional[str] = None,
        topic: Optional[str] = None,
        track: Optional[Union[Mapping[str, bool], bool]] = None,
        push: Optional[Mapping[str, Any]] = None,
        chat: Optional[Mapping[str, str]] = None,
    ) -> Json:
        """Send the same message to many subscribers in one call (one job per subscriber and channel).

        Give `subscribers` (ids) or `segment` (a filter, e.g. `{"data": {"plan": "pro"}, "has_email": True}`;
        see `preview_segment`). Returns `{"success": true, "jobs_created": n, "jobs_deduplicated": n,
        "jobs_skipped": n, "subscribers": n, "channels": [...], "topic": ...}`.
        """
        if channel is None and not channels:
            raise ValueError("batch() needs `channel` or `channels`")
        if (subscribers is None) == (segment is None):
            raise ValueError("batch() needs exactly one of `subscribers` or `segment`")
        payload = _compact(
            {
                "subscribers": list(subscribers) if subscribers is not None else None,
                "segment": dict(segment) if segment is not None else None,
                "channel": channel,
                "channels": list(channels) if channels else None,
                "template": template,
                "subject": subject,
                "body": body,
                "body_html": body_html,
                "vars": dict(vars) if vars else None,
                "scheduled_at": scheduled_at,
                "idempotency_key": idempotency_key,
                "priority": priority,
                "send_window": send_window,
                "icon": icon,
                "url": url,
                "topic": topic,
                "track": track,
                "push": dict(push) if push else None,
                "chat": dict(chat) if chat else None,
            }
        )
        return self._request("POST", "/v1/batch", json=payload)

    def preview_segment(self, segment: Mapping[str, Any]) -> Json:
        """`{"count": n, "sample": [ids]}` for a segment, before spending a batch on it."""
        return self._request("POST", "/v1/segments/preview", json=dict(segment))

    # ── Jobs ──────────────────────────────────────────────────────────────
    def get_job(self, job_id: str) -> Json:
        """Status, attempts, provider id, delivery/bounce timestamps and provider events of one job."""
        return self._request("GET", f"/v1/jobs/{job_id}")

    def cancel_job(self, job_id: str) -> Json:
        return self._request("DELETE", f"/v1/jobs/{job_id}")

    def retry_job(self, job_id: str) -> Json:
        """Re-queue a failed job."""
        return self._request("POST", f"/v1/jobs/{job_id}/retry")

    # ── Subscribers ───────────────────────────────────────────────────────
    def upsert_subscriber(
        self,
        subscriber_id: str,
        *,
        email: Optional[str] = None,
        phone: Optional[str] = None,
        first_name: Optional[str] = None,
        last_name: Optional[str] = None,
        locale: Optional[str] = None,
        timezone: Optional[str] = None,
        data: Optional[Mapping[str, Any]] = None,
    ) -> Json:
        payload = _compact(
            {"id": subscriber_id, "email": email, "phone": phone, "first_name": first_name, "last_name": last_name, "locale": locale, "timezone": timezone, "data": dict(data) if data is not None else None}
        )
        return self._request("POST", "/v1/subscribers", json=payload)

    def list_subscribers(self, *, limit: Optional[int] = None, offset: Optional[int] = None, q: Optional[str] = None) -> Json:
        return self._request("GET", "/v1/subscribers", params={"limit": limit, "offset": offset, "q": q})

    def get_subscriber(self, subscriber_id: str) -> Json:
        return self._request("GET", f"/v1/subscribers/{subscriber_id}")

    def delete_subscriber(self, subscriber_id: str) -> Json:
        """Delete the subscriber and everything attached (inbox, tokens, preferences)."""
        return self._request("DELETE", f"/v1/subscribers/{subscriber_id}")

    # ── Preferences ───────────────────────────────────────────────────────
    def get_preferences(self, subscriber_id: str) -> Json:
        return self._request("GET", f"/v1/subscribers/{subscriber_id}/preferences")

    def set_preferences(self, subscriber_id: str, preferences: Iterable[Mapping[str, Any]]) -> Json:
        """Each item: `{"channel": "email" | "*", "topic": "tips", "enabled": False}` (or `"workflow_id"`, or neither
        for the whole channel).

        Everything is enabled by default. Most specific wins: (channel, topic) > ("*", topic) > (channel, workflow)
        > (channel, "*") > ("*", "*").
        """
        return self._request("PUT", f"/v1/subscribers/{subscriber_id}/preferences", json={"preferences": list(preferences)})

    # ── Templates ─────────────────────────────────────────────────────────
    def upsert_template(self, template_id: str, *, channel: Channel, body: str, subject: Optional[str] = None, body_html: Optional[str] = None, topic: Optional[str] = None) -> Json:
        """Create or replace a template. `{{variables}}` are filled from `send(vars=...)`; `topic` is the default topic of sends using it."""
        return self._request("POST", "/v1/templates", json=_compact({"id": template_id, "channel": channel, "subject": subject, "body": body, "body_html": body_html, "topic": topic}))

    def list_templates(self, *, limit: Optional[int] = None, offset: Optional[int] = None) -> Json:
        return self._request("GET", "/v1/templates", params={"limit": limit, "offset": offset})

    def get_template(self, template_id: str) -> Json:
        return self._request("GET", f"/v1/templates/{template_id}")

    def delete_template(self, template_id: str) -> Json:
        return self._request("DELETE", f"/v1/templates/{template_id}")

    # ── Workflows ─────────────────────────────────────────────────────────
    def upsert_workflow(
        self,
        workflow_id: str,
        *,
        name: str,
        trigger_event: str,
        steps: Sequence[Mapping[str, Any]],
        description: Optional[str] = None,
        enabled: Optional[bool] = None,
    ) -> Json:
        """Create or replace a workflow.

        Steps run in order:
          {"type": "send", "channel": "email", "template": "welcome"}
          {"type": "delay", "duration_secs": 86400}
          {"type": "condition", "field": "payload.plan", "operator": "eq", "value": "pro", "on_true": 4}  # or field "inbox.is_read"
          {"type": "digest", "duration_secs": 3600, "channel": "email", "template": "hourly-digest"}
        """
        payload = _compact({"id": workflow_id, "name": name, "description": description, "trigger_event": trigger_event, "steps": list(steps), "enabled": enabled})
        return self._request("POST", "/v1/workflows", json=payload)

    def list_workflows(self) -> Json:
        return self._request("GET", "/v1/workflows")

    def get_workflow(self, workflow_id: str) -> Json:
        return self._request("GET", f"/v1/workflows/{workflow_id}")

    def delete_workflow(self, workflow_id: str) -> Json:
        return self._request("DELETE", f"/v1/workflows/{workflow_id}")

    def trigger_workflow(self, event: str, subscriber_id: str, payload: Optional[Mapping[str, Any]] = None) -> Json:
        """Fire an event; every enabled workflow listening to it starts a run. Returns `{"workflow_runs": [ids]}`."""
        return self._request("POST", "/v1/workflows/trigger", json=_compact({"event": event, "subscriber_id": subscriber_id, "payload": dict(payload) if payload is not None else None}))

    def list_workflow_runs(self, *, status: Optional[str] = None, workflow_id: Optional[str] = None, limit: Optional[int] = None, offset: Optional[int] = None) -> Json:
        return self._request("GET", "/v1/workflows/runs", params={"status": status, "workflow_id": workflow_id, "limit": limit, "offset": offset})

    def cancel_workflow_run(self, run_id: str) -> Json:
        return self._request("DELETE", f"/v1/workflows/runs/{run_id}")

    # ── Suppressions ──────────────────────────────────────────────────────
    def suppress(self, email: str, *, scope: str = "all", detail: Optional[str] = None) -> Json:
        """Stop sending to an address. `scope` is `"all"` or `"marketing"` (transactional still goes)."""
        return self._request("POST", "/v1/suppressions", json=_compact({"email": email, "scope": scope, "detail": detail}))

    def list_suppressions(self) -> Json:
        return self._request("GET", "/v1/suppressions")

    def release_suppression(self, suppression_id: str) -> Json:
        return self._request("DELETE", f"/v1/suppressions/{suppression_id}")

    # ── Inbox (in-app) ────────────────────────────────────────────────────
    def create_subscriber_token(self, subscriber_id: str, *, ttl_secs: Optional[int] = None) -> Json:
        """Short-lived token a browser or mobile app uses to read its own inbox without the API key."""
        return self._request("POST", "/v1/auth/subscriber-token", json=_compact({"subscriber_id": subscriber_id, "ttl_secs": ttl_secs}))

    def get_inbox(self, subscriber_id: str, *, limit: Optional[int] = None, offset: Optional[int] = None, filter: Optional[str] = None, q: Optional[str] = None, subscriber_token: Optional[str] = None) -> Json:
        """`filter` is `"all"` (default), `"unread"` or `"todo"`; `q` searches the text. Returns `{"items", "total", "limit", "offset"}`."""
        return self._request("GET", f"/v1/inbox/{subscriber_id}", params={"limit": limit, "offset": offset, "filter": filter, "q": q}, subscriber_token=subscriber_token)

    def unread_count(self, subscriber_id: str, *, subscriber_token: Optional[str] = None) -> Json:
        return self._request("GET", f"/v1/inbox/{subscriber_id}/unread-count", subscriber_token=subscriber_token)

    def update_inbox_message(self, subscriber_id: str, message_id: str, *, read: Optional[bool] = None, archived: Optional[bool] = None, is_todo: Optional[bool] = None, subscriber_token: Optional[str] = None) -> Json:
        return self._request("PATCH", f"/v1/inbox/{subscriber_id}/{message_id}", json=_compact({"read": read, "archived": archived, "is_todo": is_todo}), subscriber_token=subscriber_token)

    def mark_all_read(self, subscriber_id: str, *, subscriber_token: Optional[str] = None) -> Json:
        return self._request("POST", f"/v1/inbox/{subscriber_id}/read-all", subscriber_token=subscriber_token)

    def create_stream_ticket(self, subscriber_id: str, *, subscriber_token: Optional[str] = None) -> Json:
        """One-shot ticket for the SSE stream `GET /v1/inbox/:id/stream?ticket=...` (browsers cannot set headers on EventSource)."""
        return self._request("POST", f"/v1/inbox/{subscriber_id}/stream-ticket", subscriber_token=subscriber_token)


class AsyncNotifyd(_Base):
    """Asyncio client with the same methods as `Notifyd`, all awaitable."""

    def __init__(self, base_url: str, api_key: Optional[str] = None, *, timeout: float = DEFAULT_TIMEOUT, transport: Optional[httpx.AsyncBaseTransport] = None):
        super().__init__(base_url, api_key, timeout=timeout)
        self._http = httpx.AsyncClient(timeout=timeout, transport=transport)

    async def aclose(self) -> None:
        await self._http.aclose()

    async def __aenter__(self) -> "AsyncNotifyd":
        return self

    async def __aexit__(self, *exc: Any) -> None:
        await self.aclose()

    async def _request(self, method: str, path: str, *, json: Optional[Json] = None, params: Optional[Mapping[str, Any]] = None, subscriber_token: Optional[str] = None) -> Json:
        response = await self._http.request(method, self._url(path), json=json, params=_compact(params or {}), headers=self._headers(subscriber_token=subscriber_token))
        return _raise_for_status(response)


def _make_async(name: str) -> None:
    """Expose a `Notifyd` method on `AsyncNotifyd`.

    Every public method of `Notifyd` is a single `return self._request(...)`, so calling it on an
    `AsyncNotifyd` instance (whose `_request` is a coroutine function) yields an awaitable of the
    same JSON. Keep that one-call shape when adding methods.
    """
    sync_method = getattr(Notifyd, name)

    @functools.wraps(sync_method)
    async def method(self: AsyncNotifyd, *args: Any, **kwargs: Any) -> Json:
        return await sync_method(self, *args, **kwargs)

    setattr(AsyncNotifyd, name, method)


for _name, _value in list(vars(Notifyd).items()):
    if not _name.startswith("_") and callable(_value) and _name != "close":
        _make_async(_name)
