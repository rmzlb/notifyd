export type NotifydChannel = 'email' | 'sms' | 'push' | 'in_app' | (string & {});

/**
 * Email attachment. `content` is the file bytes encoded as a base64 string
 * (no data-URL prefix). `contentType` is optional — Resend infers it from
 * the filename when omitted. Attachments are only honoured on the `email`
 * channel and force the message onto the single-send path server-side.
 */
export interface NotifydAttachment {
  filename: string;
  content: string;
  contentType?: string;
}

export interface NotifydClientConfig {
  url: string;
  apiKey?: string;
  subscriberToken?: string;
  fetch?: typeof fetch;
  eventSource?: EventSourceFactory;
}

export interface SendNotificationInput {
  channel?: NotifydChannel;
  channels?: NotifydChannel[];
  to?: string;
  subscriberId?: string;
  template?: string;
  subject?: string;
  body?: string;
  bodyHtml?: string;
  vars?: Record<string, unknown>;
  scheduledAt?: string;
  idempotencyKey?: string;
  icon?: string;
  url?: string;
  /** Email attachments (email channel only). */
  attachments?: NotifydAttachment[];
}

export interface SendNotificationResponse {
  success: boolean;
  jobIds: string[];
  scheduledAt: string;
  channels: string[];
}

export interface BatchNotificationInput {
  channel?: NotifydChannel;
  channels?: NotifydChannel[];
  subscribers: string[];
  template?: string;
  subject?: string;
  body?: string;
  bodyHtml?: string;
  vars?: Record<string, unknown>;
  scheduledAt?: string;
}

export interface BatchNotificationResponse {
  success: boolean;
  jobsCreated: number;
  subscribers: number;
  channels: string[];
}

export interface SubscriberInput {
  id: string;
  email?: string;
  phone?: string;
  firstName?: string;
  lastName?: string;
  locale?: string;
  data?: Record<string, unknown>;
}

export interface Subscriber extends SubscriberInput {
  projectId?: string;
  createdAt?: string;
}

export interface ListResponse<T> {
  items: T[];
  total: number;
  limit: number;
  offset: number;
}

export interface SubscriberTokenInput {
  subscriberId: string;
  ttlHours?: number;
}

export interface SubscriberTokenResponse {
  token: string;
  subscriberId: string;
  projectId: string;
  expiresAt: string;
  ttlHours: number;
}

export interface InboxNotification {
  id: string;
  body: string;
  icon: string;
  url: string | null;
  data: Record<string, unknown> | null;
  isRead: boolean;
  readAt: string | null;
  isTodo: boolean;
  createdAt: string;
}

export interface InboxResponse extends ListResponse<InboxNotification> {}

export interface InboxQuery {
  limit?: number;
  offset?: number;
  filter?: 'all' | 'unread' | 'todo' | (string & {});
  q?: string;
}

export interface UpdateInboxMessageInput {
  read?: boolean;
  archived?: boolean;
  isTodo?: boolean;
}

export interface UpdateInboxMessageResponse {
  success: boolean;
}

export interface MarkAllReadResponse {
  success: boolean;
  updated: number;
}

export interface UnreadCountResponse {
  unreadCount: number;
}

export interface StreamTicketResponse {
  ticket: string;
  expiresInSeconds: number;
}

export interface NotifydErrorDetails {
  error?: string;
  [key: string]: unknown;
}

export class NotifydError extends Error {
  status: number;
  details: NotifydErrorDetails | string | null;

  constructor(message: string, status: number, details: NotifydErrorDetails | string | null = null) {
    super(message);
    this.name = 'NotifydError';
    this.status = status;
    this.details = details;
  }
}

export interface StreamMessageEvent {
  data: string;
}

export interface EventSourceLike {
  onmessage: ((event: StreamMessageEvent) => void) | null;
  onerror: ((error: unknown) => void) | null;
  close(): void;
}

export interface EventSourceFactory {
  new (url: string): EventSourceLike;
}

export interface OpenInboxStreamOptions {
  onMessage?: (event: StreamMessageEvent) => void;
  onError?: (error: unknown) => void;
}

export interface OpenInboxStreamResult {
  eventSource: EventSourceLike;
  url: string;
  close: () => void;
}

function normalizeUrl(url: string): string {
  return url.replace(/\/+$/, '');
}

function assertApiKey(apiKey?: string): string {
  if (!apiKey) {
    throw new Error('notifyd apiKey is required for this method');
  }
  return apiKey;
}

function assertInboxAuth(apiKey?: string, subscriberToken?: string): Record<string, string> {
  if (subscriberToken) {
    return { Authorization: `Bearer ${subscriberToken}` };
  }
  if (apiKey) {
    return { 'X-Api-Key': apiKey };
  }
  throw new Error('notifyd apiKey or subscriberToken is required for this method');
}

