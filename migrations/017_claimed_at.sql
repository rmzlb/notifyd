-- When the worker claimed a job. A job still 'processing' long after its
-- claim belongs to a worker that died mid-send: the reaper re-queues it.
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS claimed_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS jobs_processing_claimed_at
    ON jobs (claimed_at)
    WHERE status = 'processing';
