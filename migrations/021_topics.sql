-- Topics: a subscriber-facing stream of messages ("release-notes", "tips",
-- "billing") that can be opted out of per channel, independently of the
-- template or workflow that produced the message. Preferences reuse the
-- `subscriber_preferences.workflow_id` scope column: a row whose scope is a
-- topic id applies to every job carrying that topic.
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS topic TEXT;
ALTER TABLE templates ADD COLUMN IF NOT EXISTS topic TEXT;
CREATE INDEX IF NOT EXISTS jobs_project_topic ON jobs (project_id, topic) WHERE topic IS NOT NULL;
