// npm i notifyd-sdk
import { createNotifydClient, NotifydError } from 'notifyd-sdk';

const notifyd = createNotifydClient({ url: process.env.NOTIFYD_URL!, apiKey: process.env.NOTIFYD_API_KEY! });

await notifyd.upsertSubscriber({ id: 'user-42', email: 'alice@example.com', firstName: 'Alice' });

const { jobIds } = await notifyd.send({
  channels: ['email', 'in_app'],
  subscriberId: 'user-42',
  subject: 'Your order shipped',
  body: 'Hi {{first_name}}, parcel {{parcel}} is on its way.',
  vars: { first_name: 'Alice', parcel: 'FR-2041' },
  idempotencyKey: 'order-2041-shipped',   // safe to retry: the same key never sends twice
});

await new Promise((r) => setTimeout(r, 1000));
for (const id of jobIds) {
  try {
    const job = await notifyd.getJob(id);
    console.log(`${job.channel}: ${job.status} via ${job.provider ?? '-'} after ${job.attempts} attempt(s)`);
  } catch (error) {
    if (error instanceof NotifydError) console.error(error.status, error.message);
    else throw error;
  }
}
