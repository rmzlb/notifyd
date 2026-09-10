"""A newsletter to 10 000 people in one call.

What notifyd does for you here:
- one job per recipient, created in a single transaction (44k jobs/s on a laptop);
- the campaign tag puts them in the *bulk* lane: a password reset sent meanwhile goes first;
- `send_window` holds each email until it is daytime for *that* recipient (subscriber timezone);
- marketing mail gets RFC 8058 one-click unsubscribe headers; a click lands in the suppression list;
- a provider 429 pauses the channel for Retry-After without burning an attempt; 5xx fail over.
"""
import os

from notifyd import Notifyd

nd = Notifyd(os.environ["NOTIFYD_URL"], api_key=os.environ["NOTIFYD_API_KEY"])

recipients = [f"user-{i}" for i in range(10_000)]  # subscriber ids you already upserted

nd.upsert_template(
    "september-news",
    channel="email",
    subject="What's new this month, {{first_name}}",
    body="Hi {{first_name}}, here is what shipped in September…",
)

result = nd.batch(
    subscribers=recipients,
    channel="email",
    template="september-news",
    priority="bulk",
    idempotency_key="newsletter-2026-09",  # re-running this script creates nothing twice
    send_window={"start": "09:00", "end": "20:00", "days": [1, 2, 3, 4, 5]},
)
print(f"{result['jobs_created']} queued, {result['jobs_deduplicated']} already sent")
