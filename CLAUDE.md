# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Language Convention

**Working language:** The user writes in German. Respond in German.

**Code language:** Everything in the repository must be in English — without exception:
- Source code, variable/function/type names
- Database fields, migration files, SQL
- Code comments and doc comments
- Template comments (Jinja/Askama `{# … #}`)
- Test names and assertion messages
- Documentation files (`.md`, `doc/`)
- Commit messages, PR descriptions

**UI strings are internationalised** — user-facing strings are NOT hardcoded in any language. They live in `locales/{de,en,es,fr}.ftl` (Mozilla Fluent format). Every new UI string must be added to **all four** locale files with a `# translator comment` explaining context. Signal notifications remain German (group is German-speaking).

## What This Is

**Pila** is a FIFA World Cup 2026 prediction game (Tippspiel) in Rust. Users tip exact scores for every match (group + knockout) plus a single Weltmeister pick, scored by a fixed per-phase points table (not a multiplier — see `scoring.rs`). Magic-link cookie auth, Askama HTML templates, Postgres via sqlx, optional Signal-group push. ESPN's `soccer/fifa.world` scoreboard endpoint feeds the background sync worker.

**Multi-Tenancy (Tipp-Liga):** the app supports multiple isolated leagues. Every user belongs to exactly one league via `users.league_id`. Aggregate reads (leaderboard, badges, "other tips" panel, admin user list, notifications) MUST filter by `league_id` — `users.list_for_admin/list_basic/list_ids`, `predictions.list_finished_join/list_leaderboard_join/list_other_users_locked`, `special_predictions.list_with_user_names/list_all_picks`, and the entire `notifications` repo all take `league_id` as a parameter. Matches and teams stay global (one tournament). Per-league config (Signal group, default language, RSS) lives in the `league_settings` k/v table — adding a new setting means a new field on `LeagueConfig` + a key constant, no migration. The Signal `SIGNAL_GROUP_ID`/`SIGNAL_FROM_NUMBER` env vars are now per-league config; `SIGNAL_API_URL` stays global. `tests/multi_league_isolation.rs` is the regression net — every PR that adds a new aggregate query MUST extend that file.
The first user created via `/setup` is granted `can_create_league`, the super-admin permission required to create new leagues and edit any league's settings (regular `is_admin` only manages the admin's own league).
`sent_notifications` PK is `(league_id, kind, ref_id)` so two leagues can each receive the same `(kind, ref_id)` independently.

**Sister project at `../vates`** — same architecture (Axum + sqlx + Askama + Signal-cli + ESPN sync), but for NBA playoff series predictions. When in doubt about Signal-cli setup, quiet hours, idempotent notifications, or the SQLx offline workflow, vates' README/CLAUDE.md cover the same ground in more detail. Differences worth knowing: pila scores per-match (not per-series), uses stages `group`/`round_of_32`/`round_of_16`/`quarter_final`/`semi_final`/`third_place`/`final` instead of NBA rounds, polls ESPN every 30 min over a date window (not single-day), and the magic-link cookie is `pila_token` (not `vates_token`).

## Commands

```bash
cargo build --release
cargo run                          # needs DATABASE_URL in .env
cargo test                         # scoring/notifier/worker unit tests
cargo test scoring                 # one module
cargo check
cargo clippy
```

DB live on `localhost:6433` per `docker-compose.override.yml` (host port 6433 → container 5432). Inside docker-compose, app talks to db on `5432`.

```bash
docker compose up -d db            # start Postgres only
docker compose up -d app           # full stack (db + signal-cli + app)
docker compose logs -f app
docker compose exec db psql -U pila -d pila_db
```

Local dev against the override DB:

```bash
DATABASE_URL=postgres://pila:<pw>@localhost:6433/pila_db cargo run
```

## SQLx Offline Mode

All `sqlx::query!`/`query_as!` invocations are checked at compile time against `.sqlx/`. The Dockerfile sets `SQLX_OFFLINE=true`, so **after adding/changing/removing any sqlx macro you MUST run `cargo sqlx prepare` and commit `.sqlx/`** — the docker build fails otherwise. `cargo sqlx prepare` itself needs a live `DATABASE_URL`.

## Architecture

```
src/
├── main.rs          # Axum server + all HTTP handlers (handlers mod inline) + AppState wiring
├── lib.rs           # module declarations + AppState { db: PgPool }
├── auth.rs          # AuthenticatedUser FromRequestParts extractor (pila_token cookie)
├── stage.rs         # Stage enum + TournamentPhase mapping + ftl_key() for i18n
├── scoring.rs       # Pure-Rust scoring (two modes: ExactScore / WinnerOnly); no SQL; tested in-file
├── translations.rs  # T wrapper (Arc<HashMap>) loaded from locales/*.ftl at startup
├── notifier.rs      # Notifier trait + SignalNotifier + NoopNotifier + quiet-hour gate
└── worker.rs        # ESPN sync + idempotent notification dispatch (30-min loop)
```

