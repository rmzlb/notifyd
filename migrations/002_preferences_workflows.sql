-- Subscriber notification preferences
CREATE TABLE IF NOT EXISTS subscriber_preferences (
    project_id      TEXT NOT NULL,
    subscriber_id   TEXT NOT NULL,
    channel         TEXT NOT NULL,           -- "email", "sms", "in_app", "push", "*" (all)
    workflow_id     TEXT DEFAULT '*',        -- specific workflow or "*" for all
    enabled         BOOLEAN NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ DEFAULT now(),
    updated_at      TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (project_id, subscriber_id, channel, workflow_id),
    FOREIGN KEY (project_id, subscriber_id) REFERENCES subscribers(project_id, id) ON DELETE CASCADE
);

-- Workflows (multi-step notification pipelines)
CREATE TABLE IF NOT EXISTS workflows (
    id              TEXT NOT NULL,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    description     TEXT,
    trigger_event   TEXT NOT NULL,           -- "purchase.approved", "appointment.reminder"
    steps           JSONB NOT NULL DEFAULT '[]',  -- ordered array of steps
    enabled         BOOLEAN NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ DEFAULT now(),
    updated_at      TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (project_id, id)
);

-- Workflow runs (active executions)
CREATE TABLE IF NOT EXISTS workflow_runs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id      TEXT NOT NULL,
    workflow_id     TEXT NOT NULL,
    subscriber_id   TEXT NOT NULL,
    trigger_payload JSONB NOT NULL DEFAULT '{}',
    current_step    INT NOT NULL DEFAULT 0,
    status          TEXT NOT NULL DEFAULT 'running',  -- running, paused, completed, failed, cancelled
    step_state      JSONB NOT NULL DEFAULT '{}',      -- per-step state (digest buffer, etc.)
    resume_at       TIMESTAMPTZ,                      -- for delay steps
    created_at      TIMESTAMPTZ DEFAULT now(),
    updated_at      TIMESTAMPTZ DEFAULT now(),
    FOREIGN KEY (project_id, workflow_id) REFERENCES workflows(project_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_workflow_runs_resume ON workflow_runs (resume_at)
    WHERE status = 'paused' AND resume_at IS NOT NULL;
CREATE INDEX idx_workflow_runs_project ON workflow_runs (project_id, workflow_id, status);

-- Digest buffer (for digest steps that batch notifications)
CREATE TABLE IF NOT EXISTS digest_buffer (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id          UUID NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    payload         JSONB NOT NULL,
    created_at      TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX idx_digest_buffer_run ON digest_buffer (run_id, created_at);

-- Push tokens for FCM/APNs
CREATE TABLE IF NOT EXISTS push_tokens (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id      TEXT NOT NULL,
    subscriber_id   TEXT NOT NULL,
    token           TEXT NOT NULL,
    platform        TEXT NOT NULL DEFAULT 'fcm',  -- "fcm", "apns", "web"
    device_name     TEXT,
    created_at      TIMESTAMPTZ DEFAULT now(),
    last_used_at    TIMESTAMPTZ,
    FOREIGN KEY (project_id, subscriber_id) REFERENCES subscribers(project_id, id) ON DELETE CASCADE,
    UNIQUE (project_id, subscriber_id, token)
);
