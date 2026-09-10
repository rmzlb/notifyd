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
  /** Carbon-copy recipients (email channel only, maximum 10). */
  cc?: string[];
  /** Address that receives replies (email channel only). */
  replyTo?: string;
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

export interface VapidPublicKeyResponse {
  publicKey: string;
}

export interface WebPushSubscriptionInput {
  subscriberId: string;
  endpoint: string;
  keys: {
    p256dh: string;
    auth: string;
  };
  expirationTime?: string | null;
  platform?: string;
  deviceName?: string;
  userAgent?: string;
}

export interface PushToken {
  id: string;
  token: string;
  platform: string;
  deviceName?: string;
  endpoint?: string;
  expirationTime?: string;
  userAgent?: string;
}

export interface PushTokensResponse {
  tokens: PushToken[];
}

export type JobStatus = 'pending' | 'processing' | 'sent' | 'retry' | 'failed' | 'cancelled' | (string & {});

export interface ProviderEvent {
  provider: string;
  type: string;
  occurredAt: string;
  providerMessageId?: string | null;
  recipients: unknown;
  error?: unknown;
}

export interface Job {
  id: string;
  status: JobStatus;
  channel: string;
  subscriberId?: string | null;
  recipient?: string | null;
  templateId?: string | null;
  priority?: number;
  attempts: number;
  maxAttempts?: number;
  provider?: string | null;
  providerMessageId?: string | null;
  scheduledAt?: string | null;
  createdAt?: string;
  sentAt?: string | null;
  deliveredAt?: string | null;
  bouncedAt?: string | null;
  error?: string | null;
  providerEvents: ProviderEvent[];
}

export interface TemplateInput {
  id: string;
  channel: NotifydChannel;
  subject?: string;
  body: string;
  bodyHtml?: string;
}

export interface Template extends TemplateInput {}

export interface Page<T> {
  items: T[];
  total: number;
  limit: number;
  offset: number;
}

/** Steps run in order; `condition` jumps to a step index. Durations are in seconds. */
export type WorkflowStep =
  | { type: 'send'; channel: NotifydChannel; template?: string; subject?: string; body?: string; bodyHtml?: string }
  | { type: 'delay'; durationSecs: number }
  | { type: 'condition'; field: string; operator: 'eq' | 'neq' | 'gt' | 'lt'; value: unknown; onTrue?: number; onFalse?: number }
  | { type: 'digest'; durationSecs: number; channel: NotifydChannel; template?: string; subject?: string; body?: string };

export interface WorkflowInput {
  id: string;
  name: string;
  description?: string;
  triggerEvent: string;
  steps: WorkflowStep[];
  enabled?: boolean;
}

export interface Workflow extends WorkflowInput {
  createdAt?: string;
}

export interface WorkflowRun {
  id: string;
  workflowId: string;
  subscriberId: string;
  status: string;
  currentStep: number;
  resumeAt?: string | null;
  createdAt?: string;
}

export interface TriggerWorkflowInput {
  event: string;
  subscriberId: string;
  payload?: Record<string, unknown>;
}

export interface Preference {
  channel: NotifydChannel | '*';
  /** A workflow id, or `'*'` for every workflow on that channel. */
  workflowId: string;
  enabled: boolean;
}

