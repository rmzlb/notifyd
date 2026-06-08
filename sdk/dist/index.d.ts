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
    getInbox(subscriberId: string, query?: InboxQuery): Promise<InboxResponse>;
    getUnreadCount(subscriberId: string): Promise<number>;
    updateInboxMessage(subscriberId: string, messageId: string, input: UpdateInboxMessageInput): Promise<UpdateInboxMessageResponse>;
    markRead(subscriberId: string, messageId: string, read?: boolean): Promise<UpdateInboxMessageResponse>;
    markAllRead(subscriberId: string): Promise<MarkAllReadResponse>;
    createStreamTicket(subscriberId: string): Promise<StreamTicketResponse>;
    openInboxStream(subscriberId: string, options?: OpenInboxStreamOptions): Promise<OpenInboxStreamResult>;
};

export { type BatchNotificationInput, type BatchNotificationResponse, type EventSourceFactory, type EventSourceLike, type InboxNotification, type InboxQuery, type InboxResponse, type ListResponse, type MarkAllReadResponse, type NotifydAttachment, type NotifydChannel, type NotifydClientConfig, NotifydError, type NotifydErrorDetails, type OpenInboxStreamOptions, type OpenInboxStreamResult, type SendNotificationInput, type SendNotificationResponse, type StreamMessageEvent, type StreamTicketResponse, type Subscriber, type SubscriberInput, type SubscriberTokenInput, type SubscriberTokenResponse, type UnreadCountResponse, type UpdateInboxMessageInput, type UpdateInboxMessageResponse, createNotifydClient };
