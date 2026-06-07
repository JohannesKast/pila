# Architecture

This document is the human-readable architecture overview for Pila. It
explains the current module layout, the database model, the tenancy rules,
and the background flows that matter when changing behaviour. Agent-specific
workflow rules live in [`../CLAUDE.md`](../CLAUDE.md); contributor workflow
and local setup live in [`../CONTRIBUTING.md`](../CONTRIBUTING.md).

## What Pila Is

Pila is a self-hostable FIFA World Cup 2026 prediction game written in Rust.
Users submit one prediction per match plus one champion pick, collect points
according to a fixed scoring table, and compare themselves on a leaderboard.

The application is intentionally small and monolithic:

- one Axum web server
- one PostgreSQL database
- one background worker in the same binary
- Askama templates for HTML
- HTMX for partial page updates
- optional Signal and email notifications
- optional RSS news feed on the landing page

Authentication is passwordless. Each user receives a personal magic link;
opening it sets the `pila_token` cookie and authenticates the session.

## Design Principles

Pila is intentionally opinionated. The product and architecture are guided by
four principles.

### 1. Simple for players

The game should be easy to join and easy to use:

- no classic account registration flow
- no password management
- no app install requirement
- one personal magic link is enough to participate

This keeps the barrier low for family and friend groups that want a prediction
game, not another service to sign up for and maintain.

### 2. Fair, distraction-free tipping

The core promise is a fair prediction game without the usual clutter:

- no ads
- no growth mechanics around forced signups
- transparent scoring rules
- clear match-lock behaviour
- strict league isolation so one league's data never leaks into another

The architecture supports this by keeping scoring logic pure, tenancy rules
explicit, and lock/idempotency behaviour centralized instead of spreading it
across templates or ad-hoc handler code.

### 3. A page players want to come back to

Pila should not feel like a dead utilitarian form that users open only to save
their tips. The experience should invite repeat visits during the tournament.

That is why the product includes light-weight engagement layers around the core
game:

- badges for visible bragging rights
- jersey presets for a bit of identity and league flavour
- an RSS news area so the page can stay relevant between kickoffs
- room for future extensions that increase fun without undermining fairness

These additions are intentionally cosmetic or adjacent to the core scoring
logic. They should make the page more alive while keeping the actual tipping
system understandable and fair.

### 4. Low effort for the host

The person running Pila is usually doing their group a favour. Operating the
application should therefore be simple and lightweight:

- self-hosted in one small stack
- automatic migrations on startup
- optional integrations instead of mandatory external services
- operational scripts for backup, restore, invites, and admin edits
- hardened default container settings

The host should be able to provide something nice to their tipping round
without turning into a part-time operator.

## High-Level Runtime Shape

Startup in [`src/main.rs`](../src/main.rs):

1. load `.env` and tracing config
2. connect to PostgreSQL
3. run migrations via `sqlx::migrate!()`
4. build `Repos::from_pool(pool)` for the persistence boundary
5. load translations, jersey presets, RSS cache, HTTP client, and env-backed config into `AppState`
6. bootstrap notification silence for leagues that have not yet been bootstrapped
7. start the scoreboard/notification worker in normal mode, or do a one-shot sync in dev mode
8. build the Axum router, attach state and middleware, and serve requests

`AppState` lives in [`src/lib.rs`](../src/lib.rs). It deliberately keeps
request-time dependencies prebuilt and cloneable: repositories, translations,
jersey presets, HTTP client, mock time, SMTP config, and a concurrency
semaphore for global backpressure.

## Module Map

The codebase is grouped by boundary, not by framework layer alone.

### HTTP and presentation

- [`src/main.rs`](../src/main.rs): application bootstrap, router wiring, middleware, graceful shutdown
- [`src/handlers/`](../src/handlers): route handlers grouped by feature
  - `index.rs`: main prediction page
  - `predictions.rs`: match and champion submission
  - `leaderboard.rs`: ranking page
  - `auth.rs`: magic-link login
  - `setup.rs`: first-league bootstrap flow
  - `admin.rs`: per-league user administration
  - `leagues.rs`: super-admin league management and settings
  - `jersey.rs`: jersey picker and language switcher
  - `dev.rs`: dev-only routes gated by `PILA_DEV_MODE`
  - `services.rs`: shared query/orchestration helpers used by multiple handlers
  - `util.rs`: common response and translation helpers
