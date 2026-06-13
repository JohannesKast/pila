-- Enforce one display name per league, case-insensitively.
--
-- Re-opening the invite link, losing the magic link, or double-submitting the
-- join form used to create a fresh duplicate account carrying the same name
-- (e.g. three "rennitent" rows with zero predictions). This unique index makes
-- that impossible at the storage layer and is the race-safe backstop behind the
-- application-level check in the join / admin-create handlers.
--
-- IMPORTANT: existing duplicates must be resolved BEFORE this migration runs,
-- otherwise index creation fails and the app will not boot (migrations run at
-- startup). See doc/cleanup_duplicate_users.sql.
CREATE UNIQUE INDEX users_league_name_lower_key
    ON users (league_id, lower(name));
