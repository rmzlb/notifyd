-- Suppression scope: 'all' (bounce, complaint, manual block) stops every
-- email to the address; 'marketing' (commercial unsubscribe) stops bulk
-- traffic only, transactional email still goes out.
ALTER TABLE email_suppressions ADD COLUMN IF NOT EXISTS scope TEXT NOT NULL DEFAULT 'all';
