# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

**Pila** is a FIFA World Cup 2026 prediction game (Tippspiel) in Rust. Users tip exact scores for every match (group + knockout) plus a single Weltmeister pick, scored Kicktipp-style with stage multipliers. Magic-link cookie auth, Askama HTML templates, Postgres via sqlx, optional Signal-group push. ESPN's `soccer/fifa.world` scoreboard endpoint feeds the background sync worker.

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
├── main.rs       # Axum server + all HTTP handlers (handlers mod inline) + AppState wiring
├── lib.rs        # module declarations + AppState { db: PgPool }
├── auth.rs       # AuthenticatedUser FromRequestParts extractor (pila_token cookie)
├── stage.rs      # Stage enum (sqlx::Type matching Postgres `match_stage` ENUM) + multipliers + DE labels
├── scoring.rs    # Pure-Rust Kicktipp scoring; no SQL; tested in-file
├── notifier.rs   # Notifier trait + SignalNotifier + NoopNotifier + quiet-hour gate
└── worker.rs     # ESPN sync + idempotent notification dispatch (30-min loop)
```

**AppState** holds a 5-connection `PgPool`, cloned across handlers via `Router::with_state`.

**Auth** is a single Axum extractor reading the `pila_token` cookie. Routes that must be private take `AuthenticatedUser` directly; there is currently no `Option<AuthenticatedUser>` variant — every route except `/play/me/:token` requires auth. Magic links are issued out-of-band via `create_invite.sh`.

**Worker** (`worker.rs`) polls ESPN's soccer/fifa.world scoreboard once per day across `WC_WINDOW_START..=WC_WINDOW_END` (default 2026-06-01 → 2026-07-25, override via env). Single-date queries are required — soccer scoreboards do not return knockout events on date-range queries reliably. Stage classification is heuristic on the competition `notes[].headline` (`"Group A - Matchday 1"`, `"Round of 16"`, `"Quarterfinals"`, etc. — see `classify_stage`). The worker upserts into `matches` keyed on `espn_event_id` so re-runs are idempotent and missed days are backfilled.

**Notifications** fire on two triggers:

1. `match_closing_soon` — match kicks off in <24h and at least one user has not tipped.
2. `special_lock_soon` — first kickoff in tournament (= Weltmeister-tip lock) is <24h away and at least one user has no champion pick. Uses `ref_id = 0` since it's a singleton.

Idempotency: `(kind, ref_id)` PK on `sent_notifications` inside a transaction with the actual send — failed sends roll the row back so the next tick retries. Quiet hours 22:00–08:00 Europe/Berlin defer dispatch (the worker re-checks every 30 min, no scheduling).

**Bootstrap silence**: `bootstrap_notifications` runs on every startup but is gated by a `notifications_bootstrapped=true` row in `settings`; on the first run it inserts a `match_closing_soon` row for every currently-known match so deploying mid-tournament does not flood the group.

**Scoring** (`scoring.rs`) is intentionally SQL-free, takes plain ints, returns ints. Per-match base points are 4 (exact) / 2 (correct goal-diff) / 1 (correct tendency) / 0, multiplied by `Stage::multiplier()` (Group=1, R32=2, R16=3, QF=4, ThirdPlace=4, SF=5, Final=6). Champion pick is flat 10 points. **Note**: K.O. matches store the result *before* penalty shootout (90 min + ET) — a draw at that point is a valid stored score and counts for exact-result points.

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