async function parseResponse<T>(res: Response): Promise<T> {
  const contentType = res.headers.get('content-type') || '';
  const isJson = contentType.includes('application/json');
  const body = isJson ? await res.json() : await res.text();

  if (!res.ok) {
    const message =
      typeof body === 'object' && body && 'error' in body && typeof body.error === 'string'
        ? body.error
        : `notifyd request failed (${res.status})`;
    throw new NotifydError(message, res.status, (body as NotifydErrorDetails | string | null) ?? null);
  }

  return body as T;
}

function buildQuery(params?: object): string {
  if (!params) return '';

  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params as Record<string, unknown>)) {
    if (value === undefined || value === null || value === '') continue;
    search.set(key, String(value));
  }

  const query = search.toString();
  return query ? `?${query}` : '';
}

function mapSubscriber(raw: Record<string, unknown>): Subscriber {
  return {
    id: String(raw.id),
    email: asOptionalString(raw.email),
    phone: asOptionalString(raw.phone),
    firstName: asOptionalString(raw.first_name),
    lastName: asOptionalString(raw.last_name),
    locale: asOptionalString(raw.locale),
    data: asRecord(raw.data),
    projectId: asOptionalString(raw.project_id),
    createdAt: asOptionalString(raw.created_at),
  };
}

function mapInboxNotification(raw: Record<string, unknown>): InboxNotification {
  return {
    id: String(raw.id),
    body: String(raw.body ?? ''),
    icon: String(raw.icon ?? 'bell'),
    url: asOptionalString(raw.url) ?? null,
    data: asRecord(raw.data),
    isRead: Boolean(raw.is_read),
    readAt: asOptionalString(raw.read_at) ?? null,
    isTodo: Boolean(raw.is_todo),
    createdAt: String(raw.created_at ?? ''),
  };
}

function mapInboxResponse(raw: Record<string, unknown>): InboxResponse {
  const items = Array.isArray(raw.items)
    ? raw.items.map((item) => mapInboxNotification(item as Record<string, unknown>))
    : [];

  return {
    items,
    total: Number(raw.total ?? items.length),
    limit: Number(raw.limit ?? items.length),
    offset: Number(raw.offset ?? 0),
  };
}

function mapListResponse<T>(raw: Record<string, unknown>, mapper: (value: Record<string, unknown>) => T): ListResponse<T> {
  const items = Array.isArray(raw.items)
    ? raw.items.map((item) => mapper(item as Record<string, unknown>))
    : [];

  return {
    items,
    total: Number(raw.total ?? items.length),
    limit: Number(raw.limit ?? items.length),
    offset: Number(raw.offset ?? 0),
  };
}

function asOptionalString(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  return value as Record<string, unknown>;
}

