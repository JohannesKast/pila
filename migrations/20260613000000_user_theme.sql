-- Per-user colour theme preference.
--
-- The UI ships a dark "floodlight" theme and a light theme; users pick one via
-- the topbar toggle. The choice is applied client-side from the `pila_theme`
-- cookie (flash-free, before first paint) and mirrored here so it follows the
-- user across devices: on login the server seeds the cookie from this column.
--
-- Defaults to 'dark' to preserve the existing look for all current users.
ALTER TABLE users
    ADD COLUMN theme VARCHAR(5) NOT NULL DEFAULT 'dark'
    CHECK (theme IN ('dark', 'light'));
