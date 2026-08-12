-- Idempotency keys, Stripe-like semantics.
--
-- The total UNIQUE(project_id, idempotency_key) had two failure modes,
-- both observed in production:
--   1. a permanently failed job held its key forever — the same logical
--      notification could never be sent again;
--   2. the INSERT's ON CONFLICT DO UPDATE reset the existing job's status
--      to 'pending' on key reuse, so re-POSTing a key RE-SENT an already
--      delivered email (measured 2026-08-12: sent -> pending -> sent,
--      attempts 1 -> 2). The opposite of what an idempotency key promises.
--
-- The key must dedupe work that is in flight or succeeded — a terminal
-- failure releases it, and reusing a live key returns the existing job
-- untouched (enforced in src/api/send.rs alongside this index).

ALTER TABLE jobs DROP CONSTRAINT IF EXISTS jobs_project_id_idempotency_key_key;

CREATE UNIQUE INDEX IF NOT EXISTS jobs_active_idempotency_key
    ON jobs (project_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL
      AND status NOT IN ('failed', 'cancelled');
