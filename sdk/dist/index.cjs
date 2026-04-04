var __defProp = Object.defineProperty;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __export = (target, all) => {
  for (var name in all)
    __defProp(target, name, { get: all[name], enumerable: true });
};
var __copyProps = (to, from, except, desc) => {
  if (from && typeof from === "object" || typeof from === "function") {
    for (let key of __getOwnPropNames(from))
      if (!__hasOwnProp.call(to, key) && key !== except)
        __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
  }
  return to;
};
var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);

// sdk/index.ts
var index_exports = {};
__export(index_exports, {
  NotifydError: () => NotifydError,
  createNotifydClient: () => createNotifydClient
});
module.exports = __toCommonJS(index_exports);
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
    data: asRecord(raw.data),
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
function createNotifydClient(config) {
  const baseUrl = normalizeUrl(config.url);
  const fetchImpl = config.fetch ?? globalThis.fetch;
  if (!fetchImpl) {
    throw new Error("notifyd fetch implementation is not available");
  }
  async function request(path, options = {}) {
    const authHeaders = options.auth === "apiKey" ? { "X-Api-Key": assertApiKey(config.apiKey) } : assertInboxAuth(config.apiKey, config.subscriberToken);
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
          url: input.url
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
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  NotifydError,
  createNotifydClient
});
