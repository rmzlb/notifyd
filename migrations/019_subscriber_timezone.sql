-- Recipient timezone (IANA name, e.g. Europe/Paris) for send windows.
ALTER TABLE subscribers ADD COLUMN IF NOT EXISTS timezone TEXT;
