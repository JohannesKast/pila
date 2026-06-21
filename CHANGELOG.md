# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- AI matchday recaps: after the last match of a matchday (grouped by the configurable tournament timezone `AI_MATCHDAY_TZ`) finishes, the worker generates one entertaining, league-scoped recap in the league's default language, stored in `ai_matchday_reports` and shown at the top of the "Current" tab with arrow navigation between matchdays. Supports Gemini and OpenAI-compatible providers via `AI_PROVIDER`/`AI_MODEL`/`AI_API_KEY` plus optional `AI_BASE_URL` (feature off when unset); only public (post-lock) tips feed the prompt and players are referenced by display name only
- Multi-tenancy (Tipp-Liga): every user belongs to exactly one league; aggregate reads (leaderboard, badges, tips panel, notifications) are fully scoped by `league_id`
- Per-league configuration via `league_settings` k/v table (Signal group, default language, RSS feed, scoring system, knockout-only mode)
- Super-admin `can_create_league` permission; first user created via `/setup` receives it
- Magic-link cookie auth (`pila_token`)
- ESPN `soccer/fifa.world` scoreboard sync worker (30-min poll window over configurable date range)
- Idempotent notification dispatch: `match_closing_soon` (< 24 h before kickoff) and `special_lock_soon` (< 24 h before the first knockout kickoff)
- Bootstrap silence: first worker tick after deploy does not flood the Signal group with past matches
- Quiet-hour gate (22:00–08:00 Europe/Berlin) for notifications
- Two scoring systems: `ExactScore` (4/3/2/0 → 11/8/6/0 by phase) and `WinnerOnly` (1/0 → 7/0 by phase); champion pick worth flat 10 pts
- Badge engine: 30+ badges computed on-the-fly per request from a shared `BadgeContext`; no persistent badge table
- Internationalisation: `de`, `en`, `es`, `fr` via Mozilla Fluent FTL files; per-user language stored in `users.language`
- Group standings calculator
- Dev-mode routes for simulating tournament progression (mock time, random tips, random results, next-matchday simulation)
- Signal-cli REST notifier + `NoopNotifier` fallback when Signal env vars are absent
- Email notification support (SMTP) with per-user opt-in
- Operational scripts: `create_invite.sh`, `admin_edit.sh`, `backup_db.sh`, `restore_db.sh`, `signal_send.sh`
- Docker Compose stack with read-only root FS, dropped capabilities, and CPU/memory limits

### Changed

- Champion (Weltmeister) pick now stays editable until the knockout stage begins, in every league — previously it locked at the first match kickoff unless the league was knockout-only
- All user-facing strings moved from hardcoded German to Fluent FTL locale files
- Repository layer split into trait / Postgres impl / in-memory fake per module
- `AppState.db: Option<PgPool>` replaced by typed `Repos` bundle; `BootstrapRepo` encapsulates setup transaction
- Single squashed migration (`20260601000000_init.sql`) for a clean v0.1 starting point

### Fixed

- `MemoryUserRepo::find_by_token` returned hardcoded `language: "de"` instead of stored value
- `MemoryUserRepo::set_language` was a no-op stub

[Unreleased]: https://github.com/johanneskast/pila/compare/v0.1.0-rc1...HEAD
