-- Shareable invite links.
--
-- Lowers the administrative cost of onboarding: instead of the admin creating
-- one user per player by hand, the admin generates a link and shares it with a
-- group. Anyone holding the link can self-register a user in that link's
-- league via the public `/join/{token}` flow.
--
-- The token is the secret carried in the URL (same unguessable shape as the
-- magic-link token). Revoking a link simply deletes its row, after which the
-- public join flow no longer recognises the token. Links are league-scoped so
-- a join always lands the new user in the correct tenancy boundary.
CREATE TABLE invite_links (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    league_id UUID NOT NULL REFERENCES leagues(id) ON DELETE CASCADE,
    token VARCHAR(255) NOT NULL UNIQUE,
    -- Optional admin-facing note so multiple links can be told apart
    -- (e.g. "WhatsApp group", "office").
    label VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_invite_links_league ON invite_links(league_id);
