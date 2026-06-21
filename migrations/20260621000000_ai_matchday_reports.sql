-- AI-generated matchday recaps shown at the top of the "Current" tab.
--
-- One recap per (league, matchday). A matchday is the calendar day (in the
-- tournament's configured timezone) on which the matches were played. The
-- recap is generated once by the background worker after the last match of
-- the matchday has finished, then stored here verbatim (Markdown). There is
-- intentionally no backfill for matchdays that finished before this feature
-- existed: the worker only ever generates the most recent finished matchday.
CREATE TABLE ai_matchday_reports (
    id BIGSERIAL PRIMARY KEY,
    league_id UUID NOT NULL REFERENCES leagues(id) ON DELETE CASCADE,
    -- Day the matches were played, in the configured tournament timezone.
    matchday_date DATE NOT NULL,
    -- Locale the recap was written in (the league's default language at
    -- generation time).
    language VARCHAR(5) NOT NULL,
    -- Markdown body of the recap.
    content TEXT NOT NULL,
    -- Provider/model identifier used to generate the recap, for debugging.
    model VARCHAR(128) NOT NULL,
    generated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (league_id, matchday_date)
);

CREATE INDEX idx_ai_reports_league_date
    ON ai_matchday_reports (league_id, matchday_date DESC);