**AppState** holds a 5-connection `PgPool`, cloned across handlers via `Router::with_state`. It also holds `translations: HashMap<String, T>` — all four locale bundles loaded once at startup.

**Internationalisation**: supported locales are `de`, `en`, `es`, `fr`. Translations live in `locales/{de,en,es,fr}.ftl` (Mozilla Fluent format). Each FTL message has a `# translator comment` for context. The `T` struct (`src/translations.rs`) wraps `Arc<HashMap<String, String>>` — clone is cheap. Every full-page Askama template struct carries `t: T` and `lang_code: String`. The user's preferred language is stored in `users.language` (VARCHAR DEFAULT `'de'`) and surfaced via `AuthenticatedUser.language`. Language switching: `POST /profile/language` validates the locale, persists it, returns `HX-Location: /` for HTMX client-side navigation. **Rule: every new UI string must appear in all four FTL files with a translator comment.**

**Auth** is a single Axum extractor reading the `pila_token` cookie. Routes that must be private take `AuthenticatedUser` directly; there is currently no `Option<AuthenticatedUser>` variant — every route except `/play/me/:token` requires auth. Magic links are issued out-of-band via `create_invite.sh`.

**Worker** (`worker.rs`) polls the configured `ScoreboardClient` once per day across `WC_WINDOW_START..=WC_WINDOW_END` (default 2026-06-01 → 2026-07-25, override via env) and upserts the returned `SportsEvent`s via the repo layer. The worker itself contains no HTTP code — provider details live behind the `pila::scoreboard::ScoreboardClient` trait in `src/scoreboard/`. The default implementation (`EspnClient`) targets ESPN's `soccer/fifa.world` endpoint; stage classification, group-letter resolution, and flag mapping are ESPN-specific and live in `src/scoreboard/espn.rs`.

**Adding or changing a scoreboard provider:** the contract a new `ScoreboardClient` implementation must honour is documented in [`doc/scoreboard_provider.md`](doc/scoreboard_provider.md). **That file is the source of truth** — any change to `ScoreboardClient`, its DTOs (`SportsEvent`, `SportsTeam`, `MatchStatus`), or the worker's expectations of provider behaviour MUST be reflected there in the same commit. Treat it like a public API doc, not a tutorial.

**Notifications** fire on two triggers:

1. `match_closing_soon` — match kicks off in <24h and at least one user has not tipped.
2. `special_lock_soon` — first kickoff in tournament (= Weltmeister-tip lock) is <24h away and at least one user has no champion pick. Uses `ref_id = 0` since it's a singleton.

Idempotency: `(kind, ref_id)` PK on `sent_notifications` inside a transaction with the actual send — failed sends roll the row back so the next tick retries. Quiet hours 22:00–08:00 Europe/Berlin defer dispatch (the worker re-checks every 30 min, no scheduling).

**Bootstrap silence**: `bootstrap_notifications` runs on every startup but is gated by a `notifications_bootstrapped=true` row in `settings`; on the first run it inserts a `match_closing_soon` row for every currently-known match so deploying mid-tournament does not flood the group.

**Scoring** (`scoring.rs`) is intentionally SQL-free, takes plain ints, returns ints. Each league selects a `MatchScoringSystem` (`ExactScore` default, or `WinnerOnly`). Points come from a fixed table keyed on `TournamentPhase` — there are no multipliers.

`ExactScore` points (exact / goal-diff / tendency / wrong):
- Group: 4 / 3 / 2 / 0
- R32, R16: 6 / 4 / 3 / 0
- QF, SF: 8 / 6 / 5 / 0
- ThirdPlace + Final: 11 / 8 / 6 / 0

`WinnerOnly` points (correct / wrong): Group 1/0 · R32 2/0 · R16 3/0 · QF 5/0 · SF+Finals 7/0.

Champion pick is flat 10 points. **Note**: K.O. matches store the result *before* penalty shootout (90 min + ET) — a draw at that point is a valid stored score and counts for exact-result points.

**Badges** (`badges.rs`) sind die Hero-Panel-Gamification und bewusst vom Scoring entkoppelt — Badges ändern keine Punkte, sie aggregieren nur über vorhandene Predictions. **Werte werden bei jedem Request on-the-fly berechnet, nichts wird persistiert** (keine `badges`-Tabelle, kein Cache). So bleibt jeder Badge konsistent zur Realität — auch wenn ein Admin Ergebnisse nachträglich korrigiert.