- [`src/views.rs`](../src/views.rs): template-facing DTOs
- [`templates/`](../templates): Askama templates and partials

### Domain and application services

- [`src/auth.rs`](../src/auth.rs): request extractors such as `AuthenticatedUser`, `AdminUser`, `SuperAdminUser`
- [`src/scoring.rs`](../src/scoring.rs): pure scoring logic for exact-score and winner-only leagues
- [`src/stage.rs`](../src/stage.rs): tournament-stage enum and helpers
- [`src/badges.rs`](../src/badges.rs): on-the-fly hero-panel badge computation
- [`src/jersey.rs`](../src/jersey.rs): jersey preset loading
- [`src/time.rs`](../src/time.rs): mockable time abstraction for dev/test flows
- [`src/translations.rs`](../src/translations.rs): locale loading and string lookup
- [`src/mail.rs`](../src/mail.rs): SMTP rendering and delivery
- [`src/news.rs`](../src/news.rs): optional RSS cache and parsing

### Persistence and external providers

- [`src/repo/`](../src/repo): repository traits plus Postgres and in-memory implementations
- [`src/scoreboard/`](../src/scoreboard): provider-agnostic sports data boundary plus ESPN implementation
- [`src/worker.rs`](../src/worker.rs): background sync and notification dispatch loop

## Repository Architecture

`src/repo/` is the persistence boundary. Handlers and the worker depend on
traits collected in [`Repos`](../src/repo/mod.rs), not on raw SQL. Each major
entity follows the same pattern:

```text
src/repo/<entity>/
  mod.rs
  postgres.rs
  memory.rs
```

`mod.rs` defines the trait and public DTOs, `postgres.rs` contains the sqlx
implementation, and `memory.rs` provides the in-memory fake used by tests.

This has two practical consequences:

- handler and worker logic can be exercised without a live database
- multi-step behaviour such as notification idempotency can still live at the
  repo boundary where transactions exist

`Repos::from_pool` wires the production `Pg*Repo` implementations. Tests can
assemble custom `Repos` values with `Memory*Repo` instances to isolate one
behavioural path.

## Database Schema Overview

The current schema lives in
[`migrations/20260601000000_init.sql`](../migrations/20260601000000_init.sql).
Migrations run automatically on startup.

### Core tables

- `leagues`: one row per isolated tipping league
- `league_settings`: per-league key/value config without schema migrations for each new setting
- `users`: league-scoped users, magic-link token, locale, jersey preset, optional phone/email
- `teams`: globally shared tournament teams
- `matches`: globally shared tournament fixtures and live/final scores
- `predictions`: one row per `(user_id, match_id)`
- `special_predictions`: one champion pick per user
- `sent_notifications`: idempotency ledger for Signal and email sends
- `settings`: global key/value settings for app-wide flags
- `invite_links`: league-scoped shareable invite tokens for self-registration

### Important modeling choices

- Leagues are the tenancy boundary; matches and teams are global.
- `league_settings` is intentionally schemaless. Adding a new per-league
  setting normally means:
  1. add a field to `LeagueConfig`
  2. read/write a new key
  3. expose it in the admin UI
- `match_stage` is a Postgres enum mirrored by `Stage` in Rust. They must stay in sync.
- `sent_notifications` is keyed by `(league_id, kind, ref_id, user_id)` so
  the same reminder can be sent once per league and, for email, once per user.
- `special_predictions` locks relative to the tournament's first kickoff, not
  per later round.

## Multi-Tenancy Invariants

The most important non-obvious rule in the project is that every user belongs
to exactly one league via `users.league_id`, and all user-visible aggregate
data must stay inside that league.

Invariants:

