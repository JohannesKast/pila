-- Add optional email column to users for email-based notifications.
-- Email is nullable: users can log in via magic link alone.
ALTER TABLE users ADD COLUMN IF NOT EXISTS email VARCHAR(255);

-- Add unique constraint so two users in the same league cannot share an email.
-- Emails across different leagues may legitimately be the same person.
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_email_per_league ON users (league_id, email)
    WHERE email IS NOT NULL;

-- Extend sent_notifications to support per-user email tracking.
-- Group notifications (Signal) use user_id = NULL; individual email
-- notifications set user_id to the recipient.
--
-- Since PK columns cannot be NULL, we use a sentinel UUID (all zeros)
-- to represent "no specific user / group notification". We deliberately
-- do NOT add a FK constraint for user_id to avoid requiring a matching
-- users row for the sentinel.

-- 1. Add nullable column first
ALTER TABLE sent_notifications ADD COLUMN IF NOT EXISTS user_id UUID;

-- 2. Back-fill existing rows with the sentinel value
UPDATE sent_notifications SET user_id = '00000000-0000-0000-0000-000000000000' WHERE user_id IS NULL;

-- 3. Make NOT NULL
ALTER TABLE sent_notifications ALTER COLUMN user_id SET NOT NULL;

-- 4. Recreate PK with user_id included
ALTER TABLE sent_notifications DROP CONSTRAINT IF EXISTS sent_notifications_pkey;
ALTER TABLE sent_notifications ADD PRIMARY KEY (league_id, kind, ref_id, user_id);
