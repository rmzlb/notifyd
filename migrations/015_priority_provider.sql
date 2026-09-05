-- Priority lanes and provider evidence on jobs.
--
-- priority: 0 = most urgent … 100 = bulk. The worker claims jobs ordered by
-- (priority, scheduled_at), so a marketing fan-out (80) never delays a
-- transactional email (50) or a security code (10).
-- provider / provider_message_id: which connector accepted the message and
-- the provider's own identifier, for support and webhook correlation.
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS priority SMALLINT NOT NULL DEFAULT 50;
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS provider TEXT;
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS provider_message_id TEXT;

-- The claim query: status IN ('pending','retry') ORDER BY priority, scheduled_at.
CREATE INDEX IF NOT EXISTS jobs_claim_priority
    ON jobs (priority, scheduled_at)
    WHERE status IN ('pending', 'retry');

CREATE INDEX IF NOT EXISTS jobs_provider_message_id
    ON jobs (provider, provider_message_id)
    WHERE provider_message_id IS NOT NULL;

-- Five attempts with the worker's backoff (30 s, 2 min, 10 min, 30 min, 2 h)
-- ride out a provider incident of about 45 minutes. The API binds the
-- configured value; this default covers rows inserted by the workflow engine.
ALTER TABLE jobs ALTER COLUMN max_attempts SET DEFAULT 5;
