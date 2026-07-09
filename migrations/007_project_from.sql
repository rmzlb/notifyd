-- Per-project sender identity for the email connector.
-- NULL = fall back to the instance-wide [connectors.email] from/from_name.
ALTER TABLE projects ADD COLUMN IF NOT EXISTS from_email TEXT;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS from_name TEXT;
