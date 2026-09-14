-- Migration 024: per-user Telegram credentials, destinations, and link tokens.
--
-- Bot tokens and webhook secrets are stored encrypted (AES-256-GCM) using
-- TELEGRAM_ENCRYPTION_KEY (base64 32 bytes). Blob layout: [nonce(12)|ct|tag(16)].
--
-- telegram_credentials.id is the "route_id" carried in
-- jobs.payload->'chat'->>'telegram_route_id'. The worker loads credentials
-- by (id, project_id) so routes are always project-scoped and never fall back.
--
-- webhook_connection_id is the UUID in the notifyd-internal callback path:
--   notifyd: POST /v1/telegram/webhooks/<connection_id>
--   Baaton public proxy: POST /api/v1/public/telegram/webhook/<connection_id>
-- Baaton forwards the Telegram update + X-Telegram-Bot-Api-Secret-Token header
-- unchanged; notifyd validates the header against the decrypted webhook_secret.

CREATE TABLE telegram_credentials (
    id                    UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id            TEXT        NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    owner                 TEXT        NOT NULL,
    -- AES-256-GCM encrypted bot token: [nonce(12) | ciphertext | tag(16)]
    bot_token_enc         BYTEA       NOT NULL,
    bot_username          TEXT        NOT NULL,
    -- Identifies the per-bot webhook callback path.
    webhook_connection_id UUID        NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    -- AES-256-GCM encrypted webhook header secret.
    webhook_secret_enc    BYTEA       NOT NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, owner)
);

CREATE TABLE telegram_destinations (
    id                 UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id         TEXT        NOT NULL,
    owner              TEXT        NOT NULL,
    credential_id      UUID        NOT NULL REFERENCES telegram_credentials(id) ON DELETE CASCADE,
    -- Numeric Telegram chat id stored as text ("-1001234567890", "12345678").
    address            TEXT        NOT NULL,
    -- Positive integer; NULL unless the chat is a supergroup forum topic.
    telegram_thread_id BIGINT,
    -- True after getChat validation succeeds.
    verified           BOOLEAN     NOT NULL DEFAULT false,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, owner)
);

CREATE TABLE telegram_link_tokens (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id    TEXT        NOT NULL,
    owner         TEXT        NOT NULL,
    credential_id UUID        NOT NULL REFERENCES telegram_credentials(id) ON DELETE CASCADE,
    kind          TEXT        NOT NULL CHECK (kind IN ('private', 'group')),
    -- SHA-256(raw_token) as hex; the raw token only lives in the deep link URL.
    token_hash    TEXT        NOT NULL,
    expires_at    TIMESTAMPTZ NOT NULL,
    consumed_at   TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Fast webhook dispatch by connection id.
CREATE INDEX telegram_credentials_conn_idx
    ON telegram_credentials (webhook_connection_id);

-- Cascade lookups.
CREATE INDEX telegram_destinations_cred_idx
    ON telegram_destinations (credential_id);

-- Unconsumed token lookup.
CREATE INDEX telegram_link_tokens_hash_idx
    ON telegram_link_tokens (token_hash)
    WHERE consumed_at IS NULL;