export interface Suppression {
  id: string;
  email: string;
  reason: string;
  detail?: string | null;
  createdAt: string;
  releasedAt?: string | null;
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
    data: asRecord(raw.data) ?? undefined,
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

type WireStep = Record<string, unknown> & { type: string };

function stepToWire(step: WorkflowStep): WireStep {
  switch (step.type) {
    case 'send':
      return { type: 'send', channel: step.channel, template: step.template, subject: step.subject, body: step.body, body_html: step.bodyHtml };
    case 'delay':
      return { type: 'delay', duration_secs: step.durationSecs };
    case 'condition':
      return { type: 'condition', field: step.field, operator: step.operator, value: step.value, on_true: step.onTrue, on_false: step.onFalse };
    case 'digest':
      return { type: 'digest', duration_secs: step.durationSecs, channel: step.channel, template: step.template, subject: step.subject, body: step.body };
  }
}

function stepFromWire(w: WireStep): WorkflowStep {
  const str = (k: string) => (typeof w[k] === 'string' ? (w[k] as string) : undefined);
  const num = (k: string) => (typeof w[k] === 'number' ? (w[k] as number) : undefined);
  switch (w.type) {
    case 'delay':
      return { type: 'delay', durationSecs: num('duration_secs') ?? 0 };
    case 'condition':
      return { type: 'condition', field: str('field') ?? '', operator: (str('operator') ?? 'eq') as 'eq' | 'neq' | 'gt' | 'lt', value: w.value, onTrue: num('on_true'), onFalse: num('on_false') };
    case 'digest':
      return { type: 'digest', durationSecs: num('duration_secs') ?? 0, channel: (str('channel') ?? 'email') as NotifydChannel, template: str('template'), subject: str('subject'), body: str('body') };
    default:
      return { type: 'send', channel: (str('channel') ?? 'email') as NotifydChannel, template: str('template'), subject: str('subject'), body: str('body'), bodyHtml: str('body_html') };
  }
}

interface WireWorkflow {
  id: string;
  name: string;
  description?: string | null;
  trigger_event: string;
  steps: WireStep[];
  enabled?: boolean;
  created_at?: string;
}

function workflowFromWire(w: WireWorkflow): Workflow {
  return {
    id: w.id,
    name: w.name,
    description: w.description ?? undefined,
    triggerEvent: w.trigger_event,
    steps: (w.steps ?? []).map(stepFromWire),
    enabled: w.enabled,
    createdAt: w.created_at,
  };
}

interface WireRun {
  id: string;
  workflow_id: string;
  subscriber_id: string;
  status: string;
  current_step: number;
  resume_at?: string | null;
  created_at?: string;
}

function runFromWire(r: WireRun): WorkflowRun {
  return { id: r.id, workflowId: r.workflow_id, subscriberId: r.subscriber_id, status: r.status, currentStep: r.current_step, resumeAt: r.resume_at ?? null, createdAt: r.created_at };
}

interface WireJob {
  id: string;
  status: JobStatus;
  channel: string;
  subscriber_id?: string | null;
  recipient?: string | null;
  template_id?: string | null;
  priority?: number;
  attempts?: number;
  max_attempts?: number;
  provider?: string | null;
  provider_message_id?: string | null;
  scheduled_at?: string | null;
  created_at?: string;
  sent_at?: string | null;
  delivered_at?: string | null;
  bounced_at?: string | null;
  error?: string | null;
  provider_events?: Array<{ provider: string; type: string; occurred_at: string; provider_message_id?: string | null; recipients: unknown; error?: unknown }>;
}

function jobFromWire(j: WireJob): Job {
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
      error: e.error,
    })),
  };
}

interface WireTemplate {
  id: string;
  channel: string;
  subject?: string | null;
  body: string;
  body_html?: string | null;
}

function templateFromWire(t: WireTemplate): Template {
  return { id: t.id, channel: t.channel as NotifydChannel, subject: t.subject ?? undefined, body: t.body, bodyHtml: t.body_html ?? undefined };
}

interface WireSuppression {
  id: string;
  email: string;
  reason: string;
  detail?: string | null;
  created_at: string;
  released_at?: string | null;
}

