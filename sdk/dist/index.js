// sdk/index.ts
var NotifydError = class extends Error {
  status;
  details;
  constructor(message, status, details = null) {
    super(message);
    this.name = "NotifydError";
    this.status = status;
    this.details = details;
  }
};
function normalizeUrl(url) {
  return url.replace(/\/+$/, "");
}
function assertApiKey(apiKey) {
  if (!apiKey) {
    throw new Error("notifyd apiKey is required for this method");
  }
  return apiKey;
}
function assertInboxAuth(apiKey, subscriberToken) {
  if (subscriberToken) {
    return { Authorization: `Bearer ${subscriberToken}` };
  }
  if (apiKey) {
    return { "X-Api-Key": apiKey };
  }
  throw new Error("notifyd apiKey or subscriberToken is required for this method");
}
async function parseResponse(res) {
  const contentType = res.headers.get("content-type") || "";
  const isJson = contentType.includes("application/json");
  const body = isJson ? await res.json() : await res.text();
  if (!res.ok) {
    const message = typeof body === "object" && body && "error" in body && typeof body.error === "string" ? body.error : `notifyd request failed (${res.status})`;
    throw new NotifydError(message, res.status, body ?? null);
  }
  return body;
}
function buildQuery(params) {
  if (!params) return "";
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value === void 0 || value === null || value === "") continue;
    search.set(key, String(value));
  }
  const query = search.toString();
  return query ? `?${query}` : "";
}
function mapSubscriber(raw) {
  return {
    id: String(raw.id),
    email: asOptionalString(raw.email),
    phone: asOptionalString(raw.phone),
    firstName: asOptionalString(raw.first_name),
    lastName: asOptionalString(raw.last_name),
    locale: asOptionalString(raw.locale),
    data: asRecord(raw.data) ?? void 0,
    projectId: asOptionalString(raw.project_id),
    createdAt: asOptionalString(raw.created_at)
  };
}
function mapInboxNotification(raw) {
  return {
    id: String(raw.id),
    body: String(raw.body ?? ""),
    icon: String(raw.icon ?? "bell"),
    url: asOptionalString(raw.url) ?? null,
    data: asRecord(raw.data),
    isRead: Boolean(raw.is_read),
    readAt: asOptionalString(raw.read_at) ?? null,
    isTodo: Boolean(raw.is_todo),
    createdAt: String(raw.created_at ?? "")
  };
}
function mapInboxResponse(raw) {
  const items = Array.isArray(raw.items) ? raw.items.map((item) => mapInboxNotification(item)) : [];
  return {
    items,
    total: Number(raw.total ?? items.length),
    limit: Number(raw.limit ?? items.length),
    offset: Number(raw.offset ?? 0)
  };
}
function mapListResponse(raw, mapper) {
  const items = Array.isArray(raw.items) ? raw.items.map((item) => mapper(item)) : [];
  return {
    items,
    total: Number(raw.total ?? items.length),
    limit: Number(raw.limit ?? items.length),
    offset: Number(raw.offset ?? 0)
  };
}
function asOptionalString(value) {
  return typeof value === "string" ? value : void 0;
}
function asRecord(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  return value;
}
function stepToWire(step) {
  switch (step.type) {
    case "send":
      return { type: "send", channel: step.channel, template: step.template, subject: step.subject, body: step.body, body_html: step.bodyHtml };
    case "delay":
      return { type: "delay", duration_secs: step.durationSecs };
    case "condition":
      return { type: "condition", field: step.field, operator: step.operator, value: step.value, on_true: step.onTrue, on_false: step.onFalse };
    case "digest":
      return { type: "digest", duration_secs: step.durationSecs, channel: step.channel, template: step.template, subject: step.subject, body: step.body };
  }
}
function stepFromWire(w) {
  const str = (k) => typeof w[k] === "string" ? w[k] : void 0;
  const num = (k) => typeof w[k] === "number" ? w[k] : void 0;
  switch (w.type) {
    case "delay":
      return { type: "delay", durationSecs: num("duration_secs") ?? 0 };
    case "condition":
      return { type: "condition", field: str("field") ?? "", operator: str("operator") ?? "eq", value: w.value, onTrue: num("on_true"), onFalse: num("on_false") };
    case "digest":
      return { type: "digest", durationSecs: num("duration_secs") ?? 0, channel: str("channel") ?? "email", template: str("template"), subject: str("subject"), body: str("body") };
    default:
      return { type: "send", channel: str("channel") ?? "email", template: str("template"), subject: str("subject"), body: str("body"), bodyHtml: str("body_html") };
  }
}
function workflowFromWire(w) {
  return {
    id: w.id,
    name: w.name,
    description: w.description ?? void 0,
    triggerEvent: w.trigger_event,
    steps: (w.steps ?? []).map(stepFromWire),
    enabled: w.enabled,
    createdAt: w.created_at
  };
}
function runFromWire(r) {
  return { id: r.id, workflowId: r.workflow_id, subscriberId: r.subscriber_id, status: r.status, currentStep: r.current_step, resumeAt: r.resume_at ?? null, createdAt: r.created_at };
}
function jobFromWire(j) {
  return {
    id: j.id,
    status: j.status,
    channel: j.channel,
    subscriberId: j.subscriber_id ?? null,
    recipient: j.recipient ?? null,
    templateId: j.template_id ?? null,
    priority: j.priority,
    attempts: j.attempts ?? 0,
    maxAttempts: j.max_attempts,
    provider: j.provider ?? null,
    providerMessageId: j.provider_message_id ?? null,
    scheduledAt: j.scheduled_at ?? null,
    createdAt: j.created_at,
    sentAt: j.sent_at ?? null,
    deliveredAt: j.delivered_at ?? null,
    bouncedAt: j.bounced_at ?? null,
    error: j.error ?? null,
    providerEvents: (j.provider_events ?? []).map((e) => ({
      provider: e.provider,
      type: e.type,
      occurredAt: e.occurred_at,
      providerMessageId: e.provider_message_id ?? null,
      recipients: e.recipients,
      error: e.error
    }))
  };
}
function templateFromWire(t) {
  return { id: t.id, channel: t.channel, subject: t.subject ?? void 0, body: t.body, bodyHtml: t.body_html ?? void 0 };
}
function suppressionFromWire(s) {
  return { id: s.id, email: s.email, reason: s.reason, detail: s.detail ?? null, createdAt: s.created_at, releasedAt: s.released_at ?? null };
}
function createNotifydClient(config) {
  const baseUrl = normalizeUrl(config.url);
  const fetchImpl = config.fetch ?? globalThis.fetch;
  if (!fetchImpl) {
    throw new Error("notifyd fetch implementation is not available");
  }
  async function request(path, options = {}) {
    const authHeaders = options.auth === "none" ? {} : options.auth === "apiKey" ? { "X-Api-Key": assertApiKey(config.apiKey) } : assertInboxAuth(config.apiKey, config.subscriberToken);
    const res = await fetchImpl(`${baseUrl}${path}${buildQuery(options.query)}`, {
      method: options.method ?? "GET",
      headers: {
        ...authHeaders,
        "Content-Type": "application/json"
      },
      body: options.body === void 0 ? void 0 : JSON.stringify(options.body)
    });
    return parseResponse(res);
  }
  return {
    async send(input) {
      var _a;
      const response = await request("/v1/send", {
        method: "POST",
        auth: "apiKey",
        body: {
          channel: input.channel,
          channels: input.channels,
          to: input.to,
          subscriber_id: input.subscriberId,
          template: input.template,
          subject: input.subject,
          body: input.body,
          body_html: input.bodyHtml,
          vars: input.vars,
          scheduled_at: input.scheduledAt,
          idempotency_key: input.idempotencyKey,
          icon: input.icon,
          url: input.url,
          attachments: (_a = input.attachments) == null ? void 0 : _a.map((a) => ({
            filename: a.filename,
            content: a.content,
            content_type: a.contentType
          })),
          cc: input.cc,
          reply_to: input.replyTo
        }
      });
      return {
        success: response.success,
        jobIds: response.job_ids,
        scheduledAt: response.scheduled_at,
        channels: response.channels
      };
    },
    async batch(input) {
      const response = await request("/v1/batch", {
        method: "POST",
        auth: "apiKey",
        body: {
          channel: input.channel,
          channels: input.channels,
          subscribers: input.subscribers,
          template: input.template,
          subject: input.subject,
          body: input.body,
          body_html: input.bodyHtml,
          vars: input.vars,
          scheduled_at: input.scheduledAt
        }
      });
      return {
        success: response.success,
        jobsCreated: response.jobs_created,
        subscribers: response.subscribers,
        channels: response.channels
      };
    },
    async upsertSubscriber(input) {
      const response = await request("/v1/subscribers", {
        method: "POST",
        auth: "apiKey",
        body: {
          id: input.id,
          email: input.email,
          phone: input.phone,
          first_name: input.firstName,
          last_name: input.lastName,
          locale: input.locale,
          data: input.data
        }
      });
      return {
        success: response.success,
        id: response.id,
        projectId: response.project_id
      };
    },
    async listSubscribers(query) {
      const response = await request("/v1/subscribers", {
        auth: "apiKey",
        query
      });
      return mapListResponse(response, mapSubscriber);
    },
    async getSubscriber(id) {
      const response = await request(`/v1/subscribers/${encodeURIComponent(id)}`, {
        auth: "apiKey"
      });
      return mapSubscriber(response);
    },
    async deleteSubscriber(id) {
      return request(`/v1/subscribers/${encodeURIComponent(id)}`, {
        method: "DELETE",
        auth: "apiKey"
      });
    },
    async createSubscriberToken(input) {
      const response = await request("/v1/auth/subscriber-token", {
        method: "POST",
        auth: "apiKey",
        body: {
          subscriber_id: input.subscriberId,
          ttl_hours: input.ttlHours
        }
      });
      return {
        token: response.token,
        subscriberId: response.subscriber_id,
        projectId: response.project_id,
        expiresAt: response.expires_at,
        ttlHours: response.ttl_hours
      };
    },
    async getVapidPublicKey(project) {
      const response = await request("/v1/push/vapid-public-key", {
        auth: "none",
        query: { project }
      });
      return response.public_key;
    },
    async registerPushSubscription(input) {
      return request("/v1/push-tokens", {
        method: "POST",
        auth: "apiKey",
        body: {
          subscriber_id: input.subscriberId,
          endpoint: input.endpoint,
          keys: input.keys,
          expiration_time: input.expirationTime,
          platform: input.platform ?? "web",
          device_name: input.deviceName,
          user_agent: input.userAgent
        }
      });
    },
    async listPushTokens(subscriberId) {
      const response = await request(`/v1/push-tokens/subscriber/${encodeURIComponent(subscriberId)}`, {
        auth: "apiKey"
      });
      return {
        tokens: response.tokens.map((token) => ({
          id: token.id,
          token: token.token,
          platform: token.platform,
          deviceName: token.device_name,
          endpoint: token.endpoint,
          expirationTime: token.expiration_time,
          userAgent: token.user_agent
        }))
      };
    },
    async deletePushToken(id) {
      return request(`/v1/push-tokens/${encodeURIComponent(id)}`, {
        method: "DELETE",
        auth: "apiKey"
      });
    },
    async getInbox(subscriberId, query) {
      const response = await request(`/v1/inbox/${encodeURIComponent(subscriberId)}`, {
        auth: "inbox",
        query
      });
      return mapInboxResponse(response);
    },
    async getUnreadCount(subscriberId) {
      const response = await request(
        `/v1/inbox/${encodeURIComponent(subscriberId)}/unread-count`,
        { auth: "inbox" }
      );
      return response.unread_count;
    },
    async updateInboxMessage(subscriberId, messageId, input) {
      return request(
        `/v1/inbox/${encodeURIComponent(subscriberId)}/${encodeURIComponent(messageId)}`,
        {
          method: "PATCH",
          auth: "inbox",
          body: {
            read: input.read,
            archived: input.archived,
            is_todo: input.isTodo
          }
        }
      );
    },
    async markRead(subscriberId, messageId, read = true) {
      return this.updateInboxMessage(subscriberId, messageId, { read });
    },
    async markAllRead(subscriberId) {
      return request(`/v1/inbox/${encodeURIComponent(subscriberId)}/read-all`, {
        method: "POST",
        auth: "inbox"
      });
    },
    async createStreamTicket(subscriberId) {
      const response = await request(
        `/v1/inbox/${encodeURIComponent(subscriberId)}/stream-ticket`,
        {
          method: "POST",
          auth: "inbox"
        }
      );
      return {
        ticket: response.ticket,
        expiresInSeconds: response.expires_in_seconds
      };
    },
    // ── Jobs ────────────────────────────────────────────────────────────────
    /** Status of one job returned by `send` or `batch`. */
    async getJob(id) {
      return jobFromWire(await request(`/v1/jobs/${encodeURIComponent(id)}`, { auth: "apiKey" }));
    },
    /** Cancel a pending or scheduled job. */
    async cancelJob(id) {
      return request(`/v1/jobs/${encodeURIComponent(id)}`, { method: "DELETE", auth: "apiKey" });
    },
    /** Re-queue a failed job. */
    async retryJob(id) {
      return request(`/v1/jobs/${encodeURIComponent(id)}/retry`, { method: "POST", auth: "apiKey" });
    },
    // ── Templates ───────────────────────────────────────────────────────────
    /** Create or replace a template. `{{variables}}` in subject/body are filled from `send({ data })`. */
    async upsertTemplate(input) {
      return request("/v1/templates", {
        method: "POST",
        auth: "apiKey",
        body: { id: input.id, channel: input.channel, subject: input.subject, body: input.body, body_html: input.bodyHtml }
      });
    },
    async listTemplates(options = {}) {
      const page = await request("/v1/templates", { auth: "apiKey", query: options });
      return { ...page, items: page.items.map(templateFromWire) };
    },
    async getTemplate(id) {
      return templateFromWire(await request(`/v1/templates/${encodeURIComponent(id)}`, { auth: "apiKey" }));
    },
    async deleteTemplate(id) {
      return request(`/v1/templates/${encodeURIComponent(id)}`, { method: "DELETE", auth: "apiKey" });
    },
    // ── Workflows ───────────────────────────────────────────────────────────
    /** Create or replace a workflow. Runs start when `triggerWorkflow` fires its `triggerEvent`. */
    async upsertWorkflow(input) {
      return request("/v1/workflows", {
        method: "POST",
        auth: "apiKey",
        body: {
          id: input.id,
          name: input.name,
          description: input.description,
          trigger_event: input.triggerEvent,
          steps: input.steps.map(stepToWire),
          enabled: input.enabled
        }
      });
    },
    async listWorkflows() {
      const response = await request("/v1/workflows", { auth: "apiKey" });
      return (response.workflows ?? []).map(workflowFromWire);
    },
    async getWorkflow(id) {
      return workflowFromWire(await request(`/v1/workflows/${encodeURIComponent(id)}`, { auth: "apiKey" }));
    },
    async deleteWorkflow(id) {
      return request(`/v1/workflows/${encodeURIComponent(id)}`, { method: "DELETE", auth: "apiKey" });
    },
    /** Fire an event. Every enabled workflow whose `triggerEvent` matches starts a run for the subscriber. */
    async triggerWorkflow(input) {
      const response = await request("/v1/workflows/trigger", {
        method: "POST",
        auth: "apiKey",
        body: { event: input.event, subscriber_id: input.subscriberId, payload: input.payload }
      });
      return { success: response.success, workflowRuns: response.workflow_runs ?? [] };
    },
    async listWorkflowRuns(options = {}) {
      const page = await request("/v1/workflows/runs", {
        auth: "apiKey",
        query: { status: options.status, workflow_id: options.workflowId, limit: options.limit, offset: options.offset }
      });
      return { ...page, items: page.items.map(runFromWire) };
    },
    async cancelWorkflowRun(id) {
      return request(`/v1/workflows/runs/${encodeURIComponent(id)}`, { method: "DELETE", auth: "apiKey" });
    },
    // ── Preferences ─────────────────────────────────────────────────────────
    /** Everything is enabled by default. A workflow-specific row wins over the channel-wide `'*'` row. */
    async getPreferences(subscriberId) {
      const response = await request(
        `/v1/subscribers/${encodeURIComponent(subscriberId)}/preferences`,
        { auth: "apiKey" }
      );
      return (response.preferences ?? []).map((p) => ({ channel: p.channel, workflowId: p.workflow_id ?? "*", enabled: p.enabled }));
    },
    async setPreferences(subscriberId, preferences) {
      return request(`/v1/subscribers/${encodeURIComponent(subscriberId)}/preferences`, {
        method: "PUT",
        auth: "apiKey",
        body: { preferences: preferences.map((p) => ({ channel: p.channel, workflow_id: p.workflowId, enabled: p.enabled })) }
      });
    },
    // ── Suppressions ────────────────────────────────────────────────────────
    /** Stop sending to an address. Bounces and complaints are suppressed automatically; this is for manual opt-outs. */
    async suppress(email, options = {}) {
      const response = await request("/v1/suppressions", {
        method: "POST",
        auth: "apiKey",
        body: { email, scope: options.scope, detail: options.detail }
      });
      return { success: response.success, suppression: suppressionFromWire(response.suppression) };
    },
    /** Active suppressions for the project (most recent 200). */
    async listSuppressions() {
      const response = await request("/v1/suppressions", { auth: "apiKey" });
      const rows = Array.isArray(response) ? response : response.data ?? [];
      return rows.map(suppressionFromWire);
    },
    /** Allow sending to a suppressed address again. */
    async releaseSuppression(id) {
      return request(`/v1/suppressions/${encodeURIComponent(id)}`, { method: "DELETE", auth: "apiKey" });
    },
    async openInboxStream(subscriberId, options = {}) {
      const EventSourceImpl = config.eventSource ?? globalThis.EventSource;
      if (!EventSourceImpl) {
        throw new Error("notifyd EventSource implementation is not available");
      }
      const { ticket } = await this.createStreamTicket(subscriberId);
      const url = `${baseUrl}/v1/inbox/${encodeURIComponent(subscriberId)}/stream?token=${encodeURIComponent(ticket)}`;
      const eventSource = new EventSourceImpl(url);
      if (options.onMessage) {
        eventSource.onmessage = options.onMessage;
      }
      if (options.onError) {
        eventSource.onerror = options.onError;
      }
      return {
        eventSource,
        url,
        close: () => eventSource.close()
      };
    }
  };
}
export {
  NotifydError,
  createNotifydClient
};