- exact match data is global
- teams are global
- users, leaderboard entries, badges, "other users' tips", admin user lists,
  notification candidate lists, and champion-pick overviews are league-scoped
- a regular admin manages only their own league
- a super-admin is modeled as `is_admin = true` plus `can_create_league = true`
- the first user created through `/setup` becomes that super-admin

When adding a new aggregate query or cached projection, ask explicitly:
"what is the `league_id` filter?" If the answer is missing, the query is
probably wrong.

The regression net for this rule is
[`tests/multi_league_isolation.rs`](../tests/multi_league_isolation.rs).
Any new cross-user aggregate should extend that file.

## Auth and Request Model

Auth is cookie-based and intentionally simple:

- `GET /play/me/:token` validates a magic link and sets `pila_token`
- `AuthenticatedUser` resolves the cookie into the current user record
- `AdminUser` and `SuperAdminUser` are wrapper extractors that enforce role checks
- `MaybeAuthenticatedUser` exists for routes that can render differently when a session exists

State-changing POST routes are protected by the CSRF middleware in
[`src/main.rs`](../src/main.rs) using the double-submit-cookie pattern.
`/setup`, magic-link login, `/join/*`, and `/dev/*` routes are exempt.

### Invite links and self-registration

To lower onboarding effort, an admin can generate shareable invite links instead
of creating one user per player by hand:

- an admin generates/revokes links on the per-league user page
  (`POST /admin/leagues/:id/invites`, `POST /admin/invites/:id/revoke`)
- a link is a league-scoped token persisted in `invite_links`; revoking deletes
  the row so the token stops working
- anyone holding `GET /join/:token` can self-register a user in that link's
  league via `POST /join/:token` — the flow mirrors `/setup`: it sets the
  login + CSRF cookies and shows the new personal magic link to bookmark
- self-registered users are never admins and inherit the league's
  `default_language`
- the join page warns players who already have an account to reuse their
  existing magic link; if unsure, the admin can resend it from the user list
  (`POST /admin/users/:id/resend`)

## Worker and Notification Flow

The background worker in [`src/worker.rs`](../src/worker.rs) is responsible
for two separate jobs every 30 minutes.

### 1. Scoreboard sync

`update_data` iterates the configured World Cup date window
(`WC_WINDOW_START..=WC_WINDOW_END`) and calls
`ScoreboardClient::fetch_events(date)` for each UTC date. Returned events are
upserted into:

- `teams` via `TeamRepo::upsert_from_espn`
- `matches` via `MatchRepo::upsert_from_espn`

The worker is provider-agnostic. Provider-specific mapping logic belongs in
`src/scoreboard/`, currently [`src/scoreboard/espn.rs`](../src/scoreboard/espn.rs).

The canonical provider contract is documented in
[`doc/scoreboard_provider.md`](./scoreboard_provider.md). Any change to
`ScoreboardClient`, `SportsEvent`, `SportsTeam`, `MatchStatus`, or worker-side
provider expectations must update that document in the same commit.

### 2. Reminder dispatch

After syncing, the worker evaluates pending reminders per league:

- group reminder: a match locks within 24 hours and at least one league member has not tipped
- champion reminder: the champion pick locks within 24 hours and at least one league member has not picked
- email reminder variants of the same events if SMTP is configured

Dispatch is league-specific:

- each league loads its own `LeagueConfig`
- each league chooses its own default translation bundle
- Signal routing uses the league's configured group and sender number
- leagues without Signal config fall back to `NoopNotifier`
- knockout-only leagues skip group-stage reminder events

Quiet hours are enforced in [`src/notifier.rs`](../src/notifier.rs):
22:00–08:00 Europe/Berlin suppresses dispatch until the next worker tick.

### Idempotency

Notification idempotency is not bolted on in the worker. It is implemented at
the notification repo boundary through `NotificationRepo::try_send`:

1. insert `(league_id, kind, ref_id, user_id)` into `sent_notifications`
2. if the row already exists, do nothing
3. otherwise call the notifier
4. commit only on success; rollback on failure so the next tick retries

