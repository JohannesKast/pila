-- Give every user a private real name alongside their public tip name.
--
-- Players now have two names:
--   * `name`      — the public *tip name*, shown on the leaderboard and in
--                   other players' comments. Stays unique per league and can be
--                   changed at any time.
--   * `real_name` — the player's real first name. Only league admins can see
--                   it; it never appears to other players. This makes it easier
--                   for an admin to keep track of who is behind a playful tip
--                   name. Not unique — two players may share a first name.
--
-- The instance is already in production, so existing users have no real name on
-- record. We backfill `real_name` from the current tip name and only then
-- enforce NOT NULL, so the default real name equals the tip name. New columns
-- are added nullable first specifically to make this backfill possible.
ALTER TABLE users ADD COLUMN real_name VARCHAR(255);
UPDATE users SET real_name = name WHERE real_name IS NULL;
ALTER TABLE users ALTER COLUMN real_name SET NOT NULL;
