#!/usr/bin/env bash
# A workflow: welcome now, wait a day, nudge unless the trigger said the user is on a paid plan.
# Steps are indexed from 0; a condition jumps with on_true / on_false; past the end = done.
set -euo pipefail
: "${NOTIFYD_URL:?}" "${NOTIFYD_API_KEY:?}"
H=(-H "X-Api-Key: $NOTIFYD_API_KEY" -H "Content-Type: application/json")

curl -sS -X POST "$NOTIFYD_URL/v1/templates" "${H[@]}" -d '{"id": "welcome", "channel": "email", "subject": "Welcome {{first_name}}", "body": "Glad you are here."}' > /dev/null
curl -sS -X POST "$NOTIFYD_URL/v1/templates" "${H[@]}" -d '{"id": "nudge", "channel": "email", "subject": "Need a hand?", "body": "Reply to this email and a human answers."}' > /dev/null

curl -sS -X POST "$NOTIFYD_URL/v1/workflows" "${H[@]}" -d '{
  "id": "welcome-series",
  "name": "Welcome series",
  "trigger_event": "user.signup",
  "steps": [
    {"type": "send", "channel": "email", "template": "welcome"},
    {"type": "delay", "duration_secs": 86400},
    {"type": "condition", "field": "payload.plan", "operator": "eq", "value": "pro", "on_true": 4},
    {"type": "send", "channel": "email", "template": "nudge"}
  ]
}' > /dev/null

# Fire the event from your signup handler:
curl -sS -X POST "$NOTIFYD_URL/v1/workflows/trigger" "${H[@]}" \
  -d '{"event": "user.signup", "subscriber_id": "user-42", "payload": {"plan": "free"}}' | jq .

# Watch it pause on the delay (status "paused", resume_at set) and resume on its own:
curl -sS "$NOTIFYD_URL/v1/workflows/runs?workflow_id=welcome-series" "${H[@]}" | jq '.items[] | {status, current_step, resume_at}'
