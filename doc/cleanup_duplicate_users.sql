-- One-off cleanup of duplicate self-registered users.
--
-- Background: before the per-league name uniqueness was enforced, re-opening
-- the invite link, losing the magic link, or double-submitting the join form
-- created a brand-new account with the same display name. Some duplicates are
-- empty zombies (zero tips); others carry real predictions made by the same
-- person from a second account (observed: two "Freiheitskämpfer", 72 tips each;
-- two "GoalGetter", 1 tip each).
--
-- Decision rule: within each (league_id, lower(name)) group, keep the account
-- whose LAST tip is the most recent — that is the instance the player actually
-- kept using — and delete the other duplicates. Accounts with no tips rank last
-- (NULL last tip) and are always dropped when a same-name sibling exists. Ties
-- are broken by tip count, then by the older account. Dependent rows
-- (predictions, special_predictions) vanish via ON DELETE CASCADE.
--
-- Run this BEFORE deploying migration 20260613120000_unique_user_name_per_league
-- (the unique index will not build while duplicates remain). The whole script
-- runs in one transaction and aborts itself if any collision would survive, so
-- it is safe to run as-is; review the Step 0 output it prints.

BEGIN;

-- Rank every user within its same-name group. rn = 1 is the keeper.
CREATE TEMP TABLE dup_ranked ON COMMIT DROP AS
WITH activity AS (
    SELECT u.id,
           u.league_id,
           lower(u.name) AS lname,
           u.name,
           u.created_at,
           count(p.*)                                AS tips,
           max(GREATEST(p.created_at, p.updated_at)) AS last_tip
    FROM users u
    LEFT JOIN predictions p ON p.user_id = u.id
    GROUP BY u.id, u.league_id, u.name, u.created_at
)
SELECT a.*,
       count(*) OVER (PARTITION BY league_id, lname) AS group_size,
       row_number() OVER (
           PARTITION BY league_id, lname
           ORDER BY last_tip DESC NULLS LAST, tips DESC, created_at ASC
       ) AS rn
FROM activity a;

-- ── Step 0 (preview): the decision for every duplicate group ──
-- "action" = keep for the survivor (rn = 1, most recent tip), delete otherwise.
SELECT league_id,
       name,
       id,
       tips,
       last_tip,
       CASE WHEN rn = 1 THEN 'keep' ELSE 'DELETE' END AS action
FROM dup_ranked
WHERE group_size > 1
ORDER BY league_id, lname, rn;

-- ── Step 1: delete the losing duplicates ──
DELETE FROM users
WHERE id IN (SELECT id FROM dup_ranked WHERE group_size > 1 AND rn > 1);

-- ── Step 2: safety net — abort if any same-name collision still remains ──
-- Rolls back the whole transaction rather than leaving the DB in a state where
-- the unique index cannot be built.
DO $$
DECLARE remaining int;
BEGIN
    SELECT count(*) INTO remaining
    FROM (
        SELECT 1 FROM users GROUP BY league_id, lower(name) HAVING count(*) > 1
    ) dups;
    IF remaining > 0 THEN
        RAISE EXCEPTION 'cleanup left % duplicate name group(s); aborting', remaining;
    END IF;
END $$;

COMMIT;