export function createNotifydClient(config: NotifydClientConfig) {
  const baseUrl = normalizeUrl(config.url);
  const fetchImpl = config.fetch ?? globalThis.fetch;

  if (!fetchImpl) {
    throw new Error('notifyd fetch implementation is not available');
  }

  async function request<T>(
    path: string,
    options: {
      method?: string;
      auth?: 'apiKey' | 'inbox';
      body?: unknown;
      query?: object;
    } = {},
  ): Promise<T> {
    const authHeaders =
      options.auth === 'apiKey'
        ? { 'X-Api-Key': assertApiKey(config.apiKey) }
        : assertInboxAuth(config.apiKey, config.subscriberToken);

    const res = await fetchImpl(`${baseUrl}${path}${buildQuery(options.query)}`, {
      method: options.method ?? 'GET',
      headers: {
        ...authHeaders,
        'Content-Type': 'application/json',
      },
      body: options.body === undefined ? undefined : JSON.stringify(options.body),
    });

    return parseResponse<T>(res);
  }

  return {
    async send(input: SendNotificationInput): Promise<SendNotificationResponse> {
      const response = await request<{
        success: boolean;
        job_ids: string[];
        scheduled_at: string;
        channels: string[];
      }>('/v1/send', {
        method: 'POST',
        auth: 'apiKey',
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
          attachments: input.attachments?.map((a) => ({
            filename: a.filename,
            content: a.content,
            content_type: a.contentType,
          })),
        },
      });

      return {
        success: response.success,
        jobIds: response.job_ids,
        scheduledAt: response.scheduled_at,
        channels: response.channels,
      };
    },

    async batch(input: BatchNotificationInput): Promise<BatchNotificationResponse> {
      const response = await request<{
        success: boolean;
        jobs_created: number;
        subscribers: number;
        channels: string[];
      }>('/v1/batch', {
        method: 'POST',
        auth: 'apiKey',
        body: {
          channel: input.channel,
          channels: input.channels,
          subscribers: input.subscribers,
          template: input.template,
          subject: input.subject,
          body: input.body,
          body_html: input.bodyHtml,
          vars: input.vars,
          scheduled_at: input.scheduledAt,
        },
      });

      return {
        success: response.success,
        jobsCreated: response.jobs_created,
        subscribers: response.subscribers,
        channels: response.channels,
      };
    },

    async upsertSubscriber(input: SubscriberInput): Promise<{ success: boolean; id: string; projectId: string }> {
      const response = await request<{
        success: boolean;
        id: string;
        project_id: string;
      }>('/v1/subscribers', {
        method: 'POST',
        auth: 'apiKey',
        body: {
          id: input.id,
          email: input.email,
          phone: input.phone,
          first_name: input.firstName,
          last_name: input.lastName,
          locale: input.locale,
          data: input.data,
        },
      });

      return {
        success: response.success,
        id: response.id,
        projectId: response.project_id,
      };
    },

    async listSubscribers(query?: { limit?: number; offset?: number; q?: string }): Promise<ListResponse<Subscriber>> {
      const response = await request<Record<string, unknown>>('/v1/subscribers', {
        auth: 'apiKey',
        query,
      });

      return mapListResponse(response, mapSubscriber);
    },

    async getSubscriber(id: string): Promise<Subscriber> {
      const response = await request<Record<string, unknown>>(`/v1/subscribers/${encodeURIComponent(id)}`, {
        auth: 'apiKey',
      });

      return mapSubscriber(response);
    },

    async deleteSubscriber(id: string): Promise<{ success: boolean }> {
      return request<{ success: boolean }>(`/v1/subscribers/${encodeURIComponent(id)}`, {
        method: 'DELETE',
        auth: 'apiKey',
      });
    },

    async createSubscriberToken(input: SubscriberTokenInput): Promise<SubscriberTokenResponse> {
      const response = await request<{
        token: string;
        subscriber_id: string;
        project_id: string;
        expires_at: string;
        ttl_hours: number;
      }>('/v1/auth/subscriber-token', {
        method: 'POST',
        auth: 'apiKey',
        body: {
          subscriber_id: input.subscriberId,
          ttl_hours: input.ttlHours,
        },
      });

      return {
        token: response.token,
        subscriberId: response.subscriber_id,
        projectId: response.project_id,
        expiresAt: response.expires_at,
        ttlHours: response.ttl_hours,
      };
    },

    async getInbox(subscriberId: string, query?: InboxQuery): Promise<InboxResponse> {
      const response = await request<Record<string, unknown>>(`/v1/inbox/${encodeURIComponent(subscriberId)}`, {
        auth: 'inbox',
        query,
      });

      return mapInboxResponse(response);
    },

    async getUnreadCount(subscriberId: string): Promise<number> {
      const response = await request<{ unread_count: number }>(
        `/v1/inbox/${encodeURIComponent(subscriberId)}/unread-count`,
        { auth: 'inbox' },
      );
      return response.unread_count;
    },

    async updateInboxMessage(
      subscriberId: string,
      messageId: string,
      input: UpdateInboxMessageInput,
    ): Promise<UpdateInboxMessageResponse> {
      return request<UpdateInboxMessageResponse>(
        `/v1/inbox/${encodeURIComponent(subscriberId)}/${encodeURIComponent(messageId)}`,
        {
          method: 'PATCH',
          auth: 'inbox',
          body: {
            read: input.read,
            archived: input.archived,
            is_todo: input.isTodo,
          },
        },
      );
    },

    async markRead(subscriberId: string, messageId: string, read = true): Promise<UpdateInboxMessageResponse> {
      return this.updateInboxMessage(subscriberId, messageId, { read });
    },

    async markAllRead(subscriberId: string): Promise<MarkAllReadResponse> {
      return request<MarkAllReadResponse>(`/v1/inbox/${encodeURIComponent(subscriberId)}/read-all`, {
        method: 'POST',
        auth: 'inbox',
      });
    },

    async createStreamTicket(subscriberId: string): Promise<StreamTicketResponse> {
      const response = await request<{ ticket: string; expires_in_seconds: number }>(
        `/v1/inbox/${encodeURIComponent(subscriberId)}/stream-ticket`,
        {
          method: 'POST',
          auth: 'inbox',
        },
      );

      return {
        ticket: response.ticket,
        expiresInSeconds: response.expires_in_seconds,
      };
    },

    async openInboxStream(
      subscriberId: string,
      options: OpenInboxStreamOptions = {},
    ): Promise<OpenInboxStreamResult> {
      const EventSourceImpl = config.eventSource ?? (globalThis.EventSource as EventSourceFactory | undefined);

      if (!EventSourceImpl) {
        throw new Error('notifyd EventSource implementation is not available');
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
        close: () => eventSource.close(),
      };
    },
  };
}
