-- notifyd database schema

CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- Projects (one per client app)
CREATE TABLE IF NOT EXISTS projects (
    id          TEXT PRIMARY KEY,
    api_key     TEXT UNIQUE NOT NULL,
    name        TEXT NOT NULL,
    channels    TEXT[] DEFAULT '{}',
    created_at  TIMESTAMPTZ DEFAULT now()
);

-- Subscribers (users per project)
CREATE TABLE IF NOT EXISTS subscribers (
    id          TEXT NOT NULL,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    email       TEXT,
    phone       TEXT,
    first_name  TEXT,
    last_name   TEXT,
    locale      TEXT DEFAULT 'fr',
    data        JSONB DEFAULT '{}',
    created_at  TIMESTAMPTZ DEFAULT now(),
    updated_at  TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (project_id, id)
);

CREATE INDEX idx_subscribers_email ON subscribers (project_id, email);

-- Templates (optional, overrides inline body)
CREATE TABLE IF NOT EXISTS templates (
    id          TEXT NOT NULL,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    channel     TEXT NOT NULL,
    subject     TEXT,
    body        TEXT NOT NULL,
    body_html   TEXT,
    created_at  TIMESTAMPTZ DEFAULT now(),
    updated_at  TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (project_id, id, channel)
);

-- Jobs (notification queue)
CREATE TABLE IF NOT EXISTS jobs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id      TEXT NOT NULL REFERENCES projects(id),
    channel         TEXT NOT NULL,
    subscriber_id   TEXT,
    recipient       TEXT NOT NULL,
    template_id     TEXT,
    payload         JSONB NOT NULL DEFAULT '{}',
    status          TEXT NOT NULL DEFAULT 'pending',
    scheduled_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts        INT NOT NULL DEFAULT 0,
    max_attempts    INT NOT NULL DEFAULT 3,
    next_retry_at   TIMESTAMPTZ,
    idempotency_key TEXT,
    created_at      TIMESTAMPTZ DEFAULT now(),
    sent_at         TIMESTAMPTZ,
    error           TEXT,
    UNIQUE (project_id, idempotency_key)
);

CREATE INDEX idx_jobs_queue ON jobs (scheduled_at)
    WHERE status IN ('pending', 'retry');
CREATE INDEX idx_jobs_status ON jobs (status, scheduled_at);
CREATE INDEX idx_jobs_subscriber ON jobs (project_id, subscriber_id);

-- In-app inbox messages
CREATE TABLE IF NOT EXISTS inbox_messages (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id      TEXT NOT NULL,
    subscriber_id   TEXT NOT NULL,
    body            TEXT NOT NULL,
    icon            TEXT DEFAULT 'bell',
    url             TEXT,
    data            JSONB DEFAULT '{}',
    read_at         TIMESTAMPTZ,
    archived_at     TIMESTAMPTZ,
    is_todo         BOOLEAN NOT NULL DEFAULT false,
    created_at      TIMESTAMPTZ DEFAULT now(),
    FOREIGN KEY (project_id, subscriber_id) REFERENCES subscribers(project_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_inbox_subscriber ON inbox_messages (project_id, subscriber_id, created_at DESC)
    WHERE archived_at IS NULL;
CREATE INDEX idx_inbox_unread ON inbox_messages (project_id, subscriber_id)
    WHERE read_at IS NULL AND archived_at IS NULL;
