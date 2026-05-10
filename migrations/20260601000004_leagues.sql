-- Multi-League / Multi-Tenancy
--
-- A league is an isolated tipping context. Users belong to exactly one league,
-- and predictions/badges/leaderboard data must never bleed across leagues.
-- Matches stay global (everyone tips on the same World Cup fixtures); the
-- partitioning happens at the user-relationship layer.

CREATE TABLE leagues (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(255) NOT NULL,
    notifications_bootstrapped BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Per-league key/value settings. New settings (default language, RSS feed,
-- Signal config, future additions) just need a new key — no migration.
CREATE TABLE league_settings (
    league_id UUID NOT NULL REFERENCES leagues(id) ON DELETE CASCADE,
    key VARCHAR(255) NOT NULL,
    value TEXT,
    PRIMARY KEY (league_id, key)
);

-- Default league for existing users. notifications_bootstrapped = true so the
-- worker does not re-flood the group on first deploy of multi-league code.
INSERT INTO leagues (id, name, notifications_bootstrapped)
VALUES ('00000000-0000-0000-0000-000000000001', 'Default', true);

ALTER TABLE users ADD COLUMN league_id UUID REFERENCES leagues(id);
UPDATE users SET league_id = '00000000-0000-0000-0000-000000000001';
ALTER TABLE users ALTER COLUMN league_id SET NOT NULL;

ALTER TABLE users ADD COLUMN can_create_league BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX idx_users_league ON users(league_id);

-- sent_notifications must be partitioned by league so two leagues can each
-- receive (kind, ref_id) independently — the previous PK (kind, ref_id) would
-- have made one league's send suppress the other's.
ALTER TABLE sent_notifications DROP CONSTRAINT sent_notifications_pkey;
ALTER TABLE sent_notifications ADD COLUMN league_id UUID REFERENCES leagues(id);
UPDATE sent_notifications SET league_id = '00000000-0000-0000-0000-000000000001';
ALTER TABLE sent_notifications ALTER COLUMN league_id SET NOT NULL;
ALTER TABLE sent_notifications ADD PRIMARY KEY (league_id, kind, ref_id);
