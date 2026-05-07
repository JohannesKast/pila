CREATE TABLE users (
    id UUID PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    token VARCHAR(255) NOT NULL UNIQUE,
    is_admin BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

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
    user_id UUID NOT NULL REFERENCES users(id),
    match_id INTEGER NOT NULL REFERENCES matches(id),
    predicted_home INTEGER NOT NULL,
    predicted_away INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, match_id)
);

CREATE TABLE special_predictions (
    user_id UUID PRIMARY KEY REFERENCES users(id),
    champion_id INTEGER REFERENCES teams(id),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE sent_notifications (
    kind VARCHAR(40) NOT NULL,
    ref_id INTEGER NOT NULL,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (kind, ref_id)
);

CREATE TABLE settings (
    key VARCHAR(50) PRIMARY KEY,
    value TEXT
);
