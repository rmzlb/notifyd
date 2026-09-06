-- Project API keys: keep only their SHA-256 hashes.
--
-- Authentication has compared hashes since 003 (`api_key_hash`,
-- `secondary_api_key_hash`); the plaintext columns were write-only and a
-- database dump exposed every project key. Drop them.
--
-- Rows without a hash (the seed in 005 has a random plaintext key and no
-- hash) were unusable before and stay unusable: hashes are deliberately NOT
-- backfilled from plaintext.
ALTER TABLE projects DROP COLUMN IF EXISTS api_key;
ALTER TABLE projects DROP COLUMN IF EXISTS secondary_api_key;

CREATE UNIQUE INDEX IF NOT EXISTS projects_api_key_hash_idx ON projects (api_key_hash);
CREATE INDEX IF NOT EXISTS projects_secondary_api_key_hash_idx ON projects (secondary_api_key_hash);