That transaction boundary is why `try_send` belongs in the repo abstraction
instead of splitting "mark sent" and "send" across layers.

### Bootstrap silence

`bootstrap_notifications` runs on startup and inserts sentinel rows for
already-known matches in leagues whose `notifications_bootstrapped` flag is
still false. This prevents a freshly deployed or newly created league from
receiving a flood of stale "closing soon" reminders for matches that were
already near lock time before the worker started.

## Internationalisation Architecture

Translations live in [`locales/`](../locales) as Mozilla Fluent `.ftl` files.
Supported locales are currently `de`, `en`, `es`, and `fr`.

The translation flow:

1. `translations::load_all()` reads all four locale files at startup
2. `load_one()` parses Fluent resources and materialises plain string tables
3. handlers resolve one `T` bundle per request via the user's language or a fallback
4. templates and message renderers call `T::get` or `T::format`

Important behaviour:

- missing translation files are a startup failure
- missing keys do not panic; the key string itself is rendered as visible fallback
- missing format arguments also stay visible rather than crashing
- new UI strings must land in all four locale files

Two different language concepts coexist intentionally:

- `users.language`: the individual user's chosen UI language
- `LeagueConfig.default_language`: the fallback language for new users and the
  default language used for league-scoped notifications

## Scoring Model

Scoring lives in [`src/scoring.rs`](../src/scoring.rs) and is intentionally
pure Rust with no SQL or HTTP concerns.

Pila currently supports two per-league match scoring systems:

- `ExactScore`: exact score, goal difference, tendency
- `WinnerOnly`: only the predicted outcome matters

The selected system is stored in `league_settings` and loaded into
`LeagueConfig.match_scoring_system`. Handlers and services should branch on the
configured scoring system, not on ad-hoc route or template assumptions.

Champion picks are scored separately with a flat value and lock once the
tournament's first kickoff is reached.

## Badge System

Badges are implemented in [`src/badges.rs`](../src/badges.rs). They are
deliberately cosmetic:

- badges never alter leaderboard points
- badge values are computed on the fly from existing predictions and match data
- nothing is persisted or cached in a dedicated badge table

Each request builds one `BadgeContext`, and every registered badge derives its
own `BadgeDisplay` from that shared snapshot. This keeps badge logic pure and
ensures admin result corrections are reflected immediately.

Because badges are visible next to "real" points, incorrect values are a trust
problem. New badges should ship with focused tests that cover happy path, empty
data, and the relevant edge condition.

## SQLx Offline Workflow

Pila uses `sqlx::query!` and `query_as!` macros. Their compile-time metadata is
committed under [`.sqlx/`](../.sqlx).

Required workflow when changing a sqlx macro:

1. make the code change
2. ensure `DATABASE_URL` points to a live development database
3. run `cargo sqlx prepare`
4. commit the updated `.sqlx/` files in the same change

This matters because the Docker build runs with `SQLX_OFFLINE=true`; if the
metadata is stale, the build fails even when local code compiles.

## External Inputs and Configuration

The most important env vars are:

- `DATABASE_URL`: application database
- `BASE_URL`: public base URL used in generated links and notifications
- `PORT`: HTTP bind port
- `RUST_LOG`: tracing filter
- `PILA_DEV_MODE`: enables dev-only routes and one-shot sync behaviour
- `WC_WINDOW_START`, `WC_WINDOW_END`: scoreboard polling window override
- SMTP vars: enable email delivery when all required values are present

Signal configuration is intentionally split:

- `SIGNAL_API_URL` stays global because it points to the local signal-cli REST service
- worker-side reminder dispatch reads `signal_group_id` and `signal_from_number`
  from `LeagueConfig`
- invite sending in `setup.rs` and `admin.rs` still checks the global
  `AppState.signal_from_number` path, so Signal config is not fully unified yet

RSS is currently split as well:

- `RSS_FEED_URL` seeds the in-process cache from env
- `LeagueConfig.rss_feed_url` already exists as the intended per-league model

If this split is changed later, update both this document and the relevant
admin/runtime wiring in the same commit.
