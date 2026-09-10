# notifyd-sdk

TypeScript client for [notifyd](https://github.com/rmzlb/notifyd), the self-hosted
notification server (email, SMS, WhatsApp, push, in-app). Zero dependencies, works in
Node 18+, browsers and edge runtimes (`fetch` + `EventSource`).

```bash
npm i notifyd-sdk
```

```typescript
import { createNotifydClient, NotifydError } from 'notifyd-sdk';

const notifyd = createNotifydClient({ url: 'http://localhost:3400', apiKey: 'sk_myapp_…' });

const { jobIds } = await notifyd.send({
  channels: ['email', 'in_app'],
  subscriberId: 'user-42',
  subject: 'Your order shipped',
  body: 'Hi {{first_name}}, parcel {{parcel}} is on its way.',
  vars: { first_name: 'Alice', parcel: 'FR-2041' },
  idempotencyKey: 'order-2041-shipped',
});

const job = await notifyd.getJob(jobIds[0]);   // pending → processing → sent | retry | failed
```

## Surface

| Area | Methods |
| --- | --- |
| Send | `send`, `batch` |
| Jobs | `getJob`, `cancelJob`, `retryJob` |
| Subscribers | `upsertSubscriber`, `getSubscriber`, `listSubscribers`, `deleteSubscriber` |
| Preferences | `getPreferences`, `setPreferences` |
| Templates | `upsertTemplate`, `getTemplate`, `listTemplates`, `deleteTemplate` |
| Workflows | `upsertWorkflow`, `getWorkflow`, `listWorkflows`, `deleteWorkflow`, `triggerWorkflow`, `listWorkflowRuns`, `cancelWorkflowRun` |
| Suppressions | `suppress`, `listSuppressions`, `releaseSuppression` |
| Push | `getVapidPublicKey`, `registerPushSubscription`, `listPushTokens`, `deletePushToken` |
| Inbox | `createSubscriberToken`, `getInbox`, `getUnreadCount`, `updateInboxMessage`, `markRead`, `markAllRead`, `createStreamTicket`, `openInboxStream` |

Inputs and outputs are camelCase; the client maps to the API's snake_case. Any non-2xx
answer throws `NotifydError` with `status`, `message` and `details`.

## Browser inbox

The browser never holds the project API key. Your backend mints a subscriber token
(`createSubscriberToken`) and the page opens the live stream with a one-time ticket:

```typescript
const inbox = createNotifydClient({ url, subscriberToken });
const { items } = await inbox.getInbox('user-42', { filter: 'unread' });
const stream = await inbox.openInboxStream('user-42', {
  onMessage: (e) => {
    const event = JSON.parse(e.data);
    if (event.type === 'new_notification') showToast(event.notification);
    if (event.type === 'count_update') updateBadge(event.unread_count);
  },
});
// later: stream.close()
```

## Workflows

```typescript
await notifyd.upsertWorkflow({
  id: 'welcome-series',
  name: 'Welcome series',
  triggerEvent: 'user.signup',
  steps: [
    { type: 'send', channel: 'email', template: 'welcome' },
    { type: 'delay', durationSecs: 86_400 },
    { type: 'condition', field: 'payload.plan', operator: 'eq', value: 'pro', onTrue: 4 },
    { type: 'send', channel: 'email', template: 'nudge' },
  ],
});
await notifyd.triggerWorkflow({ event: 'user.signup', subscriberId: 'user-42', payload: { plan: 'free' } });
```

Full HTTP contract: [`docs/llms.txt`](https://github.com/rmzlb/notifyd/blob/main/docs/llms.txt).
Build from source: `pnpm install && pnpm build:sdk` at the repository root.
