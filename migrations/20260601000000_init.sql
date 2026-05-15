-- Initial schema for Pila.
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

CREATE TABLE users (
    id UUID PRIMARY KEY,
    league_id UUID NOT NULL REFERENCES leagues(id),
    name VARCHAR(255) NOT NULL,
    token VARCHAR(255) NOT NULL UNIQUE,
    is_admin BOOLEAN NOT NULL DEFAULT FALSE,
    can_create_league BOOLEAN NOT NULL DEFAULT FALSE,
    language VARCHAR(5) NOT NULL DEFAULT 'de',
    jersey_preset TEXT NOT NULL DEFAULT 'classic',
    phone_number VARCHAR(32),
    email VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_users_league ON users(league_id);

-- Two users in the same league cannot share an email. Across different
-- leagues the same person may legitimately register twice.
CREATE UNIQUE INDEX idx_users_email_per_league ON users (league_id, email)
    WHERE email IS NOT NULL;

CREATE TABLE teams (
    id INTEGER PRIMARY KEY,
    name VARCHAR(64) NOT NULL,
    short_name VARCHAR(32),
    flag_code VARCHAR(8),
    group_letter CHAR(1)
);

CREATE TYPE match_stage AS ENUM (
    'group',
    'round_of_32',
    'round_of_16',
    'quarter_final',
    'semi_final',
    'third_place',
    'final'
);

CREATE TABLE matches (
    id SERIAL PRIMARY KEY,
    stage match_stage NOT NULL,
    group_letter CHAR(1),
    team_home_id INTEGER REFERENCES teams(id),
    team_away_id INTEGER REFERENCES teams(id),
    score_home INTEGER,
    score_away INTEGER,
    kickoff_time TIMESTAMPTZ,
    status VARCHAR(16) NOT NULL DEFAULT 'scheduled',
    espn_event_id BIGINT UNIQUE
);

CREATE INDEX idx_matches_stage ON matches(stage);
CREATE INDEX idx_matches_kickoff ON matches(kickoff_time);

CREATE TABLE predictions (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    match_id INTEGER NOT NULL REFERENCES matches(id),
    predicted_home INTEGER NOT NULL,
    predicted_away INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, match_id)
);

CREATE TABLE special_predictions (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    champion_id INTEGER REFERENCES teams(id),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Idempotency table for outbound notifications. Partitioned by league so two
-- leagues can each receive the same (kind, ref_id) independently.
--
-- Group notifications (Signal) use the sentinel user_id '00000000-…-0';
-- individual email notifications set user_id to the recipient. No FK on
-- user_id so the sentinel does not require a matching users row.
CREATE TABLE sent_notifications (
    league_id UUID NOT NULL REFERENCES leagues(id),
    kind VARCHAR(40) NOT NULL,
    ref_id INTEGER NOT NULL,
    user_id UUID NOT NULL,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (league_id, kind, ref_id, user_id)
);

CREATE TABLE settings (
    key VARCHAR(50) PRIMARY KEY,
    value TEXT
);