Architektur:

- `Badge`-Trait: jeder Badge ist ein Unit-Struct mit statischen Metadaten (`icon`, `title`, `how_to_earn`) und `compute(&BadgeContext) -> BadgeDisplay`. Pure, kein DB-Zugriff in Badges.
- `BadgeDisplay` hat zwei Varianten:
  - `Achievement { times_earned }` — wiederholbar (z.B. Solo-Treffer pro Match); Renderer zeigt Icon + Titel groß und `×N` in der Ecke. `times_earned == 0` rendert ausgegraut.
  - `Metric(BadgeValue)` — Quoten/Streaks/Deltas; ein Wert prominent (`Count`, `Fraction`, `Percent`, `Streak`, `Delta`, `Champion`, `Empty`).
- `BadgeContext` ist ein einmal pro Request gebauter Read-only-Snapshot (alle Tipps × fertige Matches, Special-Picks, Champion-Status). Drei Queries — alle Badges teilen ihn (siehe `build_badge_context` in `main.rs`).
- `registry()` listet alle Badges in Anzeige-Reihenfolge. Neuer Badge = Struct + Trait-Impl + Eintrag in `registry()` + Tests. Handler und Template bleiben unverändert.
- Rendering: `templates/partials/badge.html` macht Pattern-Match auf `BadgeDisplay` und nutzt Helfer-Methoden (`is_achievement()`, `metric_kind()` etc.) statt direktem Enum-Match.

**Wichtig**: Badges sind sichtbar und „prahlerisch" — falsche Werte untergraben das Vertrauen ins parallel angezeigte Punkte-System. Jeder Badge MUSS Tests haben (Happy Path, Leerdaten, Edge Case wie Tie/Threshold/fehlender Vorgängertag). `cargo test --lib badges` ist Pflicht-Gate vor dem Mergen.

**Templates**: Askama compile-time templates in `templates/` (`base.html`, `index.html`, `leaderboard.html`). HTMX is used inline in handlers (e.g. `predict_match` returns a partial form fragment for `hx-swap`).

## Database

Single migration `migrations/20260601000000_init.sql`. Migrations run on app startup via `sqlx::migrate!()` — no manual step.

Key tables:

- `users` — UUID PK, `token` (magic-link login), `is_admin`
- `teams` — INTEGER PK = ESPN team ID, `flag_code` (alpha-2 for flagcdn.com), `group_letter`
- `matches` — SERIAL PK, `stage match_stage`, optional `group_letter`, nullable team FKs while TBD, `espn_event_id` UNIQUE for upsert
- `predictions` — composite PK `(user_id, match_id)`, locked once `kickoff_time < NOW()`
- `special_predictions` — one row per user, `champion_id`, locked once `MIN(kickoff_time) < NOW()`
- `sent_notifications` — `(kind, ref_id)` PK; idempotency table
- `settings` — k/v; holds `notifications_bootstrapped` flag

`match_stage` is a Postgres ENUM mirrored by `Stage` in `stage.rs` — keep both in sync (the sqlx derive uses `rename_all = "snake_case"`).

## Environment

`.env.example` documents the required vars. Minimum to boot the app:

- `POSTGRES_PASSWORD` (consumed by both `db` and `app` services in compose)
- `BASE_URL` — public URL embedded in Signal messages

Optional:

- `SIGNAL_API_URL` / `SIGNAL_FROM_NUMBER` / `SIGNAL_GROUP_ID` — leave any empty to fall back to `NoopNotifier` (app still runs)
- `WC_WINDOW_START`, `WC_WINDOW_END` (`YYYY-MM-DD`) — override the default ESPN sync window
- `PORT` — defaults to 8000
- `RUST_LOG` — defaults to `info`

## Operational scripts

```bash
./create_invite.sh        # prompts for name, inserts user + token, prints magic-link URL
./admin_edit.sh           # interactive: edit a user's predictions / champion / match results
./backup_db.sh            # gz dump under ./backups
./restore_db.sh <path>    # restore from dump
./signal_send.sh          # ad-hoc Signal group message via signal-cli REST
```

`PILA_BASE_URL` (env) overrides the printed link host in `create_invite.sh` (defaults to `http://localhost:8000`).

## Routes

- `GET  /` — index (auth required)
- `GET  /play/me/:token` — magic-link login, sets `pila_token` cookie, redirects to `/`
- `POST /predict/:match_id` — HTMX partial; returns the inline form fragment, validates 0–20 score range and not-yet-locked
- `POST /predict_special` — champion form; rejects after first kickoff
- `GET  /leaderboard` — full-page leaderboard
