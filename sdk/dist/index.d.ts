type NotifydChannel = 'email' | 'sms' | 'push' | 'in_app' | (string & {});
/**
 * Email attachment. `content` is the file bytes encoded as a base64 string
 * (no data-URL prefix). `contentType` is optional — Resend infers it from
 * the filename when omitted. Attachments are only honoured on the `email`
 * channel and force the message onto the single-send path server-side.
 */
interface NotifydAttachment {
    filename: string;
    content: string;
    contentType?: string;
}
interface NotifydClientConfig {
    url: string;
    apiKey?: string;
    subscriberToken?: string;
    fetch?: typeof fetch;
    eventSource?: EventSourceFactory;
}
interface SendNotificationInput {
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
interface SendNotificationResponse {
    success: boolean;
    jobIds: string[];
    scheduledAt: string;
    channels: string[];
}
interface BatchNotificationInput {
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
interface BatchNotificationResponse {
    success: boolean;
    jobsCreated: number;
    subscribers: number;
    channels: string[];
}
interface SubscriberInput {
    id: string;
    email?: string;
    phone?: string;
    firstName?: string;
    lastName?: string;
    locale?: string;
    data?: Record<string, unknown>;
}
interface Subscriber extends SubscriberInput {
    projectId?: string;
    createdAt?: string;
}
interface ListResponse<T> {
    items: T[];
    total: number;
    limit: number;
    offset: number;
}
interface SubscriberTokenInput {
    subscriberId: string;
    ttlHours?: number;
}
interface SubscriberTokenResponse {
    token: string;
    subscriberId: string;
    projectId: string;
    expiresAt: string;
    ttlHours: number;
}
interface InboxNotification {
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
interface InboxResponse extends ListResponse<InboxNotification> {
}
interface InboxQuery {
    limit?: number;
    offset?: number;
    filter?: 'all' | 'unread' | 'todo' | (string & {});
    q?: string;
}
interface UpdateInboxMessageInput {
    read?: boolean;
    archived?: boolean;
    isTodo?: boolean;
}
interface UpdateInboxMessageResponse {
    success: boolean;
}
interface MarkAllReadResponse {
    success: boolean;
    updated: number;
}
interface UnreadCountResponse {
    unreadCount: number;
}
interface StreamTicketResponse {
    ticket: string;
    expiresInSeconds: number;
}
interface VapidPublicKeyResponse {
    publicKey: string;
}
interface WebPushSubscriptionInput {
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
interface PushToken {
    id: string;
    token: string;
    platform: string;
    deviceName?: string;
    endpoint?: string;
    expirationTime?: string;
    userAgent?: string;
}
interface PushTokensResponse {
    tokens: PushToken[];
}
type JobStatus = 'pending' | 'processing' | 'sent' | 'retry' | 'failed' | 'cancelled' | (string & {});
interface ProviderEvent {
    provider: string;
    type: string;
    occurredAt: string;
    providerMessageId?: string | null;
    recipients: unknown;
    error?: unknown;
}
interface Job {
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
interface TemplateInput {
    id: string;
    channel: NotifydChannel;
    subject?: string;
    body: string;
    bodyHtml?: string;
}
interface Template extends TemplateInput {
}
interface Page<T> {
    items: T[];
    total: number;
    limit: number;
    offset: number;
}
/** Steps run in order; `condition` jumps to a step index. Durations are in seconds. */
type WorkflowStep = {
    type: 'send';
    channel: NotifydChannel;
    template?: string;
    subject?: string;
    body?: string;
    bodyHtml?: string;
} | {
    type: 'delay';
    durationSecs: number;
} | {
    type: 'condition';
    field: string;
    operator: 'eq' | 'neq' | 'gt' | 'lt';
    value: unknown;
    onTrue?: number;
    onFalse?: number;
} | {
    type: 'digest';
    durationSecs: number;
    channel: NotifydChannel;
    template?: string;
    subject?: string;
    body?: string;
};
interface WorkflowInput {
    id: string;
    name: string;
    description?: string;
    triggerEvent: string;
    steps: WorkflowStep[];
    enabled?: boolean;
}
interface Workflow extends WorkflowInput {
    createdAt?: string;
}
interface WorkflowRun {
    id: string;
    workflowId: string;
    subscriberId: string;
    status: string;
    currentStep: number;
    resumeAt?: string | null;
    createdAt?: string;
}
interface TriggerWorkflowInput {
    event: string;
    subscriberId: string;
    payload?: Record<string, unknown>;
}
interface Preference {
    channel: NotifydChannel | '*';
    /** A workflow id, or `'*'` for every workflow on that channel. */
    workflowId: string;
    enabled: boolean;
}
interface Suppression {
    id: string;
    email: string;
    reason: string;
    detail?: string | null;
    createdAt: string;
    releasedAt?: string | null;
}
interface NotifydErrorDetails {
    error?: string;
    [key: string]: unknown;
}
declare class NotifydError extends Error {
    status: number;
    details: NotifydErrorDetails | string | null;
    constructor(message: string, status: number, details?: NotifydErrorDetails | string | null);
}
interface StreamMessageEvent {
    data: string;
}
interface EventSourceLike {
    onmessage: ((event: StreamMessageEvent) => void) | null;
    onerror: ((error: unknown) => void) | null;
    close(): void;
}
interface EventSourceFactory {
    new (url: string): EventSourceLike;
}
interface OpenInboxStreamOptions {
    onMessage?: (event: StreamMessageEvent) => void;
    onError?: (error: unknown) => void;
}
interface OpenInboxStreamResult {
    eventSource: EventSourceLike;
    url: string;
    close: () => void;
}
declare function createNotifydClient(config: NotifydClientConfig): {
    send(input: SendNotificationInput): Promise<SendNotificationResponse>;
    batch(input: BatchNotificationInput): Promise<BatchNotificationResponse>;
    upsertSubscriber(input: SubscriberInput): Promise<{
        success: boolean;
        id: string;
        projectId: string;
    }>;
    listSubscribers(query?: {
        limit?: number;
        offset?: number;
        q?: string;
    }): Promise<ListResponse<Subscriber>>;
    getSubscriber(id: string): Promise<Subscriber>;
    deleteSubscriber(id: string): Promise<{
        success: boolean;
    }>;
    createSubscriberToken(input: SubscriberTokenInput): Promise<SubscriberTokenResponse>;
    getVapidPublicKey(project?: string): Promise<string>;
    registerPushSubscription(input: WebPushSubscriptionInput): Promise<{
        success: boolean;
    }>;
    listPushTokens(subscriberId: string): Promise<PushTokensResponse>;
    deletePushToken(id: string): Promise<{
        success: boolean;
    }>;
    getInbox(subscriberId: string, query?: InboxQuery): Promise<InboxResponse>;
    getUnreadCount(subscriberId: string): Promise<number>;
    updateInboxMessage(subscriberId: string, messageId: string, input: UpdateInboxMessageInput): Promise<UpdateInboxMessageResponse>;
    markRead(subscriberId: string, messageId: string, read?: boolean): Promise<UpdateInboxMessageResponse>;
    markAllRead(subscriberId: string): Promise<MarkAllReadResponse>;
    createStreamTicket(subscriberId: string): Promise<StreamTicketResponse>;
    /** Status of one job returned by `send` or `batch`. */
    getJob(id: string): Promise<Job>;
    /** Cancel a pending or scheduled job. */
    cancelJob(id: string): Promise<{
        success: boolean;
    }>;
    /** Re-queue a failed job. */
    retryJob(id: string): Promise<{
        success: boolean;
    }>;
    /** Create or replace a template. `{{variables}}` in subject/body are filled from `send({ data })`. */
    upsertTemplate(input: TemplateInput): Promise<{
        success: boolean;
        id: string;
    }>;
    listTemplates(options?: {
        limit?: number;
        offset?: number;
    }): Promise<Page<Template>>;
    getTemplate(id: string): Promise<Template>;
    deleteTemplate(id: string): Promise<{
        success: boolean;
    }>;
    /** Create or replace a workflow. Runs start when `triggerWorkflow` fires its `triggerEvent`. */
    upsertWorkflow(input: WorkflowInput): Promise<{
        success: boolean;
        id: string;
    }>;
    listWorkflows(): Promise<Workflow[]>;
    getWorkflow(id: string): Promise<Workflow>;
    deleteWorkflow(id: string): Promise<{
        success: boolean;
    }>;
    /** Fire an event. Every enabled workflow whose `triggerEvent` matches starts a run for the subscriber. */
    triggerWorkflow(input: TriggerWorkflowInput): Promise<{
        success: boolean;
        workflowRuns: string[];
    }>;
    listWorkflowRuns(options?: {
        status?: string;
        workflowId?: string;
        limit?: number;
        offset?: number;
    }): Promise<Page<WorkflowRun>>;
    cancelWorkflowRun(id: string): Promise<{
        success: boolean;
    }>;
    /** Everything is enabled by default. A workflow-specific row wins over the channel-wide `'*'` row. */
    getPreferences(subscriberId: string): Promise<Preference[]>;
    setPreferences(subscriberId: string, preferences: Preference[]): Promise<{
        success: boolean;
    }>;
    /** Stop sending to an address. Bounces and complaints are suppressed automatically; this is for manual opt-outs. */
    suppress(email: string, options?: {
        scope?: "all" | "marketing";
        detail?: string;
    }): Promise<{
        success: boolean;
        suppression: Suppression;
    }>;
    /** Active suppressions for the project (most recent 200). */
    listSuppressions(): Promise<Suppression[]>;
    /** Allow sending to a suppressed address again. */
    releaseSuppression(id: string): Promise<{
        success: boolean;
    }>;
    openInboxStream(subscriberId: string, options?: OpenInboxStreamOptions): Promise<OpenInboxStreamResult>;
};

export { type BatchNotificationInput, type BatchNotificationResponse, type EventSourceFactory, type EventSourceLike, type InboxNotification, type InboxQuery, type InboxResponse, type Job, type JobStatus, type ListResponse, type MarkAllReadResponse, type NotifydAttachment, type NotifydChannel, type NotifydClientConfig, NotifydError, type NotifydErrorDetails, type OpenInboxStreamOptions, type OpenInboxStreamResult, type Page, type Preference, type ProviderEvent, type PushToken, type PushTokensResponse, type SendNotificationInput, type SendNotificationResponse, type StreamMessageEvent, type StreamTicketResponse, type Subscriber, type SubscriberInput, type SubscriberTokenInput, type SubscriberTokenResponse, type Suppression, type Template, type TemplateInput, type TriggerWorkflowInput, type UnreadCountResponse, type UpdateInboxMessageInput, type UpdateInboxMessageResponse, type VapidPublicKeyResponse, type WebPushSubscriptionInput, type Workflow, type WorkflowInput, type WorkflowRun, type WorkflowStep, createNotifydClient };
