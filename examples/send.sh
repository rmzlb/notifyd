#!/usr/bin/env bash
# One call, two channels. notifyd queues, paces, retries and reports.
set -euo pipefail
: "${NOTIFYD_URL:?}" "${NOTIFYD_API_KEY:?}"

# A subscriber holds the addresses so your app only passes an id.
curl -sS -X POST "$NOTIFYD_URL/v1/subscribers" \
  -H "X-Api-Key: $NOTIFYD_API_KEY" -H "Content-Type: application/json" \
  -d '{"id": "user-42", "email": "alice@example.com", "first_name": "Alice", "timezone": "Europe/Paris"}' > /dev/null

JOB_IDS=$(curl -sS -X POST "$NOTIFYD_URL/v1/send" \
  -H "X-Api-Key: $NOTIFYD_API_KEY" -H "Content-Type: application/json" \
  -d '{
    "channels": ["email", "in_app"],
    "subscriber_id": "user-42",
    "subject": "Your order shipped",
    "body": "Hi {{first_name}}, parcel {{parcel}} is on its way.",
    "vars": {"first_name": "Alice", "parcel": "FR-2041"},
    "idempotency_key": "order-2041-shipped"
  }' | jq -r '.job_ids[]')

sleep 1
for id in $JOB_IDS; do
  curl -sS "$NOTIFYD_URL/v1/jobs/$id" -H "X-Api-Key: $NOTIFYD_API_KEY" \
    | jq -r '"\(.channel): \(.status) via \(.provider // "-") after \(.attempts) attempt(s)"'
done
