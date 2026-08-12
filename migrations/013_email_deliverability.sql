-- Email deliverability: ingest Resend webhooks (delivered / bounced /
-- complained) and stop sending to addresses that bounced or complained.
--
-- Numbered 013 on purpose: 008-012 are reserved by in-flight work on another
-- branch. sqlx applies any local migration missing from _sqlx_migrations
-- regardless of order, so the gap is safe.

-- Per-project suppression list. A row with released_at IS NULL blocks every
-- email send to that address for that project; releasing it (soft, audited)
-- re-allows sends. History is kept — release, don't delete.
CREATE TABLE IF NOT EXISTS email_suppressions (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id        TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    email             TEXT NOT NULL,
    reason            TEXT NOT NULL, -- 'bounce' | 'complaint' | 'manual'
    detail            TEXT,          -- provider message (e.g. bounce.message)
    source_job_id     UUID,          -- job whose send triggered the suppression
    provider_event_id TEXT,          -- svix message id of the triggering event
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    released_at       TIMESTAMPTZ,
    released_by       TEXT
);

-- One ACTIVE suppression per (project, address); released rows stay as history.
CREATE UNIQUE INDEX IF NOT EXISTS idx_email_suppressions_active
    ON email_suppressions (project_id, lower(email))
    WHERE released_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_email_suppressions_project
    ON email_suppressions (project_id, created_at DESC);

-- Raw provider events, keyed by the svix message id: the primary key makes
-- webhook ingestion idempotent (Resend retries deliveries), and the payload
-- is the audit trail when a suppression needs explaining months later.
CREATE TABLE IF NOT EXISTS provider_events (
    id          TEXT PRIMARY KEY,
    provider    TEXT NOT NULL DEFAULT 'resend',
    event_type  TEXT NOT NULL,
    job_id      UUID,
    payload     JSONB NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_provider_events_job
    ON provider_events (job_id) WHERE job_id IS NOT NULL;

-- Delivery lifecycle on jobs. 'sent' means the provider accepted the API
-- call; these record what actually happened afterwards.
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS delivered_at TIMESTAMPTZ;
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS bounced_at   TIMESTAMPTZ;
