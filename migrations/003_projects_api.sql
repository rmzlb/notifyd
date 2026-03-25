-- Extend projects table for API management
ALTER TABLE projects ADD COLUMN IF NOT EXISTS api_key_hash TEXT;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS secondary_api_key TEXT;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS secondary_api_key_hash TEXT;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS rate_limit_per_min INT DEFAULT 600;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS settings JSONB DEFAULT '{}';
ALTER TABLE projects ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ DEFAULT now();

-- Audit log
CREATE TABLE IF NOT EXISTS audit_log (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  TEXT NOT NULL,
    actor       TEXT NOT NULL,           -- "api_key", "subscriber:xxx", "system"
    action      TEXT NOT NULL,           -- "send", "schedule", "read_inbox", "update_preference"
    resource    TEXT,                    -- "job:uuid", "inbox:uuid", "subscriber:id"
    metadata    JSONB DEFAULT '{}',
    ip          TEXT,
    created_at  TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX idx_audit_project ON audit_log (project_id, created_at DESC);
CREATE INDEX idx_audit_actor ON audit_log (project_id, actor, created_at DESC);

-- Rate limit tracking (sliding window in-memory, but log hits for analysis)
CREATE TABLE IF NOT EXISTS rate_limit_hits (
    project_id  TEXT NOT NULL,
    window_start TIMESTAMPTZ NOT NULL,
    count       INT NOT NULL DEFAULT 0,
    PRIMARY KEY (project_id, window_start)
);
