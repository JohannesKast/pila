-- Multi-League / Multi-Tenancy
--
-- A league is an isolated tipping context. Users belong to exactly one league,
-- and predictions/badges/leaderboard data must never bleed across leagues.
-- Matches stay global (everyone tips on the same World Cup fixtures); the
-- partitioning happens at the user-relationship layer.
--
-- The first league is created interactively by the `/setup` flow together
-- with the first admin user — there is intentionally no seeded "Default"
-- league, so a fresh deploy never has dangling tenancy without an owner.

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

ALTER TABLE users ADD COLUMN league_id UUID NOT NULL REFERENCES leagues(id);
ALTER TABLE users ADD COLUMN can_create_league BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX idx_users_league ON users(league_id);

-- sent_notifications must be partitioned by league so two leagues can each
-- receive (kind, ref_id) independently — the previous PK (kind, ref_id) would
-- have made one league's send suppress the other's.
ALTER TABLE sent_notifications DROP CONSTRAINT sent_notifications_pkey;
ALTER TABLE sent_notifications ADD COLUMN league_id UUID NOT NULL REFERENCES leagues(id);
ALTER TABLE sent_notifications ADD PRIMARY KEY (league_id, kind, ref_id);