function suppressionFromWire(s: WireSuppression): Suppression {
  return { id: s.id, email: s.email, reason: s.reason, detail: s.detail ?? null, createdAt: s.created_at, releasedAt: s.released_at ?? null };
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
      auth?: 'apiKey' | 'inbox' | 'none';
      body?: unknown;
      query?: object;
    } = {},
  ): Promise<T> {
    const authHeaders =
      options.auth === 'none'
        ? {}
        : options.auth === 'apiKey'
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
          cc: input.cc,
          reply_to: input.replyTo,
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

    async getVapidPublicKey(project?: string): Promise<string> {
      const response = await request<{ public_key: string }>('/v1/push/vapid-public-key', {
        auth: 'none',
        query: { project },
      });

      return response.public_key;
    },

    async registerPushSubscription(input: WebPushSubscriptionInput): Promise<{ success: boolean }> {
      return request<{ success: boolean }>('/v1/push-tokens', {
        method: 'POST',
        auth: 'apiKey',
        body: {
          subscriber_id: input.subscriberId,
          endpoint: input.endpoint,
          keys: input.keys,
          expiration_time: input.expirationTime,
          platform: input.platform ?? 'web',
          device_name: input.deviceName,
          user_agent: input.userAgent,
        },
      });
    },

    async listPushTokens(subscriberId: string): Promise<PushTokensResponse> {
      const response = await request<{
        tokens: Array<{
          id: string;
          token: string;
          platform: string;
          device_name?: string;
          endpoint?: string;
          expiration_time?: string;
          user_agent?: string;
        }>;
      }>(`/v1/push-tokens/subscriber/${encodeURIComponent(subscriberId)}`, {
        auth: 'apiKey',
      });

      return {
        tokens: response.tokens.map((token) => ({
          id: token.id,
          token: token.token,
          platform: token.platform,
          deviceName: token.device_name,
          endpoint: token.endpoint,
          expirationTime: token.expiration_time,
          userAgent: token.user_agent,
        })),
      };
    },

    async deletePushToken(id: string): Promise<{ success: boolean }> {
      return request<{ success: boolean }>(`/v1/push-tokens/${encodeURIComponent(id)}`, {
        method: 'DELETE',
        auth: 'apiKey',
      });
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

    // ── Jobs ────────────────────────────────────────────────────────────────
    /** Status of one job returned by `send` or `batch`. */
    async getJob(id: string): Promise<Job> {
      return jobFromWire(await request<WireJob>(`/v1/jobs/${encodeURIComponent(id)}`, { auth: 'apiKey' }));
    },

    /** Cancel a pending or scheduled job. */
    async cancelJob(id: string): Promise<{ success: boolean }> {
      return request<{ success: boolean }>(`/v1/jobs/${encodeURIComponent(id)}`, { method: 'DELETE', auth: 'apiKey' });
    },

    /** Re-queue a failed job. */
    async retryJob(id: string): Promise<{ success: boolean }> {
      return request<{ success: boolean }>(`/v1/jobs/${encodeURIComponent(id)}/retry`, { method: 'POST', auth: 'apiKey' });
    },

    // ── Templates ───────────────────────────────────────────────────────────
    /** Create or replace a template. `{{variables}}` in subject/body are filled from `send({ data })`. */
    async upsertTemplate(input: TemplateInput): Promise<{ success: boolean; id: string }> {
      return request<{ success: boolean; id: string }>('/v1/templates', {
        method: 'POST',
        auth: 'apiKey',
        body: { id: input.id, channel: input.channel, subject: input.subject, body: input.body, body_html: input.bodyHtml },
      });
    },

    async listTemplates(options: { limit?: number; offset?: number } = {}): Promise<Page<Template>> {
      const page = await request<Page<WireTemplate>>('/v1/templates', { auth: 'apiKey', query: options });
      return { ...page, items: page.items.map(templateFromWire) };
    },

    async getTemplate(id: string): Promise<Template> {
      return templateFromWire(await request<WireTemplate>(`/v1/templates/${encodeURIComponent(id)}`, { auth: 'apiKey' }));
    },

    async deleteTemplate(id: string): Promise<{ success: boolean }> {
      return request<{ success: boolean }>(`/v1/templates/${encodeURIComponent(id)}`, { method: 'DELETE', auth: 'apiKey' });
    },

    // ── Workflows ───────────────────────────────────────────────────────────
    /** Create or replace a workflow. Runs start when `triggerWorkflow` fires its `triggerEvent`. */
    async upsertWorkflow(input: WorkflowInput): Promise<{ success: boolean; id: string }> {
      return request<{ success: boolean; id: string }>('/v1/workflows', {
        method: 'POST',
        auth: 'apiKey',
        body: {
          id: input.id,
          name: input.name,
          description: input.description,
          trigger_event: input.triggerEvent,
          steps: input.steps.map(stepToWire),
          enabled: input.enabled,
        },
      });
    },

    async listWorkflows(): Promise<Workflow[]> {
      const response = await request<{ workflows: WireWorkflow[] }>('/v1/workflows', { auth: 'apiKey' });
      return (response.workflows ?? []).map(workflowFromWire);
    },

    async getWorkflow(id: string): Promise<Workflow> {
      return workflowFromWire(await request<WireWorkflow>(`/v1/workflows/${encodeURIComponent(id)}`, { auth: 'apiKey' }));
    },

    async deleteWorkflow(id: string): Promise<{ success: boolean }> {
      return request<{ success: boolean }>(`/v1/workflows/${encodeURIComponent(id)}`, { method: 'DELETE', auth: 'apiKey' });
    },

    /** Fire an event. Every enabled workflow whose `triggerEvent` matches starts a run for the subscriber. */
    async triggerWorkflow(input: TriggerWorkflowInput): Promise<{ success: boolean; workflowRuns: string[] }> {
      const response = await request<{ success: boolean; workflow_runs: string[] }>('/v1/workflows/trigger', {
        method: 'POST',
        auth: 'apiKey',
        body: { event: input.event, subscriber_id: input.subscriberId, payload: input.payload },
      });
      return { success: response.success, workflowRuns: response.workflow_runs ?? [] };
    },

    async listWorkflowRuns(options: { status?: string; workflowId?: string; limit?: number; offset?: number } = {}): Promise<Page<WorkflowRun>> {
      const page = await request<Page<WireRun>>('/v1/workflows/runs', {
        auth: 'apiKey',
        query: { status: options.status, workflow_id: options.workflowId, limit: options.limit, offset: options.offset },
      });
      return { ...page, items: page.items.map(runFromWire) };
    },

    async cancelWorkflowRun(id: string): Promise<{ success: boolean }> {
      return request<{ success: boolean }>(`/v1/workflows/runs/${encodeURIComponent(id)}`, { method: 'DELETE', auth: 'apiKey' });
    },

    // ── Preferences ─────────────────────────────────────────────────────────
    /** Everything is enabled by default. A workflow-specific row wins over the channel-wide `'*'` row. */
    async getPreferences(subscriberId: string): Promise<Preference[]> {
      const response = await request<{ preferences: Array<{ channel: string; workflow_id: string | null; enabled: boolean }> }>(
        `/v1/subscribers/${encodeURIComponent(subscriberId)}/preferences`,
        { auth: 'apiKey' },
      );
      return (response.preferences ?? []).map((p) => ({ channel: p.channel as Preference['channel'], workflowId: p.workflow_id ?? '*', enabled: p.enabled }));
    },

    async setPreferences(subscriberId: string, preferences: Preference[]): Promise<{ success: boolean }> {
      return request<{ success: boolean }>(`/v1/subscribers/${encodeURIComponent(subscriberId)}/preferences`, {
        method: 'PUT',
        auth: 'apiKey',
        body: { preferences: preferences.map((p) => ({ channel: p.channel, workflow_id: p.workflowId, enabled: p.enabled })) },
      });
    },

    // ── Suppressions ────────────────────────────────────────────────────────
    /** Stop sending to an address. Bounces and complaints are suppressed automatically; this is for manual opt-outs. */
    async suppress(email: string, options: { scope?: 'all' | 'marketing'; detail?: string } = {}): Promise<{ success: boolean; suppression: Suppression }> {
      const response = await request<{ success: boolean; suppression: WireSuppression }>('/v1/suppressions', {
        method: 'POST',
        auth: 'apiKey',
        body: { email, scope: options.scope, detail: options.detail },
      });
      return { success: response.success, suppression: suppressionFromWire(response.suppression) };
    },

    /** Active suppressions for the project (most recent 200). */
    async listSuppressions(): Promise<Suppression[]> {
      const response = await request<{ data?: WireSuppression[] } | WireSuppression[]>('/v1/suppressions', { auth: 'apiKey' });
      const rows = Array.isArray(response) ? response : response.data ?? [];
      return rows.map(suppressionFromWire);
    },

    /** Allow sending to a suppressed address again. */
    async releaseSuppression(id: string): Promise<{ success: boolean }> {
      return request<{ success: boolean }>(`/v1/suppressions/${encodeURIComponent(id)}`, { method: 'DELETE', auth: 'apiKey' });
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
