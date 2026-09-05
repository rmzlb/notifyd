-- One-time SSE stream tickets, shared by every replica. Formerly kept in
-- process memory, which pinned the service to a single replica.
CREATE TABLE IF NOT EXISTS sse_tickets (
    id            TEXT PRIMARY KEY,
    project_id    TEXT NOT NULL,
    subscriber_id TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS sse_tickets_created_at ON sse_tickets (created_at);
