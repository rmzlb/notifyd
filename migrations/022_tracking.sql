-- Own open / click tracking (src/tracking.rs): first open and first click of
-- an email, recorded by this instance's pixel and redirect links. Every hit
-- is also a provider_events row with provider = 'notifyd'.
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS opened_at  TIMESTAMPTZ;
ALTER TABLE jobs ADD COLUMN IF NOT EXISTS clicked_at TIMESTAMPTZ;
