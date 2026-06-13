-- One-off cleanup of duplicate self-registered users.
--
-- Background: before the per-league name uniqueness was enforced, re-opening
-- the invite link, losing the magic link, or double-submitting the join form
-- created a brand-new account with the same display name. This left "zombie"
-- duplicates (same name, zero activity) cluttering the admin view.
--
-- This script removes only the abandoned zombies and then REPORTS any
-- collisions where two genuinely active accounts share a name — those need a
-- human decision and cannot be deleted without losing real predictions.
--
-- Run this BEFORE deploying migration 20260613120000_unique_user_name_per_league
-- (the unique index will not build while duplicates remain).
--
-- Review Step 0 output, then run the whole file in one transaction.

BEGIN;

-- ── Step 0 (dry run): list every set of same-name accounts in a league ──
-- "predictions" and "has_champion" reveal which row is the real, active one.
SELECT u.league_id,
       u.name,
       u.id,
       u.created_at,
       (SELECT count(*) FROM predictions p WHERE p.user_id = u.id) AS predictions,
       EXISTS (SELECT 1 FROM special_predictions sp
               WHERE sp.user_id = u.id AND sp.champion_id IS NOT NULL) AS has_champion
FROM users u
WHERE EXISTS (
    SELECT 1 FROM users o
    WHERE o.league_id = u.league_id
      AND lower(o.name) = lower(u.name)
      AND o.id <> u.id
)
ORDER BY u.league_id, lower(u.name), u.created_at;

-- ── Step 1: delete the abandoned duplicates ──
-- Within each (league_id, lower(name)) group keep one survivor — the row with
-- the most predictions, ties broken by the earliest created_at (the original).
-- Delete the remaining rows ONLY when they have no predictions and no champion
-- pick. Dependent rows vanish via ON DELETE CASCADE.
WITH ranked AS (
    SELECT u.id,
           u.league_id,
           lower(u.name) AS lname,
           u.created_at,
           (SELECT count(*) FROM predictions p WHERE p.user_id = u.id) AS preds,
           EXISTS (SELECT 1 FROM special_predictions sp
                   WHERE sp.user_id = u.id AND sp.champion_id IS NOT NULL) AS has_champ
    FROM users u
),
keepers AS (
    SELECT DISTINCT ON (league_id, lname) id
    FROM ranked
    ORDER BY league_id, lname, preds DESC, created_at ASC
)
DELETE FROM users
WHERE id IN (
    SELECT r.id
    FROM ranked r
    WHERE r.id NOT IN (SELECT id FROM keepers)
      AND r.preds = 0
      AND NOT r.has_champ
);

-- ── Step 2: report unresolved collisions (two ACTIVE accounts, same name) ──
-- If this returns rows (e.g. two different people both named
-- "Freiheitskämpfer", each with their own predictions), the unique index will
-- still fail to build. Resolve each by renaming one account — via the admin UI
-- or directly, e.g.:
--   UPDATE users SET name = 'Freiheitskämpfer (2)'
--   WHERE id = '49e2508e-177c-459e-8450-a51351a40045';
SELECT u.league_id,
       u.name,
       u.id,
       (SELECT count(*) FROM predictions p WHERE p.user_id = u.id) AS predictions
FROM users u
WHERE EXISTS (
    SELECT 1 FROM users o
    WHERE o.league_id = u.league_id
      AND lower(o.name) = lower(u.name)
      AND o.id <> u.id
)
ORDER BY u.league_id, lower(u.name), u.created_at;

COMMIT;
