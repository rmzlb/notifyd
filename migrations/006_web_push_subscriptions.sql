ALTER TABLE push_tokens
    ADD COLUMN IF NOT EXISTS endpoint TEXT,
    ADD COLUMN IF NOT EXISTS p256dh TEXT,
    ADD COLUMN IF NOT EXISTS auth TEXT,
    ADD COLUMN IF NOT EXISTS expiration_time TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS user_agent TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_push_tokens_project_subscriber_endpoint
    ON push_tokens (project_id, subscriber_id, endpoint)
    WHERE endpoint IS NOT NULL;
